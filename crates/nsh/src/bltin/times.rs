//! Literal port of `src/bltin/times.c` — the `times` builtin.
//! Rules: `docs/spec/port/src/bltin/times.md`.
//!
//! Built as part of the `bltin` sub-library, so `printf` here is the
//! shell's own `out1fmt` shim from `bltin.h` (the `USE_GLIBC_STDIO`
//! configuration, which would use real stdio, is not the one we port).
//!
//! Cross-module signatures assumed: `crate::output::out1fmt!` via
//! `bltin.h`'s `printf` alias.

/*
 * Copyright (c) 1999 Herbert Xu <herbert@gondor.apana.org.au>
 * This file contains code for the times builtin.
 */

use core::mem;
use libc::{c_char, c_int};

// [spec:dash:def:times.timescmd-fn]
// [spec:dash:sem:times.timescmd-fn]
pub unsafe fn timescmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
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

    printf!(
        c"%dm%fs %dm%fs\n%dm%fs %dm%fs\n".as_ptr(),
        mutime,
        utime,
        mstime,
        stime,
        mcutime,
        cutime,
        mcstime,
        cstime
    );
    0
}
