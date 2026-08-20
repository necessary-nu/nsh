//! `jobs`.
//!
//! Port of `jobscmd` from `src/jobs.c`. Rendering a job is
//! `crate::jobs::showjob`, which the shell also does unprompted when a
//! background job changes state, so it stays there.

// [spec:nsh:req:idiom.operation-modes]
use crate::context::Shell;
use crate::error::Error;
use bstr::BStr;

use crate::eval::Flow;
use crate::jobs::{JobDisplay, getjob, showjob, showjobs};
use crate::output::Dest;

// [spec:nsh:def:idiom.job-control-model]

// [spec:dash:def:jobs.jobscmd-fn]
// [spec:dash:sem:jobs.jobscmd-fn]
// [spec:posix:syn:builtin.jobs.synopsis]
// [spec:posix:req:builtin.jobs.display-background-jobs]
// [spec:posix:req:builtin.jobs.utility-syntax-guidelines]
// [spec:posix:req:builtin.jobs.option-l]
// [spec:posix:req:builtin.jobs.option-p]
// [spec:posix:req:builtin.jobs.default-display]
// [spec:posix:req:builtin.jobs.operand-job-id]
// [spec:posix:req:builtin.jobs.env-locale]
// [spec:posix:sem:builtin.jobs.env-nlspath]
// [spec:posix:req:builtin.jobs.stderr]
// [spec:posix:req:builtin.jobs.exit-status]
// [spec:posix:req:builtin.jobs.interfaces]
pub fn jobscmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut mode = JobDisplay::Standard;
    let mut opts = crate::options::Options::new(args);
    while let Some(m) = opts.next(&mut sh.diagnostics(), b"lp")? {
        if m == b'l' {
            mode = JobDisplay::Long;
        } else {
            mode = JobDisplay::ProcessGroup;
        }
    }

    let operands = opts.operands();
    if !operands.is_empty() {
        for spec in operands {
            /* `getjob` and `showjob` both take the receiver, so the
             * lookup is its own statement rather than an argument. */
            let jp = getjob(sh, Some(spec), false)?;
            showjob(sh, Dest::Stdout, jp, mode)?;
        }
    } else {
        showjobs(sh, Dest::Stdout, mode)?;
    }

    Ok(Flow::Done((0).into()))
}
