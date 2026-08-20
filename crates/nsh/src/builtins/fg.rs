//! `fg` and `bg`.
//!
//! The job table and everything that maintains it stay in `crate::jobs`;
//! this module owns the commands that restart a stopped job. `fg` and `bg`
//! share one implementation and select their behavior from the invoked name.

use crate::context::Shell;
use crate::error::Error;
use crate::evaluation::Flow;
use crate::jobs::{
    ForkMode, JobId, apply_saved_job_terminal_settings, capture_shell_terminal_settings,
    job_number, process_id, resolve_job, set_terminal_process_group, terminal_settings_error,
    wait_for_job, write_command_text, write_pipeline,
};
use crate::output::OutputDestination;
use bstr::BStr;

// [spec:nsh:def:idiom.job-control-model]

// [spec:dash:sem:jobs.fgcmd-fn]
// One definition answers for both command names and carries both claims.
// [spec:dash:sem:jobs.bgcmd-fn]
// [spec:posix:syn:builtin.bg.synopsis]
// [spec:posix:req:builtin.bg.operand-job-id]
// [spec:posix:req:builtin.bg.env-locale]
// [spec:posix:sem:builtin.bg.env-nlspath]
// [spec:posix:req:builtin.bg.stdout-format]
// [spec:posix:req:builtin.bg.stderr]
// [spec:posix:req:builtin.bg.interfaces]
// [spec:posix:syn:builtin.fg.synopsis]
// [spec:posix:req:builtin.fg.operand-job-id]
// [spec:posix:req:builtin.fg.env-locale]
// [spec:posix:sem:builtin.fg.env-nlspath]
// [spec:posix:req:builtin.fg.stdout-format]
// [spec:posix:req:builtin.fg.stderr]
// [spec:posix:req:builtin.fg.interfaces]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut job_id: JobId;
    let mode = if args[0].first() == Some(&b'f') {
        ForkMode::Foreground
    } else {
        ForkMode::Background
    };
    let mut option_scan = crate::options::Options::new(args);
    option_scan.next(&mut shell.diagnostics(), b"")?;
    let operands = option_scan.operands();
    /* `do { ... } while (*argv && *++argv)`: one pass on the current job
     * when there is no operand, otherwise one pass per operand. */
    let mut index = 0usize;
    let status = loop {
        job_id = resolve_job(shell, operands.get(index).copied(), true)?;
        if mode == ForkMode::Background {
            shell.jobs.position_running(job_id);
            shell.write_output_fmt(
                OutputDestination::Stdout,
                format_args!("[{}] ", job_number(job_id)),
            )?;
        }
        write_command_text(shell, job_id, 0, OutputDestination::Stdout)?;
        write_pipeline(shell, job_id, OutputDestination::Stdout)?;
        let status = restart_job(shell, job_id, mode)?;

        index += 1;
        if index >= operands.len() {
            break status;
        }
    };
    Ok(Flow::Done(status))
}

// [spec:dash:sem:jobs.restartjob-fn]
// [spec:posix:req:builtin.bg.resume-suspended-jobs]
// [spec:posix:req:builtin.bg.already-running-no-effect]
// [spec:posix:req:builtin.bg.exit-status]
// [spec:posix:req:builtin.bg.job-control-disabled]
// [spec:posix:req:builtin.fg.move-job-to-foreground]
// [spec:posix:req:builtin.fg.removes-known-process-id]
// [spec:posix:req:builtin.fg.exit-status]
// [spec:posix:req:builtin.fg.job-control-disabled]
// [spec:posix:req:jobctl.background-job-brought-to-foreground]
// [spec:posix:req:jobctl.continue-suspended-job]
// [spec:posix:req:jobctl.fg-terminal-settings-restore]
fn restart_job(
    shell: &mut Shell,
    job_id: JobId,
    mode: ForkMode,
) -> Result<crate::status::ExitStatus, Error> {
    let (status, terminal_error) = crate::error::with_interrupts_deferred(shell, |shell| {
        let mut terminal_error = None;
        'restart_complete: {
            if !shell.jobs[job_id].restart() {
                break 'restart_complete;
            }
            if mode == ForkMode::Foreground {
                capture_shell_terminal_settings(shell)?;
            }
            let Some(leader) = process_id(shell, job_id, 0) else {
                return Err(shell.diagnostics().shell_error(b"job has no process"));
            };
            let process_group = nsh_platform::ProcessGroupId::from_leader(leader);
            if mode == ForkMode::Foreground {
                set_terminal_process_group(shell, process_group)?;
                if let Err(error) = apply_saved_job_terminal_settings(shell, job_id) {
                    terminal_error = Some(error);
                }
            }
            if let Err(error) = nsh_platform::send_continue_to_process_group(process_group) {
                let message = shell.locale.error_message(&error).into_bytes();
                return Err(shell.diagnostics().shell_error(&message));
            }
            /* the C's `do { … } while (--i)` visits `ps[0]` before it looks
             * at the count, so a job with no processes walks the whole
             * address space; there is nothing to restart in one. */
            for process in &mut shell.jobs[job_id].processes {
                if matches!(process.status, Some(nsh_platform::ChildStatus::Stopped(_))) {
                    process.status = None;
                }
            }
        }
        let status = if mode == ForkMode::Foreground {
            wait_for_job(shell, Some(job_id))?
        } else {
            crate::status::ExitStatus::SUCCESS
        };
        Ok::<_, Error>((status, terminal_error))
    })?;
    if let Some(error) = terminal_error {
        return Err(terminal_settings_error(
            shell,
            b"Cannot restore job tty settings",
            error,
        ));
    }
    Ok(status)
}
