//! Literal port of `src/options.c` / `src/options.h`.
//! Rules: `docs/spec/port/src/options.md`.
//!
//! `optlist`, `optnames` and `optletters` are three parallel views of the same
//! option and **must stay in the same order**.  The `eflag`/`fflag`/… names of
//! `options.h` become `usize` indices, so a call site reads `optlist[eflag]`
//! and stays assignable exactly like the C macro.

use libc::{c_char, c_int, c_uint, c_void, size_t};
use core::ptr::{addr_of, addr_of_mut, null_mut};

use crate::error::{INTOFF, INTON};
use crate::memalloc::{ckfree, ckmalloc, savestr};
use crate::mystring::nullstr;
use crate::output::VaArg;
use crate::shell::cstr;
use crate::var::{setvar, setvarint, showvars, VNOFUNC, VUNSET};

// [spec:dash:def:options.shparam]
#[repr(C)]
#[derive(Copy, Clone)]
pub struct shparam {
    pub nparam: c_int,        /* # of positional parameters (without $0) */
    pub malloc: libc::c_uchar, /* if parameter list dynamically allocated */
    pub p: *mut *mut c_char,  /* parameter list */
    pub optind: c_int,        /* next parameter to be processed by getopts */
    pub optoff: c_int,        /* used by getopts */
}

pub const NOPTS: usize = 18;

/*
 * options.h spells these `#define eflag optlist[0]` etc.  The port keeps the
 * names as the *index* into `optlist`, so a call site reads
 * `optlist[iflag]` — assignable exactly like the C macro.
 */
pub const eflag: usize = 0;
pub const fflag: usize = 1;
pub const Iflag: usize = 2;
pub const iflag: usize = 3;
pub const mflag: usize = 4;
pub const nflag: usize = 5;
pub const sflag: usize = 6;
pub const xflag: usize = 7;
pub const vflag: usize = 8;
pub const Vflag: usize = 9;
pub const Eflag: usize = 10;
pub const Cflag: usize = 11;
pub const aflag: usize = 12;
pub const bflag: usize = 13;
pub const uflag: usize = 14;
pub const nolog: usize = 15;
pub const pipefail: usize = 16;
pub const debug: usize = 17;

pub static mut arg0: *mut c_char = null_mut(); /* value of $0 */
pub static mut shellparam: shparam = shparam {
    nparam: 0,
    malloc: 0,
    p: null_mut(),
    optind: 0,
    optoff: 0,
}; /* current positional parameters */
pub static mut argptr: *mut *mut c_char = null_mut(); /* argument list for builtin commands */
pub static mut optionarg: *mut c_char = null_mut(); /* set by nextopt (like getopt) */
pub static mut optptr: *mut c_char = null_mut(); /* used by nextopt */

pub static mut minusc: *mut c_char = null_mut(); /* argument to -c option */

/* `static const char *const optnames[NOPTS]` — `static mut` in Rust only
 * because a `*const c_char` array is not `Sync`; it is never written. */
static mut optnames: [*const c_char; NOPTS] = [
    b"errexit\0".as_ptr() as *const c_char,
    b"noglob\0".as_ptr() as *const c_char,
    b"ignoreeof\0".as_ptr() as *const c_char,
    b"interactive\0".as_ptr() as *const c_char,
    b"monitor\0".as_ptr() as *const c_char,
    b"noexec\0".as_ptr() as *const c_char,
    b"stdin\0".as_ptr() as *const c_char,
    b"xtrace\0".as_ptr() as *const c_char,
    b"verbose\0".as_ptr() as *const c_char,
    b"vi\0".as_ptr() as *const c_char,
    b"emacs\0".as_ptr() as *const c_char,
    b"noclobber\0".as_ptr() as *const c_char,
    b"allexport\0".as_ptr() as *const c_char,
    b"notify\0".as_ptr() as *const c_char,
    b"nounset\0".as_ptr() as *const c_char,
    b"nolog\0".as_ptr() as *const c_char,
    b"pipefail\0".as_ptr() as *const c_char,
    b"debug\0".as_ptr() as *const c_char,
];

pub static optletters: [c_char; NOPTS] = [
    b'e' as c_char,
    b'f' as c_char,
    b'I' as c_char,
    b'i' as c_char,
    b'm' as c_char,
    b'n' as c_char,
    b's' as c_char,
    b'x' as c_char,
    b'v' as c_char,
    b'V' as c_char,
    b'E' as c_char,
    b'C' as c_char,
    b'a' as c_char,
    b'b' as c_char,
    b'u' as c_char,
    0,
    0,
    0,
];

pub static mut optlist: [c_char; NOPTS] = [0; NOPTS];

/*
 * Process the shell command line arguments.
 */

// [spec:dash:def:options.procargs-fn]
// [spec:dash:sem:options.procargs-fn]
pub unsafe fn procargs(mut xargv: *mut *mut c_char) -> c_int {
    let mut i: c_int;
    let mut login: c_int;

    login = (!(*xargv.offset(0)).is_null() && **xargv.offset(0) == b'-' as c_char) as c_int;
    arg0 = *xargv.offset(0);
    if !(*xargv.offset(0)).is_null() {
        xargv = xargv.add(1);
    }
    i = 0;
    while i < NOPTS as c_int {
        optlist[i as usize] = 2;
        i += 1;
    }
    argptr = xargv;
    login |= options(1);
    xargv = argptr;
    if (*xargv).is_null() {
        if !minusc.is_null() {
            crate::error::sh_error(cstr(b"-c requires an argument\0"), &[]);
        }
        optlist[sflag] = 1;
    }
    if optlist[iflag] == 2 && optlist[sflag] == 1 {
        crate::input::input_init();
        if crate::input::stdin_istty != 0 && libc::isatty(2) != 0 {
            optlist[iflag] = 1;
        }
    }
    if optlist[mflag] == 2 {
        optlist[mflag] = optlist[iflag];
    }
    i = 0;
    while i < NOPTS as c_int {
        if optlist[i as usize] == 2 {
            optlist[i as usize] = 0;
        }
        i += 1;
    }
    /* #if DEBUG == 2 — not selected in this configuration:
     *     debug = 1;
     */
    /* POSIX 1003.2: first arg after -c cmd is $0, remainder $1... */
    let mut setarg0 = false;
    if !minusc.is_null() {
        minusc = *xargv;
        xargv = xargv.add(1);
        if !(*xargv).is_null() {
            setarg0 = true; /* goto setarg0 */
        }
    } else if optlist[sflag] == 0 {
        crate::input::setinputfile(*xargv, 0);
        setarg0 = true;
    }
    if setarg0 {
        arg0 = *xargv;
        xargv = xargv.add(1);
    }

    shellparam.p = xargv;
    shellparam.optind = 1;
    shellparam.optoff = -1;
    /* assert(shellparam.malloc == 0 && shellparam.nparam == 0); */
    while !(*xargv).is_null() {
        shellparam.nparam += 1;
        xargv = xargv.add(1);
    }
    optschanged();

    login
}

// [spec:dash:def:options.optschanged-fn]
// [spec:dash:sem:options.optschanged-fn]
pub unsafe fn optschanged() {
    if crate::shell::DEBUG {
        crate::show::opentrace();
    }
    crate::trap::setinteractive(optlist[iflag] as c_int);
    /* #ifndef SMALL */
    crate::histedit::histedit();
    crate::jobs::setjobctl(optlist[mflag] as c_int);
}

/*
 * Process shell options.  The global variable argptr contains a pointer
 * to the argument list; we advance it past the options.
 */

// [spec:dash:def:options.options-fn]
// [spec:dash:sem:options.options-fn]
unsafe fn options(cmdline: c_int) -> c_int {
    let mut p: *mut c_char;
    let mut val: c_int = 0;
    let mut c: c_int;
    let mut login: c_int = 0;

    if cmdline != 0 {
        minusc = null_mut();
    }
    loop {
        p = *argptr;
        if p.is_null() {
            break;
        }
        argptr = argptr.add(1);
        c = *p as c_int;
        p = p.add(1);
        if c == b'-' as c_int {
            val = 1;
            if *p.offset(0) == b'\0' as c_char
                || (*p.offset(0) == b'-' as c_char && *p.offset(1) == b'\0' as c_char)
            {
                if cmdline == 0 {
                    /* "-" means turn off -x and -v */
                    if *p.offset(0) == b'\0' as c_char {
                        optlist[vflag] = 0;
                        optlist[xflag] = optlist[vflag];
                    }
                    /* "--" means reset params */
                    else if (*argptr).is_null() {
                        setparam(argptr);
                    }
                }
                break; /* "-" or "--" terminates options */
            }
        } else if c == b'+' as c_int {
            val = 0;
        } else {
            argptr = argptr.offset(-1);
            break;
        }
        loop {
            c = *p as c_int;
            p = p.add(1);
            if c == b'\0' as c_int {
                break;
            }
            if c == b'c' as c_int && cmdline != 0 {
                minusc = p; /* command is after shell args */
            } else if c == b'l' as c_int && cmdline != 0 {
                login = 1;
            } else if c == b'o' as c_int {
                minus_o(*argptr, val);
                if !(*argptr).is_null() {
                    argptr = argptr.add(1);
                }
            } else {
                setoption(c, val);
            }
        }
    }

    login
}

// [spec:dash:def:options.minus-o-fn]
// [spec:dash:sem:options.minus-o-fn]
unsafe fn minus_o(name: *mut c_char, val: c_int) {
    let mut i: c_int;

    if name.is_null() {
        if val != 0 {
            crate::output::out1str(b"Current option settings\n\0".as_ptr() as *const c_char);
            i = 0;
            while i < NOPTS as c_int {
                crate::output::out1fmt(
                    cstr(b"%-16s%s\n\0"),
                    &[
                        VaArg::Str(optnames[i as usize]),
                        VaArg::Str(if optlist[i as usize] != 0 {
                            cstr(b"on\0")
                        } else {
                            cstr(b"off\0")
                        }),
                    ],
                );
                i += 1;
            }
        } else {
            i = 0;
            while i < NOPTS as c_int {
                crate::output::out1fmt(
                    cstr(b"set %s %s\n\0"),
                    &[
                        VaArg::Str(if optlist[i as usize] != 0 {
                            cstr(b"-o\0")
                        } else {
                            cstr(b"+o\0")
                        }),
                        VaArg::Str(optnames[i as usize]),
                    ],
                );
                i += 1;
            }
        }
    } else {
        i = 0;
        while i < NOPTS as c_int {
            if libc::strcmp(name, optnames[i as usize]) == 0 {
                optlist[i as usize] = val as c_char;
                return;
            }
            i += 1;
        }
        crate::error::sh_error(cstr(b"Illegal option -o %s\0"), &[VaArg::Str(name)]);
    }
}

// [spec:dash:def:options.setoption-fn]
// [spec:dash:sem:options.setoption-fn]
unsafe fn setoption(flag: c_int, val: c_int) {
    let mut i: c_int;

    i = 0;
    while i < NOPTS as c_int {
        if optletters[i as usize] as c_int == flag {
            optlist[i as usize] = val as c_char;
            if val != 0 {
                /* #%$ hack for ksh semantics */
                if flag == b'V' as c_int {
                    optlist[Eflag] = 0;
                } else if flag == b'E' as c_int {
                    optlist[Vflag] = 0;
                }
            }
            return;
        }
        i += 1;
    }
    crate::error::sh_error(cstr(b"Illegal option -%c\0"), &[VaArg::Char(flag)]);
    /* NOTREACHED */
}

/*
 * Set the shell parameters.
 */

// [spec:dash:def:options.setparam-fn]
// [spec:dash:sem:options.setparam-fn]
pub unsafe fn setparam(mut argv: *mut *mut c_char) {
    let newparam: *mut *mut c_char;
    let mut ap: *mut *mut c_char;
    let mut nparam: c_int;

    nparam = 0;
    while !(*argv.offset(nparam as isize)).is_null() {
        nparam += 1;
    }
    newparam = ckmalloc(
        (nparam as size_t + 1) * core::mem::size_of::<*mut c_char>() as size_t,
    ) as *mut *mut c_char;
    ap = newparam;
    while !(*argv).is_null() {
        *ap = savestr(*argv);
        ap = ap.add(1);
        argv = argv.add(1);
    }
    *ap = null_mut();
    freeparam(addr_of_mut!(shellparam));
    shellparam.malloc = 1;
    shellparam.nparam = nparam;
    shellparam.p = newparam;
    shellparam.optind = 1;
    shellparam.optoff = -1;
}

/*
 * Free the list of positional parameters.
 */

// [spec:dash:def:options.freeparam-fn]
// [spec:dash:sem:options.freeparam-fn]
pub unsafe fn freeparam(param: *mut shparam) {
    let mut ap: *mut *mut c_char;

    if (*param).malloc != 0 {
        ap = (*param).p;
        while !(*ap).is_null() {
            ckfree(*ap as *mut c_void);
            ap = ap.add(1);
        }
        ckfree((*param).p as *mut c_void);
    }
}

/*
 * The shift builtin command.
 */

// [spec:dash:def:options.shiftcmd-fn]
// [spec:dash:sem:options.shiftcmd-fn]
pub unsafe fn shiftcmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    let mut n: c_int;
    let mut ap1: *mut *mut c_char;
    let mut ap2: *mut *mut c_char;

    n = 1;
    if argc > 1 {
        n = crate::mystring::number(*argv.offset(1));
    }
    if n > shellparam.nparam {
        crate::error::sh_error(cstr(b"can't shift that many\0"), &[]);
    }
    INTOFF();
    shellparam.nparam -= n;
    ap1 = shellparam.p;
    loop {
        n -= 1;
        if !(n >= 0) {
            break;
        }
        if shellparam.malloc != 0 {
            ckfree(*ap1 as *mut c_void);
        }
        ap1 = ap1.add(1);
    }
    ap2 = shellparam.p;
    loop {
        *ap2 = *ap1;
        let done = (*ap2).is_null();
        ap2 = ap2.add(1);
        ap1 = ap1.add(1);
        if done {
            break;
        }
    }
    shellparam.optind = 1;
    shellparam.optoff = -1;
    INTON();
    0
}

/*
 * The set command builtin.
 */

// [spec:dash:def:options.setcmd-fn]
// [spec:dash:sem:options.setcmd-fn]
pub unsafe fn setcmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    if argc == 1 {
        return showvars(addr_of!(nullstr) as *const c_char, 0, VUNSET);
    }
    INTOFF();
    options(0);
    optschanged();
    if !(*argptr).is_null() {
        setparam(argptr);
    }
    INTON();
    0
}

// [spec:dash:def:options.getoptsreset-fn]
// [spec:dash:sem:options.getoptsreset-fn]
pub unsafe fn getoptsreset(value: *const c_char) {
    shellparam.optind = 1;
    shellparam.optoff = -1;
}

/*
 * The getopts builtin.  Shellparam.optnext points to the next argument
 * to be processed.  Shellparam.optptr points to the next character to
 * be processed in the current argument.  If shellparam.optnext is NULL,
 * then it's the first time getopts has been called.
 */

// [spec:dash:def:options.getoptscmd-fn]
// [spec:dash:sem:options.getoptscmd-fn]
pub unsafe fn getoptscmd(mut argc: c_int, mut argv: *mut *mut c_char) -> c_int {
    let optbase: *mut *mut c_char;

    nextopt(addr_of!(nullstr) as *const c_char);
    argc -= (((argptr as isize - argv as isize)
        / core::mem::size_of::<*mut c_char>() as isize)
        - 1) as c_int;
    argv = argptr.offset(-1);
    if argc < 3 {
        crate::error::sh_error(cstr(b"Usage: getopts optstring var [arg...]\0"), &[]);
    } else if argc == 3 {
        optbase = shellparam.p;
        if (shellparam.optind as c_uint) > (shellparam.nparam + 1) as c_uint {
            shellparam.optind = 1;
            shellparam.optoff = -1;
        }
    } else {
        optbase = argv.offset(3);
        if (shellparam.optind as c_uint) > (argc - 2) as c_uint {
            shellparam.optind = 1;
            shellparam.optoff = -1;
        }
    }

    getopts(*argv.offset(1), *argv.offset(2), optbase)
}

// [spec:dash:def:options.getopts-fn]
// [spec:dash:sem:options.getopts-fn]
unsafe fn getopts(
    optstr: *mut c_char,
    optvar: *mut c_char,
    optfirst: *mut *mut c_char,
) -> c_int {
    let mut p: *mut c_char;
    let mut q: *mut c_char;
    let mut c: c_char = b'?' as c_char;
    let mut done: c_int = 0;
    let mut s: [c_char; 2] = [0; 2];
    let mut optnext: *mut *mut c_char;
    let mut ind: c_int = shellparam.optind;
    let off: c_int = shellparam.optoff;

    shellparam.optind = -1;
    optnext = optfirst.offset(ind as isize - 1);

    if ind <= 1 || off < 0 || (libc::strlen(*optnext.offset(-1)) as size_t) < off as size_t {
        p = null_mut();
    } else {
        p = (*optnext.offset(-1)).offset(off as isize);
    }
    'out: loop {
        if p.is_null() || *p == b'\0' as c_char {
            /* Current word is done, advance */
            p = *optnext;
            if p.is_null() || *p != b'-' as c_char || {
                p = p.add(1);
                *p == b'\0' as c_char
            } {
                /* atend: */
                p = null_mut();
                done = 1;
                break 'out;
            }
            optnext = optnext.add(1);
            if *p.offset(0) == b'-' as c_char && *p.offset(1) == b'\0' as c_char {
                /* check for "--" — goto atend */
                p = null_mut();
                done = 1;
                break 'out;
            }
        }

        c = *p;
        p = p.add(1);
        q = if *optstr.offset(0) == b':' as c_char {
            optstr.offset(1)
        } else {
            optstr
        };
        while *q != c {
            if *q == b'\0' as c_char {
                if *optstr.offset(0) == b':' as c_char {
                    s[0] = c;
                    s[1] = b'\0' as c_char;
                    setvar(b"OPTARG\0".as_ptr() as *const c_char, s.as_ptr(), 0);
                } else {
                    crate::output::outfmt(
                        addr_of_mut!(crate::output::errout),
                        cstr(b"Illegal option -%c\n\0"),
                        &[VaArg::Char(c as c_int)],
                    );
                    crate::var::unsetvar(b"OPTARG\0".as_ptr() as *const c_char);
                }
                c = b'?' as c_char;
                break 'out;
            }
            q = q.add(1);
            if *q == b':' as c_char {
                q = q.add(1);
            }
        }

        q = q.add(1);
        if *q == b':' as c_char {
            if *p == b'\0' as c_char && {
                p = *optnext;
                p.is_null()
            } {
                if *optstr.offset(0) == b':' as c_char {
                    s[0] = c;
                    s[1] = b'\0' as c_char;
                    setvar(b"OPTARG\0".as_ptr() as *const c_char, s.as_ptr(), 0);
                    c = b':' as c_char;
                } else {
                    crate::output::outfmt(
                        addr_of_mut!(crate::output::errout),
                        cstr(b"No arg for -%c option\n\0"),
                        &[VaArg::Char(c as c_int)],
                    );
                    crate::var::unsetvar(b"OPTARG\0".as_ptr() as *const c_char);
                    c = b'?' as c_char;
                }
                break 'out;
            }

            if p == *optnext {
                optnext = optnext.add(1);
            }
            setvar(b"OPTARG\0".as_ptr() as *const c_char, p, 0);
            p = null_mut();
        } else {
            setvar(
                b"OPTARG\0".as_ptr() as *const c_char,
                addr_of!(nullstr) as *const c_char,
                0,
            );
        }
        break 'out;
    }

    /* out: */
    ind = ((optnext as isize - optfirst as isize)
        / core::mem::size_of::<*mut c_char>() as isize) as c_int
        + 1;
    setvarint(b"OPTIND\0".as_ptr() as *const c_char, ind as libc::intmax_t, VNOFUNC);
    s[0] = c;
    s[1] = b'\0' as c_char;
    setvar(optvar, s.as_ptr(), 0);

    shellparam.optoff = if !p.is_null() {
        (p as isize - *optnext.offset(-1) as isize) as c_int
    } else {
        -1
    };
    shellparam.optind = ind;

    done
}

/*
 * XXX - should get rid of.  have all builtins use getopt(3).  the
 * library getopt must have the BSD extension static variable "optreset"
 * otherwise it can't be used within the shell safely.
 *
 * Standard option processing (a la getopt) for builtin routines.  The
 * only argument that is passed to nextopt is the option string; the
 * other arguments are unnecessary.  It return the character, or '\0' on
 * end of input.
 */

// [spec:dash:def:options.nextopt-fn]
// [spec:dash:sem:options.nextopt-fn]
pub unsafe fn nextopt(optstring: *const c_char) -> c_int {
    let mut p: *mut c_char;
    let mut q: *const c_char;
    let c: c_char;

    p = optptr;
    if p.is_null() || *p == b'\0' as c_char {
        p = *argptr;
        if p.is_null() || *p != b'-' as c_char || {
            p = p.add(1);
            *p == b'\0' as c_char
        } {
            return b'\0' as c_int;
        }
        argptr = argptr.add(1);
        if *p.offset(0) == b'-' as c_char && *p.offset(1) == b'\0' as c_char {
            /* check for "--" */
            return b'\0' as c_int;
        }
    }
    c = *p;
    p = p.add(1);
    q = optstring;
    while *q != c {
        if *q == b'\0' as c_char {
            crate::error::sh_error(cstr(b"Illegal option -%c\0"), &[VaArg::Char(c as c_int)]);
        }
        q = q.add(1);
        if *q == b':' as c_char {
            q = q.add(1);
        }
    }
    q = q.add(1);
    if *q == b':' as c_char {
        if *p == b'\0' as c_char && {
            p = *argptr;
            argptr = argptr.add(1);
            p.is_null()
        } {
            crate::error::sh_error(cstr(b"No arg for -%c option\0"), &[VaArg::Char(c as c_int)]);
        }
        optionarg = p;
        p = null_mut();
    }
    optptr = p;
    c as c_int
}
