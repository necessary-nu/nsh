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
///
/// Sixteen bytes because three different things end up here and the widest
/// wins: the C library's `MB_LEN_MAX`, the ten bytes of a `\U0010FFFF`
/// written back to itself, and the six the original UTF-8 ever needed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EscapeChunk {
    bytes: [u8; 16],
    length: u8,
    pub consumed: usize,
}

impl EscapeChunk {
    const fn one(byte: u8, consumed: usize) -> Self {
        let mut bytes = [0u8; 16];
        bytes[0] = byte;
        Self {
            bytes,
            length: 1,
            consumed,
        }
    }

    /// The conversion that writes nothing, for an escape the shell accepts
    /// and has no output for.
    const fn none(consumed: usize) -> Self {
        Self {
            bytes: [0u8; 16],
            length: 0,
            consumed,
        }
    }

    /// A conversion of several bytes.
    ///
    /// Longer input is truncated rather than refused. Nothing this shell
    /// produces reaches the limit -- no charmap glibc ships encodes one
    /// character in more than six bytes -- and a charmap that did would be
    /// better served losing its tail than aborting the shell.
    fn many(written: &[u8], consumed: usize) -> Self {
        let mut bytes = [0u8; 16];
        let length = written.len().min(bytes.len());
        bytes[..length].copy_from_slice(&written[..length]);
        Self {
            bytes,
            length: length as u8,
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

/// UTF-8's original form, which is wider than the range Unicode later
/// settled on.
///
/// A shell whose values are byte strings and not text
/// ([dec:nsh:bytes-not-text]) writes what the escape names: `\U00110000` is
/// four bytes and `\U7FFFFFFF` is six, neither of which decodes to a
/// character, and a surrogate is the three bytes its number spells. The C
/// library refuses all three now, so the encoding is done here rather than
/// asked for -- which is also what Bash does, and is why a `\ud800` is the
/// same bytes under either shell.
///
/// The encoding is the plain continuation of the pattern: a leading byte
/// with `n` high bits set for `n` total bytes, then six payload bits each.
// [spec:nsh:req:compat.bash.expansion-globbing]
// [dec:nsh:bytes-not-text]
fn utf8_bytes(value: u32, consumed: usize) -> EscapeChunk {
    let length: usize = match value {
        0x80..=0x7ff => 2,
        0x800..=0xffff => 3,
        0x1_0000..=0x1f_ffff => 4,
        0x20_0000..=0x3ff_ffff => 5,
        _ => 6,
    };

    let mut bytes = [0u8; 16];
    for index in (1..length).rev() {
        bytes[index] = 0x80 | (value >> (6 * (length - 1 - index))) as u8 & 0x3f;
    }
    let lead_mask = !0u8 << (8 - length);
    bytes[0] = lead_mask | (value >> (6 * (length - 1))) as u8;

    EscapeChunk {
        bytes,
        length: length as u8,
        consumed,
    }
}

/// The bytes a `\u` or `\U` escape writes, which is the charmap's question
/// and not Unicode's.
///
/// The escape names a character; what stands for that character in the
/// output is whatever the locale's charmap says stands for it, and a
/// charmap with no spelling for it makes the escape unwritable. POSIX
/// leaves the output for an unwritable character unspecified, and Bash
/// spells it by writing the escape back canonicalised -- upper case, zero
/// padded, `\u` below U+10000 and `\U` from there up, chosen by the value
/// and not by how the script spelled it, so `\U000000cc` comes back as
/// `Ì`. Under `LC_ALL=C` the charmap is ASCII and nothing above
/// U+007F can be written, so that is what the whole range does. This shell
/// was encoding UTF-8 whatever the locale said, which made `C` two
/// character models at once: bytes everywhere the locale is consulted, and
/// UTF-8 here.
///
/// Above U+7FFFFFFF even the original UTF-8 stops, no charmap reaches
/// there, and there is no escape spelling wider than eight digits either.
/// Bash writes nothing at all, in every locale.
// [spec:nsh:req:compat.bash.expansion-globbing]
fn unicode_bytes(locale: &nsh_platform::Locale, value: u32, consumed: usize) -> EscapeChunk {
    if value < 0x80 {
        return EscapeChunk::one(value as u8, consumed);
    }
    if value > 0x7fff_ffff {
        return EscapeChunk::none(consumed);
    }
    match locale.character_encoding(value) {
        nsh_platform::CharacterEncoding::Utf8 => utf8_bytes(value, consumed),
        nsh_platform::CharacterEncoding::Bytes(encoded) => EscapeChunk::many(&encoded, consumed),
        nsh_platform::CharacterEncoding::Unrepresentable => {
            let written = if value < 0x1_0000 {
                format!("\\u{value:04X}")
            } else {
                format!("\\U{value:08X}")
            };
            EscapeChunk::many(written.as_bytes(), consumed)
        }
    }
}

// [spec:dash:sem:printf.conv-escape-fn]
// [spec:dash:sem:system.conv-escape-fn]
// [spec:nsh:req:idiom.lexer-tokens]
pub fn parse_escape(
    locale: &nsh_platform::Locale,
    input: &[u8],
    single_quoted: bool,
) -> EscapeChunk {
    let Some(&character) = input.first() else {
        return EscapeChunk::one(b'\\', 0);
    };

    match character {
        b'\\' => EscapeChunk::one(b'\\', 1),
        b'a' => EscapeChunk::one(0x07, 1),
        b'b' => EscapeChunk::one(0x08, 1),
        b'f' => EscapeChunk::one(0x0c, 1),
        /* `\E` is Bash's second spelling of `\e`, and the shell has to
         * accept it because the shell *emits* it: `${x@Q}` of an escape
         * renders `$'\E'`, so without this the shell could not read its
         * own quoted output back. Found by the `quoting` fuzz target,
         * whose whole property is that round-trip. */
        b'e' | b'E' => EscapeChunk::one(0x1b, 1),
        b'n' => EscapeChunk::one(b'\n', 1),
        b'r' => EscapeChunk::one(b'\r', 1),
        b't' => EscapeChunk::one(b'\t', 1),
        b'v' => EscapeChunk::one(0x0b, 1),
        b'\'' | b'"' if single_quoted => EscapeChunk::one(character, 1),
        /* A `\x` with no hexadecimal digit after it is not an escape at
         * all: Bash and dash both leave the two bytes exactly as they
         * were written. Reading it as the value zero put a NUL into the
         * output, which is the worse half -- a caller reading through a
         * command substitution loses it and warns. */
        // [spec:nsh:req:compat.bash.builtins-special-variables]
        b'x' => {
            let (value, digits) = hexadecimal_value(&input[1..], 2);
            if digits == 0 {
                return EscapeChunk::one(b'\\', 0);
            }
            EscapeChunk::one(value as u8, 1 + digits)
        }
        /* Same rule as `\x` above, and found by the same target four
         * minutes after that one was fixed: with no hexadecimal digit
         * after it this is not an escape, and both Bash and dash leave
         * the two bytes as written. */
        // [spec:nsh:req:compat.bash.builtins-special-variables]
        b'u' | b'U' => {
            let maximum_digits = if character == b'u' { 4 } else { 8 };
            let (value, digits) = hexadecimal_value(&input[1..], maximum_digits);
            if digits == 0 {
                return EscapeChunk::one(b'\\', 0);
            }
            unicode_bytes(locale, value, 1 + digits)
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
pub(crate) fn append_escape(
    locale: &nsh_platform::Locale,
    input: &[u8],
    output_bytes: &mut BString,
) -> bool {
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

        let converted_escape = parse_escape(locale, &input[at.min(input.len())..], false);
        at += converted_escape.consumed;
        output_bytes.extend_from_slice(converted_escape.bytes());
    }

    false
}

/// Quote arbitrary bytes so parsing the result produces the same bytes.
// [spec:dash:sem:mystring.single-quote-fn]
// [spec:nsh:req:idiom.no-mystring]
pub(crate) mod bash;

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

/// A locale whose charmap is UTF-8, and a failure where the host has none.
///
/// The three names are tried in the order a host is likely to answer to.
/// `en_US.UTF-8` earns its place at the end rather than being redundant
/// with `C.UTF-8`: setting `LOCPATH` stops glibc consulting the system
/// locale archive at all, so a host that keeps its UTF-8 locales only there
/// has none until the generated one under `LOCPATH` answers.
// [spec:nsh:req:oracle.cannot-measure-is-a-failure]
#[cfg(test)]
pub(crate) fn utf8_locale() -> nsh_platform::Locale {
    [b"C.UTF-8".as_slice(), b"C.utf8", b"en_US.UTF-8"]
        .into_iter()
        .find_map(|name| nsh_platform::Locale::new(name, &[]).ok())
        .filter(|locale| {
            matches!(
                locale.character_encoding(0xcc),
                nsh_platform::CharacterEncoding::Utf8
            )
        })
        .expect("no UTF-8 charmap: tried C.UTF-8, C.utf8 and en_US.UTF-8")
}

/// A locale whose charmap is one byte wide and is not ASCII.
///
/// This one is generated rather than installed, and the tests that use it
/// separate "asks the charmap" from "assumes UTF-8", so a host without it
/// measures nothing. Returning `None` for the caller to skip on was that
/// silence, spelled as a pass, and it spread by being cited as precedent.
/// Being unable to run is reported here instead.
// [spec:nsh:req:oracle.cannot-measure-is-a-failure]
#[cfg(test)]
pub(crate) fn latin1_locale() -> nsh_platform::Locale {
    nsh_platform::Locale::new(b"en_US.ISO-8859-1", &[]).unwrap_or_else(|error| {
        panic!(
            "en_US.ISO-8859-1 is required by this test and could not be opened: {error}\n\
             build it and name it to the run:\n\
             \x20   export LOCPATH=$(tests/build-locales.sh)"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c_locale() -> nsh_platform::Locale {
        nsh_platform::Locale::c().expect("the C locale exists")
    }

    #[test]
    fn escape_strings_preserve_nul_data() {
        let mut output = BString::new(Vec::new());
        assert!(!append_escape(&c_locale(), b"a\0b", &mut output));
        assert_eq!(output, BString::from(b"a\0b".as_slice()));
    }

    #[test]
    fn escape_chunks_are_owned_bytes() {
        let c = c_locale();
        assert_eq!(parse_escape(&c, b"n", false).bytes(), b"\n");
        assert_eq!(parse_escape(&c, b"x41z", false).bytes(), b"A");
        assert_eq!(parse_escape(&c, b"777", false).bytes(), &[0xff]);
        assert_eq!(parse_escape(&c, b"q", false).bytes(), b"\\");
        assert_eq!(parse_escape(&c, b"q", false).consumed, 0);
        let utf8 = utf8_locale();
        assert_eq!(
            parse_escape(&utf8, b"u20ac", true).bytes(),
            "\u{20ac}".as_bytes()
        );
        assert_eq!(
            parse_escape(&utf8, b"U0001f600", true).bytes(),
            "\u{1f600}".as_bytes()
        );
    }

    /// A `\u` names a character, and the locale decides which bytes it is.
    ///
    /// Under `C` the charmap is ASCII, nothing above U+007F can be written,
    /// and Bash writes the escape back canonicalised rather than inventing
    /// an encoding. nsh emitted UTF-8 whatever the locale said, which made
    /// `C` two character models at once -- bytes wherever the locale was
    /// consulted, and UTF-8 here. The spelling that comes back is chosen by
    /// the value, not by how the script wrote it, so a `\U` naming a small
    /// value returns as `\u`.
    // [spec:nsh:req:compat.bash.expansion-globbing/test]
    #[test]
    fn an_unwritable_escape_is_written_back() {
        let c = c_locale();
        let bytes = |input: &[u8]| parse_escape(&c, input, false).bytes().to_vec();
        assert_eq!(bytes(b"u00cc"), b"\\u00CC");
        assert_eq!(bytes(b"ucc"), b"\\u00CC");
        assert_eq!(bytes(b"U000000cc"), b"\\u00CC");
        assert_eq!(bytes(b"uffff"), b"\\uFFFF");
        assert_eq!(bytes(b"U0001f600"), b"\\U0001F600");
        assert_eq!(bytes(b"U7fffffff"), b"\\U7FFFFFFF");
        /* ASCII is representable in every charmap and is untouched. */
        assert_eq!(bytes(b"u0041"), b"A");
        assert_eq!(bytes(b"u007f"), &[0x7f]);
    }

    /// A charmap that can write the character is asked, not overruled.
    // [spec:nsh:req:compat.bash.expansion-globbing/test]
    #[test]
    fn a_representable_escape_uses_the_charmap() {
        let latin1 = latin1_locale();
        let bytes = |input: &[u8]| parse_escape(&latin1, input, false).bytes().to_vec();
        /* ISO-8859-1 spells U+00CC as one byte, where UTF-8 needs two. */
        assert_eq!(bytes(b"u00cc"), &[0xcc]);
        assert_eq!(bytes(b"u00ff"), &[0xff]);
        /* Above its charmap the same rule as `C` applies. */
        assert_eq!(bytes(b"u0100"), b"\\u0100");
        assert_eq!(bytes(b"u20ac"), b"\\u20AC");
    }

    /// The original UTF-8 stops at U+7FFFFFFF, and so does Bash.
    ///
    /// Above it the shell writes nothing at all, in every locale: no
    /// charmap reaches there and there is no escape spelling wider than
    /// eight digits either. nsh was continuing the encoding pattern past
    /// its end and emitting six bytes led by 0xFE or 0xFF, which are not
    /// part of any UTF-8 that ever existed.
    // [spec:nsh:req:compat.bash.expansion-globbing/test]
    #[test]
    fn nothing_is_written_past_the_encodable_range() {
        let c = c_locale();
        assert_eq!(parse_escape(&c, b"U80000000", false).bytes(), b"");
        assert_eq!(parse_escape(&c, b"Uffffffff", false).bytes(), b"");
        /* The escape is still consumed, so the digits are not output. */
        assert_eq!(parse_escape(&c, b"U80000000", false).consumed, 9);
        let utf8 = utf8_locale();
        assert_eq!(
            parse_escape(&utf8, b"U7fffffff", false).bytes(),
            &[0xfd, 0xbf, 0xbf, 0xbf, 0xbf, 0xbf]
        );
        assert_eq!(parse_escape(&utf8, b"U80000000", false).bytes(), b"");
        /* A surrogate has no encoding the C library will produce, and
         * Bash writes the three bytes its number spells. */
        assert_eq!(
            parse_escape(&utf8, b"ud800", false).bytes(),
            &[0xed, 0xa0, 0x80]
        );
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
    /// `\x` with no hexadecimal digit is not an escape.
    ///
    /// Bash and dash agree here and nsh did not: it read the escape as
    /// the value zero and consumed the `x`, which put a NUL into the
    /// output. A caller reading that through a command substitution
    /// loses the byte and is warned about it, so the NUL was the worse
    /// half of the divergence. One hex digit is still enough.
    // [spec:nsh:req:compat.bash.builtins-special-variables/test]
    #[test]
    fn an_incomplete_hex_escape_stays_as_written() {
        let c = c_locale();
        assert_eq!(parse_escape(&c, b"x", false).bytes(), b"\\");
        assert_eq!(parse_escape(&c, b"x", false).consumed, 0);
        assert_eq!(parse_escape(&c, b"xg", false).bytes(), b"\\");
        assert_eq!(parse_escape(&c, b"xg", false).consumed, 0);
        assert_eq!(parse_escape(&c, b"x4", false).bytes(), &[4]);
        assert_eq!(parse_escape(&c, b"x41", false).bytes(), b"A");
        /* `\u` and `\U` say the same thing, and the four-minute rerun
         * found them four minutes after `\x` was fixed. */
        assert_eq!(parse_escape(&c, b"u", false).bytes(), b"\\");
        assert_eq!(parse_escape(&c, b"u", false).consumed, 0);
        assert_eq!(parse_escape(&c, b"uZ", false).bytes(), b"\\");
        assert_eq!(parse_escape(&c, b"U", false).bytes(), b"\\");
        assert_eq!(parse_escape(&c, b"UZ", false).consumed, 0);
        assert_eq!(parse_escape(&c, b"u41", false).bytes(), b"A");
        assert_eq!(parse_escape(&c, b"u4", false).bytes(), &[4]);
    }
}
