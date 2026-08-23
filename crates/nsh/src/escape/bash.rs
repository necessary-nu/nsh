//! Bash's three ways of quoting a value so the shell can read it back.
//!
//! POSIX has one: wrap the bytes in single quotes and break out for each
//! `'`. That is [`super::shell_quote`], and it is enough for anything a
//! POSIX shell prints. Bash reaches for a different spelling depending on
//! what the bytes contain, and the choice is observable -- `printf %q`,
//! `${x@Q}`, `set` and `declare -p` all print through it, and a test
//! suite compares the exact characters.
//!
//! The choice is made once, by [`needs_ansi_c`]: a value holding a
//! control character or a byte that is not part of a valid character in
//! this locale can only be written as `$'...'`, because every other form
//! would have to put the raw byte in the output. Everything else takes
//! the caller's ordinary spelling -- backslashes for `printf %q`, double
//! quotes for `declare -p`, single quotes for `set`.

use bstr::{BStr, BString};

/// The bytes a backslash-quoted rendering escapes one at a time.
///
/// Everything here would otherwise be read back as syntax: the field
/// separators, the three quoting characters, the operators, and the
/// characters that start an expansion or a pattern.
const NEEDS_BACKSLASH: &[u8] = b" \t\n!\"#$&'()*;<>?[\\]^`{|}~";

/// Whether `value` can only be written as an ANSI-C quoted string.
pub(crate) fn needs_ansi_c(locale: &nsh_platform::Locale, value: &BStr) -> bool {
    let mut cursor = 0;
    while cursor < value.len() {
        let byte = value[cursor];
        if byte < 0x20 || byte == 0x7f {
            return true;
        }
        if byte.is_ascii() {
            cursor += 1;
            continue;
        }
        match character_width(locale, &value[cursor..]) {
            Some(width) => cursor += width,
            None => return true,
        }
    }
    false
}

/// How many bytes the character starting at `bytes` occupies, or `None`
/// when those bytes do not begin one.
fn character_width(locale: &nsh_platform::Locale, bytes: &[u8]) -> Option<usize> {
    let mut decoder = locale.decoder();
    for (offset, byte) in bytes.iter().copied().enumerate() {
        match decoder.push(byte) {
            nsh_platform::LocaleDecode::Complete(_) => return Some(offset + 1),
            nsh_platform::LocaleDecode::Invalid => return None,
            nsh_platform::LocaleDecode::Incomplete => {}
        }
    }
    None
}

/// `$'...'`: the only rendering that can carry a control character or a
/// byte that is not part of a character.
///
/// Bash writes an unrepresentable byte in octal rather than hexadecimal,
/// and leaves a character it can decode exactly as it found it -- which
/// is why `$'\316μ'` has one escape and one literal even though both
/// halves are non-ASCII.
pub(crate) fn ansi_c_quote(locale: &nsh_platform::Locale, value: &BStr) -> BString {
    let mut quoted = BString::from("$'");
    let mut cursor = 0;
    while cursor < value.len() {
        let byte = value[cursor];
        if let Some(escape) = short_escape(byte) {
            quoted.push(b'\\');
            quoted.push(escape);
            cursor += 1;
            continue;
        }
        if (0x20..0x7f).contains(&byte) {
            quoted.push(byte);
            cursor += 1;
            continue;
        }
        if !byte.is_ascii()
            && let Some(width) = character_width(locale, &value[cursor..])
        {
            quoted.extend_from_slice(&value[cursor..cursor + width]);
            cursor += width;
            continue;
        }
        quoted.extend_from_slice(format!("\\{byte:03o}").as_bytes());
        cursor += 1;
    }
    quoted.push(b'\'');
    quoted
}

/// The two-character escapes Bash prefers over an octal one.
const fn short_escape(byte: u8) -> Option<u8> {
    match byte {
        0x07 => Some(b'a'),
        0x08 => Some(b'b'),
        0x09 => Some(b't'),
        0x0a => Some(b'n'),
        0x0b => Some(b'v'),
        0x0c => Some(b'f'),
        0x0d => Some(b'r'),
        0x1b => Some(b'E'),
        b'\\' => Some(b'\\'),
        b'\'' => Some(b'\''),
        _ => None,
    }
}

/// `printf %q` and `${x@Q}`: the shortest spelling the shell reads back
/// as the same bytes.
// [spec:nsh:req:compat.bash.builtins-special-variables]
pub(crate) fn requote(locale: &nsh_platform::Locale, value: &BStr) -> BString {
    if value.is_empty() {
        return BString::from("''");
    }
    if needs_ansi_c(locale, value) {
        return ansi_c_quote(locale, value);
    }
    let mut quoted = BString::default();
    for byte in value.as_ref() as &[u8] {
        if NEEDS_BACKSLASH.contains(byte) {
            quoted.push(b'\\');
        }
        quoted.push(*byte);
    }
    quoted
}

/// `declare -p`: double quotes, escaping only what would still expand
/// inside them.
// [spec:nsh:req:compat.bash.builtins-special-variables]
pub(crate) fn declaration_quote(locale: &nsh_platform::Locale, value: &BStr) -> BString {
    if needs_ansi_c(locale, value) {
        return ansi_c_quote(locale, value);
    }
    let mut quoted = BString::from("\"");
    for byte in value.as_ref() as &[u8] {
        if matches!(byte, b'"' | b'\\' | b'$' | b'`') {
            quoted.push(b'\\');
        }
        quoted.push(*byte);
    }
    quoted.push(b'"');
    quoted
}

/// `set` with no operands, and `${x@Q}`: single quotes, or `$'...'`
/// where single quotes could not carry the bytes.
///
/// The difference from [`requote`] is not cosmetic. Both spellings read
/// back as the same bytes, but Bash quotes here even when nothing needs
/// it -- `${x@Q}` on `x` is `'x'` where `printf %q` is a bare `x` -- and
/// a script that compares the text can tell.
// [spec:nsh:req:compat.bash.builtins-special-variables]
pub(crate) fn readable_quote(locale: &nsh_platform::Locale, value: &BStr) -> BString {
    if needs_ansi_c(locale, value) {
        return ansi_c_quote(locale, value);
    }
    super::shell_quote(value)
}

/// `set -x`: the spelling Bash traces one word with.
///
/// Bash quotes a traced word only when it would not read back as itself,
/// which is what keeps `+ echo hi` unquoted while `+ sh -c 'echo 2'`
/// shows where the argument's boundaries are. Its own order is tested
/// here too: a word with a shell metacharacter takes single quotes even
/// when it also holds a byte that would otherwise ask for `$'...'`.
// [spec:nsh:req:compat.bash.builtins-special-variables]
pub(crate) fn trace_quote(locale: &nsh_platform::Locale, value: &BStr) -> BString {
    if value.is_empty() {
        return BString::from("''");
    }
    if contains_shell_metacharacter(value) {
        return super::shell_quote(value);
    }
    if needs_ansi_c(locale, value) {
        return ansi_c_quote(locale, value);
    }
    value.to_owned()
}

/// Whether a word holds a byte that changes meaning when it is not
/// quoted, in the set Bash's `sh_contains_shell_metas` uses.
fn contains_shell_metacharacter(value: &BStr) -> bool {
    let bytes: &[u8] = value.as_ref();
    bytes.iter().enumerate().any(|(at, byte)| match byte {
        b' ' | b'\t' | b'\n' | b'*' | b'?' | b'[' | b']' | b'{' | b'}' | b'(' | b')' | b'$'
        | b'`' | b'\\' | b'"' | b'\'' | b'|' | b'&' | b';' | b'<' | b'>' | b'!' | b'^' => true,
        b'~' => at == 0 || matches!(bytes[at - 1], b':' | b'='),
        b'#' => at == 0,
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:nsh:req:compat.bash.builtins-special-variables/test]
    #[test]
    fn a_traced_word_is_quoted_only_when_needed() {
        let locale = locale();
        assert_eq!(
            trace_quote(&locale, BStr::new(b"echo")),
            BString::from("echo")
        );
        assert_eq!(
            trace_quote(&locale, BStr::new(b"echo 2")),
            BString::from("'echo 2'")
        );
        assert_eq!(trace_quote(&locale, BStr::new(b"")), BString::from("''"));
        assert_eq!(
            trace_quote(&locale, BStr::new(&[0xff])),
            BString::from("$'\\377'")
        );
    }

    fn locale() -> nsh_platform::Locale {
        nsh_platform::Locale::c().expect("the C locale exists")
    }

    // [spec:nsh:req:compat.bash.builtins-special-variables/test]
    #[test]
    fn a_control_character_forces_ansi_c_quoting() {
        let locale = locale();
        assert_eq!(
            requote(&locale, BStr::new(b"one\ntwo")),
            BString::from("$'one\\ntwo'")
        );
        assert_eq!(
            requote(&locale, BStr::new(&[0xff])),
            BString::from("$'\\377'")
        );
        assert_eq!(
            declaration_quote(&locale, BStr::new(b"one\ntwo")),
            BString::from("$'one\\ntwo'")
        );
    }

    /// A value with nothing to hide keeps the caller's ordinary spelling.
    // [spec:nsh:req:compat.bash.builtins-special-variables/test]
    #[test]
    fn plain_bytes_keep_the_short_spelling() {
        let locale = locale();
        assert_eq!(
            requote(&locale, BStr::new(b"one two")),
            BString::from("one\\ two")
        );
        assert_eq!(
            requote(&locale, BStr::new(b"'\"")),
            BString::from("\\'\\\"")
        );
        assert_eq!(requote(&locale, BStr::new(b"")), BString::from("''"));
        assert_eq!(
            declaration_quote(&locale, BStr::new(b"hello")),
            BString::from("\"hello\"")
        );
        assert_eq!(
            readable_quote(&locale, BStr::new(b"hello")),
            BString::from("'hello'")
        );
    }
}
