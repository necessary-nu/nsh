//! Literal port of `src/show.c` / `src/show.h`.
//! Rules: `docs/spec/port/src/show.md`.
//!
//! The whole file is inside `#ifdef DEBUG`; this port compiles it in, matching
//! the manifest, which lists every symbol.  Output goes to `tracefile`, a
//! stdio `FILE *` — deliberately not the shell's own `struct output` layer.
//!
//! `trace` is C-variadic.  Rust cannot *define* a C-variadic function, but it
//! can *call* one, so `trace` is realised as a macro that calls `fprintf`
//! directly — which is exactly what the C body does through `vfprintf`.

use libc::{c_char, c_int, c_void, FILE};
use core::ptr::null_mut;

use crate::nodes::{
    node, nodelist, NAPPEND, NAND, NARG, NCLOBBER, NCMD, NFROM, NFROMFD, NFROMTO, NOR, NPIPE,
    NSEMI, NTO, NTOFD,
};
use crate::parser::{
    CTLBACKQ, CTLENDVAR, CTLESC, CTLVAR, VSASSIGN, VSLENGTH, VSMINUS, VSNORMAL, VSNUL, VSPLUS,
    VSQUESTION, VSTRIMLEFT, VSTRIMLEFTMAX, VSTRIMRIGHT, VSTRIMRIGHTMAX, VSTYPE,
};

extern "C" {
    /// `vfprintf(3)`; `va_list` decays to a pointer in the SysV ABI.
    fn vfprintf(stream: *mut FILE, format: *const c_char, ap: *mut c_void) -> c_int;
    /* stdio objects the `libc` crate does not re-export. */
    static mut stdout: *mut FILE;
    static mut stderr: *mut FILE;
    fn setlinebuf(stream: *mut FILE);
}

// [spec:dash:def:show.showtree-fn]
// [spec:dash:sem:show.showtree-fn]
pub unsafe fn showtree(n: *mut node) {
    trputs(b"showtree called\n\0".as_ptr() as *const c_char);
    shtree(n, 1, null_mut(), stdout);
}

// [spec:dash:def:show.shtree-fn]
// [spec:dash:sem:show.shtree-fn]
unsafe fn shtree(n: *mut node, ind: c_int, pfx: *mut c_char, fp: *mut FILE) {
    let mut lp: *mut nodelist;
    let s: *const c_char;

    if n.is_null() {
        return;
    }

    indent(ind, pfx, fp);
    match (*n).r#type {
        NSEMI | NAND | NOR => {
            s = match (*n).r#type {
                NSEMI => b"; \0".as_ptr() as *const c_char,
                NAND => b" && \0".as_ptr() as *const c_char,
                _ => b" || \0".as_ptr() as *const c_char,
            };
            /* binop: */
            shtree((*n).nbinary.ch1, ind, null_mut(), fp);
            /*    if (ind < 0) */
            libc::fputs(s, fp);
            shtree((*n).nbinary.ch2, ind, null_mut(), fp);
        }
        NCMD => {
            shcmd(n, fp);
            if ind >= 0 {
                libc::fputc(b'\n' as c_int, fp);
            }
        }
        NPIPE => {
            lp = (*n).npipe.cmdlist;
            while !lp.is_null() {
                shcmd((*lp).n, fp);
                if !(*lp).next.is_null() {
                    libc::fputs(b" | \0".as_ptr() as *const c_char, fp);
                }
                lp = (*lp).next;
            }
            if (*n).npipe.backgnd != 0 {
                libc::fputs(b" &\0".as_ptr() as *const c_char, fp);
            }
            if ind >= 0 {
                libc::fputc(b'\n' as c_int, fp);
            }
        }
        _ => {
            libc::fprintf(
                fp,
                b"<node type %d>\0".as_ptr() as *const c_char,
                (*n).r#type,
            );
            if ind >= 0 {
                libc::fputc(b'\n' as c_int, fp);
            }
        }
    }
}

// [spec:dash:def:show.shcmd-fn]
// [spec:dash:sem:show.shcmd-fn]
unsafe fn shcmd(cmd: *mut node, fp: *mut FILE) {
    let mut np: *mut node;
    let mut first: c_int;

    first = 1;
    np = (*cmd).ncmd.args;
    while !np.is_null() {
        if first == 0 {
            libc::putchar(b' ' as c_int);
        }
        sharg(np, fp);
        first = 0;
        np = (*np).narg.next;
    }
    np = (*cmd).ncmd.redirect;
    while !np.is_null() {
        if first == 0 {
            libc::putchar(b' ' as c_int);
        }
        let (s, dftfd): (*const c_char, c_int) = match (*np).nfile.r#type {
            NTO => (b">\0".as_ptr() as *const c_char, 1),
            NCLOBBER => (b">|\0".as_ptr() as *const c_char, 1),
            NAPPEND => (b">>\0".as_ptr() as *const c_char, 1),
            NTOFD => (b">&\0".as_ptr() as *const c_char, 1),
            NFROM => (b"<\0".as_ptr() as *const c_char, 0),
            NFROMFD => (b"<&\0".as_ptr() as *const c_char, 0),
            NFROMTO => (b"<>\0".as_ptr() as *const c_char, 0),
            _ => (b"*error*\0".as_ptr() as *const c_char, 0),
        };
        if (*np).nfile.fd != dftfd {
            libc::fprintf(fp, b"%d\0".as_ptr() as *const c_char, (*np).nfile.fd);
        }
        libc::fputs(s, fp);
        if (*np).nfile.r#type == NTOFD || (*np).nfile.r#type == NFROMFD {
            libc::fprintf(fp, b"%d\0".as_ptr() as *const c_char, (*np).ndup.dupfd);
        } else {
            sharg((*np).nfile.fname, fp);
        }
        first = 0;
        np = (*np).nfile.next;
    }
}

// [spec:dash:def:show.sharg-fn]
// [spec:dash:sem:show.sharg-fn]
unsafe fn sharg(arg: *mut node, fp: *mut FILE) {
    let mut p: *mut c_char;
    let bqlist: *mut nodelist;
    let mut subtype: c_int;

    if (*arg).r#type != NARG {
        libc::printf(b"<node type %d>\n\0".as_ptr() as *const c_char, (*arg).r#type);
        libc::abort();
    }
    bqlist = (*arg).narg.backquote;
    p = (*arg).narg.text;
    while *p != 0 {
        match *p as i8 as c_int {
            CTLESC => {
                p = p.add(1);
                libc::fputc(*p as c_int, fp);
            }
            CTLVAR => {
                libc::fputc(b'$' as c_int, fp);
                libc::fputc(b'{' as c_int, fp);
                p = p.add(1);
                subtype = *p as c_int;
                if subtype == VSLENGTH {
                    libc::fputc(b'#' as c_int, fp);
                }

                while *p != b'=' as c_char {
                    libc::fputc(*p as c_int, fp);
                    p = p.add(1);
                }

                if (subtype & VSNUL) != 0 {
                    libc::fputc(b':' as c_int, fp);
                }

                match subtype & VSTYPE {
                    VSNORMAL => {
                        libc::fputc(b'}' as c_int, fp);
                    }
                    VSMINUS => {
                        libc::fputc(b'-' as c_int, fp);
                    }
                    VSPLUS => {
                        libc::fputc(b'+' as c_int, fp);
                    }
                    VSQUESTION => {
                        libc::fputc(b'?' as c_int, fp);
                    }
                    VSASSIGN => {
                        libc::fputc(b'=' as c_int, fp);
                    }
                    VSTRIMLEFT => {
                        libc::fputc(b'#' as c_int, fp);
                    }
                    VSTRIMLEFTMAX => {
                        libc::fputc(b'#' as c_int, fp);
                        libc::fputc(b'#' as c_int, fp);
                    }
                    VSTRIMRIGHT => {
                        libc::fputc(b'%' as c_int, fp);
                    }
                    VSTRIMRIGHTMAX => {
                        libc::fputc(b'%' as c_int, fp);
                        libc::fputc(b'%' as c_int, fp);
                    }
                    VSLENGTH => {}
                    _ => {
                        libc::printf(b"<subtype %d>\0".as_ptr() as *const c_char, subtype);
                    }
                }
            }
            CTLENDVAR => {
                libc::fputc(b'}' as c_int, fp);
            }
            CTLBACKQ => {
                libc::fputc(b'$' as c_int, fp);
                libc::fputc(b'(' as c_int, fp);
                shtree((*bqlist).n, -1, null_mut(), fp);
                libc::fputc(b')' as c_int, fp);
            }
            _ => {
                libc::fputc(*p as c_int, fp);
            }
        }
        p = p.add(1);
    }
}

// [spec:dash:def:show.indent-fn]
// [spec:dash:sem:show.indent-fn]
unsafe fn indent(amount: c_int, pfx: *mut c_char, fp: *mut FILE) {
    let mut i: c_int;

    i = 0;
    while i < amount {
        if !pfx.is_null() && i == amount - 1 {
            libc::fputs(pfx, fp);
        }
        libc::fputc(b'\t' as c_int, fp);
        i += 1;
    }
}

/*
 * Debugging stuff.
 */

pub static mut tracefile: *mut FILE = null_mut();

// [spec:dash:def:show.trputc-fn]
// [spec:dash:sem:show.trputc-fn]
pub unsafe fn trputc(c: c_int) {
    if crate::options::optlist[crate::options::debug] != 1 {
        return;
    }
    libc::fputc(c, tracefile);
}

// [spec:dash:def:show.trace-fn]
// [spec:dash:sem:show.trace-fn]
/// `void trace(const char *fmt, ...)`.  Realised as a macro because Rust
/// cannot define a C-variadic function; the body is the C body's
/// `vfprintf(tracefile, fmt, va)` with the arguments passed directly.
#[macro_export]
macro_rules! trace {
    ($fmt:literal $(, $a:expr)* $(,)?) => {{
        if $crate::options::optlist[crate::options::debug] == 1 {
            ::libc::fprintf(
                $crate::show::tracefile,
                concat!($fmt, "\0").as_ptr() as *const ::libc::c_char
                $(, $a)*
            );
        }
    }};
}

/// `#define TRACE(param) trace param` (shell.h, under `#ifdef DEBUG`); with
/// `DEBUG` undefined it expands to nothing, which `crate::shell::DEBUG` being
/// a `const false` reproduces without losing the type check.
#[macro_export]
macro_rules! TRACE {
    ($($t:tt)*) => {
        if $crate::shell::DEBUG {
            $crate::trace!($($t)*)
        }
    };
}

// [spec:dash:def:show.tracev-fn]
// [spec:dash:sem:show.tracev-fn]
pub unsafe fn tracev(fmt: *const c_char, va: *mut c_void) {
    if crate::options::optlist[crate::options::debug] != 1 {
        return;
    }
    vfprintf(tracefile, fmt, va);
}

/// `#define TRACEV(param) tracev param` (shell.h, under `#ifdef DEBUG`).
#[macro_export]
macro_rules! TRACEV {
    ($fmt:expr, $va:expr) => {
        if $crate::shell::DEBUG {
            $crate::show::tracev($fmt, $va)
        }
    };
}

// [spec:dash:def:show.trputs-fn]
// [spec:dash:sem:show.trputs-fn]
pub unsafe fn trputs(s: *const c_char) {
    if crate::options::optlist[crate::options::debug] != 1 {
        return;
    }
    libc::fputs(s, tracefile);
}

// [spec:dash:def:show.trstring-fn]
// [spec:dash:sem:show.trstring-fn]
unsafe fn trstring(s: *mut c_char) {
    let mut p: *mut c_char;

    if crate::options::optlist[crate::options::debug] != 1 {
        return;
    }
    libc::fputc(b'"' as c_int, tracefile);
    p = s;
    while *p != 0 {
        let esc: Option<c_char> = match *p as i8 as c_int {
            10 => Some(b'n' as c_char),  /* '\n' */
            9 => Some(b't' as c_char),   /* '\t' */
            13 => Some(b'r' as c_char),  /* '\r' */
            34 => Some(b'"' as c_char),  /* '"'  */
            92 => Some(b'\\' as c_char), /* '\\' */
            CTLESC => Some(b'e' as c_char),
            CTLVAR => Some(b'v' as c_char),
            CTLBACKQ => Some(b'q' as c_char),
            _ => None,
        };
        match esc {
            Some(c) => {
                /* backslash: */
                libc::fputc(b'\\' as c_int, tracefile);
                libc::fputc(c as c_int, tracefile);
            }
            None => {
                if *p >= b' ' as c_char && *p <= b'~' as c_char {
                    libc::fputc(*p as c_int, tracefile);
                } else {
                    /* NB the C writes the three octal *values*, not the three
                     * octal *digit characters* — there is no `'0' +` — so the
                     * "escape" renders as control bytes.  Reproduced verbatim
                     * (src/show.c:324-327). */
                    libc::fputc(b'\\' as c_int, tracefile);
                    libc::fputc((*p as c_int) >> 6 & 0o3, tracefile);
                    libc::fputc((*p as c_int) >> 3 & 0o7, tracefile);
                    libc::fputc((*p as c_int) & 0o7, tracefile);
                }
            }
        }
        p = p.add(1);
    }
    libc::fputc(b'"' as c_int, tracefile);
}

// [spec:dash:def:show.trargs-fn]
// [spec:dash:sem:show.trargs-fn]
pub unsafe fn trargs(mut ap: *mut *mut c_char) {
    if crate::options::optlist[crate::options::debug] != 1 {
        return;
    }
    while !(*ap).is_null() {
        trstring(*ap);
        ap = ap.add(1);
        if !(*ap).is_null() {
            libc::fputc(b' ' as c_int, tracefile);
        } else {
            libc::fputc(b'\n' as c_int, tracefile);
        }
    }
}

// [spec:dash:def:show.opentrace-fn]
// [spec:dash:sem:show.opentrace-fn]
pub unsafe fn opentrace() {
    let mut s: [c_char; 100] = [0; 100];
    /* #ifdef O_APPEND */
    let flags: c_int;

    if crate::options::optlist[crate::options::debug] != 1 {
        if !tracefile.is_null() {
            libc::fflush(tracefile);
        }
        /* leave open because libedit might be using it */
        return;
    }
    /* #ifdef not_this_way — the $HOME variant is not compiled. */
    libc::strcpy(s.as_mut_ptr(), b"./trace\0".as_ptr() as *const c_char);
    if !tracefile.is_null() {
        /* #ifndef __KLIBC__ */
        if libc::freopen(s.as_ptr(), b"a\0".as_ptr() as *const c_char, tracefile).is_null() {
            libc::fprintf(
                stderr,
                b"Can't re-open %s\n\0".as_ptr() as *const c_char,
                s.as_ptr(),
            );
            crate::options::optlist[crate::options::debug] = 0;
            return;
        }
    } else {
        tracefile = libc::fopen(s.as_ptr(), b"a\0".as_ptr() as *const c_char);
        if tracefile.is_null() {
            libc::fprintf(
                stderr,
                b"Can't open %s\n\0".as_ptr() as *const c_char,
                s.as_ptr(),
            );
            crate::options::optlist[crate::options::debug] = 0;
            return;
        }
    }
    /* #ifdef O_APPEND */
    flags = libc::fcntl(libc::fileno(tracefile), libc::F_GETFL, 0);
    if flags >= 0 {
        libc::fcntl(
            libc::fileno(tracefile),
            libc::F_SETFL,
            flags | libc::O_APPEND,
        );
    }
    /* #ifndef __KLIBC__ */
    setlinebuf(tracefile);
    libc::fputs(b"\nTracing started.\n\0".as_ptr() as *const c_char, tracefile);
}
