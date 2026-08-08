//! Literal port of `src/exec.c` / `src/exec.h`.
//! Rules: `docs/spec/port/src/exec.md`.
//!
//! Translation notes (literal, bug-for-bug):
//!   * C `goto`s are reproduced with Rust labelled blocks and `loop`s.
//!     The block nesting mirrors the *order* of the C labels, so that
//!     `break 'label` is a forward `goto` and fall-through between two
//!     adjacent labels still happens.
//!   * `TRACE(...)` is a no-op unless `DEBUG` is defined; the default C
//!     build compiles it out, so the calls are dropped and left as
//!     comments where the trace text documented something.
//!   * `errno` is read through `__errno_location()`. `main.c` caches
//!     that pointer in `dash_errno` and the caching is reproduced in
//!     `shellmain`, but reads here go straight to libc, which is
//!     behaviourally identical.

use bstr::BString;
use core::ptr::{addr_of, addr_of_mut, null, null_mut};
use libc::{c_char, c_int, c_short, c_uint, c_void, size_t};

use crate::builtins::{builtincmd, BUILTIN_REGULAR, BUILTIN_SPECIAL};
use crate::error::{E_EXEC, INTOFF, INTON};
use crate::memalloc::{ckfree, ckmalloc, stalloc};
use crate::nodes::{funcnode, Node};
use crate::output::{out1, output};

// ---------------------------------------------------------------------
// src/exec.h constants
// ---------------------------------------------------------------------

/* values of cmdtype */
pub const CMDUNKNOWN: c_int = -1; /* no entry in table for command */
pub const CMDNORMAL: c_int = 0; /* command is an executable program */
pub const CMDFUNCTION: c_int = 1; /* command is a shell function */
pub const CMDBUILTIN: c_int = 2; /* command is a shell builtin */

/* action to find_command() */
pub const DO_ERR: c_int = 0x01; /* prints errors */
pub const DO_ABS: c_int = 0x02; /* checks absolute paths */
pub const DO_NOFUNC: c_int = 0x04; /* don't return shell functions, for command */
pub const DO_ALTPATH: c_int = 0x08; /* using alternate path */
pub const DO_REGBLTIN: c_int = 0x10; /* regular built-ins and functions only */

const CMDTABLESIZE: usize = 31; /* should be prime */
const ARB: usize = 1; /* actual size determined at run time */

const _PATH_BSHELL: &[u8] = b"/bin/sh\0";

// ---------------------------------------------------------------------
// src/exec.h types
// ---------------------------------------------------------------------

// [spec:dash:def:exec.cmdentry.param]
#[repr(C)]
#[derive(Clone, Copy)]
pub union param {
    pub index: c_int,
    pub cmd: *const builtincmd,
    /// The C's `struct funcnode *`. `funcnode` is `Rc<Node>` now, and this
    /// entry is still a `ckmalloc`'d C struct, so what is stored is the raw
    /// form: `Rc::into_raw` going in, `Rc::from_raw` coming out. See
    /// `nodes::copyfunc` / `nodes::freefunc`.
    pub func: *const funcnode,
}

// [spec:dash:def:exec.cmdentry]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct cmdentry {
    pub cmdtype: c_int,
    pub u: param,
}

// [spec:dash:def:exec.tblentry]
//
// `cmdname[ARB]` is a flexible array member: `ARB` is 1 and the real
// size is chosen at allocation time, so the name lives in the same
// block as the entry.
#[repr(C)]
pub struct tblentry {
    pub next: *mut tblentry,    /* next entry in hash chain */
    pub param: param,           /* definition of builtin function */
    pub cmdtype: c_short,       /* index identifying command */
    pub rehash: c_char,         /* if set, cd done since entry created */
    pub cmdname: [c_char; ARB], /* name of command */
}

// ---------------------------------------------------------------------
// module globals
// ---------------------------------------------------------------------

static mut cmdtable: [*mut tblentry; CMDTABLESIZE] = [null_mut(); CMDTABLESIZE];
static mut builtinloc: c_int = -1; /* index in path of %builtin, or -1 */

pub static mut pathopt: *const c_char = null(); /* set by padvance */

pub static mut lastcmdentry: *mut *mut tblentry = null_mut();

#[inline]
unsafe fn errno() -> c_int {
    *libc::__errno_location()
}

/* C: #define equal(s1, s2) (strcmp(s1, s2) == 0) */
#[inline]
unsafe fn equal(s1: *const c_char, s2: *const c_char) -> bool {
    libc::strcmp(s1, s2) == 0
}

// ---------------------------------------------------------------------

/*
 * Exec a program.  Never returns.  If you change this routine, you may
 * have to change the find_command routine as well.
 */

// [spec:dash:def:exec.shellexec-fn]
// [spec:dash:sem:exec.shellexec-fn]
pub unsafe fn shellexec(argv: *mut *mut c_char, path: *const c_char, mut idx: c_int) -> ! {
    let mut cmdname: *mut c_char;
    let e: c_int;
    let exerrno: c_int;
    let mut lpath: *const c_char = path;

    /* The C's `environment()` leaves its array in the stack allocator; ours
     * owns it, so the `Vec` has to outlive every `execve` below. */
    let envv = crate::var::environment();
    let envp: *mut *mut c_char = envv.as_ptr() as *mut *mut c_char;
    if !libc::strchr(*argv.offset(0), '/' as c_int).is_null() {
        tryexec(*argv.offset(0), argv, envp);
        e = errno();
    } else {
        let mut se: c_int = libc::ENOENT;
        while padvance(&mut lpath, *argv.offset(0)) >= 0 {
            cmdname = padvance_result();
            idx -= 1;
            if idx < 0 && pathopt.is_null() {
                tryexec(cmdname, argv, envp);
                if errno() != libc::ENOENT && errno() != libc::ENOTDIR {
                    se = errno();
                }
            }
        }
        e = se;
    }

    /* Map to POSIX errors */
    match e {
        libc::ELOOP | libc::ENAMETOOLONG | libc::ENOENT | libc::ENOTDIR => {
            exerrno = 127;
        }
        _ => {
            exerrno = 126;
        }
    }
    crate::eval::exitstatus = exerrno;
    /* TRACE(("shellexec failed for %s, errno %d, suppressint %d\n", ...)); */
    crate::exerror!(
        crate::error::EXEND,
        b"%s: %s\0".as_ptr() as *const c_char,
        *argv.offset(0),
        crate::error::errmsg(e, E_EXEC)
    );
    /* NOTREACHED */
}

// [spec:dash:def:exec.tryexec-fn]
// [spec:dash:sem:exec.tryexec-fn]
unsafe fn tryexec(mut cmd: *mut c_char, mut argv: *mut *mut c_char, envp: *mut *mut c_char) {
    let path_bshell: *mut c_char = _PATH_BSHELL.as_ptr() as *mut c_char;

    loop {
        // repeat:
        libc::execve(
            cmd,
            argv as *const *const c_char,
            envp as *const *const c_char,
        );
        if cmd != path_bshell && errno() == libc::ENOEXEC {
            /* *argv-- = cmd; */
            *argv = cmd;
            argv = argv.offset(-1);
            /* *argv = cmd = path_bshell; */
            cmd = path_bshell;
            *argv = cmd;
            continue; // goto repeat
        }
        break;
    }
}

// [spec:dash:def:exec.legal-pathopt-fn]
// [spec:dash:sem:exec.legal-pathopt-fn]
unsafe fn legal_pathopt(
    mut opt: *const c_char,
    term: *const c_char,
    magic: c_int,
) -> *const c_char {
    match magic {
        0 => {
            opt = null();
        }

        1 => {
            /* GNU `?:` — prefix(opt, "builtin") ?: prefix(opt, "func") */
            let p = crate::mystring::prefix(opt, b"builtin\0".as_ptr() as *const c_char);
            opt = if !p.is_null() {
                p as *const c_char
            } else {
                crate::mystring::prefix(opt, b"func\0".as_ptr() as *const c_char) as *const c_char
            };
        }

        _ => {
            opt = opt.add(libc::strcspn(opt, term));
        }
    }

    if !opt.is_null() && *opt == b'%' as c_char {
        opt = opt.add(1);
    }

    opt
}

/*
 * Do a path search.  The variable path (passed by reference) should be
 * set to the start of the path before the first call; padvance will update
 * this value as it proceeds.  Successive calls to padvance will return
 * the possible path expansions in sequence.  If an option (indicated by
 * a percent sign) appears in the path entry then the global variable
 * pathopt will be set to point to it; otherwise pathopt will be set to
 * NULL.
 *
 * If magic is 0 then pathopt recognition will be disabled.  If magic is
 * 1 we shall recognise %builtin/%func.  Otherwise we shall accept any
 * pathopt.
 */

// [spec:dash:def:exec.padvance-magic-fn]
// [spec:dash:sem:exec.padvance-magic-fn]
pub unsafe fn padvance_magic(path: &mut *const c_char, name: *const c_char, magic: c_int) -> c_int {
    let mut term: *const c_char = b"%:\0".as_ptr() as *const c_char;
    let mut lpathopt: *const c_char;
    let mut p: *const c_char;
    let mut q: *mut c_char;
    let mut start: *const c_char;
    let qlen: size_t;
    let mut len: size_t;

    if (*path).is_null() {
        return -1;
    }

    lpathopt = null();
    start = *path;

    if *start == b'%' as c_char && {
        p = legal_pathopt(start.add(1), term, magic);
        !p.is_null()
    } {
        lpathopt = start.add(1);
        start = p;
        term = b":\0".as_ptr() as *const c_char;
    }

    len = libc::strcspn(start, term);
    p = start.add(len);

    if *p == b'%' as c_char {
        let extra: size_t = libc::strchrnul(p, ':' as c_int) as usize - p as usize;

        if !legal_pathopt(p.add(1), term, magic).is_null() {
            lpathopt = p.add(1);
        } else {
            len += extra;
        }

        p = p.add(extra);
    }

    pathopt = lpathopt;
    *path = if *p == b':' as c_char {
        p.add(1)
    } else {
        null()
    };

    /* "2" is for '/' and '\0' */
    qlen = len + libc::strlen(name) + 2;
    let buf = &mut *addr_of_mut!(pathbuf);
    buf.clear();
    buf.reserve(qlen);

    if len != 0 {
        buf.extend_from_slice(core::slice::from_raw_parts(start as *const u8, len));
        buf.push(b'/');
    }
    q = buf.as_mut_ptr().add(buf.len()) as *mut c_char;
    libc::strcpy(q, name);
    /* `strcpy` wrote the name and its terminator into the reserved tail;
     * `qlen` is what the C's `growstackto` guaranteed room for, and it is
     * one more than the bytes written when `len` is zero. */
    let n = buf.len() + libc::strlen(q) + 1;
    buf.set_len(n);

    qlen as c_int
}

/// The candidate path [`padvance_magic`] builds.
///
/// The C builds it at `stackblock()` and hands the caller the *length* it
/// reserved room for, so a caller that wants to keep the candidate calls
/// `stalloc(len)` to take exactly that block. That is why `len` is the
/// return value and not the string: it is an allocation size, not a
/// strlen, and it is two larger than the path when the path component is
/// empty. Callers that kept the candidate now copy it instead.
static mut pathbuf: BString = BString::new(Vec::new());

/// The candidate path the last `padvance` built, as a C string.
pub unsafe fn padvance_result() -> *mut c_char {
    (*addr_of_mut!(pathbuf)).as_mut_ptr() as *mut c_char
}

// [spec:dash:def:exec.padvance-fn]
// [spec:dash:sem:exec.padvance-fn]
#[inline]
pub unsafe fn padvance(path: &mut *const c_char, name: *const c_char) -> c_int {
    padvance_magic(path, name, 1)
}

/*** Command hashing code ***/

// [spec:dash:def:exec.hashcmd-fn]
// [spec:dash:sem:exec.hashcmd-fn]
pub unsafe fn hashcmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    let mut pp: *mut *mut tblentry;
    let mut cmdp: *mut tblentry;
    let mut c: c_int;
    let mut entry: cmdentry = cmdentry {
        cmdtype: 0,
        u: param { index: 0 },
    };
    let mut name: *mut c_char;
    let mut clear: bool;

    clear = false;
    loop {
        c = crate::options::nextopt(b"r\0".as_ptr() as *const c_char);
        if c == 0 {
            break;
        }
        clear = true;
    }
    if clear {
        clearcmdentry();
        return 0;
    }

    if (*crate::options::argptr).is_null() {
        pp = (addr_of_mut!(cmdtable) as *mut *mut tblentry);
        while pp < (addr_of_mut!(cmdtable) as *mut *mut tblentry).add(CMDTABLESIZE) {
            cmdp = *pp;
            while !cmdp.is_null() {
                if (*cmdp).cmdtype as c_int == CMDNORMAL {
                    printentry(cmdp);
                }
                cmdp = (*cmdp).next;
            }
            pp = pp.add(1);
        }
        return 0;
    }
    c = 0;
    loop {
        name = *crate::options::argptr;
        if name.is_null() {
            break;
        }
        cmdp = cmdlookup(name, 0);
        if !cmdp.is_null()
            && ((*cmdp).cmdtype as c_int == CMDNORMAL
                || ((*cmdp).cmdtype as c_int == CMDBUILTIN
                    && ((*(*cmdp).param.cmd).flags & BUILTIN_REGULAR) == 0
                    && builtinloc > 0))
        {
            delete_cmd_entry();
        }
        find_command(name, &mut entry, DO_ERR, crate::var::pathval());
        if entry.cmdtype == CMDUNKNOWN {
            c = 1;
        }
        crate::options::argptr = crate::options::argptr.add(1);
    }
    c
}

// [spec:dash:def:exec.printentry-fn]
// [spec:dash:sem:exec.printentry-fn]
unsafe fn printentry(cmdp: *mut tblentry) {
    let mut idx: c_int;
    let mut path: *const c_char;
    let name: *mut c_char;

    idx = (*cmdp).param.index;
    path = crate::var::pathval();
    loop {
        padvance(&mut path, (*cmdp).cmdname.as_ptr());
        idx -= 1;
        if idx < 0 {
            break;
        }
    }
    name = padvance_result();
    crate::output::out1str(name);
    crate::out1fmt!(
        (core::ptr::addr_of!(crate::mystring::snlfmt) as *const c_char),
        if (*cmdp).rehash != 0 {
            b"*\0".as_ptr() as *const c_char
        } else {
            (core::ptr::addr_of!(crate::shell::nullstr) as *const c_char)
        }
    );
}

// [spec:dash:def:exec.test-exec-fn]
// [spec:dash:sem:exec.test-exec-fn]
unsafe fn test_exec(fullname: *const c_char, statb: *mut libc::stat64) -> c_int {
    if ((*statb).st_mode & libc::S_IFMT) != libc::S_IFREG {
        return 0;
    }

    if ((*statb).st_mode & 0o111) != 0o111 &&
        /* HAVE_FACCESSAT; the non-faccessat build uses test_access(statb, X_OK) */
        test_file_access(fullname, libc::X_OK) == 0
    {
        return 0;
    }

    1
}

/*
 * Resolve a command name.  If you change this routine, you may have to
 * change the shellexec routine as well.
 */

// [spec:dash:def:exec.find-command-fn]
// [spec:dash:sem:exec.find-command-fn]
pub unsafe fn find_command(
    name: *mut c_char,
    entry: *mut cmdentry,
    mut act: c_int,
    path: *const c_char,
) {
    let mut cmdp: *mut tblentry;
    let mut idx: c_int;
    let mut prev: c_int;
    let mut fullname: *mut c_char;
    let mut statb: libc::stat64 = core::mem::zeroed();
    let mut e: c_int;
    let mut updatetbl: c_int;
    let mut bcmd: *const builtincmd;
    let mut len: c_int;
    let mut lpath: *const c_char = path;

    /* If name contains a slash, don't use PATH or hash table */
    if !libc::strchr(name, '/' as c_int).is_null() {
        (*entry).u.index = -1;
        'absdone: {
            if (act & DO_ABS) != 0 {
                'absfail: {
                    while libc::stat64(name, &mut statb) < 0 {
                        /* SYSV: retry on EINTR */
                        break 'absfail;
                    }
                    if test_exec(name, &mut statb) == 0 {
                        break 'absfail;
                    }
                    break 'absdone;
                }
                // absfail:
                (*entry).cmdtype = CMDUNKNOWN;
                return;
            }
        }
        (*entry).cmdtype = CMDNORMAL;
        return;
    }

    updatetbl = (path == crate::var::pathval()) as c_int;
    if updatetbl == 0 {
        act |= DO_ALTPATH;
    }

    bcmd = null();

    'success: {
        'builtin_success: {
            'fail: {
                /* If name is in the table, check answer will be ok */
                cmdp = cmdlookup(name, 0);
                if !cmdp.is_null() {
                    let bit: c_int;

                    match (*cmdp).cmdtype as c_int {
                        CMDFUNCTION => {
                            bit = DO_NOFUNC;
                        }
                        CMDBUILTIN => {
                            bit = if ((*(*cmdp).param.cmd).flags & BUILTIN_REGULAR) != 0 {
                                0
                            } else {
                                DO_REGBLTIN
                            };
                        }
                        /* `default:` (DEBUG: abort()) falls through to CMDNORMAL */
                        _ => {
                            bit = DO_ALTPATH | DO_REGBLTIN;
                        }
                    }
                    if (act & bit) != 0 {
                        if (act & bit & DO_REGBLTIN) != 0 {
                            break 'fail;
                        }

                        updatetbl = 0;
                        cmdp = null_mut();
                    } else if (*cmdp).rehash == 0 {
                        /* if not invalidated by cd, we're done */
                        break 'success;
                    }
                }

                /* If %builtin not in path, check for builtin next */
                bcmd = find_builtin(name);
                if !bcmd.is_null()
                    && ((((*bcmd).flags & BUILTIN_REGULAR) as c_int)
                        | (act & DO_ALTPATH)
                        | ((builtinloc <= 0) as c_int))
                        != 0
                {
                    break 'builtin_success;
                }

                if (act & DO_REGBLTIN) != 0 {
                    break 'fail;
                }

                /* We have to search path. */
                prev = -1; /* where to start */
                if !cmdp.is_null() && (*cmdp).rehash != 0 {
                    /* doing a rehash */
                    if (*cmdp).cmdtype as c_int == CMDBUILTIN {
                        prev = builtinloc;
                    } else {
                        prev = (*cmdp).param.index;
                    }
                }

                e = libc::ENOENT;
                idx = -1;
                'padvloop: loop {
                    // loop:
                    len = padvance(&mut lpath, name);
                    if len < 0 {
                        break 'padvloop;
                    }
                    let lpathopt: *const c_char = pathopt;

                    fullname = padvance_result();
                    idx += 1;
                    if !lpathopt.is_null() {
                        if *lpathopt == b'b' as c_char {
                            if !bcmd.is_null() {
                                break 'builtin_success;
                            }
                            continue 'padvloop;
                        } else if (act & DO_NOFUNC) == 0 {
                            /* handled below */
                        } else {
                            /* ignore unimplemented options */
                            continue 'padvloop;
                        }
                    }
                    /* if rehash, don't redo absolute path names */
                    if *fullname.offset(0) == b'/' as c_char && idx <= prev {
                        if idx < prev {
                            continue 'padvloop;
                        }
                        /* TRACE(("searchexec \"%s\": no change\n", name)); */
                        break 'success;
                    }
                    loop {
                        if libc::stat64(fullname, &mut statb) >= 0 {
                            break;
                        }
                        /* SYSV: retry on EINTR */
                        if errno() != libc::ENOENT && errno() != libc::ENOTDIR {
                            e = errno();
                        }
                        continue 'padvloop; // goto loop
                    }
                    if !lpathopt.is_null() {
                        /* this is a %func directory */
                        /* `stalloc(len)` took the candidate out of the way
                         * because `readcmdfile` runs shell code that can
                         * search the path again; the copy is what keeps it,
                         * and `stunalloc` is the copy going out of scope. */
                        let kept = (*addr_of!(pathbuf)).clone();
                        let fullname = kept.as_ptr() as *mut c_char;
                        crate::shellmain::readcmdfile(fullname);
                        cmdp = cmdlookup(name, 0);
                        if cmdp.is_null() || (*cmdp).cmdtype as c_int != CMDFUNCTION {
                            crate::sh_error!(
                                b"%s not defined in %s\0".as_ptr() as *const c_char,
                                name,
                                fullname
                            );
                        }
                        break 'success;
                    }
                    e = libc::EACCES; /* if we fail, this will be the error */
                    if test_exec(fullname, &mut statb) == 0 {
                        continue 'padvloop;
                    }
                    /* TRACE(("searchexec \"%s\" returns \"%s\"\n", name, fullname)); */
                    if updatetbl == 0 {
                        (*entry).cmdtype = CMDNORMAL;
                        (*entry).u.index = idx;
                        return;
                    }
                    INTOFF();
                    cmdp = cmdlookup(name, 1);
                    (*cmdp).cmdtype = CMDNORMAL as c_short;
                    (*cmdp).param.index = idx;
                    INTON();
                    break 'success;
                }

                /* We failed.  If there was an entry for this command, delete it */
                if !cmdp.is_null() && updatetbl != 0 {
                    delete_cmd_entry();
                }
                if (act & DO_ERR) != 0 {
                    crate::sh_warnx!(
                        b"%s: %s\0".as_ptr() as *const c_char,
                        name,
                        crate::error::errmsg(e, E_EXEC)
                    );
                }
                // fall through into fail:
            }
            // fail:
            (*entry).cmdtype = CMDUNKNOWN;
            return;
        }
        // builtin_success:
        if updatetbl == 0 {
            (*entry).cmdtype = CMDBUILTIN;
            (*entry).u.cmd = bcmd;
            return;
        }
        INTOFF();
        cmdp = cmdlookup(name, 1);
        (*cmdp).cmdtype = CMDBUILTIN as c_short;
        (*cmdp).param.cmd = bcmd;
        INTON();
        // fall through into success:
    }
    // success:
    (*cmdp).rehash = 0;
    (*entry).cmdtype = (*cmdp).cmdtype as c_int;
    (*entry).u = (*cmdp).param;
}

/*
 * Search the table of builtin commands.
 */

// [spec:dash:def:exec.find-builtin-fn]
// [spec:dash:sem:exec.find-builtin-fn]
pub unsafe fn find_builtin(name: *const c_char) -> *const builtincmd {
    let bp: *const builtincmd;

    bp = libc::bsearch(
        &name as *const *const c_char as *const c_void,
        crate::builtins::builtincmd.as_ptr() as *const c_void,
        crate::builtins::NUMBUILTINS as size_t,
        core::mem::size_of::<builtincmd>() as size_t,
        Some(crate::mystring::pstrcmp),
    ) as *const builtincmd;
    bp
}

/*
 * Called when a cd is done.  Marks all commands so the next time they
 * are executed they will be rehashed.
 */

// [spec:dash:def:exec.hashcd-fn]
// [spec:dash:sem:exec.hashcd-fn]
pub unsafe fn hashcd() {
    let mut pp: *mut *mut tblentry;
    let mut cmdp: *mut tblentry;

    pp = (addr_of_mut!(cmdtable) as *mut *mut tblentry);
    while pp < (addr_of_mut!(cmdtable) as *mut *mut tblentry).add(CMDTABLESIZE) {
        cmdp = *pp;
        while !cmdp.is_null() {
            if (*cmdp).cmdtype as c_int == CMDNORMAL
                || ((*cmdp).cmdtype as c_int == CMDBUILTIN
                    && ((*(*cmdp).param.cmd).flags & BUILTIN_REGULAR) == 0
                    && builtinloc > 0)
            {
                (*cmdp).rehash = 1;
            }
            cmdp = (*cmdp).next;
        }
        pp = pp.add(1);
    }
}

/*
 * Fix command hash table when PATH changed.
 * Called before PATH is changed.  The argument is the new value of PATH;
 * pathval() still returns the old value at this point.
 * Called with interrupts off.
 */

// [spec:dash:def:exec.changepath-fn]
// [spec:dash:sem:exec.changepath-fn]
pub unsafe fn changepath(newval: *const c_char) {
    let mut new: *const c_char;
    let mut idx: c_int;
    let mut bltin: c_int;

    new = newval;
    idx = 0;
    bltin = -1;
    loop {
        if *new == b'%' as c_char
            && !crate::mystring::prefix(new.add(1), b"builtin\0".as_ptr() as *const c_char)
                .is_null()
        {
            bltin = idx;
            break;
        }
        new = libc::strchr(new, ':' as c_int);
        if new.is_null() {
            break;
        }
        idx += 1;
        new = new.add(1);
    }
    builtinloc = bltin;
    clearcmdentry();
}

/*
 * Clear out command entries.  The argument specifies the first entry in
 * PATH which has changed.
 */

// [spec:dash:def:exec.clearcmdentry-fn]
// [spec:dash:sem:exec.clearcmdentry-fn]
unsafe fn clearcmdentry() {
    let mut tblp: *mut *mut tblentry;
    let mut pp: *mut *mut tblentry;
    let mut cmdp: *mut tblentry;

    INTOFF();
    tblp = (addr_of_mut!(cmdtable) as *mut *mut tblentry);
    while tblp < (addr_of_mut!(cmdtable) as *mut *mut tblentry).add(CMDTABLESIZE) {
        pp = tblp;
        loop {
            cmdp = *pp;
            if cmdp.is_null() {
                break;
            }
            if (*cmdp).cmdtype as c_int == CMDNORMAL
                || ((*cmdp).cmdtype as c_int == CMDBUILTIN
                    && ((*(*cmdp).param.cmd).flags & BUILTIN_REGULAR) == 0
                    && builtinloc > 0)
            {
                *pp = (*cmdp).next;
                ckfree(cmdp as *mut c_void);
            } else {
                pp = &mut (*cmdp).next;
            }
        }
        tblp = tblp.add(1);
    }
    INTON();
}

/*
 * Locate a command in the command hash table.  If "add" is nonzero,
 * add the command to the table if it is not already present.  The
 * variable "lastcmdentry" is set to point to the address of the link
 * pointing to the entry, so that delete_cmd_entry can delete the
 * entry.
 *
 * Interrupts must be off if called with add != 0.
 */

// [spec:dash:def:exec.cmdlookup-fn]
// [spec:dash:sem:exec.cmdlookup-fn]
unsafe fn cmdlookup(name: *const c_char, add: c_int) -> *mut tblentry {
    let mut hashval: c_uint;
    let mut p: *const c_char;
    let mut cmdp: *mut tblentry;
    let mut pp: *mut *mut tblentry;

    p = name;
    hashval = ((*p as libc::c_uchar) as c_uint) << 4;
    while *p != 0 {
        hashval = hashval.wrapping_add((*p as libc::c_uchar) as c_uint);
        p = p.add(1);
    }
    hashval &= 0x7FFF;
    pp = (addr_of_mut!(cmdtable) as *mut *mut tblentry).add((hashval as usize) % CMDTABLESIZE);
    cmdp = *pp;
    while !cmdp.is_null() {
        if equal((*cmdp).cmdname.as_ptr(), name) {
            break;
        }
        pp = &mut (*cmdp).next;
        cmdp = *pp;
    }
    if add != 0 && cmdp.is_null() {
        cmdp = ckmalloc(core::mem::size_of::<tblentry>() - ARB + libc::strlen(name) + 1)
            as *mut tblentry;
        *pp = cmdp;
        (*cmdp).next = null_mut();
        (*cmdp).cmdtype = CMDUNKNOWN as c_short;
        libc::strcpy((*cmdp).cmdname.as_mut_ptr(), name);
    }
    lastcmdentry = pp;
    cmdp
}

/*
 * Delete the command entry returned on the last lookup.
 */

// [spec:dash:def:exec.delete-cmd-entry-fn]
// [spec:dash:sem:exec.delete-cmd-entry-fn]
unsafe fn delete_cmd_entry() {
    let cmdp: *mut tblentry;

    INTOFF();
    cmdp = *lastcmdentry;
    *lastcmdentry = (*cmdp).next;
    if (*cmdp).cmdtype as c_int == CMDFUNCTION {
        crate::nodes::freefunc((*cmdp).param.func);
    }
    ckfree(cmdp as *mut c_void);
    INTON();
}

// [spec:dash:def:exec.getcmdentry-fn]
// [spec:dash:sem:exec.getcmdentry-fn]
//
// The whole function lives inside `#ifdef notdef` in `src/exec.c`
// (lines 698-712) and is not compiled into the shell. It is carried
// here as an annotated, never-compiled stub so the manifest symbol has
// a target site; `#[cfg(any())]` is the Rust equivalent of the
// unsatisfiable `#ifdef notdef` guard, and the body is the literal
// translation of the dead C.
#[cfg(any())]
pub unsafe fn getcmdentry(name: *mut c_char, entry: *mut cmdentry) {
    let cmdp: *mut tblentry = cmdlookup(name, 0);

    if !cmdp.is_null() {
        (*entry).u = (*cmdp).param;
        (*entry).cmdtype = (*cmdp).cmdtype as c_int;
    } else {
        (*entry).cmdtype = CMDUNKNOWN;
        (*entry).u.index = 0;
    }
}

/*
 * Add a new command entry, replacing any existing command entry for
 * the same name - except special builtins.
 */

// [spec:dash:def:exec.addcmdentry-fn]
// [spec:dash:sem:exec.addcmdentry-fn]
unsafe fn addcmdentry(name: *mut c_char, entry: *mut cmdentry) {
    let cmdp: *mut tblentry;

    cmdp = cmdlookup(name, 1);
    if (*cmdp).cmdtype as c_int == CMDFUNCTION {
        crate::nodes::freefunc((*cmdp).param.func);
    }
    (*cmdp).cmdtype = (*entry).cmdtype as c_short;
    (*cmdp).param = (*entry).u;
    (*cmdp).rehash = 0;
}

/*
 * Define a shell function.
 */

// [spec:dash:def:exec.defun-fn]
// [spec:dash:sem:exec.defun-fn]
pub unsafe fn defun(func: &Node) {
    let mut entry: cmdentry = cmdentry {
        cmdtype: 0,
        u: param { index: 0 },
    };

    INTOFF();
    entry.cmdtype = CMDFUNCTION;
    entry.u.func = crate::nodes::copyfunc(func);
    addcmdentry(func.ndefun().text.as_ptr(), &mut entry);
    INTON();
}

/*
 * Delete a function if it exists.
 */

// [spec:dash:def:exec.unsetfunc-fn]
// [spec:dash:sem:exec.unsetfunc-fn]
pub unsafe fn unsetfunc(name: *const c_char) {
    let cmdp: *mut tblentry;

    cmdp = cmdlookup(name, 0);
    if !cmdp.is_null() && (*cmdp).cmdtype as c_int == CMDFUNCTION {
        delete_cmd_entry();
    }
}

/*
 * Locate and print what a word is...
 */

// [spec:dash:def:exec.typecmd-fn]
// [spec:dash:sem:exec.typecmd-fn]
pub unsafe fn typecmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    let mut err: c_int = 0;

    crate::options::nextopt((core::ptr::addr_of!(crate::shell::nullstr) as *const c_char));
    while !(*crate::options::argptr).is_null() {
        let p = *crate::options::argptr;
        crate::options::argptr = crate::options::argptr.add(1);
        err |= describe_command(out1, p, null(), 1);
    }
    err
}

// [spec:dash:def:exec.describe-command-fn]
// [spec:dash:sem:exec.describe-command-fn]
unsafe fn describe_command(
    out: *mut output,
    command: *mut c_char,
    mut path: *const c_char,
    verbose: c_int,
) -> c_int {
    let mut entry: cmdentry = cmdentry {
        cmdtype: 0,
        u: param { index: 0 },
    };
    let cmdp: *mut tblentry;
    let ap: *const crate::alias::alias;

    'out_label: {
        if verbose != 0 {
            crate::output::outstr(command, out);
        }

        /* First look at the keywords */
        if !crate::parser::findkwd(command).is_null() {
            crate::output::outstr(
                if verbose != 0 {
                    b" is a shell keyword\0".as_ptr() as *const c_char
                } else {
                    command as *const c_char
                },
                out,
            );
            break 'out_label;
        }

        /* Then look at the aliases */
        ap = crate::alias::lookupalias(command, 0);
        if !ap.is_null() {
            if verbose != 0 {
                crate::outfmt!(
                    out,
                    b" is an alias for %s\0".as_ptr() as *const c_char,
                    (*ap).val
                );
            } else {
                crate::output::outstr(b"alias \0".as_ptr() as *const c_char, out);
                crate::alias::printalias(ap);
                return 0;
            }
            break 'out_label;
        }

        /* Then if the standard search path is used, check if it is
         * a tracked alias.
         */
        if path.is_null() {
            path = crate::var::pathval();
            cmdp = cmdlookup(command, 0);
        } else {
            cmdp = null_mut();
        }

        if !cmdp.is_null() {
            entry.cmdtype = (*cmdp).cmdtype as c_int;
            entry.u = (*cmdp).param;
        } else {
            /* Finally use brute force */
            find_command(command, &mut entry, DO_ABS, path);
        }

        match entry.cmdtype {
            CMDNORMAL => {
                let mut j: c_int = entry.u.index;
                let p: *mut c_char;
                if j == -1 {
                    p = command;
                } else {
                    loop {
                        padvance(&mut path, command);
                        j -= 1;
                        if j < 0 {
                            break;
                        }
                    }
                    p = padvance_result();
                }
                if verbose != 0 {
                    crate::outfmt!(
                        out,
                        b" is%s %s\0".as_ptr() as *const c_char,
                        if !cmdp.is_null() {
                            b" a tracked alias for\0".as_ptr() as *const c_char
                        } else {
                            (core::ptr::addr_of!(crate::shell::nullstr) as *const c_char)
                        },
                        p
                    );
                } else {
                    crate::output::outstr(p, out);
                }
            }

            CMDFUNCTION => {
                if verbose != 0 {
                    crate::output::outstr(b" is a shell function\0".as_ptr() as *const c_char, out);
                } else {
                    crate::output::outstr(command, out);
                }
            }

            CMDBUILTIN => {
                if verbose != 0 {
                    crate::outfmt!(
                        out,
                        b" is a %sshell builtin\0".as_ptr() as *const c_char,
                        if ((*entry.u.cmd).flags & BUILTIN_SPECIAL) != 0 {
                            b"special \0".as_ptr() as *const c_char
                        } else {
                            (core::ptr::addr_of!(crate::shell::nullstr) as *const c_char)
                        }
                    );
                } else {
                    crate::output::outstr(command, out);
                }
            }

            _ => {
                if verbose != 0 {
                    crate::output::outstr(b": not found\n\0".as_ptr() as *const c_char, out);
                }
                return 127;
            }
        }
    }
    // out:
    crate::output::outc('\n' as c_int, out);
    0
}

// [spec:dash:def:exec.commandcmd-fn]
// [spec:dash:sem:exec.commandcmd-fn]
pub unsafe fn commandcmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    let cmd: *mut c_char;
    let mut c: c_int;
    const VERIFY_BRIEF: c_int = 1;
    const VERIFY_VERBOSE: c_int = 2;
    let mut verify: c_int = 0;
    let mut path: *const c_char = null();

    loop {
        c = crate::options::nextopt(b"pvV\0".as_ptr() as *const c_char);
        if c == 0 {
            break;
        }
        if c == 'V' as c_int {
            verify |= VERIFY_VERBOSE;
        } else if c == 'v' as c_int {
            verify |= VERIFY_BRIEF;
        } else {
            /* DEBUG: `else if (c != 'p') abort();` */
            path = crate::var::defpath();
        }
    }

    cmd = *crate::options::argptr;
    if verify != 0 && !cmd.is_null() {
        return describe_command(out1, cmd, path, verify - VERIFY_BRIEF);
    }

    0
}

// ---------------------------------------------------------------------
// src/exec.h declarations whose definitions live in src/bltin/test.c.
//
// The manifest attributes these two symbols to `src/exec.h` (the
// declaration site), while the bodies are owned by the `test.*` rules
// in `src/bltin/test.c`. Rust has no separate declaration form, so the
// `exec.*` annotation is carried on a forwarding wrapper.
// ---------------------------------------------------------------------

// [spec:dash:def:exec.test-file-access-fn]
// [spec:dash:sem:exec.test-file-access-fn]
#[inline]
pub unsafe fn test_file_access(path: *const c_char, mode: c_int) -> c_int {
    crate::bltin::test::test_file_access(path, mode)
}

// [spec:dash:def:exec.test-access-fn]
// [spec:dash:sem:exec.test-access-fn]
#[inline]
pub unsafe fn test_access(sp: *const libc::stat64, stmode: c_int) -> c_int {
    crate::bltin::test::test_access(sp, stmode)
}
