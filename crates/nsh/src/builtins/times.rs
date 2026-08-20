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
use crate::evaluation::Flow;
use crate::output::OutputDestination;
use bstr::BStr;

// [spec:dash:sem:times.timescmd-fn]
// [spec:posix:syn:builtin.times.synopsis]
// [spec:posix:req:builtin.times.output-format]
// [spec:posix:req:builtin.times.tms-correspondence]
// [spec:posix:req:builtin.times.stderr]
// [spec:posix:req:builtin.times.exit-status]
// [spec:posix:sem:builtin.times.utility-defaults]
pub fn run(shell: &mut Shell, _args: &[&BStr]) -> Result<Flow, Error> {
    let times = nsh_platform::process_times();
    let mutime: i32;
    let mstime: i32;
    let mcutime: i32;
    let mcstime: i32;
    let mut utime = times.user;
    let mut stime = times.system;
    let mut cutime = times.children_user;
    let mut cstime = times.children_system;

    mutime = (utime / 60.0) as i32;
    utime -= mutime as f64 * 60.0;

    mstime = (stime / 60.0) as i32;
    stime -= mstime as f64 * 60.0;

    mcutime = (cutime / 60.0) as i32;
    cutime -= mcutime as f64 * 60.0;

    mcstime = (cstime / 60.0) as i32;
    cstime -= mcstime as f64 * 60.0;

    shell.write_output_fmt(
        OutputDestination::Stdout,
        format_args!(
            "{mutime}m{utime:.6}s {mstime}m{stime:.6}s\n\
             {mcutime}m{cutime:.6}s {mcstime}m{cstime:.6}s\n"
        ),
    )?;
    Ok(Flow::Done((0).into()))
}
