//! The operating-system and native-runtime boundary for `nsh`.
//!
//! Public functions in this crate are safe. Raw ABI details and target
//! selection stay here so the shell crate sees one portable interface.

#![deny(unsafe_op_in_unsafe_fn)]

use core::fmt;
use core::num::NonZeroU32;

/// The positive identity of one operating-system process.
// [spec:nsh:def:idiom.process-identity]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProcessId(NonZeroU32);

impl ProcessId {
    pub const fn new(value: u32) -> Option<Self> {
        match NonZeroU32::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

impl fmt::Display for ProcessId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(formatter)
    }
}

/// The positive identity of one operating-system process group.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProcessGroupId(NonZeroU32);

impl ProcessGroupId {
    pub const fn new(value: u32) -> Option<Self> {
        match NonZeroU32::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub const fn from_leader(process: ProcessId) -> Self {
        match NonZeroU32::new(process.get()) {
            Some(identity) => Self(identity),
            None => unreachable!(),
        }
    }

    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

impl fmt::Display for ProcessGroupId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(formatter)
    }
}

/// Which process `setpgid` changes. The calling process is an operation,
/// not a magic zero-valued process identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessSelector {
    CurrentProcess,
    Process(ProcessId),
}

/// A POSIX signal destination without the integer sign encoding used by
/// `kill(2)`.
// [spec:nsh:def:idiom.process-identity]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessTarget {
    Process(ProcessId),
    CurrentProcessGroup,
    ProcessGroup(ProcessGroupId),
    AllProcesses,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForkResult {
    Child,
    Parent(ProcessId),
}

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
