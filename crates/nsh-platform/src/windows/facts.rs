//! The Windows answers to the questions `unix::facts` answers on a
//! POSIX host: who the process is, what the host is called, and whether
//! a descriptor has anything to read.

use super::{AsDescriptor, TerminalSettings};

/// `ENABLE_ECHO_INPUT`, which `read -s` clears.
const ENABLE_ECHO_INPUT: u32 = 0x0004;

/// `ENABLE_LINE_INPUT`, which `read -n` clears. It is the console's
/// spelling of `ICANON`: with it set the console returns nothing until
/// Enter.
const ENABLE_LINE_INPUT: u32 = 0x0002;

/// The first descriptor number the process cannot use.
///
/// The CRT's descriptor table is what a shell redirection names here, and
/// its documented ceiling is 8192.
#[must_use]
pub fn descriptor_limit() -> u32 {
    8192
}

/// The identity Windows reports for the process.
///
/// Declared here rather than in `windows` because this is the module
/// that answers who the process is, and because a constructor has to sit
/// with the private field it fills. `ca0fa37` fixed a Windows build
/// broken by exactly that split, when this module could not reach a
/// private field `windows` declared.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserId(u32);

impl UserId {
    /// Windows has no superuser in the POSIX sense, and nothing in the
    /// shell may take a privileged path because of this answer.
    pub fn is_root(self) -> bool {
        false
    }

    pub fn as_raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GroupId(u32);

impl GroupId {
    pub fn as_raw(self) -> u32 {
        self.0
    }
}

/// Windows draws no distinction between a real and an effective user, so
/// these two answer alike, and neither varies at runtime.
#[must_use]
pub fn real_uid() -> UserId {
    UserId(1)
}

#[must_use]
pub fn effective_uid() -> UserId {
    UserId(1)
}

#[must_use]
pub fn effective_gid() -> GroupId {
    GroupId(1)
}

pub fn supplementary_groups() -> std::io::Result<Vec<GroupId>> {
    Ok(Vec::new())
}

#[must_use]
pub fn host_name() -> Option<std::ffi::OsString> {
    std::env::var_os("COMPUTERNAME")
}

/// Windows has no descriptor readiness primitive the shell can use
/// here, so a timed read reports "available" and lets the read itself
/// block.
pub fn wait_for_input(_fd: &impl AsDescriptor, _timeout: Option<f64>) -> std::io::Result<bool> {
    Ok(true)
}

impl TerminalSettings {
    /// The same settings with console echo turned off, for `read -s`.
    #[must_use]
    pub fn without_echo(&self) -> Self {
        Self(self.0 & !ENABLE_ECHO_INPUT)
    }

    /// The same settings out of line-input mode, for `read -n`.
    #[must_use]
    pub fn without_canonical_input(&self) -> Self {
        Self(self.0 & !ENABLE_LINE_INPUT)
    }
}
