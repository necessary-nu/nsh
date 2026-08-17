//! `alias`.
//!
//! Port of `aliascmd` from `src/alias.c`. The alias table itself stays in
//! `crate::alias`, where the parser and the line editor read it; this is
//! the command that prints and defines entries in it.

use crate::context::Shell;
use crate::error::Error;
use bstr::BStr;
use core::ffi::c_int;
use std::io::Write;

use crate::alias::{lookup_alias, printalias, setalias};
use crate::eval::Flow;

// [spec:dash:def:alias.aliascmd-fn]
// [spec:dash:sem:alias.aliascmd-fn]
pub fn aliascmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut ret: c_int = 0;

    if args.len() == 1 {
        /* Rendered inside the walk, written after it: the walk holds
         * `sh.aliases` borrowed and the write wants `sh.io`. */
        let lines: Vec<Vec<u8>> = sh
            .aliases
            .entries()
            .map(|(name, value)| {
                printalias(BStr::new(name.as_slice()), BStr::new(value.as_slice()))
            })
            .collect();
        for line in lines {
            let _ = sh.io.stdout().write_all(&line);
        }
        return Ok(Flow::Done(0));
    }
    for word in &args[1..] {
        /* n + 1: funny ksh stuff (from 44lite) */
        let equals = (!word.is_empty())
            .then(|| word[1..].iter().position(|&byte| byte == b'=').map(|at| at + 1))
            .flatten();
        if word.is_empty() || equals.is_none() {
            if let Some(value) = lookup_alias(sh, word, false) {
                let line = printalias(word, BStr::new(value.as_slice()));
                let _ = sh.io.stdout().write_all(&line);
            } else {
                let mut message = b"alias: ".to_vec();
                message.extend_from_slice(word);
                message.extend_from_slice(b" not found\n");
                let _ = sh.io.stderr().write_all(&message);
                ret = 1;
            }
        } else {
            let equals = equals.expect("the definition branch");
            setalias(sh, BStr::new(&word[..equals]), BStr::new(&word[equals + 1..]))?;
        }
    }

    Ok(Flow::Done(ret))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::alias::lookup_alias;

    /// Defining prints nothing and lands in the table the parser reads,
    /// which is the whole of what `alias name=value` is for.
    #[test]
    fn a_definition_reaches_the_table() {
        let _guard = crate::testutil::lock();
        let mut owned = Shell::new(crate::streams::Streams::INHERIT);
        let sh = &mut owned;
        assert_eq!(
            aliascmd(sh, &[BStr::new("alias"), BStr::new("ll=ls -l")]).unwrap(),
            Flow::Done(0)
        );
        assert!(lookup_alias(sh, BStr::new(b"ll"), false).is_some());
    }

    /// A name that is not defined is a diagnostic and a failing status,
    /// and it does not stop the words after it being defined.
    #[test]
    fn an_unknown_name_fails_without_stopping() {
        let _guard = crate::testutil::lock();
        let mut owned = Shell::new(crate::streams::Streams::INHERIT);
        let sh = &mut owned;
        assert_eq!(
            aliascmd(sh, &[
                BStr::new("alias"),
                BStr::new("nosuchalias"),
                BStr::new("after=1"),
            ])
            .unwrap(),
            Flow::Done(1)
        );
        assert!(lookup_alias(sh, BStr::new(b"after"), false).is_some());
    }
}
