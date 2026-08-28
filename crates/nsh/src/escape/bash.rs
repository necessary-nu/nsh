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
/// `#` and `~` are deliberately absent: both are syntax only in
/// certain positions, and `is_syntax_here` decides those. `,` is
/// present because Bash escapes it wherever it appears.
const NEEDS_BACKSLASH: &[u8] = b" \t\n!\"$&'()*,;<>?[\\]^`{|}";

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
    let bytes: &[u8] = value.as_ref();
    let mut quoted = BString::default();
    for (at, byte) in bytes.iter().enumerate() {
        if NEEDS_BACKSLASH.contains(byte) || is_syntax_here(bytes, at) {
            quoted.push(b'\\');
        }
        quoted.push(*byte);
    }
    quoted
}

/// Whether a byte is syntax only because of where it stands.
///
/// Two of them are not escaped everywhere, and escaping them everywhere
/// is what `printf %q` was doing. A `#` begins a comment only as the
/// first byte of a word. A `~` begins a tilde expansion only at the
/// front, or straight after the `:` or `=` that separates the fields of
/// an assignment -- which is the rule
/// [`contains_shell_metacharacter`] already applies, stated once more
/// here because the two answer different questions about the same byte.
// [spec:nsh:req:compat.bash.builtins-special-variables]
fn is_syntax_here(bytes: &[u8], at: usize) -> bool {
    match bytes[at] {
        b'#' => at == 0,
        b'~' => at == 0 || matches!(bytes[at - 1], b':' | b'='),
        _ => false,
    }
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
    bash_quote(value)
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
        return bash_quote(value);
    }
    if needs_ansi_c(locale, value) {
        return ansi_c_quote(locale, value);
    }
    value.to_owned()
}

/// Single quotes spelled the way Bash spells them.
///
/// Both shells carry an apostrophe by leaving the quoted run and coming
/// back, and they choose different ways to carry it: dash writes it
/// inside double quotes and Bash writes it as a backslash escape. The
/// bytes they denote are identical, so this is a divergence in the text
/// and not in the meaning -- but `${x@Q}` and `set` exist to be read and
/// compared, and [`crate::escape::shell_quote`] keeps dash's spelling
/// for the POSIX-mode callers that must not move.
// [spec:nsh:req:compat.bash.builtins-special-variables]
fn bash_quote(value: &BStr) -> BString {
    let bytes: &[u8] = value.as_ref();
    if bytes.is_empty() {
        return BString::from("''");
    }
    /* Bash's own special case: a value that is one apostrophe is spelled
     * as a bare backslash escape rather than as empty runs around one. */
    if bytes == b"'" {
        return BString::from("\\'");
    }
    let mut quoted = BString::from("'");
    for byte in bytes {
        if *byte == b'\'' {
            quoted.extend_from_slice(b"'\\''");
            continue;
        }
        quoted.push(*byte);
    }
    quoted.push(b'\'');
    quoted
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
    /// `%q` escapes two bytes only where they are syntax.
    ///
    /// Derived byte by byte from the pinned Bash 5.3 build rather than
    /// inferred: a `~` starts a tilde expansion at the front of a word or
    /// straight after the `:` or `=` of an assignment and nowhere else, a
    /// `#` starts a comment only at the front, and a `,` is escaped
    /// everywhere -- which nsh was not doing at all.
    // [spec:nsh:req:compat.bash.builtins-special-variables/test]
    #[test]
    fn requote_escapes_a_byte_where_it_is_syntax() {
        let locale = locale();
        let q = |value: &[u8]| requote(&locale, BStr::new(value)).to_vec();
        assert_eq!(q(b"~"), b"\\~");
        assert_eq!(q(b"~a"), b"\\~a");
        assert_eq!(q(b":~"), b":\\~");
        assert_eq!(q(b"a=~"), b"a=\\~");
        assert_eq!(q(b"a=b=~"), b"a=b=\\~");
        assert_eq!(q(b"a=x:~/y"), b"a=x:\\~/y");
        /* The three the fuzzer found: a tilde with an ordinary byte in
         * front of it is not an expansion and Bash leaves it alone. */
        assert_eq!(q(b"a~b"), b"a~b");
        assert_eq!(q(b"a~"), b"a~");
        assert_eq!(q(b"P~2T"), b"P~2T");
        assert_eq!(q(b"~~"), b"\\~~");
        assert_eq!(q(b"#"), b"\\#");
        assert_eq!(q(b"a#b"), b"a#b");
        assert_eq!(q(b":#"), b":#");
        assert_eq!(q(b","), b"\\,");
        assert_eq!(q(b"a,b"), b"a\\,b");
    }

    /// `${x@Q}` and `set` spell an apostrophe the way Bash does.
    ///
    /// dash carries it inside double quotes and Bash carries it as a
    /// backslash escape. Both denote the same bytes, so this is the text
    /// and not the meaning -- but these strings exist to be pasted and
    /// diffed, and `crate::escape::shell_quote` still owes dash its own
    /// spelling, which is why this lives here and not there.
    // [spec:nsh:req:compat.bash.builtins-special-variables/test]
    #[test]
    fn bash_carries_an_apostrophe_as_a_backslash_escape() {
        let locale = locale();
        let q = |value: &[u8]| readable_quote(&locale, BStr::new(value)).to_vec();
        assert_eq!(q(b"a'b"), b"'a'\\''b'");
        assert_eq!(q(b"a'b'c"), b"'a'\\''b'\\''c'");
        assert_eq!(q(b"'a"), b"''\\''a'");
        assert_eq!(q(b"a'"), b"'a'\\'''");
        assert_eq!(q(b"''"), b"''\\'''\\'''");
        assert_eq!(q(b""), b"''");
        assert_eq!(q(b"plain"), b"'plain'");
        /* Bash's own special case, and the one a general rule misses. */
        assert_eq!(q(b"'"), b"\\'");
        /* dash's spelling is unmoved, because POSIX mode still uses it. */
        assert_eq!(
            crate::escape::shell_quote(BStr::new(b"a'b")),
            b"'a'\"'\"'b'".as_slice()
        );
    }
}
