//! The operating-system and native-runtime boundary for `nsh`.
//!
//! Public functions in this crate are safe. Raw ABI details and target
//! selection stay here so the shell crate sees one portable interface.

#![deny(unsafe_op_in_unsafe_fn)]

/// Portable error cases synthesized by the shell rather than returned by an
/// operating-system operation.
// [spec:nsh:req:idiom.platform-errors]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformErrorKind {
    AlreadyExists,
    BadDescriptor,
    NotFound,
    PermissionDenied,
}

#[cfg(unix)]
include!("unix.rs");

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;
