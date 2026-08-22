//! The Windows answers to the questions `unix_facts` answers on a
//! POSIX host: who the process is, what the host is called, and whether
//! a descriptor has anything to read.

use super::{AsDescriptor, TerminalSettings, UserId};

/// `ENABLE_ECHO_INPUT`, which `read -s` clears.
const ENABLE_ECHO_INPUT: u32 = 0x0004;

#[must_use]
pub fn real_uid() -> UserId {
    UserId(1)
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
}
