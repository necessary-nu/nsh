//! Literal port of `src/bltin/times.c` — the `times` builtin.
//! Rules: `docs/spec/port/src/bltin/times.md`.
//!
//! Built as part of the `bltin` sub-library, but its fixed numeric record
//! now uses Rust formatting directly instead of the old C-variadic shim.

/*
 * Copyright (c) 1999 Herbert Xu <herbert@gondor.apana.org.au>
 * This file contains code for the times builtin.
 */

use crate::error::Error;
use core::mem;
use libc::{c_char, c_int};
use bstr::BStr;
use std::io::Write as _;

// [spec:dash:def:times.timescmd-fn]
// [spec:dash:sem:times.timescmd-fn]
pub unsafe fn timescmd(_args: &[&BStr]) -> Result<c_int, Error> {
    let mut buf: libc::tms = mem::zeroed();
    let clk_tck: libc::c_long = libc::sysconf(libc::_SC_CLK_TCK);
    let mutime: c_int;
    let mstime: c_int;
    let mcutime: c_int;
    let mcstime: c_int;
    let mut utime: f64;
    let mut stime: f64;
    let mut cutime: f64;
    let mut cstime: f64;

    libc::times(&mut buf);

    utime = buf.tms_utime as f64 / clk_tck as f64;
    mutime = (utime / 60.0) as c_int;
    utime -= mutime as f64 * 60.0;

    stime = buf.tms_stime as f64 / clk_tck as f64;
    mstime = (stime / 60.0) as c_int;
    stime -= mstime as f64 * 60.0;

    cutime = buf.tms_cutime as f64 / clk_tck as f64;
    mcutime = (cutime / 60.0) as c_int;
    cutime -= mcutime as f64 * 60.0;

    cstime = buf.tms_cstime as f64 / clk_tck as f64;
    mcstime = (cstime / 60.0) as c_int;
    cstime -= mcstime as f64 * 60.0;

    let _ = write!(
        &mut *crate::output::stdout(),
        "{mutime}m{utime:.6}s {mstime}m{stime:.6}s\n\
         {mcutime}m{cutime:.6}s {mcstime}m{cstime:.6}s\n"
    );
    Ok(0)
}
