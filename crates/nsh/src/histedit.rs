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

use bstr::{BStr, BString};
use core::ffi::CStr;
use core::mem;
use core::ptr;
use libc::{c_char, c_int};
use nshedit::domain::EditingMode;
use std::fs::File;
use std::io::Write;
use std::os::fd::FromRawFd;

use crate::linedit::{History, HistoryEvent, LineEditor};

/// `#include <sys/param.h>` — MAXPATHLEN.

// The old myhistedit typedefs now map to owned semantic fields in this state.
// [spec:dash:def:myhistedit.history]
// [spec:dash:def:myhistedit.edit-line]
// [spec:dash:def:myhistedit.hist-event]
struct HistEditState {
    history: Option<History>,
    editor: Option<LineEditor>,
}

// [spec:dash:def:myhistedit.history-fn]
// [spec:dash:sem:myhistedit.history-fn]
static mut STATE: HistEditState = HistEditState {
    history: None,
    editor: None,
};

#[inline]
unsafe fn state_mut() -> &'static mut HistEditState {
    &mut *ptr::addr_of_mut!(STATE)
}

#[inline]
pub(crate) unsafe fn history_mut() -> Option<&'static mut History> {
    state_mut().history.as_mut()
}

#[must_use]
pub unsafe fn history_active() -> bool {
    (*ptr::addr_of!(STATE)).history.is_some()
}

#[must_use]
pub unsafe fn editing_active() -> bool {
    (*ptr::addr_of!(STATE)).editor.is_some()
}

/// Read edited bytes directly into the parser's owned input buffer.
pub unsafe fn read_edit_line(
    sh: &mut crate::context::Shell,
    destination: &mut [u8],
) -> Result<usize, crate::linedit::LineEditorError> {
    let state = state_mut();
    match (&mut state.editor, &mut state.history) {
        (Some(editor), Some(history)) => editor.read_into(sh, history, destination),
        _ => Ok(0),
    }
}

/// Retain one physical input line, either starting or continuing a command.
pub unsafe fn record_history_line(bytes: &[u8], first: bool) {
    let Some(history) = history_mut() else {
        return;
    };
    if first {
        let _ = history.enter(bytes);
    } else {
        let _ = history.append(bytes);
    }
}

pub static mut displayhist: c_int = 0;

// ---------------------------------------------------------------------
// src/error.h:84-98 — INTOFF / INTON, expanded literally over the globals
// they are defined in terms of. `barrier()` has no Rust equivalent and no
// meaning for a port that does not rely on GCC's instruction scheduling.
// ---------------------------------------------------------------------

macro_rules! INTOFF {
    () => {{
        crate::error::suppressint += 1;
    }};
}

macro_rules! INTON {
    () => {{
        /* In step with `error::INTON`, including the `onint()` it no
         * longer makes -- see there for why the delivery moved. */
        crate::error::suppressint -= 1;
    }};
}

// ---------------------------------------------------------------------
// src/options.h:47-63 — the option flags are `#define`s over optlist[].
// ---------------------------------------------------------------------

/// `#define iflag optlist[3]` (src/options.h:50)
#[inline]
unsafe fn iflag() -> c_char {
    crate::options::optlist[3]
}

/// `#define Vflag optlist[9]` (src/options.h:56)
#[inline]
unsafe fn Vflag() -> c_char {
    crate::options::optlist[9]
}

/// `#define Eflag optlist[10]` (src/options.h:57)
#[inline]
unsafe fn Eflag() -> c_char {
    crate::options::optlist[10]
}

/*
 * Set history and editing status.  Called whenever the status may
 * have changed (figures out what to do).
 */
// [spec:dash:def:histedit.histedit-fn]
// [spec:dash:sem:histedit.histedit-fn]
// [spec:dash:def:myhistedit.histedit-fn]
// [spec:dash:sem:myhistedit.histedit-fn]
pub unsafe fn histedit(sh: &mut crate::context::Shell) {
    if iflag() != 0 {
        if !history_active() {
            INTOFF!();
            state_mut().history = Some(History::new());
            INTON!();
            sethistsize(sh, crate::var::histsizeval());
        }

        let sin: c_int = crate::streams::streams().stdin;
        let serr: c_int = crate::streams::streams().stderr;
        let mode = if Vflag() != 0 {
            Some(EditingMode::Vi)
        } else if Eflag() != 0 {
            Some(EditingMode::Emacs)
        } else {
            None
        };

        if let Some(mode) = mode
            && !editing_active()
            && libc::isatty(sin) != 0
        {
            INTOFF!();
            match LineEditor::new(sin, serr, mode) {
                Ok(editor) => state_mut().editor = Some(editor),
                Err(_) => {
                    state_mut().editor = None;
                    let _ = (&mut *crate::output::stderr())
                        .write_all(b"sh: can't initialize editing\n");
                }
            }
            INTON!();
        } else if mode.is_none() && editing_active() {
            INTOFF!();
            state_mut().editor = None;
            INTON!();
        }

        if let (Some(mode), Some(editor)) = (mode, state_mut().editor.as_mut()) {
            editor.set_mode(mode);
        }
    } else {
        INTOFF!();
        let state = state_mut();
        state.editor = None;
        state.history = None;
        INTON!();
    }
}

// [spec:dash:def:histedit.sethistsize-fn]
// [spec:dash:sem:histedit.sethistsize-fn]
// [spec:dash:def:myhistedit.sethistsize-fn]
// [spec:dash:sem:myhistedit.sethistsize-fn]
pub unsafe fn sethistsize(_sh: &mut crate::context::Shell, hs: *const c_char) {
    let Some(history) = history_mut() else {
        return;
    };
    let histsize = if hs.is_null() || *hs == 0 {
        100
    } else {
        let parsed = libc::atoi(hs);
        if parsed < 0 { 100 } else { parsed }
    };
    history.set_limit(histsize as usize);
}

// [spec:dash:def:histedit.setterm-fn]
// [spec:dash:sem:histedit.setterm-fn]
// [spec:dash:def:myhistedit.setterm-fn]
// [spec:dash:sem:myhistedit.setterm-fn]
pub unsafe fn setterm(term: *const c_char) {
    if term.is_null() {
        return;
    }
    let Some(editor) = state_mut().editor.as_mut() else {
        return;
    };
    if editor
        .set_terminal(core::ffi::CStr::from_ptr(term).to_bytes())
        .is_err()
    {
        let mut message = b"sh: Can't set terminal type ".to_vec();
        message.extend_from_slice(core::ffi::CStr::from_ptr(term).to_bytes());
        message.push(b'\n');
        let errors = &mut *crate::output::stderr();
        let _ = errors.write_all(&message);
        let _ = errors.write_all(b"sh: Using dumb terminal settings.\n");
    }
}
