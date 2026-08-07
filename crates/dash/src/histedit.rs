//! Literal port of `src/histedit.c` and `src/myhistedit.h` —
//! command-line editing and history, plus the `fc` builtin.
//! Rules: `docs/spec/port/src/histedit.md`, `docs/spec/port/src/myhistedit.md`.
//!
//! # libedit
//!
//! The whole of `histedit.c` is inside `#ifndef SMALL` and is written
//! against libedit. This crate has no libedit dependency; the [`libedit`]
//! module below keeps that API's names and signatures and forwards them to
//! [`crate::linedit`], which implements history and line editing in Rust
//! (rustyline). The control flow in this file is unchanged by that
//! substitution, because the contract is the behaviour of `fc`/`histcmd`
//! and the history-recording calls, not the libedit API.
//!
//! History semantics are exact — the `H_*` operations were derived
//! empirically from libedit 3.1 and `histcmd` depends on their orientation
//! (`H_FIRST` is the *newest* entry, `H_PREV` walks toward *newer*). See
//! [`crate::linedit`]. Interactive key bindings and `~/.editrc` are
//! rustyline's and are deliberately *not* a reproduction of libedit's.
//!
//! # Cross-module signatures assumed (see the port report)
//!
//!   * `crate::error::{jmploc, jmp_buf, handler, setjmp, longjmp,
//!     sh_error!, suppressint, intpending, onint}`
//!   * `crate::output::{out1fmt!, outfmt!, out1str, out2str, out2}`
//!   * `crate::memalloc::{stacknxt, sstrend, stalloc, growstackstr, _STPUTC}`
//!   * `crate::options::{optlist, arg0, optionarg}`
//!   * `crate::var::{bltinlookup, histsizeval}`
//!   * `crate::mystring::is_number`
//!   * `crate::eval::evalstring`
//!   * `crate::parser::getprompt`
//!   * `crate::shellmain::readcmdfile` (src/main.c:283)

use core::mem;
use core::ptr;
use libc::{c_char, c_int, c_void, FILE};

extern "C" {
    fn sprintf(s: *mut c_char, format: *const c_char, ...) -> c_int;
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

// ---------------------------------------------------------------------
// src/myhistedit.h — the history/editing interface.
//
// The three typedefs below are given here as their `SMALL` forms, except
// that `HistEvent` has to be libedit's real struct: `histcmd` reads
// `he.num` and `he.str`, which the `SMALL` `typedef int HistEvent` cannot
// provide, and `histedit.c` is only ever compiled in a non-`SMALL` build.
// ---------------------------------------------------------------------

// [spec:dash:def:myhistedit.history]
pub type History = crate::linedit::History;

// [spec:dash:def:myhistedit.edit-line]
pub type EditLine = crate::linedit::EditLine;

// [spec:dash:def:myhistedit.hist-event]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HistEvent {
    pub num: c_int,
    pub str: *const c_char,
}

// [spec:dash:def:myhistedit.history-fn]
// [spec:dash:sem:myhistedit.history-fn]
/// The `SMALL`-build stub for libedit's `history()`: accepts the same
/// arguments and does nothing. In a normal build the real variadic libedit
/// function is used instead — see [`libedit::history`], the shim this port
/// calls from `histedit.c`'s call sites.
pub unsafe fn history(h: *mut History, he: *mut HistEvent, action: c_int, p: *mut c_char) {
    crate::linedit::history_op(h, he, action, crate::linedit::Arg::Str(p as *const c_char));
}

/// history cookie
pub static mut hist: *mut History = ptr::null_mut();
/// editline cookie
pub static mut el: *mut EditLine = ptr::null_mut();
pub static mut displayhist: c_int = 0;
static mut el_in: *mut FILE = ptr::null_mut();
static mut el_out: *mut FILE = ptr::null_mut();

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
// src/memalloc.h:78-97 — the stack-string macros used by fc_replace.
// ---------------------------------------------------------------------

/// `#define stackblock() ((void *)stacknxt)`
macro_rules! stackblock {
    () => {
        crate::memalloc::stacknxt
    };
}

/// `#define STARTSTACKSTR(p) ((p) = stackblock())`
macro_rules! STARTSTACKSTR {
    ($p:ident) => {
        $p = stackblock!()
    };
}

/// `#define STPUTC(c, p) ((p) = _STPUTC((c), (p)))`
macro_rules! STPUTC {
    ($c:expr, $p:ident) => {
        $p = crate::memalloc::_STPUTC($c as c_int, $p)
    };
}

/// `#define STACKSTRNUL(p) ...`
macro_rules! STACKSTRNUL {
    ($p:ident) => {{
        if $p == crate::memalloc::sstrend {
            $p = crate::memalloc::growstackstr() as *mut _;
            *$p = 0;
        } else {
            *$p = 0;
        }
    }};
}

/// `#define grabstackstr(p) stalloc((char *)(p) - (char *)stackblock())`
macro_rules! grabstackstr {
    ($p:expr) => {
        crate::memalloc::stalloc(($p as usize) - (stackblock!() as usize))
    };
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

// ---------------------------------------------------------------------
// The libedit shim.
//
// Same names and signatures as the libedit entry points `histedit.c`
// calls, forwarding to `crate::linedit`. The `H_*` / `EL_*` values are
// libedit's real ones from <histedit.h> (libedit 3.1-20250104), verified
// against the installed header rather than transcribed.
// ---------------------------------------------------------------------

pub(crate) mod libedit {
    use super::{EditLine, History};
    use core::ptr;
    use libc::{c_char, c_int, FILE};

    pub const H_FUNC: c_int = 0;
    pub const H_SETSIZE: c_int = 1;
    pub const H_GETSIZE: c_int = 2;
    pub const H_FIRST: c_int = 3;
    pub const H_LAST: c_int = 4;
    pub const H_PREV: c_int = 5;
    pub const H_NEXT: c_int = 6;
    /* H_SET is 7 and H_CURR is 8 -- verified against /usr/include/histedit.h
     * (libedit 3.1-20250104). These were transposed here, which was inert
     * only because the functions below are stubs that never read them. */
    pub const H_SET: c_int = 7;
    pub const H_CURR: c_int = 8;
    pub const H_ADD: c_int = 9;
    pub const H_ENTER: c_int = 10;
    pub const H_APPEND: c_int = 11;
    pub const H_END: c_int = 12;
    pub const H_NEXT_STR: c_int = 13;
    pub const H_PREV_STR: c_int = 14;
    pub const H_NEXT_EVENT: c_int = 15;
    pub const H_PREV_EVENT: c_int = 16;

    pub const EL_PROMPT: c_int = 0;
    pub const EL_TERMINAL: c_int = 1;
    pub const EL_EDITOR: c_int = 2;
    pub const EL_HIST: c_int = 10;
    pub const EL_PROMPT_ESC: c_int = 21;

    /// `History *history_init(void)`
    pub unsafe fn history_init() -> *mut History {
        crate::linedit::history_init()
    }

    /// `void history_end(History *)`
    pub unsafe fn history_end(h: *mut History) {
        crate::linedit::history_end(h)
    }

    /// `EditLine *el_init(const char *, FILE *, FILE *, FILE *)`
    pub unsafe fn el_init(
        prog: *const c_char,
        fin: *mut FILE,
        fout: *mut FILE,
        ferr: *mut FILE,
    ) -> *mut EditLine {
        crate::linedit::el_init(
            prog,
            fin as *mut libc::c_void,
            fout as *mut libc::c_void,
            ferr as *mut libc::c_void,
        )
    }

    /// `void el_end(EditLine *)`
    pub unsafe fn el_end(e: *mut EditLine) {
        crate::linedit::el_end(e)
    }

    /// `int el_source(EditLine *, const char *)`
    pub unsafe fn el_source(e: *mut EditLine, f: *const c_char) -> c_int {
        crate::linedit::el_source(e, f)
    }

    /// `int history(History *, HistEvent *, int op, ...)` — variadic, so a
    /// macro rather than a function.
    macro_rules! history {
        ($h:expr, $he:expr, $act:expr $(,)?) => {
            $crate::linedit::history_op($h, $he, $act, $crate::linedit::Arg::None)
        };
        ($h:expr, $he:expr, $act:expr, $arg:expr $(,)?) => {
            $crate::linedit::history_op(
                $h, $he, $act,
                <_ as $crate::linedit::IntoArg>::into_arg($arg),
            )
        };
    }
    pub(crate) use history;

    /// `int el_set(EditLine *, int op, ...)` — variadic, so a macro.
    /// `int el_set(EditLine *, int op, ...)`. Only the four operations
    /// `histedit.c` actually issues are routed; anything else reports the
    /// failure libedit gives for an unknown op.
    macro_rules! el_set {
        ($e:expr, $crate_op:expr, history, $hist:expr $(,)?) => {
            $crate::linedit::el_set_hist($e, $hist)
        };
        ($e:expr, $op:expr, $f:expr, $esc:expr $(,)?) => {{
            let _ = $f;
            $crate::linedit::el_set_prompt($e, $op, $esc)
        }};
        ($e:expr, $op:expr, $arg:expr $(,)?) => {{
            let op = $op;
            if op == $crate::histedit::libedit::EL_EDITOR {
                $crate::linedit::el_set_editor($e, $arg)
            } else if op == $crate::histedit::libedit::EL_TERMINAL {
                $crate::linedit::el_set_terminal($e, $arg)
            } else {
                -1 as libc::c_int
            }
        }};
    }
    pub(crate) use el_set;
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
    let el_err: *mut FILE;

    // #define editing (Eflag || Vflag)
    macro_rules! editing {
        () => {
            (Eflag() != 0 || Vflag() != 0)
        };
    }

    if iflag() != 0 {
        if hist.is_null() {
            /*
             * turn history on
             */
            INTOFF!();
            hist = libedit::history_init();
            INTON!();

            if !hist.is_null() {
                sethistsize(crate::var::histsizeval());
            } else {
                crate::output::out2str(c"sh: can't initialize history\n".as_ptr());
            }
        }
        let sin: c_int = crate::streams::streams().stdin;
        let serr: c_int = crate::streams::streams().stderr;
        if editing!() && el.is_null() && libc::isatty(sin) != 0 {
            /* && isatty(2) ??? */
            /*
             * turn editing on
             */
            INTOFF!();
            'ok: {
                'bad: {
                    /* The C names 0 and 2. dash writes the editor's output
                     * to stderr, not stdout, which is why this is `serr`
                     * and not `streams().stdout`. */
                    if el_in.is_null() {
                        el_in = libc::fdopen(sin, c"r".as_ptr());
                    }
                    if el_out.is_null() {
                        el_out = libc::fdopen(serr, c"w".as_ptr());
                    }
                    if el_in.is_null() || el_out.is_null() {
                        break 'bad; /* goto bad */
                    }
                    el_err = el_out;
                    // #if DEBUG
                    //     if (tracefile) el_err = tracefile;
                    // #endif  (DEBUG is not defined in the ported build)
                    el = libedit::el_init(crate::options::arg0, el_in, el_out, el_err);
                    if !el.is_null() {
                        if !hist.is_null() {
                            libedit::el_set!(el, libedit::EL_HIST, history, hist);
                        }
                        libedit::el_set!(
                            el,
                            libedit::EL_PROMPT_ESC,
                            crate::parser::getprompt,
                            0o1 as c_int
                        );
                        break 'ok;
                    }
                    /* else fall through to bad: */
                }
                // bad:
                crate::output::out2str(c"sh: can't initialize editing\n".as_ptr());
            }
            INTON!();
        } else if !editing!() && !el.is_null() {
            INTOFF!();
            libedit::el_end(el);
            el = ptr::null_mut();
            INTON!();
        }
        if !el.is_null() {
            if Vflag() != 0 {
                libedit::el_set!(el, libedit::EL_EDITOR, c"vi".as_ptr());
            } else if Eflag() != 0 {
                libedit::el_set!(el, libedit::EL_EDITOR, c"emacs".as_ptr());
            }
            libedit::el_source(el, ptr::null());
        }
    } else {
        INTOFF!();
        if !el.is_null() {
            /* no editing if not interactive */
            libedit::el_end(el);
            el = ptr::null_mut();
        }
        if !hist.is_null() {
            libedit::history_end(hist);
            hist = ptr::null_mut();
        }
        INTON!();
    }
}

// [spec:dash:def:histedit.sethistsize-fn]
// [spec:dash:sem:histedit.sethistsize-fn]
// [spec:dash:def:myhistedit.sethistsize-fn]
// [spec:dash:sem:myhistedit.sethistsize-fn]
pub unsafe fn sethistsize(hs: *const c_char) {
    let mut histsize: c_int;
    let mut he: HistEvent = mem::zeroed();

    if !hist.is_null() {
        if hs.is_null()
            || *hs == 0
            || {
                histsize = libc::atoi(hs);
                histsize
            } < 0
        {
            histsize = 100;
        }
        libedit::history!(hist, &mut he, libedit::H_SETSIZE, histsize);
    }
}

// [spec:dash:def:histedit.setterm-fn]
// [spec:dash:sem:histedit.setterm-fn]
// [spec:dash:def:myhistedit.setterm-fn]
// [spec:dash:sem:myhistedit.setterm-fn]
pub unsafe fn setterm(term: *const c_char) {
    if !el.is_null() && !term.is_null() {
        if libedit::el_set!(el, libedit::EL_TERMINAL, term) != 0 {
            crate::output::outfmt!(
                crate::output::out2,
                c"sh: Can't set terminal type %s\n".as_ptr(),
                term
            );
            crate::output::outfmt!(
                crate::output::out2,
                c"sh: Using dumb terminal settings.\n".as_ptr()
            );
        }
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
    let mut he: HistEvent = mem::zeroed();
    let mut lflg: c_int = 0;
    let mut nflg: c_int = 0;
    let mut rflg: c_int = 0;
    let mut sflg: c_int = 0;
    // C declares these uninitialised; Rust demands definite
    // initialisation before the `body` closure below captures them. Every
    // one is written before it is read, exactly as in the C, so the
    // initialisers are dead stores.
    let mut i: c_int = 0;
    let mut retval: c_int = 0;
    let mut firststr: *const c_char = ptr::null();
    let mut laststr: *const c_char = ptr::null();
    let mut first: c_int = 0;
    let mut last: c_int = 0;
    let mut direction: c_int = 0;
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
    let mut efp: *mut FILE = ptr::null_mut();
    // The `(void) &var` statements at src/histedit.c:196-210 exist only to
    // stop GCC keeping those variables in registers, where longjmp could
    // clobber them; they have no Rust equivalent.

    if hist.is_null() {
        crate::error::sh_error!(c"history not active".as_ptr());
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
                crate::error::sh_error!(c"option -%c expects argument".as_ptr(), optopt);
                /* NOTREACHED */
            }
            /* case '?': default: */
            _ => {
                crate::error::sh_error!(c"unknown option: -%c".as_ptr(), optopt);
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
                crate::error::sh_error!(c"called recursively too many times".as_ptr());
            }
            /*
             * Set editor.
             */
            if sflg == 0 {
                if editor.is_null() && {
                    editor = crate::var::bltinlookup(c"FCEDIT".as_ptr());
                    editor.is_null()
                } && {
                    editor = crate::var::bltinlookup(c"EDITOR".as_ptr());
                    editor.is_null()
                } {
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
                crate::error::sh_error!(c"too many args".as_ptr());
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
                crate::error::sh_error!(c"too many args".as_ptr());
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
         * XXX - this should not depend on the event numbers
         * always increasing.  Add sequence numbers or offset
         * to the history element in next (diskbased) release.
         */
        direction = if first < last {
            libedit::H_PREV
        } else {
            libedit::H_NEXT
        };

        /*
         * If editing, grab a temp file.
         */
        if !editor.is_null() {
            let fd: c_int;
            INTOFF!(); /* easier */
            sprintf(
                editfile.as_mut_ptr(),
                c"%s_shXXXXXX".as_ptr(),
                _PATH_TMP.as_ptr(),
            );
            fd = libc::mkstemp(editfile.as_mut_ptr());
            if fd < 0 {
                crate::error::sh_error!(
                    c"can't create temporary file %s".as_ptr(),
                    editfile.as_ptr()
                );
            }
            efp = libc::fdopen(fd, c"w".as_ptr());
            if efp.is_null() {
                libc::close(fd);
                crate::error::sh_error!(c"can't allocate stdio buffer for temp".as_ptr());
            }
        }

        /*
         * Loop through selected history events.  If listing or executing,
         * do it now.  Otherwise, put into temp file and call the editor
         * after.
         *
         * The history interface needs rethinking, as the following
         * convolutions will demonstrate.
         */
        libedit::history!(hist, &mut he, libedit::H_FIRST);
        retval = libedit::history!(hist, &mut he, libedit::H_NEXT_EVENT, first);
        while retval != -1 {
            if lflg != 0 {
                if nflg == 0 {
                    crate::output::out1fmt!(c"%5d ".as_ptr(), he.num);
                }
                crate::output::out1str(he.str);
            } else {
                let s: *const c_char = if !pat.is_null() {
                    fc_replace(he.str, pat, repl)
                } else {
                    he.str
                };

                if sflg != 0 {
                    if displayhist != 0 {
                        crate::output::out2str(s);
                    }

                    crate::eval::evalstring(s as *mut c_char, 0);
                    if displayhist != 0 && !hist.is_null() {
                        libedit::history!(hist, &mut he, libedit::H_ENTER, s);
                    }

                    break;
                } else {
                    libc::fputs(s, efp);
                }
            }
            /*
             * At end?  (if we were to lose last, we'd sure be
             * messed up).
             */
            if he.num == last {
                break;
            }
            retval = libedit::history!(hist, &mut he, direction);
        }
        if !editor.is_null() {
            let editcmd: *mut c_char;

            libc::fclose(efp);
            editcmd = crate::memalloc::stalloc(
                libc::strlen(editor) + libc::strlen(editfile.as_ptr()) + 2,
            ) as *mut c_char;
            sprintf(editcmd, c"%s %s".as_ptr(), editor, editfile.as_ptr());
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
unsafe fn fc_replace(mut s: *const c_char, p: *mut c_char, mut r: *mut c_char) -> *const c_char {
    let mut dest: *mut c_char;
    let plen: c_int = libc::strlen(p) as c_int;

    STARTSTACKSTR!(dest);
    while *s != 0 {
        if *s == *p && libc::strncmp(s, p, plen as usize) == 0 {
            while *r != 0 {
                STPUTC!(*r, dest);
                r = r.add(1);
            }
            s = s.add(plen as usize);
            *p = 0; /* so no more matches */
        } else {
            STPUTC!(*s, dest);
            s = s.add(1);
        }
    }
    STACKSTRNUL!(dest);
    dest = grabstackstr!(dest) as *mut c_char;

    dest
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
    let mut he: HistEvent = mem::zeroed();
    let mut s: *const c_char = str;
    let mut relative: c_int = 0;
    let mut i: c_int;
    let mut retval: c_int;

    retval = libedit::history!(hist, &mut he, libedit::H_FIRST);
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
    if crate::mystring::is_number(s) != 0 {
        i = libc::atoi(s);
        if relative != 0 {
            // while (retval != -1 && i--)
            while retval != -1 && {
                let __t = i;
                i -= 1;
                __t != 0
            } {
                retval = libedit::history!(hist, &mut he, libedit::H_NEXT);
            }
            if retval == -1 {
                retval = libedit::history!(hist, &mut he, libedit::H_LAST);
            }
        } else {
            retval = libedit::history!(hist, &mut he, libedit::H_NEXT_EVENT, i);
            if retval == -1 {
                /*
                 * the notion of first and last is
                 * backwards to that of the history package
                 */
                retval = libedit::history!(
                    hist,
                    &mut he,
                    if last != 0 {
                        libedit::H_FIRST
                    } else {
                        libedit::H_LAST
                    }
                );
                if retval != -1 && last != 0 {
                    retval = libedit::history!(hist, &mut he, libedit::H_NEXT);
                }
            }
        }
        if retval == -1 {
            crate::error::sh_error!(
                c"history number %s not found (internal error)".as_ptr(),
                str
            );
        }
    } else {
        /*
         * pattern
         */
        retval = libedit::history!(hist, &mut he, libedit::H_PREV_STR, str);
        if retval == -1 {
            crate::error::sh_error!(c"history pattern not found: %s".as_ptr(), str);
        }
    }
    he.num
}
