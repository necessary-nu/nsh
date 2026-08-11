//! Literal port of `src/bltin/printf.c` — the `printf` and `echo`
//! builtins.
//! Rules: `docs/spec/port/src/bltin/printf.md`.
//!
//! `conv_escape` is declared in `system.h` and shared with `parser.c`
//! (which calls it with `mbchar = true`), so it is `pub` here and carries
//! both the `printf.*` and the `system.*` rule ids.
//!
//! ## The conversions
//!
//! The C builtin parses a conversion only far enough to find its end,
//! then hands the specification back to C's `printf` as a format string
//! and lets libc render it. This port renders in [`conv`], from Rust's
//! own formatting. That removes the three things the C arrangement
//! needed and this one does not: the `libc::snprintf` bridge with its
//! `PF`/`ASPF` arity switch, `mklong`'s rewrite of a specification to
//! `PRIdMAX`, and `print_escape_str`'s run of `X`s — which existed only
//! because a `%b` result can contain NUL and so could not be handed to a
//! C string function at all. Rendering over bytes has no such problem.
//!
//! Cross-module signatures assumed (see the port report):
//!   * Nothing from `crate::memalloc`.  The buffers this file deals in —
//!     the escaped string and each rendered conversion — are all owned,
//!     so `USTPUTC`/`STADJUST` below are the two macros `conv_escape`
//!     still needs to write through a bare cursor and touch no region.
//!   * `crate::output::stdout`, which is an `io::Write`.
//!   * `crate::error::{sh_error!, sh_warnx!}` via `bltin.h`'s aliases
//!   * `crate::mystring::{nullstr, snlfmt}` — `char nullstr[1]`
//!     (src/shell.h:74) and `const char snlfmt[]` (src/mystring.h:52)
//!   * `crate::options::{nextopt, argptr}`
//!   * `crate::parser::{CTLESC, CTLMBCHAR}` (src/parser.h:43,47)
//!   * `crate::syntax::{sqsyntax, SYNBASE, CCTL}` — the generated syntax
//!     tables; `SQSYNTAX` is `sqsyntax + SYNBASE` with `SYNBASE` 129
//!     (src/mksyntax.c:147,152)

use core::ffi::CStr;
use core::ptr;
use libc::{c_char, c_double, c_int, c_uint, intmax_t, uintmax_t};
use std::io::Write as _;

use bstr::BString;

mod conv;

use conv::Spec;

// glibc extensions/C99 functions the `libc` crate does not reliably expose.
//
// `strtoimax`/`strtoumax`: glibc >= 2.38 redirects both through
// `__isoc23_*` for any translation unit that enables the C23 strtol
// semantics (`__GLIBC_USE (C23_STRTOL)`), which is the case for the dash
// build; those variants additionally accept `0b`/`0B` binary constants
// when the base is 0 or 2.  The C `printf.c` therefore really calls
// `__isoc23_strtoimax`, so `printf %d 0b11` prints 3 — link the same
// symbols here or the port silently loses binary literals.
unsafe extern "C" {
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

const SKIP1: &core::ffi::CStr = c"#-+ 0";
const SKIP2: &core::ffi::CStr = c"*0123456789";

/// Write one rendered conversion to standard output.
unsafe fn emit(bytes: &[u8]) {
    let _ = (&mut *crate::output::stdout()).write_all(bytes);
}

/// The number at the front of a `SKIP2` run.
///
/// The C skipped the run without reading it, because printf would parse
/// the digits again from the same bytes. The run may hold a `*` the C's
/// own width branch already declined to treat as one — `%5*d` — and the
/// number ends there, exactly where a C conversion's would.
unsafe fn leading_number(at: *const c_char, len: usize) -> c_int {
    let mut value: c_int = 0;
    for offset in 0..len {
        let byte = *at.add(offset) as u8;
        if !byte.is_ascii_digit() {
            break;
        }
        /* An over-long width is a field nobody can print; saturating
         * keeps it merely enormous rather than wrapping it negative. */
        value = value
            .saturating_mul(10)
            .saturating_add((byte - b'0') as c_int);
    }
    value
}

// [spec:dash:def:printf.print-escape-str-fn]
// [spec:dash:sem:printf.print-escape-str-fn]
unsafe fn print_escape_str(f: *const c_char, spec: &Spec, s: *mut c_char) -> c_int {
    let done: c_int;
    /* The C's `q` is a cursor into the stack block and `stackblock()` its
     * base.  Both are this buffer: `len` is its length and `q[-1]` its
     * last byte. */
    let mut buf = BString::default();

    done = conv_escape_str(s, &mut buf);
    let len = buf.len();

    /* `conv_escape_str`'s do-while exits only on the iteration that writes
     * the terminating NUL, so `q[-1]` always exists. */
    debug_assert!(len >= 1);

    /* `q[-1] = (!!((f[1] - 's') | done) - 1) & f[2];`
     *
     * The mask is all-ones only when the conversion character sits right
     * after the `%` *and* no `\c` stopped the conversion, so the byte
     * appended in place of the terminator is `echo`'s own separator —
     * the space between arguments or the closing newline. Every route in
     * from `printfcmd` has a NUL there and appends nothing. */
    let close = (((((*f.add(1) as c_int - b's' as c_int) | done) != 0) as c_int - 1)
        & *f.add(2) as c_int) as u8;
    buf[len - 1] = close;
    let total = len - 1 + (close != 0) as usize;

    /* The C could not lay this out itself: a `%b` result may contain NUL,
     * so it formatted a run of `X`s of the same length and copied the real
     * bytes back over them afterwards. Rendering over bytes needs no
     * stand-in — and with nothing to lay out, a bare specification is
     * still just the converted string. */
    if spec.is_bare() {
        emit(&buf[..total]);
    } else {
        emit(&spec.string(&buf[..len - 1]));
    }

    done
}

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
        crate::error::sh_error(b"usage: printf format [arg ...]");
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

                let start: *mut c_char;
                let nextch: c_char;
                let mut spec = Spec::bare();

                if ch == b'\\' as c_int {
                    let ret: c_uint;
                    /* `STARTSTACKSTR(cp); CHECKSTRSPACE(4, cp)` — one
                     * escape's worth of scratch and nothing else; see
                     * `CONV_ESCAPE_SLOP` for why 4 is not the bound. */
                    let mut cp: [c_char; CONV_ESCAPE_SLOP] = [0; CONV_ESCAPE_SLOP];

                    ret = conv_escape(fmt, cp.as_mut_ptr(), false);
                    fmt = fmt.add((ret >> 4) as usize);
                    debug_assert!((ret & 15) as usize <= CONV_ESCAPE_SLOP);
                    emit(core::slice::from_raw_parts(
                        cp.as_ptr() as *const u8,
                        (ret & 15) as usize,
                    ));
                    continue;
                }
                if ch != b'%' as c_int
                    || (*fmt == b'%' as c_char && {
                        fmt = fmt.add(1);
                        true
                    })
                {
                    emit(&[ch as u8]);
                    continue;
                }

                /* Ok - we've found a format specification.  Save its
                address for the diagnostic, and collect it as we go: the C
                only had to find the end, because it handed the text
                itself to printf. */
                start = fmt.sub(1);

                /* skip to field width */
                let flags = libc::strspn(fmt, SKIP1.as_ptr());
                for offset in 0..flags {
                    spec.flag(*fmt.add(offset) as u8);
                }
                fmt = fmt.add(flags);
                if *fmt == b'*' as c_char {
                    fmt = fmt.add(1);
                    spec.set_width(getuintmax(1) as c_int);
                } else {
                    /* skip to possible '.',
                     * get following precision
                     */
                    let digits = libc::strspn(fmt, SKIP2.as_ptr());
                    spec.set_width(leading_number(fmt, digits));
                    fmt = fmt.add(digits);
                }

                if *fmt == b'.' as c_char {
                    fmt = fmt.add(1);
                    if *fmt == b'*' as c_char {
                        fmt = fmt.add(1);
                        spec.set_precision(getuintmax(1) as c_int);
                    } else {
                        let digits = libc::strspn(fmt, SKIP2.as_ptr());
                        spec.set_precision(leading_number(fmt, digits));
                        fmt = fmt.add(digits);
                    }
                }

                ch = *fmt as c_int;
                if ch == 0 {
                    crate::error::sh_error(b"missing format character");
                }
                /* null terminate format string to we can use it
                as an argument to printf. */
                nextch = *fmt.add(1);
                *fmt.add(1) = 0;
                match ch as u8 {
                    b'b' => {
                        /* The C rewrote the `b` to an `s` here so that its
                         * printf would accept the specification; nothing
                         * reads the conversion character now. */
                        /* escape if a \c was encountered */
                        if print_escape_str(start, &spec, getstr()) != 0 {
                            break 'out; /* goto out */
                        }
                    }
                    b'c' => {
                        let p: c_int = getchr();
                        emit(&spec.character(p));
                    }
                    b's' => {
                        let p: *mut c_char = getstr();
                        emit(&spec.string(CStr::from_ptr(p).to_bytes()));
                    }
                    /* `mklong` widened the specification to `PRIdMAX` so
                     * that C's printf would read a whole `intmax_t` off
                     * the varargs. The value arrives typed here. */
                    b'd' | b'i' => {
                        let p: uintmax_t = getuintmax(1);
                        emit(&spec.signed(p as i64));
                    }
                    b'o' | b'u' | b'x' | b'X' => {
                        let p: uintmax_t = getuintmax(0);
                        emit(&spec.unsigned(p, ch as u8));
                    }
                    b'a' | b'A' | b'e' | b'E' | b'f' | b'F' | b'g' | b'G' => {
                        let p: c_double = getdouble();
                        emit(&spec.double(p, ch as u8));
                    }
                    _ => {
                        let mut message = Vec::new();
                        message.extend_from_slice(CStr::from_ptr(start).to_bytes());
                        message.extend_from_slice(b": invalid directive");
                        crate::error::sh_error(&message);
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
        let mut message = Vec::new();
        message.extend_from_slice(CStr::from_ptr(s).to_bytes());
        if ep == s {
            message.extend_from_slice(b": expected numeric value");
        } else {
            message.extend_from_slice(b": not completely converted");
        }
        crate::error::sh_warnx(&message);
        rval = 1;
    } else if *libc::__errno_location() == libc::ERANGE {
        let mut message = Vec::new();
        message.extend_from_slice(CStr::from_ptr(s).to_bytes());
        message.extend_from_slice(b": ");
        message.extend_from_slice(CStr::from_ptr(libc::strerror(libc::ERANGE)).to_bytes());
        crate::error::sh_warnx(&message);
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

        /* echo's three formats are `%s`, `%s ` and `%s\n`: a bare
         * conversion, whose trailing byte `print_escape_str` appends as
         * the separator. */
        nonl = print_escape_str(fmt, &Spec::bare(), if !s.is_null() { s } else { nullstr() });

        if !(nonl == 0 && !(*argv).is_null()) {
            break;
        }
    }
    0
}
