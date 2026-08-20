//! Shell-owned interactive editing and history policy.
//! Rules: `docs/spec/port/src/histedit.md`, `docs/spec/port/src/myhistedit.md`.
//!
//! `nsh` owns one semantic history store and, while editing is enabled, one
//! native [`nshedit`] session.  The integration has no libedit-shaped shim:
//! lifecycle, mode changes, reads, history insertion, and `fc` selection are
//! ordinary Rust operations over owned values.
//!
//! # Cross-module signatures assumed (see the port report)
//!
//!   * `crate::error::{jmploc, jmp_buf, handler, setjmp, longjmp,
//!     sh_error, suppressint, intpending, onint}`
//!   * `crate::output::{stderr, stdout}`
//!   * `crate::options::{ShellOption, arg0, optionarg}`
//!   * `crate::var::{bltinlookup, histsizeval}`
//!   * `crate::number::parse_decimal`
//!   * `crate::eval::evalstring`
//!   * `crate::parser::getprompt`
//!   * `crate::runtime::readcmdfile` (src/main.c:283)

use bstr::{BStr, BString};
use nsh_platform::ShellBytesExt as _;
use nshedit::domain::EditingMode;
use std::fs::File;
use std::io::{Read as _, Seek as _, Write};

use super::{History, LineEditor, LineEditorError, declared_terminal_supports_line_editing};
use crate::fd::LogicalDescriptor;
use crate::options::ShellOption;
// [spec:nsh:def:idiom.shell-options]

const DEFAULT_HISTORY_SIZE: usize = 128;

/// `#include <sys/param.h>` — MAXPATHLEN.

// The old myhistedit typedefs now map to owned semantic fields in this state.
// [spec:dash:def:myhistedit.history]
// [spec:dash:def:myhistedit.edit-line]
// [spec:dash:def:myhistedit.hist-event]
pub(crate) struct HistEditState {
    history: Option<History>,
    history_file: Option<File>,
    editor: Option<LineEditor>,
    pub(crate) fc_depth: usize,
}

impl HistEditState {
    pub(crate) fn new() -> Self {
        Self {
            history: None,
            history_file: None,
            editor: None,
            fc_depth: 0,
        }
    }

    // [spec:posix:req:param.ps1-exclamation-expansion]
    pub(crate) fn expand_prompt_exclamation_marks(&self, prompt: &BStr) -> BString {
        let number = self
            .history
            .as_ref()
            .and_then(History::next_number)
            .unwrap_or(1)
            .to_string();
        let mut expanded = Vec::with_capacity(prompt.len());
        let mut index = 0;
        while index < prompt.len() {
            if prompt[index] != b'!' {
                expanded.push(prompt[index]);
                index += 1;
            } else if prompt.get(index + 1) == Some(&b'!') {
                expanded.push(b'!');
                index += 2;
            } else {
                expanded.extend_from_slice(number.as_bytes());
                index += 1;
            }
        }
        BString::from(expanded)
    }
}

// [spec:dash:def:myhistedit.history-fn]
// [spec:dash:sem:myhistedit.history-fn]
#[inline]
pub(crate) fn history_mut(sh: &mut crate::context::Shell) -> Option<&mut History> {
    sh.histedit.history.as_mut()
}

#[must_use]
pub fn history_active(sh: &crate::context::Shell) -> bool {
    sh.histedit.history.is_some()
}

#[must_use]
pub fn editing_active(sh: &crate::context::Shell) -> bool {
    sh.histedit.editor.is_some()
}

/// Read edited bytes directly into the parser's owned input buffer.
pub fn read_edit_line(
    sh: &mut crate::context::Shell,
    destination: &mut [u8],
) -> Result<usize, LineEditorError> {
    let Some(mut editor) = sh.histedit.editor.take() else {
        return Ok(0);
    };
    let Some(mut history) = sh.histedit.history.take() else {
        sh.histedit.editor = Some(editor);
        return Ok(0);
    };

    // The editor asks the shell for prompts and aliases while it reads, so
    // neither editor nor history may remain borrowed through `sh` here.
    let result = editor.read_into(sh, &mut history, destination);
    sh.histedit.history = Some(history);
    sh.histedit.editor = Some(editor);
    result
}

/// Retain one physical input line, either starting or continuing a command.
// [spec:posix:req:edit.history-list]
pub fn record_history_line(
    sh: &mut crate::context::Shell,
    bytes: &[u8],
    first: bool,
    from_input: bool,
) {
    let recorded = {
        let Some(history) = history_mut(sh) else {
            return;
        };
        if first {
            history.enter(bytes, from_input).is_ok()
        } else {
            history.append(bytes)
        }
    };
    if !recorded {
        // History is optional; a failed store is disabled without failing the command.
        sh.histedit.history = None;
        sh.histedit.history_file = None;
        return;
    }
    save_history(sh);
}

/*
 * Set history and editing status.  Called whenever the status may
 * have changed (figures out what to do).
 */
// [spec:dash:def:histedit.histedit-fn]
// [spec:dash:sem:histedit.histedit-fn]
// [spec:dash:def:myhistedit.histedit-fn]
// [spec:dash:sem:myhistedit.histedit-fn]
// [spec:posix:req:builtin.fc.env-histfile-initialization]
// [spec:posix:req:builtin.fc.env-histfile-sharing-and-deletion]
// [spec:posix:req:builtin.fc.env-histfile]
// [spec:posix:req:edit.history-list]
// [spec:nsh:req:interactive.default-history-navigation]
pub fn histedit(sh: &mut crate::context::Shell) {
    if sh.options.enabled(ShellOption::Interactive) {
        if !history_active(sh) {
            crate::error::with_interrupts_deferred(sh, |sh| {
                sh.histedit.history = Some(History::new());
                sh.histedit.history_file = crate::var::lookup_bytes(sh, BStr::new(b"HISTFILE"))
                    .filter(|name| !name.is_empty())
                    .and_then(|name| {
                        let Ok(path) = name.try_to_path_buf() else {
                            return None;
                        };
                        let Ok(file) = nsh_platform::open_history_file(&path) else {
                            return None;
                        };
                        let Ok(duplicate) =
                            nsh_platform::duplicate_cloexec(&file, LogicalDescriptor::COUNT as i32)
                        else {
                            return None;
                        };
                        Some(duplicate.into_file())
                    });
            });
            /* Hoisted out of the argument list, which also takes the
             * shell; see the note in `eval.rs`'s `evalcommand`. */
            let size = crate::var::histsizeval(sh);
            sethistsize(sh, BStr::new(size.as_slice()));
            let mut saved = Vec::new();
            let file_ready = match sh.histedit.history_file.as_mut() {
                Some(file) => {
                    file.rewind().is_ok()
                        && file.read_to_end(&mut saved).is_ok()
                        && file.rewind().is_ok()
                }
                None => true,
            };
            if !file_ready {
                saved.clear();
                sh.histedit.history_file = None;
            }
            let mut load_failed = false;
            if let Some(history) = history_mut(sh) {
                for line in saved.split_inclusive(|byte| *byte == b'\n') {
                    if !line.is_empty() && history.enter(line, false).is_err() {
                        load_failed = true;
                        break;
                    }
                }
            }
            if load_failed {
                // A broken optional history store must not affect shell startup.
                sh.histedit.history = None;
                sh.histedit.history_file = None;
            }
        }

        // [spec:nsh:def:idiom.logical-descriptors]
        let stdin = sh.fds.get(LogicalDescriptor::STDIN);
        let stderr = sh.fds.get(LogicalDescriptor::STDERR);
        let mode = if sh.options.enabled(ShellOption::Vi) {
            Some(EditingMode::Vi)
        } else if sh.options.enabled(ShellOption::Emacs)
            || declared_terminal_supports_line_editing()
        {
            // Every native editor needs a command family. Emacs is nshedit's
            // insertion-oriented baseline; selecting it here does not mutate
            // the shell's `emacs` option.
            Some(EditingMode::Emacs)
        } else {
            None
        };

        if let Some(mode) = mode
            && !editing_active(sh)
            && stdin.as_ref().is_some_and(nsh_platform::is_terminal)
        {
            crate::error::with_interrupts_deferred(sh, |sh| {
                let editor = match (stdin.as_ref(), stderr.as_ref()) {
                    (Some(input), Some(output)) => LineEditor::new(&sh.locale, input, output, mode),
                    _ => Err(std::io::Error::from(std::io::ErrorKind::NotConnected).into()),
                };
                match editor {
                    Ok(editor) => sh.histedit.editor = Some(editor),
                    Err(_) => {
                        sh.histedit.editor = None;
                        if sh
                            .io
                            .stderr()
                            .write_all(b"sh: can't initialize editing\n")
                            .is_err()
                        {
                            // There is no alternate channel for an editor-startup warning.
                        }
                    }
                }
            });
        } else if mode.is_none() && editing_active(sh) {
            crate::error::with_interrupts_deferred(sh, |sh| {
                sh.histedit.editor = None;
            });
        }

        if let (Some(mode), Some(editor)) = (mode, sh.histedit.editor.as_mut()) {
            editor.set_mode(mode);
        }
    } else {
        crate::error::with_interrupts_deferred(sh, |sh| {
            sh.histedit.editor = None;
            sh.histedit.history = None;
            sh.histedit.history_file = None;
        });
    }
}

/// Persist a root interactive shell's retained history, if its configured
/// file remained readable and writable. Failure leaves in-memory operation
/// unaffected, as POSIX requires when the file cannot be used.
pub(crate) fn save_history(sh: &mut crate::context::Shell) {
    if sh.shell_level != 0 {
        return;
    }
    let Some(history) = sh.histedit.history.as_ref() else {
        return;
    };
    let contents = history.file_contents();
    let Some(file) = sh.histedit.history_file.as_mut() else {
        return;
    };
    if file.set_len(0).is_err() || file.rewind().is_err() {
        return;
    }
    if file.write_all(&contents).is_err() || file.flush().is_err() {
        // Persistence is optional; retained in-memory history remains usable.
    }
}

// [spec:dash:def:histedit.sethistsize-fn]
// [spec:dash:sem:histedit.sethistsize-fn]
// [spec:dash:def:myhistedit.sethistsize-fn]
// [spec:dash:sem:myhistedit.sethistsize-fn]
// [spec:posix:req:builtin.fc.env-histsize]
pub fn sethistsize(sh: &mut crate::context::Shell, hs: &BStr) {
    let histsize = if hs.is_empty() {
        DEFAULT_HISTORY_SIZE
    } else {
        let mut input = hs
            .iter()
            .copied()
            .skip_while(|byte| sh.locale.is_space(*byte));
        let negative = matches!(input.clone().next(), Some(b'-'));
        if matches!(input.clone().next(), Some(b'-' | b'+')) {
            input.next();
        }
        let value = input
            .take_while(u8::is_ascii_digit)
            .fold(0usize, |value, digit| {
                value
                    .saturating_mul(10)
                    .saturating_add((digit - b'0') as usize)
            });
        if negative && value != 0 {
            DEFAULT_HISTORY_SIZE
        } else {
            value
        }
    };
    let Some(history) = history_mut(sh) else {
        return;
    };
    history.set_limit(histsize);
}

// [spec:dash:def:histedit.setterm-fn]
// [spec:dash:sem:histedit.setterm-fn]
// [spec:dash:def:myhistedit.setterm-fn]
// [spec:dash:sem:myhistedit.setterm-fn]
pub fn setterm(sh: &mut crate::context::Shell, term: &BStr) {
    let Some(editor) = sh.histedit.editor.as_mut() else {
        return;
    };
    if editor.set_terminal(term).is_err() {
        let mut message = b"sh: Can't set terminal type ".to_vec();
        message.extend_from_slice(term);
        message.push(b'\n');
        message.extend_from_slice(b"sh: Using dumb terminal settings.\n");
        if sh.io.stderr().write_all(&message).is_err() {
            // Terminal fallback is already installed; stderr has no fallback sink.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noninteractive_ignores_editor_request() {
        let streams = crate::streams::Streams::capture().unwrap();
        let mut shell = crate::context::Shell::new(streams);
        shell.options.set(ShellOption::Emacs, true);

        histedit(&mut shell);

        assert!(!editing_active(&shell));
        assert!(!history_active(&shell));
    }

    // [spec:posix:req:builtin.fc.env-histfile/test]
    #[test]
    fn history_file_starts_unconfigured() {
        let state = HistEditState::new();
        assert!(state.history_file.is_none());
        assert_eq!(
            state.expand_prompt_exclamation_marks(BStr::new(b"[!][!!][!!!]")),
            BString::from("[1][!][!1]")
        );
    }

    // [spec:posix:req:builtin.fc.env-histsize/test]
    #[test]
    fn unset_histsize_retains_posix_minimum() {
        let mut shell = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        shell.histedit.history = Some(History::new());
        sethistsize(&mut shell, BStr::new(b""));

        let history = shell.histedit.history.as_mut().unwrap();
        for number in 1..=DEFAULT_HISTORY_SIZE {
            history.enter(number.to_string().as_bytes(), false).unwrap();
        }
        assert_eq!(history.len(), DEFAULT_HISTORY_SIZE);

        history.enter(b"newest", false).unwrap();
        assert_eq!(history.len(), DEFAULT_HISTORY_SIZE);
        assert_eq!(history.oldest().unwrap().number, 2);
    }
}
