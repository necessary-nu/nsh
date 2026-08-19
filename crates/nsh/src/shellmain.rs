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
use core::ffi::c_int;
use std::io::Write;

use crate::error::FORCEINTON;
use crate::eval::{EV_EXIT, SKIPFUNC, SKIPFUNCDEF};
use crate::jobs::SHOW_CHANGED;

/* `MKINIT struct jmploc main_handler;` was here — the outermost handler,
 * which the generated `FORKRESET` block re-pointed `handler` at after a
 * fork so that a child unwound to its own top level. Nothing unwinds, so
 * there is no handler and no static to hold one. */

/// src/main.h: `#define rootshell (!shlvl)`
#[inline]
pub fn rootshell(sh: &Shell) -> c_int {
    (sh.shell_level == 0) as c_int
}

/* src/options.h: `#define iflag optlist[3]` and friends. */
#[inline]
fn iflag(sh: &crate::context::Shell) -> c_int {
    sh.options.flag(crate::options::iflag) as c_int
}
#[inline]
fn Iflag(sh: &crate::context::Shell) -> c_int {
    sh.options.flag(crate::options::Iflag) as c_int
}
#[inline]
fn sflag(sh: &crate::context::Shell) -> c_int {
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
fn etext() -> c_int {
    0
}

/*
 * Main routine.  We initialize things, parse the arguments, execute
 * profiles if we're a login shell, and then call cmdloop to execute
 * commands.  The setjmp call sets up the location to jump to when an
 * exception occurs.  When an exception occurs the variable "state"
 * is used to figure out how far we had gotten.
 */

/// The literal port of `main()` in `src/main.c`, taking the shell it is
/// to run as. [`main_fn`] is what a caller outside the crate reaches.
// [spec:dash:def:main.main-fn]
// [spec:dash:sem:main.main-fn]
// [spec:posix:syn:sh.synopsis]
// [spec:posix:req:sh.command-language-interpreter]
// [spec:posix:req:sh.pathname-expansion-file-size]
// [spec:posix:sem:sh.redirection-offset-maximum]
// [spec:posix:req:sh.utility-syntax-guidelines]
// [spec:posix:req:sh.set-derived-options]
// [spec:posix:def:sh.interactive]
// [spec:posix:req:sh.stderr-diagnostics-only]
// [spec:posix:sem:sh.output-files]
// [spec:posix:req:sh.exit-status-otherwise]
// [spec:posix:req:sh.consequences-of-errors]
// [spec:posix:req:xcu.limits.minimum-values]
// [spec:posix:req:xcu.limits.more-liberal-values]
// [spec:posix:sem:xcu.limits.symbol-retrieval]
// [spec:posix:sem:xcu.limits.reachability-not-guaranteed]
// [spec:posix:def:xcu.limits.symbolic]
// [spec:posix:req:xcu.limits.posix2-symlinks]
// [spec:posix:req:xcu.grammar-notation.implementation-freedom]
// [spec:posix:req:xcu.description.equivalent-functionality]
// [spec:posix:req:xcu.description.declaration-utility]
// [spec:posix:req:xcu.arbitrary-file-size]
// [spec:posix:req:xcu.defaults.exit-status-successful-completion]
// [spec:posix:req:param.env]
// [spec:posix:def:shell.command-language-interpreter]
// [spec:posix:sem:shell.input-sources]
// [spec:posix:req:exit.interactive-abandons-command]
pub fn main(sh: &mut Shell, argv: &[Vec<u8>]) -> crate::status::ExitStatus {
    let mut state: c_int; /* volatile */

    /* #if PROFILE: monitor(4, etext, profile_buf, sizeof profile_buf, 50); */

    state = 0;

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
    let mut explicit_exit_status: Option<c_int> = None;

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
                        sh.root_pid = nsh_platform::process_id() as c_int;
                        sh.current_pid = sh.root_pid;
                        crate::init::init(sh)?;
                        /* `setstackmark(smark)`, popped at `state3` and
                         * on the exception path, bounded what `procargs`
                         * and the profile reads left in the region. */
                        let login: c_int = crate::options::procargs(sh, argv)?;
                        if login != 0 {
                            state = 1;
                            match read_profile(sh, BStr::new(b"/etc/profile"))? {
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
                        state = 2;
                        match read_profile(sh, BStr::new(b"$HOME/.profile"))? {
                            crate::eval::Flow::Done(_) => {}
                            exit @ crate::eval::Flow::Exit { .. } => return Ok(exit),
                        }
                        pc = 2;
                    }
                    2 => {
                        // state2:
                        state = 3;
                        if
                        /* #ifndef linux: getuid() == geteuid() &&
                         *                getgid() == getegid() && */
                        iflag(sh) != 0 {
                            if let Some(shinit) = crate::var::lookup_bytes(sh, BStr::new(b"ENV"))
                                .filter(|value| !value.is_empty())
                            {
                                match read_profile(sh, BStr::new(shinit.as_slice()))? {
                                    crate::eval::Flow::Done(_) => {}
                                    exit @ crate::eval::Flow::Exit { .. } => return Ok(exit),
                                }
                            }
                        }
                        pc = 3;
                    }
                    3 => {
                        // state3:
                        state = 4;
                        if let Some(command) = sh.options.minusc.clone() {
                            /* With EV_EXIT this always ends in
                             * `Flow::Exit`, which is the C's EXEND
                             * reaching the handler and taking
                             * `goto exit`. Returning it here reaches
                             * the same place by the same decision. */
                            match crate::eval::evalstring(
                                sh,
                                BStr::new(command.as_slice()),
                                if sflag(sh) != 0 { 0 } else { EV_EXIT },
                            )? {
                                crate::eval::Flow::Done(_) => {}
                                exit @ crate::eval::Flow::Exit { .. } => return Ok(exit),
                            }
                        }

                        if sflag(sh) != 0 || sh.options.minusc.is_none() {
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
                        leaving = Some(crate::trap::exitshell(sh, explicit_exit_status.take()));
                        return Ok(crate::eval::Flow::END);
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
         * arrive as three different shapes now, and an explicit exit's
         * selected status travels with its control-flow value. */
        let e_is_exit: bool;
        let selected_status: Option<c_int>;
        let interrupted: bool;
        let unrecoverable_read: bool;

        match &outcome {
            /* `exit:` is the only way out of the body that does not
             * reach here, because the `leaving` check above returns
             * first. A `Flow::Done` would mean the loop fell out of `pc`
             * without reaching either, which it cannot. */
            Ok(crate::eval::Flow::Done(_)) => {
                unreachable!("main's body leaves only by exiting or by failing")
            }
            Ok(crate::eval::Flow::Exit { status }) => {
                e_is_exit = true;
                selected_status = *status;
                interrupted = false;
                unrecoverable_read = false;
            }
            Err(e) => {
                e_is_exit = false;
                selected_status = None;
                /* The C read `exception == EXINT` here, for the bare
                 * newline it writes before the next prompt. */
                interrupted = e.is_interrupt();
                unrecoverable_read = e.is_unrecoverable_read();
                /* This is the outermost catch, and the status the raise
                 * took travels in the value now. Everything downstream --
                 * `exitshell`, and an interactive resume's next `$?` --
                 * reads the shell, so the shell is written here. */
                sh.status = e.status();
            }
        }
        drop(outcome);

        /* the handler */
        {
            let s: c_int;

            if let Some(status) = selected_status {
                sh.status = status;
            }
            crate::init::exitreset(sh);

            s = state;
            // [spec:posix:req:exit.shell-error-consequences]
            // [spec:posix:req:exit.unrecoverable-read-error]
            if e_is_exit || unrecoverable_read || s == 0 || iflag(sh) == 0 || sh.shell_level != 0 {
                explicit_exit_status = if e_is_exit { selected_status } else { None };
                entry = 5; // goto exit
                continue;
            }

            crate::init::reset(sh);

            if interrupted
            /* #if ATTY: && (!attyset() || equal(termval(), "emacs")) */
            {
                let _ = sh.io.stderr().write_all(b"\n");
            }
            FORCEINTON(sh); /* enable interrupts */
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
/// descriptors 0, 1 and 2. [`crate::streams::Streams::INHERIT`] snapshots
/// the frontend process's descriptor table into the shell's owned logical
/// table; supplied streams need no process-wide installation step.
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
pub fn main_fn(argv: Vec<Vec<u8>>, streams: crate::streams::Streams) -> crate::status::ExitStatus {
    /* The shell this process runs as. There is one, it is made here, and
     * every function that has been threaded so far reaches its state
     * through the borrow that starts on the next line
     * ([dec:nsh:no-ambient-state]). */
    let Ok(mut sh) = Shell::try_new(streams) else {
        return crate::status::ExitStatus::from_raw(2);
    };
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
    main(&mut sh, &argv)
}

/*
 * Read and execute commands.  "Top" is nonzero for the top level command
 * loop; it turns on prompting if the shell is interactive.
 */

// [spec:dash:def:main.cmdloop-fn]
// [spec:dash:sem:main.cmdloop-fn]
// [spec:posix:req:builtin.set.opt-o-ignoreeof]
pub(crate) fn cmdloop(
    sh: &mut Shell,
    top: c_int,
) -> Result<crate::eval::Flow, crate::error::Error> {
    let mut inter: c_int;
    let mut status: c_int = 0;
    let mut numeof: c_int = 0;
    /* `set -i` can change prompting and the other live interactive option
     * effects, but it cannot turn a command file into an interactive input
     * source. Capture that property before the first command can mutate the
     * option table. */
    let interactive_input = iflag(sh) != 0 && top != 0;

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
            i = crate::eval::flow!(if top != 0 {
                crate::eval::eval_top_level(sh, n.as_ref(), 0)
            } else {
                crate::eval::evaltree(sh, n.as_ref(), 0)
            });
            if n.is_some() {
                status = i;
            }
        } else {
            // Only the interactive top-level loop may treat EOF as a request
            // for another input record. A command file has ended even when a
            // script enabled the interactive-only `ignoreeof` option.
            if !interactive_input {
                /* Preserve dash's line termination when a command file used
                 * the runtime `set -i` extension: prompting may be live, but
                 * EOF still terminates this non-interactive input source. */
                if Iflag(sh) == 0 && iflag(sh) != 0 {
                    let _ = sh.io.stderr().write_all(b"\n");
                }
                break;
            }
            if Iflag(sh) == 0 && numeof >= 50 {
                break;
            }
            if crate::jobs::stoppedjobs(sh) == 0 {
                if Iflag(sh) == 0 {
                    // [spec:nsh:req:compat.smoosh.interactive-job-prompt]
                    // A real terminal needs a line ending after the user's
                    // EOF keystroke. A forced-interactive pipe has no echoed
                    // keystroke to terminate, so the prompt is already the
                    // complete byte stream.
                    if iflag(sh) != 0 && sh.input.stdin_istty != 0 {
                        let _ = sh.io.stderr().write_all(b"\n");
                    }
                    break;
                }
                let _ = sh
                    .io
                    .stderr()
                    .write_all(b"\nUse \"exit\" to leave shell.\n");
            }
            crate::input::rearm_stdin_after_eof(sh);
            numeof = numeof.saturating_add(1);
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
pub(crate) fn exit_from_child(
    sh: &mut Shell,
    outcome: Result<crate::eval::Flow, crate::error::Error>,
) -> ! {
    let selected_status = match &outcome {
        Ok(crate::eval::Flow::Exit { status }) => *status,
        _ => None,
    };
    /* Same as `main`'s handler: the catch writes the status, because
     * `exitshell` below leaves the process with it. */
    if let Err(e) = &outcome {
        sh.status = e.status();
    }
    drop(outcome);
    if let Some(status) = selected_status {
        sh.status = status;
    }
    crate::init::exitreset(sh);
    /* `exitshell` returns now, and this is one of the three `_exit`s that
     * stay: it ends a child the library forked, which
     * [dec:nsh:fork-child-is-a-terminus] makes a terminus rather than a
     * frame. Returning from here would carry the child back up through
     * frames the parent owns. */
    let status = crate::trap::exitshell(sh, selected_status);
    nsh_platform::exit_immediately(status.code().into());
}

/*
 * Read /etc/profile or .profile.  Return on error.
 */

// [spec:dash:def:main.read-profile-fn]
// [spec:dash:sem:main.read-profile-fn]
fn read_profile(sh: &mut Shell, name: &BStr) -> Result<crate::eval::Flow, crate::error::Error> {
    /* `expandstr` hands back the expanded name as bytes now, and
     * `setinputfile` still opens through a `char *`, so the terminator is
     * put back here — on a local this frame owns. The C's pointer was the
     * expansion buffer's base, live only until the next expansion; this
     * one is live until the frame ends, which covers the `cmdloop` below
     * that the C's did not. */
    let mut name: BString = crate::parser::expandstr(sh, name)?;
    name.push(b'\0');

    if !crate::input::setinputfile(
        sh,
        BStr::new(crate::mystring::cstr_prefix(&name)),
        crate::input::INPUT_PUSH_FILE | crate::input::INPUT_NOFILE_OK,
    )? {
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

/// Read and execute a file of commands: the `.` built-in's engine, and
/// how a login shell reads its profile.
// [spec:dash:def:main.readcmdfile-fn]
// [spec:dash:sem:main.readcmdfile-fn]
pub fn readcmdfile(sh: &mut Shell, name: &BStr) -> Result<crate::eval::Flow, crate::error::Error> {
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
