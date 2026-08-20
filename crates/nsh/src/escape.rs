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

use bstr::{BStr, BString};

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
pub(crate) fn isodigit(byte: u8) -> bool {
    matches!(byte, b'0'..=b'7')
}

/// `#define octtobin(c) ((c) - '0')`
#[inline]
fn octtobin(byte: u8) -> u32 {
    u32::from(byte - b'0')
}

// Character constants used as `match` patterns; Rust cannot cast inside a
// pattern the way a C `case` label can.
const CH_BACKSLASH: u8 = b'\\';
const CH_X: u8 = b'x';
const CH_U: u8 = b'u';
const CH_A: u8 = b'a';
const CH_B: u8 = b'b';
const CH_F: u8 = b'f';
const CH_E: u8 = b'e';
const CH_N: u8 = b'n';
const CH_R: u8 = b'r';
const CH_T: u8 = b't';
const CH_V: u8 = b'v';

/// The bytes written and input bytes consumed by one escape conversion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EscapeChunk {
    pub written: usize,
    pub consumed: usize,
}

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
pub fn conv_escape(input: &[u8], out: &mut [u8; CONV_ESCAPE_SLOP], mbchar: bool) -> EscapeChunk {
    /* The C's `out`, as the offset it always was. */
    let mut o: usize = 0;
    let mut at: isize = 0;
    let mut value: u32;
    let och: u8;
    let mut ch: u8;

    let byte_at = |at: isize| -> u8 {
        usize::try_from(at)
            .ok()
            .and_then(|index| input.get(index))
            .copied()
            .unwrap_or(0)
    };
    ch = byte_at(at);
    value = u32::from(ch);

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
                    value = value.wrapping_sub(u32::from(b'a'));
                    value = value.wrapping_add(0x07 /* '\a' */);
                }

                CH_E => value = 0o33,   /* <ESC> */
                CH_N => value = 0o12,   /* newline */
                CH_R => value = 0o15,   /* carriage-return */
                CH_T => value = 0o11,   /* tab */
                CH_V => value = 0o13,   /* vertical-tab */

                _ => {
                    // default:
                    if mbchar && (ch == b'"' || ch == b'\'') {
                        break 'sw;
                    }

                    if ch == b'U' {
                        ch = 8;
                        goto_hex = true;
                        break 'sw;
                    }

                    value = u32::from(b'\\');

                    if isodigit(ch) {
                        ch = 3;
                        value = 0;
                        loop {
                            value <<= 3;
                            value = value.wrapping_add(octtobin(byte_at(at)));
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
                let c = byte_at(at);
                let d: u32;

                if c.is_ascii_digit() {
                    d = u32::from(c - b'0');
                } else {
                    let cl: u8;

                    cl = c & !0x20;
                    if matches!(cl, b'A'..=b'F') {
                        d = u32::from(cl - b'A') + 10;
                    } else {
                        at -= 1;
                        break;
                    }
                }

                value <<= 4;
                value = value.wrapping_add(d);

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
                    let mboff: isize = if mbchar { 0 } else { -2 };
                    let uni: u32 = value;
                    let len: usize;

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
                    let highest = 2 + mboff + isize::try_from((len + 1).max(3)).unwrap();
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
    EscapeChunk {
        written: o,
        consumed: at as usize,
    }
}

/*
 * Print SysV echo(1) style escape string
 *	Halts processing string if a \c escape is encountered.
 */
/// Expand a whole string's escapes into `cp`, in the dialect `echo` and
/// `printf`'s `%b` share.
///
/// Returns 0, or 0x100 when a `\c` was found — "stop all further output",
/// which both callers obey. Input and output are both length-delimited.
// [spec:dash:def:printf.conv-escape-str-fn]
// [spec:dash:sem:printf.conv-escape-str-fn]
pub(crate) fn conv_escape_str(input: &[u8], cp: &mut BString) -> bool {
    let mut at = 0usize;
    let byte_at = |index: usize| -> u8 { input.get(index).copied().unwrap_or(0) };

    /* convert string into a temporary buffer... */
    /* `STARTSTACKSTR(cp)` — the buffer is the caller's, and the C's `*sp =
     * cp` at the end is its length. */
    debug_assert!(cp.is_empty());

    while at < input.len() {
        let ret: EscapeChunk;
        let ch: u8;

        /* `CHECKSTRSPACE(4, cp)` — the room `conv_escape` writes into
         * through the raw cursor below; see `CONV_ESCAPE_SLOP`. */
        cp.reserve(CONV_ESCAPE_SLOP);

        let c = byte_at(at);
        at += 1;
        if c != b'\\' {
            cp.push(c);
            continue;
        } else {
            ch = byte_at(at);
            if ch == b'c' {
                return true;
            }
        }

        /*
         * %b string octal constants are not like those in C.
         * They start with a \0, and are followed by 0, 1, 2,
         * or 3 octal digits.
         */
        if ch == b'0' && isodigit(byte_at(at + 1)) {
            at += 1;
        }

        /* Finally test for sequences valid in the format string */
        let mut scratch: [u8; CONV_ESCAPE_SLOP] = [0; CONV_ESCAPE_SLOP];
        ret = conv_escape(&input[at.min(input.len())..], &mut scratch, false);
        at += ret.consumed;
        debug_assert!(ret.written <= CONV_ESCAPE_SLOP);
        cp.extend_from_slice(&scratch[..ret.written]);
    }

    false
}

/// Quote arbitrary bytes so parsing the result produces the same bytes.
// [spec:dash:def:mystring.single-quote-fn]
// [spec:dash:sem:mystring.single-quote-fn]
// [spec:nsh:req:idiom.no-mystring]
pub(crate) fn shell_quote(mut input: &BStr) -> BString {
    let mut quoted = BString::new(Vec::new());
    loop {
        let ordinary = input
            .iter()
            .position(|&byte| byte == b'\'')
            .unwrap_or(input.len());
        quoted.push(b'\'');
        quoted.extend_from_slice(&input[..ordinary]);
        quoted.push(b'\'');
        input = &input[ordinary..];

        let quotes = input
            .iter()
            .position(|&byte| byte != b'\'')
            .unwrap_or(input.len());
        if quotes == 0 {
            break;
        }
        quoted.push(b'"');
        quoted.extend_from_slice(&input[..quotes]);
        quoted.push(b'"');
        input = &input[quotes..];
        if input.is_empty() {
            break;
        }
    }
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_strings_preserve_nul_data() {
        let mut output = BString::new(Vec::new());
        assert!(!conv_escape_str(b"a\0b", &mut output));
        assert_eq!(output, BString::from(b"a\0b".as_slice()));
    }

    // [spec:dash:sem:mystring.single-quote-fn/test]
    #[test]
    fn shell_quote_is_requotable() {
        assert_eq!(shell_quote(BStr::new(b"abc")), b"'abc'".as_slice());
        assert_eq!(shell_quote(BStr::new(b"")), b"''".as_slice());
        assert_eq!(shell_quote(BStr::new(b"a'b")), b"'a'\"'\"'b'".as_slice());
        assert_eq!(shell_quote(BStr::new(b"'")), b"''\"'\"".as_slice());
        assert_eq!(shell_quote(BStr::new(b"a b|c$d")), b"'a b|c$d'".as_slice());
    }

    // [spec:dash:sem:mystring.single-quote-fn/test]
    #[test]
    fn shell_quote_handles_all_bytes() {
        for byte in 1_u8..=u8::MAX {
            let actual = shell_quote(BStr::new(&[byte]));
            if byte == b'\'' {
                assert_eq!(actual, b"''\"'\"".as_slice());
            } else {
                assert_eq!(actual.as_slice(), [b'\'', byte, b'\'']);
            }
        }
    }
}
