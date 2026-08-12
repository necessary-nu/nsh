//! `wait`.
//!
//! Port of `waitcmd` from `src/jobs.c`.
//!
//! With no operand it waits for every job, and the loop that does so is
//! the one place the shell blocks with no job in mind; with operands it
//! waits per job, and a pid that names no job is not an error -- the
//! status for it is 127.

use crate::error::Error;
use bstr::BStr;
use libc::{c_int, pid_t};

use crate::jobs::{
    DOWAIT_WAITCMD, DOWAIT_WAITCMD_ALL, JOBRUNNING, curjob, dowait, getjob, getstatus, jobs,
};
use crate::options::Options;

// [spec:dash:def:jobs.waitcmd-fn]
// [spec:dash:sem:jobs.waitcmd-fn]
pub unsafe fn waitcmd(args: &[&BStr]) -> Result<c_int, Error> {
    let mut jobp: Option<usize>;
    let mut retval: c_int;
    let mut jp: Option<usize>;

    let mut opts = crate::options::Options::new(args);
    opts.next(b"");
    retval = 0;

    let operands = opts.operands();
    'out_lbl: {
        if operands.is_empty() {
            /* wait for all jobs */
            loop {
                jp = curjob;
                loop {
                    let Some(i) = jp else {
                        /* no running procs */
                        break 'out_lbl;
                    };
                    if jobs()[i].state as c_int == JOBRUNNING {
                        break;
                    }
                    jobs()[i].waited = 1;
                    jp = jobs()[i].prev_job;
                }
                if dowait(DOWAIT_WAITCMD_ALL, None) == 0 {
                    // sigout:
                    retval = 128 + crate::trap::pending_sig;
                    break 'out_lbl;
                }
            }
        }

        retval = 127;
        for spec in operands {
            let target = crate::shell::cstring(spec);
            'repeat: {
                if spec.first() != Some(&b'%') {
                    let pid: pid_t = crate::mystring::number(target.as_ptr())?;
                    jobp = curjob;
                    /* `goto start` enters the do/while at `start:` */
                    let mut at_start = true;
                    loop {
                        if !at_start {
                            /* C indexes `job->ps[job->nprocs - 1]`, which
                             * for a job that has not forked yet is
                             * `ps[-1]`; such a job matches no pid. */
                            let i = jobp.unwrap();
                            if jobs()[i].ps.last().map_or(false, |p| p.pid == pid) {
                                break;
                            }
                            jobp = jobs()[i].prev_job;
                        }
                        at_start = false;
                        // start:
                        if jobp.is_none() {
                            break 'repeat;
                        }
                    }
                } else {
                    jobp = Some(getjob(target.as_ptr(), 0)?);
                }
                /* loop until process terminated or stopped */
                if dowait(DOWAIT_WAITCMD, jobp) == 0 {
                    // sigout:
                    retval = 128 + crate::trap::pending_sig;
                    break 'out_lbl;
                }
                let i = jobp.unwrap();
                jobs()[i].waited = 1;
                retval = getstatus(i);
            }
            // repeat:
        }
    }
    // out:
    Ok(retval)
}
