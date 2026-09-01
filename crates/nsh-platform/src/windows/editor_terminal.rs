//! The native line editor's terminal operations behind nsh's platform
//! seam.
//!
//! The POSIX file of this name forwards to `nshedit-plat`. Windows has
//! no such backend, so the same surface is answered from the console
//! mode word directly: editing clears line and echo input, quoted input
//! also clears processed input, and the control characters an editor
//! would read out of `termios` are the fixed console ones.

use std::time::Duration;

use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Console::{
    CONSOLE_SCREEN_BUFFER_INFO, ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT,
    GetConsoleScreenBufferInfo,
};
use windows_sys::Win32::System::Threading::WaitForSingleObject;

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalApply {
    AfterOutput,
    AfterOutputAndDiscardInput,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalControlCharacter {
    Erase,
    Kill,
    EndOfFile,
    WordErase,
    LiteralNext,
    Reprint,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EditorTerminalAttributes(u32);

impl EditorTerminalAttributes {
    pub fn for_editing(self) -> Self {
        Self(self.0 & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT))
    }

    pub fn for_quoted_input(self) -> Self {
        Self(self.0 & !ENABLE_PROCESSED_INPUT)
    }

    pub fn control_character(self, character: TerminalControlCharacter) -> u8 {
        match character {
            TerminalControlCharacter::Erase => 8,
            TerminalControlCharacter::Kill => 21,
            TerminalControlCharacter::EndOfFile => 26,
            TerminalControlCharacter::WordErase => 23,
            TerminalControlCharacter::LiteralNext => 22,
            TerminalControlCharacter::Reprint => 18,
        }
    }
}

pub fn editor_terminal_attributes(
    input: &impl AsDescriptor,
) -> std::io::Result<EditorTerminalAttributes> {
    TerminalSettings::capture(input).map(|settings| EditorTerminalAttributes(settings.0))
}

pub fn apply_editor_terminal_attributes(
    input: &impl AsDescriptor,
    _when: TerminalApply,
    attributes: &EditorTerminalAttributes,
) -> std::io::Result<()> {
    TerminalSettings(attributes.0).apply(input)
}

pub fn editor_terminal_size(output: &impl AsDescriptor) -> std::io::Result<(usize, usize)> {
    let mut information = CONSOLE_SCREEN_BUFFER_INFO::default();
    // SAFETY: the output handle is borrowed and the record is writable.
    if unsafe { GetConsoleScreenBufferInfo(raw_handle(output), &mut information) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let columns = i32::from(information.srWindow.Right - information.srWindow.Left + 1);
    let rows = i32::from(information.srWindow.Bottom - information.srWindow.Top + 1);
    Ok((columns.max(0) as usize, rows.max(0) as usize))
}

pub fn wait_for_terminal_input(
    input: &impl AsDescriptor,
    timeout: Duration,
) -> std::io::Result<bool> {
    let milliseconds = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX - 1);
    // SAFETY: the descriptor is borrowed for the duration of the wait.
    match unsafe { WaitForSingleObject(raw_handle(input), milliseconds) } {
        WAIT_OBJECT_0 => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        _ => Err(std::io::Error::last_os_error()),
    }
}
