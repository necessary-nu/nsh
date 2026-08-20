//! Literal port of `src/shell.c` / `src/shell.h`.
//! Rules: `docs/spec/port/src/shell.md`.
//!
//! There is no `src/shell.c`; `shell.h` is the umbrella header holding
//! build-wide typedefs and configuration.  The preprocessor knobs
//! (`JOBS`, `BSD`, `DO_SHAREDVFORK`, `STATIC`, `TRACE`,
//! `TRACEV`) have no runtime meaning here and are recorded as plain
//! constants or comments.

use core::ffi::{c_int, c_void};

/* JOBS -> 1 if you have Berkeley job control, 0 otherwise. */
pub const JOBS: c_int = 1;
/* define BSD if you are running 4.2 BSD or later. */
pub const BSD: c_int = 1;

/*
 * define DEBUG=1 to compile in debugging ('set -o debug' to turn on)
 * define DEBUG=2 to compile in and turn on debugging.
 *
 * Not defined in the shipped build.  Ports of `#ifdef DEBUG` blocks test
 * this constant so the code still type-checks while remaining dead.
 */
pub const DEBUG: bool = false;

// [spec:dash:def:shell.pointer]
pub type pointer = *mut c_void;

/*
 * `extern char nullstr[1];` — the null string is defined in
 * `mystring.c`, so it lives in `crate::mystring`.
 */

/*
 * `likely`/`unlikely` are branch hints only; they have no effect on
 * behaviour and Rust has no stable equivalent, so they are the identity.
 */
#[inline(always)]
pub fn likely(x: bool) -> bool {
    x
}

#[inline(always)]
pub fn unlikely(x: bool) -> bool {
    x
}

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

/*
 * The other direction had a helper here -- take a `char *` the shell is
 * about to stop owning and keep its bytes, terminator included.  It was
 * `strlen` plus a `from_raw_parts`, which is one `CStr::to_bytes_with_nul`
 * and no longer worth a name; its two callers in `expand.rs` spell it and
 * say why the terminator travels.
 */

/// A word a builtin was handed, as the C string an interface that has not
/// been converted yet still wants.
///
/// The bytes stop at the first NUL, which is where a `char *` reader would
/// have stopped anyway -- so this cannot fail, and it says out loud that
/// the truncation belongs to the C interface rather than to the word.
///
/// Every call is a place the byte string has to become a C string again,
/// which makes the remaining ones countable. Not part of shell.h.
#[inline]
pub fn cstring(arg: &bstr::BStr) -> std::ffi::CString {
    let bytes: &[u8] = arg.as_ref();
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    std::ffi::CString::new(&bytes[..end]).expect("the bytes stop at the first NUL")
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
