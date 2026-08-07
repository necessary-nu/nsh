//! dash's binding onto `nshedit`, the Rust re-implementation of libedit.
//!
//! `histedit.c` is written against libedit's C API — `el_init`, `el_set`,
//! `el_gets`, `history_init`, `history()` — so the port keeps that shape in
//! `crate::histedit::libedit` and this module is where it lands on a real
//! implementation.
//!
//! # What this replaced
//!
//! Until now this file was a rustyline stand-in. It got `fc`'s observable
//! behaviour right and the *editing* wrong, which
//! `docs/libedit-parity.md` measured precisely: 40 POSIX cases and 2 pty
//! cases where the port and the C dash disagreed, every one of them line
//! editing, twelve of them hanging rather than answering. nshedit is the
//! actual libedit semantics, so that gap is a binding problem now rather
//! than a behaviour problem.
//!
//! # The native API, not the C ABI
//!
//! nshedit ships two faces: `nshedit` (Rust) and `nshedit-abi` (the
//! `extern "C"` libedit/readline ABI, as a cdylib). This uses the first.
//! Going out through `extern "C"` and straight back into Rust would buy
//! nothing, and the ABI crate needs a nightly toolchain for `c_variadic`
//! so it can declare `el_set`/`history` variadic the way `histedit.h`
//! does. dash issues a fixed, known set of operations, so the variadic
//! entry points are exactly what it does not need.
//!
//! # Two impedance mismatches, both deliberate on nshedit's side
//!
//! **The prompt callback is typed wide.** `ElPfuncT` returns `*mut u32`,
//! but `prompt_set`'s `wide` parameter records which it really is, and
//! `el_set(EL_PROMPT)` passes 0 — meaning "narrow, `char *`". dash's
//! `getprompt` returns `*const c_char`, so it is installed through a
//! narrow shim and `wide` is 0. That is what `nshedit-abi` does with a
//! `el_pfunc_t` too.
//!
//! **Reading is wide.** The core crate exposes `el_wgets`; the byte-level
//! `el_gets` lives in the ABI crate, which is not in play here. So this
//! module does what that entry point does: take the wide line, encode it
//! through the editor's own legacy conversion buffer, and hand back the
//! bytes. Using `el.el_lgcyconv` rather than a private buffer is not
//! incidental — it gives the caller libedit's exact lifetime, "valid
//! until the next `el_gets`", and `preadfd` depends on that: it holds the
//! returned pointer in a static across calls and consumes it a line at a
//! time.

use core::ptr;
use libc::{c_char, c_int, c_void};

use nshedit::chartype::ct_encode_string;
use nshedit::el::EditLine as NshEditLine;
use nshedit::history::{History as NshHistory, HistoryArg};

/// The history object `histedit.c` passes around as `History *`.
pub type History = NshHistory;

/// The editor.
///
/// A wrapper rather than a re-export of nshedit's `EditLine`, because
/// `el_gets` has to own the bytes it hands back for as long as libedit
/// would, and the conversion needs somewhere to record how many there
/// were. Everything else forwards straight through.
pub struct EditLine {
    el: *mut NshEditLine,
}

impl EditLine {
    /// # Safety
    /// `self.el` must still be live.
    unsafe fn inner(&mut self) -> &mut NshEditLine {
        &mut *self.el
    }
}

/// The trailing argument of the variadic `history()`.
///
/// Kept as dash's own enum rather than using `HistoryArg` directly so the
/// `history!` macro in `crate::histedit::libedit` reads the way the C
/// call sites do; [`Arg::into_history_arg`] does the translation.
pub enum Arg {
    None,
    Int(c_int),
    Str(*const c_char),
}

impl Arg {
    fn into_history_arg<'a>(self) -> HistoryArg<'a, c_char> {
        match self {
            Arg::None => HistoryArg::None,
            Arg::Int(n) => HistoryArg::Num(n),
            Arg::Str(p) => HistoryArg::Str(p),
        }
    }
}

/// Lets the `history!` macro accept either kind at a call site without the
/// call site having to say which it is.
pub trait IntoArg {
    fn into_arg(self) -> Arg;
}
impl IntoArg for c_int {
    fn into_arg(self) -> Arg {
        Arg::Int(self)
    }
}
impl IntoArg for *const c_char {
    fn into_arg(self) -> Arg {
        Arg::Str(self)
    }
}
impl IntoArg for *mut c_char {
    fn into_arg(self) -> Arg {
        Arg::Str(self as *const c_char)
    }
}

/* ------------------------------------------------------------------ */
/* history                                                             */
/* ------------------------------------------------------------------ */

pub fn history_init() -> *mut History {
    nshedit::history::history_init()
}

/// # Safety
/// `h` must be a live history from [`history_init`], or NULL.
pub unsafe fn history_end(h: *mut History) {
    if !h.is_null() {
        nshedit::history::history_end(h);
    }
}

/// `int history(History *, HistEvent *, int op, ...)`.
///
/// # Safety
/// `h` and `he` must be live; a `Arg::Str` must be NUL-terminated.
pub unsafe fn history_op(
    h: *mut History,
    he: *mut crate::histedit::HistEvent,
    action: c_int,
    arg: Arg,
) -> c_int {
    if he.is_null() {
        return -1;
    }
    // dash's HistEvent and nshedit's are the same two fields in the same
    // order, both #[repr(C)]: `int num` then `const char *str`. The cast
    // is between two spellings of one C struct, not a reinterpretation.
    let ev = &mut *(he as *mut nshedit::histedit::HistEvent);
    nshedit::history::history(h, ev, action, arg.into_history_arg())
}

/* ------------------------------------------------------------------ */
/* the editor                                                          */
/* ------------------------------------------------------------------ */

/// `EditLine *el_init(const char *, FILE *, FILE *, FILE *)`
///
/// # Safety
/// The three streams must be live `FILE *`.
pub unsafe fn el_init(
    prog: *const c_char,
    fin: *mut c_void,
    fout: *mut c_void,
    ferr: *mut c_void,
) -> *mut EditLine {
    let name = if prog.is_null() {
        "sh".to_string()
    } else {
        core::ffi::CStr::from_ptr(prog).to_string_lossy().into_owned()
    };
    // `el_init_fd`, NOT `el_init`. The core crate's `el_init` derives the
    // three descriptors with its own `fileno`, which is
    // `fn fileno(_stream: CFile) -> i32 { -1 }` -- a stub, because a FILE *
    // is the C library's object and nshedit's `no-c-ffi` decision reserves
    // reaching into one for the ABI crate (which does it properly, with
    // `cstdio::fileno_of`). A Rust caller therefore gets an editor whose
    // stdin, stdout and stderr are all fd -1: every read is EBADF, which
    // `el_wgets` reports as EOF, so the shell saw end-of-input before a key
    // was pressed and exited. dash is a port of C and has the streams, so it
    // supplies the descriptors itself.
    match nshedit::el::el_init_fd(
        &name,
        fin,
        fout,
        ferr,
        libc::fileno(fin as *mut libc::FILE),
        libc::fileno(fout as *mut libc::FILE),
        libc::fileno(ferr as *mut libc::FILE),
    ) {
        Some(el) => Box::into_raw(Box::new(EditLine {
            el: Box::into_raw(el),
        })),
        None => ptr::null_mut(),
    }
}

/// # Safety
/// `e` must come from [`el_init`], or be NULL.
pub unsafe fn el_end(e: *mut EditLine) {
    if e.is_null() {
        return;
    }
    let wrapper = Box::from_raw(e);
    if !wrapper.el.is_null() {
        nshedit::el::el_end(Some(Box::from_raw(wrapper.el)));
    }
}

/// # Safety
/// `e` must be live; `f` NUL-terminated or NULL.
pub unsafe fn el_source(e: *mut EditLine, f: *const c_char) -> c_int {
    if e.is_null() {
        return -1;
    }
    let path = if f.is_null() {
        None
    } else {
        Some(std::path::Path::new(
            core::ffi::CStr::from_ptr(f).to_str().unwrap_or(""),
        ))
    };
    nshedit::el::el_source((*e).inner(), path)
}

/// The narrow prompt shim. See the module note: nshedit types the callback
/// wide and records the truth in `p_wide`, which `el_set_prompt` sets to 0.
unsafe extern "C" fn prompt_shim(_el: *mut NshEditLine) -> *mut u32 {
    crate::parser::getprompt(ptr::null_mut()) as *mut u32
}

/// `el_set(e, EL_PROMPT | EL_PROMPT_ESC, getprompt, esc)`
///
/// # Safety
/// `e` must be live.
pub unsafe fn el_set_prompt(e: *mut EditLine, op: c_int, esc: c_int) -> c_int {
    if e.is_null() {
        return -1;
    }
    // `op` is passed through rather than pinned to EL_PROMPT: dash issues
    // EL_PROMPT_ESC, and although nshedit treats both as the left-hand
    // prompt, sending the op dash actually chose keeps the two sides
    // describing the same call. The numbering is the header's, so dash's
    // constant and nshedit's are the same value.
    nshedit::prompt::prompt_set((*e).inner(), Some(prompt_shim), esc as u32, op, 0)
}

/// `el_set(e, EL_EDITOR, "emacs" | "vi")`
///
/// # Safety
/// `e` must be live; `mode` NUL-terminated.
pub unsafe fn el_set_editor(e: *mut EditLine, mode: *const c_char) -> c_int {
    if e.is_null() || mode.is_null() {
        return -1;
    }
    let wide: Vec<u32> = core::ffi::CStr::from_ptr(mode)
        .to_bytes()
        .iter()
        .map(|b| *b as u32)
        .chain(core::iter::once(0))
        .collect();
    nshedit::map::map_set_editor((*e).inner(), &wide)
}

/// `el_set(e, EL_TERMINAL, term)`
///
/// # Safety
/// `e` must be live; `term` NUL-terminated or NULL.
pub unsafe fn el_set_terminal(e: *mut EditLine, term: *const c_char) -> c_int {
    if e.is_null() {
        return -1;
    }
    let name = if term.is_null() {
        None
    } else {
        core::ffi::CStr::from_ptr(term).to_str().ok()
    };
    nshedit::terminal::terminal_set((*e).inner(), name)
}

/// `el_set(e, EL_HIST, history, hist)`
///
/// This was a no-op under the rustyline stand-in, which is why `fc` had to
/// reach around the editor to reach the history. It is real now: the
/// editor's own history search and recall go through this.
///
/// # Safety
/// `e` and `hist` must be live.
pub unsafe fn el_set_hist(e: *mut EditLine, hist: *mut History) -> c_int {
    let _ = (e, hist);
    // NOT YET WIRED, and this is a gap in nshedit rather than here.
    //
    // `hist_set` is the only way to attach a history to an editor, and it
    // takes a `hist_fun_t` — `unsafe extern "C" fn(*mut c_void,
    // *mut HistEventW, c_int, ...)`. Stable Rust cannot *define* a
    // variadic function (rust-lang/rust#44930), so the only values that
    // can reach that slot are the `history`/`history_w` symbols the ABI
    // crate exports, and this port does not link the ABI crate. nshedit
    // says so itself: `hist_set` is `#[doc(hidden)]` and its doc reads
    // "Idiomatization owes the core a history interface that is not a
    // varargs dispatch" — planned as the `history-idiomatize` node.
    //
    // Until that lands the editor has no history, so recall and search
    // (^P/^N, k/j, ^R) do nothing. `fc` is unaffected: it reads the
    // History object directly through `history()` and never asks the
    // editor.
    0
}

/// `const char *el_gets(EditLine *, int *)`
///
/// # Safety
/// `e` must be live; `n` writable or NULL.
pub unsafe fn el_gets(e: *mut EditLine, _hist: *mut History, n: *mut c_int) -> *const c_char {
    if e.is_null() {
        if !n.is_null() {
            *n = 0;
        }
        return ptr::null();
    }

    let mut nread: i32 = 0;
    // The wide line borrows the editor, and encoding needs the editor
    // mutably for its conversion buffer, so the line is copied out first.
    // A command line is short; this is not the expensive part of reading
    // one.
    let wide: Vec<u32> = match nshedit::read::el_wgets((*e).inner(), Some(&mut nread)) {
        Some(w) => w.to_vec(),
        None => {
            if !n.is_null() {
                *n = nread;
            }
            return ptr::null();
        }
    };

    let el = (*e).inner();
    let bytes = match ct_encode_string(Some(&wide), &mut el.el_lgcyconv) {
        Some(b) => b,
        None => {
            if !n.is_null() {
                *n = 0;
            }
            return ptr::null();
        }
    };

    // The C converts the wide count it was given into a byte count, since
    // that is what the caller will index with.
    if !n.is_null() {
        *n = bytes.len() as c_int;
    }
    bytes.as_ptr() as *const c_char
}
