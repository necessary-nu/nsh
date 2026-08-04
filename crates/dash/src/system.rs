//! Literal port of `src/system.c` / `src/system.h`.
//! Rules: `docs/spec/port/src/system.md`.
//!
//! The portability layer.  Almost everything here is conditional on a
//! `HAVE_*` macro from `configure`: on a system that provides the
//! function, none of it is compiled and the libc version is used.  Per
//! the port rules these are treated as *contracts the target must
//! satisfy*, so each one is defined unconditionally here and the C's
//! `#ifndef` guard is recorded in a comment.
//!
//! Note the `ctype` block at the top of `system.c`: on a platform
//! without `HAVE_ISALPHA` the header's macros are renamed to `_is*` on
//! include, so the file can then define real functions of the standard
//! names that call through to them.  Here `_is*` is spelled `libc::is*`.

use core::ptr::addr_of_mut;

use libc::{c_char, c_double, c_int, c_long, c_uint, c_void, size_t, ssize_t};

use crate::output::VaArg;
use crate::shell::{cstr, DEBUG};

/* `#ifndef SSIZE_MAX #define SSIZE_MAX ((ssize_t)((size_t)-1 >> 1))` */
pub const SSIZE_MAX: ssize_t = (usize::MAX >> 1) as ssize_t;

/*
 * `NSIG` is not re-exported by the libc crate; glibc's value is 65 and
 * it is only used to bound the `strsignal` table lookup.
 */
const NSIG: c_int = 65;

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

/* `#ifndef HAVE_MEMFD_CREATE` */
// [spec:dash:def:system.memfd-create-fn]
// [spec:dash:sem:system.memfd-create-fn]
#[inline]
pub unsafe fn memfd_create(_name: *const c_char, _flags: c_uint) -> c_int {
    -1
}

/* `#ifndef HAVE_MEMPCPY` */
// [spec:dash:def:system.mempcpy-fn]
// [spec:dash:sem:system.mempcpy-fn]
pub unsafe fn mempcpy(dest: *mut c_void, src: *const c_void, n: size_t) -> *mut c_void {
    (libc::memcpy(dest, src, n) as *mut u8).add(n) as *mut c_void
}

/* `#ifndef HAVE_STPCPY` */
// [spec:dash:def:system.stpcpy-fn]
// [spec:dash:sem:system.stpcpy-fn]
pub unsafe fn stpcpy(dest: *mut c_char, src: *const c_char) -> *mut c_char {
    let len: size_t = libc::strlen(src);
    *dest.add(len) = 0;
    mempcpy(dest as *mut c_void, src as *const c_void, len) as *mut c_char
}

/* `#ifndef HAVE_STRCHRNUL` */
// [spec:dash:def:system.strchrnul-fn]
// [spec:dash:sem:system.strchrnul-fn]
pub unsafe fn strchrnul(s: *const c_char, c: c_int) -> *mut c_char {
    let mut p: *mut c_char = libc::strchr(s, c);
    if p.is_null() {
        p = (s as *mut c_char).add(libc::strlen(s));
    }
    p
}

/* `#ifndef HAVE_STRSIGNAL` */
// [spec:dash:def:system.strsignal-fn]
// [spec:dash:sem:system.strsignal-fn]
pub unsafe fn strsignal(sig: c_int) -> *mut c_char {
    static mut buf: [c_char; 19] = [0; 19];

    /*
     * `sys_siglist` is not exported by the `libc` crate and glibc has
     * demoted it to a compat-only symbol, so libc `strsignal` stands in
     * for the table lookup `sys_siglist[sig]`.
     */
    if (sig as c_uint) < NSIG as c_uint {
        let p = libc::strsignal(sig);
        if !p.is_null() {
            return p;
        }
    }
    crate::output::fmtstr(
        addr_of_mut!(buf) as *mut c_char,
        core::mem::size_of::<[c_char; 19]>(),
        cstr(b"Signal %d\0"),
        &[VaArg::Int(sig)],
    );
    addr_of_mut!(buf) as *mut c_char
}

/* `#ifndef HAVE_STRTOD` */
// [spec:dash:def:system.strtod-fn]
// [spec:dash:sem:system.strtod-fn]
#[inline]
pub unsafe fn strtod(nptr: *const c_char, endptr: *mut *mut c_char) -> c_double {
    *endptr = nptr as *mut c_char;
    0.0
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
extern "C" {
    #[link_name = "__isoc23_strtoimax"]
    pub fn strtoimax(
        nptr: *const c_char,
        endptr: *mut *mut c_char,
        base: c_int,
    ) -> libc::intmax_t;
    #[link_name = "__isoc23_strtoumax"]
    pub fn strtoumax(
        nptr: *const c_char,
        endptr: *mut *mut c_char,
        base: c_int,
    ) -> libc::uintmax_t;
}

/* comparator type of `int (*cmp)(const void *, const void *)` */
pub type __compar_fn_t = unsafe extern "C" fn(*const c_void, *const c_void) -> c_int;

/* `#ifndef HAVE_BSEARCH` */
// [spec:dash:def:system.bsearch-fn]
// [spec:dash:sem:system.bsearch-fn]
pub unsafe fn bsearch(
    key: *const c_void,
    base: *const c_void,
    nmemb: size_t,
    size: size_t,
    cmp: __compar_fn_t,
) -> *mut c_void {
    let mut base = base;
    let mut nmemb = nmemb;

    while nmemb != 0 {
        let mididx: size_t = nmemb / 2;
        let midobj: *const c_void = (base as *const u8).add(mididx * size) as *const c_void;
        let diff: c_int = cmp(key, midobj);

        if diff == 0 {
            return midobj as *mut c_void;
        }

        if diff > 0 {
            base = (midobj as *const u8).add(size) as *const c_void;
            nmemb -= mididx + 1;
        } else {
            nmemb = mididx;
        }
    }

    core::ptr::null_mut()
}

/* `#ifndef HAVE_KILLPG` */
// [spec:dash:def:system.killpg-fn]
// [spec:dash:sem:system.killpg-fn]
#[inline]
pub unsafe fn killpg(pid: libc::pid_t, signal: c_int) -> c_int {
    if DEBUG {
        if pid < 0 {
            libc::abort();
        }
    }
    libc::kill(-pid, signal)
}

/* `#ifndef HAVE_SYSCONF #define _SC_CLK_TCK 2` */
pub const _SC_CLK_TCK: c_int = 2;

// [spec:dash:def:system.sysconf-fn]
// [spec:dash:sem:system.sysconf-fn]
pub unsafe fn sysconf(name: c_int) -> c_long {
    crate::error::sh_error(cstr(b"no sysconf for: %d\0"), &[VaArg::Int(name)]);
}

/* `#ifndef HAVE_TEE` */
// [spec:dash:def:system.tee-fn]
// [spec:dash:sem:system.tee-fn]
#[inline]
pub unsafe fn tee(_fd_in: c_int, _fd_out: c_int, _len: size_t, _flags: c_uint) -> ssize_t {
    -1
}

/* `#ifndef HAVE_FNMATCH` */
// [spec:dash:def:system.fnmatch-fn]
// [spec:dash:sem:system.fnmatch-fn]
#[inline]
pub unsafe fn fnmatch(_pattern: *const c_char, _string: *const c_char, _flags: c_int) -> c_int {
    -1
}

/* `#ifndef HAVE_GLOB` */
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
 * that single definition in `crate::bltin::printf`; this is the
 * equivalent of the header's prototype.
 */
pub type conv_escape_fn = unsafe fn(str_: *mut c_char, out: *mut c_char, mbchar: bool) -> c_uint;

/* ------------------------------------------------------------------ */
/* src/system.c — the `#ifndef HAVE_ISALPHA` ctype wrappers            */
/* ------------------------------------------------------------------ */

// [spec:dash:def:system.isalnum-fn]
// [spec:dash:sem:system.isalnum-fn]
pub unsafe fn isalnum(c: c_int) -> c_int {
    libc::isalnum(c)
}

// [spec:dash:def:system.iscntrl-fn]
// [spec:dash:sem:system.iscntrl-fn]
pub unsafe fn iscntrl(c: c_int) -> c_int {
    libc::iscntrl(c)
}

// [spec:dash:def:system.islower-fn]
// [spec:dash:sem:system.islower-fn]
pub unsafe fn islower(c: c_int) -> c_int {
    libc::islower(c)
}

// [spec:dash:def:system.isspace-fn]
// [spec:dash:sem:system.isspace-fn]
pub unsafe fn isspace(c: c_int) -> c_int {
    libc::isspace(c)
}

// [spec:dash:def:system.isalpha-fn]
// [spec:dash:sem:system.isalpha-fn]
pub unsafe fn isalpha(c: c_int) -> c_int {
    libc::isalpha(c)
}

// [spec:dash:def:system.isdigit-fn]
// [spec:dash:sem:system.isdigit-fn]
pub unsafe fn isdigit(c: c_int) -> c_int {
    libc::isdigit(c)
}

// [spec:dash:def:system.isprint-fn]
// [spec:dash:sem:system.isprint-fn]
pub unsafe fn isprint(c: c_int) -> c_int {
    libc::isprint(c)
}

// [spec:dash:def:system.isupper-fn]
// [spec:dash:sem:system.isupper-fn]
pub unsafe fn isupper(c: c_int) -> c_int {
    libc::isupper(c)
}

/*
 * Two variants exist in the C.  With HAVE_DECL_ISBLANK (and without
 * HAVE_ISALPHA) it is `return _isblank(c);`; without the declaration at
 * all it is `return c == ' ' || c == '\t';`.  The former is ported,
 * since it is what a hosted build gets.
 */
// [spec:dash:def:system.isblank-fn]
// [spec:dash:sem:system.isblank-fn]
pub unsafe fn isblank(c: c_int) -> c_int {
    libc::isblank(c)
}

// [spec:dash:def:system.isgraph-fn]
// [spec:dash:sem:system.isgraph-fn]
pub unsafe fn isgraph(c: c_int) -> c_int {
    libc::isgraph(c)
}

// [spec:dash:def:system.ispunct-fn]
// [spec:dash:sem:system.ispunct-fn]
pub unsafe fn ispunct(c: c_int) -> c_int {
    libc::ispunct(c)
}

// [spec:dash:def:system.isxdigit-fn]
// [spec:dash:sem:system.isxdigit-fn]
pub unsafe fn isxdigit(c: c_int) -> c_int {
    libc::isxdigit(c)
}

// ---------------------------------------------------------------------
// Unit tests for this module's functions.
//
// Most of this file is either a thin wrapper over libc or an
// `#ifndef HAVE_…` fallback that a hosted build never compiles. Both are
// still ported code with a stated contract, so both are asserted -- the
// fallbacks precisely because nothing else in the tree exercises them,
// which is how a wrong one would go unnoticed.
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

    // [spec:dash:sem:system.stpcpy-fn/test]
    #[test]
    fn stpcpy_copies_and_returns_the_nul() {
        unsafe {
            let mut dst = [0 as c_char; 16];
            let end = stpcpy(dst.as_mut_ptr(), CStr0::new("abc").p());
            assert_eq!(s(dst.as_ptr()), "abc");
            // Returns a pointer to the terminating NUL, not to dest.
            assert_eq!(end as usize, dst.as_ptr() as usize + 3);
            assert_eq!(*end, 0);
            let end2 = stpcpy(end, CStr0::new("de").p());
            assert_eq!(s(dst.as_ptr()), "abcde");
            assert_eq!(end2 as usize, dst.as_ptr() as usize + 5);
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

    // [spec:dash:sem:system.strsignal-fn/test]
    #[test]
    fn strsignal_names_known_signals_and_numbers_the_rest() {
        unsafe {
            assert!(s(strsignal(libc::SIGINT)).contains("Interrupt"));
            assert!(!s(strsignal(libc::SIGTERM)).is_empty());
            // Out of range falls through to the "Signal %d" buffer.
            assert_eq!(s(strsignal(NSIG as c_int + 5)), format!("Signal {}", NSIG as c_int + 5));
            // Negative is also out of range: the check is an unsigned
            // compare, so a negative signal number wraps above NSIG.
            assert_eq!(s(strsignal(-1)), "Signal -1");
        }
    }

    // [spec:dash:sem:system.bsearch-fn/test]
    #[test]
    fn bsearch_finds_members_and_reports_misses() {
        unsafe {
            let table: [c_int; 5] = [1, 3, 5, 7, 9];
            unsafe extern "C" fn cmp(a: *const c_void, b: *const c_void) -> c_int {
                *(a as *const c_int) - *(b as *const c_int)
            }
            let n = table.len();
            let sz = core::mem::size_of::<c_int>();
            for (i, want) in table.iter().enumerate() {
                let hit = bsearch(
                    want as *const c_int as *const c_void,
                    table.as_ptr() as *const c_void,
                    n as size_t,
                    sz,
                    cmp,
                );
                assert!(!hit.is_null());
                assert_eq!(hit as usize, table.as_ptr() as usize + i * sz);
            }
            let missing: c_int = 4;
            assert!(bsearch(
                &missing as *const c_int as *const c_void,
                table.as_ptr() as *const c_void,
                n as size_t,
                sz,
                cmp,
            )
            .is_null());
            // An empty table is a miss, not a crash.
            assert!(bsearch(
                &missing as *const c_int as *const c_void,
                table.as_ptr() as *const c_void,
                0,
                sz,
                cmp,
            )
            .is_null());
        }
    }

    // [spec:dash:sem:system.sysconf-fn/test]
    #[test]
    fn sysconf_fallback_always_raises() {
        let _g = crate::testutil::lock();
        // This is the `#ifndef HAVE_SYSCONF` fallback, and it does not
        // wrap libc: the C body is a bare
        // `sh_error("no sysconf for: %d", name)`, so it never returns a
        // value for any name at all -- including one the host supports.
        // A first draft asserted `sysconf(_SC_OPEN_MAX) > 0` and got an
        // unwind, which is the function behaving correctly.
        for name in [libc::_SC_OPEN_MAX, libc::_SC_CLK_TCK, _SC_CLK_TCK, -12345] {
            assert!(crate::testutil::raises(|| unsafe {
                sysconf(name);
            }));
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

    // [spec:dash:sem:system.killpg-fn/test]
    #[test]
    fn killpg_signals_a_process_group() {
        let _g = crate::testutil::lock();
        unsafe {
            // Signal 0 is the existence probe: no signal is sent, so this
            // asserts the call reaches the right group without disturbing
            // the test process.
            assert_eq!(killpg(libc::getpgrp(), 0), 0);
            // A group that cannot exist reports ESRCH.
            assert_eq!(killpg(0x7fff_fff0, 0), -1);
            assert_eq!(*libc::__errno_location(), libc::ESRCH);
        }
    }

    // The `#ifndef HAVE_…` fallbacks. A hosted build compiles none of
    // these -- redir.rs and input.rs call libc's memfd_create and tee
    // directly, and the shell uses its own pattern matcher rather than
    // fnmatch/glob. They are asserted anyway because a fallback nothing
    // exercises is exactly where a wrong constant survives.
    //
    // [spec:dash:sem:system.memfd-create-fn/test]
    // [spec:dash:sem:system.tee-fn/test]
    // [spec:dash:sem:system.fnmatch-fn/test]
    // [spec:dash:sem:system.glob64-fn/test]
    // [spec:dash:sem:system.globfree64-fn/test]
    // [spec:dash:sem:system.strtod-fn/test]
    #[test]
    fn unavailable_facility_fallbacks_report_failure() {
        unsafe {
            assert_eq!(memfd_create(CStr0::new("x").p(), 0), -1);
            assert_eq!(tee(0, 1, 16, 0), -1);
            assert_eq!(fnmatch(CStr0::new("*").p(), CStr0::new("a").p(), 0), -1);

            let mut g: glob64_t = core::mem::zeroed();
            assert_eq!(glob64(CStr0::new("*").p(), 0, None, &mut g), -1);
            // globfree64 is a no-op, so the pair is safe to call after a
            // failed glob -- which is what the callers do.
            globfree64(&mut g);

            // The strtod fallback parses nothing: it reports zero and
            // leaves endptr AT the start, which is how a caller detects
            // "no conversion performed".
            let nptr = CStr0::new("1.5");
            let mut end: *mut c_char = core::ptr::null_mut();
            assert_eq!(strtod(nptr.p(), &mut end), 0.0);
            assert_eq!(end as *const c_char, nptr.p());
        }
    }

    // The ctype wrappers. Asserted over the whole unsigned-char range
    // plus EOF rather than at a few sample points, because the thing that
    // goes wrong with these is a boundary, not a typical case.
    //
    // [spec:dash:sem:system.isalnum-fn/test]
    // [spec:dash:sem:system.iscntrl-fn/test]
    // [spec:dash:sem:system.islower-fn/test]
    // [spec:dash:sem:system.isspace-fn/test]
    // [spec:dash:sem:system.isalpha-fn/test]
    // [spec:dash:sem:system.isdigit-fn/test]
    // [spec:dash:sem:system.isprint-fn/test]
    // [spec:dash:sem:system.isupper-fn/test]
    // [spec:dash:sem:system.isblank-fn/test]
    // [spec:dash:sem:system.isgraph-fn/test]
    // [spec:dash:sem:system.ispunct-fn/test]
    // [spec:dash:sem:system.isxdigit-fn/test]
    #[test]
    fn ctype_wrappers_agree_with_the_c_locale_classification() {
        unsafe {
            for c in 0..=255i32 {
                let ch = c as u8 as char;
                assert_eq!(isalnum(c) != 0, ch.is_ascii_alphanumeric(), "isalnum {c}");
                assert_eq!(isalpha(c) != 0, ch.is_ascii_alphabetic(), "isalpha {c}");
                assert_eq!(isdigit(c) != 0, ch.is_ascii_digit(), "isdigit {c}");
                assert_eq!(isxdigit(c) != 0, ch.is_ascii_hexdigit(), "isxdigit {c}");
                assert_eq!(islower(c) != 0, ch.is_ascii_lowercase(), "islower {c}");
                assert_eq!(isupper(c) != 0, ch.is_ascii_uppercase(), "isupper {c}");
                assert_eq!(iscntrl(c) != 0, ch.is_ascii_control(), "iscntrl {c}");
                assert_eq!(ispunct(c) != 0, ch.is_ascii_punctuation(), "ispunct {c}");
                assert_eq!(isgraph(c) != 0, ch.is_ascii_graphic(), "isgraph {c}");
                assert_eq!(isprint(c) != 0, ch.is_ascii() && !ch.is_ascii_control(), "isprint {c}");
                assert_eq!(isspace(c) != 0, matches!(c as u8, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'), "isspace {c}");
                assert_eq!(isblank(c) != 0, matches!(c as u8, b' ' | b'\t'), "isblank {c}");
            }
            // EOF must classify as nothing at all.
            for f in [isalnum, isalpha, isdigit, isxdigit, islower, isupper,
                      iscntrl, ispunct, isgraph, isprint, isspace, isblank] {
                assert_eq!(f(-1), 0);
            }
        }
    }
}
