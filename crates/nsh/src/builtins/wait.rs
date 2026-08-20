//! `wait`.
//!
//! Port of `waitcmd` from `src/jobs.c`.
//!
//! With no operand it waits for every job, and the loop that does so is
//! the one place the shell blocks with no job in mind; with operands it
//! waits per job, and a pid that names no job is not an error -- the
//! status for it is 127.

use crate::context::Shell;
use crate::error::Error;
use crate::eval::Flow;
use crate::jobs::{
    DOWAIT_WAITCMD, DOWAIT_WAITCMD_ALL, JobId, dowait, getjob, getstatus, remove_waited_job,
};
use bstr::BStr;

// [spec:nsh:def:idiom.job-control-model]

// [spec:dash:def:jobs.waitcmd-fn]
// [spec:dash:sem:jobs.waitcmd-fn]
// [spec:posix:req:signal.trap-during-wait]
// [spec:posix:syn:builtin.wait.synopsis]
// [spec:posix:req:builtin.wait.await-children]
// [spec:posix:req:builtin.wait.no-operands]
// [spec:posix:req:builtin.wait.exit-status-last-operand]
// [spec:posix:req:builtin.wait.exit-status-values]
// [spec:posix:req:builtin.wait.pid-operands]
// [spec:posix:req:builtin.wait.remove-waited-for-pid]
// [spec:posix:def:builtin.wait.operand-pid-number]
// [spec:posix:req:builtin.wait.operand-pid-job-id]
// [spec:posix:req:builtin.wait.env-vars]
// [spec:posix:sem:builtin.wait.env-nlspath]
// [spec:posix:req:builtin.wait.stderr]
// [spec:posix:req:builtin.wait.interfaces]
// [spec:posix:req:builtin.wait.exit-status-signal]
pub fn waitcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut jobp: Option<JobId>;
    let mut retval: crate::status::ExitStatus;
    let mut jp: Option<JobId>;

    let mut opts = crate::options::Options::new(args);
    opts.next(&mut sh.diagnostics(), b"")?;
    retval = crate::status::ExitStatus::SUCCESS;

    let operands = opts.operands();
    'out_lbl: {
        if operands.is_empty() {
            /* wait for all jobs */
            loop {
                jp = None;
                for i in sh.jobs.order_snapshot() {
                    if sh.jobs[i].is_running() {
                        jp = Some(i);
                        break;
                    }
                    sh.jobs[i].waited = true;
                    remove_waited_job(&mut sh.interrupt_deferral, &mut sh.jobs, i);
                }
                if jp.is_none() {
                    /* no running procs */
                    break 'out_lbl;
                }
                if dowait(sh, DOWAIT_WAITCMD_ALL, None)? == 0 {
                    // sigout:
                    retval = crate::siginbox::signals()
                        .pending_signal()
                        .expect("an interrupted wait records its signal")
                        .as_status();
                    break 'out_lbl;
                }
            }
        }

        for spec in operands {
            retval = crate::status::ExitStatus::NOT_FOUND;
            'repeat: {
                if spec.first() != Some(&b'%') {
                    let process = u32::try_from(crate::number::parse_nonnegative(
                        &mut sh.diagnostics(),
                        spec,
                    )?)
                    .ok()
                    .and_then(nsh_platform::ProcessId::new);
                    jobp = None;
                    for i in sh.jobs.order_snapshot() {
                        if sh.jobs[i]
                            .ps
                            .last()
                            .is_some_and(|candidate| Some(candidate.pid) == process)
                        {
                            jobp = Some(i);
                            break;
                        }
                    }
                    if jobp.is_none() {
                        break 'repeat;
                    }
                } else {
                    jobp = Some(getjob(sh, Some(spec), 0)?);
                }
                /* loop until process terminated or stopped */
                if dowait(sh, DOWAIT_WAITCMD, jobp)? == 0 {
                    // sigout:
                    retval = crate::siginbox::signals()
                        .pending_signal()
                        .expect("an interrupted wait records its signal")
                        .as_status();
                    break 'out_lbl;
                }
                let i = jobp.unwrap();
                sh.jobs[i].waited = true;
                retval = getstatus(sh, i);
                remove_waited_job(&mut sh.interrupt_deferral, &mut sh.jobs, i);
            }
            // repeat:
        }
    }
    // out:
    Ok(Flow::Done((retval).into()))
}
