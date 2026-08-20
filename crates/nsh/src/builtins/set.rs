//! `set`.
//!
//! Port of `setcmd` from `src/options.c`. The option scan it runs is
//! `crate::options::options`, shared with the shell's own command line so
//! that `set -x` and `sh -x` cannot drift apart; with operands left over
//! it replaces the positional parameters.
//!
//! With no arguments at all it prints the variables instead, which is the
//! one thing about `set` that has nothing to do with options.

use crate::context::Shell;
use crate::error::Error;
use bstr::BStr;

use crate::evaluation::Flow;
use crate::options::{apply_option_changes, options, set_positional_parameters};
use crate::variables::{VariableSelection, show_vars};

// [spec:dash:def:options.setcmd-fn]
// [spec:dash:sem:options.setcmd-fn]
// [spec:posix:syn:builtin.set.synopsis]
// [spec:posix:req:builtin.set.no-operands-writes-variables]
// [spec:posix:req:builtin.set.variable-output-reinput]
// [spec:posix:sem:builtin.set.options-and-arguments]
// [spec:posix:req:builtin.set.positional-parameters]
// [spec:posix:req:builtin.set.utility-defaults]
// [spec:posix:req:builtin.set.stderr-diagnostics-only]
// [spec:posix:req:builtin.set.exit-status]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    if args.len() == 1 {
        show_vars(shell, BStr::new(b""), VariableSelection::Set)?;
        return Ok(Flow::Done((0).into()));
    }
    crate::error::with_interrupts_deferred(shell, |shell| {
        let scan = options(shell, args, 1)?;
        apply_option_changes(shell)?;
        if scan.next < args.len() {
            set_positional_parameters(shell, &args[scan.next..]);
        }
        Ok::<(), Error>(())
    })?;
    Ok(Flow::Done((0).into()))
}
