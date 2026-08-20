//! Shared scaffolding for the crate's unit tests. `#[cfg(test)]` only.
//!
//! Two facts about this codebase shape every test in the crate.
//!
//! **A few operating-system properties are process-global.** Shell-owned
//! variables, files, and control state live on each `Shell`, but locale,
//! signal dispositions, the current directory, and inherited descriptors
//! still belong to the hosting process. Cargo runs tests on multiple threads,
//! so a test that changes one of those properties holds [`lock`] for its
//! duration. Tests confined to one shell instance do not need it.
//!
//! **Errors are values.** A fallible function returns
//! `Result<_, error::Error>` and a test asserts on the returned error --
//! its `message()` and its `status()` -- which pins what the shell said
//! rather than how it left. There was a `raises` helper here for the
//! functions that were still `-> !`, which armed a handler and reported
//! whether the body jumped; it went with the machinery when
//! `errors-are-values` finished, along with the last `-> !`.

use std::sync::{Mutex, MutexGuard, OnceLock};

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
pub fn forked(body: impl FnOnce()) -> i32 {
    let _g = lock();
    nsh_platform::run_in_child(body).expect("forked test process failed")
}
