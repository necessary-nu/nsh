//! How a variable is spelled back.
//!
//! `declare -p`, `readonly -p`, `export -p`, `${name@A}` and `${name@a}`
//! all answer the same question -- what would recreate this name -- and
//! they answer it out of this module, so a key needing
//! [`crate::escape::bash::subscript_quote`] cannot be quoted one way by
//! one printer and another way by another. That was the point of
//! extracting `declare -p`'s body in the first place; the four printers
//! that now read it are why it lives here rather than beside the special
//! variables it arrived with.
//!
//! Two letter tables sit next to each other on purpose.
//! [`attribute_letters`] is Bash's `var_attribute_string`, which
//! `${name@a}` renders, and [`declaration_flags`] is its
//! `attribute_string`, which a `declare -p` line carries; they order the
//! same attributes differently, and Bash keeps them apart too.

use bstr::{BStr, BString};

use super::value::{VariableKind, VariableValue};
use crate::context::Shell;

/// The attribute letters `${name@a}` renders, in the order Bash's
/// `var_attribute_string` writes them.
// [spec:nsh:req:compat.bash.builtins-special-variables]
pub(crate) fn attribute_letters(shell: &Shell, name: &BStr) -> BString {
    use super::value::BashAttribute;

    let mut letters = BString::default();
    let Some(attributes) = super::variable_attributes(shell, name) else {
        return letters;
    };
    let bash = super::value::bash_attributes(shell, name).unwrap_or_default();
    match super::value::variable_kind(shell, name) {
        Some(VariableKind::Indexed) => letters.push(b'a'),
        Some(VariableKind::Associative) => letters.push(b'A'),
        Some(VariableKind::Scalar) | None => {}
    }
    for (attribute, letter) in [
        (BashAttribute::Integer, b'i'),
        (BashAttribute::Nameref, b'n'),
    ] {
        if bash.contains(attribute) {
            letters.push(letter);
        }
    }
    if attributes.read_only {
        letters.push(b'r');
    }
    if bash.contains(BashAttribute::Trace) {
        letters.push(b't');
    }
    if attributes.exported {
        letters.push(b'x');
    }
    for (attribute, letter) in [
        (BashAttribute::Lowercase, b'l'),
        (BashAttribute::Uppercase, b'u'),
    ] {
        if bash.contains(attribute) {
            letters.push(letter);
        }
    }
    letters
}

/// The attribute letters a `declare -p` line carries for `name`, or
/// `None` for a name with no entry at all.
///
/// Bash writes these in a different order from the `${name@a}` letters
/// above -- `declare -Ar`, but `${m@a}` is `Ar` only by coincidence and
/// `-rx` is `rx` there and `rx` here while `-lu` is `lu` there and `ul`
/// here. `attribute_string` and `var_attribute_string` are two tables in
/// Bash and they are two here, next to each other so that the difference
/// is visible rather than surprising. A name that carries nothing takes
/// `-`, which is what makes `declare -- x="1"`.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn declaration_flags(shell: &Shell, name: &BStr) -> Option<BString> {
    use super::value::BashAttribute;

    // A name declared without a value still has a declaration to print;
    // only a name with no entry at all is missing.
    let attributes = super::variable_attributes(shell, name)?;
    let bash = super::value::bash_attributes(shell, name).unwrap_or_default();
    let mut flags = BString::default();
    match super::value::variable_kind(shell, name) {
        Some(VariableKind::Indexed) => flags.push(b'a'),
        Some(VariableKind::Associative) => flags.push(b'A'),
        Some(VariableKind::Scalar) | None => {}
    }
    for (attribute, letter) in [
        (BashAttribute::Integer, b'i'),
        (BashAttribute::Lowercase, b'l'),
        (BashAttribute::Nameref, b'n'),
        (BashAttribute::Trace, b't'),
        (BashAttribute::Uppercase, b'u'),
    ] {
        if bash.contains(attribute) {
            flags.push(letter);
        }
    }
    if attributes.read_only {
        flags.push(b'r');
    }
    if attributes.exported {
        flags.push(b'x');
    }
    if flags.is_empty() {
        flags.push(b'-');
    }
    Some(flags)
}

/// The `=value` a `declare -p` line writes for a stored value, and
/// nothing at all for a name that holds none.
///
/// This is the spelling that has to read back: the key goes through
/// [`crate::escape::bash::subscript_quote`] and the element beside it
/// through `declaration_quote`, so a blank or a metacharacter on either
/// side comes back as itself. `${name[@]@A}` asks the same question by a
/// different route and is answered from here, which is what keeps the
/// two spellings from drifting apart.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn declaration_value(shell: &Shell, value: &VariableValue) -> BString {
    let mut text = BString::default();
    if value.kind() == VariableKind::Scalar {
        if let Some(scalar) = value.scalar_ref() {
            text.push(b'=');
            text.extend_from_slice(&crate::escape::bash::declaration_quote(
                &shell.locale,
                scalar,
            ));
        }
        return text;
    }
    text.extend_from_slice(b"=(");
    let keys = super::arrays::keys(value);
    let elements = super::arrays::elements(value);
    for (position, (key, element)) in keys.iter().zip(elements.iter()).enumerate() {
        if position > 0 {
            text.push(b' ');
        }
        text.push(b'[');
        text.extend_from_slice(&crate::escape::bash::subscript_quote(
            &shell.locale,
            BStr::new(key.as_slice()),
        ));
        text.extend_from_slice(b"]=");
        text.extend_from_slice(&crate::escape::bash::declaration_quote(
            &shell.locale,
            BStr::new(element.as_slice()),
        ));
    }
    /* Bash pads the closing paren for associative arrays only, and only
     * when there is an element to pad away from: an empty one prints
     * `=()` like an indexed array. */
    if value.kind() == VariableKind::Associative && !keys.is_empty() {
        text.push(b' ');
    }
    text.push(b')');
    text
}

/// The whole `declare -p` line for `name`, or `None` for a name with no
/// entry at all.
///
/// `declare -p`, and in the Bash dialect `readonly -p` and `export -p`,
/// all print this; `${name[@]@A}` spells the same thing from the same
/// two halves. One renderer is what keeps a key that needs
/// [`crate::escape::bash::subscript_quote`] from being quoted one way by
/// one printer and another way by another.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn declaration_line(shell: &Shell, name: &BStr) -> Option<BString> {
    let mut line = BString::from("declare -");
    line.extend_from_slice(&declaration_flags(shell, name)?);
    line.push(b' ');
    line.extend_from_slice(name.as_ref());
    if let Some(value) = super::value::variable_value(shell, name) {
        line.extend_from_slice(&declaration_value(shell, value));
    }
    Some(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::lock;
    use crate::variables::{VariableAttributes, arrays};

    /// The `${name@a}` table is Bash's order and not the `declare -p`
    /// one, which is the whole reason the two live side by side.
    // [spec:nsh:req:compat.bash.builtins-special-variables/test]
    #[test]
    fn attribute_letters_follow_bash_order() {
        let _guard = lock();
        let mut shell = Shell::new(crate::streams::Streams::INHERIT);
        shell.options.set(crate::options::ShellOption::Bash, true);
        crate::variables::special::dialect_changed(&mut shell);
        let name = BStr::new(b"Tattr");
        arrays::ensure_kind(
            &mut shell,
            name,
            VariableKind::Indexed,
            VariableAttributes::NONE,
            arrays::ReadOnlyGuard::Enforce,
        )
        .unwrap();
        assert_eq!(attribute_letters(&shell, name), BString::from("a"));
        super::super::add_attributes(&mut shell, name, VariableAttributes::READ_ONLY);
        assert_eq!(attribute_letters(&shell, name), BString::from("ar"));
        assert_eq!(
            attribute_letters(&shell, BStr::new(b"Tmissing")),
            BString::default()
        );
    }
}
