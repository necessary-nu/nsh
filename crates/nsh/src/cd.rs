//! Literal port of `src/cd.c` / `src/cd.h`.
//! Rules: `docs/spec/port/src/cd.md`.
//!
//! `__CYGWIN__` is not selected, so `updatepwd` does no path normalisation.
//! `__GLIBC__` *is* selected, so `getpwd` uses `getcwd(0, 0)`.

use bstr::{BStr, BString, ByteSlice};
use core::ptr::{addr_of, addr_of_mut, null_mut};
use libc::{c_char, c_int, c_void};

use crate::error::{INTOFF, INTON};
use crate::mystring::{dotdir, homestr, nullstr};
use crate::options::{argptr, nextopt};
use crate::output::VaArg;
use crate::shell::cstr;
use crate::var::{bltinlookup, setvar, VEXPORT};

const CD_PHYSICAL: c_int = 1;
const CD_PRINT: c_int = 2;

/* The C's `nullstr` sentinel is `None`.  It is a sentinel, not an empty
 * path: `getpwd` never returns an empty string on success and `updatepwd`
 * never produces one, so no reachable value collides with it. */
static mut curdir: Option<BString> = None; /* current working directory */
static mut physdir: Option<BString> = None; /* physical working directory */

#[inline]
unsafe fn errno() -> c_int {
    *libc::__errno_location()
}

/// The bytes `setvar` and `out1fmt` want: a path with the terminator the C's
/// readers read up to, `nullstr`'s empty string when the sentinel is set.
unsafe fn cbytes(s: &Option<BString>) -> Vec<u8> {
    let mut v = match s {
        Some(b) => b.to_vec(),
        None => Vec::new(),
    };
    v.push(0);
    v
}

// [spec:dash:def:cd.cdopt-fn]
// [spec:dash:sem:cd.cdopt-fn]
unsafe fn cdopt() -> c_int {
    let mut flags: c_int = 0;
    let mut i: c_int;
    let mut j: c_int;

    j = b'L' as c_int;
    loop {
        i = nextopt(b"LP\0".as_ptr() as *const c_char);
        if i == 0 {
            break;
        }
        if i != j {
            flags ^= CD_PHYSICAL;
            j = i;
        }
    }

    flags
}

// [spec:dash:def:cd.cdcmd-fn]
// [spec:dash:sem:cd.cdcmd-fn]
pub unsafe fn cdcmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    let mut dest: *const c_char;
    let mut path: *const c_char;
    let mut p: *const c_char;
    let mut c: c_char;
    let mut statb: libc::stat64 = core::mem::zeroed();
    let mut flags: c_int;
    let mut len: c_int;

    flags = cdopt();
    dest = *argptr;
    if dest.is_null() {
        dest = bltinlookup(addr_of!(homestr) as *const c_char);
    } else if *dest.offset(0) == b'-' as c_char && *dest.offset(1) == b'\0' as c_char {
        dest = bltinlookup(b"OLDPWD\0".as_ptr() as *const c_char);
        flags |= CD_PRINT;
    }
    if dest.is_null() {
        dest = addr_of!(nullstr) as *const c_char;
    }

    let mut step6 = false;
    if *dest == b'/' as c_char {
        step6 = true; /* goto step6 */
    } else if *dest == b'.' as c_char {
        c = *dest.offset(1);
        loop {
            /* dotdot: */
            if c == b'\0' as c_char || c == b'/' as c_char {
                step6 = true; /* goto step6 */
                break;
            }
            if c == b'.' as c_char {
                c = *dest.offset(2);
                if c != b'.' as c_char {
                    continue; /* goto dotdot */
                }
            }
            break;
        }
    }

    let mut out = false;
    /* The CDPATH candidate `docd` is handed, copied out of `padvance`'s
     * buffer.  Held across the whole loop rather than per iteration
     * because `p` still points into it after the `break`. */
    let mut keptbuf: Vec<u8> = Vec::new();
    if !step6 {
        if *dest == 0 {
            dest = addr_of!(dotdir) as *const c_char;
        }
        path = bltinlookup(b"CDPATH\0".as_ptr() as *const c_char);
        loop {
            p = path;
            len = crate::exec::padvance_magic(&mut path, dest, 0);
            if len < 0 {
                break;
            }
            c = *p;
            /* `stalloc(len)` took the candidate the C had built in the
             * stack block; the copy is what takes it out of `padvance`'s
             * buffer, which the `docd` below can overwrite.  `len` is
             * `padvance`'s *allocation* size, one more than the string's
             * length when the PATH component is empty, so the buffer is
             * sized from it and the bytes are copied by hand. */
            keptbuf.clear();
            keptbuf.resize(len as usize, 0);
            libc::strcpy(
                keptbuf.as_mut_ptr() as *mut c_char,
                crate::exec::padvance_result(),
            );
            debug_assert!(libc::strlen(keptbuf.as_ptr() as *const c_char) < len as usize);
            p = keptbuf.as_ptr() as *const c_char;

            if libc::stat64(p, &mut statb) >= 0 && (statb.st_mode & libc::S_IFMT) == libc::S_IFDIR
            {
                if c != 0 && c != b':' as c_char {
                    flags |= CD_PRINT;
                }
                /* docd: */
                if docd(p, flags) == 0 {
                    out = true; /* goto out */
                    break;
                }
                /* goto err */
                crate::error::sh_error(cstr(b"can't cd to %s\0"), &[VaArg::Str(dest)]);
            }
        }
    }

    if !out {
        /* step6: */
        p = dest;
        /* docd: */
        if docd(p, flags) != 0 {
            /* err: */
            crate::error::sh_error(cstr(b"can't cd to %s\0"), &[VaArg::Str(dest)]);
            /* NOTREACHED */
        }
    }

    /* out: */
    if (flags & CD_PRINT) != 0 {
        let d = cbytes(&*addr_of!(curdir));
        crate::output::out1fmt(
            addr_of!(crate::mystring::snlfmt) as *const c_char,
            &[VaArg::Str(d.as_ptr() as *const c_char)],
        );
    }
    0
}

/*
 * Actually do the chdir.  We also call hashcd to let the routines in exec.c
 * know that the current directory has changed.
 */

// [spec:dash:def:cd.docd-fn]
// [spec:dash:sem:cd.docd-fn]
unsafe fn docd(mut dest: *const c_char, flags: c_int) -> c_int {
    let mut dir: *const c_char = null_mut();
    let err: c_int;

    /* `TRACE(("docd(\"%s\", %d) called\n", dest, flags));` — `#ifdef DEBUG`
     * in `shell.h`, and the dash build does not define it. */

    INTOFF();
    if (flags & CD_PHYSICAL) == 0 {
        dir = updatepwd(dest);
        if !dir.is_null() {
            dest = dir;
        }
    }
    err = libc::chdir(dest);
    if err == 0 {
        setpwd(dir, 1);
        crate::exec::hashcd();
    }
    /* out: */
    INTON();
    err
}

/*
 * Update curdir (the name of the current directory) in response to a
 * cd command.
 */

// [spec:dash:def:cd.updatepwd-fn]
// [spec:dash:sem:cd.updatepwd-fn]
unsafe fn updatepwd(dir: *const c_char) -> *const c_char {
    /* `lim` is `stackblock() + 1` in the C, re-read after `makestrspace`
     * because the block can move; against an owned buffer it is just an
     * index, and `new > lim` is a comparison of lengths. */
    let mut lim: usize;

    /* #ifdef __CYGWIN__ — not selected. */

    /* `sstrdup(dir)`.  The copy outlives the whole walk because the
     * components below borrow it while `new` grows. */
    let cdcompbuf: Vec<u8> =
        core::slice::from_raw_parts(dir as *const u8, libc::strlen(dir)).to_vec();
    let new = &mut *addr_of_mut!(pwdbuf);
    new.clear();
    if *dir != b'/' as c_char {
        let Some(cur) = &*addr_of!(curdir) else {
            return null_mut();
        };
        new.extend_from_slice(cur);
    }
    new.reserve(libc::strlen(dir) + 2);
    lim = 1;
    if *dir != b'/' as c_char {
        /* `*(new - 1)` reads before the stack block when `curdir` is empty.
         * It cannot be — `curdir` is either `nullstr`, which returned above,
         * or a path `updatepwd` itself produced — so this only differs from
         * the C on a path the C reads out of bounds on. */
        if new.last() != Some(&b'/') {
            new.push(b'/');
        }
        if new.len() > lim && new[lim] == b'/' {
            lim += 1;
        }
    } else {
        new.push(b'/');
        if *dir.offset(1) == b'/' as c_char && *dir.offset(2) != b'/' as c_char {
            new.push(b'/');
            lim += 1;
        }
    }
    /* `strtok(cdcomppath, "/")` walked from just past the leading slashes the
     * arm above consumed; an empty field is exactly what `strtok` never
     * yields, so skipping them here would change nothing. */
    for p in cdcompbuf.split_str(b"/") {
        if p.is_empty() {
            continue;
        }
        if p == b".." {
            while new.len() > lim {
                new.pop();
                if new[new.len() - 1] == b'/' {
                    break;
                }
            }
        } else if p == b"." {
            /* nothing */
        } else {
            /* fall through / default: */
            new.extend_from_slice(p);
            new.push(b'/');
        }
    }
    if new.len() > lim {
        new.pop();
    }
    /* `*new = '\0'` — the C writes the terminator at the cursor without
     * advancing it, and the caller reads the block as a C string. */
    new.push(0);
    new.as_ptr() as *const c_char
}

/// [`updatepwd`]'s result, which the C left in the stack block for its one
/// caller to read before the next `cd`.
static mut pwdbuf: BString = BString::new(Vec::new());

/*
 * Find out what the current directory is. If we already know the current
 * directory, this routine returns immediately.
 */

// [spec:dash:def:cd.getpwd-fn]
// [spec:dash:sem:cd.getpwd-fn]
unsafe fn getpwd() -> Option<BString> {
    /* #ifdef __GLIBC__ — `getcwd(0, 0)` allocates as it needs to, which is
     * what makes it work for a path longer than `PATH_MAX`; the buffer is
     * libc's, so it is copied out and released rather than kept. */
    let dir: *mut c_char = libc::getcwd(null_mut(), 0);

    if !dir.is_null() {
        let owned =
            BString::from(core::slice::from_raw_parts(dir as *const u8, libc::strlen(dir)));
        libc::free(dir as *mut c_void);
        return Some(owned);
    }

    crate::error::sh_warnx(
        cstr(b"getcwd() failed: %s\0"),
        &[VaArg::Str(libc::strerror(errno()))],
    );
    None
}

// [spec:dash:def:cd.pwdcmd-fn]
// [spec:dash:sem:cd.pwdcmd-fn]
pub unsafe fn pwdcmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    let flags: c_int;

    flags = cdopt();
    let dir = if flags != 0 {
        if (*addr_of!(physdir)).is_none() {
            setpwd_inner(Pwd::Current, 0);
        }
        cbytes(&*addr_of!(physdir))
    } else {
        cbytes(&*addr_of!(curdir))
    };
    crate::output::out1fmt(
        addr_of!(crate::mystring::snlfmt) as *const c_char,
        &[VaArg::Str(dir.as_ptr() as *const c_char)],
    );
    0
}

/// What `setpwd`'s `val` says, which the C encodes in two pointer
/// comparisons against a value the caller cannot construct any other way.
enum Pwd<'a> {
    /// `setpwd(NULL, …)` — ask the kernel, and take the answer for both
    /// `curdir` and `physdir`.
    Unknown,
    /// `setpwd(curdir, …)` — `pwdcmd`'s call.  Refresh `physdir`; `curdir`
    /// already holds the logical path and keeps it.
    Current,
    /// `setpwd(p, …)` — adopt `p` as the logical path.
    New(&'a BStr),
}

// [spec:dash:def:cd.setpwd-fn]
// [spec:dash:sem:cd.setpwd-fn]
pub unsafe fn setpwd(val: *const c_char, setold: c_int) {
    if val.is_null() {
        setpwd_inner(Pwd::Unknown, setold);
    } else {
        let bytes = core::slice::from_raw_parts(val as *const u8, libc::strlen(val));
        setpwd_inner(Pwd::New(BStr::new(bytes)), setold);
    }
}

unsafe fn setpwd_inner(val: Pwd, setold: c_int) {
    if setold != 0 {
        let old = cbytes(&*addr_of!(curdir));
        setvar(
            b"OLDPWD\0".as_ptr() as *const c_char,
            old.as_ptr() as *const c_char,
            VEXPORT,
        );
    }
    INTOFF();
    /* `free(physdir)` guarded by `physdir != oldcur`: the C's `curdir` and
     * `physdir` are one allocation after a `setpwd(NULL, …)`, and the guard
     * exists only to stop the double free.  Two owned copies say the same
     * thing without the alias. */
    physdir = None;
    match val {
        Pwd::Unknown | Pwd::Current => {
            let s = getpwd();
            if matches!(val, Pwd::Unknown) {
                curdir = s.clone();
            }
            physdir = s;
        }
        Pwd::New(v) => {
            curdir = Some(v.to_owned());
        }
    }
    let dir = cbytes(&*addr_of!(curdir));
    INTON();
    setvar(
        b"PWD\0".as_ptr() as *const c_char,
        dir.as_ptr() as *const c_char,
        VEXPORT,
    );
}
