//! nsh — a POSIX shell as a library.
//!
//! Today this is still the literal port of dash 0.5.13.5 it began as:
//! every module mirrors the C source file of the same name, function for
//! function, with the same control flow and the same names. Each item
//! carries the `[spec:dash:def:…]` / `[spec:dash:sem:…]` annotations of
//! the rule it implements; the corresponding claims for the C source
//! live in `plan/annotations.styx`.
//!
//! It is not a library yet in any sense but Cargo's. Thirty-three public
//! modules is not an API, it is the transliteration left open — see
//! [dec:nsh:public-surface]. `docs/idiomatization.md` is the path from
//! here to a surface an embedder can hold, and the properties that decide
//! when it has arrived.
//!
//! The command-line shell lives in `crates/nsh-cli` and links this crate
//! as an external dependency, so anything the frontend needs that is not
//! `pub` here is a compile error rather than something a reader has to
//! notice.

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

// ---- the proposed public API, unimplemented --------------------------
//
// `api` is a design artefact, not code: every body is `todo!()` and
// nothing else in the crate refers to it. It is compiled so that the
// signatures in `docs/api-design.md` are checked by the compiler rather
// than by reading. It is deleted by the `public-api` node, which replaces
// it with the implementation.
//
// Behind a feature that nothing enables by default, because leaving it
// `pub` would put a surface an embedder can call -- and that panics on
// every call -- into the crate's public API. [dec:nsh:public-surface]
// exists to make that surface honest, so a sketch shipping as part of it
// would contradict the decision it was written to serve.
#[cfg(feature = "api-sketch")]
pub mod api;

// ---- foundation -----------------------------------------------------
pub mod error;
pub mod memalloc;
pub mod mystring;
pub mod output;
pub mod shell;
pub mod streams;
pub mod system;

// ---- unit-test scaffolding (test builds only) ------------------------
#[cfg(test)]
pub mod testutil;

// ---- generated tables ------------------------------------------------
pub mod builtins;
pub mod nodes;
pub mod signames;
pub mod syntax;

// ---- shell state -----------------------------------------------------
pub mod alias;
pub mod cd;
pub mod init;
pub mod input;
pub mod mail;
pub mod options;
pub mod redir;
pub mod trap;
pub mod var;

// ---- parsing and expansion -------------------------------------------
pub mod arith_yacc;
pub mod arith_yylex;
pub mod expand;
pub mod parser;

// ---- execution --------------------------------------------------------
pub mod eval;
pub mod exec;
pub mod jobs;
pub mod miscbltin;
pub mod shellmain;

// ---- builtins ---------------------------------------------------------
pub mod bltin;
pub mod histedit;
pub mod linedit;
