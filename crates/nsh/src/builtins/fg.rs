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

use crate::error::Error;
use bstr::BStr;
use libc::{c_int, pid_t};
use std::io::Write;

use crate::error::{INTOFF, INTON};
use crate::jobs::{
    CUR_RUNNING, FORK_BG, FORK_FG, JOBDONE, JOBRUNNING, JOBSTOPPED, getjob, jobno, jobs, outcmd,
    ps_pid, set_curjob, showpipe, waitforjob, xxtcsetpgrp,
};
use crate::options::Options;
use crate::output::Output;

// [spec:dash:def:jobs.fgcmd-fn]
// [spec:dash:sem:jobs.fgcmd-fn]
// `bgcmd` is this function: the C declares it
// `__attribute__((alias("fgcmd")))`, so one definition answers for both
// names and carries both claims.
// [spec:dash:def:jobs.bgcmd-fn]
// [spec:dash:sem:jobs.bgcmd-fn]
pub unsafe fn fgcmd(args: &[&BStr]) -> Result<c_int, Error> {
    let mut jp: usize;
    let out: *mut Output;
    let mode: c_int;
    let mut retval: c_int = 0;

    mode = if args[0].first() == Some(&b'f') {
        FORK_FG
    } else {
        FORK_BG
    };
    let mut opts = crate::options::Options::new(args);
    opts.next(b"")?;
    let operands = opts.operands();
    out = crate::output::stdout();
    /* `do { ... } while (*argv && *++argv)`: one pass on the current job
     * when there is no operand, otherwise one pass per operand. */
    let mut index = 0usize;
    loop {
        let spec = operands.get(index).map(|s| crate::shell::cstring(s));
        jp = getjob(spec.as_ref().map_or(core::ptr::null(), |s| s.as_ptr()), 1)?;
        if mode == FORK_BG {
            set_curjob(jp, CUR_RUNNING);
            let _ = write!(&mut *out, "[{}] ", jobno(jp));
        }
        outcmd(jp, 0, out);
        showpipe(jp, out);
        retval = restartjob(jp, mode)?;

        index += 1;
        if index >= operands.len() {
            break;
        }
    }
    Ok(retval)
}

// [spec:dash:def:jobs.restartjob-fn]
// [spec:dash:sem:jobs.restartjob-fn]
unsafe fn restartjob(jp: usize, mode: c_int) -> Result<c_int, Error> {
    let status: c_int;
    let pgid: pid_t;

    INTOFF();
    'out_lbl: {
        if jobs()[jp].state as c_int == JOBDONE {
            break 'out_lbl;
        }
        jobs()[jp].state = JOBRUNNING as u8;
        pgid = ps_pid(jp, 0);
        if mode == FORK_FG {
            xxtcsetpgrp(pgid)?;
        }
        libc::killpg(pgid, libc::SIGCONT);
        /* the C's `do { … } while (--i)` visits `ps[0]` before it looks
         * at the count, so a job with no processes walks the whole
         * address space; there is nothing to restart in one. */
        for i in 0..jobs()[jp].ps.len() {
            if libc::WIFSTOPPED(jobs()[jp].ps[i].status) {
                jobs()[jp].ps[i].status = -1;
            }
        }
    }
    // out:
    status = if mode == FORK_FG {
        waitforjob(Some(jp))?
    } else {
        0
    };
    INTON();
    Ok(status)
}
