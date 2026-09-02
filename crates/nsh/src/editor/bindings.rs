//! What the shell binds, and the commands only the shell names.
//!
//! POSIX gives vi-mode editing to `sh`, so which key means which motion is
//! the shell's answer rather than nshedit's.  This is that answer: the
//! terminal and arrow sequences, the vi command keymap, the `stty` control
//! characters read from the terminal's own attributes, and the alias and
//! user-command effects nshedit dispatches back here by name -- move to
//! first non-blank and its delete, change and yank operators, and the
//! undo-all that restores the line as it was first entered.
//!
//! `mod.rs` keeps the editor and its read loop; this keeps the table they
//! are configured with.

use bstr::BStr;
use nsh_platform::{
    EditorTerminalAttributes as TerminalAttributes, TerminalControlCharacter as ControlCharacter,
};
use nshedit::domain::{
    Action, ArgumentCommand, Binding, CommandName, CommandSequence, Direction, EditTarget,
    EffectCommand, HistorySearchCommand, ImmediateCommand, InputMode, KeySequence, KeymapMode,
    Motion, Outcome, Refresh, Text, TextUnit, WordTraversal, YankPlacement,
};
use nshedit::editor::effect::{AliasResponse, HostFailure};
use nshedit::editor::{Editor, TerminalControl};

use super::{
    CHANGE_TO_FIRST_NONBLANK, DELETE_TO_FIRST_NONBLANK, DISPLAY_EXPANSIONS, EXPAND_ALL,
    FIRST_NONBLANK, NativeEditor, REPEAT_HISTORY_SEARCH, REVERSE_HISTORY_SEARCH, UNDO_ALL_CHANGES,
    YANK_TO_FIRST_NONBLANK, host_failure, text_from_bytes, text_to_bytes,
};

// [spec:posix:sem:edit.append-last-bigword]
pub(super) fn install_shell_bindings<T: TerminalControl>(
    editor: &mut Editor<T>,
    terminal_attributes: Option<&TerminalAttributes>,
) -> Result<(), nshedit::domain::Error> {
    let terminal_bindings = [
        (
            "\u{1b}[A",
            Binding::Effect(EffectCommand::NavigateHistory(Direction::Previous)),
        ),
        (
            "\u{1b}[B",
            Binding::Effect(EffectCommand::NavigateHistory(Direction::Next)),
        ),
        (
            "\u{1b}[C",
            Binding::Action(Action::Move(Motion::Character(Direction::Next))),
        ),
        (
            "\u{1b}[D",
            Binding::Action(Action::Move(Motion::Character(Direction::Previous))),
        ),
        (
            "\u{1b}[H",
            Binding::Action(Action::Move(Motion::StartOfLine)),
        ),
        ("\u{1b}[F", Binding::Action(Action::Move(Motion::EndOfLine))),
        (
            "\u{1b}[3~",
            Binding::Action(Action::Delete(EditTarget::Character(Direction::Next))),
        ),
    ];
    for mode in [
        KeymapMode::Emacs,
        KeymapMode::ViInsert,
        KeymapMode::ViCommand,
    ] {
        for (sequence, binding) in &terminal_bindings {
            editor.bind(mode, KeySequence::try_from(*sequence)?, binding.clone());
        }
    }
    editor.bind(
        KeymapMode::ViInsert,
        KeySequence::try_from("\t")?,
        Binding::Effect(EffectCommand::Complete),
    );

    let vi_command_bindings = [
        (
            "\u{1}",
            Binding::Action(Action::Move(Motion::StartOfBuffer)),
        ),
        (
            "\u{8}",
            Binding::Immediate(ImmediateCommand::DeletePreviousUnit),
        ),
        (
            "\u{b}",
            Binding::Action(Action::Kill(EditTarget::Motion(Motion::EndOfBuffer))),
        ),
        ("\u{c}", Binding::Action(Action::Refresh(Refresh::Full))),
        (
            "\u{e}",
            Binding::Effect(EffectCommand::NavigateHistory(Direction::Next)),
        ),
        (
            "\u{10}",
            Binding::Effect(EffectCommand::NavigateHistory(Direction::Previous)),
        ),
        (
            "\u{12}",
            Binding::Action(Action::Refresh(Refresh::Redisplay)),
        ),
        (
            "\u{15}",
            Binding::Action(Action::Kill(EditTarget::Motion(Motion::StartOfBuffer))),
        ),
        (
            "\u{17}",
            Binding::Immediate(ImmediateCommand::TraverseWords {
                direction: Direction::Previous,
                operation: WordTraversal::Kill,
            }),
        ),
        (
            " ",
            Binding::Action(Action::Move(Motion::Character(Direction::Next))),
        ),
        (
            "+",
            Binding::Effect(EffectCommand::NavigateHistory(Direction::Next)),
        ),
        (
            "-",
            Binding::Effect(EffectCommand::NavigateHistory(Direction::Previous)),
        ),
        (
            "0",
            Binding::Immediate(ImmediateCommand::StartOfLineOrArgument),
        ),
        // Entering insert mode after the cursor before requesting ordinary
        // insert-mode completion makes the command-mode cursor's character
        // part of the current bigword and leaves the editor in insert mode.
        ("\\", Binding::Macro(Text::from("a\t"))),
        ("_", Binding::Effect(EffectCommand::InsertHistoryWord)),
        (
            "^",
            Binding::User(
                CommandName::new(FIRST_NONBLANK).expect("static shell command name is valid"),
            ),
        ),
        ("@", Binding::Effect(EffectCommand::ExpandAlias)),
        (":", Binding::Effect(EffectCommand::ReadEditorCommand)),
        (
            "=",
            Binding::User(
                CommandName::new(DISPLAY_EXPANSIONS).expect("static shell command name is valid"),
            ),
        ),
        (
            "*",
            Binding::User(
                CommandName::new(EXPAND_ALL).expect("static shell command name is valid"),
            ),
        ),
        (
            "J",
            Binding::Effect(EffectCommand::SearchHistory(HistorySearchCommand::Prefix(
                Direction::Next,
            ))),
        ),
        (
            "K",
            Binding::Effect(EffectCommand::SearchHistory(HistorySearchCommand::Prefix(
                Direction::Previous,
            ))),
        ),
        (
            "P",
            Binding::Immediate(ImmediateCommand::PasteRegister(YankPlacement::AtCursor)),
        ),
        ("u", Binding::Action(Action::Undo)),
        (
            "U",
            Binding::User(
                CommandName::new(UNDO_ALL_CHANGES).expect("static shell command name is valid"),
            ),
        ),
        (
            "Y",
            Binding::Action(Action::Copy(EditTarget::Motion(Motion::EndOfLine))),
        ),
        (
            "X",
            // The native action vocabulary deliberately applies a counted
            // `Kill(Character)` only once.  Express vi's counted `X` as the
            // ordinary delete-operator/backward-motion interaction; macro
            // bindings preserve and replay the caller's count.
            Binding::Macro(Text::from("dh")),
        ),
        (
            "j",
            Binding::Effect(EffectCommand::NavigateHistory(Direction::Next)),
        ),
        (
            "k",
            Binding::Effect(EffectCommand::NavigateHistory(Direction::Previous)),
        ),
        // [spec:posix:req:edit.history-search-repeat]
        ("n", REPEAT_HISTORY_SEARCH),
        ("N", REVERSE_HISTORY_SEARCH),
        (
            "p",
            Binding::Immediate(ImmediateCommand::PasteRegister(YankPlacement::AfterCursor)),
        ),
        (
            "x",
            Binding::Action(Action::Kill(EditTarget::Character(Direction::Next))),
        ),
        (
            "~",
            Binding::Immediate(ImmediateCommand::ToggleCaseAndAdvance),
        ),
        (
            "c^",
            Binding::User(
                CommandName::new(CHANGE_TO_FIRST_NONBLANK)
                    .expect("static shell command name is valid"),
            ),
        ),
        // POSIX shell vi mode gives `cw` the traditional change-to-word-end
        // behavior, preserving the separator before the next word.
        ("cw", Binding::Macro(Text::from("ce"))),
        (
            "d^",
            Binding::User(
                CommandName::new(DELETE_TO_FIRST_NONBLANK)
                    .expect("static shell command name is valid"),
            ),
        ),
        (
            "y^",
            Binding::User(
                CommandName::new(YANK_TO_FIRST_NONBLANK)
                    .expect("static shell command name is valid"),
            ),
        ),
    ];
    for (sequence, binding) in vi_command_bindings {
        editor.bind(
            KeymapMode::ViCommand,
            KeySequence::try_from(sequence)?,
            binding,
        );
    }
    for digit in '1'..='9' {
        editor.bind(
            KeymapMode::ViCommand,
            KeySequence::new(Text::from(digit.to_string()))?,
            Binding::Sequence(CommandSequence::Argument(ArgumentCommand::StartDigit)),
        );
    }

    if let Some(attributes) = terminal_attributes {
        install_terminal_character(
            editor,
            attributes,
            ControlCharacter::Erase,
            &[
                KeymapMode::Emacs,
                KeymapMode::ViInsert,
                KeymapMode::ViCommand,
            ],
            Binding::Immediate(ImmediateCommand::DeletePreviousUnit),
        )?;
        install_terminal_character(
            editor,
            attributes,
            ControlCharacter::Kill,
            &[KeymapMode::Emacs, KeymapMode::ViInsert],
            Binding::Action(Action::Kill(EditTarget::Buffer)),
        )?;
        // [spec:posix:req:edit.insert-end-of-file]
        install_terminal_character(
            editor,
            attributes,
            ControlCharacter::EndOfFile,
            &[
                KeymapMode::Emacs,
                KeymapMode::ViInsert,
                KeymapMode::ViCommand,
            ],
            Binding::Immediate(ImmediateCommand::EndOfInputIfEmpty),
        )?;
        install_terminal_character(
            editor,
            attributes,
            ControlCharacter::WordErase,
            &[KeymapMode::Emacs, KeymapMode::ViInsert],
            Binding::Immediate(ImmediateCommand::TraverseWords {
                direction: Direction::Previous,
                operation: WordTraversal::Kill,
            }),
        )?;
        // [spec:posix:sem:edit.insert-literal-next]
        install_terminal_character(
            editor,
            attributes,
            ControlCharacter::LiteralNext,
            &[KeymapMode::Emacs, KeymapMode::ViInsert],
            Binding::Sequence(CommandSequence::QuotedInsert),
        )?;
        install_terminal_character(
            editor,
            attributes,
            ControlCharacter::Reprint,
            &[KeymapMode::Emacs, KeymapMode::ViInsert],
            Binding::Action(Action::Refresh(Refresh::Full)),
        )?;
    }
    Ok(())
}

fn install_terminal_character<T: TerminalControl>(
    editor: &mut Editor<T>,
    attributes: &TerminalAttributes,
    character: ControlCharacter,
    modes: &[KeymapMode],
    binding: Binding,
) -> Result<(), nshedit::domain::Error> {
    let byte = attributes.control_character(character);
    let sequence = KeySequence::new(text_from_bytes(&[byte]))?;
    for mode in modes {
        editor.bind(*mode, sequence.clone(), binding.clone());
    }
    Ok(())
}

pub(super) fn refresh_shell_bindings<T: TerminalControl>(
    editor: &mut Editor<T>,
    attributes: Option<&TerminalAttributes>,
) -> Result<(), nshedit::domain::Error> {
    let mode = editor.config().editing_mode();
    editor.reset_bindings(mode);
    install_shell_bindings(editor, attributes)
}

// [spec:posix:req:edit.command-alias-insert]
pub(super) fn shell_alias(
    shell: &mut crate::context::Shell,
    name: &Text,
    enter_insert: bool,
) -> Result<AliasResponse, HostFailure> {
    let name = text_to_bytes(name).map_err(host_failure)?;
    if name.contains(&0) {
        return Err(HostFailure::Failed(
            "an editor alias name contains NUL".into(),
        ));
    }
    let Some(expansion) = shell.aliases.lookup(BStr::new(&name), false) else {
        return Ok(AliasResponse::Missing);
    };
    let mut macro_text = Text::default();
    // POSIX `@letter` inserts ordinary alias text.  Starting the native
    // macro in Vi insertion mode gives embedded escape sequences and later
    // command keys their normal editor meaning without baking shell policy
    // into nshedit's generic alias effect.
    if enter_insert {
        macro_text.push(TextUnit::Scalar('i'));
    }
    macro_text.extend(text_from_bytes(&expansion).as_units().iter().copied());
    Ok(AliasResponse::Expansion(macro_text))
}

pub(super) fn shell_user_command(
    editor: &mut NativeEditor,
    name: &str,
    vi_original_line: Option<&Text>,
) -> Result<Outcome, HostFailure> {
    let first_nonblank = first_nonblank_index(editor.line());
    let destination = editor.line().index(first_nonblank).map_err(host_failure)?;
    match name {
        FIRST_NONBLANK => editor
            .execute(Action::Move(Motion::Absolute(destination)))
            .map_err(host_failure),
        DELETE_TO_FIRST_NONBLANK | CHANGE_TO_FIRST_NONBLANK | YANK_TO_FIRST_NONBLANK => {
            let cursor = editor.cursor().get();
            let start = cursor.min(first_nonblank);
            let mut end = cursor.max(first_nonblank);
            if start == end && start < editor.line().len() {
                end += 1;
            }
            let span = editor.line().span(start..end).map_err(host_failure)?;
            let action = if name == YANK_TO_FIRST_NONBLANK {
                Action::Copy(EditTarget::Span(span))
            } else {
                Action::Kill(EditTarget::Span(span))
            };
            let outcome = editor.execute(action).map_err(host_failure)?;
            if name == CHANGE_TO_FIRST_NONBLANK {
                editor
                    .execute(Action::SetModes {
                        input: InputMode::Insert,
                        keymap: KeymapMode::ViInsert,
                    })
                    .map_err(host_failure)
            } else {
                Ok(outcome)
            }
        }
        // [spec:posix:req:edit.undo]
        UNDO_ALL_CHANGES => {
            let whole_line = editor
                .line()
                .span(0..editor.line().len())
                .map_err(host_failure)?;
            editor
                .replace(whole_line, vi_original_line.cloned().unwrap_or_default())
                .map_err(host_failure)?;
            if editor.line().is_empty() {
                editor
                    .execute(Action::Refresh(Refresh::Full))
                    .map_err(host_failure)
            } else {
                editor
                    .execute(Action::Move(Motion::Character(Direction::Previous)))
                    .map_err(host_failure)
            }
        }
        _ => Err(HostFailure::Unavailable),
    }
}

fn first_nonblank_index(line: &Text) -> usize {
    line.as_units()
        .iter()
        .position(|unit| {
            !matches!(
                unit,
                TextUnit::Scalar(' ' | '\t') | TextUnit::RawByte(b' ' | b'\t')
            )
        })
        .unwrap_or(0)
}

pub(super) fn next_vi_original_line(
    original: Option<Text>,
    current: &Text,
    entered_command_mode: bool,
    copied_line: bool,
) -> Option<Text> {
    if copied_line || (entered_command_mode && original.is_none()) {
        Some(current.clone())
    } else {
        original
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_state_preserves_owned_text() {
        assert_eq!(first_nonblank_index(&Text::from(" \tword")), 2);
        assert_eq!(first_nonblank_index(&text_from_bytes(b" \t\xffword")), 2);
        assert_eq!(first_nonblank_index(&Text::default()), 0);

        let typed = text_from_bytes(b"typed\xff");
        let original = next_vi_original_line(None, &typed, true, false);
        assert_eq!(original, Some(typed.clone()));

        let changed = Text::from("changed");
        let original = next_vi_original_line(original, &changed, true, false);
        assert_eq!(original, Some(typed));

        let history = Text::from("history");
        let original = next_vi_original_line(original, &history, false, true);
        assert_eq!(original, Some(history.clone()));
        assert_eq!(
            next_vi_original_line(original, &changed, false, false),
            Some(history)
        );
    }
}
