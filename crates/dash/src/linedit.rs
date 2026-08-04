//! The line-editing and history backend, replacing libedit with rustyline.
//!
//! `histedit.c` is written against libedit's C API, so the port keeps that
//! API shape (`history_init`, `history(h, &he, H_OP, ...)`, `el_init`,
//! `el_set`, `el_gets`) and implements it here in Rust. Everything above
//! this module — `histcmd`, `str_to_event`, the `H_ENTER`/`H_APPEND`
//! recording in `input.rs` — is unchanged and still a literal port.
//!
//! # Fidelity
//!
//! The history side is exact. The `H_*` semantics below were derived
//! empirically from libedit 3.1-20250104 rather than from its
//! documentation, because two of them are surprising:
//!
//! ```text
//!   H_FIRST     the NEWEST entry           H_LAST   the OLDEST entry
//!   H_NEXT      moves toward OLDER         H_PREV   moves toward NEWER
//!   H_PREV_STR  prefix search toward OLDER (despite the name)
//!   H_NEXT_STR  prefix search toward NEWER
//!   H_APPEND    appends to the newest entry's text
//! ```
//!
//! `histcmd` depends on exactly this: it computes
//! `direction = first < last ? H_PREV : H_NEXT`, which is only correct if
//! `H_PREV` walks toward higher event numbers.
//!
//! The *interactive editing* side is *not* a faithful reproduction of
//! libedit and cannot be: key bindings, the `~/.editrc` file and the
//! history-file format are rustyline's, not libedit's. `fc`'s observable
//! behaviour is matched; which keystroke moves the cursor is not.

use core::ptr;
use libc::{c_char, c_int};
use std::collections::VecDeque;
use std::ffi::{CStr, CString};

use crate::histedit::HistEvent;

/// One history entry. libedit hands out `const char *` into its own
/// storage and keeps it valid until the entry is evicted, so the `CString`
/// must be owned here and only the pointer handed out.
struct Entry {
    num: c_int,
    text: CString,
}

pub struct History {
    /// Newest last, so index order matches event-number order.
    entries: VecDeque<Entry>,
    /// Index into `entries` of the cursor, or `None` before the first seek.
    cursor: Option<usize>,
    /// `H_SETSIZE`.  libedit's `history_def_init` starts it at 0 and dash
    /// always calls `sethistsize()` straight after `history_init()`, so a
    /// non-zero value is in force before the first entry.  0 is *not*
    /// "unbounded": `history_def_enter` trims with
    /// `while (h->cur > h->max && h->cur > 0)`, so a size of 0 throws the
    /// list away after every insert.  `HISTSIZE=0` (and any non-numeric
    /// `HISTSIZE`, since `sethistsize` uses `atoi`) relies on that.
    max: usize,
    next_num: c_int,
}

impl History {
    fn new() -> Self {
        History {
            entries: VecDeque::new(),
            cursor: None,
            max: 0,
            next_num: 1,
        }
    }

    fn fill(&self, he: *mut HistEvent, idx: usize) -> c_int {
        match self.entries.get(idx) {
            Some(e) => {
                unsafe {
                    (*he).num = e.num;
                    (*he).str = e.text.as_ptr();
                }
                0
            }
            None => -1,
        }
    }

    fn seek(&mut self, he: *mut HistEvent, idx: Option<usize>) -> c_int {
        match idx {
            Some(i) if i < self.entries.len() => {
                self.cursor = Some(i);
                self.fill(he, i)
            }
            _ => -1,
        }
    }

    fn newest(&self) -> Option<usize> {
        self.entries.len().checked_sub(1)
    }

    fn enter(&mut self, s: &CStr) {
        let num = self.next_num;
        self.next_num += 1;
        self.entries.push_back(Entry {
            num,
            text: s.to_owned(),
        });
        // libedit drops the oldest once the list exceeds its size:
        //   while (h->cur > h->max && h->cur > 0)
        //           history_def_delete(h, ev, h->list.prev);
        // There is no `max == 0 means unbounded` case.
        while self.entries.len() > self.max {
            self.entries.pop_front();
        }
        self.cursor = self.newest();
    }

    /// `H_APPEND` concatenates onto the newest entry rather than adding a
    /// new one. dash uses it for continuation lines, so a multi-line
    /// command becomes a single history event.
    fn append(&mut self, s: &CStr) {
        if let Some(last) = self.entries.back_mut() {
            let mut bytes = last.text.as_bytes().to_vec();
            bytes.extend_from_slice(s.to_bytes());
            if let Ok(joined) = CString::new(bytes) {
                last.text = joined;
            }
        } else {
            self.enter(s);
        }
    }

    /// Prefix search. `back` walks toward older entries (`H_PREV_STR`),
    /// otherwise toward newer (`H_NEXT_STR`).
    ///
    /// libedit's `history_prev_string` is
    ///
    /// ```text
    /// for (retval = HCURR(h, ev); retval != -1; retval = HNEXT(h, ev))
    ///         if (Strncmp(str, ev->str, len) == 0)
    ///                 return 0;
    /// ```
    ///
    /// so the scan *starts on the cursor itself* — the entry the cursor is
    /// on is tested first and can be the match.  That matters directly:
    /// `str_to_event` seeks `H_FIRST` before searching, and by then the
    /// `fc ...` command line has already been recorded by `input.c`, so
    /// `fc -l fc` / `fc -s fc` must match that very line.  Failure leaves
    /// the cursor where the walk ran out, i.e. on the oldest (`H_PREV_STR`)
    /// or newest (`H_NEXT_STR`) entry.
    fn search(&mut self, he: *mut HistEvent, pat: &CStr, back: bool) -> c_int {
        let pat = pat.to_bytes();
        // HCURR: -1 when the cursor is on the list head.
        let mut i = match self.cursor {
            Some(c) if c < self.entries.len() => c,
            _ => return -1,
        };
        loop {
            if self.entries[i].text.to_bytes().starts_with(pat) {
                self.cursor = Some(i);
                return self.fill(he, i);
            }
            // HNEXT / HPREV: on failure the cursor is left where it is.
            i = if back {
                match i.checked_sub(1) {
                    Some(n) => n,
                    None => {
                        self.cursor = Some(i);
                        return -1;
                    }
                }
            } else {
                let n = i + 1;
                if n >= self.entries.len() {
                    self.cursor = Some(i);
                    return -1;
                }
                n
            };
            self.cursor = Some(i);
        }
    }

    /// `history_next_event`:
    ///
    /// ```text
    /// for (retval = HFIRST(h, ev); retval != -1; retval = HNEXT(h, ev))
    ///         if (ev->num == num)
    ///                 break;
    /// ```
    ///
    /// i.e. it re-seeks to the newest entry and walks toward older, so the
    /// cursor is left on the match, or on the oldest entry when there is
    /// none (and on the list head when the list is empty).
    fn find_event(&mut self, he: *mut HistEvent, num: c_int) -> c_int {
        self.cursor = self.newest();
        let mut i = match self.cursor {
            Some(c) => c,
            None => return -1,
        };
        loop {
            if self.entries[i].num == num {
                self.cursor = Some(i);
                return self.fill(he, i);
            }
            match i.checked_sub(1) {
                Some(n) => {
                    i = n;
                    self.cursor = Some(i);
                }
                None => return -1,
            }
        }
    }

    /// Every command in `histcmd`/`str_to_event`/`input.rs`. Returns
    /// libedit's 0-on-success / -1-on-failure.
    pub fn op(&mut self, he: *mut HistEvent, action: c_int, arg: Arg) -> c_int {
        use crate::histedit::libedit as op;
        match action {
            op::H_SETSIZE => {
                // history_setsize() rejects a negative size and otherwise
                // only stores it (history_def_setsize is `h->max = num`).
                // The trim is *not* done here — it happens on the next
                // H_ENTER — so a size change is invisible until then.
                if let Arg::Int(n) = arg {
                    if n < 0 {
                        return -1;
                    }
                    self.max = n as usize;
                }
                0
            }
            op::H_GETSIZE => {
                unsafe { (*he).num = self.max as c_int };
                0
            }
            // history_def_first/last move the cursor unconditionally (to
            // the list head when the list is empty) and only then report.
            op::H_FIRST => {
                self.cursor = self.newest();
                let i = self.cursor;
                self.seek(he, i)
            }
            op::H_LAST => {
                self.cursor = if self.entries.is_empty() {
                    None
                } else {
                    Some(0)
                };
                let i = self.cursor;
                self.seek(he, i)
            }
            /* H_NEXT moves toward OLDER, H_PREV toward NEWER -- see the
             * module comment; `histcmd` relies on this orientation. */
            op::H_NEXT => match self.cursor {
                Some(c) => self.seek(he, c.checked_sub(1)),
                None => -1,
            },
            op::H_PREV => match self.cursor {
                Some(c) => self.seek(he, Some(c + 1)),
                None => -1,
            },
            op::H_CURR => match self.cursor {
                Some(c) => self.fill(he, c),
                None => -1,
            },
            op::H_SET => {
                if let Arg::Int(n) = arg {
                    self.find_event(he, n)
                } else {
                    -1
                }
            }
            op::H_NEXT_EVENT | op::H_PREV_EVENT => {
                if let Arg::Int(n) = arg {
                    self.find_event(he, n)
                } else {
                    -1
                }
            }
            op::H_ENTER | op::H_ADD => {
                if let Arg::Str(s) = arg {
                    if s.is_null() {
                        return -1;
                    }
                    let cs = unsafe { CStr::from_ptr(s) };
                    if action == op::H_ENTER {
                        self.enter(cs);
                    } else {
                        self.append(cs);
                    }
                    let i = self.newest();
                    self.seek(he, i)
                } else {
                    -1
                }
            }
            op::H_APPEND => {
                if let Arg::Str(s) = arg {
                    if s.is_null() {
                        return -1;
                    }
                    self.append(unsafe { CStr::from_ptr(s) });
                    let i = self.newest();
                    self.seek(he, i)
                } else {
                    -1
                }
            }
            op::H_PREV_STR | op::H_NEXT_STR => {
                if let Arg::Str(s) = arg {
                    if s.is_null() {
                        return -1;
                    }
                    let back = action == op::H_PREV_STR;
                    self.search(he, unsafe { CStr::from_ptr(s) }, back)
                } else {
                    -1
                }
            }
            op::H_END => {
                self.entries.clear();
                self.cursor = None;
                0
            }
            _ => -1,
        }
    }

    /// The lines rustyline should offer on up-arrow, oldest first.
    fn lines(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|e| String::from_utf8_lossy(e.text.to_bytes()).trim_end().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

/// The extra argument of libedit's variadic `history()`, which is an int
/// for the seek/size operations and a string for the add/search ones.
#[derive(Clone, Copy)]
pub enum Arg {
    None,
    Int(c_int),
    Str(*const c_char),
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

pub fn history_init() -> *mut History {
    Box::into_raw(Box::new(History::new()))
}

pub unsafe fn history_end(h: *mut History) {
    if !h.is_null() {
        drop(Box::from_raw(h));
    }
}

pub unsafe fn history_op(h: *mut History, he: *mut HistEvent, action: c_int, arg: Arg) -> c_int {
    if h.is_null() || he.is_null() {
        return -1;
    }
    (*h).op(he, action, arg)
}

// ---------------------------------------------------------------------
// The editor half.
// ---------------------------------------------------------------------

pub struct EditLine {
    editor: Option<rustyline::Editor<(), rustyline::history::DefaultHistory>>,
    /// `EL_PROMPT_ESC` hands us `getprompt`; call it for each line.
    prompt_fn: Option<unsafe fn(*mut libc::c_void) -> *const c_char>,
    vi_mode: bool,
    /// `el_gets` returns a borrowed pointer that must stay valid until the
    /// next call, exactly as libedit's does.
    last: Option<CString>,
}

pub fn el_init() -> *mut EditLine {
    // libedit emits none of rustyline's modern terminal protocol, so turn
    // off what can be turned off. Bracketed paste in particular wraps every
    // prompt in \e[?2004h/l, which shows up as noise against the C shell.
    let cfg = rustyline::Config::builder().bracketed_paste(false).build();
    let r = rustyline::Editor::with_config(cfg);
    if std::env::var_os("DASH_LINEDIT_DEBUG").is_some() {
        eprintln!("[linedit] el_init: editor ok={}", r.is_ok());
    }
    let editor = r.ok();
    Box::into_raw(Box::new(EditLine {
        editor,
        prompt_fn: None,
        vi_mode: false,
        last: None,
    }))
}

pub unsafe fn el_end(e: *mut EditLine) {
    if !e.is_null() {
        drop(Box::from_raw(e));
    }
}

pub unsafe fn el_set_prompt(
    e: *mut EditLine,
    f: unsafe fn(*mut libc::c_void) -> *const c_char,
) -> c_int {
    if e.is_null() {
        return -1;
    }
    (*e).prompt_fn = Some(f);
    0
}

pub unsafe fn el_set_editor(e: *mut EditLine, mode: *const c_char) -> c_int {
    if e.is_null() || mode.is_null() {
        return -1;
    }
    let m = CStr::from_ptr(mode);
    (*e).vi_mode = m.to_bytes() == b"vi";
    if let Some(ed) = (*e).editor.as_mut() {
        use rustyline::config::{Configurer, EditMode};
        ed.set_edit_mode(if (*e).vi_mode {
            EditMode::Vi
        } else {
            EditMode::Emacs
        });
    }
    0
}

/// `el_set(el, EL_TERMINAL, term)`. libedit re-reads the termcap entry;
/// rustyline discovers the terminal itself, so there is nothing to do.
/// dash treats a non-zero return as a fatal `sh_error`, so report success.
pub unsafe fn el_set_terminal(_e: *mut EditLine, _term: *const c_char) -> c_int {
    0
}

/// `el_source` reads `~/.editrc`. rustyline has no equivalent; libedit
/// also returns -1 when the file is absent and dash ignores the result.
pub unsafe fn el_source(_e: *mut EditLine, _f: *const c_char) -> c_int {
    -1
}

/// `const char *el_gets(EditLine *, int *count)`.
///
/// Returns a line including its trailing newline (what dash's reader
/// expects), or NULL at EOF. `count` receives the byte length.
pub unsafe fn el_gets(e: *mut EditLine, hist: *mut History, n: *mut c_int) -> *const c_char {
    if e.is_null() {
        return ptr::null();
    }
    if std::env::var_os("DASH_LINEDIT_DEBUG").is_some() {
        eprintln!("[linedit] el_gets entered");
    }
    let prompt = match (*e).prompt_fn {
        Some(f) => {
            let p = f(ptr::null_mut());
            if p.is_null() {
                String::new()
            } else {
                // getprompt embeds \1 guards around non-printing runs
                // (EL_PROMPT_ESC); strip them, they are not to be printed.
                String::from_utf8_lossy(CStr::from_ptr(p).to_bytes())
                    .replace('\u{1}', "")
            }
        }
        None => String::new(),
    };

    let ed = match (*e).editor.as_mut() {
        Some(ed) => ed,
        None => {
            if std::env::var_os("DASH_LINEDIT_DEBUG").is_some() {
                eprintln!("[linedit] no editor: rustyline::Editor::new() failed");
            }
            return ptr::null();
        }
    };

    // Keep rustyline's recall list in step with the shell's own history,
    // which `input.rs` maintains through H_ENTER/H_APPEND.
    if !hist.is_null() {
        use rustyline::history::History as _;
        let want = (*hist).lines();
        if ed.history().len() != want.len() {
            let _ = ed.clear_history();
            for l in want {
                let _ = ed.add_history_entry(l);
            }
        }
    }

    match ed.readline(&prompt) {
        Ok(mut line) => {
            line.push('\n');
            let bytes = line.into_bytes();
            let len = bytes.len() as c_int;
            match CString::new(bytes) {
                Ok(cs) => {
                    (*e).last = Some(cs);
                    if !n.is_null() {
                        *n = len;
                    }
                    (*e).last.as_ref().unwrap().as_ptr()
                }
                Err(_) => ptr::null(),
            }
        }
        // Ctrl-D on an empty line, and Ctrl-C, both read as end of input
        // here; dash's caller treats NULL as EOF.
        Err(e) => {
            if std::env::var_os("DASH_LINEDIT_DEBUG").is_some() {
                eprintln!("[linedit] readline error: {e:?}");
            }
            if !n.is_null() {
                *n = 0;
            }
            ptr::null()
        }
    }
}
