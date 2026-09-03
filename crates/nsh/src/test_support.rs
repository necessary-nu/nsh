//! Shared scaffolding for the crate's unit tests. `#[cfg(test)]` only.
//!
//! Two facts about this codebase shape every test in the crate.
//!
//! **A few operating-system properties are process-global.** Shell-owned
//! variables, files, and control state live on each `Shell`, but locale,
//! signal dispositions, the current directory, the file-creation mask and
//! inherited descriptors still belong to the hosting process. Cargo runs
//! tests on multiple threads, so a test that changes one of those
//! properties holds [`lock`] for its duration -- and so does a test that
//! only *observes* one. Observing is not the weaker case:
//! `builtins::umask`'s tests drive the mask to 0o777 and restore it, all
//! under the lock and exactly as this asks, while
//! `editor::completion`'s fixture merely creates a temporary directory
//! and three entries in it, and got them mode 000 and an `EACCES` in 201
//! runs out of 2,000. Nor is there a per-call way out: the kernel
//! applies `mode & ~umask` to every
//! `mkdir` and `open`, so an explicit mode is masked too, and only
//! `chmod` on an existing file escapes. Tests confined to one shell
//! instance do not need the lock. `docs/api-design.md` §6 is the list.
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
