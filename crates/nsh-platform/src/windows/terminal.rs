//! Console-mode snapshots, and the two questions asked of a descriptor
//! before one is taken.
//!
//! The snapshot is the console mode word, kept opaque for the same
//! reason the POSIX side keeps `termios` opaque: the shell saves and
//! restores terminal state around a job without being told what that
//! state is. `is_terminal` and `terminal_canonical_mode` are the same
//! `GetConsoleMode` call asked for an answer rather than for the
//! settings.

use windows_sys::Win32::System::Console::{ENABLE_LINE_INPUT, GetConsoleMode, SetConsoleMode};

use super::*;

#[derive(Clone, Copy)]
pub struct TerminalSettings(pub(crate) u32);

impl TerminalSettings {
    pub fn capture(fd: &impl AsDescriptor) -> std::io::Result<Self> {
        let mut mode = 0;
        // SAFETY: the descriptor is borrowed and `mode` is writable.
        if unsafe { GetConsoleMode(raw_handle(fd), &mut mode) } == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(Self(mode))
        }
    }

    pub fn apply(&self, fd: &impl AsDescriptor) -> std::io::Result<()> {
        // SAFETY: the descriptor is borrowed and the mode came from the
        // console API for this class of handle.
        if unsafe { SetConsoleMode(raw_handle(fd), self.0) } == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

pub fn terminal_canonical_mode(fd: &impl AsDescriptor) -> Option<bool> {
    TerminalSettings::capture(fd)
        .ok()
        .map(|settings| settings.0 & ENABLE_LINE_INPUT != 0)
}

pub fn is_terminal(fd: &impl AsDescriptor) -> bool {
    TerminalSettings::capture(fd).is_ok()
}
