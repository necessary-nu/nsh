//! `exit`.
//!
//! Port of `exitcmd` from `src/main.c`. The C raises `EXEXIT` and the
//! exception unwinds to the shell's top level, which is where the process
//! ends. It returns that decision instead: `Flow::EXIT` in the `Ok`
//! position, because an exit is control flow and not an error
//! (`[dec:nsh:errors-are-values]`, `docs/api-design.md` 3.1). The status
//! it was asked for is in `eval::savestatus`, which `init::exitreset`
//! restores -- and `Flow::Exit`'s `by_exitcmd` is precisely the bit that
//! tells `exitreset` to do so. A library may not end the process, so the
//! frontend is what turns the returned exit into a process status -- see
//! `[dec:nsh:shell-as-library]`.

use crate::context::Shell;
use crate::error::Error;
use crate::eval::Flow;
use bstr::BStr;

// [spec:dash:def:main.exitcmd-fn]
// [spec:dash:sem:main.exitcmd-fn]
pub unsafe fn exitcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    if crate::jobs::stoppedjobs(sh) != 0 {
        return Ok(Flow::Done(0));
    }

    if let Some(status) = args.get(1) {
        let status = crate::shell::cstring(status);
        crate::eval::savestatus = crate::mystring::number(status.as_ptr())?;
    }

    Ok(Flow::EXIT)
}
