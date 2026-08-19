//! The operating-system and native-runtime boundary for `nsh`.
//!
//! Public functions in this crate are safe. Raw ABI details and target
//! selection stay here so the shell crate sees one portable interface.

#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(unix)]
include!("unix.rs");

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;
