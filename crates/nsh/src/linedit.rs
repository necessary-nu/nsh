//! Native `nshedit` integration for interactive `nsh` sessions.
//!
//! The editor and its read driver are Rust values.  They borrow duplicated
//! terminal descriptors, consume and produce owned [`Text`] values, and ask
//! the shell for host services through typed effects.  No libedit operation
//! codes, callbacks, C streams, or ABI structs cross this module.

// ---------------------------------------------------------------------
// The shell's line editing, claimed here.
//
// POSIX describes vi-mode editing as behaviour of `sh`, so the rules are
// the shell's to satisfy.  The motions and editing state live in nshedit;
// this module supplies the shell-owned prompt, descriptors, history and
// completion effects that make them an nsh session.
// [spec:posix:req:edit.block-mode-terminals]
// [spec:posix:req:edit.change-motion]
// [spec:posix:sem:edit.change-to-end-and-line]
// [spec:posix:req:edit.command-case-toggle]
// [spec:posix:req:edit.command-comment]
// [spec:posix:req:edit.command-count]
// [spec:posix:req:edit.command-invoke-vi]
// [spec:posix:req:edit.command-newline]
// [spec:posix:sem:edit.command-redraw]
// [spec:posix:req:edit.command-repeat]
// [spec:posix:def:edit.cursor-terminology]
// [spec:posix:req:edit.delete-char]
// [spec:posix:req:edit.delete-motion]
// [spec:posix:req:edit.enter-insert-mode]
// [spec:posix:req:edit.escape-to-command-mode]
// [spec:posix:req:edit.insert-deletion]
// [spec:posix:sem:edit.insert-escape]
// [spec:posix:req:edit.insert-interrupt]
// [spec:posix:req:edit.insert-mode-default]
// [spec:posix:req:edit.insert-mode-special-characters]
// [spec:posix:req:edit.insert-newline]
// [spec:posix:req:edit.motion-char]
// [spec:posix:req:edit.motion-char-search]
// [spec:posix:req:edit.motion-char-search-repeat]
// [spec:posix:def:edit.motion-command-set]
// [spec:posix:req:edit.motion-line-position]
// [spec:posix:req:edit.motion-word-backward]
// [spec:posix:req:edit.motion-word-end]
// [spec:posix:req:edit.motion-word-forward]
// [spec:posix:req:edit.put-save-buffer]
// [spec:posix:req:edit.replace-char]
// [spec:posix:req:edit.set-o-vi]
// [spec:posix:req:edit.sigint-command-mode]
// [spec:posix:def:edit.stty-characters]
// [spec:posix:req:edit.up-option]
// [spec:posix:req:edit.vi-mode-editing]
// [spec:posix:def:edit.word-bigword-terms]
// [spec:posix:req:edit.yank-motion]

use nshedit::domain::{
    Action, ArgumentCommand, Binding, CommandName, CommandSequence, Direction, EditTarget,
    EditingMode, EditorConfig, EffectCommand, HistorySearchCommand, ImmediateCommand, InputMode,
    KeySequence, KeymapMode, Motion, Outcome, Prompt, Refresh, ScreenSize, SignalPolicy,
    TerminalLiteral, Text, TextUnit, WordTraversal, YankPlacement,
};
use nshedit::editor::effect::{
    AliasResponse, HistoryResponse, HistorySearchInput, HistorySearchResponse, HistorySelection,
    HostFailure, PromptSide, ReadEffect, ReadOutcome,
};
use nshedit::editor::{
    CompletionCandidate, DriverError, Editor, ReadDriver, ReadResult, ReadStep, StartError,
    SystemTerminal, TerminalProfile,
};
use nshedit::history::HistoryCursor;
use nshedit_plat::terminal::{ControlCharacter, TerminalAttributes};
use std::error::Error as StdError;
use std::ffi::{CStr, OsStr, OsString};
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::os::fd::{AsRawFd, BorrowedFd, RawFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

type NativeEditor = Editor<SystemTerminal<'static>>;

const FIRST_NONBLANK: &str = "nsh-vi-first-nonblank";
const DELETE_TO_FIRST_NONBLANK: &str = "nsh-vi-delete-to-first-nonblank";
const CHANGE_TO_FIRST_NONBLANK: &str = "nsh-vi-change-to-first-nonblank";
const YANK_TO_FIRST_NONBLANK: &str = "nsh-vi-yank-to-first-nonblank";

mod history;
pub use history::{History, HistoryError, HistoryEvent};

/// A native editor/session integration error.
#[derive(Debug)]
pub enum LineEditorError {
    Io(io::Error),
    Start(StartError),
    Driver(DriverError),
    Domain(nshedit::domain::Error),
    TerminalProfile(Box<str>),
    OpaqueCodePoint,
}

impl fmt::Display for LineEditorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::Start(error) => error.fmt(formatter),
            Self::Driver(error) => error.fmt(formatter),
            Self::Domain(error) => error.fmt(formatter),
            Self::TerminalProfile(error) => formatter.write_str(error),
            Self::OpaqueCodePoint => {
                formatter.write_str("editor returned a non-terminal opaque code point")
            }
        }
    }
}

impl StdError for LineEditorError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Start(error) => Some(error),
            Self::Driver(error) => Some(error),
            Self::Domain(error) => Some(error),
            Self::TerminalProfile(_) | Self::OpaqueCodePoint => None,
        }
    }
}

impl From<io::Error> for LineEditorError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<StartError> for LineEditorError {
    fn from(error: StartError) -> Self {
        Self::Start(error)
    }
}

impl From<DriverError> for LineEditorError {
    fn from(error: DriverError) -> Self {
        Self::Driver(error)
    }
}

impl From<nshedit::domain::Error> for LineEditorError {
    fn from(error: nshedit::domain::Error) -> Self {
        Self::Domain(error)
    }
}

/// One interactive editor with its terminal and stream capabilities.
pub struct LineEditor {
    /// Taken and finished before the descriptor-owning files are dropped.
    editor: Option<NativeEditor>,
    driver: ReadDriver,
    input: File,
    output: File,
    input_fd: RawFd,
    output_fd: RawFd,
    history_cursor: HistoryCursor,
    live_history_line: Option<Text>,
    last_history_pattern: Option<Text>,
    pending_line: Vec<u8>,
    pending_offset: usize,
}

impl LineEditor {
    /// Duplicate the shell-owned descriptors and activate a native session.
    ///
    /// # Safety
    /// `input_fd` and `output_fd` must be live descriptors for the duration of
    /// this call.  The constructed value owns duplicates thereafter.
    pub unsafe fn new(
        input_fd: RawFd,
        output_fd: RawFd,
        mode: EditingMode,
    ) -> Result<Self, LineEditorError> {
        let input = duplicate_file(input_fd)?;
        let output = duplicate_file(output_fd)?;
        let owned_input_fd = input.as_raw_fd();
        let owned_output_fd = output.as_raw_fd();

        // SAFETY: the two `File`s remain fields of `LineEditor`.  `Drop`
        // takes and finishes the editor before either file can close its fd.
        // BorrowedFd stores the descriptor number, not an address into File,
        // so moving LineEditor does not invalidate the borrow.
        let terminal_input: BorrowedFd<'static> = BorrowedFd::borrow_raw(owned_input_fd);
        let terminal_output: BorrowedFd<'static> = BorrowedFd::borrow_raw(owned_output_fd);
        let terminal_attributes = nshedit_plat::terminal::read_attributes(terminal_input).ok();
        let config = EditorConfig::default()
            .with_editing_mode(mode)
            .with_signal_policy(SignalPolicy::Ignore);
        let mut editor = Editor::new(config, SystemTerminal::new(terminal_input, terminal_output))?;
        let size = SystemTerminal::screen_size(terminal_output)
            .unwrap_or_else(|_| ScreenSize::new(24, 80).expect("the fallback screen is valid"));
        editor.configure_display(default_terminal_profile(), size);
        install_shell_bindings(&mut editor, terminal_attributes.as_ref())?;

        Ok(Self {
            editor: Some(editor),
            driver: ReadDriver::default(),
            input,
            output,
            input_fd: owned_input_fd,
            output_fd: owned_output_fd,
            history_cursor: HistoryCursor::new(),
            live_history_line: None,
            last_history_pattern: None,
            pending_line: Vec::new(),
            pending_offset: 0,
        })
    }

    pub fn set_mode(&mut self, mode: EditingMode) {
        let editor = self.editor_mut();
        editor.reconfigure(editor.config().with_editing_mode(mode));
    }

    pub fn set_terminal(&mut self, name: &[u8]) -> Result<(), LineEditorError> {
        let name = core::str::from_utf8(name).map_err(|error| {
            LineEditorError::TerminalProfile(error.to_string().into_boxed_str())
        })?;
        let size = self.screen_size();
        let profile = match nshterm::TermInfo::from_name(name) {
            Ok(entry) => TerminalProfile::from_terminfo(&entry),
            Err(error) => {
                self.editor_mut()
                    .configure_display(TerminalProfile::plain(), size);
                return Err(LineEditorError::TerminalProfile(
                    error.to_string().into_boxed_str(),
                ));
            }
        };
        self.editor_mut().configure_display(profile, size);
        Ok(())
    }

    /// Fill a parser buffer from the current edited line, retaining any tail
    /// that did not fit for the next call.
    ///
    /// # Safety
    /// Prompt, variable and editor effects call into the shell's legacy
    /// single-threaded global state.
    pub unsafe fn read_into(
        &mut self,
        sh: &mut crate::context::Shell,
        history: &mut History,
        destination: &mut [u8],
    ) -> Result<usize, LineEditorError> {
        if destination.is_empty() {
            return Ok(0);
        }
        if self.pending_offset == self.pending_line.len() {
            self.pending_line.clear();
            self.pending_offset = 0;
            /* A `catch_unwind` sat here, to put the terminal back into
             * cooked mode before re-raising. What it was really catching
             * was `onint`'s longjmp leaving the line editor on a ^C --
             * `errors-are-values` made that an ordinary `Err` return, and
             * unpinned `panic = "unwind"`, under which a panic does not
             * unwind and the guard could never have run. Dead code that
             * looks live is worse than none. */
            let line = self.drive_line(sh, history)?;
            let Some(mut line) = line else {
                return Ok(0);
            };
            line.push(b'\n');
            self.pending_line = line;
        }
        let available = &self.pending_line[self.pending_offset..];
        let count = available.len().min(destination.len());
        destination[..count].copy_from_slice(&available[..count]);
        self.pending_offset += count;
        Ok(count)
    }

    unsafe fn drive_line(
        &mut self,
        sh: &mut crate::context::Shell,
        history: &mut History,
    ) -> Result<Option<Vec<u8>>, LineEditorError> {
        self.editor_mut().reset_line();
        self.history_cursor.reset();
        self.live_history_line = None;
        let mut step = {
            let (editor, driver) = self.editor_and_driver();
            driver.begin(editor)?
        };
        loop {
            step = match step {
                ReadStep::Prompt(pending) => {
                    let prompt = match pending.request().side {
                        PromptSide::Left => shell_prompt(sh),
                        PromptSide::Right => Prompt::default(),
                    };
                    let (editor, driver) = self.editor_and_driver();
                    driver.resume_prompt(editor, &pending, Ok(prompt))?
                }
                ReadStep::Resize(pending) => {
                    let response = SystemTerminal::screen_size(self.output_borrowed())
                        .map_err(|_| HostFailure::Unavailable);
                    let (editor, driver) = self.editor_and_driver();
                    driver.resume_resize(editor, &pending, response)?
                }
                ReadStep::Read(pending) => {
                    let response = self.read_effect(*pending.request());
                    let (editor, driver) = self.editor_and_driver();
                    driver.resume_read(editor, &pending, response)?
                }
                ReadStep::History(pending) => {
                    let request = pending.request();
                    let direction = request.direction;
                    let count = request.count.get();
                    let edited_line = self.editor_mut().line().clone();
                    match history.current_editor_text(&self.history_cursor) {
                        Some(selected) if selected != edited_line => {
                            self.live_history_line = Some(edited_line);
                        }
                        None if direction == Direction::Previous => {
                            self.live_history_line = Some(edited_line);
                        }
                        _ => {}
                    }
                    let mode = self.editor_mut().config().editing_mode();
                    let mut response =
                        history.navigate_editor(&mut self.history_cursor, direction, count, mode);
                    if matches!(response.selection(), HistorySelection::Live)
                        && let Some(line) = self.live_history_line.clone()
                    {
                        response = if response.reached_boundary() {
                            HistoryResponse::entry(line).at_boundary()
                        } else {
                            HistoryResponse::entry(line)
                        };
                    }
                    let (editor, driver) = self.editor_and_driver();
                    driver.resume_history(editor, &pending, Ok(response))?
                }
                ReadStep::HistorySearch(pending) => {
                    let request = pending.request();
                    let direction = request.direction;
                    let matching = request.matching;
                    let input = request.input.clone();
                    let mut live_line = None;
                    let pattern = match input {
                        HistorySearchInput::Pattern(pattern) => Ok(pattern),
                        HistorySearchInput::Prompted => {
                            live_line = Some(self.editor_mut().line().clone());
                            let prompt = match direction {
                                Direction::Previous => Text::from("\n/"),
                                Direction::Next => Text::from("\n?"),
                            };
                            self.read_host_text(&prompt, true).and_then(|pattern| {
                                if pattern.is_empty() {
                                    self.last_history_pattern
                                        .clone()
                                        .ok_or(HostFailure::Cancelled)
                                } else {
                                    self.last_history_pattern = Some(pattern.clone());
                                    Ok(pattern)
                                }
                            })
                        }
                        HistorySearchInput::Incremental(_) => {
                            let prompt = match direction {
                                Direction::Previous => Text::from("\nbck: "),
                                Direction::Next => Text::from("\nfwd: "),
                            };
                            self.read_host_text(&prompt, true)
                        }
                    };
                    let response = match pattern {
                        Ok(pattern) => {
                            let mut selection = history.search_editor(
                                &mut self.history_cursor,
                                &pattern,
                                direction,
                                matching,
                            );
                            if matches!(selection.selection(), HistorySelection::Unchanged)
                                && let Some(line) = live_line
                            {
                                selection = HistoryResponse::entry(line).at_boundary();
                            }
                            Ok(HistorySearchResponse {
                                history: selection,
                                pattern,
                            })
                        }
                        Err(HostFailure::Cancelled) if live_line.is_some() => {
                            Ok(HistorySearchResponse {
                                history: HistoryResponse::entry(
                                    live_line.expect("checked prompted-line snapshot"),
                                )
                                .at_boundary(),
                                pattern: Text::default(),
                            })
                        }
                        Err(error) => Err(error),
                    };
                    let (editor, driver) = self.editor_and_driver();
                    driver.resume_history_search(editor, &pending, response)?
                }
                ReadStep::HistoryLine(pending) => {
                    let response = history
                        .select_editor_line(&mut self.history_cursor, pending.request().position());
                    let (editor, driver) = self.editor_and_driver();
                    driver.resume_history_line(editor, &pending, Ok(response))?
                }
                ReadStep::HistoryWord(pending) => {
                    let response = history.newest_word(pending.request().position);
                    let (editor, driver) = self.editor_and_driver();
                    driver.resume_history_word(editor, &pending, Ok(response))?
                }
                ReadStep::Alias(pending) => {
                    let enter_insert = self.editor_mut().keymap_mode() == KeymapMode::ViCommand;
                    let response = shell_alias(sh, &pending.request().name, enter_insert);
                    let (editor, driver) = self.editor_and_driver();
                    driver.resume_alias(editor, &pending, response)?
                }
                ReadStep::EditorCommand(pending) => {
                    let (editor, driver) = self.editor_and_driver();
                    driver.resume_editor_command(editor, &pending, Err(HostFailure::Unavailable))?
                }
                ReadStep::ExternalEdit(pending) => {
                    let response = self.external_edit(&pending.request().line);
                    let (editor, driver) = self.editor_and_driver();
                    driver.resume_external_edit(editor, &pending, response)?
                }
                ReadStep::RecordHistory(pending) => {
                    // `input::preadbuffer` is authoritative: it knows whether
                    // this is H_ENTER or a multiline H_APPEND.
                    self.history_cursor.reset();
                    let (editor, driver) = self.editor_and_driver();
                    driver.resume_history_record(editor, &pending, Ok(()))?
                }
                ReadStep::Completion(pending) => {
                    let response = Ok(completion_candidates(&pending.request().query));
                    let (editor, driver) = self.editor_and_driver();
                    driver.resume_completion(editor, &pending, response)?
                }
                ReadStep::UserCommand(pending) => {
                    let response =
                        shell_user_command(self.editor_mut(), pending.request().name.as_str());
                    let (editor, driver) = self.editor_and_driver();
                    driver.resume_user_command(editor, &pending, response)?
                }
                ReadStep::Signal(pending) => {
                    let (editor, driver) = self.editor_and_driver();
                    driver.resume_signal(editor, &pending, Ok(()))?
                }
                ReadStep::Display(display) => {
                    let editor = self.editor.as_mut().expect("live native editor");
                    self.driver.display(editor, &display, &mut self.output)?
                }
                ReadStep::Complete(result) => {
                    let result = match result {
                        ReadResult::Accepted(line) => Some(text_to_bytes(&line)?),
                        ReadResult::Character(unit) => {
                            Some(text_to_bytes(&core::iter::once(unit).collect())?)
                        }
                        ReadResult::Interrupted(_) => {
                            self.discard_display_image();
                            None
                        }
                        ReadResult::Command | ReadResult::EndOfInput => None,
                    };
                    self.editor_mut().reset_line();
                    self.history_cursor.reset();
                    return Ok(result);
                }
            };
        }
    }

    fn read_effect(&mut self, purpose: ReadEffect) -> Result<ReadOutcome, HostFailure> {
        if purpose == ReadEffect::KeySequence
            && SystemTerminal::bytes_ready(self.input_borrowed()).unwrap_or(0) == 0
        {
            return Ok(ReadOutcome::TimedOut);
        }
        let mut byte = [0];
        match self.input.read(&mut byte) {
            Ok(0) => Ok(ReadOutcome::EndOfInput),
            Ok(_) => Ok(ReadOutcome::Bytes(byte.into())),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                Err(HostFailure::Interrupted)
            }
            Err(error) => Err(host_failure(error)),
        }
    }

    fn read_host_text(
        &mut self,
        prompt: &Text,
        cancel_on_escape: bool,
    ) -> Result<Text, HostFailure> {
        self.output
            .write_all(&text_to_bytes(prompt).map_err(host_failure)?)
            .and_then(|()| self.output.flush())
            .map_err(host_failure)?;
        let mut bytes = Vec::new();
        loop {
            let mut byte = [0];
            match self.input.read(&mut byte) {
                Ok(0) => return Err(HostFailure::Cancelled),
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                    return Err(HostFailure::Interrupted);
                }
                Err(error) => return Err(host_failure(error)),
            }
            match byte[0] {
                b'\r' | b'\n' => {
                    self.output.write_all(b"\n").map_err(host_failure)?;
                    return Ok(text_from_bytes(&bytes));
                }
                0x1b if cancel_on_escape => return Err(HostFailure::Cancelled),
                0x07 => return Err(HostFailure::Cancelled),
                0x08 | 0x7f => {
                    if bytes.pop().is_some() {
                        self.output.write_all(b"\x08 \x08").map_err(host_failure)?;
                    }
                }
                byte if bytes.len() == 4096 => {
                    let _ = byte;
                    return Err(HostFailure::Failed("command input is too long".into()));
                }
                byte => {
                    bytes.push(byte);
                    self.output.write_all(&[byte]).map_err(host_failure)?;
                }
            }
        }
    }

    fn external_edit(&mut self, line: &Text) -> Result<Text, HostFailure> {
        static NEXT_EDIT_FILE: AtomicU64 = AtomicU64::new(0);
        let serial = NEXT_EDIT_FILE.fetch_add(1, Ordering::Relaxed);
        let mut path = std::env::temp_dir();
        path.push(format!("nsh-edit-{}-{serial}.tmp", std::process::id()));
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(host_failure)?;
        let result = (|| {
            file.write_all(&text_to_bytes(line).map_err(host_failure)?)
                .and_then(|()| file.write_all(b"\n"))
                .and_then(|()| file.flush())
                .map_err(host_failure)?;
            let editor = unsafe { shell_editor() };
            Command::new(editor)
                .arg(&path)
                .status()
                .map_err(host_failure)?;
            file.rewind().map_err(host_failure)?;
            let mut edited = Vec::new();
            file.read_to_end(&mut edited).map_err(host_failure)?;
            if edited.last() == Some(&b'\n') {
                edited.pop();
            }
            Ok(text_from_bytes(&edited))
        })();
        drop(file);
        let _ = std::fs::remove_file(path);
        result
    }

    fn editor_mut(&mut self) -> &mut NativeEditor {
        self.editor.as_mut().expect("live native editor")
    }

    fn editor_and_driver(&mut self) -> (&mut NativeEditor, &mut ReadDriver) {
        (
            self.editor.as_mut().expect("live native editor"),
            &mut self.driver,
        )
    }

    fn input_borrowed(&self) -> BorrowedFd<'_> {
        // SAFETY: `self.input` owns this descriptor.
        unsafe { BorrowedFd::borrow_raw(self.input_fd) }
    }

    fn output_borrowed(&self) -> BorrowedFd<'_> {
        // SAFETY: `self.output` owns this descriptor.
        unsafe { BorrowedFd::borrow_raw(self.output_fd) }
    }

    fn screen_size(&self) -> ScreenSize {
        SystemTerminal::screen_size(self.output_borrowed())
            .unwrap_or_else(|_| ScreenSize::new(24, 80).expect("the fallback screen is valid"))
    }

    /// Host signal handling can print a newline while unwinding past the
    /// driver.  Its committed frame then no longer describes the terminal;
    /// reinstalling the owned profile makes the next prompt a full frame.
    fn discard_display_image(&mut self) {
        let size = self.screen_size();
        let profile = self.editor_mut().terminal_profile().cloned();
        if let Some(profile) = profile {
            self.editor_mut().configure_display(profile, size);
        }
    }
}

impl Drop for LineEditor {
    fn drop(&mut self) {
        if let Some(editor) = self.editor.take() {
            let _ = editor.finish();
        }
    }
}

unsafe fn duplicate_file(fd: RawFd) -> io::Result<File> {
    let borrowed = BorrowedFd::borrow_raw(fd);
    let owned = borrowed.try_clone_to_owned()?;
    Ok(File::from(owned))
}

fn default_terminal_profile() -> TerminalProfile {
    std::env::var("TERM")
        .ok()
        .and_then(|name| nshterm::TermInfo::from_name(&name).ok())
        .map(|entry| TerminalProfile::from_terminfo(&entry))
        .unwrap_or_else(TerminalProfile::ansi)
}

fn install_shell_bindings(
    editor: &mut NativeEditor,
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
        (
            "^",
            Binding::User(
                CommandName::new(FIRST_NONBLANK).expect("static shell command name is valid"),
            ),
        ),
        (":", Binding::Effect(EffectCommand::ReadEditorCommand)),
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
        ("U", Binding::Effect(EffectCommand::RestoreHistoryLine)),
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

fn install_terminal_character(
    editor: &mut NativeEditor,
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

unsafe fn shell_alias(sh: &mut crate::context::Shell, name: &Text, enter_insert: bool) -> Result<AliasResponse, HostFailure> {
    let mut name = text_to_bytes(name).map_err(host_failure)?;
    if name.contains(&0) {
        return Err(HostFailure::Failed(
            "an editor alias name contains NUL".into(),
        ));
    }
    name.push(0);
    let alias = crate::alias::lookupalias(sh, name.as_ptr().cast(), 0);
    if alias.is_null() {
        return Ok(AliasResponse::Missing);
    }
    let expansion = CStr::from_ptr((*alias).val).to_bytes();
    let mut macro_text = Text::default();
    // POSIX `@letter` inserts ordinary alias text.  Starting the native
    // macro in Vi insertion mode gives embedded escape sequences and later
    // command keys their normal editor meaning without baking shell policy
    // into nshedit's generic alias effect.
    if enter_insert {
        macro_text.push(TextUnit::Scalar('i'));
    }
    macro_text.extend(text_from_bytes(expansion).as_units().iter().copied());
    Ok(AliasResponse::Expansion(macro_text))
}

fn shell_user_command(editor: &mut NativeEditor, name: &str) -> Result<Outcome, HostFailure> {
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
        _ => Err(HostFailure::Unavailable),
    }
}

fn first_nonblank_index(line: &Text) -> usize {
    line.as_units()
        .iter()
        .position(|unit| !is_line_blank(unit))
        .unwrap_or(0)
}

fn is_line_blank(unit: &TextUnit) -> bool {
    matches!(
        unit,
        TextUnit::Scalar(' ' | '\t') | TextUnit::RawByte(b' ' | b'\t')
    )
}

unsafe fn shell_prompt(sh: &mut crate::context::Shell) -> Prompt {
    let pointer = crate::parser::getprompt(sh);
    if pointer.is_null() {
        return Prompt::default();
    }
    prompt_from_text(&text_from_bytes(CStr::from_ptr(pointer).to_bytes()), 0x01)
}

fn prompt_from_text(text: &Text, escape: u32) -> Prompt {
    let marker = TextUnit::from_code_point(escape);
    let mut prompt = Prompt::default();
    let mut literal = false;
    for part in text.as_units().split(|unit| *unit == marker) {
        if literal {
            let bytes = part.iter().copied().collect::<Text>();
            let bytes = text_to_bytes(&bytes).unwrap_or_default();
            prompt.push_literal(TerminalLiteral::from(bytes));
        } else {
            prompt.push_text(part.iter().copied().collect::<Text>());
        }
        literal = !literal;
    }
    prompt
}

fn text_from_bytes(bytes: &[u8]) -> Text {
    let mut text = Text::default();
    let mut remaining = bytes;
    while !remaining.is_empty() {
        match core::str::from_utf8(remaining) {
            Ok(valid) => {
                text.extend(valid.chars().map(TextUnit::Scalar));
                break;
            }
            Err(error) => {
                let valid = &remaining[..error.valid_up_to()];
                text.extend(
                    core::str::from_utf8(valid)
                        .expect("valid_up_to identifies valid UTF-8")
                        .chars()
                        .map(TextUnit::Scalar),
                );
                remaining = &remaining[error.valid_up_to()..];
                let invalid = error.error_len().unwrap_or(remaining.len());
                text.extend(remaining[..invalid].iter().copied().map(TextUnit::RawByte));
                remaining = &remaining[invalid..];
            }
        }
    }
    text
}

fn text_to_bytes(text: &Text) -> Result<Vec<u8>, LineEditorError> {
    let mut bytes = Vec::new();
    for unit in text {
        match unit {
            TextUnit::Scalar(character) => {
                let mut encoded = [0; 4];
                bytes.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
            }
            TextUnit::RawByte(byte) => bytes.push(*byte),
            TextUnit::OpaqueCodePoint(_) => return Err(LineEditorError::OpaqueCodePoint),
        }
    }
    Ok(bytes)
}

fn host_failure(error: impl fmt::Display) -> HostFailure {
    let message = error.to_string();
    HostFailure::Failed(message.into_boxed_str())
}

fn completion_candidates(
    query: &nshedit::editor::CompletionQuery,
) -> nshedit::editor::CompletionCandidates {
    let Ok(stem) = text_to_bytes(query.stem()) else {
        return Vec::new().into();
    };
    let split = stem
        .iter()
        .rposition(|byte| *byte == b'/')
        .map_or((b"".as_slice(), stem.as_slice()), |position| {
            (&stem[..=position], &stem[position + 1..])
        });
    let (prefix, basename) = split;
    let directory = if prefix.is_empty() {
        PathBuf::from(".")
    } else if prefix == b"/" {
        PathBuf::from("/")
    } else {
        PathBuf::from(OsStr::from_bytes(&prefix[..prefix.len() - 1]))
    };
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new().into();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.as_bytes();
            if !name.starts_with(basename) {
                return None;
            }
            let mut insertion = prefix.to_vec();
            insertion.extend_from_slice(name);
            let suffix = entry
                .file_type()
                .ok()
                .filter(|kind| kind.is_dir())
                .map_or(" ", |_| "/");
            Some(CompletionCandidate::new(text_from_bytes(&insertion)).with_suffix(suffix))
        })
        .collect()
}

unsafe fn shell_editor() -> OsString {
    for name in [c"EDITOR", c"VISUAL"] {
        let value = crate::var::bltinlookup(name.as_ptr());
        if !value.is_null() {
            let bytes = CStr::from_ptr(value).to_bytes();
            if !bytes.is_empty() {
                return OsString::from_vec(bytes.to_vec());
            }
        }
    }
    OsString::from("vi")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_literals_do_not_contribute_columns() {
        let prompt = prompt_from_text(&text_from_bytes(b"x\x01\x1b[31m\x01> "), 1);
        assert_eq!(prompt.parts().len(), 3);
    }

    #[test]
    fn first_nonblank_preserves_raw_bytes() {
        assert_eq!(first_nonblank_index(&Text::from(" \tword")), 2);
        assert_eq!(first_nonblank_index(&text_from_bytes(b" \t\xffword")), 2);
        assert_eq!(first_nonblank_index(&Text::default()), 0);
    }
}
