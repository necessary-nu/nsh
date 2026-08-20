//! `export` and `readonly`.
//!
//! Port of `exportcmd` from `src/var.c`. One function under two names,
//! telling them apart by the word it was called as -- the two differ only
//! in which flag they set on the variable.
//!
//! The variable table stays in `crate::variables`. What is here is the argument
//! handling: with no operands it prints the set, and with them it sets a
//! flag on names that exist and creates the ones that do not.

use crate::context::Shell;
use crate::error::Error;
use bstr::BStr;

use crate::evaluation::Flow;
use crate::options::Options;
use crate::variables::{
    VariableAttributes, VariableSelection, add_attributes, set_bytes, show_vars,
    variable_attributes,
};

// [spec:dash:sem:var.exportcmd-fn]
// [spec:posix:syn:builtin.export.synopsis]
// [spec:posix:req:builtin.export.set-attribute]
// [spec:posix:req:builtin.export.declaration-utility]
// [spec:posix:req:builtin.export.utility-syntax-guidelines]
// [spec:posix:req:builtin.export.p-output-format]
// [spec:posix:req:builtin.export.p-output-reinput]
// [spec:posix:sem:builtin.export.no-arguments]
// [spec:posix:req:builtin.export.stderr]
// [spec:posix:req:builtin.export.exit-status]
// [spec:posix:sem:builtin.export.utility-defaults]
// [spec:posix:syn:builtin.readonly.synopsis]
// [spec:posix:req:builtin.readonly.set-attribute]
// [spec:posix:def:builtin.readonly.attribute]
// [spec:posix:req:builtin.readonly.application-constraint]
// [spec:posix:req:builtin.readonly.declaration-utility]
// [spec:posix:req:builtin.readonly.utility-syntax-guidelines]
// [spec:posix:sem:builtin.readonly.p-output-format]
// [spec:posix:req:builtin.readonly.p-output-reinput]
// [spec:posix:sem:builtin.readonly.no-arguments]
// [spec:posix:req:builtin.readonly.stderr]
// [spec:posix:req:builtin.readonly.exit-status]
// [spec:posix:sem:builtin.readonly.utility-defaults]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    /* `export` and `readonly` are one builtin telling itself apart by the
     * word it was called as. */
    let (attribute, selection) = if args[0].first() == Some(&b'r') {
        (VariableAttributes::READ_ONLY, VariableSelection::ReadOnly)
    } else {
        (VariableAttributes::EXPORTED, VariableSelection::Exported)
    };

    let mut option_scan = Options::new(args);
    let export_operands = option_scan.next(&mut shell.diagnostics(), b"p")?.is_none();
    let operands = option_scan.operands();
    if export_operands && !operands.is_empty() {
        for word in operands {
            match word.iter().position(|&byte| byte == b'=') {
                Some(at) => {
                    let name = BStr::new(&word[..at]);
                    if variable_attributes(shell, name)
                        .is_some_and(|attributes| attributes.read_only)
                    {
                        let mut message = name.to_vec();
                        message.extend_from_slice(b": is read only");
                        return Err(shell.diagnostics().builtin_error_value(1, &message));
                    }
                    set_bytes(shell, name, Some(BStr::new(&word[at + 1..])), attribute)?;
                }
                None => {
                    if !add_attributes(shell, word, attribute) {
                        set_bytes(shell, word, None, attribute)?;
                    }
                }
            }
        }
    } else {
        show_vars(shell, args[0], selection)?;
    }
    Ok(Flow::Done((0).into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::lock;
    use crate::variables::{VariableAttributes, lookup_bytes, set_bytes, variable_attributes};

    /// The shell is the caller's: `export` reads and writes the variable
    /// table, which belongs to an instance, so a `Shell` made in here
    /// would be a different set of variables from the one the test set up.
    fn invoke(shell: &mut Shell, name: &[u8], words: &[&[u8]]) -> Flow {
        let mut args = vec![BStr::new(name)];
        args.extend(words.iter().map(|word| BStr::new(*word)));
        super::run(shell, &args).unwrap()
    }

    /// The word the builtin was called as picks the flag, which is the
    /// whole of the difference between the two commands.
    #[test]
    fn the_calling_name_picks_the_flag() {
        let _g = lock();
        let mut owned = Shell::new(crate::streams::Streams::INHERIT);
        let shell = &mut owned;
        let name = BStr::new("Texport");
        set_bytes(shell, name, Some(BStr::new("v")), VariableAttributes::FIXED).unwrap();

        assert_eq!(
            invoke(shell, b"export", &[b"Texport"]),
            Flow::Done((0).into())
        );
        assert!(variable_attributes(shell, name).unwrap().exported);
        assert!(!variable_attributes(shell, name).unwrap().read_only);

        assert_eq!(
            invoke(shell, b"readonly", &[b"Texport"]),
            Flow::Done((0).into())
        );
        assert!(variable_attributes(shell, name).unwrap().read_only);
    }

    /// An operand carrying a value assigns as well as flags, which is
    /// what makes `export` one of the assignment builtins.
    #[test]
    fn an_operand_may_assign() {
        let _g = lock();
        let mut owned = Shell::new(crate::streams::Streams::INHERIT);
        let shell = &mut owned;
        assert_eq!(
            invoke(shell, b"export", &[b"Texport2=set"]),
            Flow::Done((0).into())
        );
        let name = BStr::new("Texport2");
        assert_eq!(
            lookup_bytes(shell, name).map(Vec::from),
            Some(b"set".to_vec())
        );
        assert!(variable_attributes(shell, name).unwrap().exported);
    }
}
