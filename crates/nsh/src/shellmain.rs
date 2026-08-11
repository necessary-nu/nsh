//! Literal port of `src/main.c` / `src/main.h`.
//! Rules: `docs/spec/port/src/main.md` (the rule ids use the `main.`
//! prefix even though the module is called `shellmain`, since `main` is
//! taken by the binary crate root).
//!
//! Translation notes (literal, bug-for-bug):
//!   * `main()`'s `setjmp` on `main_handler` becomes
//!     `crate::eval::setjmp_catch` over the startup sequence, and the
//!     `state1`..`state4` labels become an explicit label program
//!     counter that the jump handler re-enters at. `state` is written
//!     through a raw pointer, which is what makes it survive the
//!     unwind exactly as C's `volatile int state` survives the
//!     `longjmp`.
//!   * `PROFILE` is 0 and `GPROF` undefined, so `monitor()`/`_mcleanup`
//!     are not compiled; `DEBUG` is off, so `opentrace`/`trargs` and
//!     every `TRACE` are compiled out.
//!   * The `#ifndef linux` real/effective id check around `$ENV` is not
//!     compiled on Linux and is noted where it would go.
//!   * `FLUSHERR` is never defined in the dash build, so the
//!     `flushout(out2)` calls guarded by it are absent here too.

use core::ptr::{addr_of_mut, null_mut};
use libc::{c_char, c_int};
use std::ffi::CStr;
use std::io::Write;

use crate::error::{FORCEINTON, jmploc};
use crate::eval::{EV_EXIT, SKIPFUNC, SKIPFUNCDEF, evalskip};
use crate::jobs::SHOW_CHANGED;

/* pid of main shell */
pub static mut rootpid: c_int = 0;
/* pid of current shell */
pub static mut mypid: c_int = 0;
/* shell level: 0 for the main shell, 1 for its children, and so on */
pub static mut shlvl: c_int = 0;

/* glibc sucks — `main()` caches `__errno_location()` here so that the
 * `errno` macro does not repeat the TLS lookup. The port reads errno
 * straight from libc, which is behaviourally identical; the cache is
 * still populated so anything reading it observes the same pointer. */
pub static mut dash_errno: *mut c_int = null_mut();

/* `MKINIT struct jmploc main_handler;` — the outermost handler. The
 * generated `FORKRESET` block re-points `handler` at it after a fork so
 * that a child unwinds to its own top level; that block lives in
 * `crate::init`. */
pub static mut main_handler: jmploc = jmploc::new();

/* src/main.h: `#define rootshell (!shlvl)` */
#[inline]
pub unsafe fn rootshell() -> c_int {
    (shlvl == 0) as c_int
}

/* src/options.h: `#define iflag optlist[3]` and friends. */
#[inline]
unsafe fn iflag() -> c_int {
    crate::options::optlist[crate::options::iflag] as c_int
}
#[inline]
unsafe fn Iflag() -> c_int {
    crate::options::optlist[crate::options::Iflag] as c_int
}
#[inline]
unsafe fn sflag() -> c_int {
    crate::options::optlist[crate::options::sflag] as c_int
}

// [spec:dash:def:main.etext-fn]
// [spec:dash:sem:main.etext-fn]
//
// `extern int etext();` is the linker-provided end-of-text symbol; it is
// declared only under `#if PROFILE` (0 in this build) and its address is
// passed to `monitor()` to bound the profiling range. There is nothing
// to reimplement — a Rust build gets profiling from its own tooling — so
// this is the annotated no-op that stands in for the profiling-setup
// site. It is never called.
#[allow(dead_code)]
unsafe fn etext() -> c_int {
    0
}

/*
 * Main routine.  We initialize things, parse the arguments, execute
 * profiles if we're a login shell, and then call cmdloop to execute
 * commands.  The setjmp call sets up the location to jump to when an
 * exception occurs.  When an exception occurs the variable "state"
 * is used to figure out how far we had gotten.
 */

// [spec:dash:def:main.main-fn]
// [spec:dash:sem:main.main-fn]
pub unsafe fn main(argc: c_int, argv: *mut *mut c_char) -> c_int {
    let mut state: c_int; /* volatile */

    dash_errno = libc::__errno_location();

    /* #if PROFILE: monitor(4, etext, profile_buf, sizeof profile_buf, 50); */

    state = 0;

    /* `state` is live across the jump, so it is reached through a raw
     * pointer rather than captured by reference. */
    let state_p: *mut c_int = &mut state;

    /* Where the startup sequence resumes: 0 is the top, 1..4 are the
     * `state1`..`state4` labels, and 5 is `exit:`.
     *
     * `exit:` is inside the loop, and that is load bearing. In the C,
     * `setjmp(main_handler.loc)` is armed in `main`'s own frame, so it
     * stays a live jump target for as long as `main` runs — including
     * during the `exitshell()` at `exit:`. `forkreset()` relies on
     * exactly that: it points `handler` back at `main_handler`, so a
     * subshell forked from inside an EXIT trap raises `EXEXIT` into
     * this frame and leaves through `goto exit`.
     *
     * A `catch_unwind` is not a frame-lifetime jump target — it only
     * catches while its body runs. With `exitshell()` called after the
     * loop, that subshell's unwind had no handler on the stack, escaped
     * `main`, and the child died with Rust's panic status 101, which the
     * trap then reported as `$?`. */
    let mut entry: c_int = 0;

    loop {
        let jumped = crate::eval::setjmp_catch(addr_of_mut!(main_handler), || unsafe {
            let mut pc: c_int = entry;
            loop {
                match pc {
                    0 => {
                        crate::error::handler = addr_of_mut!(main_handler);
                        /* #ifdef DEBUG:
                         *   opentrace();
                         *   trputs("Shell args:  ");  trargs(argv); */
                        rootpid = libc::getpid();
                        mypid = rootpid;
                        crate::init::init();
                        /* `setstackmark(smark)`, popped at `state3` and on
                         * the exception path, bounded what `procargs` and
                         * the profile reads left in the region. */
                        let login: c_int = crate::options::procargs(argv);
                        if login != 0 {
                            *state_p = 1;
                            read_profile(b"/etc/profile\0".as_ptr() as *const c_char);
                            pc = 1; /* fall into state1: */
                        } else {
                            pc = 2; /* the `if (login)` body is skipped */
                        }
                    }
                    1 => {
                        // state1:
                        *state_p = 2;
                        read_profile(b"$HOME/.profile\0".as_ptr() as *const c_char);
                        pc = 2;
                    }
                    2 => {
                        // state2:
                        *state_p = 3;
                        if
                        /* #ifndef linux: getuid() == geteuid() &&
                         *                getgid() == getegid() && */
                        iflag() != 0 {
                            let shinit: *mut c_char =
                                crate::var::lookupvar(b"ENV\0".as_ptr() as *const c_char);
                            if !shinit.is_null() && *shinit != b'\0' as c_char {
                                read_profile(shinit);
                            }
                        }
                        pc = 3;
                    }
                    3 => {
                        // state3:
                        *state_p = 4;
                        if !crate::options::minusc.is_null() {
                            crate::eval::evalstring(
                                crate::options::minusc,
                                if sflag() != 0 { 0 } else { EV_EXIT },
                            );
                        }

                        if sflag() != 0 || crate::options::minusc.is_null() {
                            pc = 4;
                        } else {
                            pc = 5; /* goto exit */
                        }
                    }
                    4 => {
                        /* state4: XXX ??? - why isn't this before the "if"
                         * statement */
                        cmdloop(1);
                        pc = 5; /* falls into exit: */
                    }
                    _ => {
                        // exit:
                        /* #if PROFILE: monitor(0); */
                        /* #if GPROF: _mcleanup(); */
                        crate::trap::exitshell();
                        /* NOTREACHED — exitshell() ends in _exit(). */
                    }
                }
            }
        });

        /* `jumped == 0` is unreachable: every path through the body ends
         * at `exit:`, and `exitshell()` never returns. */
        let _ = jumped;

        /* setjmp returned non-zero: an exception unwound to main. */
        {
            let e: c_int;
            let s: c_int;

            crate::init::exitreset();

            e = crate::error::exception;

            s = *state_p;
            if e == crate::error::EXEND
                || e == crate::error::EXEXIT
                || s == 0
                || iflag() == 0
                || shlvl != 0
            {
                entry = 5; // goto exit
                continue;
            }

            crate::init::reset();

            if e == crate::error::EXINT
            /* #if ATTY: && (!attyset() || equal(termval(), "emacs")) */
            {
                let _ = (*crate::output::stderr()).write_all(b"\n");
            }
            FORCEINTON(); /* enable interrupts */
            entry = if s == 1 {
                1 /* goto state1 */
            } else if s == 2 {
                2 /* goto state2 */
            } else if s == 3 {
                3 /* goto state3 */
            } else {
                4 /* goto state4 */
            };
        }
    }
}

/// Glue for the binary crate root: turns Rust's `Vec<String>` argv into
/// the NUL-terminated `char **` the literal `main` above expects. Not
/// part of `src/main.c`.
/* argv arrives as raw bytes, not `String`: C argv elements are arbitrary
 * NUL-terminated byte strings and dash passes non-UTF-8 through untouched.
 * See the comment in main.rs. */
/// Run the shell to completion on `streams`.
///
/// The `streams` argument is [dec:nsh:host-owns-streams] at the entry
/// point: the shell is *given* its three streams rather than assuming
/// descriptors 0, 1 and 2. A frontend that has already lent the shell the
/// standard descriptors -- which is what `crate::streams::install` does,
/// and what the `dash` binary wants -- passes
/// [`crate::streams::Streams::INHERIT`].
///
/// This still ends in `_exit` rather than returning, because the shell's
/// exception mechanism is C's and `exitshell` terminates the process.
/// Making it return is [dec:nsh:errors-are-values], not this.
pub fn main_fn(argc: c_int, argv: Vec<Vec<u8>>, streams: crate::streams::Streams) -> ! {
    unsafe { crate::streams::set(streams) };
    let mut owned: Vec<*mut c_char> = Vec::with_capacity(argv.len() + 1);
    for a in &argv {
        let mut bytes: Vec<u8> = a.clone();
        bytes.push(0);
        bytes.shrink_to_fit();
        let p = bytes.as_mut_ptr() as *mut c_char;
        core::mem::forget(bytes);
        owned.push(p);
    }
    owned.push(null_mut());
    let p = owned.as_mut_ptr();
    core::mem::forget(owned);
    unsafe {
        main(argc, p);
    }
    /* main() never returns: it ends in exitshell(). */
    std::process::exit(255);
}

/*
 * Read and execute commands.  "Top" is nonzero for the top level command
 * loop; it turns on prompting if the shell is interactive.
 */

// [spec:dash:def:main.cmdloop-fn]
// [spec:dash:sem:main.cmdloop-fn]
unsafe fn cmdloop(top: c_int) -> c_int {
    let mut inter: c_int;
    let mut status: c_int = 0;
    let mut numeof: c_int = 0;

    /* TRACE(("cmdloop(%d) called\n", top)); */
    loop {
        let skip: c_int;

        /* `setstackmark`/`popstackmark` per iteration: the parse tree and
         * everything the command allocated used to live in the region
         * between them. */
        if crate::jobs::jobctl != 0 {
            crate::jobs::showjobs(crate::output::stderr(), SHOW_CHANGED);
        }
        inter = 0;
        if iflag() != 0 && top != 0 {
            inter += 1;
            crate::mail::chkmail();
        }
        let parsed = crate::parser::parsecmd(inter);
        /* showtree(n); DEBUG */
        if let crate::parser::ParseResult::Tree(n) = parsed {
            let i: c_int;

            crate::jobs::job_warning = if crate::jobs::job_warning == 2 { 1 } else { 0 };
            numeof = 0;
            i = crate::eval::evaltree(n.as_ref(), 0);
            if n.is_some() {
                status = i;
            }
        } else {
            if top == 0 || numeof >= 50 {
                break;
            }
            if crate::jobs::stoppedjobs() == 0 {
                if Iflag() == 0 {
                    if iflag() != 0 {
                        let _ = (*crate::output::stderr()).write_all(b"\n");
                    }
                    break;
                }
                let _ = (*crate::output::stderr()).write_all(b"\nUse \"exit\" to leave shell.\n");
            }
            numeof += 1;
        }
        skip = evalskip;
        if skip != 0 {
            evalskip &= !(SKIPFUNC | SKIPFUNCDEF);
            break;
        }
    }

    status
}

/*
 * Read /etc/profile or .profile.  Return on error.
 */

// [spec:dash:def:main.read-profile-fn]
// [spec:dash:sem:main.read-profile-fn]
unsafe fn read_profile(name: *const c_char) {
    let name: *const c_char = crate::parser::expandstr(name);

    if crate::input::setinputfile(
        name,
        crate::input::INPUT_PUSH_FILE | crate::input::INPUT_NOFILE_OK,
    ) < 0
    {
        return;
    }

    cmdloop(0);
    crate::input::popfile();
}

/*
 * Read a file containing shell functions.
 */

// [spec:dash:def:main.readcmdfile-fn]
// [spec:dash:sem:main.readcmdfile-fn]
pub unsafe fn readcmdfile(name: *mut c_char) {
    crate::input::setinputfile(name, crate::input::INPUT_PUSH_FILE);
    cmdloop(0);
    crate::input::popfile();
}

/*
 * Take commands from a file.  To be compatible we should do a path
 * search for the file, which is necessary to find sub-commands.
 */

// [spec:dash:def:main.find-dot-file-fn]
// [spec:dash:sem:main.find-dot-file-fn]
/// The path the running `.` was found at.  See `dotcmd` for why this
/// outlives the frame that built it.
static mut dotfile_kept: Vec<u8> = Vec::new();

/// The C returns a `stalloc`'d copy of the candidate — "This will be
/// freed by the caller", meaning `dotcmd`'s enclosing `popstackmark`.
/// `dotcmd` keeps the pointer in `commandname` for the whole of
/// `cmdloop`, so the bytes have to outlive the call and cannot be a local
/// of this function; the caller owns the buffer and this fills it.
unsafe fn find_dot_file(basename: *mut c_char, out: &mut Vec<u8>) -> *mut c_char {
    let mut fullname: *mut c_char;
    let mut path: *const c_char = crate::var::pathval();
    let mut statb: libc::stat64 = core::mem::zeroed();
    let mut len: c_int;

    /* don't try this for absolute or relative paths */
    if !libc::strchr(basename, '/' as c_int).is_null() {
        return basename;
    }

    loop {
        len = crate::exec::padvance(&mut path, basename);
        if len < 0 {
            break;
        }
        fullname = crate::exec::padvance_result();
        if (crate::exec::pathopt.is_null() || *crate::exec::pathopt == b'f' as c_char)
            && libc::stat64(fullname, &mut statb) == 0
            && (statb.st_mode & libc::S_IFMT) == libc::S_IFREG
        {
            /* This will be freed by the caller. */
            /* `len` is `padvance`'s *allocation* size, one more than the
             * string's length when the PATH component is empty, so the
             * buffer is sized from it and the bytes copied by hand. */
            out.clear();
            debug_assert!(len > 0);
            out.resize(len as usize, 0);
            libc::strcpy(out.as_mut_ptr() as *mut c_char, fullname);
            debug_assert!(libc::strlen(out.as_ptr() as *const c_char) < len as usize);
            return out.as_mut_ptr() as *mut c_char;
        }
    }

    /* not found in the PATH */
    let mut message = Vec::new();
    message.extend_from_slice(CStr::from_ptr(basename).to_bytes());
    message.extend_from_slice(b": not found");
    crate::error::sh_error(&message);
    /* NOTREACHED */
}

// [spec:dash:def:main.dotcmd-fn]
// [spec:dash:sem:main.dotcmd-fn]
pub unsafe fn dotcmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    let mut status: c_int = 0;

    crate::options::nextopt((core::ptr::addr_of!(crate::shell::nullstr) as *const c_char));
    let argv: *mut *mut c_char = crate::options::argptr;

    if !(*argv).is_null() {
        let fullname: *mut c_char;
        let mut dotfile: Vec<u8> = Vec::new();

        fullname = find_dot_file(*argv, &mut dotfile);
        crate::input::setinputfile(fullname, crate::input::INPUT_PUSH_FILE);
        crate::eval::commandname = fullname;
        status = cmdloop(0);
        crate::input::popfile();
        /* `commandname` still points at these bytes when this returns,
         * and `evalbltin`'s epilogue reads it — `flushall(); if
         * (outerr(out1)) sh_warnx("%s: I/O error", commandname);` —
         * *before* restoring `savecmdname`.  The C is safe there because
         * the block is `stalloc`'d and the enclosing mark has not popped;
         * a local `Vec` would be freed one statement too early.
         *
         * So the allocation is handed to a static slot instead of
         * dropped.  Moving a `Vec` moves the header, not the bytes, so
         * `fullname` stays valid — asserted below, because that is the
         * whole reason this line works.  The slot's previous occupant is
         * unreferenced by then: every `evalbltin` restores `commandname`
         * on the way out, so the only window in which a `dotfile` is
         * still named is between `dotcmd` returning and that restore, and
         * no other `dotcmd` can run inside it.
         *
         * The buffer is empty when `find_dot_file` returned its argument
         * without searching — a name containing `/` — and then
         * `commandname` points at the word `evalcommand` expanded, as it
         * does in the C.  The emptiness test is the discriminator, and it
         * is sound because a filled buffer is `strlen(name) + 2` bytes at
         * the very least.  A first draft asserted unconditionally and
         * this is the case that found it. */
        if !dotfile.is_empty() {
            debug_assert_eq!(dotfile.as_ptr(), fullname as *const u8);
            let kept = &mut *addr_of_mut!(dotfile_kept);
            *kept = dotfile;
            debug_assert_eq!(kept.as_ptr(), fullname as *const u8);
        }
    }

    status
}

// [spec:dash:def:main.exitcmd-fn]
// [spec:dash:sem:main.exitcmd-fn]
pub unsafe fn exitcmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    if crate::jobs::stoppedjobs() != 0 {
        return 0;
    }

    if argc > 1 {
        crate::eval::savestatus = crate::mystring::number(*argv.offset(1));
    }

    crate::error::exraise(crate::error::EXEXIT);
    /* NOTREACHED */
}
