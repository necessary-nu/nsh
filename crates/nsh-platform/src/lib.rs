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

/// An owned description of the program image that should replace this one.
///
/// Native strings preserve every path, argument, and environment code unit.
/// Platform implementations may borrow these values while materializing their
/// private ABI representation, but no pointer or terminator is part of this
/// public type.
// [spec:nsh:req:idiom.exec-boundary]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProgramImage {
    pub(crate) path: std::path::PathBuf,
    pub(crate) arguments: Vec<std::ffi::OsString>,
    pub(crate) environment: Vec<(std::ffi::OsString, std::ffi::OsString)>,
}

impl ProgramImage {
    #[must_use]
    pub fn new(
        path: std::path::PathBuf,
        arguments: Vec<std::ffi::OsString>,
        environment: Vec<(std::ffi::OsString, std::ffi::OsString)>,
    ) -> Self {
        Self {
            path,
            arguments,
            environment,
        }
    }
}

mod descriptor_name;
pub use descriptor_name::{
    descriptor_name, descriptor_name_directory, publish_descriptor_across_exec,
};

/* The one subject both hosts answer from the same file, so it is
 * declared here rather than inside either of them. Both host roots
 * reached it with a `#[path]` at one time or another, which is what a
 * shared child looks like when it is declared from the wrong place. */
mod signal_names;
pub use signal_names::{SIGNAL_COUNT, SIGNAL_NAMES};

/* The POSIX host. Its four pre-`src/unix/` subjects sit beside this file
 * rather than under `unix/`, and so are declared here for the same reason
 * `windows_facts` is: a `mod` item names a file relative to the module it
 * is written in, and these are not in the directory `unix.rs` owns. That
 * is the whole of why the list is this long, and it is the price of every
 * module below sitting at the path the source puts it at. */
// [spec:nsh:req:idiom.declared-module-tree]
#[cfg(unix)]
mod descriptor;
#[cfg(unix)]
pub use descriptor::{AsDescriptor, BorrowedDescriptor, Descriptor, move_fd_cloexec};
#[cfg(unix)]
mod locale;
#[cfg(unix)]
pub use locale::{Locale, LocaleCategory, LocaleCharacter, LocaleDecode, LocaleDecoder};
#[cfg(unix)]
mod terminal;
#[cfg(unix)]
pub use terminal::{TerminalSettings, is_terminal, terminal_canonical_mode};
#[cfg(unix)]
mod unix_facts;
#[cfg(unix)]
pub use unix_facts::{
    GroupId, UserId, descriptor_limit, effective_gid, effective_uid, host_name, real_uid,
    supplementary_groups, wait_for_input,
};
/* The only user of `nshedit-plat`, and so the only part of the boundary
 * the `edit` feature gates. */
#[cfg(all(unix, feature = "edit"))]
mod editor_terminal;
#[cfg(all(unix, feature = "edit"))]
pub use editor_terminal::{
    EditorTerminalAttributes, TerminalApply, TerminalControlCharacter,
    apply_editor_terminal_attributes, editor_terminal_attributes, editor_terminal_size,
    wait_for_terminal_input,
};
#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;
#[cfg(windows)]
mod windows_facts;
#[cfg(windows)]
pub use windows_facts::{
    GroupId, UserId, descriptor_limit, effective_gid, effective_uid, host_name, real_uid,
    supplementary_groups, wait_for_input,
};

/// How one locale's charmap writes a character it was asked to write.
///
/// The three answers are different in kind, not in degree, which is why
/// this is not `Option<Vec<u8>>`: a charmap can spell the character, or it
/// has no spelling for it at all, or it is UTF-8 -- and UTF-8 is answered
/// by naming it rather than by encoding it, because the C library has
/// since narrowed UTF-8 to Unicode's range while the shells that write
/// these escapes still use the original. Encoding it here would quietly
/// drop the surrogates and the range above U+10FFFF that a script can
/// still ask for.
pub enum CharacterEncoding {
    /// The charmap is UTF-8; the caller supplies the encoding.
    Utf8,
    /// The charmap's own spelling for the character.
    Bytes(Vec<u8>),
    /// The charmap has no spelling for this character.
    Unrepresentable,
}

impl Locale {
    /// How this locale's charmap writes the character numbered `value`.
    #[cfg(unix)]
    pub fn character_encoding(&self, value: u32) -> CharacterEncoding {
        if self.charmap_is_utf8() {
            return CharacterEncoding::Utf8;
        }
        match self.encode_character(value) {
            Some(bytes) => CharacterEncoding::Bytes(bytes),
            None => CharacterEncoding::Unrepresentable,
        }
    }

    /// Windows shell text is UTF-8 whatever the locale is named, so there
    /// is one charmap here and it is the one the caller encodes itself.
    #[cfg(windows)]
    pub fn character_encoding(&self, _value: u32) -> CharacterEncoding {
        CharacterEncoding::Utf8
    }
}

/// Host facts a Bash-compatible shell publishes as variables, and the two
/// clocks it reads: the monotonic one behind `SECONDS` and the wall clock
/// behind `EPOCHSECONDS`.
///
/// These live here rather than in the shell for the reason every other
/// operating-system fact does: the shell crate names no platform API of
/// its own. Nothing here is a descriptor or a signal, so the module is
/// shared by both targets and only the identity and hostname lookups
/// differ.
pub mod facts {
    use std::hash::BuildHasher as _;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Seconds elapsed on a monotonic clock since an arbitrary origin
    /// fixed the first time this is called in the process.
    ///
    /// A shell reads differences, never the value, so the origin only
    /// has to be stable -- and a monotonic origin is what keeps
    /// `SECONDS` from jumping when the system clock is set.
    #[must_use]
    pub fn monotonic_seconds() -> f64 {
        static ORIGIN: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        let origin = ORIGIN.get_or_init(std::time::Instant::now);
        origin.elapsed().as_secs_f64()
    }

    /// Whole seconds and nanoseconds since the Unix epoch.
    ///
    /// A clock set before 1970 reports zero rather than a negative time,
    /// which is what `EPOCHREALTIME` can render.
    #[must_use]
    pub fn wall_clock() -> (u64, u32) {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or((0, 0), |elapsed| {
                (elapsed.as_secs(), elapsed.subsec_nanos())
            })
    }

    /// A seed the host chose, for a generator whose output must not be
    /// predictable from anything a script can observe.
    ///
    /// The standard library seeds its hash keys from the operating
    /// system's randomness at first use, which is the same source a
    /// dedicated call would reach and needs no further dependency.
    #[must_use]
    pub fn entropy_seed() -> u64 {
        let state = std::collections::hash_map::RandomState::new();
        state.hash_one(std::time::Instant::now().elapsed().as_nanos() as u64)
    }

    /// `OSTYPE`.
    #[must_use]
    pub const fn operating_system_type() -> &'static str {
        if cfg!(target_os = "linux") {
            "linux-gnu"
        } else if cfg!(target_os = "macos") {
            "darwin"
        } else if cfg!(windows) {
            "msys"
        } else {
            std::env::consts::OS
        }
    }

    /// `HOSTTYPE`.
    #[must_use]
    pub const fn hardware_type() -> &'static str {
        std::env::consts::ARCH
    }

    /// `MACHTYPE`, which Bash spells as a configuration triple.
    #[must_use]
    pub const fn machine_type() -> &'static str {
        if cfg!(all(target_arch = "x86_64", target_os = "linux")) {
            "x86_64-pc-linux-gnu"
        } else if cfg!(all(target_arch = "aarch64", target_os = "linux")) {
            "aarch64-unknown-linux-gnu"
        } else if cfg!(all(target_arch = "x86_64", target_os = "macos")) {
            "x86_64-apple-darwin"
        } else if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
            "arm64-apple-darwin"
        } else {
            std::env::consts::ARCH
        }
    }
}
