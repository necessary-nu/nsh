//! Literal port of `src/output.c` / `src/output.h`.
//! Rules: `docs/spec/port/src/output.md`.
//!
//! Shell output routines.  We use our own output routines because:
//!	When a builtin command is interrupted we have to discard
//!		any pending output.
//!	When a builtin command appears in back quotes, we want to
//!		save the output of the command in a region obtained
//!		via malloc, rather than doing a fork and reading the
//!		output of the command via a pipe.
//!	Our output routines may be smaller than the stdio routines.
//!
//! ## The one structural deviation in this module
//!
//! Stable Rust cannot *define* a C-variadic function and cannot build a
//! `va_list`.  Every `...` parameter in `output.h` therefore becomes a
//! `&[VaArg]` slice, `va_list` is that slice, `va_start`/`va_end`
//! disappear and `va_copy` is a copy of the slice reference (which is
//! exactly the semantics `xvasprintf` needs: a second, independent pass
//! over the same arguments).
//!
//! Because there is no `va_list` to hand to libc, `vsnprintf` itself is
//! re-implemented in `c_vsnprintf` below: it walks the format string and
//! renders each individual conversion by calling libc `snprintf` with a
//! one-conversion format and a single, correctly typed argument.  That
//! keeps C's exact padding/precision/`%j`/`%z` behaviour without a
//! hand-written number formatter.

use core::ptr::addr_of_mut;

use libc::{c_char, c_double, c_int, c_long, c_longlong, c_uint, c_ulong, c_ulonglong, c_void,
           size_t};

use crate::error::{INTOFF, INTON};
use crate::memalloc::{popstackmark, setstackmark, stackblocksize, stalloc, stackmark};
use crate::shell::{cstr, likely};

const OUTBUFSIZ: size_t = 8192; /* BUFSIZ */
pub const MEM_OUT: c_int = -3; /* output to dynamically allocated memory */

pub const OUTPUT_ERR: c_int = 0o1; /* error occurred on output */

// [spec:dash:def:output.output]
#[repr(C)]
pub struct output {
    pub nextc: *mut c_char,
    pub end: *mut c_char,
    pub buf: *mut c_char,
    pub bufsize: size_t,
    pub fd: c_int,
    pub flags: c_int,
}

pub static mut output: output = output {
    nextc: core::ptr::null_mut(),
    end: core::ptr::null_mut(),
    buf: core::ptr::null_mut(),
    bufsize: OUTBUFSIZ,
    fd: 1,
    flags: 0,
};
pub static mut errout: output = output {
    nextc: core::ptr::null_mut(),
    end: core::ptr::null_mut(),
    buf: core::ptr::null_mut(),
    bufsize: 0,
    fd: 2,
    flags: 0,
};
pub static mut preverrout: output = output {
    nextc: core::ptr::null_mut(),
    end: core::ptr::null_mut(),
    buf: core::ptr::null_mut(),
    bufsize: 0,
    fd: 0,
    flags: 0,
};
/*
 * #ifdef notyet
 * struct output memout = { .fd = MEM_OUT, ... };
 * #endif
 */
pub static mut out1: *mut output = addr_of_mut!(output);
pub static mut out2: *mut output = addr_of_mut!(errout);

/* ------------------------------------------------------------------ */
/* varargs stand-in                                                    */
/* ------------------------------------------------------------------ */

/*
 * One promoted C variadic argument.  The variant selects the type the
 * value is passed to libc `snprintf` as, and hence the length modifier
 * the rebuilt conversion carries.
 */
#[derive(Copy, Clone)]
pub enum VaArg {
    Int(c_int),
    Uint(c_uint),
    Long(c_long),
    Ulong(c_ulong),
    LongLong(c_longlong),
    Ulonglong(c_ulonglong),
    Intmax(libc::intmax_t),
    Uintmax(libc::uintmax_t),
    Size(size_t),
    Double(c_double),
    Char(c_int),
    Str(*const c_char),
    Ptr(*const c_void),
}

/* `va_list` — see the module comment. */
pub type va_list<'a> = &'a [VaArg];

macro_rules! snp {
    ($f:expr, $v:expr, $out:expr) => {{
        let n = libc::snprintf(core::ptr::null_mut(), 0, $f, $v);
        if n > 0 {
            let mut b: Vec<u8> = vec![0u8; n as usize + 1];
            let _ = libc::snprintf(b.as_mut_ptr() as *mut c_char, n as usize + 1, $f, $v);
            b.truncate(n as usize);
            $out.extend_from_slice(&b);
        }
    }};
}

/* Take the next argument, or a zero if the caller supplied too few (C
 * reads garbage off the stack in that case; a zero is the closest
 * defined behaviour). */
fn take_arg(ap: &[VaArg], argi: &mut usize) -> VaArg {
    if *argi < ap.len() {
        let a = ap[*argi];
        *argi += 1;
        a
    } else {
        VaArg::Int(0)
    }
}

/* `*` width / `.*` precision arguments are plain ints. */
fn take_int(ap: &[VaArg], argi: &mut usize) -> c_int {
    match take_arg(ap, argi) {
        VaArg::Int(v) => v,
        VaArg::Uint(v) => v as c_int,
        VaArg::Long(v) => v as c_int,
        VaArg::Ulong(v) => v as c_int,
        VaArg::LongLong(v) => v as c_int,
        VaArg::Ulonglong(v) => v as c_int,
        VaArg::Intmax(v) => v as c_int,
        VaArg::Uintmax(v) => v as c_int,
        VaArg::Size(v) => v as c_int,
        VaArg::Char(v) => v,
        _ => 0,
    }
}

/* Render one conversion.  `prefix` is `%` plus flags, width and
 * precision; the length modifier is regenerated from the argument's own
 * type so the libc call is always type-correct. */
unsafe fn format_one(prefix: &[u8], conv: u8, arg: VaArg, out: &mut Vec<u8>) {
    let mut spec: Vec<u8> = prefix.to_vec();
    match arg {
        VaArg::Long(_) | VaArg::Ulong(_) => spec.push(b'l'),
        VaArg::LongLong(_) | VaArg::Ulonglong(_) => spec.extend_from_slice(b"ll"),
        VaArg::Intmax(_) | VaArg::Uintmax(_) => spec.push(b'j'),
        VaArg::Size(_) => spec.push(b'z'),
        _ => {}
    }
    spec.push(conv);
    spec.push(0);
    let f = spec.as_ptr() as *const c_char;
    match arg {
        VaArg::Int(v) => snp!(f, v, out),
        VaArg::Uint(v) => snp!(f, v, out),
        VaArg::Long(v) => snp!(f, v, out),
        VaArg::Ulong(v) => snp!(f, v, out),
        VaArg::LongLong(v) => snp!(f, v, out),
        VaArg::Ulonglong(v) => snp!(f, v, out),
        VaArg::Intmax(v) => snp!(f, v, out),
        VaArg::Uintmax(v) => snp!(f, v, out),
        VaArg::Size(v) => snp!(f, v, out),
        VaArg::Double(v) => snp!(f, v, out),
        VaArg::Char(v) => snp!(f, v, out),
        VaArg::Str(v) => snp!(f, v, out),
        VaArg::Ptr(v) => snp!(f, v, out),
    }
}

/*
 * Stand-in for libc `vsnprintf` over the ported `va_list`.  Same
 * contract: write at most `length` bytes including the terminating NUL,
 * return the number of characters the full result would need.
 */
unsafe fn c_vsnprintf(
    outbuf: *mut c_char,
    length: size_t,
    fmt: *const c_char,
    ap: &[VaArg],
) -> c_int {
    let mut out: Vec<u8> = Vec::new();
    let mut argi: usize = 0;
    let mut f = fmt;

    while *f != 0 {
        let c = *f as u8;
        if c != b'%' {
            out.push(c);
            f = f.add(1);
            continue;
        }
        f = f.add(1);

        let mut spec: Vec<u8> = Vec::new();
        spec.push(b'%');

        /* flags */
        loop {
            let cc = *f as u8;
            if cc == b'-' || cc == b'+' || cc == b' ' || cc == b'#' || cc == b'0' || cc == b'\'' {
                spec.push(cc);
                f = f.add(1);
            } else {
                break;
            }
        }

        /* field width */
        if *f as u8 == b'*' {
            f = f.add(1);
            let w = take_int(ap, &mut argi);
            spec.extend_from_slice(format!("{}", w).as_bytes());
        } else {
            while (*f as u8).is_ascii_digit() {
                spec.push(*f as u8);
                f = f.add(1);
            }
        }

        /* precision */
        if *f as u8 == b'.' {
            f = f.add(1);
            if *f as u8 == b'*' {
                f = f.add(1);
                let p = take_int(ap, &mut argi);
                /* a negative precision is as if omitted */
                if p >= 0 {
                    spec.push(b'.');
                    spec.extend_from_slice(format!("{}", p).as_bytes());
                }
            } else {
                spec.push(b'.');
                while (*f as u8).is_ascii_digit() {
                    spec.push(*f as u8);
                    f = f.add(1);
                }
            }
        }

        /* length modifiers — dropped, regenerated from the argument */
        loop {
            let cc = *f as u8;
            if cc == b'h'
                || cc == b'l'
                || cc == b'j'
                || cc == b'z'
                || cc == b't'
                || cc == b'L'
                || cc == b'q'
            {
                f = f.add(1);
            } else {
                break;
            }
        }

        let conv = *f as u8;
        if conv == 0 {
            out.extend_from_slice(&spec);
            break;
        }
        f = f.add(1);
        if conv == b'%' {
            out.push(b'%');
            continue;
        }

        let arg = take_arg(ap, &mut argi);
        format_one(&spec, conv, arg, &mut out);
    }

    let n = out.len();
    if !outbuf.is_null() && length != 0 {
        let m = if n < length - 1 { n } else { length - 1 };
        core::ptr::copy_nonoverlapping(out.as_ptr(), outbuf as *mut u8, m);
        *outbuf.add(m) = 0;
    }
    n as c_int
}

/* ------------------------------------------------------------------ */
/* src/output.c                                                        */
/* ------------------------------------------------------------------ */

// [spec:dash:def:output.outmem-fn]
// [spec:dash:sem:output.outmem-fn]
pub unsafe fn outmem(p: *const c_char, len: size_t, dest: *mut output) {
    let bufsize: size_t;
    let offset: size_t;
    let mut nleft: size_t;

    nleft = ((*dest).end as usize).wrapping_sub((*dest).nextc as usize);
    if likely(nleft >= len) {
        /* buffered: */
        (*dest).nextc =
            crate::system::mempcpy((*dest).nextc as *mut c_void, p as *const c_void, len)
                as *mut c_char;
        return;
    }

    bufsize = (*dest).bufsize;
    if bufsize == 0 {
        /* unbuffered — fall through to the direct write */
    } else if (*dest).buf.is_null() {
        /*
         * #ifdef notyet
         *	if (dest->fd == MEM_OUT && len > bufsize) bufsize = len;
         * #endif
         */
        offset = 0;
        /*
         * #ifdef notyet
         *	goto alloc;
         * } else if (dest->fd == MEM_OUT) {
         *	offset = bufsize;
         *	if (bufsize >= len) bufsize <<= 1; else bufsize += len;
         *	if (bufsize < offset) goto err;
         * alloc:
         * #endif
         */
        INTOFF();
        (*dest).buf = crate::memalloc::ckrealloc((*dest).buf as *mut c_void, bufsize) as *mut c_char;
        (*dest).bufsize = bufsize;
        (*dest).end = (*dest).buf.add(bufsize);
        (*dest).nextc = (*dest).buf.add(offset);
        INTON();
    } else {
        flushout(dest);
    }

    nleft = ((*dest).end as usize).wrapping_sub((*dest).nextc as usize);
    /*
     * NOTE (faithfully reproduced, src/output.c:187): this second test is
     * `>` where the first was `>=`, so a run that would exactly fill the
     * buffer is written straight out instead of buffered.
     */
    if nleft > len {
        /* goto buffered; */
        (*dest).nextc =
            crate::system::mempcpy((*dest).nextc as *mut c_void, p as *const c_void, len)
                as *mut c_char;
        return;
    }

    if xwrite((*dest).fd, p as *const c_void, len) != 0 {
        /* err: */
        (*dest).flags |= OUTPUT_ERR;
    }
}

// [spec:dash:def:output.outstr-fn]
// [spec:dash:sem:output.outstr-fn]
pub unsafe fn outstr(p: *const c_char, file: *mut output) {
    let len: size_t;

    len = libc::strlen(p);
    outmem(p, len, file);
}

// [spec:dash:def:output.outcslow-fn]
// [spec:dash:sem:output.outcslow-fn]
pub unsafe fn outcslow(c: c_int, dest: *mut output) {
    let buf: c_char = c as c_char;
    outmem(&buf as *const c_char, 1, dest);
}

// [spec:dash:def:output.flushall-fn]
// [spec:dash:sem:output.flushall-fn]
pub unsafe fn flushall() {
    flushout(addr_of_mut!(output));
    /*
     * #ifdef FLUSHERR
     *	flushout(&errout);
     * #endif
     * — FLUSHERR is not defined in the shipped build.
     */
}

// [spec:dash:def:output.flushout-fn]
// [spec:dash:sem:output.flushout-fn]
pub unsafe fn flushout(dest: *mut output) {
    let len: size_t;

    len = ((*dest).nextc as usize).wrapping_sub((*dest).buf as usize);
    if len == 0 || (*dest).fd < 0 {
        return;
    }
    (*dest).nextc = (*dest).buf;
    if xwrite((*dest).fd, (*dest).buf as *const c_void, len) != 0 {
        (*dest).flags |= OUTPUT_ERR;
    }
}

// [spec:dash:def:output.outfmt-fn]
// [spec:dash:sem:output.outfmt-fn]
pub unsafe fn outfmt(file: *mut output, fmt: *const c_char, ap: &[VaArg]) {
    doformat(file, fmt, ap);
}

// [spec:dash:def:output.out1fmt-fn]
// [spec:dash:sem:output.out1fmt-fn]
pub unsafe fn out1fmt(fmt: *const c_char, ap: &[VaArg]) {
    doformat(out1, fmt, ap);
}

// [spec:dash:def:output.fmtstr-fn]
// [spec:dash:sem:output.fmtstr-fn]
pub unsafe fn fmtstr(outbuf: *mut c_char, length: size_t, fmt: *const c_char, ap: &[VaArg]) -> c_int {
    let ret: c_int;

    ret = xvsnprintf(outbuf, length, fmt, ap);
    if ret > length as c_int {
        length as c_int
    } else {
        ret
    }
}

// [spec:dash:def:output.xvasprintf-fn]
// [spec:dash:sem:output.xvasprintf-fn]
unsafe fn xvasprintf(
    sp: *mut *mut c_char,
    size: size_t,
    f: *const c_char,
    ap: &[VaArg],
) -> c_int {
    let s: *mut c_char;
    let mut len: c_int;
    let ap2: &[VaArg];

    ap2 = ap; /* va_copy(ap2, ap) */
    len = xvsnprintf(*sp, size, f, ap2);
    /* va_end(ap2) */
    if len < 0 {
        crate::error::sh_error(cstr(b"xvsnprintf failed\0"), &[]);
    }
    if (len as size_t) < size {
        return len;
    }

    s = stalloc(
        (if (len as size_t) >= stackblocksize() {
            len as size_t
        } else {
            stackblocksize()
        }) + 1,
    ) as *mut c_char;
    *sp = s;
    len = xvsnprintf(s, (len + 1) as size_t, f, ap);
    len
}

// [spec:dash:def:output.xasprintf-fn]
// [spec:dash:sem:output.xasprintf-fn]
pub unsafe fn xasprintf(sp: *mut *mut c_char, f: *const c_char, ap: &[VaArg]) -> c_int {
    let ret: c_int;

    ret = xvasprintf(sp, 0, f, ap);
    ret
}

// [spec:dash:def:output.doformat-fn]
// [spec:dash:sem:output.doformat-fn]
pub unsafe fn doformat(dest: *mut output, f: *const c_char, ap: &[VaArg]) {
    let mut smark: stackmark = stackmark::new();
    let mut s: *mut c_char;
    let len: c_int;
    let olen: c_int;

    setstackmark(&mut smark);
    s = (*dest).nextc;
    olen = ((*dest).end as isize).wrapping_sub((*dest).nextc as isize) as c_int;
    len = xvasprintf(&mut s, olen as size_t, f, ap);
    if likely(olen > len) {
        (*dest).nextc = (*dest).nextc.offset(len as isize);
    } else {
        /* out: is reached either way; only the buffered case skips this */
        outmem(s, len as size_t, dest);
    }
    popstackmark(&mut smark);
}

/*
 * Version of write which resumes after a signal is caught.
 */

// [spec:dash:def:output.xwrite-fn]
// [spec:dash:sem:output.xwrite-fn]
pub unsafe fn xwrite(fd: c_int, p: *const c_void, n: size_t) -> c_int {
    let mut buf: *const c_char = p as *const c_char;
    let mut n = n;

    while n != 0 {
        let mut i: isize;
        let mut m: size_t;

        m = n;
        if m > crate::system::SSIZE_MAX as size_t {
            m = crate::system::SSIZE_MAX as size_t;
        }
        loop {
            i = libc::write(fd, buf as *const c_void, m);
            if !(i < 0 && *libc::__errno_location() == libc::EINTR) {
                break;
            }
        }
        if i < 0 {
            return -1;
        }
        buf = buf.offset(i);
        n -= i as size_t;
    }
    0
}

/*
 * The three routines below sit inside `#ifdef notyet` *and*
 * `#ifdef USE_GLIBC_STDIO`, neither of which is defined in any shipped
 * configuration: `struct output` has no `stream` member and there is no
 * `memout`.  Their annotations therefore ride on equally inactive
 * bodies, with the C retained as a comment.
 */

// [spec:dash:def:output.initstreams-fn]
// [spec:dash:sem:output.initstreams-fn]
pub unsafe fn initstreams() {
    /* output.stream = stdout; */
    /* errout.stream = stderr; */
}

// [spec:dash:def:output.openmemout-fn]
// [spec:dash:sem:output.openmemout-fn]
pub unsafe fn openmemout() {
    /* INTOFF; */
    /* memout.stream = open_memstream(&memout.buf, &memout.bufsize); */
    /* INTON; */
}

// [spec:dash:def:output.closememout-fn]
// [spec:dash:sem:output.closememout-fn]
pub unsafe fn __closememout() -> c_int {
    /* int error; */
    /* error = fclose(memout.stream); */
    /* memout.stream = NULL; */
    /* return error; */
    0
}

// [spec:dash:def:output.xvsnprintf-fn]
// [spec:dash:sem:output.xvsnprintf-fn]
unsafe fn xvsnprintf(outbuf: *mut c_char, length: size_t, fmt: *const c_char, ap: &[VaArg]) -> c_int {
    let ret: c_int;

    /*
     * #ifdef __sun
     *	vsnprintf() on older versions of Solaris returns -1 when passed
     *	a length of 0.  To avoid this, use a dummy 1-character buffer
     *	instead.  Not applicable to the targets this port builds for.
     * #endif
     */

    INTOFF();
    ret = c_vsnprintf(outbuf, length, fmt, ap);
    INTON();
    ret
}

/* ------------------------------------------------------------------ */
/* src/output.h                                                        */
/* ------------------------------------------------------------------ */

// [spec:dash:def:output.freestdout-fn]
// [spec:dash:sem:output.freestdout-fn]
#[inline]
pub unsafe fn freestdout() {
    output.nextc = output.buf;
    output.flags = 0;
}

// [spec:dash:def:output.outc-fn]
// [spec:dash:sem:output.outc-fn]
#[inline]
pub unsafe fn outc(ch: c_int, file: *mut output) {
    if (*file).nextc == (*file).end {
        outcslow(ch, file);
    } else {
        *(*file).nextc = ch as c_char;
        (*file).nextc = (*file).nextc.add(1);
    }
}

/* `#define out1c(c) outc((c), out1)` */
#[inline(always)]
pub unsafe fn out1c(c: c_int) {
    outc(c, out1);
}

/* `#define out2c(c) outcslow((c), out2)` */
#[inline(always)]
pub unsafe fn out2c(c: c_int) {
    outcslow(c, out2);
}

/* `#define out1mem(s, l) outmem((s), (l), out1)` */
#[inline(always)]
pub unsafe fn out1mem(s: *const c_char, l: size_t) {
    outmem(s, l, out1);
}

/* `#define out1str(s) outstr((s), out1)` */
#[inline(always)]
pub unsafe fn out1str(s: *const c_char) {
    outstr(s, out1);
}

/* `#define out2str(s) outstr((s), out2)` */
#[inline(always)]
pub unsafe fn out2str(s: *const c_char) {
    outstr(s, out2);
}

/* `#define outerr(f) (f)->flags` */
#[inline(always)]
pub unsafe fn outerr(f: *mut output) -> c_int {
    (*f).flags
}

// ---------------------------------------------------------------------
// Variadic compatibility layer.
//
// C's `outfmt(file, fmt, ...)` has no stable-Rust function equivalent, so
// the formatted-output entry points above take an explicit `&[VaArg]`.
// The macros below restore the C call shape, which is what the ported
// call sites throughout the crate use. `VaArg::from` adapts the raw C
// values those sites pass; the reflexive `From<T> for T` means a site may
// also pass an explicit `VaArg` variant.
//
// Widths are per LP64, where `c_long`/`intmax_t` and
// `c_ulong`/`uintmax_t`/`size_t` collapse onto `i64`/`u64`. The formatter
// regenerates each conversion's length modifier from the variant, so a
// `%zu` rendered through `Uintmax` is still correct on this target.
// ---------------------------------------------------------------------

impl From<*const c_char> for VaArg {
    fn from(v: *const c_char) -> Self { VaArg::Str(v) }
}
impl From<*mut c_char> for VaArg {
    fn from(v: *mut c_char) -> Self { VaArg::Str(v as *const c_char) }
}
impl From<*const c_void> for VaArg {
    fn from(v: *const c_void) -> Self { VaArg::Ptr(v) }
}
impl From<*mut c_void> for VaArg {
    fn from(v: *mut c_void) -> Self { VaArg::Ptr(v as *const c_void) }
}
impl From<i32> for VaArg {
    fn from(v: i32) -> Self { VaArg::Int(v) }
}
impl From<u32> for VaArg {
    fn from(v: u32) -> Self { VaArg::Uint(v) }
}
impl From<i64> for VaArg {
    fn from(v: i64) -> Self { VaArg::Intmax(v as libc::intmax_t) }
}
impl From<u64> for VaArg {
    fn from(v: u64) -> Self { VaArg::Uintmax(v as libc::uintmax_t) }
}
impl From<f64> for VaArg {
    fn from(v: f64) -> Self { VaArg::Double(v) }
}

#[macro_export]
macro_rules! outfmt {
    ($file:expr, $fmt:expr $(, $arg:expr)* $(,)?) => {
        $crate::output::outfmt($file, $fmt, &[$($crate::output::VaArg::from($arg)),*])
    };
}

#[macro_export]
macro_rules! out1fmt {
    ($fmt:expr $(, $arg:expr)* $(,)?) => {
        $crate::output::out1fmt($fmt, &[$($crate::output::VaArg::from($arg)),*])
    };
}

#[macro_export]
macro_rules! fmtstr {
    ($buf:expr, $len:expr, $fmt:expr $(, $arg:expr)* $(,)?) => {
        $crate::output::fmtstr($buf, $len, $fmt, &[$($crate::output::VaArg::from($arg)),*])
    };
}

#[macro_export]
macro_rules! xasprintf {
    ($sp:expr, $f:expr $(, $arg:expr)* $(,)?) => {
        $crate::output::xasprintf($sp, $f, &[$($crate::output::VaArg::from($arg)),*])
    };
}

pub use crate::{fmtstr, out1fmt, outfmt, xasprintf};

// ---------------------------------------------------------------------
// Unit tests for this module's functions.
//
// Everything here writes to a real file descriptor, because that is what
// distinguishes this module's behaviour: whether a byte is sitting in the
// buffer or has reached the fd is the whole question for outmem, flushout
// and outc. Each test owns a pipe and a `struct output` pointing at it.
// ---------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::CStr0;

    /// A `struct output` writing into a pipe, with a buffer of `bufsize`.
    struct Sink {
        out: Box<output>,
        buf: Vec<c_char>,
        r: c_int,
        w: c_int,
    }

    impl Sink {
        fn new(bufsize: usize) -> Sink {
            let mut fds = [0 as c_int; 2];
            assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
            let mut buf = vec![0 as c_char; bufsize.max(1)];
            let base = buf.as_mut_ptr();
            let out = Box::new(output {
                nextc: base,
                end: unsafe { base.add(bufsize) },
                buf: base,
                bufsize: bufsize as size_t,
                fd: fds[1],
                flags: 0,
            });
            Sink { out, buf, r: fds[0], w: fds[1] }
        }
        fn p(&mut self) -> *mut output {
            &mut *self.out as *mut output
        }
        /// Bytes that have actually reached the pipe.
        fn drained(&mut self) -> Vec<u8> {
            unsafe {
                // Close the writer so the read sees EOF rather than blocking.
                libc::close(self.w);
                self.w = -1;
                let mut got = Vec::new();
                let mut tmp = [0u8; 4096];
                loop {
                    let n = libc::read(self.r, tmp.as_mut_ptr() as *mut c_void, tmp.len());
                    if n <= 0 {
                        break;
                    }
                    got.extend_from_slice(&tmp[..n as usize]);
                }
                got
            }
        }
        /// Bytes still sitting in the buffer, unflushed.
        fn buffered(&self) -> usize {
            (self.out.nextc as usize) - (self.out.buf as usize)
        }
    }

    impl Drop for Sink {
        fn drop(&mut self) {
            unsafe {
                if self.w >= 0 {
                    libc::close(self.w);
                }
                libc::close(self.r);
            }
            let _ = &self.buf;
        }
    }

    // [spec:dash:sem:output.outmem-fn/test]
    // [spec:dash:sem:output.flushout-fn/test]
    #[test]
    fn outmem_buffers_until_the_buffer_cannot_hold_the_write() {
        unsafe {
            let mut s = Sink::new(16);
            let p = s.p();
            outmem(CStr0::new("abc").p(), 3, p);
            // Fits: buffered, nothing on the fd yet.
            assert_eq!(s.buffered(), 3);
            outmem(CStr0::new("defghij").p(), 7, p);
            assert_eq!(s.buffered(), 10);
            flushout(p);
            assert_eq!(s.buffered(), 0);
            assert_eq!(s.drained(), b"abcdefghij");
        }
        unsafe {
            // A write larger than the buffer cannot be buffered, so the
            // buffer is flushed and the payload goes straight out.
            let mut s = Sink::new(4);
            let p = s.p();
            outmem(CStr0::new("ab").p(), 2, p);
            outmem(CStr0::new("0123456789").p(), 10, p);
            flushout(p);
            assert_eq!(s.drained(), b"ab0123456789");
        }
    }

    // [spec:dash:sem:output.flushout-fn/test]
    #[test]
    fn flushout_is_a_noop_when_empty_or_closed() {
        unsafe {
            let mut s = Sink::new(16);
            let p = s.p();
            // Nothing buffered: no write, no error flag.
            flushout(p);
            assert_eq!((*p).flags & OUTPUT_ERR, 0);
            // A negative fd is the "discard" case and must not be written
            // to; the buffer is left alone.
            outmem(CStr0::new("xy").p(), 2, p);
            (*p).fd = -1;
            flushout(p);
            assert_eq!(s.buffered(), 2);
            (*p).fd = s.w;
            flushout(p);
            assert_eq!(s.drained(), b"xy");
        }
    }

    // [spec:dash:sem:output.flushout-fn/test]
    #[test]
    fn flushout_records_an_error_on_a_bad_descriptor() {
        unsafe {
            let mut s = Sink::new(16);
            let p = s.p();
            outmem(CStr0::new("z").p(), 1, p);
            (*p).fd = 9999; // never opened
            flushout(p);
            assert_ne!((*p).flags & OUTPUT_ERR, 0);
            assert_ne!(outerr(p), 0);
            (*p).fd = -1; // keep Drop quiet
        }
    }

    // [spec:dash:sem:output.outstr-fn/test]
    // [spec:dash:sem:output.outcslow-fn/test]
    // [spec:dash:sem:output.outc-fn/test]
    #[test]
    fn outstr_outcslow_and_outc_append_bytes() {
        unsafe {
            let mut s = Sink::new(64);
            let p = s.p();
            outstr(CStr0::new("hi").p(), p);
            outcslow('!' as c_int, p);
            outc(' ' as c_int, p);
            outc('o' as c_int, p);
            // outstr stops at the NUL; outcslow and outc each add one byte.
            assert_eq!(s.buffered(), 5);
            flushout(p);
            assert_eq!(s.drained(), b"hi! o");
        }
    }

    // [spec:dash:sem:output.fmtstr-fn/test]
    #[test]
    fn fmtstr_formats_into_a_caller_buffer_and_truncates() {
        unsafe {
            let mut buf = [0 as c_char; 32];
            let n = fmtstr(
                buf.as_mut_ptr(),
                buf.len() as size_t,
                CStr0::new("%s=%d").p(),
                &[VaArg::Str(CStr0::new("x").p()), VaArg::Int(42)],
            );
            assert_eq!(crate::testutil::s(buf.as_ptr()), "x=42");
            assert_eq!(n, 4);
            // On truncation fmtstr CLAMPS to `length`; it does not report
            // what would have been written. The C is
            // `ret > (int)length ? length : ret`, so a caller detects
            // truncation by the result reaching the buffer size, not by it
            // exceeding it -- unlike snprintf, which is what a first draft
            // assumed here.
            let mut small = [0 as c_char; 4];
            let n2 = fmtstr(
                small.as_mut_ptr(),
                small.len() as size_t,
                CStr0::new("%s").p(),
                &[VaArg::Str(CStr0::new("abcdefgh").p())],
            );
            assert_eq!(crate::testutil::s(small.as_ptr()), "abc");
            assert_eq!(n2, 4);
        }
    }

    // [spec:dash:sem:output.doformat-fn/test]
    // [spec:dash:sem:output.outfmt-fn/test]
    #[test]
    fn outfmt_and_doformat_write_formatted_output() {
        unsafe {
            let mut s = Sink::new(64);
            let p = s.p();
            outfmt(p, CStr0::new("[%s:%d]").p(),
                   &[VaArg::Str(CStr0::new("k").p()), VaArg::Int(7)]);
            doformat(p, CStr0::new("%c").p(), &[VaArg::Int('!' as c_int)]);
            flushout(p);
            assert_eq!(s.drained(), b"[k:7]!");
        }
    }

    // [spec:dash:sem:output.xasprintf-fn/test]
    // [spec:dash:sem:output.xvasprintf-fn/test]
    #[test]
    fn xasprintf_allocates_on_the_stack_and_reports_the_length() {
        let _g = crate::testutil::lock();
        unsafe {
            let mut mark: crate::memalloc::stackmark = core::mem::zeroed();
            crate::memalloc::setstackmark(&mut mark);
            let mut sp: *mut c_char = core::ptr::null_mut();
            let n = xasprintf(&mut sp, CStr0::new("%s-%d").p(),
                              &[VaArg::Str(CStr0::new("id").p()), VaArg::Int(9)]);
            assert_eq!(crate::testutil::s(sp), "id-9");
            assert_eq!(n, 4);
            // Long enough to force the grow path rather than the initial
            // guess.
            let long = "y".repeat(2048);
            let n2 = xasprintf(&mut sp, CStr0::new("%s").p(),
                               &[VaArg::Str(CStr0::new(&long).p())]);
            assert_eq!(n2, 2048);
            assert_eq!(crate::testutil::s(sp).len(), 2048);
            crate::memalloc::popstackmark(&mut mark);
        }
    }

    // [spec:dash:sem:output.xvsnprintf-fn/test]
    #[test]
    fn xvsnprintf_is_snprintf_with_interrupts_held() {
        unsafe {
            let mut buf = [0 as c_char; 8];
            let n = xvsnprintf(buf.as_mut_ptr(), 8, CStr0::new("%d").p(), &[VaArg::Int(-5)]);
            assert_eq!(crate::testutil::s(buf.as_ptr()), "-5");
            assert_eq!(n, 2);
            // A zero length writes nothing but still returns the required
            // size; dash relies on that in xvasprintf's first pass.
            let n0 = xvsnprintf(core::ptr::null_mut(), 0, CStr0::new("abcd").p(), &[]);
            assert_eq!(n0, 4);
        }
    }

    // [spec:dash:sem:output.xwrite-fn/test]
    #[test]
    fn xwrite_writes_everything_or_reports_failure() {
        unsafe {
            let mut s = Sink::new(1);
            let payload = vec![b'q'; 200_000];
            // Larger than a pipe buffer, so this only succeeds if xwrite
            // loops over partial writes -- which is its whole purpose.
            let w = s.w;
            let reader = std::thread::spawn({
                let r = s.r;
                move || {
                    let mut got = 0usize;
                    let mut tmp = [0u8; 8192];
                    loop {
                        let n = libc::read(r, tmp.as_mut_ptr() as *mut c_void, tmp.len());
                        if n <= 0 { break; }
                        got += n as usize;
                    }
                    got
                }
            });
            assert_eq!(xwrite(w, payload.as_ptr() as *const c_void, payload.len()), 0);
            libc::close(w);
            s.w = -1;
            assert_eq!(reader.join().unwrap(), 200_000);
            // A closed descriptor fails rather than looping.
            assert_eq!(xwrite(9999, payload.as_ptr() as *const c_void, 1), -1);
        }
    }

    // [spec:dash:sem:output.freestdout-fn/test]
    #[test]
    fn freestdout_resets_the_buffer_and_error_flag() {
        let _g = crate::testutil::lock();
        unsafe {
            let saved = (output.nextc, output.buf, output.end, output.flags, output.fd);
            let mut b = vec![0 as c_char; 16];
            output.buf = b.as_mut_ptr();
            output.nextc = b.as_mut_ptr().add(5);
            output.end = b.as_mut_ptr().add(16);
            output.flags = OUTPUT_ERR;

            freestdout();

            assert_eq!(output.nextc, output.buf);
            assert_eq!(output.flags, 0);
            (output.nextc, output.buf, output.end, output.flags, output.fd) = saved;
        }
    }

    // [spec:dash:sem:output.flushall-fn/test]
    // [spec:dash:sem:output.out1fmt-fn/test]
    #[test]
    fn out1fmt_and_flushall_go_through_the_stdout_stream() {
        let _g = crate::testutil::lock();
        unsafe {
            let saved = (output.nextc, output.buf, output.end, output.bufsize,
                         output.fd, output.flags);
            let mut s = Sink::new(64);
            // Point the global stdout stream at the pipe.
            output.buf = s.out.buf;
            output.nextc = s.out.buf;
            output.end = s.out.end;
            output.bufsize = s.out.bufsize;
            output.fd = s.w;
            output.flags = 0;

            out1fmt(CStr0::new("n=%d").p(), &[VaArg::Int(3)]);
            // Buffered, not yet written.
            assert_eq!((output.nextc as usize) - (output.buf as usize), 3);
            flushall();
            assert_eq!(output.nextc, output.buf);

            (output.nextc, output.buf, output.end, output.bufsize,
             output.fd, output.flags) = saved;
            assert_eq!(s.drained(), b"n=3");
        }
    }

    // The three routines below sit inside `#ifdef notyet` and
    // `#ifdef USE_GLIBC_STDIO`, neither defined in any shipped
    // configuration: `struct output` has no `stream` member and there is
    // no `memout`. Their bodies are empty in both languages, so what can
    // be asserted is that they are callable and inert -- which is the
    // contract, and is what would break if someone gave one a body.
    //
    // [spec:dash:sem:output.initstreams-fn/test]
    // [spec:dash:sem:output.openmemout-fn/test]
    // [spec:dash:sem:output.closememout-fn/test]
    #[test]
    fn inactive_glibc_stdio_hooks_are_inert() {
        let _g = crate::testutil::lock();
        unsafe {
            let before = (output.nextc, output.buf, output.fd, errout.fd);
            initstreams();
            openmemout();
            assert_eq!(__closememout(), 0);
            assert_eq!((output.nextc, output.buf, output.fd, errout.fd), before);
        }
    }
}
