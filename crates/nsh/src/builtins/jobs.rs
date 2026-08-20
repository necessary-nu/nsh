//! `jobs`.
//!
//! Port of `jobscmd` from `src/jobs.c`. Rendering a job is
//! `crate::jobs::showjob`, which the shell also does unprompted when a
//! background job changes state, so it stays there.

// [spec:nsh:req:idiom.operation-modes]
use crate::context::Shell;
use crate::error::Error;
use bstr::BStr;

use crate::evaluation::Flow;
use crate::jobs::{JobDisplay, resolve_job, write_job, write_jobs};
use crate::output::OutputDestination;

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
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut mode = JobDisplay::Standard;
    let mut option_scan = crate::options::Options::new(args);
    while let Some(option) = option_scan.next(&mut shell.diagnostics(), b"lp")? {
        if option == b'l' {
            mode = JobDisplay::Long;
        } else {
            mode = JobDisplay::ProcessGroup;
        }
    }

    let operands = option_scan.operands();
    if !operands.is_empty() {
        for spec in operands {
            /* `getjob` and `showjob` both take the receiver, so the
             * lookup is its own statement rather than an argument. */
            let job_id = resolve_job(shell, Some(spec), false)?;
            write_job(shell, OutputDestination::Stdout, job_id, mode)?;
        }
    } else {
        write_jobs(shell, OutputDestination::Stdout, mode)?;
    }

    Ok(Flow::Done((0).into()))
}
