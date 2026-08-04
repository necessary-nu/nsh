//! Literal port of `src/shell.c` / `src/shell.h`.
//! Rules: `docs/spec/port/src/shell.md`.
//!
//! There is no `src/shell.c`; `shell.h` is the umbrella header holding
//! build-wide typedefs and configuration.  The preprocessor knobs
//! (`JOBS`, `BSD`, `DO_SHAREDVFORK`, `STATIC`, `MKINIT`, `TRACE`,
//! `TRACEV`) have no runtime meaning here and are recorded as plain
//! constants or comments.

use libc::{c_char, c_int, c_void};

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
pub use crate::mystring::nullstr;

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
 * Helper used throughout the port to spell a C string literal: `b"…\0"`
 * byte strings stand in for C's `"…"`, and this turns one into the
 * `const char *` the ported signatures expect.  Not part of shell.h.
 */
#[inline(always)]
pub fn cstr(s: &'static [u8]) -> *const c_char {
    s.as_ptr() as *const c_char
}
