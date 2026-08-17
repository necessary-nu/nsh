//! Literal port of `src/bltin/times.c` — the `times` builtin.
//! Rules: `docs/spec/port/src/bltin/times.md`.
//!
//! Built as part of the `bltin` sub-library, but its fixed numeric record
//! now uses Rust formatting directly instead of the old C-variadic shim.

/*
 * Copyright (c) 1999 Herbert Xu <herbert@gondor.apana.org.au>
 * This file contains code for the times builtin.
 */

use crate::context::Shell;
use crate::error::Error;
use crate::eval::Flow;
use core::ffi::c_int;
use bstr::BStr;
use std::io::Write as _;

// [spec:dash:def:times.timescmd-fn]
// [spec:dash:sem:times.timescmd-fn]
pub fn timescmd(sh: &mut Shell, _args: &[&BStr]) -> Result<Flow, Error> {
    let times = nsh_platform::process_times();
    let mutime: c_int;
    let mstime: c_int;
    let mcutime: c_int;
    let mcstime: c_int;
    let mut utime = times.user;
    let mut stime = times.system;
    let mut cutime = times.children_user;
    let mut cstime = times.children_system;

    mutime = (utime / 60.0) as c_int;
    utime -= mutime as f64 * 60.0;

    mstime = (stime / 60.0) as c_int;
    stime -= mstime as f64 * 60.0;

    mcutime = (cutime / 60.0) as c_int;
    cutime -= mcutime as f64 * 60.0;

    mcstime = (cstime / 60.0) as c_int;
    cstime -= mcstime as f64 * 60.0;

    let _ = write!(
        sh.io.stdout(),
        "{mutime}m{utime:.6}s {mstime}m{stime:.6}s\n\
         {mcutime}m{cutime:.6}s {mcstime}m{cstime:.6}s\n"
    );
    Ok(Flow::Done(0))
}
