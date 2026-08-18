//! Owned shell alias table.
//!
//! Rules: `docs/spec/port/src/alias.md`.

use bstr::{BStr, BString};
use core::ffi::c_int;
use std::collections::BTreeMap;

use crate::context::Shell;
use crate::error::{Error, INTOFF, INTON};

pub const ALIASINUSE: c_int = 1;
pub const ALIASDEAD: c_int = 2;

#[derive(Clone, Debug)]
struct Alias {
    value: BString,
    in_use: bool,
    dead: bool,
}

/// Every alias, ordered bytewise by name.
pub struct AliasTable {
    map: BTreeMap<BString, Alias>,
}

impl AliasTable {
    // [spec:posix:req:token.alias-not-inherited]
    pub(crate) const fn new() -> Self {
        Self {
            map: BTreeMap::new(),
        }
    }

    pub(crate) fn entries(&self) -> impl Iterator<Item = (&BString, &BString)> {
        self.map.iter().map(|(name, alias)| (name, &alias.value))
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

fn valid_name(name: &BStr) -> bool {
    !name.is_empty()
        && name.iter().all(|&byte| {
            crate::syntax::BASESYNTAX(byte as i8 as c_int) == crate::syntax::CWORD
        })
}

// [spec:dash:def:alias.setalias-fn]
// [spec:dash:sem:alias.setalias-fn]
// [spec:posix:req:token.alias-change-timing]
pub(crate) fn setalias(sh: &mut Shell, name: &BStr, value: &BStr) -> Result<(), Error> {
    if !valid_name(name) {
        let mut message = b"Invalid alias name: ".to_vec();
        message.extend_from_slice(name);
        message.push(b'=');
        message.extend_from_slice(value);
        return Err(sh.sh_error_value(&message));
    }

    INTOFF(sh);
    match sh.aliases.map.get_mut(name) {
        Some(alias) => {
            alias.value = value.to_owned();
            alias.dead = false;
        }
        None => {
            sh.aliases.map.insert(
                name.to_owned(),
                Alias {
                    value: value.to_owned(),
                    in_use: false,
                    dead: false,
                },
            );
        }
    }
    INTON(sh);
    Ok(())
}

/// Return an owned alias expansion. `check_in_use` implements the parser's
/// recursive-alias guard.
// [spec:dash:def:alias.lookupalias-pub-fn]
// [spec:dash:sem:alias.lookupalias-pub-fn]
pub(crate) fn lookup_alias(sh: &Shell, name: &BStr, check_in_use: bool) -> Option<BString> {
    sh.aliases.map.get(name).and_then(|alias| {
        (!check_in_use || !alias.in_use).then(|| alias.value.clone())
    })
}

/// Mark an alias expansion active until the corresponding input string is
/// released.
pub(crate) fn begin_expansion(sh: &mut Shell, name: &BStr) {
    if let Some(alias) = sh.aliases.map.get_mut(name) {
        alias.in_use = true;
    }
}

/// Release an alias expansion and complete a deferred `unalias`.
pub(crate) fn finish_expansion(sh: &mut Shell, name: &BStr) {
    let remove = sh.aliases.map.get_mut(name).is_some_and(|alias| {
        alias.in_use = false;
        alias.dead
    });
    if remove {
        sh.aliases.map.remove(name);
    }
}

// [spec:dash:def:alias.unalias-fn]
// [spec:dash:sem:alias.unalias-fn]
pub(crate) fn unalias(sh: &mut Shell, name: &BStr) -> c_int {
    INTOFF(sh);
    let Some(alias) = sh.aliases.map.get_mut(name) else {
        INTON(sh);
        return 1;
    };
    if alias.in_use {
        alias.dead = true;
    } else {
        sh.aliases.map.remove(name);
    }
    INTON(sh);
    0
}

// [spec:dash:def:alias.rmaliases-fn]
// [spec:dash:sem:alias.rmaliases-fn]
pub fn rmaliases(sh: &mut Shell) {
    INTOFF(sh);
    sh.aliases.map.retain(|_, alias| {
        if alias.in_use {
            alias.dead = true;
            true
        } else {
            false
        }
    });
    INTON(sh);
}

// [spec:dash:def:alias.printalias-fn]
// [spec:dash:sem:alias.printalias-fn]
pub(crate) fn printalias(name: &BStr, value: &BStr) -> Vec<u8> {
    let mut definition = Vec::with_capacity(name.len() + value.len() + 1);
    definition.extend_from_slice(name);
    definition.push(b'=');
    definition.extend_from_slice(value);
    let mut line = crate::mystring::single_quote(BStr::new(&definition)).to_vec();
    line.push(b'\n');
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_invalid_name_returns_its_complaint() {
        let _guard = crate::testutil::lock();
        let mut shell = Shell::new(crate::streams::Streams::INHERIT);
        let error = setalias(&mut shell, BStr::new(b"a b"), BStr::new(b"value"))
            .expect_err("a space is not a word character");
        assert_eq!(error.message(), BStr::new(b"Invalid alias name: a b=value"));
        assert!(shell.aliases.is_empty());
    }

    // [spec:dash:sem:alias.setalias-fn/test]
    // [spec:dash:sem:alias.lookupalias-pub-fn/test]
    // [spec:dash:sem:alias.unalias-fn/test]
    // [spec:dash:sem:alias.rmaliases-fn/test]
    #[test]
    fn an_active_alias_defers_removal_and_can_be_revived() {
        let _guard = crate::testutil::lock();
        let mut shell = Shell::new(crate::streams::Streams::INHERIT);
        let name = BStr::new(b"held");
        setalias(&mut shell, name, BStr::new(b"old")).unwrap();
        begin_expansion(&mut shell, name);
        assert!(lookup_alias(&shell, name, true).is_none());
        assert_eq!(unalias(&mut shell, name), 0);
        assert_eq!(
            lookup_alias(&shell, name, false)
                .as_ref()
                .map(|value| value.as_slice()),
            Some(b"old".as_slice())
        );

        setalias(&mut shell, name, BStr::new(b"new")).unwrap();
        finish_expansion(&mut shell, name);
        assert_eq!(
            lookup_alias(&shell, name, false)
                .as_ref()
                .map(|value| value.as_slice()),
            Some(b"new".as_slice())
        );
        assert_eq!(unalias(&mut shell, name), 0);
        assert!(lookup_alias(&shell, name, false).is_none());
    }
}
