//! The C escape decoder, shared by `echo`, `printf` and the parser.
//!
//! Port of the escape half of `src/bltin/printf.c`; `conv_escape` is
//! declared in `system.h` and shared with `parser.c` (which calls it with
//! `mbchar = true`), so it carries both the `printf.*` and the `system.*`
//! rule ids.
//!
//! It lives here rather than inside `builtins::echo` because two callers
//! is what shared means: a decoder the parser needs cannot sit inside a
//! builtin without the parser depending on one. `conv_escape_str` is here
//! for the same reason -- `echo`'s words and `printf`'s `%b` argument are
//! the same dialect, the one where an octal escape is written `\0nnn`.
//!
//! Cross-module signatures assumed (see the port report):
//!   * Nothing from `crate::memalloc`.  The one buffer this file deals in
//!     is owned, so `USTPUTC`/`STADJUST` below are the two macros
//!     `conv_escape` still needs to write through a bare cursor and touch
//!     no region.
//!   * `crate::parser::{CTLESC, CTLMBCHAR}` (src/parser.h:43,47)
//!   * `crate::syntax::SyntaxContext` for bytes that need framing inside
//!     single quotes.
//!     (src/mksyntax.c:147,152)

use bstr::BString;
use core::ffi::{c_char, c_int, c_uint};

// ---------------------------------------------------------------------
// src/memalloc.h:78-97 -- the two stack-string macros `conv_escape` still
// needs.  Both are pure cursor arithmetic; neither touches the region,
// and both are now written over a buffer and an offset rather than a raw
// pointer, so the bound they write within is checked.
// ---------------------------------------------------------------------

///
/// The C guards both of its calls with `CHECKSTRSPACE(4, cp)`, and 4 is not
/// enough. `\U0001F600` takes the `len == 4` arm, which writes the four
/// encoded bytes at `out + mboff` and *then* `USTPUTC(len, out);
/// USTPUTC(CTLMBCHAR, out)` at `out + len` and `out + len + 1` — bytes 5 and
/// 6 with `mbchar` false, bytes 7 and 8 with it true. It returns before them,
/// so they are scratch the next write overwrites; but they are written, and
/// in a 504-byte stack block nobody notices. A fixed buffer is exactly as
/// long as it is declared to be, so the port has to size it by what the C
/// *writes* rather than by what the C says.
///
/// This is the size of `conv_escape`'s destination rather than an amount
/// callers must remember to reserve, which is why every call site can now
/// pass a `[u8; CONV_ESCAPE_SLOP]` and stop thinking about it.
pub const CONV_ESCAPE_SLOP: usize = 8;

/// `#define USTPUTC(c, p) (*p++ = (c))`, over a buffer and an offset.
macro_rules! USTPUTC {
    ($c:expr, $buf:ident, $o:ident) => {{
        $buf[$o] = $c as u8;
        $o += 1;
    }};
}

/// `#define STADJUST(amount, p) (p += (amount))`.
///
/// The amount is signed and is genuinely negative here — `mboff` is -2 in
/// the non-`mbchar` case — so the arithmetic is done in `isize` and the
/// result is asserted back into range rather than wrapped.
macro_rules! STADJUST {
    ($amount:expr, $o:ident) => {{
        let adjusted = $o as isize + ($amount) as isize;
        debug_assert!(adjusted >= 0, "the escape cursor stays inside its scratch");
        $o = adjusted as usize;
    }};
}

/// `#define isodigit(c) ((c) >= '0' && (c) <= '7')`
#[inline]
pub(crate) fn isodigit(c: c_int) -> bool {
    c >= b'0' as c_int && c <= b'7' as c_int
}

/// `#define octtobin(c) ((c) - '0')`
#[inline]
fn octtobin(c: c_int) -> c_int {
    c - b'0' as c_int
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
/// The destination is a fixed scratch buffer rather than a raw cursor,
/// and its size is the one this function needs.
///
/// That turns a comment into a type. The `\u` arm writes *above* the
/// length it reports — four encoded bytes where `len` are counted, plus a
/// closing pair — so every caller had to reserve [`CONV_ESCAPE_SLOP`]
/// rather than the C's 4, and a caller that read the C and reserved 4
/// would have corrupted whatever followed. Nothing in a signature said
/// so; the `debug_assert` at the end of that arm was the only guard, and
/// only in a debug build. An `&mut [u8; CONV_ESCAPE_SLOP]` says it at
/// every call site, in every profile.
///
/// The cursor is an index, which also makes the backward `STADJUST` --
/// `mboff` is -2 when `!mbchar`, so the framing bytes are deliberately
/// overwritten by the payload -- ordinary arithmetic instead of pointer
/// arithmetic that happens to stay in bounds.
// [spec:nsh:req:idiom.lexer-tokens]
pub fn conv_escape(input: &[u8], out: &mut [u8; CONV_ESCAPE_SLOP], mbchar: bool) -> c_uint {
    /* The C's `out`, as the offset it always was. */
    let mut o: usize = 0;
    let mut at: isize = 0;
    let mut value: c_uint;
    let och: c_int;
    let mut ch: c_int;

    let byte_at = |at: isize| -> c_int {
        usize::try_from(at)
            .ok()
            .and_then(|index| input.get(index))
            .copied()
            .unwrap_or(0) as c_char as c_int
    };
    ch = byte_at(at);
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
                            value = value.wrapping_add(octtobin(byte_at(at)) as c_uint);
                            at += 1;
                            ch -= 1;
                            if !(ch != 0 && isodigit(byte_at(at))) {
                                break;
                            }
                        }
                    }

                    at -= 1;

                    goto_check_value = true;
                }
            }
        }

        if goto_hex {
            // hex:
            och = ch;
            value = 0;
            loop {
                at += 1;
                let c: c_int = byte_at(at);
                let d: c_int;

                if c >= b'0' as c_int && c <= b'9' as c_int {
                    d = c - b'0' as c_int;
                } else {
                    let cl: c_int;

                    cl = c & !0x20;
                    if cl >= b'A' as c_int && cl <= b'F' as c_int {
                        d = cl - b'A' as c_int + 10;
                    } else {
                        at -= 1;
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

                    USTPUTC!(crate::parser::CTLMBCHAR, out, o);
                    USTPUTC!(len, out, o);
                    STADJUST!(mboff, o);
                    /* `memcpy(out, &value, 4)` — four bytes written where
                     * `len` are counted, which is the whole reason the
                     * scratch has to be bigger than the return value. */
                    out[o..o + 4].copy_from_slice(&value.to_ne_bytes());
                    STADJUST!(len, o);
                    USTPUTC!(len, out, o);
                    USTPUTC!(crate::parser::CTLMBCHAR, out, o);
                    STADJUST!(mboff, o);

                    /* The highest byte the block above touches, counted from
                     * the start of `out`: the four encoded bytes end at
                     * `2 + mboff + 3` and the closing pair at
                     * `2 + mboff + len + 1`.  It is past the length this
                     * returns, which is why the scratch is `CONV_ESCAPE_SLOP`
                     * and not the C's 4.  The assertion stays as
                     * documentation; the indexing above now enforces it in
                     * every profile rather than only in a debug build. */
                    let highest = 2 + mboff + if len + 1 > 3 { len + 1 } else { 3 };
                    debug_assert!(highest >= 0 && (highest as usize) < CONV_ESCAPE_SLOP);
                }

                break 'out_noput; /* goto out_noput */
            }
        }

        if goto_check_value {
            // check_value:
            if crate::syntax::SyntaxContext::SingleQuoted
                .classify(crate::syntax::InputUnit::Byte(value as u8))
                != crate::syntax::SyntaxClass::Control
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
                USTPUTC!(crate::parser::CTLESC, out, o);
            }
        }

        USTPUTC!(value, out, o);
    }

    // out_noput:
    at += 1;
    debug_assert!(at >= 0, "an escape never consumes a negative byte count");
    (o as c_uint) | ((at as c_uint) << 4)
}

/*
 * Print SysV echo(1) style escape string
 *	Halts processing string if a \c escape is encountered.
 */
/// Expand a whole string's escapes into `cp`, in the dialect `echo` and
/// `printf`'s `%b` share.
///
/// Returns 0, or 0x100 when a `\c` was found — "stop all further output",
/// which both callers obey. The value's low byte is 0, which is also what
/// ends the loop, and what `cp`'s final byte becomes: the terminator the
/// caller either overwrites with a separator or trims.
// [spec:dash:def:printf.conv-escape-str-fn]
// [spec:dash:sem:printf.conv-escape-str-fn]
pub(crate) fn conv_escape_str(input: &[u8], cp: &mut BString) -> c_int {
    let mut c: c_int;
    let mut at = 0usize;
    let byte_at =
        |index: usize| -> c_int { input.get(index).copied().unwrap_or(0) as c_char as c_int };

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

        c = byte_at(at);
        at += 1;
        if c != b'\\' as c_int {
            ch = 0; /* unused on this path */
            goto_putchar = true;
        } else {
            ch = byte_at(at);
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
            if ch == b'0' as c_int && isodigit(byte_at(at + 1)) {
                at += 1;
            }

            /* Finally test for sequences valid in the format string */
            /* The C lets `conv_escape` write into the stack block past
             * the cursor and then commits part of it. Here it writes into
             * scratch and the committed prefix is appended, which is the
             * same bytes and the same length -- what the C left above the
             * cursor for the next write to overwrite is simply not copied
             * out. */
            let mut scratch: [u8; CONV_ESCAPE_SLOP] = [0; CONV_ESCAPE_SLOP];
            ret = conv_escape(&input[at.min(input.len())..], &mut scratch, false);
            at += (ret >> 4) as usize;
            debug_assert!((ret & 15) as usize <= CONV_ESCAPE_SLOP);
            cp.extend_from_slice(&scratch[..(ret & 15) as usize]);
        }

        // } while (c & 0xff);
        if (c & 0xff) == 0 {
            break;
        }
    }

    c
}
