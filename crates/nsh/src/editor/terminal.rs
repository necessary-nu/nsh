//! Owned terminal state shared with the shell's long-lived line editor.

use nsh_platform::{EditorTerminalAttributes as TerminalAttributes, TerminalApply as ApplyWhen};
use nshedit::domain::{EditorConfig, ScreenSize, TerminalMode};
use nshedit::editor::TerminalControl;
use std::fs::File;
use std::io;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(super) struct TerminalSnapshots {
    pub(super) original: Option<TerminalAttributes>,
    editing: Option<TerminalAttributes>,
    quoted: Option<TerminalAttributes>,
}

impl TerminalSnapshots {
    pub(super) fn replace(&mut self, original: TerminalAttributes) {
        let editing = original.for_editing();
        let quoted = editing.for_quoted_input();
        self.original = Some(original);
        self.editing = Some(editing);
        self.quoted = Some(quoted);
    }

    fn mode(&self, mode: TerminalMode) -> Option<TerminalAttributes> {
        match mode {
            TerminalMode::Cooked => self.original,
            TerminalMode::Editing => self.editing,
            TerminalMode::Quoted => self.quoted,
        }
    }
}

/// An editor terminal that owns its duplicated descriptors.
///
/// `nshedit::SystemTerminal` is intentionally borrowed. Storing it beside
/// the files it borrows would require a self-reference, so nsh implements the
/// same public terminal-control contract over owned files instead.
pub(super) struct OwnedTerminal {
    input: File,
    output: File,
    locale: nsh_platform::Locale,
    snapshots: Arc<Mutex<TerminalSnapshots>>,
    restoration_due: bool,
}

impl OwnedTerminal {
    pub(super) fn new(
        input: File,
        output: File,
        locale: nsh_platform::Locale,
        snapshots: Arc<Mutex<TerminalSnapshots>>,
    ) -> Self {
        Self {
            input,
            output,
            locale,
            snapshots,
            restoration_due: false,
        }
    }

    pub(super) fn screen_size(output: &File) -> io::Result<ScreenSize> {
        let (rows, columns) = nsh_platform::editor_terminal_size(output)?;
        ScreenSize::new(rows, columns)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    fn apply(&self, when: ApplyWhen, attributes: Option<&TerminalAttributes>) -> io::Result<()> {
        match attributes {
            Some(attributes) => {
                nsh_platform::apply_editor_terminal_attributes(&self.input, when, attributes)
            }
            None => Ok(()),
        }
    }
}

impl TerminalControl for OwnedTerminal {
    fn activate(&mut self, _config: EditorConfig) -> io::Result<()> {
        if !nsh_platform::is_terminal(&self.output) {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "editor output is not a terminal",
            ));
        }
        let original = nsh_platform::editor_terminal_attributes(&self.input).map_err(|error| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                format!("editor input: {}", self.locale.error_message(&error)),
            )
        })?;
        self.snapshots
            .lock()
            .expect("terminal snapshots are not poisoned")
            .replace(original);
        self.restoration_due = true;
        let editing = self
            .snapshots
            .lock()
            .expect("terminal snapshots are not poisoned")
            .editing;
        self.apply(ApplyWhen::AfterOutput, editing.as_ref())
    }

    fn set_mode(&mut self, mode: TerminalMode) -> io::Result<()> {
        let attributes = self
            .snapshots
            .lock()
            .expect("terminal snapshots are not poisoned")
            .mode(mode);
        self.apply(ApplyWhen::AfterOutput, attributes.as_ref())
    }

    fn restore(&mut self) -> io::Result<()> {
        if !self.restoration_due {
            return Ok(());
        }
        self.restoration_due = false;
        let original = self
            .snapshots
            .lock()
            .expect("terminal snapshots are not poisoned")
            .original;
        self.apply(ApplyWhen::AfterOutputAndDiscardInput, original.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    struct TestTerminal;

    impl TerminalControl for TestTerminal {
        fn activate(&mut self, _config: EditorConfig) -> io::Result<()> {
            Ok(())
        }

        fn set_mode(&mut self, _mode: TerminalMode) -> io::Result<()> {
            Ok(())
        }

        fn restore(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    // [spec:posix:def:edit.stty-characters/test]
    #[test]
    fn terminal_binding_refresh_drops_stale() {
        let config = EditorConfig::default().with_editing_mode(EditingMode::Vi);
        let mut editor = Editor::new(config, TestTerminal).unwrap();
        let stale = KeySequence::try_from("\u{18}\u{18}").unwrap();
        editor.bind(
            KeymapMode::ViInsert,
            stale.clone(),
            Binding::Action(Action::Move(Motion::StartOfLine)),
        );

        refresh_shell_bindings(&mut editor, None).unwrap();
        assert!(editor.binding(KeymapMode::ViInsert, &stale).is_none());
        let up = KeySequence::try_from("\u{1b}[A").unwrap();
        assert_eq!(
            editor.binding(KeymapMode::ViInsert, &up),
            Some(&Binding::Effect(EffectCommand::NavigateHistory(
                Direction::Previous,
            ))),
        );
    }
}
