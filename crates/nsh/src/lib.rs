//! nsh — a POSIX shell as a library.
//!
//! Today this is still the literal port of dash 0.5.13.5 it began as:
//! every module mirrors the C source file of the same name, function for
//! function, with the same control flow and the same names. Each item
//! carries the `[spec:dash:def:…]` / `[spec:dash:sem:…]` annotations of
//! the rule it implements; the corresponding claims for the C source
//! live in `plan/annotations.styx`.
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
//! port of `main()` and is what the frontend and the four integration
//! tests invoke, and [`streams`], whose `install` lends the shell the
//! host's descriptors for the duration. The other thirty-six are
//! `pub(crate)`.
//!
//! `#![deny(missing_docs)]` is on, which is the point of closing rather
//! than a tidiness measure: [dec:nsh:public-surface] asked for the surface
//! property to be *measured* under that lint, and a surface of thirty-eight
//! transliterated modules could never have carried it. Closing took it from
//! unaffordable to sixteen items.
//!
//! The command-line shell lives in `crates/nsh-cli` and links this crate
//! as an external dependency, so anything the frontend needs that is not
//! `pub` here is a compile error rather than something a reader has to
//! notice.

#![deny(missing_docs)]
#![allow(dead_code)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(unused_variables)]
#![allow(clippy::all)]
// Edition 2024 turns this on, and on this crate it fires 5,010 times:
// 2,747 calls to an unsafe function, 1,221 raw-pointer dereferences,
// 1,018 reads of a mutable static, 24 union-field accesses. Every one is
// inside a function already declared `unsafe`, so wrapping the bodies
// restates what the signature says and buries the warnings that mean
// something.
//
// The count is the point. [dec:nsh:minimal-unsafe] tracks unsafe
// *functions* — 598 of 794 — which says nothing about how much unsafe is
// inside them. 5,010 operations is the figure that falls as `owned-data`,
// `errors-are-values` and `no-ambient-state` remove raw pointers and
// statics, and it is what to turn this lint on against once it is
// affordable. Re-measure by deleting this line.
#![allow(unsafe_op_in_unsafe_fn)]

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
pub(crate) mod arith_yylex;
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
