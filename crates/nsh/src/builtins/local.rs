//! `local`.
//!
//! Port of `localcmd` from `src/var.c`. Making a variable local is
//! `crate::variables`'s `mklocal` -- it has to save the old value where the
//! function's return will find it -- so this is the guard and the loop.
//!
//! It scans no options at all, which is why `local -x` localises a
//! variable called `-x` rather than complaining.

use crate::context::Shell;
use crate::error::Error;
use bstr::BStr;

use crate::evaluation::Flow;
use crate::variables::{VariableAttributes, make_local_bytes};

// [spec:dash:def:var.localcmd-fn]
// [spec:dash:sem:var.localcmd-fn]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    if !shell.variables.in_function() {
        return Err(shell.diagnostics().shell_error(b"not in a function"));
    }

    /* `local` scans no options at all, so every word after the command
     * name is a name to localise -- including one that starts with `-`. */
    for name in &args[1..] {
        make_local_bytes(shell, name, VariableAttributes::NONE)?;
    }
    Ok(Flow::Done((0).into()))
}
