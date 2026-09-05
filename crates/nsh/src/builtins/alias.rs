//! `alias`.
//!
//! Port of `aliascmd` from `src/alias.c`. The alias table itself stays in
//! `crate::alias`, where the parser and the line editor read it; this is
//! the command that prints and defines entries in it.
//!
//! # Three shapes for one line
//!
//! dash prints `'name=value'`, quoting the whole assignment. This shell
//! prints `name='value'` because
//! `[spec:posix:req:builtin.alias.stdout-format]` requires the name and
//! the equals sign unquoted, and that answer is the POSIX dialect's and
//! must not move. The reference prints a third form again,
//! `alias name='value'`, so the listing re-enters as commands rather than
//! as assignments -- and it takes `-p`, which is the same listing spelled
//! explicitly.
//!
//! The prefix is not simply "what Bash does". `bash --posix` drops it
//! from a bare `alias` and from a name query, and keeps it for `-p`. Bash
//! mode is measured against plain `bash`, as `exec`'s and `hash`'s
//! letters are, so the prefix is on every line this dialect prints.
//!
//! `-p` is not a filter. It prints the whole table and ignores its
//! operands entirely: `alias a=1; alias -p nosuch` prints `alias a='1'`
//! and succeeds, where `alias nosuch` alone reports and fails, and
//! `alias -p zz=1` defines nothing.
//!
//! Every claim above is measured against the pinned Bash 5.3.15 by
//! `crates/nsh-cli/tests/bash_alias_listing.rs`, which runs each case
//! through both shells and compares; nothing here is a recorded answer.

use crate::context::Shell;
use crate::error::Error;
use bstr::BStr;

use crate::alias::{format_alias, set_alias};
use crate::evaluation::Flow;
use crate::output::OutputDestination;

// [spec:dash:sem:alias.aliascmd-fn]
// [spec:posix:syn:builtin.alias.synopsis]
// [spec:posix:req:builtin.alias.create-or-display]
// [spec:posix:def:builtin.alias.definition]
// [spec:posix:req:builtin.alias.execution-environment]
// [spec:posix:req:builtin.alias.operands]
// [spec:posix:req:builtin.alias.env-locale]
// [spec:posix:sem:builtin.alias.env-nlspath]
// [spec:posix:req:builtin.alias.stderr]
// [spec:posix:req:builtin.alias.interfaces]
// [spec:posix:req:builtin.alias.exit-status]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut failed = false;
    let bash = shell.options.dialect() == crate::options::Dialect::Bash;
    let mut list_all = false;
    let operands: &[&BStr] = if bash {
        let mut option_scan = crate::options::Options::new(args);
        while option_scan.next(&mut shell.diagnostics(), b"p")?.is_some() {
            list_all = true;
        }
        option_scan.operands()
    } else {
        /* dash has no option scan at all, so `-p` is a name it does not
         * hold and `--` is another. Both must go on being reported. */
        args.get(1..).unwrap_or_default()
    };

    if list_all || operands.is_empty() {
        /* Rendered inside the walk, written after it: the walk holds
         * `sh.aliases` borrowed and the write wants `sh.io`. */
        let lines: Vec<Vec<u8>> = shell
            .aliases
            .entries()
            .map(|(name, value)| {
                listing_line(
                    bash,
                    BStr::new(name.as_slice()),
                    BStr::new(value.as_slice()),
                )
            })
            .collect();
        for line in lines {
            shell.write_output(OutputDestination::Stdout, &line)?;
        }
        return Ok(Flow::Done((0).into()));
    }
    for word in operands {
        /* n + 1: funny ksh stuff (from 44lite) */
        let equals = (!word.is_empty())
            .then(|| {
                word[1..]
                    .iter()
                    .position(|&byte| byte == b'=')
                    .map(|at| at + 1)
            })
            .flatten();
        match equals {
            None => {
                if let Some(value) = shell.aliases.lookup(word, false) {
                    let line = listing_line(bash, word, BStr::new(value.as_slice()));
                    shell.write_output(OutputDestination::Stdout, &line)?;
                } else {
                    let mut message = b"alias: ".to_vec();
                    message.extend_from_slice(word);
                    message.extend_from_slice(b" not found\n");
                    shell.write_output(OutputDestination::Stderr, &message)?;
                    failed = true;
                }
            }
            Some(equals) => {
                set_alias(
                    shell,
                    BStr::new(&word[..equals]),
                    BStr::new(&word[equals + 1..]),
                )?;
            }
        }
    }

    Ok(Flow::Done(i32::from(failed).into()))
}

/// One printed entry, in whichever dialect's shape.
///
/// The Bash dialect's `alias ` is a prefix on the POSIX line rather than
/// a format of its own, so the quoting the POSIX rule fixes is the same
/// quoting in both and only one of the two can drift.
fn listing_line(bash: bool, name: &BStr, value: &BStr) -> Vec<u8> {
    let line = format_alias(name, value);
    if !bash {
        return line;
    }
    let mut prefixed = b"alias ".to_vec();
    prefixed.extend_from_slice(&line);
    prefixed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Defining prints nothing and lands in the table the parser reads,
    /// which is the whole of what `alias name=value` is for.
    #[test]
    fn a_definition_reaches_the_table() {
        let _guard = crate::test_support::lock();
        let mut owned = Shell::new(crate::streams::Streams::INHERIT);
        let shell = &mut owned;
        assert_eq!(
            run(shell, &[BStr::new("alias"), BStr::new("ll=ls -l")]).unwrap(),
            Flow::Done((0).into())
        );
        assert!(shell.aliases.lookup(BStr::new(b"ll"), false).is_some());
    }

    /// A name that is not defined is a diagnostic and a failing status,
    /// and it does not stop the words after it being defined.
    #[test]
    fn an_unknown_name_fails_without_stopping() {
        let _guard = crate::test_support::lock();
        let mut owned = Shell::new(crate::streams::Streams::INHERIT);
        let shell = &mut owned;
        assert_eq!(
            run(
                shell,
                &[
                    BStr::new("alias"),
                    BStr::new("nosuchalias"),
                    BStr::new("after=1"),
                ]
            )
            .unwrap(),
            Flow::Done((1).into())
        );
        assert!(shell.aliases.lookup(BStr::new(b"after"), false).is_some());
    }
}
