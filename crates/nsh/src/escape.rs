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
pub const ESCAPE_OUTPUT_CAPACITY: usize = 8;

fn append_output_byte(byte: u8, output: &mut [u8], output_index: &mut usize) {
    output[*output_index] = byte;
    *output_index += 1;
}

/// `#define STADJUST(amount, p) (p += (amount))`.
///
/// The amount is signed and is genuinely negative here — `mboff` is -2 in
/// the non-`mbchar` case — so the arithmetic is done in `isize` and the
/// result is asserted back into range rather than wrapped.
fn adjust_output_index(output_index: &mut usize, amount: isize) {
    let adjusted = *output_index as isize + amount;
    debug_assert!(adjusted >= 0, "the escape cursor stays inside its scratch");
    *output_index = adjusted as usize;
}

/// `#define isodigit(c) ((c) >= '0' && (c) <= '7')`
#[inline]
pub(crate) fn is_octal_digit(byte: u8) -> bool {
    matches!(byte, b'0'..=b'7')
}

/// `#define octtobin(c) ((c) - '0')`
#[inline]
fn octal_digit_value(byte: u8) -> u32 {
    u32::from(byte - b'0')
}

// Character constants used as `match` patterns; Rust cannot cast inside a
// pattern the way a C `case` label can.
const BACKSLASH: u8 = b'\\';
const LOWER_X: u8 = b'x';
const LOWER_U: u8 = b'u';
const LOWER_A: u8 = b'a';
const LOWER_B: u8 = b'b';
const LOWER_F: u8 = b'f';
const LOWER_E: u8 = b'e';
const LOWER_N: u8 = b'n';
const LOWER_R: u8 = b'r';
const LOWER_T: u8 = b't';
const LOWER_V: u8 = b'v';

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
pub fn parse_escape(
    input: &[u8],
    output: &mut [u8; ESCAPE_OUTPUT_CAPACITY],
    preserve_multibyte_framing: bool,
) -> EscapeChunk {
    /* The C's `out`, as the offset it always was. */
    let mut output_index: usize = 0;
    let mut at: isize = 0;
    let mut value: u32;
    let digit_limit: u8;
    let mut character: u8;

    let byte_at = |at: isize| -> u8 {
        usize::try_from(at)
            .ok()
            .and_then(|index| input.get(index))
            .copied()
            .unwrap_or(0)
    };
    character = byte_at(at);
    value = u32::from(character);

    // The C switch's `default:` label falls into `check_value:`, which falls
    // into `case '\\':`; `case 'x':` falls into `hex:`, which can jump back
    // to `check_value:` or forward to `out_noput:`. The three flags below
    // encode those gotos; the blocks stay in source order except that the
    // `default:` arm has to move last, as Rust requires.
    let mut parse_hex_escape = false;
    let mut validate_value = false;
    let mut emit_backslash = false;

    'skip_output: {
        'dispatch_complete: {
            match character {
                BACKSLASH => {
                    emit_backslash = true;
                }

                LOWER_X => {
                    character = 2;
                    parse_hex_escape = true;
                }

                LOWER_U => {
                    character = 4;
                    parse_hex_escape = true;
                }

                LOWER_A /* alert */ | LOWER_B /* backspace */ | LOWER_F /* form-feed */ => {
                    value = value.wrapping_sub(u32::from(b'a'));
                    value = value.wrapping_add(0x07 /* '\a' */);
                }

                LOWER_E => value = 0o33,   /* <ESC> */
                LOWER_N => value = 0o12,   /* newline */
                LOWER_R => value = 0o15,   /* carriage-return */
                LOWER_T => value = 0o11,   /* tab */
                LOWER_V => value = 0o13,   /* vertical-tab */

                _ => {
                    // default:
                    if preserve_multibyte_framing && (character == b'"' || character == b'\'') {
                        break 'dispatch_complete;
                    }

                    if character == b'U' {
                        character = 8;
                        parse_hex_escape = true;
                        break 'dispatch_complete;
                    }

                    value = u32::from(b'\\');

                    if is_octal_digit(character) {
                        character = 3;
                        value = 0;
                        loop {
                            value <<= 3;
                            value = value.wrapping_add(octal_digit_value(byte_at(at)));
                            at += 1;
                            character -= 1;
                            if !(character != 0 && is_octal_digit(byte_at(at))) {
                                break;
                            }
                        }
                    }

                    at -= 1;

                    validate_value = true;
                }
            }
        }

        if parse_hex_escape {
            // hex:
            digit_limit = character;
            value = 0;
            loop {
                at += 1;
                let byte = byte_at(at);
                let digit: u32;

                if byte.is_ascii_digit() {
                    digit = u32::from(byte - b'0');
                } else {
                    let uppercase_byte: u8;

                    uppercase_byte = byte & !0x20;
                    if matches!(uppercase_byte, b'A'..=b'F') {
                        digit = u32::from(uppercase_byte - b'A') + 10;
                    } else {
                        at -= 1;
                        break;
                    }
                }

                value <<= 4;
                value = value.wrapping_add(digit);

                character -= 1;
                if character == 0 {
                    break;
                }
            }

            if digit_limit <= 2 {
                validate_value = true;
            } else if value < 0x80 {
                validate_value = true;
            } else {
                if value < 0x110000 {
                    let multibyte_offset: isize = if preserve_multibyte_framing { 0 } else { -2 };
                    let unicode_scalar: u32 = value;
                    let encoded_length: usize;

                    value = 0x80 << 8 | (value & 0xfc0) << 2 | 0x80 | (value & 0x3f);

                    if unicode_scalar < 0x800 {
                        value |= 0x40 << 8;
                        encoded_length = 2;
                    } else {
                        value |= 0x80 << 16 | (unicode_scalar & 0x3f000) << 4;
                        if unicode_scalar < 0x10000 {
                            value |= 0x60 << 16;
                            encoded_length = 3;
                        } else {
                            value |= 0xf0 << 24 | (unicode_scalar & !0x3ffff) << 6;
                            encoded_length = 4;
                        }
                    }

                    // htonl(): host order to big-endian, i.e. UTF-8 order.
                    value = (value << ((4 - encoded_length) * 8)).to_be();

                    append_output_byte(
                        crate::parser::LEGACY_MULTIBYTE as u8,
                        output,
                        &mut output_index,
                    );
                    append_output_byte(encoded_length as u8, output, &mut output_index);
                    adjust_output_index(&mut output_index, multibyte_offset);
                    /* `memcpy(out, &value, 4)` — four bytes written where
                     * `len` are counted, which is the whole reason the
                     * scratch has to be bigger than the return value. */
                    output[output_index..output_index + 4].copy_from_slice(&value.to_ne_bytes());
                    adjust_output_index(&mut output_index, encoded_length as isize);
                    append_output_byte(encoded_length as u8, output, &mut output_index);
                    append_output_byte(
                        crate::parser::LEGACY_MULTIBYTE as u8,
                        output,
                        &mut output_index,
                    );
                    adjust_output_index(&mut output_index, multibyte_offset);

                    /* The highest byte the block above touches, counted from
                     * the start of `out`: the four encoded bytes end at
                     * `2 + mboff + 3` and the closing pair at
                     * `2 + mboff + len + 1`.  It is past the length this
                     * returns, which is why the scratch is `CONV_ESCAPE_SLOP`
                     * and not the C's 4.  The assertion stays as
                     * documentation; the indexing above now enforces it in
                     * every profile rather than only in a debug build. */
                    let highest_written_index = 2
                        + multibyte_offset
                        + isize::try_from((encoded_length + 1).max(3)).unwrap();
                    debug_assert!(
                        highest_written_index >= 0
                            && (highest_written_index as usize) < ESCAPE_OUTPUT_CAPACITY
                    );
                }

                break 'skip_output; /* goto out_noput */
            }
        }

        if validate_value {
            // check_value:
            if crate::syntax::SyntaxContext::SingleQuoted
                .classify(crate::syntax::InputUnit::Byte(value as u8))
                != crate::syntax::SyntaxClass::Control
            {
                emit_backslash = false;
            } else {
                /* fall through */
                emit_backslash = true;
            }
        }

        if emit_backslash {
            // case '\\':
            if preserve_multibyte_framing {
                append_output_byte(
                    crate::parser::LEGACY_ESCAPE as u8,
                    output,
                    &mut output_index,
                );
            }
        }

        append_output_byte(value as u8, output, &mut output_index);
    }

    // out_noput:
    at += 1;
    debug_assert!(at >= 0, "an escape never consumes a negative byte count");
    EscapeChunk {
        written: output_index,
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
pub(crate) fn append_escape(input: &[u8], output_bytes: &mut BString) -> bool {
    let mut at = 0usize;
    let byte_at = |index: usize| -> u8 { input.get(index).copied().unwrap_or(0) };

    /* convert string into a temporary buffer... */
    /* `STARTSTACKSTR(cp)` — the buffer is the caller's, and the C's `*sp =
     * cp` at the end is its length. */
    debug_assert!(output_bytes.is_empty());

    while at < input.len() {
        let converted_escape: EscapeChunk;
        let character: u8;

        /* `CHECKSTRSPACE(4, cp)` — the room `conv_escape` writes into
         * through the raw cursor below; see `CONV_ESCAPE_SLOP`. */
        output_bytes.reserve(ESCAPE_OUTPUT_CAPACITY);

        let byte = byte_at(at);
        at += 1;
        if byte != b'\\' {
            output_bytes.push(byte);
            continue;
        } else {
            character = byte_at(at);
            if character == b'c' {
                return true;
            }
        }

        /*
         * %b string octal constants are not like those in C.
         * They start with a \0, and are followed by 0, 1, 2,
         * or 3 octal digits.
         */
        if character == b'0' && is_octal_digit(byte_at(at + 1)) {
            at += 1;
        }

        /* Finally test for sequences valid in the format string */
        let mut scratch: [u8; ESCAPE_OUTPUT_CAPACITY] = [0; ESCAPE_OUTPUT_CAPACITY];
        converted_escape = parse_escape(&input[at.min(input.len())..], &mut scratch, false);
        at += converted_escape.consumed;
        debug_assert!(converted_escape.written <= ESCAPE_OUTPUT_CAPACITY);
        output_bytes.extend_from_slice(&scratch[..converted_escape.written]);
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
        assert!(!append_escape(b"a\0b", &mut output));
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
