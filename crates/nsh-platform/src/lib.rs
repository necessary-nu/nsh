//! The operating-system and native-runtime boundary for `nsh`.
//!
//! Public functions in this crate are safe. Raw ABI details and target
//! selection stay here so the shell crate sees one portable interface.

#![deny(unsafe_op_in_unsafe_fn)]

use core::fmt;
use core::num::{NonZeroI32, NonZeroU32};

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

/// A process group observed through a namespace boundary.
///
/// Linux reports zero when a terminal or process group belongs to an ancestor
/// PID namespace. That is a real, comparable state even though zero is not a
/// process-group identity in the caller's namespace.
// [spec:nsh:req:idiom.process-group-zero-state]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessGroupState {
    /// The group exists outside the caller's PID namespace.
    OutsideNamespace,
    /// A positive group visible in the caller's PID namespace.
    Visible(ProcessGroupId),
}

impl ProcessGroupState {
    #[cfg(unix)]
    fn from_platform_value(value: i32) -> Option<Self> {
        if value == 0 {
            Some(Self::OutsideNamespace)
        } else {
            u32::try_from(value)
                .ok()
                .and_then(ProcessGroupId::new)
                .map(Self::Visible)
        }
    }

    #[cfg(unix)]
    fn platform_value(self) -> std::io::Result<i32> {
        match self {
            Self::OutsideNamespace => Ok(0),
            Self::Visible(group) => i32::try_from(group.get())
                .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput)),
        }
    }

    #[cfg(windows)]
    fn nonnegative_platform_value(self) -> u32 {
        match self {
            Self::OutsideNamespace => 0,
            Self::Visible(group) => group.get(),
        }
    }
}

impl From<ProcessGroupId> for ProcessGroupState {
    fn from(group: ProcessGroupId) -> Self {
        Self::Visible(group)
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

/// A positive operating-system signal number.
// [spec:nsh:def:idiom.signal-wait]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Signal(NonZeroI32);

impl Signal {
    pub const fn new(number: i32) -> Option<Self> {
        match NonZeroI32::new(number) {
            Some(number) if number.is_positive() => Some(Self(number)),
            _ => None,
        }
    }

    pub const fn number(self) -> i32 {
        self.0.get()
    }
}

impl fmt::Display for Signal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.number().fmt(formatter)
    }
}

/// What a `kill`-style operation requests. Signal zero probes for a process;
/// it is not represented as an invalid `Signal`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignalRequest {
    Probe,
    Deliver(Signal),
}

/// The decoded state returned by waiting for a child process.
// [spec:nsh:def:idiom.signal-wait]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChildStatus {
    Exited(u8),
    Signaled { signal: Signal, core_dumped: bool },
    Stopped(Signal),
    Continued,
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
