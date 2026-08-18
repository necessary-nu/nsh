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
use bstr::BStr;
use core::ffi::{c_int};

use crate::eval::Flow;
use crate::jobs::{
    DOWAIT_WAITCMD, DOWAIT_WAITCMD_ALL, JOBRUNNING, dowait, getjob, getstatus,
};

// [spec:dash:def:jobs.waitcmd-fn]
// [spec:dash:sem:jobs.waitcmd-fn]
// [spec:posix:req:signal.trap-during-wait]
// [spec:posix:syn:builtin.wait.synopsis]
// [spec:posix:req:builtin.wait.await-children]
// [spec:posix:req:builtin.wait.no-operands]
// [spec:posix:req:builtin.wait.exit-status-last-operand]
// [spec:posix:req:builtin.wait.exit-status-values]
// [spec:posix:req:builtin.wait.pid-operands]
// [spec:posix:def:builtin.wait.operand-pid-number]
// [spec:posix:req:builtin.wait.operand-pid-job-id]
// [spec:posix:req:builtin.wait.env-vars]
// [spec:posix:sem:builtin.wait.env-nlspath]
// [spec:posix:req:builtin.wait.stderr]
// [spec:posix:req:builtin.wait.interfaces]
// [spec:posix:req:builtin.wait.exit-status-signal]
pub fn waitcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut jobp: Option<usize>;
    let mut retval: c_int;
    let mut jp: Option<usize>;

    let mut opts = crate::options::Options::new(args);
    opts.next(sh, b"")?;
    retval = 0;

    let operands = opts.operands();
    'out_lbl: {
        if operands.is_empty() {
            /* wait for all jobs */
            loop {
                jp = sh.jobs.curjob;
                loop {
                    let Some(i) = jp else {
                        /* no running procs */
                        break 'out_lbl;
                    };
                    if sh.jobs.tab[i].state as c_int == JOBRUNNING {
                        break;
                    }
                    sh.jobs.tab[i].waited = 1;
                    jp = sh.jobs.tab[i].prev_job;
                }
                if dowait(sh, DOWAIT_WAITCMD_ALL, None)? == 0 {
                    // sigout:
                    retval = 128 + crate::siginbox::signals().pending_signal();
                    break 'out_lbl;
                }
            }
        }

        for spec in operands {
            retval = 127;
            'repeat: {
                if spec.first() != Some(&b'%') {
                    let pid: i32 = crate::mystring::number(sh, spec)?;
                    jobp = sh.jobs.curjob;
                    /* `goto start` enters the do/while at `start:` */
                    let mut at_start = true;
                    loop {
                        if !at_start {
                            /* C indexes `job->ps[job->nprocs - 1]`, which
                             * for a job that has not forked yet is
                             * `ps[-1]`; such a job matches no pid. */
                            let i = jobp.unwrap();
                            if sh.jobs.tab[i].ps.last().map_or(false, |p| p.pid == pid) {
                                break;
                            }
                            jobp = sh.jobs.tab[i].prev_job;
                        }
                        at_start = false;
                        // start:
                        if jobp.is_none() {
                            break 'repeat;
                        }
                    }
                } else {
                    jobp = Some(getjob(sh, Some(spec), 0)?);
                }
                /* loop until process terminated or stopped */
                if dowait(sh, DOWAIT_WAITCMD, jobp)? == 0 {
                    // sigout:
                    retval = 128 + crate::siginbox::signals().pending_signal();
                    break 'out_lbl;
                }
                let i = jobp.unwrap();
                sh.jobs.tab[i].waited = 1;
                retval = getstatus(sh, i);
            }
            // repeat:
        }
    }
    // out:
    Ok(Flow::Done(retval))
}
