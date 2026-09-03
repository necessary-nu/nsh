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

use bstr::BString;

use crate::evaluation::Flow;
use crate::options::Options;
use crate::status::ExitStatus;
use crate::variables::arrays::{self, ArraySelector, ReadOnlyGuard};
use crate::variables::nameref::{self, RefusedTarget};
use crate::variables::value::VariableKind;
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

    /* `-a` and `-A` are Bash's, and they are not attributes here: they
     * say how a compound operand is to be read, which is why
     * `readonly -A m` with no value leaves `m` a plain name. The POSIX
     * dialect has no arrays and keeps refusing both letters. */
    // [spec:nsh:req:compat.bash.arrays-declarations]
    let letters: &[u8] = if shell.options.dialect() == crate::options::Dialect::Bash {
        b"paA"
    } else {
        b"p"
    };
    let mut option_scan = Options::new(args);
    let mut print = false;
    while let Some(letter) = option_scan.next(&mut shell.diagnostics(), letters)? {
        match letter {
            b'a' => shell.evaluation.declared_kind = Some(VariableKind::Indexed),
            b'A' => shell.evaluation.declared_kind = Some(VariableKind::Associative),
            _ => print = true,
        }
    }
    let export_operands = !print;
    let operands = option_scan.operands();
    let mut status = ExitStatus::SUCCESS;
    if export_operands && !operands.is_empty() {
        for word in operands {
            /* `export NAME${suffix}+=value` reaches here as one expanded
             * word, so the `+=` is the built-in's to read. */
            // [spec:nsh:req:compat.bash.arrays-declarations]
            match arrays::split_assignment_operand(word) {
                (name, Some(value), append) => {
                    let name = BStr::new(name.as_slice());
                    if variable_attributes(shell, name)
                        .is_some_and(|attributes| attributes.read_only)
                    {
                        let mut message = name.to_vec();
                        message.extend_from_slice(b": is read only");
                        // [spec:nsh:req:compat.bash.error-boundary]
                        return Err(shell.diagnostics().builtin_error_value(1, &message));
                    }
                    let value = BStr::new(value.as_slice());
                    match shell.evaluation.declared_kind {
                        /* The attribute still lands: Bash reports the
                         * kind it will not convert, drops the value, and
                         * leaves `declare -ar a` behind. */
                        // [spec:nsh:req:compat.bash.arrays-declarations]
                        Some(kind) if !arrays::convertible(shell, name, kind) => {
                            arrays::reject_conversion(shell, args[0], name, kind)?;
                            status = ExitStatus::FAILURE;
                            add_attributes(shell, name, attribute);
                            continue;
                        }
                        Some(kind) => store_array_element(shell, name, kind, value, append)?,
                        None if append => arrays::assign_text_target(shell, name, value, true)?,
                        None => {
                            set_bytes(shell, name, Some(value), attribute)?;
                            continue;
                        }
                    }
                    match nameref::attributed_name(shell, name) {
                        Ok(target) => {
                            add_attributes(shell, BStr::new(target.as_slice()), attribute);
                        }
                        Err(refusal) => report_refusal(shell, name, &refusal),
                    }
                }
                (_, None, _) => {
                    /* A `declare -n` reference is read through here, so
                     * `readonly rr` protects what `rr` names rather than
                     * `rr` itself. */
                    // [spec:nsh:req:compat.bash.functions-scoping]
                    let target = match nameref::attributed_name(shell, word) {
                        Ok(target) => target,
                        Err(refusal) => {
                            report_refusal(shell, word, &refusal);
                            continue;
                        }
                    };
                    let target = BStr::new(target.as_slice());
                    if add_attributes(shell, target, attribute) {
                        continue;
                    }
                    /* The entry is brought into being bare rather than
                     * through `set_bytes`, which would let `set -a` mark
                     * it: `set -a; readonly -a z=(1)` is `declare -ar z`
                     * in the reference where `set -a; readonly -a z`,
                     * with nothing behind it, is `declare -rx z`. */
                    // [spec:nsh:req:compat.bash.arrays-declarations]
                    if declares_a_held_value(shell, word) {
                        nameref::ensure_entry(shell, target);
                        add_attributes(shell, target, attribute);
                        continue;
                    }
                    set_bytes(shell, target, None, attribute)?;
                }
            }
        }
    } else {
        /* With no operand there is no compound value for `-a` or `-A` to
         * describe, and Bash spends the letter on the listing instead:
         * `readonly -a` names the read-only indexed arrays and nothing
         * else. */
        // [spec:nsh:req:compat.bash.arrays-declarations]
        show_vars(shell, args[0], selection, shell.evaluation.declared_kind)?;
    }
    Ok(Flow::Done(status))
}

/// Store the value of an operand the array letter reached, as the zero
/// element of the array it declares.
///
/// The element writer rather than `set_bytes` because that one marks the
/// name under `set -a` and this must not: `set -a; readonly -a z=1` is
/// `declare -ar z` in the reference where `set -a; readonly z=1` is
/// `declare -rx z`.
// [spec:nsh:req:compat.bash.arrays-declarations]
fn store_array_element(
    shell: &mut Shell,
    name: &BStr,
    kind: VariableKind,
    value: &BStr,
    append: bool,
) -> Result<(), Error> {
    arrays::ensure_kind(
        shell,
        name,
        kind,
        VariableAttributes::NONE,
        ReadOnlyGuard::Enforce,
    )?;
    let selector = match kind {
        VariableKind::Associative => ArraySelector::Key(BString::from("0")),
        VariableKind::Indexed | VariableKind::Scalar => ArraySelector::Index(0),
    };
    arrays::assign_element(
        shell,
        name,
        &selector,
        value,
        append,
        ReadOnlyGuard::Enforce,
    )
}

/// Whether the array letter has a compound value coming for this
/// operand, which is what makes it a declaration rather than an
/// assignment and so puts it out of `set -a`'s reach.
// [spec:nsh:req:compat.bash.arrays-declarations]
fn declares_a_held_value(shell: &Shell, word: &BStr) -> bool {
    shell.evaluation.declared_kind.is_some()
        && shell
            .evaluation
            .held_declarations
            .iter()
            .any(|held| held == word)
}

/// Report a reference the attribute could not be applied through.
///
/// Bash writes both of these and leaves the exit status at zero, so the
/// operands beside this one still take their attribute and the command
/// still answers 0.
// [spec:nsh:req:compat.bash.functions-scoping]
fn report_refusal(shell: &mut Shell, name: &BStr, refusal: &RefusedTarget) {
    let mut message = Vec::new();
    match refusal {
        RefusedTarget::Circular => {
            message.extend_from_slice(b"warning: ");
            message.extend_from_slice(name.as_ref());
            message.extend_from_slice(b": circular name reference");
        }
        RefusedTarget::NoName(text) => {
            message.push(b'`');
            message.extend_from_slice(text.as_slice());
            message.extend_from_slice(b"': not a valid identifier");
        }
    }
    shell.diagnostics().shell_warning(&message);
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
