//! Interactive editing, command history, and the `fc` builtin.
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
//!   * `crate::options::{optlist, arg0, optionarg}`
//!   * `crate::var::{bltinlookup, histsizeval}`
//!   * `crate::mystring::is_number`
//!   * `crate::eval::evalstring`
//!   * `crate::parser::getprompt`
//!   * `crate::shellmain::readcmdfile` (src/main.c:283)

use bstr::BStr;
use core::ffi::{c_char, c_int};
use nshedit::domain::EditingMode;
use std::io::{IsTerminal as _, Write};
use std::os::fd::AsFd as _;

use crate::linedit::{History, LineEditor};

/// `#include <sys/param.h>` — MAXPATHLEN.

// The old myhistedit typedefs now map to owned semantic fields in this state.
// [spec:dash:def:myhistedit.history]
// [spec:dash:def:myhistedit.edit-line]
// [spec:dash:def:myhistedit.hist-event]
pub(crate) struct HistEditState {
    history: Option<History>,
    editor: Option<LineEditor>,
    pub(crate) fc_depth: c_int,
}

impl HistEditState {
    pub(crate) fn new() -> Self {
        Self {
            history: None,
            editor: None,
            fc_depth: 0,
        }
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
) -> Result<usize, crate::linedit::LineEditorError> {
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
pub fn record_history_line(sh: &mut crate::context::Shell, bytes: &[u8], first: bool) {
    let Some(history) = history_mut(sh) else {
        return;
    };
    if first {
        let _ = history.enter(bytes);
    } else {
        let _ = history.append(bytes);
    }
}


// ---------------------------------------------------------------------
// src/options.h:47-63 — the option flags are `#define`s over optlist[].
// ---------------------------------------------------------------------

/// `#define iflag optlist[3]` (src/options.h:50)
#[inline]
fn iflag(sh: &crate::context::Shell) -> c_char {
    sh.options.flag(3)
}

/// `#define Vflag optlist[9]` (src/options.h:56)
#[inline]
fn Vflag(sh: &crate::context::Shell) -> c_char {
    sh.options.flag(9)
}

/// `#define Eflag optlist[10]` (src/options.h:57)
#[inline]
fn Eflag(sh: &crate::context::Shell) -> c_char {
    sh.options.flag(10)
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
pub fn histedit(sh: &mut crate::context::Shell) {
    if iflag(sh) != 0 {
        if !history_active(sh) {
            crate::error::INTOFF(sh);
            sh.histedit.history = Some(History::new());
            crate::error::INTON(sh);
            /* Hoisted out of the argument list, which also takes the
             * shell; see the note in `eval.rs`'s `evalcommand`. */
            let size = crate::var::histsizeval(sh);
            sethistsize(sh, BStr::new(size.as_slice()));
        }

        let stdin = sh.fds.get(0).ok().flatten();
        let stderr = sh.fds.get(2).ok().flatten();
        let mode = if Vflag(sh) != 0 {
            Some(EditingMode::Vi)
        } else if Eflag(sh) != 0 {
            Some(EditingMode::Emacs)
        } else {
            None
        };

        if let Some(mode) = mode
            && !editing_active(sh)
            && stdin.as_ref().is_some_and(|fd| fd.as_fd().is_terminal())
        {
            crate::error::INTOFF(sh);
            let editor = match (stdin.as_ref(), stderr.as_ref()) {
                (Some(input), Some(output)) => {
                    LineEditor::new(&sh.locale, input, output, mode)
                }
                _ => Err(std::io::Error::from(std::io::ErrorKind::NotConnected).into()),
            };
            match editor {
                Ok(editor) => sh.histedit.editor = Some(editor),
                Err(_) => {
                    sh.histedit.editor = None;
                    let _ = sh.io.stderr()
                        .write_all(b"sh: can't initialize editing\n");
                }
            }
            crate::error::INTON(sh);
        } else if mode.is_none() && editing_active(sh) {
            crate::error::INTOFF(sh);
            sh.histedit.editor = None;
            crate::error::INTON(sh);
        }

        if let (Some(mode), Some(editor)) = (mode, sh.histedit.editor.as_mut()) {
            editor.set_mode(mode);
        }
    } else {
        crate::error::INTOFF(sh);
        sh.histedit.editor = None;
        sh.histedit.history = None;
        crate::error::INTON(sh);
    }
}

// [spec:dash:def:histedit.sethistsize-fn]
// [spec:dash:sem:histedit.sethistsize-fn]
// [spec:dash:def:myhistedit.sethistsize-fn]
// [spec:dash:sem:myhistedit.sethistsize-fn]
pub fn sethistsize(sh: &mut crate::context::Shell, hs: &BStr) {
    let histsize = if hs.is_empty() {
        100
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
                value.saturating_mul(10).saturating_add((digit - b'0') as usize)
            });
        if negative && value != 0 { 100 } else { value }
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
    if editor
        .set_terminal(term)
        .is_err()
    {
        let mut message = b"sh: Can't set terminal type ".to_vec();
        message.extend_from_slice(term);
        message.push(b'\n');
        let errors = sh.io.stderr();
        let _ = errors.write_all(&message);
        let _ = errors.write_all(b"sh: Using dumb terminal settings.\n");
    }
}
