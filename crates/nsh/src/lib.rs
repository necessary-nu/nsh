//! nsh — a POSIX shell as a library.
//!
//! Today this is still the literal port of dash 0.5.13.5 it began as:
//! every module mirrors the C source file of the same name, function for
//! function, with the same control flow and the same names. Each item
//! carries the `[spec:dash:def:…]` / `[spec:dash:sem:…]` annotations of
//! the rule it implements; the corresponding claims for the C source
//! live in `plan/annotations.styx`.
//!
//! It is not a library yet in any sense but Cargo's. Thirty-five public
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

// ---- build-time generators (literal ports of src/mk*.c) --------------
pub mod gen;

// ---- shell state -----------------------------------------------------
pub mod alias;
pub mod cd;
pub mod init;
pub mod input;
pub mod mail;
pub mod options;
pub mod redir;
pub mod show;
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
