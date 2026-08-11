//! Builtins imported from standalone BSD utilities.
//! Rules: `docs/spec/port/src/bltin/bltin.md`.
//!
//! The C `bltin.h` compatibility macros used to be reproduced here and
//! forwarded stdio-like calls into the shell output layer. The Rust callers
//! now use [`crate::output::Output`] as an ordinary writer. `printf` keeps
//! its POSIX conversion renderer private to that builtin; the other modules
//! use Rust formatting directly.

pub mod printf;
pub mod test;
pub mod times;

// src/bltin/bltin.h:86 — int echocmd(int, char **);
// Declared here, defined in printf.c.
// [spec:dash:def:bltin.echocmd-fn]
// [spec:dash:sem:bltin.echocmd-fn]
pub use self::printf::echocmd;
