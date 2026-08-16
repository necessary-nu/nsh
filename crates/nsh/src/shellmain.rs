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

use crate::context::Shell;
use bstr::{BStr, BString};
use core::ptr::null_mut;
use libc::{c_char, c_int};
use std::ffi::CStr;
use std::io::Write;

use crate::error::FORCEINTON;
use crate::eval::{EV_EXIT, SKIPFUNC, SKIPFUNCDEF};
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

/* `MKINIT struct jmploc main_handler;` was here — the outermost handler,
 * which the generated `FORKRESET` block re-pointed `handler` at after a
 * fork so that a child unwound to its own top level. Nothing unwinds, so
 * there is no handler and no static to hold one. */

/* src/main.h: `#define rootshell (!shlvl)` */
#[inline]
pub unsafe fn rootshell() -> c_int {
    (shlvl == 0) as c_int
}

/* src/options.h: `#define iflag optlist[3]` and friends. */
#[inline]
unsafe fn iflag(sh: &crate::context::Shell) -> c_int {
    sh.options.flag(crate::options::iflag) as c_int
}
#[inline]
unsafe fn Iflag(sh: &crate::context::Shell) -> c_int {
    sh.options.flag(crate::options::Iflag) as c_int
}
#[inline]
unsafe fn sflag(sh: &crate::context::Shell) -> c_int {
    sh.options.flag(crate::options::sflag) as c_int
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
pub unsafe fn main(
    sh: &mut Shell,
    argc: c_int,
    argv: *mut *mut c_char,
) -> crate::status::ExitStatus {
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

    /* Set by the `exit:` arm, which is what `exitshell` now hands back
     * instead of ending the process. It has to be a captured local rather
     * than the closure's return type, because `exit:` must stay *inside*
     * the loop for the reason the comment above gives, and the closure's
     * `Result<Flow, Error>` is the shape the handler below reads. */
    let mut leaving: Option<crate::status::ExitStatus> = None;

    loop {
        /* What the body had to say, which the C read off `exception`
         * afterwards. `Ok(Flow::Exit { .. })` is EXEND or EXEXIT, `Err` is
         * EXERROR, and a `jumped` of true is what is left travelling by
         * longjmp -- EXINT, until step F. */
        let outcome = (|| -> Result<crate::eval::Flow, crate::error::Error> {
                let mut pc: c_int = entry;
                loop {
                    match pc {
                        0 => {
                            /* #ifdef DEBUG:
                             *   opentrace();
                             *   trputs("Shell args:  ");  trargs(argv); */
                            rootpid = libc::getpid();
                            mypid = rootpid;
                            crate::init::init(sh)?;
                            /* `setstackmark(smark)`, popped at `state3` and
                             * on the exception path, bounded what `procargs`
                             * and the profile reads left in the region. */
                            let login: c_int = crate::options::procargs(sh, argv)?;
                            if login != 0 {
                                *state_p = 1;
                                match read_profile(
                                    sh,
                                    b"/etc/profile\0".as_ptr() as *const c_char,
                                )? {
                                    crate::eval::Flow::Done(_) => {}
                                    exit @ crate::eval::Flow::Exit { .. } => return Ok(exit),
                                }
                                pc = 1; /* fall into state1: */
                            } else {
                                pc = 2; /* the `if (login)` body is skipped */
                            }
                        }
                        1 => {
                            // state1:
                            *state_p = 2;
                            match read_profile(sh, b"$HOME/.profile\0".as_ptr() as *const c_char)? {
                                crate::eval::Flow::Done(_) => {}
                                exit @ crate::eval::Flow::Exit { .. } => return Ok(exit),
                            }
                            pc = 2;
                        }
                        2 => {
                            // state2:
                            *state_p = 3;
                            if
                            /* #ifndef linux: getuid() == geteuid() &&
                             *                getgid() == getegid() && */
                            iflag(sh) != 0 {
                                let shinit: *mut c_char =
                                    crate::var::lookupvar(sh, b"ENV\0".as_ptr() as *const c_char);
                                if !shinit.is_null() && *shinit != b'\0' as c_char {
                                    match read_profile(sh, shinit)? {
                                        crate::eval::Flow::Done(_) => {}
                                        exit @ crate::eval::Flow::Exit { .. } => return Ok(exit),
                                    }
                                }
                            }
                            pc = 3;
                        }
                        3 => {
                            // state3:
                            *state_p = 4;
                            if !sh.options.minusc.is_null() {
                                /* With EV_EXIT this always ends in
                                 * `Flow::Exit`, which is the C's EXEND
                                 * reaching the handler and taking
                                 * `goto exit`. Returning it here reaches
                                 * the same place by the same decision. */
                                match crate::eval::evalstring(
                                    sh,
                                    sh.options.minusc,
                                    if sflag(sh) != 0 { 0 } else { EV_EXIT },
                                )? {
                                    crate::eval::Flow::Done(_) => {}
                                    exit @ crate::eval::Flow::Exit { .. } => return Ok(exit),
                                }
                            }

                            if sflag(sh) != 0 || sh.options.minusc.is_null() {
                                pc = 4;
                            } else {
                                pc = 5; /* goto exit */
                            }
                        }
                        4 => {
                            /* state4: XXX ??? - why isn't this before the "if"
                             * statement */
                            match cmdloop(sh, 1)? {
                                crate::eval::Flow::Done(_) => {}
                                exit @ crate::eval::Flow::Exit { .. } => return Ok(exit),
                            }
                            pc = 5; /* falls into exit: */
                        }
                        _ => {
                            // exit:
                            /* #if PROFILE: monitor(0); */
                            /* #if GPROF: _mcleanup(); */
                            leaving = Some(crate::trap::exitshell(sh));
                            return Ok(crate::eval::Flow::Exit {
                                by_exitcmd: false,
                            });
                        }
                    }
                }
        })();

        /* `exit:` ran. It used to end the process from inside the closure;
         * it returns a status now, and this is where the status leaves.
         * Checked before the handler because the handler would otherwise
         * run `exitreset` a second time over a shell that has already
         * finished exiting. */
        if let Some(status) = leaving {
            return status;
        }

        /* The C read `exception` here. The three things it distinguished
         * arrive as three different shapes now, and `exitreset` is told
         * which rather than reading a global. */
        let e_is_exit: bool;
        let by_exitcmd: bool;
        let interrupted: bool;

        match &outcome {
            /* `exit:` is the only way out of the body that does not
             * reach here, because the `leaving` check above returns
             * first. A `Flow::Done` would mean the loop fell out of `pc`
             * without reaching either, which it cannot. */
            Ok(crate::eval::Flow::Done(_)) => {
                unreachable!("main's body leaves only by exiting or by failing")
            }
            Ok(crate::eval::Flow::Exit { by_exitcmd: b }) => {
                e_is_exit = true;
                by_exitcmd = *b;
                interrupted = false;
            }
            Err(e) => {
                e_is_exit = false;
                by_exitcmd = false;
                /* The C read `exception == EXINT` here, for the bare
                 * newline it writes before the next prompt. */
                interrupted = e.is_interrupt();
                /* This is the outermost catch, and the status the raise
                 * took travels in the value now. Everything downstream --
                 * `exitshell`'s `savestatus = exitstatus` and its
                 * `_exit(exitstatus)`, and an interactive resume's next
                 * `$?` -- reads the shell, so the shell is written here. */
                sh.status = e.status();
            }
        }
        drop(outcome);

        /* the handler */
        {
            let s: c_int;

            crate::init::exitreset(sh, by_exitcmd);

            s = *state_p;
            if e_is_exit || s == 0 || iflag(sh) == 0 || shlvl != 0 {
                entry = 5; // goto exit
                continue;
            }

            crate::init::reset(sh);

            if interrupted
            /* #if ATTY: && (!attyset() || equal(termval(), "emacs")) */
            {
                let _ = sh.io.stderr().write_all(b"\n");
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
/// **It returns now**, with the status the shell left with.
///
/// It used to end in `_exit`, because `exitshell` did.
/// [dec:nsh:host-owns-the-process] makes ending the host's process
/// something a library may not do on its own authority, and answers it
/// with an absence rather than a grant — so the status comes back and the
/// frontend calls `std::process::exit`.
///
/// A caller that forks and then calls this **must end the child itself**.
/// Before, the child could not return; now it can, and it would carry on
/// executing whatever followed the fork.
pub fn main_fn(
    argc: c_int,
    argv: Vec<Vec<u8>>,
    streams: crate::streams::Streams,
) -> crate::status::ExitStatus {
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
    /* The shell this process runs as. There is one, it is made here, and
     * every function that has been threaded so far reaches its state
     * through the borrow that starts on the next line
     * ([dec:nsh:no-ambient-state]). */
    let mut sh = Shell::new(streams);
    /* And it is a shell that owns its process, so it gets the host that
     * says so. `Shell::new` defaults to `NoHost` because a *library* shell
     * must; calling this function is the caller stating that this process
     * is the shell, which is the grant [dec:nsh:host-owns-signals] asks
     * for. Without this line the frontend would install no handler at all
     * and every signal behaviour would change at once.
     *
     * `attach` before anything that could ask the host to install one: the
     * sink is the only part of the shell a handler may touch, so the host
     * has to be holding it before a handler could exist. Same order
     * `Builder::build` keeps, for the same reason. */
    sh.host = Box::new(crate::host::ProcessHost);
    sh.host.attach(crate::siginbox::signals());
    unsafe { main(&mut sh, argc, p) }
}

/*
 * Read and execute commands.  "Top" is nonzero for the top level command
 * loop; it turns on prompting if the shell is interactive.
 */

// [spec:dash:def:main.cmdloop-fn]
// [spec:dash:sem:main.cmdloop-fn]
pub(crate) unsafe fn cmdloop(
    sh: &mut Shell,
    top: c_int,
) -> Result<crate::eval::Flow, crate::error::Error> {
    let mut inter: c_int;
    let mut status: c_int = 0;
    let mut numeof: c_int = 0;

    /* TRACE(("cmdloop(%d) called\n", top)); */
    loop {
        let skip: c_int;

        /* `setstackmark`/`popstackmark` per iteration: the parse tree and
         * everything the command allocated used to live in the region
         * between them. */
        if sh.jobs.jobctl != 0 {
            /* An interrupt taken while announcing changed jobs leaves
             * through the read-eval loop, like any other. */
            crate::jobs::showjobs(sh, crate::output::Dest::Stderr, SHOW_CHANGED)?;
        }
        inter = 0;
        if iflag(sh) != 0 && top != 0 {
            inter += 1;
            crate::mail::chkmail(sh);
        }
        let parsed = crate::parser::parsecmd(sh, inter)?;
        /* showtree(n); DEBUG */
        if let crate::parser::ParseResult::Tree(n) = parsed {
            let i: c_int;

            sh.jobs.job_warning = if sh.jobs.job_warning == 2 { 1 } else { 0 };
            numeof = 0;
            i = crate::eval::flow!(crate::eval::evaltree(sh, n.as_ref(), 0));
            if n.is_some() {
                status = i;
            }
        } else {
            if top == 0 || numeof >= 50 {
                break;
            }
            if crate::jobs::stoppedjobs(sh) == 0 {
                if Iflag(sh) == 0 {
                    if iflag(sh) != 0 {
                        let _ = sh.io.stderr().write_all(b"\n");
                    }
                    break;
                }
                let _ = sh.io.stderr().write_all(b"\nUse \"exit\" to leave shell.\n");
            }
            numeof += 1;
        }
        skip = sh.eval.evalskip;
        if skip != 0 {
            sh.eval.evalskip &= !(SKIPFUNC | SKIPFUNCDEF);
            break;
        }
    }

    Ok(crate::eval::Flow::Done(status))
}

/// End a forked child, the way `main`'s handler ends one.
///
/// Three children cannot hand their outcome back — `evalbackcmd`'s, most
/// sharply, because it sits under the whole expansion chain, which has no
/// business carrying control flow that exists only on the far side of a
/// `fork`. This is what would have happened to them if they could.
///
/// It is exact rather than approximate, and the reason is `forkchild`'s
/// `shlvl += 1` (`jobs.rs:877`). `main`'s handler tests
/// `e_is_exit || s == 0 || iflag() == 0 || shlvl != 0`, so in *any* forked
/// child the last disjunct is true and every outcome — an exit, a `set -e`
/// abort, a diagnostic, an interrupt — takes `goto exit` and nothing else.
/// There is no resume path to reproduce, so the whole of the handler for a
/// child is `exitreset` and then `exitshell`.
pub(crate) unsafe fn exit_from_child(
    sh: &mut Shell,
    outcome: Result<crate::eval::Flow, crate::error::Error>,
) -> ! {
    let by_exitcmd = matches!(
        outcome,
        Ok(crate::eval::Flow::Exit {
            by_exitcmd: true,
            ..
        })
    );
    /* Same as `main`'s handler: the catch writes the status, because
     * `exitshell` below leaves the process with it. */
    if let Err(e) = &outcome {
        sh.status = e.status();
    }
    drop(outcome);
    crate::init::exitreset(sh, by_exitcmd);
    /* `exitshell` returns now, and this is one of the three `_exit`s that
     * stay: it ends a child the library forked, which
     * [dec:nsh:fork-child-is-a-terminus] makes a terminus rather than a
     * frame. Returning from here would carry the child back up through
     * frames the parent owns. */
    let status = crate::trap::exitshell(sh);
    libc::_exit(status.code().into());
}

/*
 * Read /etc/profile or .profile.  Return on error.
 */

// [spec:dash:def:main.read-profile-fn]
// [spec:dash:sem:main.read-profile-fn]
unsafe fn read_profile(
    sh: &mut Shell,
    name: *const c_char,
) -> Result<crate::eval::Flow, crate::error::Error> {
    /* `expandstr` hands back the expanded name as bytes now, and
     * `setinputfile` still opens through a `char *`, so the terminator is
     * put back here — on a local this frame owns. The C's pointer was the
     * expansion buffer's base, live only until the next expansion; this
     * one is live until the frame ends, which covers the `cmdloop` below
     * that the C's did not. */
    let mut name: BString = crate::parser::expandstr(sh, name)?;
    name.push(b'\0');

    if crate::input::setinputfile(sh,
        name.as_ptr() as *const c_char,
        crate::input::INPUT_PUSH_FILE | crate::input::INPUT_NOFILE_OK,
    )? < 0
    {
        return Ok(crate::eval::Flow::Done(0));
    }

    /* An `exit` in a profile ends the shell before it ever reads a
     * command, so this call is one the exit has to travel out of. */
    let flow = cmdloop(sh, 0)?;
    if let crate::eval::Flow::Exit { .. } = flow {
        return Ok(flow);
    }
    crate::input::popfile(sh);
    Ok(flow)
}

/*
 * Read a file containing shell functions.
 */

// [spec:dash:def:main.readcmdfile-fn]
// [spec:dash:sem:main.readcmdfile-fn]
pub unsafe fn readcmdfile(
    sh: &mut Shell,
    name: *mut c_char,
) -> Result<crate::eval::Flow, crate::error::Error> {
    crate::input::setinputfile(sh, name, crate::input::INPUT_PUSH_FILE)?;
    let flow = cmdloop(sh, 0)?;
    if let crate::eval::Flow::Exit { .. } = flow {
        return Ok(flow);
    }
    crate::input::popfile(sh);
    Ok(flow)
}

/*
 * Take commands from a file.  To be compatible we should do a path
 * search for the file, which is necessary to find sub-commands.
 */
