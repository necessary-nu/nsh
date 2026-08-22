//! Name references and function-scoped declarations for the Bash dialect.
//!
//! Two Bash-only rules about *which* entry an assignment reaches live
//! here. A `declare -n` variable holds the name of another variable, so
//! every read, write, and unset of it has to be redirected before the
//! ordinary variable table is touched. A declaration inside a function
//! body is local by default, so the declaration builtin has to save the
//! caller's entry into the frame the *function* owns rather than into
//! the transient frame the builtin's own invocation pushed.
//!
//! Neither rule is reachable from the POSIX dialect: `declare` is a
//! Bash-only built-in and nothing else sets [`BashAttribute::Nameref`],
//! so the redirection below is a single map probe on every other write.

use bstr::{BStr, BString};

use super::value::{BashAttribute, bash_attributes, set_bash_attribute};
use super::{
    Callback, LocalVariable, Variable, VariableAttributes, VariableState, arrays, run_callback,
    set_entry, valid_name,
};
use crate::context::Shell;
use crate::error::Error;

/// How far a chain of name references may be followed.
///
/// Bash reports a circular reference rather than looping; the depth cap
/// catches the same condition for a chain that never repeats a name.
const REFERENCE_DEPTH: usize = 64;

/// Where a name reference finally points.
// [spec:nsh:req:compat.bash.functions-scoping]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Target {
    /// A whole variable.
    Name(BString),
    /// One element, as in `declare -n ref='a[2]'`.
    Element { base: BString, subscript: BString },
}

impl Target {
    /// The spelling a parameter expansion or `unset` operand would use.
    pub(crate) fn text(&self) -> BString {
        match self {
            Self::Name(name) => name.clone(),
            Self::Element { base, subscript } => {
                let mut text = base.clone();
                text.push(b'[');
                text.extend_from_slice(subscript);
                text.push(b']');
                text
            }
        }
    }
}

/// Whether a fresh local declaration keeps the outer value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LocalValue {
    /// Bash's `local x`: the name starts out unset.
    Discard,
    /// The caller assigns immediately afterwards.
    Assigned,
}

/// Split `a[expr]` into its name and subscript bytes.
fn split_subscript(text: &BStr) -> Option<(&BStr, &BStr)> {
    let bytes: &[u8] = text.as_ref();
    if bytes.last() != Some(&b']') {
        return None;
    }
    let open = bytes.iter().position(|byte| *byte == b'[')?;
    if open == 0 {
        return None;
    }
    Some((
        BStr::new(&bytes[..open]),
        BStr::new(&bytes[open + 1..bytes.len() - 1]),
    ))
}

/// The name a `declare -n` entry currently points at, if it points anywhere.
///
/// A value that is not a name is not a reference at all: Bash leaves
/// `ref=1; typeset -n ref` reading and writing `ref` itself.
fn reference_target(shell: &Shell, name: &BStr) -> Option<Target> {
    if !bash_attributes(shell, name)?.contains(BashAttribute::Nameref) {
        return None;
    }
    let text = match &shell.variables.entries.get(name)?.state {
        VariableState::Unset => return None,
        VariableState::Set(value) => value.scalar_ref()?.to_owned(),
    };
    if text.is_empty() {
        return None;
    }
    if let Some((base, subscript)) = split_subscript(BStr::new(text.as_slice())) {
        return valid_name(&shell.locale, base).then(|| Target::Element {
            base: base.to_owned(),
            subscript: subscript.to_owned(),
        });
    }
    valid_name(&shell.locale, BStr::new(text.as_slice())).then_some(Target::Name(text))
}

/// Follow a chain of name references. `None` reports a circular chain.
// [spec:nsh:req:compat.bash.functions-scoping]
pub(crate) fn follow(shell: &Shell, name: &BStr) -> Option<Target> {
    let mut current = Target::Name(name.to_owned());
    let mut seen: Vec<BString> = Vec::new();
    loop {
        let Target::Name(here) = &current else {
            return Some(current);
        };
        let Some(next) = reference_target(shell, BStr::new(here.as_slice())) else {
            return Some(current);
        };
        if next == current {
            return Some(current);
        }
        if seen.iter().any(|visited| visited == here) || seen.len() >= REFERENCE_DEPTH {
            return None;
        }
        seen.push(here.clone());
        current = next;
    }
}

/// The name a read of `name` should actually look up.
///
/// `None` reports a circular chain, which Bash reads as unset.
// [spec:nsh:req:compat.bash.functions-scoping]
pub(crate) fn read_name(shell: &Shell, name: &BStr) -> Option<BString> {
    if let Some((base, subscript)) = split_subscript(name) {
        // `${ref[i]}` resolves the reference and keeps its own subscript.
        return match follow(shell, base)? {
            Target::Name(target) => Some(
                Target::Element {
                    base: target,
                    subscript: subscript.to_owned(),
                }
                .text(),
            ),
            // A reference that already selects an element cannot take a
            // second subscript; Bash rejects it, this reads the element.
            element => Some(element.text()),
        };
    }
    follow(shell, name).map(|target| target.text())
}

/// Whether `name` is spelled the way a name reference must be.
// [spec:nsh:req:compat.bash.functions-scoping]
pub(crate) fn is_valid_target(shell: &Shell, text: &BStr) -> bool {
    match split_subscript(text) {
        Some((base, _)) => valid_name(&shell.locale, base),
        None => valid_name(&shell.locale, text),
    }
}

/// Whether `name` carries `-n` over something that is not a name.
///
/// Bash keeps the attribute while the reference is merely empty, but
/// drops it once an assignment lands on a reference it cannot follow.
fn holds_a_broken_reference(shell: &Shell, name: &BStr) -> bool {
    let Some(entry) = shell.variables.entries.get(name) else {
        return false;
    };
    let VariableState::Set(value) = &entry.state else {
        return false;
    };
    value
        .scalar_ref()
        .is_some_and(|text| !text.is_empty() && !is_valid_target(shell, text))
}

/// The variable an array assignment to `name` should reach.
///
/// `None` refuses the assignment: Bash reports a reference that selects
/// an element or closes a cycle and leaves the array untouched.
// [spec:nsh:req:compat.bash.functions-scoping]
pub(crate) fn element_base(shell: &mut Shell, name: &BStr) -> Option<BString> {
    match follow(shell, name)? {
        Target::Name(target) if target == name => {
            // A compound value cannot be a reference, so the attribute
            // goes rather than the assignment.
            clear_reference(shell, name);
            Some(target)
        }
        Target::Name(target) => Some(target),
        // `ref[0]=v` where `ref` is `array[0]` names no identifier.
        Target::Element { .. } => None,
    }
}

/// Redirect one scalar assignment through a name reference.
///
/// Reports whether the write has been performed here; `false` leaves the
/// ordinary path to store the value under `name` itself.
// [spec:nsh:req:compat.bash.functions-scoping]
pub(crate) fn assign_through(
    shell: &mut Shell,
    name: &BStr,
    value: &BStr,
    attributes: VariableAttributes,
) -> Result<bool, Error> {
    if !bash_attributes(shell, name)
        .is_some_and(|declared| declared.contains(BashAttribute::Nameref))
    {
        return Ok(false);
    }
    let Some(target) = follow(shell, name) else {
        // A cycle has nothing to write to. Bash reports it and carries
        // on rather than abandoning the rest of the script.
        return Ok(true);
    };
    match target {
        Target::Name(target) if target == name => {
            if holds_a_broken_reference(shell, name) {
                clear_reference(shell, name);
            }
            Ok(false)
        }
        Target::Name(target) => {
            let target = BStr::new(target.as_slice());
            let converted = declared_value(shell, target, value)?;
            let stored = converted.as_ref().map_or(value, |text| BStr::new(text));
            crate::error::with_interrupts_deferred(shell, |shell| {
                set_entry(
                    shell,
                    target,
                    Some(stored),
                    attributes,
                    super::CallbackPolicy::Run,
                    arrays::ReadOnlyGuard::Enforce,
                )
            })?;
            Ok(true)
        }
        Target::Element { base, subscript } => {
            let base = BStr::new(base.as_slice());
            let selector = arrays::resolve_selector(shell, base, BStr::new(subscript.as_slice()))?;
            arrays::assign_element(
                shell,
                base,
                &selector,
                value,
                false,
                arrays::ReadOnlyGuard::Enforce,
            )?;
            Ok(true)
        }
    }
}

/// Reshape a value the way `name`'s declaration attributes require.
///
/// `None` means the bytes are already what should be stored, which is
/// the answer for every variable that carries no declaration attribute.
// [spec:nsh:req:compat.bash.functions-scoping]
pub(crate) fn declared_value(
    shell: &mut Shell,
    name: &BStr,
    value: &BStr,
) -> Result<Option<BString>, Error> {
    let Some(declared) = bash_attributes(shell, name) else {
        return Ok(None);
    };
    if declared.contains(BashAttribute::Integer) {
        let number = crate::arithmetic::evaluate(shell, value)?;
        return Ok(Some(BString::from(number.to_string())));
    }
    // Case conversion follows the C locale, as it does in Bash for every
    // byte outside the portable character set.
    if declared.contains(BashAttribute::Uppercase) {
        return Ok(Some(BString::from(value.to_ascii_uppercase())));
    }
    if declared.contains(BashAttribute::Lowercase) {
        return Ok(Some(BString::from(value.to_ascii_lowercase())));
    }
    Ok(None)
}

/// Record a declared name so its attributes have an entry to live on.
// [spec:nsh:req:compat.bash.functions-scoping]
pub(crate) fn ensure_entry(shell: &mut Shell, name: &BStr) {
    if !valid_name(&shell.locale, name) || shell.variables.entries.contains_key(name) {
        return;
    }
    shell.variables.entries.insert(
        name.to_owned(),
        Variable::unset(VariableAttributes::NONE, Callback::None),
    );
}

/// Whether a function body is currently running.
// [spec:nsh:req:compat.bash.functions-scoping]
pub(crate) fn in_function_scope(shell: &Shell) -> bool {
    !shell.variables.function_frames.is_empty()
}

/// Run a function body with the local frame that call owns recorded.
///
/// `evalcommand` pushed that frame before it resolved the command, so it
/// is already on the stack; what a declaration inside the body needs is
/// to know *which* frame it is.
// [spec:nsh:req:compat.bash.functions-scoping]
pub(crate) fn with_function_scope<T>(shell: &mut Shell, body: impl FnOnce(&mut Shell) -> T) -> T {
    let frame = shell.variables.locals.len().saturating_sub(1);
    shell.variables.function_frames.push(frame);
    let outcome = body(shell);
    shell.variables.function_frames.pop();
    outcome
}

/// Make `name` local to the running function body.
///
/// The saved entry belongs in the frame the *function call* pushed. A
/// declaration built-in runs with its own transient frame on top, and
/// saving there would restore the caller's value the moment the built-in
/// returned.
// [spec:nsh:req:compat.bash.functions-scoping]
pub(crate) fn make_local(shell: &mut Shell, name: &BStr, value: LocalValue) {
    let Some(frame) = shell.variables.function_frames.last().copied() else {
        return;
    };
    if frame >= shell.variables.locals.len() {
        return;
    }
    // A second declaration of a name this frame already owns keeps the
    // value the first one gave it, as Bash's repeated `local` does.
    let already_local = shell.variables.locals[frame]
        .entries
        .iter()
        .any(|local| match local {
            LocalVariable::Options(_) => false,
            LocalVariable::Created(held) | LocalVariable::Saved { name: held, .. } => held == name,
        });
    if already_local {
        return;
    }
    crate::error::with_interrupts_deferred(shell, |shell| {
        let Some(previous) = shell.variables.entries.get(name).cloned() else {
            shell.variables.locals[frame]
                .entries
                .push(LocalVariable::Created(name.to_owned()));
            shell.variables.entries.insert(
                name.to_owned(),
                Variable::unset(VariableAttributes::NONE, Callback::None),
            );
            return;
        };
        let callback = previous.callback;
        shell.variables.locals[frame]
            .entries
            .push(LocalVariable::Saved {
                name: name.to_owned(),
                previous,
            });
        // Bash's `local x` starts the name unset rather than inheriting
        // the caller's value or its declaration attributes.
        shell.variables.entries.insert(
            name.to_owned(),
            Variable::unset(VariableAttributes::NONE, callback),
        );
        if value == LocalValue::Discard {
            run_callback(shell, callback, None);
        }
    });
}

/// `declare -n ref=...` and `+n` write the reference itself, never the
/// variable it currently points at.
// [spec:nsh:req:compat.bash.functions-scoping]
pub(crate) fn clear_reference(shell: &mut Shell, name: &BStr) {
    set_bash_attribute(shell, name, BashAttribute::Nameref, false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::lock;
    use crate::variables::value::VariableKind;
    use crate::variables::{lookup_bytes, set_bytes};

    fn shell() -> Shell {
        Shell::new(crate::streams::Streams::INHERIT)
    }

    fn declare_reference(shell: &mut Shell, name: &BStr, text: &BStr) {
        set_bytes(shell, name, Some(text), VariableAttributes::NONE).unwrap();
        set_bash_attribute(shell, name, BashAttribute::Nameref, true);
    }

    /// A chain of references resolves to the variable at its end, and a
    /// value that is not a name is not a reference at all.
    // [spec:nsh:req:compat.bash.functions-scoping/test]
    #[test]
    fn a_chain_resolves_to_its_final_name() {
        let _g = lock();
        let shell = &mut shell();
        set_bytes(
            shell,
            BStr::new("Tnr_x"),
            Some(BStr::new("foo")),
            VariableAttributes::NONE,
        )
        .unwrap();
        declare_reference(shell, BStr::new("Tnr_a"), BStr::new("Tnr_x"));
        declare_reference(shell, BStr::new("Tnr_b"), BStr::new("Tnr_a"));

        assert_eq!(
            read_name(shell, BStr::new("Tnr_b")),
            Some(BString::from("Tnr_x"))
        );

        declare_reference(shell, BStr::new("Tnr_bad"), BStr::new("1"));
        assert_eq!(
            read_name(shell, BStr::new("Tnr_bad")),
            Some(BString::from("Tnr_bad"))
        );
    }

    /// Mutually recursive references are reported rather than looped on.
    // [spec:nsh:req:compat.bash.functions-scoping/test]
    #[test]
    fn a_circular_chain_has_no_target() {
        let _g = lock();
        let shell = &mut shell();
        declare_reference(shell, BStr::new("Tnr_one"), BStr::new("Tnr_two"));
        declare_reference(shell, BStr::new("Tnr_two"), BStr::new("Tnr_one"));

        assert_eq!(read_name(shell, BStr::new("Tnr_one")), None);
        // The write is refused rather than raised: Bash reports the cycle
        // and carries on with the rest of the script.
        assert!(
            assign_through(
                shell,
                BStr::new("Tnr_one"),
                BStr::new("v"),
                VariableAttributes::NONE
            )
            .unwrap()
        );
        assert_eq!(
            lookup_bytes(shell, BStr::new("Tnr_one")),
            Some(BString::from("Tnr_two"))
        );
    }

    /// Writing through a reference reaches the variable it names, and a
    /// reference with no usable target writes itself.
    // [spec:nsh:req:compat.bash.functions-scoping/test]
    #[test]
    fn a_write_follows_the_reference() {
        let _g = lock();
        let shell = &mut shell();
        set_bytes(
            shell,
            BStr::new("Tnr_t"),
            Some(BStr::new("old")),
            VariableAttributes::NONE,
        )
        .unwrap();
        declare_reference(shell, BStr::new("Tnr_r"), BStr::new("Tnr_t"));

        assert!(
            assign_through(
                shell,
                BStr::new("Tnr_r"),
                BStr::new("new"),
                VariableAttributes::NONE
            )
            .unwrap()
        );
        assert_eq!(
            lookup_bytes(shell, BStr::new("Tnr_t")),
            Some(BString::from("new"))
        );
        assert_eq!(
            lookup_bytes(shell, BStr::new("Tnr_r")),
            Some(BString::from("Tnr_t"))
        );

        // A reference over something that is not a name loses the
        // attribute rather than the assignment.
        declare_reference(shell, BStr::new("Tnr_self"), BStr::new("9"));
        assert!(
            !assign_through(
                shell,
                BStr::new("Tnr_self"),
                BStr::new("v"),
                VariableAttributes::NONE
            )
            .unwrap()
        );
        assert!(
            !bash_attributes(shell, BStr::new("Tnr_self"))
                .expect("the name exists")
                .contains(BashAttribute::Nameref)
        );
    }

    /// A reference may select one array element.
    // [spec:nsh:req:compat.bash.functions-scoping/test]
    #[test]
    fn a_reference_may_name_an_element() {
        let _g = lock();
        let shell = &mut shell();
        arrays::ensure_kind(
            shell,
            BStr::new("Tnr_arr"),
            VariableKind::Indexed,
            VariableAttributes::NONE,
        )
        .unwrap();
        arrays::assign_element(
            shell,
            BStr::new("Tnr_arr"),
            &arrays::ArraySelector::Index(2),
            BStr::new("two"),
            false,
            arrays::ReadOnlyGuard::Enforce,
        )
        .unwrap();
        declare_reference(shell, BStr::new("Tnr_e"), BStr::new("Tnr_arr[2]"));

        assert_eq!(
            read_name(shell, BStr::new("Tnr_e")),
            Some(BString::from("Tnr_arr[2]"))
        );
        assert_eq!(element_base(shell, BStr::new("Tnr_e")), None);
    }

    /// The integer and case attributes reshape the stored bytes.
    // [spec:nsh:req:compat.bash.functions-scoping/test]
    #[test]
    fn declaration_attributes_reshape_a_value() {
        let _g = lock();
        let shell = &mut shell();
        let name = BStr::new("Tnr_i");
        set_bytes(shell, name, Some(BStr::new("0")), VariableAttributes::NONE).unwrap();
        set_bash_attribute(shell, name, BashAttribute::Integer, true);
        set_bytes(
            shell,
            name,
            Some(BStr::new("2 + 3")),
            VariableAttributes::NONE,
        )
        .unwrap();
        assert_eq!(lookup_bytes(shell, name), Some(BString::from("5")));

        let upper = BStr::new("Tnr_u");
        set_bytes(shell, upper, Some(BStr::new("x")), VariableAttributes::NONE).unwrap();
        set_bash_attribute(shell, upper, BashAttribute::Uppercase, true);
        set_bytes(
            shell,
            upper,
            Some(BStr::new("abc")),
            VariableAttributes::NONE,
        )
        .unwrap();
        assert_eq!(lookup_bytes(shell, upper), Some(BString::from("ABC")));
    }
}
