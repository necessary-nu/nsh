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
//! THERE IS ONE LETTER TABLE, not two. This module was written believing
//! that `declare -p` ordered its letters differently from `${name@a}`,
//! and it does not: measured over all 79 attribute combinations on the
//! pinned 5.3.15, `declare -p` prints exactly the letters `${name@a}`
//! prints, `declare -rl` and `declare -tl` and `declare -xu` included.
//! Bash's `print_var_attributes` calls `var_attribute_string`, the same
//! function `@a` reaches, so [`attribute_letters`] is the table and
//! [`declaration_flags`] is that table with the `-` a bare
//! `declare -- x` needs.

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
/// The letters are [`attribute_letters`]'s, because Bash's are: this
/// once held a second table in `attribute_string` order and printed
/// `declare -lr` where the reference prints `declare -rl`. A name that
/// carries nothing takes `-`, which is what makes `declare -- x="1"`
/// and is the whole of the difference from the transform's letters.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn declaration_flags(shell: &Shell, name: &BStr) -> Option<BString> {
    /* A name declared without a value still has a declaration to print;
     * what is missing is a name with no entry at all, and a slot this
     * shell reserved for a callback, which Bash reports as `not found`
     * because it has no variable there either. */
    super::variable_attributes(shell, name)?;
    if super::value::reserved_slot(shell, name) {
        return None;
    }
    let mut flags = attribute_letters(shell, name);
    if flags.is_empty() {
        flags.push(b'-');
    }
    Some(flags)
}

/// The letters `${name@A}` writes its `declare` prefix with, or `None`
/// for a name that is spelled bare.
///
/// Bash prefixes the assignment with `declare ` exactly when the name
/// carries an attribute, and `local` is one: a global `x=1` spells
/// `x='1'` and a `local x=1` spells `declare x='1'`, with no letters
/// between them either way. So an empty `Some` is a real answer here and
/// is not the same as `None`, which is why this is not
/// [`declaration_flags`] with the `-` taken off.
// [spec:nsh:req:compat.bash.functions-scoping]
pub(crate) fn transform_flags(shell: &Shell, name: &BStr) -> Option<BString> {
    let letters = attribute_letters(shell, name);
    if letters.is_empty() && !is_local(shell, name) {
        return None;
    }
    Some(letters)
}

/// Whether `name` belongs to the function body now running, which is
/// Bash's `att_local`.
///
/// The *current* body decides, not any body: a name a caller made local
/// is an ordinary variable to the function it calls, measured as
/// `${x@A}` spelling `declare x='1'` in the declaring body and `x='1'`
/// in a callee. `local` is not an attribute of the entry here -- it is a
/// record in the frame that will restore the caller's -- so this reads
/// the frames rather than the variable. Everything from the frame the
/// call pushed upwards belongs to the same call, a declaration
/// built-in's transient frame included.
// [spec:nsh:req:compat.bash.functions-scoping]
fn is_local(shell: &Shell, name: &BStr) -> bool {
    use super::LocalVariable;

    let Some(frame) = shell.variables.function_frames.last().copied() else {
        return false;
    };
    shell
        .variables
        .locals
        .get(frame..)
        .into_iter()
        .flatten()
        .flat_map(|scope| scope.entries.iter())
        .any(|local| match local {
            LocalVariable::Options(_) => false,
            LocalVariable::Created(held) | LocalVariable::Saved { name: held, .. } => held == name,
        })
}

/// Whether `${name@A}` has anything to say about `name`.
///
/// A name with no entry has nothing, and so has an entry that holds no
/// value and carries no attribute -- a bare `declare y` at the global
/// level is empty in the reference, where `declare -i n` and a `local q`
/// both print. The value is what makes an ordinary `x=1` printable, and
/// the attributes are what make a name with no value printable.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn is_spellable(shell: &Shell, name: &BStr) -> bool {
    super::value::variable_value(shell, name).is_some() || transform_flags(shell, name).is_some()
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
