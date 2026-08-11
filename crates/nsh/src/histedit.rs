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

use bstr::BString;
use core::ffi::CStr;
use core::mem;
use core::ptr;
use libc::{c_char, c_int};
use nshedit::domain::EditingMode;
use std::fs::File;
use std::io::Write;
use std::os::fd::FromRawFd;

use crate::linedit::{History, HistoryEvent, LineEditor};

unsafe extern "C" {
    // `<getopt.h>` state. `libc` 0.2 exposes `getopt` but not these.
    static mut optind: c_int;
    static mut optopt: c_int;
    static mut optarg: *mut c_char;
}

/// `#include <sys/param.h>` — MAXPATHLEN.
const MAXPATHLEN: usize = libc::PATH_MAX as usize;
/// `#include <paths.h>` — `_PATH_TMP`.
const _PATH_TMP: &core::ffi::CStr = c"/tmp/";

/// max recursions through fc
const MAXHISTLOOPS: c_int = 4;
/// default editor *should* be $EDITOR
const DEFEDITOR: &core::ffi::CStr = c"ed";

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
unsafe fn history_mut() -> Option<&'static mut History> {
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
    destination: &mut [u8],
) -> Result<usize, crate::linedit::LineEditorError> {
    let state = state_mut();
    match (&mut state.editor, &mut state.history) {
        (Some(editor), Some(history)) => editor.read_into(history, destination),
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
        crate::error::suppressint -= 1;
        if crate::error::suppressint == 0 && crate::error::intpending != 0 {
            crate::error::onint();
        }
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
pub unsafe fn histedit() {
    if iflag() != 0 {
        if !history_active() {
            INTOFF!();
            state_mut().history = Some(History::new());
            INTON!();
            sethistsize(crate::var::histsizeval());
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
pub unsafe fn sethistsize(hs: *const c_char) {
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

/*
 *  This command is provided since POSIX decided to standardize
 *  the Korn shell fc command.  Oh well...
 */
// [spec:dash:def:histedit.histcmd-fn]
// [spec:dash:sem:histedit.histcmd-fn]
// [spec:dash:def:myhistedit.histcmd-fn]
// [spec:dash:sem:myhistedit.histcmd-fn]
pub unsafe fn histcmd(mut argc: c_int, mut argv: *mut *mut c_char) -> c_int {
    let mut ch: c_int;
    let mut editor: *const c_char = ptr::null();
    let mut lflg: c_int = 0;
    let mut nflg: c_int = 0;
    let mut rflg: c_int = 0;
    let mut sflg: c_int = 0;
    // C declares these uninitialised; Rust demands definite
    // initialisation before the `body` closure below captures them. Every
    // one is written before it is read, exactly as in the C, so the
    // initialisers are dead stores.
    let mut i: c_int = 0;
    let mut firststr: *const c_char = ptr::null();
    let mut laststr: *const c_char = ptr::null();
    let mut first: c_int = 0;
    let mut last: c_int = 0;
    /* ksh "fc old=new" crap */
    let mut pat: *mut c_char = ptr::null_mut();
    let mut repl: *mut c_char = ptr::null_mut();
    static mut active: c_int = 0;
    let mut jmploc: crate::error::jmploc = mem::zeroed();
    // C leaves this uninitialised (`struct jmploc *volatile savehandler;`);
    // Rust needs it definitely initialised before the setjmp branch reads
    // it, and nothing can longjmp here before it is assigned.
    let mut savehandler: *mut crate::error::jmploc = ptr::null_mut();
    let mut editfile: [c_char; MAXPATHLEN + 1] = [0; MAXPATHLEN + 1];
    let mut edit_file: Option<File> = None;
    // The `(void) &var` statements at src/histedit.c:196-210 exist only to
    // stop GCC keeping those variables in registers, where longjmp could
    // clobber them; they have no Rust equivalent.

    if !history_active() {
        crate::error::sh_error(b"history not active");
    }

    // #ifdef __GLIBC__
    optind = 0;
    // #else
    //     optreset = 1; optind = 1; /* initialize getopt */
    // #endif
    loop {
        // while (not_fcnumber(argv[optind ?: 1]) &&
        //        (ch = getopt(argc, argv, ":e:lnrs")) != -1)
        let idx: c_int = if optind != 0 { optind } else { 1 };
        if not_fcnumber(*argv.add(idx as usize)) == 0 {
            break;
        }
        ch = libc::getopt(argc, argv, c":e:lnrs".as_ptr());
        if ch == -1 {
            break;
        }
        match ch as u8 {
            b'e' => {
                editor = optarg;
            }
            b'l' => {
                lflg = 1;
            }
            b'n' => {
                nflg = 1;
            }
            b'r' => {
                rflg = 1;
            }
            b's' => {
                sflg = 1;
            }
            b':' => {
                let mut message = b"option -".to_vec();
                message.push(optopt as u8);
                message.extend_from_slice(b" expects argument");
                crate::error::sh_error(&message);
                /* NOTREACHED */
            }
            /* case '?': default: */
            _ => {
                let mut message = b"unknown option: -".to_vec();
                message.push(optopt as u8);
                crate::error::sh_error(&message);
                /* NOTREACHED */
            }
        }
    }
    optind = if optind != 0 { optind } else { 1 };
    argc -= optind;
    argv = argv.add(optind as usize);

    /*
     * If executing...
     *
     * The C arms a handler here (`if (setjmp(jmploc.loc)) { ... }`) and
     * leaves it installed for *the whole rest of the function* — there is
     * no `out:` label, and `handler` is deliberately not restored on the
     * normal path (that dangling `handler` is the C's, and is preserved).
     * Handlers in this port are established by `eval::setjmp_catch`, so
     * everything the C guards has to live in the closure and the non-zero
     * arm becomes the code after the call. When the guard is not entered
     * (`fc -l`), the C runs the same tail with no handler installed, so
     * the closure is called directly instead.
     */
    let executing: bool = lflg == 0 || !editor.is_null() || sflg != 0;
    if executing {
        lflg = 0; /* ignore */
        editfile[0] = 0;
        /*
         * Catch interrupts to reset active counter and
         * cleanup temp files.
         *
         * `savehandler = handler` is the C's first statement after
         * `setjmp` returns 0; hoisting it above the `setjmp_catch` call
         * is invisible (nothing can jump to `jmploc` before `handler`
         * points at it) and is what lets the cleanup arm read it.
         */
        savehandler = crate::error::handler;
    }
    let jl: *mut crate::error::jmploc = ptr::addr_of_mut!(jmploc);
    let mut body = || {
        if executing {
            crate::error::handler = jl;
            active += 1;
            if active > MAXHISTLOOPS {
                active = 0;
                displayhist = 0;
                crate::error::sh_error(b"called recursively too many times");
            }
            /*
             * Set editor.
             */
            if sflg == 0 {
                if editor.is_null()
                    && {
                        editor = crate::var::bltinlookup(c"FCEDIT".as_ptr());
                        editor.is_null()
                    }
                    && {
                        editor = crate::var::bltinlookup(c"EDITOR".as_ptr());
                        editor.is_null()
                    }
                {
                    editor = DEFEDITOR.as_ptr();
                }
                if *editor == b'-' as c_char && *editor.add(1) == 0 {
                    sflg = 1; /* no edit */
                    editor = ptr::null();
                }
            }
        }

        /*
         * If -s is specified, accept [old=new] first only
         */
        if sflg != 0 {
            if argc > 0 && {
                repl = libc::strchr(*argv, b'=' as c_int);
                !repl.is_null()
            } {
                pat = *argv;
                *repl = 0;
                repl = repl.add(1);
                argc -= 1;
                argv = argv.add(1);
            }
            if argc >= 2 {
                crate::error::sh_error(b"too many args");
            }
        }

        /*
         * determine [first] and [last]
         */
        match argc {
            0 => {
                firststr = if lflg != 0 {
                    c"-16".as_ptr()
                } else {
                    c"-1".as_ptr()
                };
                laststr = c"-1".as_ptr();
            }
            1 => {
                firststr = *argv;
                laststr = if lflg != 0 { c"-1".as_ptr() } else { *argv };
            }
            2 => {
                firststr = *argv;
                laststr = *argv.add(1);
            }
            _ => {
                crate::error::sh_error(b"too many args");
                /* NOTREACHED */
            }
        }
        /*
         * Turn into event numbers.
         */
        first = str_to_event(firststr, 0);
        last = str_to_event(laststr, 1);

        if rflg != 0 {
            i = last;
            last = first;
            first = i;
        }
        /*
         * If editing, grab a temp file.
         */
        if !editor.is_null() {
            let fd: c_int;
            INTOFF!(); /* easier */
            let path = _PATH_TMP.to_bytes();
            let suffix = b"_shXXXXXX\0";
            debug_assert!(path.len() + suffix.len() <= editfile.len());
            ptr::copy_nonoverlapping(
                path.as_ptr() as *const c_char,
                editfile.as_mut_ptr(),
                path.len(),
            );
            ptr::copy_nonoverlapping(
                suffix.as_ptr() as *const c_char,
                editfile.as_mut_ptr().add(path.len()),
                suffix.len(),
            );
            fd = libc::mkstemp(editfile.as_mut_ptr());
            if fd < 0 {
                let mut message = b"can't create temporary file ".to_vec();
                message.extend_from_slice(CStr::from_ptr(editfile.as_ptr()).to_bytes());
                crate::error::sh_error(&message);
            }
            edit_file = Some(File::from_raw_fd(fd));
        }

        // Snapshot the semantic range before `evalstring` can re-enter the
        // shell and mutate history.
        let events = history_mut()
            .map(|history| history.range(first, last))
            .unwrap_or_default();
        for event in events {
            let mut line = nul_terminated(&event.line);
            if lflg != 0 {
                if nflg == 0 {
                    let _ = write!(&mut *crate::output::stdout(), "{:5} ", event.number);
                }
                let _ = (&mut *crate::output::stdout()).write_all(
                    core::ffi::CStr::from_ptr(line.as_ptr() as *const c_char).to_bytes(),
                );
            } else {
                let mut replaced = if pat.is_null() {
                    None
                } else {
                    Some(fc_replace(line.as_ptr() as *const c_char, pat, repl))
                };
                let s: *mut c_char = match &mut replaced {
                    Some(replaced) => replaced.as_mut_ptr() as *mut c_char,
                    None => line.as_mut_ptr() as *mut c_char,
                };

                if sflg != 0 {
                    if displayhist != 0 {
                        let _ = (&mut *crate::output::stderr())
                            .write_all(core::ffi::CStr::from_ptr(s).to_bytes());
                    }

                    crate::eval::evalstring(s, 0);
                    if displayhist != 0 && history_active() {
                        record_history_line(core::ffi::CStr::from_ptr(s).to_bytes(), true);
                    }

                    break;
                } else {
                    let file = edit_file
                        .as_mut()
                        .expect("fc edit file must exist while an editor is selected");
                    let _ = file.write_all(core::ffi::CStr::from_ptr(s).to_bytes());
                }
            }
        }
        if !editor.is_null() {
            /* The C `stalloc`s `strlen(editor) + strlen(editfile) + 2` —
             * the two strings, the separating space and the terminator —
             * and lets `fccmd`'s enclosing mark release it.  `evalstring`
             * copies what it is given, so the buffer is dead as soon as
             * that call returns and can be this block's. */
            let mut editcmdbuf: Vec<u8> =
                Vec::with_capacity(libc::strlen(editor) + libc::strlen(editfile.as_ptr()) + 2);
            editcmdbuf.extend_from_slice(CStr::from_ptr(editor).to_bytes());
            editcmdbuf.push(b' ');
            editcmdbuf.extend_from_slice(CStr::from_ptr(editfile.as_ptr()).to_bytes());
            editcmdbuf.push(0);
            let editcmd: *mut c_char = editcmdbuf.as_mut_ptr() as *mut c_char;

            drop(edit_file.take());
            /* XXX - should use no JC command */
            crate::eval::evalstring(editcmd, 0);
            INTON!();
            /* XXX - should read back - quick tst */
            crate::shellmain::readcmdfile(editfile.as_mut_ptr());
            libc::unlink(editfile.as_ptr());
        }

        if lflg == 0 && active > 0 {
            active -= 1;
        }
        if displayhist != 0 {
            displayhist = 0;
        }
    };

    if executing {
        if crate::eval::setjmp_catch(jl, body) != 0 {
            active = 0;
            drop(edit_file.take());
            if editfile[0] != 0 {
                libc::unlink(editfile.as_ptr());
            }
            crate::error::handler = savehandler;
            crate::error::raise_longjmp(crate::error::handler, 1);
        }
    } else {
        body();
    }
    0
}

// [spec:dash:def:histedit.fc-replace-fn]
// [spec:dash:sem:histedit.fc-replace-fn]
//
// The C returns `grabstackstr(dest)`, which reserves the bytes *before* the
// `STACKSTRNUL` — so the terminator sits one past the allocation and the
// caller reads it anyway. An owned string carries its own terminator, and
// returning it makes the lifetime the caller's rather than the enclosing
// stack mark's, which matters because the caller hands it to `evalstring`.
unsafe fn fc_replace(mut s: *const c_char, p: *mut c_char, mut r: *mut c_char) -> BString {
    let mut dest: BString = BString::new(Vec::new());
    let plen: c_int = libc::strlen(p) as c_int;

    while *s != 0 {
        if *s == *p && libc::strncmp(s, p, plen as usize) == 0 {
            while *r != 0 {
                dest.push(*r as u8);
                r = r.add(1);
            }
            s = s.add(plen as usize);
            *p = 0; /* so no more matches */
        } else {
            dest.push(*s as u8);
            s = s.add(1);
        }
    }
    dest.push(0);

    dest
}

fn nul_terminated(line: &[u8]) -> BString {
    let mut result = BString::from(line);
    result.push(0);
    result
}

// [spec:dash:def:histedit.not-fcnumber-fn]
// [spec:dash:sem:histedit.not-fcnumber-fn]
// [spec:dash:def:myhistedit.not-fcnumber-fn]
// [spec:dash:sem:myhistedit.not-fcnumber-fn]
pub unsafe fn not_fcnumber(mut s: *mut c_char) -> c_int {
    if s.is_null() {
        return 0;
    }
    if *s == b'-' as c_char {
        s = s.add(1);
    }
    (crate::mystring::is_number(s) == 0) as c_int
}

// [spec:dash:def:histedit.str-to-event-fn]
// [spec:dash:sem:histedit.str-to-event-fn]
// [spec:dash:def:myhistedit.str-to-event-fn]
// [spec:dash:sem:myhistedit.str-to-event-fn]
pub unsafe fn str_to_event(str: *const c_char, last: c_int) -> c_int {
    let mut s: *const c_char = str;
    let mut relative: c_int = 0;
    match *s as u8 {
        b'-' => {
            relative = 1;
            /*FALLTHROUGH*/
            s = s.add(1);
        }
        b'+' => {
            s = s.add(1);
        }
        _ => {}
    }
    let event: Option<HistoryEvent> = if crate::mystring::is_number(s) != 0 {
        let i = libc::atoi(s);
        if relative != 0 {
            history_mut().and_then(|history| {
                usize::try_from(i)
                    .ok()
                    .and_then(|offset| history.relative(offset))
                    .or_else(|| history.oldest())
            })
        } else {
            history_mut().and_then(|history| {
                history.numbered(i).or_else(|| {
                    if last != 0 {
                        history.relative(1)
                    } else {
                        history.oldest()
                    }
                })
            })
        }
    } else {
        let prefix = core::ffi::CStr::from_ptr(str).to_bytes();
        history_mut().and_then(|history| history.prefixed(prefix))
    };

    match event {
        Some(event) => event.number,
        None if crate::mystring::is_number(s) != 0 => {
            let mut message = b"history number ".to_vec();
            message.extend_from_slice(CStr::from_ptr(str).to_bytes());
            message.extend_from_slice(b" not found (internal error)");
            crate::error::sh_error(&message)
        }
        None => {
            let mut message = b"history pattern not found: ".to_vec();
            message.extend_from_slice(CStr::from_ptr(str).to_bytes());
            crate::error::sh_error(&message)
        }
    }
}
