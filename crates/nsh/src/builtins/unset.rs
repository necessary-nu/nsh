//! `unset`.
//!
//! Port of `unsetcmd` from `src/var.c`. `-v` and `-f` choose between the
//! variable table and the function table; the last one given wins, and
//! with neither it is the variable table.

use crate::context::Shell;
use crate::error::Error;
use crate::evaluation::Flow;
use crate::options::Options;
use crate::variables::{unset_bytes, variable_attributes};
use bstr::BStr;

// [spec:dash:sem:var.unsetcmd-fn]
// [spec:posix:syn:builtin.unset.synopsis]
// [spec:posix:req:builtin.unset.unset-names]
// [spec:posix:req:builtin.unset.v-option]
// [spec:posix:req:builtin.unset.f-option]
// [spec:posix:req:builtin.unset.no-option]
// [spec:posix:req:builtin.unset.not-previously-set]
// [spec:posix:req:builtin.unset.utility-syntax-guidelines]
// [spec:posix:sem:builtin.unset.empty-assignment-and-special-parameters]
// [spec:posix:req:builtin.unset.stderr]
// [spec:posix:req:builtin.unset.exit-status]
// [spec:posix:sem:builtin.unset.utility-defaults]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut flag: u8 = 0;

    let mut option_scan = Options::new(args);
    while let Some(option) = option_scan.next(&mut shell.diagnostics(), b"vf")? {
        flag = option;
    }

    for name in option_scan.operands() {
        if flag != b'f' {
            // Bash unsets the variable a name reference points at.
            // [spec:nsh:req:compat.bash.functions-scoping]
            let referenced = crate::variables::nameref::read_name(shell, name);
            let name = referenced
                .as_ref()
                .map_or(*name, |target| BStr::new(target.as_slice()));
            if variable_attributes(shell, name).is_some_and(|attributes| attributes.read_only) {
                let mut message = name.to_vec();
                message.extend_from_slice(b" is read-only");
                // [spec:nsh:req:compat.bash.error-boundary]
                return Err(shell.diagnostics().dialect_builtin_error(1, &message));
            }
            // `unset a[i]` removes one element; the whole-array forms
            // clear the elements but keep the declaration.
            if let Some((base, subscript)) = subscripted(name) {
                let base = base.to_owned();
                let subscript = subscript.to_owned();
                let selector = crate::variables::arrays::resolve_text_selector(
                    shell,
                    BStr::new(base.as_slice()),
                    BStr::new(subscript.as_slice()),
                )?;
                crate::variables::arrays::unset_element(
                    shell,
                    BStr::new(base.as_slice()),
                    &selector,
                )?;
                continue;
            }
            unset_bytes(shell, name)?;
            continue;
        }
        if flag != b'v' {
            crate::execution::unset_function(
                &mut shell.interrupt_deferral,
                &mut shell.commands,
                name,
            );
        }
    }
    Ok(Flow::Done((0).into()))
}

/// Split `a[expr]` into its name and subscript, when the operand is one.
// [spec:nsh:req:compat.bash.arrays-declarations]
fn subscripted(operand: &BStr) -> Option<(&BStr, &BStr)> {
    let bytes: &[u8] = operand.as_ref();
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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::lock;
    use crate::variables::{VariableAttributes, lookup_bytes, set_bytes};

    /// With neither option, and with `-v`, the variable goes; `-f` leaves
    /// it alone because it is looking at the other table.
    #[test]
    fn the_option_picks_the_table() {
        let _g = lock();
        let name = BStr::new("Tunset");
        let shell = &mut Shell::new(crate::streams::Streams::INHERIT);

        set_bytes(shell, name, Some(BStr::new("v")), VariableAttributes::NONE).unwrap();
        assert_eq!(
            run(shell, &[BStr::new("unset"), BStr::new("Tunset")]).unwrap(),
            Flow::Done((0).into())
        );
        assert!(lookup_bytes(shell, name).is_none());

        set_bytes(shell, name, Some(BStr::new("v")), VariableAttributes::NONE).unwrap();
        assert_eq!(
            run(
                shell,
                &[BStr::new("unset"), BStr::new("-v"), BStr::new("Tunset")]
            )
            .unwrap(),
            Flow::Done((0).into())
        );
        assert!(lookup_bytes(shell, name).is_none());

        set_bytes(shell, name, Some(BStr::new("v")), VariableAttributes::NONE).unwrap();
        assert_eq!(
            run(
                shell,
                &[BStr::new("unset"), BStr::new("-f"), BStr::new("Tunset")]
            )
            .unwrap(),
            Flow::Done((0).into())
        );
        assert!(
            lookup_bytes(shell, name).is_some(),
            "-f is the function table"
        );
        unset_bytes(shell, name).unwrap();
    }

    /// The last option given wins, so `-f -v` unsets the variable.
    #[test]
    fn the_last_option_wins() {
        let _g = lock();
        let mut owned = Shell::new(crate::streams::Streams::INHERIT);
        let shell = &mut owned;
        let name = BStr::new("Tunset2");
        set_bytes(shell, name, Some(BStr::new("v")), VariableAttributes::NONE).unwrap();
        assert_eq!(
            run(
                shell,
                &[
                    BStr::new("unset"),
                    BStr::new("-f"),
                    BStr::new("-v"),
                    BStr::new("Tunset2"),
                ]
            )
            .unwrap(),
            Flow::Done((0).into())
        );
        assert!(lookup_bytes(shell, name).is_none());
    }
}
