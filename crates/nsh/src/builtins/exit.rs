//! `exit`.
//!
//! Port of `exitcmd` from `src/main.c`. It raises rather than returns:
//! the exception unwinds to the shell's top level, which is where the
//! process ends. A library may not end the process, so the frontend is
//! what turns that unwind into an exit status -- see
//! `[dec:nsh:shell-as-library]`.

use crate::error::Error;
use bstr::BStr;
use libc::c_int;

// [spec:dash:def:main.exitcmd-fn]
// [spec:dash:sem:main.exitcmd-fn]
pub unsafe fn exitcmd(args: &[&BStr]) -> Result<c_int, Error> {
    if crate::jobs::stoppedjobs() != 0 {
        return Ok(0);
    }

    if let Some(status) = args.get(1) {
        let status = crate::shell::cstring(status);
        crate::eval::savestatus = crate::mystring::number(status.as_ptr());
    }

    crate::error::exraise(crate::error::EXEXIT);
    /* NOTREACHED */
}
