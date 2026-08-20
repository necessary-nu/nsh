//! nsh — a POSIX shell as a library.
//!
//! It began as a port of dash 0.5.13.5, which remains a differential
//! regression oracle behind POSIX and explicit nsh behavior. The implementation
//! uses owned Rust data, explicit state and typed control flow rather than
//! preserving C representations or defects.
//! Items carry `[spec:dash:sem:…]` annotations for inherited behavior that
//! remains relevant. Historical C signatures stay in `docs/spec/port/` as
//! provenance, not as structural contracts on the Rust implementation.
//! [spec:nsh:sem:idiom.specified-defects+1]
//!
//! **The surface is closed.** It was thirty-eight public modules, which is
//! not an API but the transliteration left open — see
//! [dec:nsh:public-surface]. What an embedder writes is now the handful of
//! names re-exported below: [`Shell`] and its [`Builder`], [`Source`] and
//! [`Startup`],
//! [`Streams`], [`ExitStatus`] and [`Signal`], [`Error`], and the [`Host`]
//! seam with [`Disposition`], [`NoHost`] and [`ProcessHost`].
//! `crates/nsh/examples/embed.rs` is written against exactly that set and
//! is run, not merely compiled.
//!
//! [`streams`] stays public because it owns the shell's initial logical
//! standard streams. Startup itself is reached through
//! [`Shell::run_to_completion`]; no translated `main` module is part of the
//! library surface.
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
// [spec:nsh:req:idiom.strict-lints]
// [spec:nsh:req:idiom.regression-gates]
#![deny(unsafe_code)]
#![deny(dead_code)]
#![deny(non_camel_case_types)]
#![deny(non_snake_case)]
#![deny(non_upper_case_globals)]
#![deny(unused_variables)]
#![deny(unused_must_use)]
#![deny(clippy::correctness)]

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
// The crate, rather than a translated C module graph, is the target artifact.
// [spec:nsh:req:idiom.port-provenance+1]
pub use crate::builder::Builder;
pub use crate::context::Shell;
pub use crate::error::Error;
pub use crate::host::{Disposition, Host, NoHost, ProcessHost, SignalSink};
pub use crate::options::ShellOption;
pub use crate::source::{Source, Startup};
pub use crate::status::{ExitStatus, Signal};
pub use crate::streams::Streams;

// ---- the shell instance ----------------------------------------------
//
// The receiver every function that touches shell state is being given,
// ahead of the state itself moving onto it. [dec:nsh:no-ambient-state].
// The module graph contains only live Rust implementation, with source-port
// configuration and declaration scaffolding rejected by a repository test.
// [spec:nsh:req:idiom.no-port-fossils]
pub(crate) mod builder;
pub(crate) mod context;
pub(crate) mod host;
pub(crate) mod source;

// ---- foundation -----------------------------------------------------
// [spec:nsh:req:idiom.rust-naming]
pub(crate) mod descriptors;
pub(crate) mod error;
pub(crate) mod escape;
pub(crate) mod number;
pub(crate) mod output;
pub(crate) mod signal_inbox;
pub(crate) mod status;
pub mod streams;

// ---- unit-test scaffolding (test builds only) ------------------------
#[cfg(test)]
pub(crate) mod test_support;

// ---- the builtins ----------------------------------------------------
pub(crate) mod builtins;

// ---- generated tables ------------------------------------------------
pub(crate) mod nodes;
pub(crate) mod signal_names;
pub(crate) mod syntax;
// [spec:nsh:def:idiom.word-ir]
pub(crate) mod word;

// ---- shell state -----------------------------------------------------
pub(crate) mod alias;
pub(crate) mod input;
pub(crate) mod lifecycle;
pub(crate) mod mail;
pub(crate) mod options;
pub(crate) mod redirection;
pub(crate) mod resource;
pub(crate) mod trap;
pub(crate) mod variables;
pub(crate) mod working_directory;

// ---- parsing and expansion -------------------------------------------
// [spec:nsh:req:idiom.module-boundaries]
pub(crate) mod arithmetic;
pub(crate) mod expand;
pub(crate) mod parser;
pub(crate) mod pattern;

// ---- execution --------------------------------------------------------
pub(crate) mod evaluation;
pub(crate) mod execution;
pub(crate) mod jobs;
pub(crate) mod runtime;

// ---- interactive editor ----------------------------------------------
pub(crate) mod editor;
