//! Owned shell alias table.
//!
//! Rules: `docs/spec/port/src/alias.md`.

use bstr::{BStr, BString};
use std::collections::BTreeMap;

use crate::context::Shell;
use crate::error::Error;

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

    fn set(&mut self, name: &BStr, value: &BStr) {
        match self.map.get_mut(name) {
            Some(alias) => {
                alias.value = value.to_owned();
                alias.dead = false;
            }
            None => {
                self.map.insert(
                    name.to_owned(),
                    Alias {
                        value: value.to_owned(),
                        in_use: false,
                        dead: false,
                    },
                );
            }
        }
    }

    /// Return an owned alias expansion. `check_in_use` implements the
    /// parser's recursive-alias guard.
    // [spec:dash:def:alias.lookupalias-pub-fn]
    // [spec:dash:sem:alias.lookupalias-pub-fn]
    pub(crate) fn lookup(&self, name: &BStr, check_in_use: bool) -> Option<BString> {
        self.map
            .get(name)
            .and_then(|alias| (!check_in_use || !alias.in_use).then(|| alias.value.clone()))
    }

    /// Mark an alias expansion active until the corresponding input string is
    /// released.
    pub(crate) fn begin_expansion(&mut self, name: &BStr) {
        if let Some(alias) = self.map.get_mut(name) {
            alias.in_use = true;
        }
    }

    /// Release an alias expansion and complete a deferred `unalias`.
    pub(crate) fn finish_expansion(&mut self, name: &BStr) {
        let remove = self.map.get_mut(name).is_some_and(|alias| {
            alias.in_use = false;
            alias.dead
        });
        if remove {
            self.map.remove(name);
        }
    }

    fn remove(&mut self, name: &BStr) -> bool {
        let Some(alias) = self.map.get_mut(name) else {
            return false;
        };
        if alias.in_use {
            alias.dead = true;
        } else {
            self.map.remove(name);
        }
        true
    }

    fn clear(&mut self) {
        self.map.retain(|_, alias| {
            if alias.in_use {
                alias.dead = true;
                true
            } else {
                false
            }
        });
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

// [spec:nsh:req:idiom.lexer-tokens]
fn valid_name(name: &BStr) -> bool {
    !name.is_empty()
        && name.iter().all(|&byte| {
            crate::syntax::SyntaxContext::Base.classify(crate::syntax::InputUnit::Byte(byte))
                == crate::syntax::SyntaxClass::Word
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
        return Err(sh.diagnostics().sh_error_value(&message));
    }

    sh.interrupt_deferral
        .run_with(&mut sh.aliases, |aliases| aliases.set(name, value));
    Ok(())
}

// [spec:dash:def:alias.unalias-fn]
// [spec:dash:sem:alias.unalias-fn]
pub(crate) fn unalias(
    interrupts: &mut crate::error::InterruptDeferral,
    aliases: &mut AliasTable,
    name: &BStr,
) -> bool {
    interrupts.run_with(aliases, |aliases| aliases.remove(name))
}

// [spec:dash:def:alias.rmaliases-fn]
// [spec:dash:sem:alias.rmaliases-fn]
pub fn rmaliases(interrupts: &mut crate::error::InterruptDeferral, aliases: &mut AliasTable) {
    interrupts.run_with(aliases, AliasTable::clear);
}

// [spec:dash:def:alias.printalias-fn]
// [spec:dash:sem:alias.printalias-fn]
// [spec:posix:req:builtin.alias.stdout-format]
pub(crate) fn printalias(name: &BStr, value: &BStr) -> Vec<u8> {
    let quoted = crate::escape::shell_quote(value);
    let mut line = Vec::with_capacity(name.len() + quoted.len() + 2);
    line.extend_from_slice(name);
    line.push(b'=');
    line.extend_from_slice(&quoted);
    line.push(b'\n');
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:posix:req:builtin.alias.stdout-format/test]
    #[test]
    fn alias_display_quotes_only_value() {
        assert_eq!(
            printalias(BStr::new(b"sample"), BStr::new(b"a'b")),
            b"sample='a'\"'\"'b'\n".as_slice()
        );
    }

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
        shell.aliases.begin_expansion(name);
        assert!(shell.aliases.lookup(name, true).is_none());
        assert!(unalias(
            &mut shell.interrupt_deferral,
            &mut shell.aliases,
            name
        ));
        assert_eq!(
            shell
                .aliases
                .lookup(name, false)
                .as_ref()
                .map(|value| value.as_slice()),
            Some(b"old".as_slice())
        );

        setalias(&mut shell, name, BStr::new(b"new")).unwrap();
        shell.aliases.finish_expansion(name);
        assert_eq!(
            shell
                .aliases
                .lookup(name, false)
                .as_ref()
                .map(|value| value.as_slice()),
            Some(b"new".as_slice())
        );
        assert!(unalias(
            &mut shell.interrupt_deferral,
            &mut shell.aliases,
            name
        ));
        assert!(shell.aliases.lookup(name, false).is_none());
    }
}
