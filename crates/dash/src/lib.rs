//! dash — Debian Almquist Shell, Rust port.
//!
//! This is a **literal port**: every module mirrors the C source
//! file of the same name, function for function, with the same control
//! flow and the same names. It is deliberately un-idiomatic and
//! deliberately bug-for-bug — idiomatisation comes later, and behaviour changes
//! (including bug fixes) come after the port is proven green.
//!
//! Each item carries the `[spec:dash:def:…]` / `[spec:dash:sem:…]`
//! annotations of the rule it implements; the corresponding claims for
//! the C source live in `plan/annotations.styx`.

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
pub mod system;

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
