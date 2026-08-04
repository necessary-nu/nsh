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
//! **Errors are unwinds.** `sh_error` and friends end in
//! `error::exraise`, which raises the `Longjmp` payload that
//! `eval::setjmp_catch` catches. A test that wants to assert an error
//! path must arm a handler with [`raises`] rather than expect a return
//! value -- these functions are `-> !`.

use std::sync::{Mutex, MutexGuard, OnceLock};

use core::ptr::addr_of_mut;
use libc::c_char;

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

/// Run `body` with an exception handler armed, and report whether it
/// raised.
///
/// This is the test-side counterpart of the shell's own
/// `setjmp_catch`/`exraise` pair: the C would `longjmp` to the nearest
/// handler, and the port unwinds to the nearest `catch_unwind`. Returns
/// `true` when the body raised, `false` when it ran to completion.
pub fn raises<F: FnOnce()>(body: F) -> bool {
    unsafe {
        let mut loc: crate::error::jmploc = crate::error::jmploc::new();
        let saved = crate::error::handler;
        let result = crate::eval::setjmp_catch(addr_of_mut!(loc), || {
            crate::error::handler = addr_of_mut!(loc);
            body();
        });
        crate::error::handler = saved;
        result != 0
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
