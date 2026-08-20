//! `fg` and `bg`.
//!
//! Port of `fgcmd` and `restartjob` from `src/jobs.c`. The job table and
//! everything that maintains it stay in `crate::jobs`; this is the
//! command that restarts a stopped job.
//!
//! `bg` is this same function under the other name -- the C spells it
//! `__attribute__((alias("fgcmd")))` -- and the two are told apart by the
//! word the builtin was called as, so the table's `bg` row points here
//! rather than at a module that would only forward.

use crate::context::Shell;
use crate::error::Error;
use bstr::BStr;
use core::ffi::c_int;
use std::io::Write;

use crate::error::{INTOFF, INTON};
use crate::eval::Flow;
use crate::jobs::{
    CUR_RUNNING, FORK_BG, FORK_FG, JobId, apply_saved_job_terminal_settings,
    capture_shell_terminal_settings, getjob, jobno, outcmd, ps_pid, set_curjob, showpipe,
    terminal_settings_error, waitforjob, xxtcsetpgrp,
};
use crate::output::Dest;

// [spec:nsh:def:idiom.job-control-model]

// [spec:dash:def:jobs.fgcmd-fn]
// [spec:dash:sem:jobs.fgcmd-fn]
// `bgcmd` is this function: the C declares it
// `__attribute__((alias("fgcmd")))`, so one definition answers for both
// names and carries both claims.
// [spec:dash:def:jobs.bgcmd-fn]
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
pub fn fgcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut jp: JobId;
    let mode: c_int;

    mode = if args[0].first() == Some(&b'f') {
        FORK_FG
    } else {
        FORK_BG
    };
    let mut opts = crate::options::Options::new(args);
    opts.next(sh, b"")?;
    let operands = opts.operands();
    /* `do { ... } while (*argv && *++argv)`: one pass on the current job
     * when there is no operand, otherwise one pass per operand. */
    let mut index = 0usize;
    let retval = loop {
        jp = getjob(sh, operands.get(index).copied(), 1)?;
        if mode == FORK_BG {
            set_curjob(sh, jp, CUR_RUNNING);
            let _ = write!(sh.io.get(Dest::Stdout), "[{}] ", jobno(jp));
        }
        outcmd(sh, jp, 0, Dest::Stdout);
        showpipe(sh, jp, Dest::Stdout);
        let status = restartjob(sh, jp, mode)?;

        index += 1;
        if index >= operands.len() {
            break status;
        }
    };
    Ok(Flow::Done((retval).into()))
}

// [spec:dash:def:jobs.restartjob-fn]
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
fn restartjob(sh: &mut Shell, jp: JobId, mode: c_int) -> Result<crate::status::ExitStatus, Error> {
    let process_group: nsh_platform::ProcessGroupId;
    let mut terminal_error = None;

    INTOFF(sh);
    'out_lbl: {
        if !sh.jobs[jp].restart() {
            break 'out_lbl;
        }
        if mode == FORK_FG {
            capture_shell_terminal_settings(sh)?;
        }
        let Some(leader) = ps_pid(sh, jp, 0) else {
            return Err(sh.sh_error_value(b"job has no process"));
        };
        process_group = nsh_platform::ProcessGroupId::from_leader(leader);
        if mode == FORK_FG {
            xxtcsetpgrp(sh, process_group)?;
            if let Err(error) = apply_saved_job_terminal_settings(sh, jp) {
                terminal_error = Some(error);
            }
        }
        let _ = nsh_platform::send_continue_to_process_group(process_group);
        /* the C's `do { … } while (--i)` visits `ps[0]` before it looks
         * at the count, so a job with no processes walks the whole
         * address space; there is nothing to restart in one. */
        for i in 0..sh.jobs[jp].ps.len() {
            if matches!(
                sh.jobs[jp].ps[i].status,
                Some(nsh_platform::ChildStatus::Stopped(_))
            ) {
                sh.jobs[jp].ps[i].status = None;
            }
        }
    }
    // out:
    let status = if mode == FORK_FG {
        waitforjob(sh, Some(jp))
    } else {
        Ok(crate::status::ExitStatus::SUCCESS)
    };
    INTON(sh);
    let status = status?;
    if let Some(error) = terminal_error {
        return Err(terminal_settings_error(
            sh,
            b"Cannot restore job tty settings",
            error,
        ));
    }
    Ok(status)
}
