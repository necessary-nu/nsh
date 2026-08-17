//! nsh — a POSIX shell as a library.
//!
//! It began as a port of dash 0.5.13.5, whose observable behavior remains
//! the reference. The implementation now uses owned Rust data, explicit
//! state and typed control flow rather than preserving C representations.
//! Items carry `[spec:dash:def:…]` / `[spec:dash:sem:…]` annotations for
//! the rules they implement; corresponding C-source claims live in
//! `plan/annotations.styx`.
//!
//! **The surface is closed.** It was thirty-eight public modules, which is
//! not an API but the transliteration left open — see
//! [dec:nsh:public-surface]. What an embedder writes is now the handful of
//! names re-exported below: [`Shell`] and its [`Builder`], [`Source`],
//! [`Streams`], [`ExitStatus`] and [`Signal`], [`Error`], and the [`Host`]
//! seam with [`Disposition`], [`NoHost`] and [`ProcessHost`].
//! `crates/nsh/examples/embed.rs` is written against exactly that set and
//! is run, not merely compiled.
//!
//! Two modules stay public because they have callers outside the crate and
//! nothing smaller would serve them: [`shellmain`], whose `main_fn` is the
//! port of `main()` and is what the frontend and integration tests invoke,
//! and [`streams`], which owns the shell's initial logical standard streams.
//! The other thirty-six are `pub(crate)`.
//!
//! `#![deny(missing_docs)]` is on, which is the point of closing rather
//! than a tidiness measure: [dec:nsh:public-surface] asked for the surface
//! property to be *measured* under that lint, and a surface of thirty-eight
//! transliterated modules could never have carried it. Closing took it from
//! unaffordable to sixteen items.
//!
//!
//! ## Three things an embedder has to know, which are not types
//!
//! `docs/api-design.md` §6 and §11 carry these; they are here because a
//! reader of the crate should not have to find the document first.
//!
//! **The shell reaps any child of the process.** `wait3(status, flags,
//! NULL)` is `waitpid(-1)`, so a shell running in your process will reap
//! a `std::process::Child` you were holding, and your `wait()` gets
//! `ECHILD` for a status now sitting in a job table you cannot see. This
//! is not fixable by tracking pids: reaping is destructive, the only
//! peek-without-reap primitive returns the same foreign child forever and
//! turns the blocking wait into a spin, and dispatching properly would
//! need the shell to own `SIGCHLD` for the whole process, which
//! [dec:nsh:host-owns-signals] forbids. Do not run other children
//! concurrently with a shell you are driving.
//!
//! **`fork` from a multithreaded host carries only the calling thread**,
//! and the library's children allocate before they `exec` -- or never
//! `exec` at all, a subshell being a shell. Same caveat as
//! `Command::pre_exec`, same cause, and not removable.
//!
//! **[`Shell`]'s `Drop` neither waits nor kills, and that is a promise
//! rather than an omission.** A foreground job is waited to completion
//! before [`Shell::run`] returns; a background job is not, so
//! `sh.run(b"sleep 10 &")` returns with the child alive, exactly as
//! `dash -c 'sleep 10 &'` does. A `Drop` that waited would block the host
//! on a script's `&`; one that killed would exceed every grant on
//! [`Host`]. Neither is a tidiness fix to add later.
//!
//! The command-line shell lives in `crates/nsh-cli` and links this crate
//! as an external dependency, so anything the frontend needs that is not
//! `pub` here is a compile error rather than something a reader has to
//! notice.

#![deny(missing_docs)]
#![deny(unsafe_code)]
#![allow(dead_code)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(unused_variables)]
#![allow(clippy::all)]

// ---- the public API --------------------------------------------------
//
// `api.rs` stood here: a type-checked sketch of `docs/api-design.md`
// whose every body was `todo!()`, behind a feature nothing enabled,
// because a surface an embedder can call and that panics on every call
// would have contradicted [dec:nsh:public-surface] rather than served it.
// It did the job it was built for -- the borrow shapes, `Host`'s object
// safety, and whether a built-in that re-enters evaluation can be written
// at all are questions a document cannot answer and a compiler can -- and
// it is gone, because all of it is real now. Its module doc said the
// re-exports would move here when the list was empty. This is that.
//
// The names below are what an embedder writes. Each is implemented in the
// module that owns the concept, and named here so that reaching one does
// not mean knowing which module that is.
pub use crate::builder::Builder;
pub use crate::context::Shell;
pub use crate::error::Error;
pub use crate::host::{Disposition, Host, NoHost, ProcessHost, SignalSink};
pub use crate::source::Source;
pub use crate::status::{ExitStatus, Signal};
pub use crate::streams::Streams;

// ---- the shell instance ----------------------------------------------
//
// The receiver every function that touches shell state is being given,
// ahead of the state itself moving onto it. [dec:nsh:no-ambient-state].
pub(crate) mod builder;
pub(crate) mod host;
pub(crate) mod context;
pub(crate) mod source;

// ---- foundation -----------------------------------------------------
pub(crate) mod error;
pub(crate) mod escape;
pub(crate) mod fd;
pub(crate) mod mystring;
pub(crate) mod output;
pub(crate) mod shell;
pub(crate) mod siginbox;
pub(crate) mod status;
pub mod streams;
pub(crate) mod system;

// ---- unit-test scaffolding (test builds only) ------------------------
#[cfg(test)]
pub(crate) mod testutil;

// ---- the builtins ----------------------------------------------------
pub(crate) mod builtins;

// ---- generated tables ------------------------------------------------
pub(crate) mod nodes;
pub(crate) mod signames;
pub(crate) mod syntax;

// ---- shell state -----------------------------------------------------
pub(crate) mod alias;
pub(crate) mod cd;
pub(crate) mod init;
pub(crate) mod input;
pub(crate) mod mail;
pub(crate) mod options;
pub(crate) mod redir;
pub(crate) mod trap;
pub(crate) mod var;

// ---- parsing and expansion -------------------------------------------
pub(crate) mod arith_yacc;
pub(crate) mod expand;
pub(crate) mod parser;
pub(crate) mod pmatch;

// ---- execution --------------------------------------------------------
pub(crate) mod eval;
pub(crate) mod exec;
pub(crate) mod jobs;
pub mod shellmain;

// ---- builtins ---------------------------------------------------------
pub(crate) mod histedit;
pub(crate) mod linedit;
