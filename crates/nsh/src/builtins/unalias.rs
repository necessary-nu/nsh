//! `unalias`.
//!
//! Port of `unaliascmd` from `src/alias.c`. Removing an entry is
//! `crate::alias`'s business -- an alias being read has to survive its
//! own removal -- so this is the option scan and the diagnostic.

use crate::context::Shell;
use crate::error::Error;
use bstr::BStr;

use crate::alias::{clear_aliases, unalias};
use crate::evaluation::Flow;
use crate::options::Options;
use crate::output::OutputDestination;

// [spec:dash:sem:alias.unaliascmd-fn]
// [spec:posix:syn:builtin.unalias.synopsis]
// [spec:posix:req:builtin.unalias.remove-definitions]
// [spec:posix:req:builtin.unalias.utility-syntax-guidelines]
// [spec:posix:req:builtin.unalias.opt-a]
// [spec:posix:req:builtin.unalias.operand-alias-name]
// [spec:posix:req:builtin.unalias.env-locale]
// [spec:posix:sem:builtin.unalias.env-nlspath]
// [spec:posix:req:builtin.unalias.stderr]
// [spec:posix:req:builtin.unalias.interfaces]
// [spec:posix:req:builtin.unalias.exit-status]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut failed = false;

    let mut option_scan = Options::new(args);
    while let Some(opt) = option_scan.next(&mut shell.diagnostics(), b"a")? {
        if opt == b'a' {
            clear_aliases(&mut shell.interrupt_deferral, &mut shell.aliases);
            return Ok(Flow::Done((0).into()));
        }
    }
    for name in option_scan.operands() {
        if !unalias(&mut shell.interrupt_deferral, &mut shell.aliases, name) {
            let mut message = b"unalias: ".to_vec();
            message.extend_from_slice(name);
            message.extend_from_slice(b" not found\n");
            shell.write_output(OutputDestination::Stderr, &message)?;
            failed = true;
        }
    }

    Ok(Flow::Done(i32::from(failed).into()))
}
