//! `alias`.
//!
//! Port of `aliascmd` from `src/alias.c`. The alias table itself stays in
//! `crate::alias`, where the parser and the line editor read it; this is
//! the command that prints and defines entries in it.

use crate::context::Shell;
use crate::error::Error;
use bstr::BStr;

use crate::alias::{format_alias, set_alias};
use crate::evaluation::Flow;
use crate::output::OutputDestination;

// [spec:dash:def:alias.aliascmd-fn]
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

    if args.len() == 1 {
        /* Rendered inside the walk, written after it: the walk holds
         * `sh.aliases` borrowed and the write wants `sh.io`. */
        let lines: Vec<Vec<u8>> = shell
            .aliases
            .entries()
            .map(|(name, value)| {
                format_alias(BStr::new(name.as_slice()), BStr::new(value.as_slice()))
            })
            .collect();
        for line in lines {
            shell.write_output(OutputDestination::Stdout, &line)?;
        }
        return Ok(Flow::Done((0).into()));
    }
    for word in &args[1..] {
        /* n + 1: funny ksh stuff (from 44lite) */
        let equals = (!word.is_empty())
            .then(|| {
                word[1..]
                    .iter()
                    .position(|&byte| byte == b'=')
                    .map(|at| at + 1)
            })
            .flatten();
        if word.is_empty() || equals.is_none() {
            if let Some(value) = shell.aliases.lookup(word, false) {
                let line = format_alias(word, BStr::new(value.as_slice()));
                shell.write_output(OutputDestination::Stdout, &line)?;
            } else {
                let mut message = b"alias: ".to_vec();
                message.extend_from_slice(word);
                message.extend_from_slice(b" not found\n");
                shell.write_output(OutputDestination::Stderr, &message)?;
                failed = true;
            }
        } else {
            let equals = equals.expect("the definition branch");
            set_alias(
                shell,
                BStr::new(&word[..equals]),
                BStr::new(&word[equals + 1..]),
            )?;
        }
    }

    Ok(Flow::Done(i32::from(failed).into()))
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
