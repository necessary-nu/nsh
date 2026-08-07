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
//! **Reading.** `el_gets` is the core's, not a reimplementation of it.
//! It briefly was one — the byte-level entry point lived only in the ABI
//! crate — and the count it reported was the encoded slice's length,
//! which is right only while nobody sets EL_UNBUFFERED. The library
//! reports the count separately for exactly that reason, so this passes
//! the library's `nread` through untouched.

// ---------------------------------------------------------------------
// The shell's line editing, claimed here.
//
// POSIX describes vi-mode editing as behaviour of `sh`, so the rules are
// the shell's to satisfy. The behaviour itself is nshedit's -- every
// motion, every command, the insert/command mode split -- and this file
// is the whole of what decides that the shell has it: which editor is
// attached, in which mode, reading from which descriptors, against which
// history. Claiming them here points a reader at the one file in this
// repo that can turn any of them off.
//
// An impl claim is not a pass. Evidence is the `/test` facet on the
// cases in posix/harness/cases_editing.py, and several of these carry a
// `manual` disposition -- `command-invoke-vi`, `command-redraw`,
// `insert-interrupt`, `sigint-command-mode` -- meaning unmeasured, not
// satisfied.
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

use core::cell::RefCell;
use core::ptr;
use std::rc::Rc;

use libc::{c_char, c_int, c_void};

use nshedit::el::EditLine as NshEditLine;
use nshedit::hist::{EditorHistory, HistLine, HistText};
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

/// Lets the editor walk the history `histedit.c` owns.
///
/// The editor performs exactly four operations -- H_FIRST, H_LAST,
/// H_NEXT, H_PREV, each with no trailing argument -- so this adapts the
/// store dash already has rather than moving dash onto nshedit's own.
/// That matters because `fc`, `H_ENTER` and `H_APPEND` go through the
/// C-shaped `history()` on the same object; one store, two faces.
///
/// Holding the store as a raw pointer is sound because of the order
/// `histedit()` tears things down in: the non-interactive branch calls
/// `el_end` and only then `history_end`, so the editor is always gone
/// before the store it reads is freed. The NULL check is belt and
/// braces for a future edit that reorders them.
struct HistoryRef {
    h: *mut History,
}

impl HistoryRef {
    /// One of the four walks, as a `HistLine`.
    fn walk(&mut self, op: c_int) -> Option<HistLine> {
        if self.h.is_null() {
            return None;
        }
        let mut ev = nshedit::histedit::HistEvent {
            num: 0,
            str: ptr::null(),
        };
        // SAFETY: `h` is a live store from `history_init`; see the type note.
        let rv = unsafe { nshedit::history::history(self.h, &mut ev, op, HistoryArg::None) };
        if rv == -1 || ev.str.is_null() {
            return None;
        }
        // The store is narrow, so hand the bytes over as bytes: `Narrow`
        // exists exactly so neither side transcodes. dash's entries are
        // NUL-terminated; the arm wants no terminator.
        let bytes = unsafe { core::ffi::CStr::from_ptr(ev.str) }.to_bytes().to_vec();
        Some(HistLine {
            num: ev.num,
            text: HistText::Narrow(bytes),
        })
    }
}

impl EditorHistory for HistoryRef {
    fn first(&mut self) -> Option<HistLine> {
        self.walk(crate::histedit::libedit::H_FIRST)
    }
    fn last(&mut self) -> Option<HistLine> {
        self.walk(crate::histedit::libedit::H_LAST)
    }
    fn next(&mut self) -> Option<HistLine> {
        self.walk(crate::histedit::libedit::H_NEXT)
    }
    fn prev(&mut self) -> Option<HistLine> {
        self.walk(crate::histedit::libedit::H_PREV)
    }
}

/// `el_set(e, EL_HIST, history, hist)`
///
/// # Safety
/// `e` and `hist` must be live.
pub unsafe fn el_set_hist(e: *mut EditLine, hist: *mut History) -> c_int {
    if e.is_null() || hist.is_null() {
        return -1;
    }
    let store: Rc<RefCell<dyn EditorHistory>> = Rc::new(RefCell::new(HistoryRef { h: hist }));
    (*e).inner().set_history(store);
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
    let line = nshedit::read::el_gets((*e).inner(), Some(&mut nread));

    // `nread`, never `line.len()`. Under EL_UNBUFFERED the returned slice
    // runs past the reported count into what an earlier line left in the
    // conversion buffer (ERR-core-api-26), so the count is the only honest
    // answer for how much of it is this line. dash does not set
    // EL_UNBUFFERED, so the two agree today; taking the length here would
    // be a latent bug waiting for the option to be used.
    //
    // `preadfd` consumes what it gets by count and never looks for a NUL
    // -- `memcpy(buf, rl_cp, min(nr, el_len))`, then advances both -- so
    // the un-terminated slice is safe to hand back as a `char *`.
    if !n.is_null() {
        *n = nread;
    }
    match line {
        Some(bytes) => bytes.as_ptr() as *const c_char,
        None => ptr::null(),
    }
}

// ---------------------------------------------------------------------
// Unit tests for the history adapter.
//
// The editor itself needs a terminal and is covered by the pty suite and
// the POSIX editing cases. `HistoryRef` needs neither: it is the four
// walks over a store, and getting one of the four opcodes wrong would
// give recall that works in one direction and silently stops in the
// other -- which a differential run against dash would show as a hang,
// not as a wrong answer.
// ---------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::CStr0;

    /// A store holding `lines`, oldest first.
    fn store(lines: &[&str]) -> *mut History {
        let h = history_init();
        assert!(!h.is_null());
        let mut ev = crate::histedit::HistEvent {
            num: 0,
            str: ptr::null(),
        };
        unsafe {
            history_op(h, &mut ev, crate::histedit::libedit::H_SETSIZE, Arg::Int(10));
            for l in lines {
                let s = CStr0::new(l);
                history_op(
                    h,
                    &mut ev,
                    crate::histedit::libedit::H_ENTER,
                    Arg::Str(s.p()),
                );
            }
        }
        h
    }

    fn text(line: &HistLine) -> String {
        match &line.text {
            HistText::Narrow(b) => String::from_utf8_lossy(b).into_owned(),
            HistText::Wide(w) => w.iter().filter_map(|c| char::from_u32(*c)).collect(),
        }
    }

    #[test]
    fn first_is_the_newest_and_last_is_the_oldest() {
        let h = store(&["one", "two", "three"]);
        let mut r = HistoryRef { h };
        // libedit's naming is the surprise this asserts: H_FIRST is the
        // NEWEST entry and H_LAST the oldest, which is why `histcmd`
        // computes its direction the way it does.
        assert_eq!(text(&r.first().unwrap()), "three");
        assert_eq!(text(&r.last().unwrap()), "one");
        unsafe { history_end(h) };
    }

    #[test]
    fn next_walks_older_and_prev_walks_newer() {
        let h = store(&["one", "two", "three"]);
        let mut r = HistoryRef { h };
        assert_eq!(text(&r.first().unwrap()), "three");
        // H_NEXT moves toward OLDER despite the name.
        assert_eq!(text(&r.next().unwrap()), "two");
        assert_eq!(text(&r.next().unwrap()), "one");
        // Off the oldest end reports nothing rather than wrapping.
        assert!(r.next().is_none());
        // ...and back toward the newest.
        assert_eq!(text(&r.prev().unwrap()), "two");
        assert_eq!(text(&r.prev().unwrap()), "three");
        assert!(r.prev().is_none());
        unsafe { history_end(h) };
    }

    #[test]
    fn entries_come_back_as_narrow_bytes_without_a_terminator() {
        let h = store(&["ab"]);
        let mut r = HistoryRef { h };
        let line = r.first().unwrap();
        match line.text {
            // The arm matters: dash's store is HistoryGen<c_char>, so a
            // Wide answer here would mean a transcode round-trip on every
            // keypress.
            HistText::Narrow(ref b) => assert_eq!(b, b"ab"),
            HistText::Wide(_) => panic!("narrow store answered Wide"),
        }
        // Event numbers are what `fc` addresses ranges by.
        assert!(line.num > 0);
        unsafe { history_end(h) };
    }

    #[test]
    fn an_empty_store_and_a_detached_one_report_nothing() {
        let h = store(&[]);
        let mut r = HistoryRef { h };
        assert!(r.first().is_none());
        assert!(r.last().is_none());
        unsafe { history_end(h) };

        // `histedit()` frees the store only after `el_end`, so this is
        // belt and braces -- but a detached shim must be inert, not a
        // dereference of NULL.
        let mut detached = HistoryRef { h: ptr::null_mut() };
        assert!(detached.first().is_none());
        assert!(detached.prev().is_none());
    }
}
