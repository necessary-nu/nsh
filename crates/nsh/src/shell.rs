//! Shell-wide helpers that do not belong to a subsystem.
//! Rules: `docs/spec/port/src/shell.md`.

// [spec:nsh:req:idiom.no-port-fossils]

use core::ffi::c_int;

/*
 * Hack to calculate maximum length.
 * (length * 8 - 1) * log10(2) + 1 + 1 + 12
 * The second 1 is for the minus sign and the 12 is a safety margin.
 */
// [spec:dash:def:shell.max-int-length-fn]
// [spec:dash:sem:shell.max-int-length-fn]
#[inline]
pub fn max_int_length(bytes: c_int) -> c_int {
    ((bytes * 8 - 1) as f64 * 0.30102999566398119521 + 14.0) as c_int
}

/// Flush the coverage profile before a `_exit`.
///
/// Only compiled for the instrumented build (`--cfg coverage`, set by
/// tests/harness/covrust.sh), where it is the difference between a
/// measurement and an empty directory.
///
/// dash never returns from `main`: every exit path ends in `_exit`, which
/// is faithful -- the C does the same, and it is what stops a forked
/// child running the parent's atexit handlers. But LLVM's coverage
/// runtime writes its profile FROM an atexit handler, so an instrumented
/// dash produced no profile at all while a trivial Rust program under the
/// identical sandbox produced one. The sandbox was innocent; `_exit` was
/// the whole story.
#[inline]
pub fn flush_coverage() {
    nsh_platform::flush_coverage_profile();
}

/// Zero the coverage counters in a freshly forked child.
///
/// Only compiled for the instrumented build, and it is what makes the
/// measurement arithmetically possible at all.
///
/// A fork copies the parent's counters. Without this the child then
/// merges the parent's counts into the shared profile a second time, its
/// own children merge them a third, and a shell forks per command, per
/// pipeline stage and per command substitution. The counts do not drift,
/// they compound: a full corpus run produced counters around 5e18 and
/// `llvm-profdata` refused every profile with "counter overflow".
///
/// Resetting here means a child reports only what the child ran. What it
/// executed before the fork is not lost -- the parent already counted it.
#[inline]
pub fn reset_coverage() {
    nsh_platform::reset_coverage_counters();
}
