//! Bash's `let`: arithmetic evaluated for its effects, not its value.
//!
//! There is no second arithmetic implementation here and there must not
//! be: `let x=1` is `$((x=1))` with the result thrown away, so this is a
//! loop over [`crate::arithmetic::evaluate`] and a status. The status is
//! the one thing `let` adds -- it reports *false* when the last
//! expression evaluated to zero, which is the opposite sense to an exit
//! status and the reason `let x=0` is a failing command.

use bstr::BStr;

use crate::context::Shell;
use crate::error::Error;
use crate::evaluation::Flow;
use crate::status::ExitStatus;

// [spec:nsh:req:compat.bash.builtins-special-variables]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let expressions = &args[1.min(args.len())..];
    if expressions.is_empty() {
        return Err(shell.diagnostics().shell_error(b"let: expression expected"));
    }
    let mut last = 0i64;
    for expression in expressions {
        last = crate::arithmetic::evaluate(shell, expression)?;
    }
    Ok(Flow::Done(if last == 0 {
        ExitStatus::FAILURE
    } else {
        ExitStatus::SUCCESS
    }))
}
