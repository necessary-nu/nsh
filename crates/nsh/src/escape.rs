//! Shell escape decoding shared by `echo`, `printf`, and dollar-single quotes.
//!
//! `echo` and `printf` use the same escape dialect, including the special
//! `\0nnn` spelling for octal bytes. Dollar-single quotes add quote escapes
//! and handle control-character notation in the parser.

use bstr::{BStr, BString};

#[inline]
pub(crate) fn is_octal_digit(byte: u8) -> bool {
    matches!(byte, b'0'..=b'7')
}

#[inline]
fn octal_digit_value(byte: u8) -> u32 {
    u32::from(byte - b'0')
}

/// The bytes written and input bytes consumed by one escape conversion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EscapeChunk {
    bytes: [u8; 4],
    length: u8,
    pub consumed: usize,
}

impl EscapeChunk {
    const fn one(byte: u8, consumed: usize) -> Self {
        Self {
            bytes: [byte, 0, 0, 0],
            length: 1,
            consumed,
        }
    }

    const fn empty(consumed: usize) -> Self {
        Self {
            bytes: [0; 4],
            length: 0,
            consumed,
        }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.length)]
    }
}

fn hexadecimal_digit_value(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some(u32::from(byte - b'0')),
        b'a'..=b'f' => Some(u32::from(byte - b'a') + 10),
        b'A'..=b'F' => Some(u32::from(byte - b'A') + 10),
        _ => None,
    }
}

fn hexadecimal_value(input: &[u8], maximum_digits: usize) -> (u32, usize) {
    let mut value = 0u32;
    let mut consumed = 0usize;
    for byte in input.iter().copied().take(maximum_digits) {
        let Some(digit) = hexadecimal_digit_value(byte) else {
            break;
        };
        value = (value << 4) | digit;
        consumed += 1;
    }
    (value, consumed)
}

fn unicode_bytes(value: u32, consumed: usize) -> EscapeChunk {
    if value < 0x80 {
        return EscapeChunk::one(value as u8, consumed);
    }
    if value >= 0x11_0000 {
        return EscapeChunk::empty(consumed);
    }

    let mut bytes = [0u8; 4];
    let length = if value < 0x800 {
        bytes[0] = 0xc0 | (value >> 6) as u8;
        bytes[1] = 0x80 | (value & 0x3f) as u8;
        2
    } else if value < 0x1_0000 {
        bytes[0] = 0xe0 | (value >> 12) as u8;
        bytes[1] = 0x80 | ((value >> 6) & 0x3f) as u8;
        bytes[2] = 0x80 | (value & 0x3f) as u8;
        3
    } else {
        bytes[0] = 0xf0 | (value >> 18) as u8;
        bytes[1] = 0x80 | ((value >> 12) & 0x3f) as u8;
        bytes[2] = 0x80 | ((value >> 6) & 0x3f) as u8;
        bytes[3] = 0x80 | (value & 0x3f) as u8;
        4
    };
    EscapeChunk {
        bytes,
        length,
        consumed,
    }
}

// [spec:dash:sem:printf.conv-escape-fn]
// [spec:dash:sem:system.conv-escape-fn]
// [spec:nsh:req:idiom.lexer-tokens]
pub fn parse_escape(input: &[u8], single_quoted: bool) -> EscapeChunk {
    let Some(&character) = input.first() else {
        return EscapeChunk::one(b'\\', 0);
    };

    match character {
        b'\\' => EscapeChunk::one(b'\\', 1),
        b'a' => EscapeChunk::one(0x07, 1),
        b'b' => EscapeChunk::one(0x08, 1),
        b'f' => EscapeChunk::one(0x0c, 1),
        b'e' => EscapeChunk::one(0x1b, 1),
        b'n' => EscapeChunk::one(b'\n', 1),
        b'r' => EscapeChunk::one(b'\r', 1),
        b't' => EscapeChunk::one(b'\t', 1),
        b'v' => EscapeChunk::one(0x0b, 1),
        b'\'' | b'"' if single_quoted => EscapeChunk::one(character, 1),
        b'x' => {
            let (value, digits) = hexadecimal_value(&input[1..], 2);
            EscapeChunk::one(value as u8, 1 + digits)
        }
        b'u' | b'U' => {
            let maximum_digits = if character == b'u' { 4 } else { 8 };
            let (value, digits) = hexadecimal_value(&input[1..], maximum_digits);
            unicode_bytes(value, 1 + digits)
        }
        b'0'..=b'7' => {
            let mut value = 0u32;
            let mut consumed = 0usize;
            for byte in input.iter().copied().take(3) {
                if !is_octal_digit(byte) {
                    break;
                }
                value = (value << 3) | octal_digit_value(byte);
                consumed += 1;
            }
            EscapeChunk::one(value as u8, consumed)
        }
        _ => EscapeChunk::one(b'\\', 0),
    }
}

/// Append a whole string's escapes to `output_bytes`, in the dialect `echo` and
/// `printf`'s `%b` share.
///
/// Returns whether a `\c` requested that all further output stop.
// [spec:dash:sem:printf.conv-escape-str-fn]
pub(crate) fn append_escape(input: &[u8], output_bytes: &mut BString) -> bool {
    let mut at = 0usize;
    let byte_at = |index: usize| -> u8 { input.get(index).copied().unwrap_or(0) };

    debug_assert!(output_bytes.is_empty());

    while at < input.len() {
        let byte = byte_at(at);
        at += 1;
        if byte != b'\\' {
            output_bytes.push(byte);
            continue;
        }
        let character = byte_at(at);
        if character == b'c' {
            return true;
        }

        /*
         * %b string octal constants are not like those in C.
         * They start with a \0, and are followed by 0, 1, 2,
         * or 3 octal digits.
         */
        if character == b'0' && is_octal_digit(byte_at(at + 1)) {
            at += 1;
        }

        let converted_escape = parse_escape(&input[at.min(input.len())..], false);
        at += converted_escape.consumed;
        output_bytes.extend_from_slice(converted_escape.bytes());
    }

    false
}

/// Quote arbitrary bytes so parsing the result produces the same bytes.
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

    #[test]
    fn escape_chunks_are_owned_bytes() {
        assert_eq!(parse_escape(b"n", false).bytes(), b"\n");
        assert_eq!(parse_escape(b"x41z", false).bytes(), b"A");
        assert_eq!(parse_escape(b"u20ac", true).bytes(), "€".as_bytes());
        assert_eq!(parse_escape(b"U0001f600", true).bytes(), "😀".as_bytes());
        assert_eq!(parse_escape(b"777", false).bytes(), &[0xff]);
        assert_eq!(parse_escape(b"q", false).bytes(), b"\\");
        assert_eq!(parse_escape(b"q", false).consumed, 0);
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
