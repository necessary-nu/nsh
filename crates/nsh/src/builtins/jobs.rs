//! `jobs`.
//!
//! Port of `jobscmd` from `src/jobs.c`. Rendering a job is
//! `crate::jobs::showjob`, which the shell also does unprompted when a
//! background job changes state, so it stays there.

use crate::error::Error;
use bstr::BStr;
use libc::c_int;

use crate::jobs::{SHOW_PGID, SHOW_PID, getjob, showjob, showjobs};
use crate::options::Options;
use crate::output::Output;

// [spec:dash:def:jobs.jobscmd-fn]
// [spec:dash:sem:jobs.jobscmd-fn]
pub unsafe fn jobscmd(args: &[&BStr]) -> Result<c_int, Error> {
    let mut mode: c_int;
    let out: *mut Output;

    mode = 0;
    let mut opts = crate::options::Options::new(args);
    while let Some(m) = opts.next(b"lp")? {
        if m == b'l' {
            mode = SHOW_PID;
        } else {
            mode = SHOW_PGID;
        }
    }

    out = crate::output::stdout();
    let operands = opts.operands();
    if !operands.is_empty() {
        for spec in operands {
            let spec = crate::shell::cstring(spec);
            showjob(out, getjob(spec.as_ptr(), 0)?, mode);
        }
    } else {
        showjobs(out, mode);
    }

    Ok(0)
}
