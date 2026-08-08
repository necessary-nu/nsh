//! Literal port of `src/bltin/printf.c` — the `printf` and `echo`
//! builtins.
//! Rules: `docs/spec/port/src/bltin/printf.md`.
//!
//! `conv_escape` is declared in `system.h` and shared with `parser.c`
//! (which calls it with `mbchar = true`), so it is `pub` here and carries
//! both the `printf.*` and the `system.*` rule ids.
//!
//! Cross-module signatures assumed (see the port report):
//!   * `crate::memalloc::{stackmark, setstackmark, popstackmark}` — the
//!     region mark that bounds the block `xasprintf` allocates.  The three
//!     buffers this file builds — the escaped string, the run of `X`s that
//!     stands in for it, and `mklong`'s widened format — are owned, so
//!     `USTPUTC`/`STADJUST` below are the two macros `conv_escape` still
//!     needs to write through a bare cursor and touch no region.
//!   * `crate::output::{out1mem, out1fmt!, outc, out1c, xasprintf!}`
//!   * `crate::error::{sh_error!, sh_warnx!}` via `bltin.h`'s aliases
//!   * `crate::mystring::{nullstr, snlfmt}` — `char nullstr[1]`
//!     (src/shell.h:74) and `const char snlfmt[]` (src/mystring.h:52)
//!   * `crate::options::{nextopt, argptr}`
//!   * `crate::parser::{CTLESC, CTLMBCHAR}` (src/parser.h:43,47)
//!   * `crate::syntax::{sqsyntax, SYNBASE, CCTL}` — the generated syntax
//!     tables; `SQSYNTAX` is `sqsyntax + SYNBASE` with `SYNBASE` 129
//!     (src/mksyntax.c:147,152)

use core::mem;
use core::ptr;
use libc::{c_char, c_double, c_int, c_uint, c_void, intmax_t, uintmax_t};

use bstr::BString;

// glibc extensions/C99 functions the `libc` crate does not reliably expose.
//
// `strtoimax`/`strtoumax`: glibc >= 2.38 redirects both through
// `__isoc23_*` for any translation unit that enables the C23 strtol
// semantics (`__GLIBC_USE (C23_STRTOL)`), which is the case for the dash
// build; those variants additionally accept `0b`/`0B` binary constants
// when the base is 0 or 2.  The C `printf.c` therefore really calls
// `__isoc23_strtoimax`, so `printf %d 0b11` prints 3 — link the same
// symbols here or the port silently loses binary literals.
extern "C" {
    fn strchrnul(s: *const c_char, c: c_int) -> *mut c_char;
    #[link_name = "__isoc23_strtoimax"]
    fn strtoimax(nptr: *const c_char, endptr: *mut *mut c_char, base: c_int) -> intmax_t;
    #[link_name = "__isoc23_strtoumax"]
    fn strtoumax(nptr: *const c_char, endptr: *mut *mut c_char, base: c_int) -> uintmax_t;
}

// ---------------------------------------------------------------------
// src/memalloc.h:78-97 — the two stack-string macros `conv_escape` still
// needs.  Both are pure cursor arithmetic; neither touches the region.
// ---------------------------------------------------------------------

/// The most `conv_escape` can write past the cursor it is given.
///
/// The C guards both of its calls with `CHECKSTRSPACE(4, cp)`, and 4 is not
/// enough. `\U0001F600` takes the `len == 4` arm, which writes the four
/// encoded bytes at `out + mboff` and *then* `USTPUTC(len, out);
/// USTPUTC(CTLMBCHAR, out)` at `out + len` and `out + len + 1` — bytes 5 and
/// 6 with `mbchar` false, bytes 7 and 8 with it true. It returns before them,
/// so they are scratch the next write overwrites; but they are written, and
/// in a 504-byte stack block nobody notices. Spare capacity is exactly as
/// long as it is reserved to be, so the port has to reserve what the C
/// writes rather than what the C says.
///
/// `parser.rs` reaches the `mbchar` arm and its `CHECKSTRSPACE(MAX(MB_LEN_MAX,
/// 16) + 7, out)` already covers 8.
pub const CONV_ESCAPE_SLOP: usize = 8;

/// `#define USTPUTC(c, p) (*p++ = (c))`
macro_rules! USTPUTC {
    ($c:expr, $p:ident) => {{
        *$p = $c as c_char;
        $p = $p.add(1);
    }};
}

/// `#define STADJUST(amount, p) (p += (amount))`
macro_rules! STADJUST {
    ($amount:expr, $p:ident) => {
        $p = $p.offset($amount as isize)
    };
}

/// `extern char nullstr[1];` (src/shell.h:74), defined in src/mystring.c:60.
#[inline]
unsafe fn nullstr() -> *mut c_char {
    ptr::addr_of!(crate::mystring::nullstr) as *const c_char as *mut c_char
}

/// `extern const char snlfmt[];` (src/mystring.h:52) — `"%s\n"`.
#[inline]
unsafe fn snlfmt() -> *const c_char {
    ptr::addr_of!(crate::mystring::snlfmt) as *const c_char
}

static mut rval: c_int = 0;
static mut gargv: *mut *mut c_char = ptr::null_mut();

/// `#define isodigit(c) ((c) >= '0' && (c) <= '7')`
#[inline]
fn isodigit(c: c_int) -> bool {
    c >= b'0' as c_int && c <= b'7' as c_int
}

/// `#define octtobin(c) ((c) - '0')`
#[inline]
fn octtobin(c: c_int) -> c_int {
    c - b'0' as c_int
}

// ---------------------------------------------------------------------
// src/bltin/printf.c:62-90 — PF and ASPF.
//
// C has no way to build a variadic call at runtime, so both macros switch
// on how many `*` width/precision values were collected into `array` and
// write out the three calls longhand. `param` and `array` have to be
// passed explicitly here because `macro_rules!` is hygienic and cannot
// reach the caller's locals the way a C macro does.
// ---------------------------------------------------------------------

macro_rules! PF {
    ($param:expr, $array:expr, $f:expr, $func:expr) => {{
        let __array: *mut c_int = $array;
        // (char *)param - (char *)array
        match ($param as usize).wrapping_sub(__array as usize) {
            // case 0:
            0 => {
                printf!($f, $func);
            }
            // case sizeof(*param):
            __n if __n == mem::size_of::<c_int>() => {
                printf!($f, *__array.add(0), $func);
            }
            // default:
            _ => {
                printf!($f, *__array.add(0), *__array.add(1), $func);
            }
        }
    }};
}

macro_rules! ASPF {
    ($param:expr, $array:expr, $sp:expr, $f:expr, $func:expr) => {{
        let __array: *mut c_int = $array;
        let ret: c_int;
        // (char *)param - (char *)array
        match ($param as usize).wrapping_sub(__array as usize) {
            // case 0:
            0 => {
                ret = crate::output::xasprintf!($sp, $f, $func);
            }
            // case sizeof(*param):
            __n if __n == mem::size_of::<c_int>() => {
                ret = crate::output::xasprintf!($sp, $f, *__array.add(0), $func);
            }
            // default:
            _ => {
                ret = crate::output::xasprintf!($sp, $f, *__array.add(0), *__array.add(1), $func);
            }
        }
        ret
    }};
}

// [spec:dash:def:printf.print-escape-str-fn]
// [spec:dash:sem:printf.print-escape-str-fn]
unsafe fn print_escape_str(
    f: *const c_char,
    param: *mut c_int,
    array: *mut c_int,
    s: *mut c_char,
) -> c_int {
    let mut smark: crate::memalloc::stackmark = mem::zeroed();
    let mut p: *const c_char;
    let done: c_int;
    let mut len: c_int;
    let mut total: c_int;
    /* The C's `q` is a cursor into the stack block and `stackblock()` its
     * base.  Both are this buffer: `len` is its length, `q[-1]` its last
     * byte, and the `q = stackblock()` the C re-reads after `makestrspace`
     * is the buffer itself, which cannot move under it. */
    let mut buf = BString::default();

    /* The mark is still needed: `ASPF` goes through `xasprintf`, which
     * `stalloc`s the block it formats into.  Nothing else here is region. */
    crate::memalloc::setstackmark(&mut smark);
    done = conv_escape_str(s, &mut buf);
    len = buf.len() as c_int;
    total = len - 1;

    /* `conv_escape_str`'s do-while exits only on the iteration that writes
     * the terminating NUL, so `q[-1]` always exists. */
    debug_assert!(len >= 1);

    // q[-1] = (!!((f[1] - 's') | done) - 1) & f[2];
    let close = (((((*f.add(1) as c_int - b's' as c_int) | done) != 0) as c_int - 1)
        & *f.add(2) as c_int) as u8;
    buf[len as usize - 1] = close;
    total += (close != 0) as c_int;

    p = buf.as_ptr() as *const c_char;

    'easy: {
        if *f.add(1) == b's' as c_char {
            break 'easy; /* goto easy */
        }

        /* The mask above is 0 whenever `f[1]` is not 's', which is every
         * path that reaches here, so the run of `X`s is exactly as long as
         * the converted string. */
        debug_assert_eq!(total, len - 1);

        /* `p = makestrspace(len, q); memset(p, 'X', total); p[total] = 0;`
         * The C puts the `X`s directly above the converted string because
         * the region is the only scratch it has, and nothing ever reads the
         * two as one string — so here they are a buffer of their own. */
        let mut xs: Vec<u8> = vec![b'X'; total as usize];
        xs.push(0);

        let mut r: *mut c_char = xs.as_mut_ptr() as *mut c_char;
        // `ASPF(&p, f, p)`: the out-parameter has to be a raw pointer, not
        // `&mut r`, or the borrow would conflict with reading `r` as the
        // conversion's argument in the same call.
        total = ASPF!(param, array, ptr::addr_of_mut!(r), f, r);

        /* The formatted result carries the field width, with the `X`s
         * standing in for the real bytes — which may contain NUL and so
         * cannot go through `printf` at all.  Put them back. */
        len = strchrnul(r, b'X' as c_int).offset_from(r) as c_int;
        libc::memcpy(
            r.add(len as usize) as *mut c_void,
            buf.as_ptr() as *const c_void,
            libc::strspn(r.add(len as usize), c"X".as_ptr()),
        );
        p = r;
    }

    // easy:
    crate::output::out1mem(p, total as usize);

    crate::memalloc::popstackmark(&mut smark);
    done
}

const SKIP1: &core::ffi::CStr = c"#-+ 0";
const SKIP2: &core::ffi::CStr = c"*0123456789";

// [spec:dash:def:printf.printfcmd-fn]
// [spec:dash:sem:printf.printfcmd-fn]
pub unsafe fn printfcmd(argc: c_int, mut argv: *mut *mut c_char) -> c_int {
    let mut fmt: *mut c_char;
    let format: *mut c_char;
    let mut ch: c_int;

    rval = 0;

    crate::options::nextopt(nullstr());

    argv = crate::options::argptr;
    format = *argv;

    if format.is_null() {
        error!(c"usage: printf format [arg ...]".as_ptr());
    }

    argv = argv.add(1);
    gargv = argv;

    'out: {
        loop {
            /*
             * Basic algorithm is to scan the format string for conversion
             * specifications -- once one is found, find out if the field
             * width or precision is a '*'; if it is, gather up value.
             * Note, format strings are reused as necessary to use up the
             * provided arguments, arguments of zero/null string are
             * provided to use up the format string.
             */

            /* find next format specification */
            fmt = format;
            loop {
                ch = *fmt as c_int;
                fmt = fmt.add(1);
                if ch == 0 {
                    break;
                }

                let mut start: *mut c_char;
                let nextch: c_char;
                let mut array: [c_int; 2] = [0; 2];
                let mut param: *mut c_int;
                /* `mklong`'s widened format, held for as long as the
                 * conversion that reads it. */
                let mut longfmt = BString::default();

                if ch == b'\\' as c_int {
                    let ret: c_uint;
                    /* `STARTSTACKSTR(cp); CHECKSTRSPACE(4, cp)` — one
                     * escape's worth of scratch and nothing else; see
                     * `CONV_ESCAPE_SLOP` for why 4 is not the bound. */
                    let mut cp: [c_char; CONV_ESCAPE_SLOP] = [0; CONV_ESCAPE_SLOP];

                    ret = conv_escape(fmt, cp.as_mut_ptr(), false);
                    fmt = fmt.add((ret >> 4) as usize);
                    debug_assert!((ret & 15) as usize <= CONV_ESCAPE_SLOP);
                    crate::output::out1mem(cp.as_ptr(), (ret & 15) as usize);
                    continue;
                }
                if ch != b'%' as c_int
                    || (*fmt == b'%' as c_char && {
                        fmt = fmt.add(1);
                        true
                    })
                {
                    putchar!(ch);
                    continue;
                }

                /* Ok - we've found a format specification,
                Save its address for a later printf(). */
                start = fmt.sub(1);
                param = array.as_mut_ptr();

                /* skip to field width */
                fmt = fmt.add(libc::strspn(fmt, SKIP1.as_ptr()));
                if *fmt == b'*' as c_char {
                    fmt = fmt.add(1);
                    *param = getuintmax(1) as c_int;
                    param = param.add(1);
                } else {
                    /* skip to possible '.',
                     * get following precision
                     */
                    fmt = fmt.add(libc::strspn(fmt, SKIP2.as_ptr()));
                }

                if *fmt == b'.' as c_char {
                    fmt = fmt.add(1);
                    if *fmt == b'*' as c_char {
                        fmt = fmt.add(1);
                        *param = getuintmax(1) as c_int;
                        param = param.add(1);
                    } else {
                        fmt = fmt.add(libc::strspn(fmt, SKIP2.as_ptr()));
                    }
                }

                ch = *fmt as c_int;
                if ch == 0 {
                    error!(c"missing format character".as_ptr());
                }
                /* null terminate format string to we can use it
                as an argument to printf. */
                nextch = *fmt.add(1);
                *fmt.add(1) = 0;
                match ch as u8 {
                    b'b' => {
                        *fmt = b's' as c_char;
                        /* escape if a \c was encountered */
                        if print_escape_str(start, param, array.as_mut_ptr(), getstr()) != 0 {
                            break 'out; /* goto out */
                        }
                        *fmt = b'b' as c_char;
                    }
                    b'c' => {
                        let p: c_int = getchr();
                        PF!(param, array.as_mut_ptr(), start, p);
                    }
                    b's' => {
                        let p: *mut c_char = getstr();
                        PF!(param, array.as_mut_ptr(), start, p);
                    }
                    b'd' | b'i' => {
                        let p: uintmax_t = getuintmax(1);
                        start = mklong(&mut longfmt, start, fmt);
                        PF!(param, array.as_mut_ptr(), start, p);
                    }
                    b'o' | b'u' | b'x' | b'X' => {
                        let p: uintmax_t = getuintmax(0);
                        start = mklong(&mut longfmt, start, fmt);
                        PF!(param, array.as_mut_ptr(), start, p);
                    }
                    b'a' | b'A' | b'e' | b'E' | b'f' | b'F' | b'g' | b'G' => {
                        let p: c_double = getdouble();
                        PF!(param, array.as_mut_ptr(), start, p);
                    }
                    _ => {
                        error!(c"%s: invalid directive".as_ptr(), start);
                    }
                }
                fmt = fmt.add(1);
                *fmt = nextch;
            }

            if !(gargv != argv && !(*gargv).is_null()) {
                break;
            }
        }
    }

    // out:
    rval
}

/*
 * Print SysV echo(1) style escape string
 *	Halts processing string if a \c escape is encountered.
 */
// [spec:dash:def:printf.conv-escape-str-fn]
// [spec:dash:sem:printf.conv-escape-str-fn]
unsafe fn conv_escape_str(mut str: *mut c_char, cp: &mut BString) -> c_int {
    let mut c: c_int;

    /* convert string into a temporary buffer... */
    /* `STARTSTACKSTR(cp)` — the buffer is the caller's, and the C's `*sp =
     * cp` at the end is its length. */
    debug_assert!(cp.is_empty());

    loop {
        let ret: c_uint;
        let ch: c_int;

        /* `CHECKSTRSPACE(4, cp)` — the room `conv_escape` writes into
         * through the raw cursor below; see `CONV_ESCAPE_SLOP`. */
        cp.reserve(CONV_ESCAPE_SLOP);

        // `goto putchar` is taken from two places; the flag replaces it.
        let mut goto_putchar = false;

        c = *str as c_int;
        str = str.add(1);
        if c != b'\\' as c_int {
            ch = 0; /* unused on this path */
            goto_putchar = true;
        } else {
            ch = *str as c_int;
            if ch == b'c' as c_int {
                /* \c as in SYSV echo - abort all processing.... */
                c = 0x100;
                goto_putchar = true;
            }
        }

        if goto_putchar {
            // putchar:
            /* `USTPUTC(c, cp)` truncates to `char`, which is what turns
             * `\c`'s 0x100 into the terminating NUL. */
            cp.push(c as u8);
        } else {
            /*
             * %b string octal constants are not like those in C.
             * They start with a \0, and are followed by 0, 1, 2,
             * or 3 octal digits.
             */
            if ch == b'0' as c_int && isodigit(*str.add(1) as c_int) {
                str = str.add(1);
            }

            /* Finally test for sequences valid in the format string */
            let at = cp.len();
            ret = conv_escape(str, cp.as_mut_ptr().add(at) as *mut c_char, false);
            str = str.add((ret >> 4) as usize);
            /* `cp += ret & 15` is the commit of what `conv_escape` wrote
             * past the cursor; what it wrote above that stays uncommitted,
             * for the next write to overwrite as the C's does. */
            debug_assert!((ret & 15) as usize <= CONV_ESCAPE_SLOP);
            cp.set_len(at + (ret & 15) as usize);
        }

        // } while (c & 0xff);
        if (c & 0xff) == 0 {
            break;
        }
    }

    c
}

// Character constants used as `match` patterns; Rust cannot cast inside a
// pattern the way a C `case` label can.
const CH_BACKSLASH: c_int = b'\\' as c_int;
const CH_X: c_int = b'x' as c_int;
const CH_U: c_int = b'u' as c_int;
const CH_A: c_int = b'a' as c_int;
const CH_B: c_int = b'b' as c_int;
const CH_F: c_int = b'f' as c_int;
const CH_E: c_int = b'e' as c_int;
const CH_N: c_int = b'n' as c_int;
const CH_R: c_int = b'r' as c_int;
const CH_T: c_int = b't' as c_int;
const CH_V: c_int = b'v' as c_int;

/*
 * Print "standard" escape characters
 */
// [spec:dash:def:printf.conv-escape-fn]
// [spec:dash:sem:printf.conv-escape-fn]
// [spec:dash:def:system.conv-escape-fn]
// [spec:dash:sem:system.conv-escape-fn]
pub unsafe fn conv_escape(str0: *mut c_char, out0: *mut c_char, mbchar: bool) -> c_uint {
    let mut out: *mut c_char = out0;
    let mut str: *mut c_char = str0;
    let mut value: c_uint;
    let och: c_int;
    let mut ch: c_int;

    ch = *str as c_int;
    value = ch as c_uint;

    // The C switch's `default:` label falls into `check_value:`, which falls
    // into `case '\\':`; `case 'x':` falls into `hex:`, which can jump back
    // to `check_value:` or forward to `out_noput:`. The three flags below
    // encode those gotos; the blocks stay in source order except that the
    // `default:` arm has to move last, as Rust requires.
    let mut goto_hex = false;
    let mut goto_check_value = false;
    let mut goto_backslash = false;

    'out_noput: {
        'sw: {
            match ch {
                CH_BACKSLASH => {
                    goto_backslash = true;
                }

                CH_X => {
                    ch = 2;
                    goto_hex = true;
                }

                CH_U => {
                    ch = 4;
                    goto_hex = true;
                }

                CH_A /* alert */ | CH_B /* backspace */ | CH_F /* form-feed */ => {
                    value = value.wrapping_sub(b'a' as c_uint);
                    value = value.wrapping_add(0x07 /* '\a' */);
                }

                CH_E => value = 0o33,   /* <ESC> */
                CH_N => value = 0o12,   /* newline */
                CH_R => value = 0o15,   /* carriage-return */
                CH_T => value = 0o11,   /* tab */
                CH_V => value = 0o13,   /* vertical-tab */

                _ => {
                    // default:
                    if mbchar && (ch == b'"' as c_int || ch == b'\'' as c_int) {
                        break 'sw;
                    }

                    if ch == b'U' as c_int {
                        ch = 8;
                        goto_hex = true;
                        break 'sw;
                    }

                    value = b'\\' as c_uint;

                    if isodigit(ch) {
                        ch = 3;
                        value = 0;
                        loop {
                            value <<= 3;
                            value = value.wrapping_add(octtobin(*str as c_int) as c_uint);
                            str = str.add(1);
                            ch -= 1;
                            if !(ch != 0 && isodigit(*str as c_int)) {
                                break;
                            }
                        }
                    }

                    str = str.sub(1);

                    goto_check_value = true;
                }
            }
        }

        if goto_hex {
            // hex:
            och = ch;
            value = 0;
            loop {
                str = str.add(1);
                let c: c_int = *str as c_int;
                let d: c_int;

                if c >= b'0' as c_int && c <= b'9' as c_int {
                    d = c - b'0' as c_int;
                } else {
                    let cl: c_int;

                    cl = c & !0x20;
                    if cl >= b'A' as c_int && cl <= b'F' as c_int {
                        d = cl - b'A' as c_int + 10;
                    } else {
                        str = str.sub(1);
                        break;
                    }
                }

                value <<= 4;
                value = value.wrapping_add(d as c_uint);

                ch -= 1;
                if ch == 0 {
                    break;
                }
            }

            if och <= 2 {
                goto_check_value = true;
            } else if value < 0x80 {
                goto_check_value = true;
            } else {
                if value < 0x110000 {
                    let mboff: c_int = (mbchar as c_int - 1) * 2;
                    let uni: c_uint = value;
                    let len: c_int;

                    value = 0x80 << 8 | (value & 0xfc0) << 2 | 0x80 | (value & 0x3f);

                    if uni < 0x800 {
                        value |= 0x40 << 8;
                        len = 2;
                    } else {
                        value |= 0x80 << 16 | (uni & 0x3f000) << 4;
                        if uni < 0x10000 {
                            value |= 0x60 << 16;
                            len = 3;
                        } else {
                            value |= 0xf0 << 24 | (uni & !0x3ffff) << 6;
                            len = 4;
                        }
                    }

                    // htonl(): host order to big-endian, i.e. UTF-8 order.
                    value = (value << ((4 - len) * 8)).to_be();

                    USTPUTC!(crate::parser::CTLMBCHAR, out);
                    USTPUTC!(len, out);
                    STADJUST!(mboff, out);
                    ptr::copy_nonoverlapping(
                        &value as *const c_uint as *const u8,
                        out as *mut u8,
                        4,
                    );
                    STADJUST!(len, out);
                    USTPUTC!(len, out);
                    USTPUTC!(crate::parser::CTLMBCHAR, out);
                    STADJUST!(mboff, out);

                    /* The highest byte the block above touches, counted from
                     * `out0`: the four encoded bytes end at `2 + mboff + 3`
                     * and the closing pair at `2 + mboff + len + 1`.  It is
                     * past the length this returns, so every caller has to
                     * have reserved `CONV_ESCAPE_SLOP` and not the C's 4. */
                    let highest = 2 + mboff + if len + 1 > 3 { len + 1 } else { 3 };
                    debug_assert!(highest >= 0 && (highest as usize) < CONV_ESCAPE_SLOP);
                }

                break 'out_noput; /* goto out_noput */
            }
        }

        if goto_check_value {
            // check_value:
            // if (SQSYNTAX[(signed char)value] != CCTL) break;
            if crate::syntax::sqsyntax
                [(crate::syntax::SYNBASE as isize + (value as i8) as isize) as usize]
                as c_int
                != crate::syntax::CCTL as c_int
            {
                goto_backslash = false;
            } else {
                /* fall through */
                goto_backslash = true;
            }
        }

        if goto_backslash {
            // case '\\':
            if mbchar {
                USTPUTC!(crate::parser::CTLESC, out);
            }
        }

        USTPUTC!(value, out);
    }

    // out_noput:
    str = str.add(1);
    (out.offset_from(out0) as c_uint) | ((str.offset_from(str0) as c_uint) << 4)
}

/// `PRIdMAX` on glibc/LP64 — `sizeof(PRIdMAX)` counts the NUL.
const PRIdMAX: &core::ffi::CStr = c"ld";
const SIZEOF_PRIdMAX: usize = 3;

// [spec:dash:def:printf.mklong-fn]
// [spec:dash:sem:printf.mklong-fn]
unsafe fn mklong(dest: &mut BString, str: *const c_char, ch: *const c_char) -> *mut c_char {
    /*
     * Replace a string like "%92.3u" with "%92.3"PRIuMAX.
     *
     * Although C99 does not guarantee it, we assume PRIiMAX,
     * PRIoMAX, PRIuMAX, PRIxMAX, and PRIXMAX are all the same
     * as PRIdMAX with the final 'd' replaced by the corresponding
     * character.
     */

    let len: usize;

    len = ch.offset_from(str) as usize + SIZEOF_PRIdMAX;
    /* `STARTSTACKSTR(copy); copy = makestrspace(len, copy)`.  The C hands
     * back a pointer into the block, live until the next `stalloc`; the
     * caller uses it as the format of the very next `printf` and nothing
     * allocates in between.  Here it is the caller's buffer, live for the
     * whole conversion. */
    debug_assert_eq!(PRIdMAX.to_bytes_with_nul().len(), SIZEOF_PRIdMAX);
    dest.clear();
    dest.extend_from_slice(core::slice::from_raw_parts(
        str as *const u8,
        len - SIZEOF_PRIdMAX,
    ));
    dest.extend_from_slice(PRIdMAX.to_bytes_with_nul());
    dest[len - 2] = *ch as u8;
    dest.as_mut_ptr() as *mut c_char
}

// [spec:dash:def:printf.getchr-fn]
// [spec:dash:sem:printf.getchr-fn]
unsafe fn getchr() -> c_int {
    let mut val: c_int = 0;

    if !(*gargv).is_null() {
        // val = **gargv++;
        val = **gargv as c_int;
        gargv = gargv.add(1);
    }
    val
}

// [spec:dash:def:printf.getstr-fn]
// [spec:dash:sem:printf.getstr-fn]
unsafe fn getstr() -> *mut c_char {
    let mut val: *mut c_char = nullstr();

    if !(*gargv).is_null() {
        val = *gargv;
        gargv = gargv.add(1);
    }
    val
}

// [spec:dash:def:printf.getuintmax-fn]
// [spec:dash:sem:printf.getuintmax-fn]
unsafe fn getuintmax(sign: c_int) -> uintmax_t {
    let mut val: uintmax_t = 0;
    let cp: *mut c_char;
    let mut ep: *mut c_char = ptr::null_mut();

    cp = *gargv;
    'out: {
        if cp.is_null() {
            break 'out; /* goto out */
        }
        gargv = gargv.add(1);

        val = *cp.add(1) as u8 as uintmax_t;
        if *cp == b'"' as c_char || *cp == b'\'' as c_char {
            break 'out; /* goto out */
        }

        *libc::__errno_location() = 0;
        val = if sign != 0 {
            strtoimax(cp, &mut ep, 0) as uintmax_t
        } else {
            strtoumax(cp, &mut ep, 0)
        };
        check_conversion(cp, ep);
    }
    // out:
    val
}

// [spec:dash:def:printf.getdouble-fn]
// [spec:dash:sem:printf.getdouble-fn]
unsafe fn getdouble() -> c_double {
    let val: c_double;
    let cp: *mut c_char;
    let mut ep: *mut c_char = ptr::null_mut();

    cp = *gargv;
    if cp.is_null() {
        return 0.0;
    }
    gargv = gargv.add(1);

    if *cp == b'"' as c_char || *cp == b'\'' as c_char {
        return *cp.add(1) as u8 as c_double;
    }

    *libc::__errno_location() = 0;
    val = libc::strtod(cp, &mut ep);
    check_conversion(cp, ep);
    val
}

// [spec:dash:def:printf.check-conversion-fn]
// [spec:dash:sem:printf.check-conversion-fn]
unsafe fn check_conversion(s: *const c_char, ep: *const c_char) {
    if *ep != 0 {
        if ep == s {
            warnx!(c"%s: expected numeric value".as_ptr(), s);
        } else {
            warnx!(c"%s: not completely converted".as_ptr(), s);
        }
        rval = 1;
    } else if *libc::__errno_location() == libc::ERANGE {
        warnx!(c"%s: %s".as_ptr(), s, libc::strerror(libc::ERANGE));
        rval = 1;
    }
}

// [spec:dash:def:printf.echocmd-fn]
// [spec:dash:sem:printf.echocmd-fn]
pub unsafe fn echocmd(argc: c_int, mut argv: *mut *mut c_char) -> c_int {
    let mut lastfmt: *const c_char = snlfmt();
    let mut nonl: c_int;

    argv = argv.add(1);
    if !(*argv).is_null() && libc::strcmp(*argv, c"-n".as_ptr()) == 0 {
        argv = argv.add(1);
        lastfmt = c"%s".as_ptr();
    }

    loop {
        let mut fmt: *const c_char = c"%s ".as_ptr();
        let s: *mut c_char = *argv;

        // if (!s || !*++argv) — `++argv` is not evaluated when s is NULL.
        if s.is_null() || {
            argv = argv.add(1);
            (*argv).is_null()
        } {
            fmt = lastfmt;
        }

        nonl = print_escape_str(
            fmt,
            ptr::null_mut(),
            ptr::null_mut(),
            if !s.is_null() { s } else { nullstr() },
        );

        if !(nonl == 0 && !(*argv).is_null()) {
            break;
        }
    }
    0
}
