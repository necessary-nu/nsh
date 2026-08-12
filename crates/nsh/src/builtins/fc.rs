//! `fc`.
//!
//! Port of `histcmd` and its helpers from `src/histedit.c`.
//!
//! The history list itself stays in `crate::histedit`, which is where
//! the line editor writes it; this is the command that lists, edits and
//! re-runs entries. It re-enters evaluation three ways -- `-s`, the
//! editor it spawns, and the file it reads back -- so like `eval` it
//! depends on its words not borrowing from the shell.
//!
//! It is also the last builtin holding a `char **`, and
//! `crate::builtins::writable_args` says why: `getopt(3)` permutes the
//! array it scans, and `fc -s old=new` splits that word in place.

use bstr::{BStr, BString, ByteSlice};
use core::ffi::CStr;
use core::mem;
use core::ptr;
use libc::{c_char, c_int};
use std::fs::File;
use std::io::Write;
use std::os::fd::FromRawFd;

use crate::error::{INTOFF, INTON};
use crate::histedit::{displayhist, history_active, history_mut, record_history_line};

unsafe extern "C" {
    // `<getopt.h>` state. `libc` 0.2 exposes `getopt` but not these.
    // `fc` is the only builtin that scans with `getopt(3)` rather than
    // with `crate::options::Options`, so this is the only place that
    // needs them -- and the process-global state they are is the limit
    // `[dec:nsh:no-ambient-state]` records rather than one this node
    // lifts.
    static mut optind: c_int;
    static mut optopt: c_int;
    static mut optarg: *mut c_char;
}
use crate::linedit::HistoryEvent;

const MAXPATHLEN: usize = libc::PATH_MAX as usize;
/// `#include <paths.h>` — `_PATH_TMP`.
const _PATH_TMP: &core::ffi::CStr = c"/tmp/";

/// max recursions through fc
const MAXHISTLOOPS: c_int = 4;
/// default editor *should* be $EDITOR
const DEFEDITOR: &core::ffi::CStr = c"ed";

/// What the option scan found: the five flags `fc` reads afterwards.
///
/// Extracted from `histcmd` because it is a self-contained phase -- the
/// scan ends and nothing after it looks at an option again -- and because
/// `histcmd` is long enough without it.
struct Flags {
    editor: *const c_char,
    lflg: c_int,
    nflg: c_int,
    rflg: c_int,
    sflg: c_int,
}

/// `getopt(3)` over `fc`'s own array, stopping at the first word that
/// could be a history number so that `fc -2` names an entry rather than
/// an option.
unsafe fn scan_options(argc: c_int, argv: *mut *mut c_char) -> Flags {
    let mut ch: c_int;
    let mut flags = Flags {
        editor: ptr::null(),
        lflg: 0,
        nflg: 0,
        rflg: 0,
        sflg: 0,
    };

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
                flags.editor = optarg;
            }
            b'l' => {
                flags.lflg = 1;
            }
            b'n' => {
                flags.nflg = 1;
            }
            b'r' => {
                flags.rflg = 1;
            }
            b's' => {
                flags.sflg = 1;
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
    flags
}

/*
 *  This command is provided since POSIX decided to standardize
 *  the Korn shell fc command.  Oh well...
 */
// [spec:dash:def:histedit.histcmd-fn]
// [spec:dash:sem:histedit.histcmd-fn]
// [spec:dash:def:myhistedit.histcmd-fn]
// [spec:dash:sem:myhistedit.histcmd-fn]
pub unsafe fn histcmd(args: &[&BStr]) -> c_int {
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

    /* `getopt(3)` keeps its state in process globals and permutes the
     * array it is given, and `fc -s old=new` splits that word in place
     * and then truncates it -- writes that `$_` shows, so they have to
     * land on the shell's own words. `writable_args` says why that is the
     * one builtin still holding a `char **`; that the scan is libc's at
     * all is the limit `[dec:nsh:no-ambient-state]` records, and not this
     * node's to lift. */
    let mut slots = crate::builtins::writable_args(args);
    let mut argc: c_int = args.len() as c_int;
    let mut argv: *mut *mut c_char = slots.as_mut_ptr();

    let flags = scan_options(argc, argv);
    editor = flags.editor;
    lflg = flags.lflg;
    nflg = flags.nflg;
    rflg = flags.rflg;
    sflg = flags.sflg;

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
                repl = CStr::from_ptr(*argv)
                    .to_bytes()
                    .find_byte(b'=')
                    .map_or(ptr::null_mut(), |at| (*argv).add(at));
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
            INTOFF(); /* easier */
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
            INTON();
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
unsafe fn fc_replace(s: *const c_char, p: *mut c_char, r: *mut c_char) -> BString {
    let hay = CStr::from_ptr(s).to_bytes();
    /* The C walks `s` a byte at a time and asks `*s == *p && strncmp(s,
     * p, plen)` at each position, which is `find`. The leading-byte test
     * is not an optimisation, though: it is also what makes an *empty*
     * pattern match nothing, because the loop only runs while `*s` is
     * non-NUL and `*p` is the NUL. `find` on an empty needle answers 0,
     * so the emptiness is checked rather than inherited. */
    let hit = {
        let pat = CStr::from_ptr(p).to_bytes();
        if pat.is_empty() {
            None
        } else {
            hay.find(pat).map(|at| (at, pat.len()))
        }
    };

    let mut dest: BString = BString::new(Vec::new());
    match hit {
        Some((at, plen)) => {
            dest.extend_from_slice(&hay[..at]);
            dest.extend_from_slice(CStr::from_ptr(r).to_bytes());
            dest.extend_from_slice(&hay[at + plen..]);
            /* `so no more matches` — the C truncates the pattern in
             * place, and the buffer belongs to the caller, so the
             * suppression carries across the whole range of events and
             * not just the rest of this line. */
            *p = 0;
        }
        None => dest.extend_from_slice(hay),
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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::testutil::CStr0;

    /// The option scan stops at the first word that could be a history
    /// number, which is what lets `fc -2` name an entry two back rather
    /// than fail as an unknown option. `not_fcnumber` is that test.
    fn is_number_word(s: &str) -> bool {
        let word = CStr0::new(s);
        unsafe { not_fcnumber(word.p() as *mut c_char) == 0 }
    }

    #[test]
    fn a_negative_number_is_an_event() {
        assert!(is_number_word("-2"));
        assert!(is_number_word("-10"));
    }

    /// A plain number is one too, and so is the absence of a word: the C
    /// hands it `argv[optind]`, which is NULL past the end.
    #[test]
    fn plain_number_and_end_are_events() {
        assert!(is_number_word("2"));
        assert!(unsafe { not_fcnumber(ptr::null_mut()) == 0 });
    }

    #[test]
    fn an_option_is_not_an_event() {
        assert!(!is_number_word("-l"));
        assert!(!is_number_word("-e"));
        assert!(!is_number_word("--"));
    }

    /// A bare `-` is not a number, and neither is a name.
    #[test]
    fn a_word_is_not_an_event() {
        assert!(!is_number_word("-"));
        assert!(!is_number_word("echo"));
    }
}
