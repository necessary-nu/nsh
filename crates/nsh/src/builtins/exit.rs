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
        return Ok(Flow::Done(0));
    }

    let status = match args.get(1) {
        Some(status) => crate::mystring::number(sh, status)?,
        None => sh.status,
    };

    // In an EXIT action, the operand-free form uses the action's
    // then-current status rather than the status that entered the action.
    // Carrying the value now also keeps nested traps independent.
    // [spec:nsh:req:compat.smoosh.trap-status]
    Ok(Flow::exit(status))
}
