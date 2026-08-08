//! Literal port of `src/cd.c` / `src/cd.h`.
//! Rules: `docs/spec/port/src/cd.md`.
//!
//! `__CYGWIN__` is not selected, so `updatepwd` does no path normalisation.
//! `__GLIBC__` *is* selected, so `getpwd` uses `getcwd(0, 0)`.

use bstr::BString;
use core::ptr::{addr_of, addr_of_mut, null_mut};
use libc::{c_char, c_int, c_void, size_t};

use crate::error::{INTOFF, INTON};
use crate::memalloc::{savestr, stalloc};
use crate::mystring::{dotdir, homestr, nullstr};
use crate::options::{argptr, nextopt};
use crate::output::VaArg;
use crate::shell::cstr;
use crate::var::{bltinlookup, setvar, VEXPORT};

const CD_PHYSICAL: c_int = 1;
const CD_PRINT: c_int = 2;

static mut curdir: *mut c_char = addr_of!(nullstr) as *mut c_char; /* current working directory */
static mut physdir: *mut c_char = addr_of!(nullstr) as *mut c_char; /* physical working directory */

#[inline]
unsafe fn errno() -> c_int {
    *libc::__errno_location()
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
             * buffer, which the `docd` below can overwrite. */
            let kept = stalloc(len as size_t) as *mut c_char;
            libc::strcpy(kept, crate::exec::padvance_result());
            p = kept as *const c_char;

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
        crate::output::out1fmt(
            addr_of!(crate::mystring::snlfmt) as *const c_char,
            &[VaArg::Str(curdir)],
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
    let mut p: *mut c_char;
    let mut cdcomppath: *mut c_char;
    /* `lim` is `stackblock() + 1` in the C, re-read after `makestrspace`
     * because the block can move; against an owned buffer it is just an
     * index, and `new > lim` is a comparison of lengths. */
    let mut lim: usize;

    /* #ifdef __CYGWIN__ — not selected. */

    cdcomppath = crate::mystring::sstrdup(dir);
    let new = &mut *addr_of_mut!(pwdbuf);
    new.clear();
    if *dir != b'/' as c_char {
        if curdir == addr_of!(nullstr) as *mut c_char {
            return null_mut();
        }
        new.extend_from_slice(core::slice::from_raw_parts(
            curdir as *const u8,
            libc::strlen(curdir),
        ));
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
        cdcomppath = cdcomppath.add(1);
        if *dir.offset(1) == b'/' as c_char && *dir.offset(2) != b'/' as c_char {
            new.push(b'/');
            cdcomppath = cdcomppath.add(1);
            lim += 1;
        }
    }
    p = libc::strtok(cdcomppath, b"/\0".as_ptr() as *const c_char);
    while !p.is_null() {
        if *p == b'.' as c_char
            && *p.offset(1) == b'.' as c_char
            && *p.offset(2) == b'\0' as c_char
        {
            while new.len() > lim {
                new.pop();
                if new[new.len() - 1] == b'/' {
                    break;
                }
            }
        } else if *p == b'.' as c_char && *p.offset(1) == b'\0' as c_char {
            /* nothing */
        } else {
            /* fall through / default: */
            new.extend_from_slice(core::slice::from_raw_parts(p as *const u8, libc::strlen(p)));
            new.push(b'/');
        }
        p = libc::strtok(null_mut(), b"/\0".as_ptr() as *const c_char);
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
unsafe fn getpwd() -> *mut c_char {
    /* #ifdef __GLIBC__ */
    let dir: *mut c_char = libc::getcwd(null_mut(), 0);

    if !dir.is_null() {
        return dir;
    }

    crate::error::sh_warnx(
        cstr(b"getcwd() failed: %s\0"),
        &[VaArg::Str(libc::strerror(errno()))],
    );
    addr_of!(nullstr) as *mut c_char
}

// [spec:dash:def:cd.pwdcmd-fn]
// [spec:dash:sem:cd.pwdcmd-fn]
pub unsafe fn pwdcmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    let flags: c_int;
    let mut dir: *const c_char = curdir;

    flags = cdopt();
    if flags != 0 {
        if physdir == addr_of!(nullstr) as *mut c_char {
            setpwd(dir, 0);
        }
        dir = physdir;
    }
    crate::output::out1fmt(
        addr_of!(crate::mystring::snlfmt) as *const c_char,
        &[VaArg::Str(dir)],
    );
    0
}

// [spec:dash:def:cd.setpwd-fn]
// [spec:dash:sem:cd.setpwd-fn]
pub unsafe fn setpwd(val: *const c_char, setold: c_int) {
    let oldcur: *mut c_char;
    let mut dir: *mut c_char;

    oldcur = curdir;
    dir = curdir;

    if setold != 0 {
        setvar(b"OLDPWD\0".as_ptr() as *const c_char, oldcur, VEXPORT);
    }
    INTOFF();
    if physdir != addr_of!(nullstr) as *mut c_char {
        if physdir != oldcur {
            libc::free(physdir as *mut c_void);
        }
        physdir = addr_of!(nullstr) as *mut c_char;
    }
    if oldcur as *const c_char == val || val.is_null() {
        let s: *mut c_char = getpwd();
        physdir = s;
        if val.is_null() {
            dir = s;
        }
    } else {
        dir = savestr(val);
    }
    if oldcur != dir && oldcur != addr_of!(nullstr) as *mut c_char {
        libc::free(oldcur as *mut c_void);
    }
    curdir = dir;
    INTON();
    setvar(b"PWD\0".as_ptr() as *const c_char, dir, VEXPORT);
}
