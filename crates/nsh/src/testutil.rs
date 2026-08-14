//! Shared scaffolding for the crate's unit tests. `#[cfg(test)]` only.
//!
//! Two facts about this codebase shape every test in the crate.
//!
//! **The shell's state is process-global.** It is a literal port of C
//! that keeps its variables, its stack allocator, its open files and its
//! exception handler in statics. Cargo runs tests on multiple threads in
//! one process, so any test that touches that state has to hold
//! [`lock`] for its duration. Tests that only call pure functions do not
//! need it.
//!
//! **Errors are values.** A fallible function returns
//! `Result<_, error::Error>` and a test asserts on the returned error --
//! its `message()` and its `status()` -- which pins what the shell said
//! rather than how it left. There was a `raises` helper here for the
//! functions that were still `-> !`, which armed a handler and reported
//! whether the body jumped; it went with the machinery when
//! `errors-are-values` finished, along with the last `-> !`.

use std::sync::{Mutex, MutexGuard, OnceLock};

use core::ptr::addr_of_mut;
use libc::{c_char, c_int};

/// Serialises tests that touch shell globals.
///
/// Returns the guard rather than taking a closure so a test can hold it
/// across several statements. Poisoning is ignored: a panic inside one
/// test would otherwise cascade into unrelated failures, and every test
/// here re-establishes the state it depends on.
pub fn lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    match LOCK.get_or_init(|| Mutex::new(())).lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// A NUL-terminated C string that lives as long as the test.
///
/// `CString::as_ptr()` on a temporary dangles at the end of the
/// statement, which is a real hazard when every function under test
/// takes `*const c_char`.
pub struct CStr0(std::ffi::CString);

impl CStr0 {
    pub fn new(s: &str) -> Self {
        CStr0(std::ffi::CString::new(s).expect("test string contains NUL"))
    }
    pub fn p(&self) -> *const c_char {
        self.0.as_ptr()
    }
}

/// Borrow a `*const c_char` as a Rust `&str` for comparison.
///
/// # Safety
/// `p` must be non-NULL and NUL-terminated.
pub unsafe fn s(p: *const c_char) -> String {
    std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
}

/// Run `body` in a forked child and return the child's exit status.
///
/// Some ported functions end in `exit()` -- every `main`, and
/// `trap::exitshell` -- so calling them in-process would take the test
/// runner with them. Forking is the only way to observe what they do and
/// still have a test report afterwards. Anything the child writes to a
/// file is visible to the parent, which is how the generator tests check
/// their output.
///
/// Takes [`lock`] internally: forking a process whose other threads may
/// hold locks is only safe if nothing else is running, and cargo runs
/// tests on several threads by default.
pub fn forked(body: impl FnOnce()) -> c_int {
    let _g = lock();
    unsafe {
        let pid = libc::fork();
        assert!(pid >= 0, "fork failed");
        if pid == 0 {
            body();
            // Only reached if `body` did NOT exit on its own.
            libc::_exit(0);
        }
        let mut status: c_int = 0;
        assert_eq!(libc::waitpid(pid, &mut status, 0), pid);
        if libc::WIFEXITED(status) {
            libc::WEXITSTATUS(status)
        } else {
            // Encode a signal death as 128+n, the shell's own convention.
            128 + libc::WTERMSIG(status)
        }
    }
}
