//! The native line editor's terminal operations behind nsh's platform seam.

use std::time::Duration;

use crate::AsDescriptor;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalApply {
    AfterOutput,
    AfterOutputAndDiscardInput,
}

impl TerminalApply {
    fn native(self) -> nshedit_plat::terminal::ApplyWhen {
        match self {
            Self::AfterOutput => nshedit_plat::terminal::ApplyWhen::AfterOutput,
            Self::AfterOutputAndDiscardInput => {
                nshedit_plat::terminal::ApplyWhen::AfterOutputAndDiscardInput
            }
        }
    }
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

impl TerminalControlCharacter {
    fn native(self) -> nshedit_plat::terminal::ControlCharacter {
        match self {
            Self::Erase => nshedit_plat::terminal::ControlCharacter::Erase,
            Self::Kill => nshedit_plat::terminal::ControlCharacter::Kill,
            Self::EndOfFile => nshedit_plat::terminal::ControlCharacter::EndOfFile,
            Self::WordErase => nshedit_plat::terminal::ControlCharacter::WordErase,
            Self::LiteralNext => nshedit_plat::terminal::ControlCharacter::LiteralNext,
            Self::Reprint => nshedit_plat::terminal::ControlCharacter::Reprint,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EditorTerminalAttributes(nshedit_plat::terminal::TerminalAttributes);

impl EditorTerminalAttributes {
    pub fn for_editing(self) -> Self {
        Self(self.0.for_editing())
    }

    pub fn for_quoted_input(self) -> Self {
        Self(self.0.for_quoted_input())
    }

    pub fn control_character(self, character: TerminalControlCharacter) -> u8 {
        self.0.control_character(character.native())
    }
}

pub fn editor_terminal_attributes(
    input: &impl AsDescriptor,
) -> std::io::Result<EditorTerminalAttributes> {
    nshedit_plat::terminal::read_attributes(input.as_platform_descriptor().0)
        .map(EditorTerminalAttributes)
}

pub fn apply_editor_terminal_attributes(
    input: &impl AsDescriptor,
    when: TerminalApply,
    attributes: &EditorTerminalAttributes,
) -> std::io::Result<()> {
    nshedit_plat::terminal::apply_attributes(
        input.as_platform_descriptor().0,
        when.native(),
        &attributes.0,
    )
}

pub fn editor_terminal_size(output: &impl AsDescriptor) -> std::io::Result<(usize, usize)> {
    nshedit_plat::terminal::screen_size(output.as_platform_descriptor().0)
}

pub fn wait_for_terminal_input(
    input: &impl AsDescriptor,
    timeout: Duration,
) -> std::io::Result<bool> {
    nshedit_plat::terminal::wait_for_input(input.as_platform_descriptor().0, timeout)
}
