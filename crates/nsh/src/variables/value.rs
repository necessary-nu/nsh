//! Structural Bash variable values.
//!
//! Scalar-facing APIs keep borrowing or cloning element zero, while Bash
//! consumers can work with sparse indexed and associative maps directly.

use std::collections::BTreeMap;

use bstr::{BStr, BString};

use super::{Variable, VariableState};
use crate::context::Shell;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VariableKind {
    Scalar,
    Indexed,
    Associative,
}

#[derive(Clone, Debug, Eq, PartialEq)]
// [spec:nsh:req:compat.bash.value-model]
pub(crate) enum VariableValue {
    Scalar(BString),
    Indexed(BTreeMap<u64, BString>),
    Associative(BTreeMap<BString, BString>),
}

impl VariableValue {
    pub(crate) fn empty(kind: VariableKind) -> Self {
        match kind {
            VariableKind::Scalar => Self::Scalar(BString::default()),
            VariableKind::Indexed => Self::Indexed(BTreeMap::new()),
            VariableKind::Associative => Self::Associative(BTreeMap::new()),
        }
    }

    pub(crate) fn kind(&self) -> VariableKind {
        match self {
            Self::Scalar(_) => VariableKind::Scalar,
            Self::Indexed(_) => VariableKind::Indexed,
            Self::Associative(_) => VariableKind::Associative,
        }
    }

    /// Bash's un-subscripted scalar view: element/index `0` for arrays.
    pub(crate) fn scalar_ref(&self) -> Option<&BStr> {
        match self {
            Self::Scalar(value) => Some(BStr::new(value.as_slice())),
            Self::Indexed(values) => values.get(&0).map(|value| BStr::new(value.as_slice())),
            Self::Associative(values) => values
                .get(BStr::new(b"0"))
                .map(|value| BStr::new(value.as_slice())),
        }
    }

    pub(crate) fn scalar_owned(&self) -> Option<BString> {
        self.scalar_ref().map(BStr::to_owned)
    }

    /// A plain `name=value` assignment preserves an established array kind
    /// and writes its zero element, as Bash does.
    pub(crate) fn assign_scalar(&mut self, value: &BStr) {
        match self {
            Self::Scalar(current) => *current = value.to_owned(),
            Self::Indexed(values) => {
                values.insert(0, value.to_owned());
            }
            Self::Associative(values) => {
                values.insert(BString::from("0"), value.to_owned());
            }
        }
    }

    pub(crate) fn indexed(&self, index: u64) -> Option<&BStr> {
        let Self::Indexed(values) = self else {
            return None;
        };
        values.get(&index).map(|value| BStr::new(value.as_slice()))
    }

    pub(crate) fn set_indexed(&mut self, index: u64, value: &BStr) -> bool {
        let Self::Indexed(values) = self else {
            return false;
        };
        values.insert(index, value.to_owned());
        true
    }

    pub(crate) fn unset_indexed(&mut self, index: u64) -> Option<BString> {
        let Self::Indexed(values) = self else {
            return None;
        };
        values.remove(&index)
    }

    pub(crate) fn indexed_keys(&self) -> Option<Vec<u64>> {
        let Self::Indexed(values) = self else {
            return None;
        };
        Some(values.keys().copied().collect())
    }

    pub(crate) fn associative(&self, key: &BStr) -> Option<&BStr> {
        let Self::Associative(values) = self else {
            return None;
        };
        values.get(key).map(|value| BStr::new(value.as_slice()))
    }

    pub(crate) fn set_associative(&mut self, key: &BStr, value: &BStr) -> bool {
        let Self::Associative(values) = self else {
            return false;
        };
        values.insert(key.to_owned(), value.to_owned());
        true
    }

    pub(crate) fn unset_associative(&mut self, key: &BStr) -> Option<BString> {
        let Self::Associative(values) = self else {
            return None;
        };
        values.remove(key)
    }

    pub(crate) fn associative_keys(&self) -> Option<Vec<BString>> {
        let Self::Associative(values) = self else {
            return None;
        };
        Some(values.keys().cloned().collect())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BashAttribute {
    Integer,
    Lowercase,
    Nameref,
    Trace,
    Uppercase,
}

impl BashAttribute {
    const fn mask(self) -> u8 {
        match self {
            Self::Integer => 1 << 0,
            Self::Lowercase => 1 << 1,
            Self::Nameref => 1 << 2,
            Self::Trace => 1 << 3,
            Self::Uppercase => 1 << 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct BashAttributes {
    bits: u8,
}

impl BashAttributes {
    pub(crate) const fn new() -> Self {
        Self { bits: 0 }
    }

    pub(crate) fn contains(self, attribute: BashAttribute) -> bool {
        self.bits & attribute.mask() != 0
    }

    pub(crate) fn set(&mut self, attribute: BashAttribute, enabled: bool) {
        if enabled {
            self.bits |= attribute.mask();
        } else {
            self.bits &= !attribute.mask();
        }
    }
}

pub(crate) fn variable_value<'a>(shell: &'a Shell, name: &BStr) -> Option<&'a VariableValue> {
    match &shell.variables.entries.get(name)?.state {
        // A declared array holds nothing until something is stored, so
        // there is no value to borrow and `${z[@]}` reads none.
        VariableState::Unset | VariableState::Declared(_) => None,
        VariableState::Set(value) => Some(value),
    }
}

pub(crate) fn variable_value_owned(shell: &mut Shell, name: &BStr) -> Option<VariableValue> {
    super::special::refresh(shell, name);
    variable_value(shell, name).cloned()
}

/// The kind `declare -p` spells, which a name has from the moment it is
/// declared rather than from the moment it is assigned.
pub(crate) fn variable_kind(shell: &Shell, name: &BStr) -> Option<VariableKind> {
    match &shell.variables.entries.get(name)?.state {
        VariableState::Unset => None,
        VariableState::Declared(kind) => Some(*kind),
        VariableState::Set(value) => Some(value.kind()),
    }
}

/// The names a `declare -p` listing prints: every entry but a reserved
/// slot.
///
/// A declaration is what the listing carries, and an entry is what a
/// declaration leaves behind. `declare -a z`, `declare -i n`,
/// `readonly x`, `declare y` and a `local q` are all lines in Bash with
/// nothing beside them, and Bash's own set shows the same rule on its
/// own names -- `declare -i BASHPID`, `declare -- BASH_ARGV0` and
/// `declare -a FUNCNAME` are in its `declare -p` with no value.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn declared_names(shell: &Shell) -> Vec<BString> {
    shell
        .variables
        .entries
        .iter()
        .filter(|(_, var)| !var.is_reserved_slot())
        .map(|(name, _)| name.clone())
        .collect()
}

/// Whether the entry for `name` is one this shell reserved rather than
/// one a script made.
pub(crate) fn reserved_slot(shell: &Shell, name: &BStr) -> bool {
    shell
        .variables
        .entries
        .get(name)
        .is_some_and(Variable::is_reserved_slot)
}

/// The names that hold a value, which `${!prefix@}` answers with.
///
/// Bash lists `hello=()` there, an assigned empty array being a value,
/// and leaves out the declared-and-never-assigned `declare -a z`.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn valued_names(shell: &Shell) -> Vec<BString> {
    shell
        .variables
        .entries
        .iter()
        .filter(|(_, var)| matches!(var.state, VariableState::Set(_)))
        .map(|(name, _)| name.clone())
        .collect()
}

pub(crate) fn bash_attributes(shell: &Shell, name: &BStr) -> Option<BashAttributes> {
    shell
        .variables
        .entries
        .get(name)
        .map(|var| var.bash_attributes)
}

/// Take the export attribute away, which only `declare +x` asks for.
// [spec:nsh:req:compat.bash.functions-scoping]
pub(crate) fn clear_exported(shell: &mut Shell, name: &BStr) -> bool {
    let Some(var) = shell.variables.entries.get_mut(name) else {
        return false;
    };
    var.attributes.exported = false;
    true
}

pub(crate) fn set_bash_attribute(
    shell: &mut Shell,
    name: &BStr,
    attribute: BashAttribute,
    enabled: bool,
) -> bool {
    let Some(var) = shell.variables.entries.get_mut(name) else {
        return false;
    };
    var.bash_attributes.set(attribute, enabled);
    true
}

impl Variable {
    /// Whether this entry is a slot the shell keeps for itself, exactly
    /// as it made it.
    ///
    /// `initialize_variables` enters `MAIL`, `MAILPATH`, `HISTSIZE`,
    /// `TERM` and the five locale names so that a later assignment has
    /// a callback to run, and enters them `FIXED` and holding nothing.
    /// Bash has no variable there at all -- `declare -p LANG` in a
    /// shell started without one reports `not found`, and none of the
    /// eight is in its `declare -p` -- so a listing carrying them would
    /// report this shell's bookkeeping as the script's declarations.
    /// The first attempt at this listing did exactly that, adding seven
    /// names the reference does not have.
    ///
    /// The three tests are one question asked of the three places a
    /// script can leave a mark: a value, a dash attribute, a `declare`
    /// letter. `readonly MAIL` and `declare -i MAILPATH` are
    /// declarations of names that happen to have had a slot, and they
    /// are listed. What is left over is a bare `declare MAIL`, which
    /// changes nothing about the entry and so cannot be told from the
    /// slot it found; `list-a-declaration-of-a-reserved-name` holds
    /// that, and it needs a mark of its own rather than a finer test
    /// here.
    // [spec:nsh:req:compat.bash.arrays-declarations]
    fn is_reserved_slot(&self) -> bool {
        matches!(self.state, VariableState::Unset)
            && self.attributes == super::VariableAttributes::FIXED
            && self.bash_attributes == BashAttributes::new()
    }

    pub(super) fn scalar(&self) -> Option<&BStr> {
        match &self.state {
            VariableState::Unset | VariableState::Declared(_) => None,
            VariableState::Set(value) => value.scalar_ref(),
        }
    }

    pub(super) fn scalar_owned(&self) -> Option<BString> {
        match &self.state {
            VariableState::Unset | VariableState::Declared(_) => None,
            VariableState::Set(value) => value.scalar_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::variables::{
        Callback, Variable, VariableAttributes, VariableState, lookup_bytes, set_bytes, unset_bytes,
    };

    // [spec:nsh:req:compat.bash.value-model/test]
    #[test]
    fn indexed_storage_is_sparse() {
        let mut value = VariableValue::empty(VariableKind::Indexed);
        assert!(value.set_indexed(2, BStr::new(b"two")));
        assert!(value.set_indexed(1_000_000, BStr::new(b"far")));

        assert_eq!(value.indexed_keys(), Some(vec![2, 1_000_000]));
        assert_eq!(value.indexed(1_000_000), Some(BStr::new(b"far")));
        assert_eq!(value.unset_indexed(2), Some(BString::from("two")));
    }

    // [spec:nsh:req:compat.bash.value-model/test]
    #[test]
    fn associative_storage_preserves_bytes() {
        let mut value = VariableValue::empty(VariableKind::Associative);
        let key = BStr::new(&[b'k', 0xff]);
        let element = BStr::new(&[b'v', 0xfe]);
        assert!(value.set_associative(key, element));

        assert_eq!(value.associative(key), Some(element));
        assert_eq!(value.associative_keys(), Some(vec![key.to_owned()]));
        assert_eq!(value.unset_associative(key), Some(element.to_owned()));
    }

    // [spec:nsh:req:compat.bash.value-model/test]
    #[test]
    fn scalar_assignment_retains_array_kind() {
        let mut indexed = VariableValue::empty(VariableKind::Indexed);
        indexed.assign_scalar(BStr::new(b"zero"));
        assert_eq!(indexed.kind(), VariableKind::Indexed);
        assert_eq!(indexed.scalar_ref(), Some(BStr::new(b"zero")));

        let mut associative = VariableValue::empty(VariableKind::Associative);
        associative.assign_scalar(BStr::new(b"zero"));
        assert_eq!(associative.kind(), VariableKind::Associative);
        assert_eq!(associative.scalar_ref(), Some(BStr::new(b"zero")));
    }

    // [spec:nsh:req:compat.bash.value-model/test]
    #[test]
    fn attributes_toggle_independently() {
        let mut attributes = BashAttributes::new();
        attributes.set(BashAttribute::Integer, true);
        attributes.set(BashAttribute::Nameref, true);
        attributes.set(BashAttribute::Integer, false);

        assert!(!attributes.contains(BashAttribute::Integer));
        assert!(attributes.contains(BashAttribute::Nameref));
        assert!(!attributes.contains(BashAttribute::Uppercase));
    }

    // [spec:nsh:req:compat.bash.value-model/test]
    #[test]
    fn scalar_api_preserves_array_structure() {
        let mut shell = Shell::new(crate::streams::Streams::INHERIT);
        let name = BStr::new(b"array");
        shell.variables.entries.insert(
            name.to_owned(),
            Variable {
                attributes: VariableAttributes::NONE,
                state: VariableState::Set(VariableValue::empty(VariableKind::Indexed)),
                bash_attributes: BashAttributes::new(),
                callback: Callback::None,
                dynamic_lineno: false,
            },
        );

        assert_eq!(variable_kind(&shell, name), Some(VariableKind::Indexed));
        assert_eq!(lookup_bytes(&mut shell, name), None);
        set_bytes(
            &mut shell,
            name,
            Some(BStr::new(b"zero")),
            VariableAttributes::NONE,
        )
        .unwrap();
        assert_eq!(variable_kind(&shell, name), Some(VariableKind::Indexed));
        assert_eq!(lookup_bytes(&mut shell, name), Some(BString::from("zero")));

        assert!(set_bash_attribute(
            &mut shell,
            name,
            BashAttribute::Integer,
            true,
        ));
        set_bytes(
            &mut shell,
            name,
            Some(BStr::new(b"next")),
            VariableAttributes::NONE,
        )
        .unwrap();
        assert!(
            bash_attributes(&shell, name)
                .expect("array attributes")
                .contains(BashAttribute::Integer)
        );
    }

    // [spec:nsh:req:compat.bash.value-model/test]
    // [spec:nsh:def:idiom.variable-expansion-state/test]
    #[test]
    fn unset_and_empty_remain_distinct() {
        let attributes = VariableAttributes {
            exported: true,
            read_only: true,
            ..VariableAttributes::NONE
        };
        assert!(attributes.exported);
        assert!(attributes.read_only);

        let mut shell = Shell::new(crate::streams::Streams::INHERIT);
        let scalar = BStr::new(b"scalar");
        set_bytes(
            &mut shell,
            scalar,
            Some(BStr::new(b"")),
            VariableAttributes::NONE,
        )
        .unwrap();
        assert_eq!(variable_kind(&shell, scalar), Some(VariableKind::Scalar));
        assert_eq!(lookup_bytes(&mut shell, scalar), Some(BString::default()));

        let array = BStr::new(b"array");
        shell.variables.entries.insert(
            array.to_owned(),
            Variable {
                attributes: VariableAttributes::NONE,
                state: VariableState::Set(VariableValue::empty(VariableKind::Associative)),
                bash_attributes: BashAttributes::new(),
                callback: Callback::None,
                dynamic_lineno: false,
            },
        );
        assert_eq!(
            variable_kind(&shell, array),
            Some(VariableKind::Associative)
        );
        assert_eq!(
            variable_value(&shell, array).and_then(VariableValue::associative_keys),
            Some(Vec::new())
        );

        unset_bytes(&mut shell, array).unwrap();
        assert_eq!(variable_kind(&shell, array), None);
        assert_eq!(lookup_bytes(&mut shell, array), None);
    }
}
