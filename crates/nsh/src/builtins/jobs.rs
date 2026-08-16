//! `jobs`.
//!
//! Port of `jobscmd` from `src/jobs.c`. Rendering a job is
//! `crate::jobs::showjob`, which the shell also does unprompted when a
//! background job changes state, so it stays there.

use crate::context::Shell;
use crate::error::Error;
use bstr::BStr;
use libc::c_int;

use crate::eval::Flow;
use crate::jobs::{SHOW_PGID, SHOW_PID, getjob, showjob, showjobs};
use crate::options::Options;
use crate::output::Dest;

// [spec:dash:def:jobs.jobscmd-fn]
// [spec:dash:sem:jobs.jobscmd-fn]
pub unsafe fn jobscmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut mode: c_int;

    mode = 0;
    let mut opts = crate::options::Options::new(args);
    while let Some(m) = opts.next(b"lp")? {
        if m == b'l' {
            mode = SHOW_PID;
        } else {
            mode = SHOW_PGID;
        }
    }

    let operands = opts.operands();
    if !operands.is_empty() {
        for spec in operands {
            let spec = crate::shell::cstring(spec);
            /* `getjob` and `showjob` both take the receiver, so the
             * lookup is its own statement rather than an argument. */
            let jp = getjob(sh, spec.as_ptr(), 0)?;
            showjob(sh, Dest::Stdout, jp, mode);
        }
    } else {
        showjobs(sh, Dest::Stdout, mode)?;
    }

    Ok(Flow::Done(0))
}
