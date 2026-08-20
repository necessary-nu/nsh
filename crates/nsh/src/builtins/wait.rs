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
use crate::evaluation::Flow;
use crate::jobs::{
    JobId, WaitMode, job_exit_status, reap_children, remove_waited_job, resolve_job,
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
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut job_id: Option<JobId>;
    let mut status: crate::status::ExitStatus;

    let mut option_scan = crate::options::Options::new(args);
    option_scan.next(&mut shell.diagnostics(), b"")?;
    status = crate::status::ExitStatus::SUCCESS;

    let operands = option_scan.operands();
    'wait_complete: {
        if operands.is_empty() {
            /* wait for all jobs */
            loop {
                job_id = None;
                for job_index in shell.jobs.order_snapshot() {
                    if shell.jobs[job_index].is_running() {
                        job_id = Some(job_index);
                        break;
                    }
                    shell.jobs[job_index].waited = true;
                    remove_waited_job(&mut shell.interrupt_deferral, &mut shell.jobs, job_index);
                }
                if job_id.is_none() {
                    /* no running procs */
                    break 'wait_complete;
                }
                if !reap_children(shell, WaitMode::CommandAll, None)? {
                    // sigout:
                    status = crate::signal_inbox::signals()
                        .pending_signal()
                        .expect("an interrupted wait records its signal")
                        .as_status();
                    break 'wait_complete;
                }
            }
        }

        for spec in operands {
            status = crate::status::ExitStatus::NOT_FOUND;
            'operand_complete: {
                if spec.first() != Some(&b'%') {
                    let process = u32::try_from(crate::number::parse_nonnegative(
                        &mut shell.diagnostics(),
                        spec,
                    )?)
                    .ok()
                    .and_then(nsh_platform::ProcessId::new);
                    job_id = None;
                    for job_index in shell.jobs.order_snapshot() {
                        if shell.jobs[job_index]
                            .processes
                            .last()
                            .is_some_and(|candidate| Some(candidate.process_id) == process)
                        {
                            job_id = Some(job_index);
                            break;
                        }
                    }
                    if job_id.is_none() {
                        break 'operand_complete;
                    }
                } else {
                    job_id = Some(resolve_job(shell, Some(spec), false)?);
                }
                /* loop until process terminated or stopped */
                if !reap_children(shell, WaitMode::Command, job_id)? {
                    // sigout:
                    status = crate::signal_inbox::signals()
                        .pending_signal()
                        .expect("an interrupted wait records its signal")
                        .as_status();
                    break 'wait_complete;
                }
                let job_index = job_id.unwrap();
                shell.jobs[job_index].waited = true;
                status = job_exit_status(shell, job_index);
                remove_waited_job(&mut shell.interrupt_deferral, &mut shell.jobs, job_index);
            }
            // repeat:
        }
    }
    // out:
    Ok(Flow::Done(status))
}
