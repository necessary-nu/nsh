//! `exit`.
//!
//! Port of `exitcmd` from `src/main.c`. The C raises `EXEXIT` and the
//! exception unwinds to the shell's top level, which is where the process
//! ends. It returns that decision instead in the `Ok`
//! position, because an exit is control flow and not an error
//! (`[dec:nsh:errors-are-values]`, `docs/api-design.md` 3.1). The selected
//! status travels in `Flow::Exit` with that decision rather than through
//! ambient evaluator state. A library may not end the process, so the
//! frontend is what turns the returned exit into a process status -- see
//! `[dec:nsh:shell-as-library]`.

use crate::context::Shell;
use crate::error::Error;
use crate::eval::Flow;
use bstr::BStr;

// [spec:dash:def:main.exitcmd-fn]
// [spec:dash:sem:main.exitcmd-fn]
// [spec:posix:syn:builtin.exit.syn]
// [spec:posix:req:builtin.exit.cause-shell-exit]
// [spec:posix:req:builtin.exit.wait-status-from-n]
// [spec:posix:sem:builtin.exit.invalid-n-unspecified]
// [spec:posix:req:builtin.exit.default-n]
// [spec:posix:req:builtin.exit.exit-trap]
// [spec:posix:req:builtin.exit.stderr]
// [spec:posix:req:builtin.exit.interfaces]
// [spec:posix:sem:builtin.exit.exit-status]
pub fn exitcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    if crate::jobs::stoppedjobs(sh) != 0 {
        return Ok(Flow::Done((0).into()));
    }

    let status = match args.get(1) {
        Some(status) => crate::status::ExitStatus::from_code(crate::number::parse_nonnegative(
            &mut sh.diagnostics(),
            status,
        )?),
        // POSIX gives operand-less `exit` the status immediately preceding
        // a trap action when the command directly ends that action. A
        // subshell clears this context, so the Smoosh nested-action case
        // still uses the subshell's then-current status.
        None => sh.eval.trap_default_exit_status.unwrap_or(sh.status),
    };

    // Carrying the selected value keeps nested traps independent.
    // [spec:nsh:req:compat.smoosh.trap-status]
    Ok(Flow::exit(status))
}
