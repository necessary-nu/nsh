//! Literal port of `src/system.c` / `src/system.h`.
//! Rules: `docs/spec/port/src/system.md`.
//!
//! The portability layer.  Almost everything here is conditional on a
//! `HAVE_*` macro from `configure`: on a system that provides the
//! function, none of it is compiled and the libc version is used.
//!
//! What survives is what the shell calls.  Twenty exported items had no
//! caller in the crate — the twelve `ctype` wrappers, `stpcpy`,
//! `strsignal`, `strtod`, `killpg`, `sysconf`, `tee`, `memfd_create` and
//! `fnmatch` — because every caller goes to `libc::` directly.  For all
//! but `fnmatch` the reference build's `config.h` defines the matching
//! `HAVE_*`, so the C compiles none of them either; `HAVE_FNMATCH` is the
//! one that is undefined, but its only call site is behind
//! `if (FNMATCH_IS_ENABLED)`, which is 0.  They are gone, and their rules
//! are retired in `docs/spec/port/src/system.md`.
//!
//! `glob64` is the other stub the C really compiles (`HAVE_GLOB` is
//! undefined); `expand.rs` still reaches it behind `GLOB_IS_ENABLED`, so
//! it stays.
//!
//! `bsearch` and its comparator type are gone as well. The `#ifndef
//! HAVE_BSEARCH` arm is a claim about the target, and `core` answers it
//! unconditionally: `<[T]>::binary_search_by` exists on every target this
//! crate builds for, so there is no configuration in which a fallback is
//! reachable. Its one caller was `mystring.c`'s `findstring`, now
//! `parser::findkwd`. `strtoumax` went with it — `bltin/printf.rs`
//! declares its own binding and nothing else asked for one — as did the
//! `conv_escape` prototype alias, which named a type no signature used.

use bstr::ByteSlice;
use libc::{c_char, c_int, c_void, size_t, ssize_t};

/* `#ifndef SSIZE_MAX #define SSIZE_MAX ((ssize_t)((size_t)-1 >> 1))`.
 * `ssize_t::MAX` is the same value; the two readers are in `output.rs`,
 * whose conversion is a separate task. */
pub const SSIZE_MAX: ssize_t = ssize_t::MAX;

/* std has no signal mask at all — `sigprocmask` is the only spelling —
 * so this is not a shim over something better, it is the operation. */
// [spec:dash:def:system.sigclearmask-fn]
// [spec:dash:sem:system.sigclearmask-fn]
#[inline]
pub unsafe fn sigclearmask() {
    /*
     * The HAVE_SIGSETMASK arm is `sigsetmask(0)` (with the glibc
     * deprecation warning suppressed); the `libc` crate does not export
     * the BSD spelling, so the portable arm is what is ported.  Both
     * unblock every signal.
     */
    let mut set: libc::sigset_t = core::mem::zeroed();
    libc::sigemptyset(&mut set);
    libc::sigprocmask(libc::SIG_SETMASK, &set, core::ptr::null_mut());
}

/* `#ifndef HAVE_MEMPCPY`.  `copy_nonoverlapping` plus the length is the
 * whole of it: the callers are raw-pointer cursors, so the signature
 * stays pointer-shaped, but nothing here needs libc to move bytes. */
// [spec:dash:def:system.mempcpy-fn]
// [spec:dash:sem:system.mempcpy-fn]
pub unsafe fn mempcpy(dest: *mut c_void, src: *const c_void, n: size_t) -> *mut c_void {
    let dest = dest as *mut u8;
    core::ptr::copy_nonoverlapping(src as *const u8, dest, n);
    dest.add(n) as *mut c_void
}

/* `#ifndef HAVE_STRCHRNUL`.  `find_byte(c).unwrap_or(len)` is exactly
 * this: the miss returns the NUL rather than NULL, which is the only
 * reason callers prefer it to `strchr`.  Searching for 0 finds the
 * terminator either way, because `to_bytes` stops there and the miss
 * lands on it. */
// [spec:dash:def:system.strchrnul-fn]
// [spec:dash:sem:system.strchrnul-fn]
pub unsafe fn strchrnul(s: *const c_char, c: c_int) -> *mut c_char {
    let bytes = core::ffi::CStr::from_ptr(s).to_bytes();
    let off = bytes.find_byte(c as u8).unwrap_or(bytes.len());
    (s as *mut c_char).add(off)
}

/*
 * `#ifndef HAVE_STRTOIMAX #define strtoimax strtoll #endif` and the
 * matching `strtoumax`.  The `libc` crate does not declare either, so
 * they are declared here; on every target dash builds for the real
 * functions exist.
 */
/*
 * glibc >= 2.38 redirects `strtoimax`/`strtoumax` through `__isoc23_*`
 * for any translation unit with C23 strtol semantics enabled, which is
 * every unit in the dash build (`features.h` defaults
 * `__GLIBC_USE_C2X_STRTOL` to 1). Those variants also accept `0b`/`0B`
 * binary constants when the base is 0 or 2. `nm -D` on the reference
 * binary shows `__isoc23_strtoimax@GLIBC_2.38`, so the C really does
 * accept them — `$((0b11))` is 3 — and binding the plain symbol here
 * silently loses binary literals.
 */
unsafe extern "C" {
    #[link_name = "__isoc23_strtoimax"]
    pub fn strtoimax(
        nptr: *const c_char,
        endptr: *mut *mut c_char,
        base: c_int,
    ) -> libc::intmax_t;
}

/* `#ifndef HAVE_GLOB`.  `expand.rs`'s `expandmeta_glob` reaches these
 * behind `GLOB_IS_ENABLED`, which is 0; the `glob` crate is not a
 * replacement for either arm — `docs/std-replacements.md` §5.4. */
pub const GLOB_ERR: c_int = 1 << 0; /* Return on read errors.  */
pub const GLOB_MARK: c_int = 1 << 1; /* Append a slash to each name.  */
pub const GLOB_NOSORT: c_int = 1 << 2; /* Don't sort the names.  */
pub const GLOB_DOOFFS: c_int = 1 << 3; /* Insert PGLOB->gl_offs NULLs.  */
pub const GLOB_NOCHECK: c_int = 1 << 4; /* If nothing matches, return the pattern.  */
pub const GLOB_APPEND: c_int = 1 << 5; /* Append to results of a previous call.  */
pub const GLOB_NOESCAPE: c_int = 1 << 6; /* Backslashes don't quote metacharacters.  */
pub const GLOB_PERIOD: c_int = 1 << 7; /* Leading `.' can be matched by metachars.  */
pub const GLOB_MAGCHAR: c_int = 1 << 8; /* Set in gl_flags if any metachars seen.  */
pub const GLOB_ALTDIRFUNC: c_int = 1 << 9; /* Use gl_opendir et al functions.  */
pub const GLOB_BRACE: c_int = 1 << 10; /* Expand "{a,b}" to "a" "b".  */
pub const GLOB_NOMAGIC: c_int = 1 << 11; /* If no magic chars, return the pattern.  */
pub const GLOB_TILDE: c_int = 1 << 12; /* Expand ~user and ~ to home directories. */
pub const GLOB_ONLYDIR: c_int = 1 << 13; /* Match only directories.  */
pub const GLOB_TILDE_CHECK: c_int = 1 << 14; /* Like GLOB_TILDE but return an error
                                             if the user name is not available.  */

pub const GLOB_NOSPACE: c_int = 1; /* Ran out of memory.  */
pub const GLOB_ABORTED: c_int = 2; /* Read error.  */
pub const GLOB_NOMATCH: c_int = 3; /* No matches found.  */
pub const GLOB_NOSYS: c_int = 4; /* Not implemented.  */

/* `struct dirent64;` / `struct stat64;` — opaque forward declarations. */
pub enum dirent64 {}
pub enum stat64 {}

// [spec:dash:def:system.glob64-t]
#[repr(C)]
pub struct glob64_t {
    pub gl_pathc: size_t,
    pub gl_pathv: *mut *mut c_char,
    pub gl_offs: size_t,
    pub gl_flags: c_int,

    // [spec:dash:def:system.gl-closedir-fn]
    // [spec:dash:sem:system.gl-closedir-fn]
    pub gl_closedir: Option<unsafe extern "C" fn(*mut c_void)>,
    // [spec:dash:def:system.gl-readdir-fn]
    // [spec:dash:sem:system.gl-readdir-fn]
    pub gl_readdir: Option<unsafe extern "C" fn(*mut c_void) -> *mut dirent64>,
    // [spec:dash:def:system.gl-opendir-fn]
    // [spec:dash:sem:system.gl-opendir-fn]
    pub gl_opendir: Option<unsafe extern "C" fn(*const c_char) -> *mut c_void>,
    // [spec:dash:def:system.gl-lstat-fn]
    // [spec:dash:sem:system.gl-lstat-fn]
    pub gl_lstat: Option<unsafe extern "C" fn(*const c_char, *mut stat64) -> c_int>,
    // [spec:dash:def:system.gl-stat-fn]
    // [spec:dash:sem:system.gl-stat-fn]
    pub gl_stat: Option<unsafe extern "C" fn(*const c_char, *mut stat64) -> c_int>,
}

// [spec:dash:def:system.glob64-fn]
// [spec:dash:sem:system.glob64-fn]
#[inline]
pub unsafe fn glob64(
    _pattern: *const c_char,
    _flags: c_int,
    _errfunc: Option<unsafe extern "C" fn(*const c_char, c_int) -> c_int>,
    _pglob: *mut glob64_t,
) -> c_int {
    -1
}

// [spec:dash:def:system.globfree64-fn]
// [spec:dash:sem:system.globfree64-fn]
#[inline]
pub unsafe fn globfree64(_pglob: *mut glob64_t) {}

/*
 * `#define uninitialized_var(x) x = x` — a trick to suppress an
 * uninitialized variable warning without generating any code.  It has no
 * Rust analogue and no port rule.
 */

/*
 * `unsigned conv_escape(char *str, char *out, bool mbchar);`
 *
 * Declared in system.h but *defined* in `src/bltin/printf.c`, so this
 * header symbol has no body of its own.  The port carries both the
 * `system.conv-escape-fn` and `printf.conv-escape-fn` annotations on
 * that single definition in `crate::bltin::printf`; Rust has no place
 * for a second declaration of it.
 */

// ---------------------------------------------------------------------
// Unit tests for this module's functions.
//
// What is left is either a thin wrapper over libc or an `#ifndef HAVE_…`
// fallback that a hosted build never compiles. Both are still ported code
// with a stated contract, so both are asserted -- the fallbacks precisely
// because nothing else in the tree exercises them, which is how a wrong
// one would go unnoticed.
// ---------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{s, CStr0};

    // [spec:dash:sem:system.mempcpy-fn/test]
    #[test]
    fn mempcpy_copies_and_returns_the_end() {
        unsafe {
            let src = b"hello world";
            let mut dst = [0u8; 16];
            let end = mempcpy(
                dst.as_mut_ptr() as *mut c_void,
                src.as_ptr() as *const c_void,
                5,
            );
            assert_eq!(&dst[..5], b"hello");
            // The point of mempcpy over memcpy: it returns dest+n, so
            // appends chain without a second strlen.
            assert_eq!(end as usize, dst.as_ptr() as usize + 5);
            let end2 = mempcpy(end, b"!".as_ptr() as *const c_void, 1);
            assert_eq!(&dst[..6], b"hello!");
            assert_eq!(end2 as usize, dst.as_ptr() as usize + 6);
            // A zero-length copy still reports the (unmoved) end.
            assert_eq!(
                mempcpy(dst.as_mut_ptr() as *mut c_void, src.as_ptr() as *const c_void, 0) as usize,
                dst.as_ptr() as usize
            );
        }
    }

    // [spec:dash:sem:system.strchrnul-fn/test]
    #[test]
    fn strchrnul_falls_back_to_the_nul() {
        unsafe {
            let hay = CStr0::new("abcdef");
            assert_eq!(s(strchrnul(hay.p(), 'c' as c_int)), "cdef");
            assert_eq!(s(strchrnul(hay.p(), 'a' as c_int)), "abcdef");
            // The whole reason this exists rather than strchr: a miss
            // returns the NUL, never NULL, so callers need no branch.
            let miss = strchrnul(hay.p(), 'z' as c_int);
            assert!(!miss.is_null());
            assert_eq!(*miss, 0);
            assert_eq!(miss as usize, hay.p() as usize + 6);
            // Searching for NUL finds the terminator.
            assert_eq!(strchrnul(hay.p(), 0) as usize, hay.p() as usize + 6);
        }
    }

    // [spec:dash:sem:system.sigclearmask-fn/test]
    #[test]
    fn sigclearmask_unblocks_everything() {
        let _g = crate::testutil::lock();
        unsafe {
            let mut blocked: libc::sigset_t = core::mem::zeroed();
            libc::sigemptyset(&mut blocked);
            libc::sigaddset(&mut blocked, libc::SIGUSR1);
            let mut saved: libc::sigset_t = core::mem::zeroed();
            libc::sigprocmask(libc::SIG_BLOCK, &blocked, &mut saved);
            let mut now: libc::sigset_t = core::mem::zeroed();
            libc::sigprocmask(libc::SIG_SETMASK, core::ptr::null(), &mut now);
            assert_eq!(libc::sigismember(&now, libc::SIGUSR1), 1);

            sigclearmask();

            libc::sigprocmask(libc::SIG_SETMASK, core::ptr::null(), &mut now);
            assert_eq!(libc::sigismember(&now, libc::SIGUSR1), 0);
            libc::sigprocmask(libc::SIG_SETMASK, &saved, core::ptr::null_mut());
        }
    }

    // The `#ifndef HAVE_GLOB` fallback. `HAVE_GLOB` really is undefined
    // in the reference build, so the C compiles this pair, and `expand.c`
    // reaches it behind `GLOB_IS_ENABLED`, which is 0. It is asserted
    // anyway because a fallback nothing exercises is exactly where a
    // wrong constant survives.
    //
    // [spec:dash:sem:system.glob64-fn/test]
    // [spec:dash:sem:system.globfree64-fn/test]
    #[test]
    fn the_glob_fallback_reports_failure() {
        unsafe {
            let mut g: glob64_t = core::mem::zeroed();
            assert_eq!(glob64(CStr0::new("*").p(), 0, None, &mut g), -1);
            // globfree64 is a no-op, so the pair is safe to call after a
            // failed glob -- which is what the callers do.
            globfree64(&mut g);
        }
    }
}
