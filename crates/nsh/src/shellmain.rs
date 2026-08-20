//! Literal port of `src/main.c` / `src/main.h`.
//! Rules: `docs/spec/port/src/main.md` (the rule ids use the `main.`
//! prefix even though the module is called `shellmain`, since `main` is
//! taken by the binary crate root).
//!
//! Translation notes:
//!   * Startup is a sequence of named operations. Each operation declares
//!     where an interactive shell recovers if it fails; no translated label
//!     state or non-local jump remains.
//!   * `PROFILE` is 0 and `GPROF` undefined, so `monitor()`/`_mcleanup`
//!     are not compiled; `DEBUG` is off, so `opentrace`/`trargs` and
//!     every `TRACE` are compiled out.
//!   * The `#ifndef linux` real/effective id check around `$ENV` is not
//!     compiled on Linux and is noted where it would go.
//!   * `FLUSHERR` is never defined in the dash build, so the
//!     `flushout(out2)` calls guarded by it are absent here too.

// [spec:nsh:req:idiom.operation-modes]
use crate::context::Shell;
use bstr::{BStr, BString};
use core::ffi::c_int;
use std::io::Write;

use crate::eval::EvalContext;
use crate::jobs::JobDisplay;
use crate::options::ShellOption;
// [spec:nsh:def:idiom.shell-options]

/// src/main.h: `#define rootshell (!shlvl)`
#[inline]
pub fn rootshell(sh: &Shell) -> c_int {
    (sh.shell_level == 0) as c_int
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartupTask {
    Initialize,
    SystemProfile,
    UserProfile,
    Environment,
    Command,
    CommandLoop,
}

impl StartupTask {
    const fn recovery(self) -> Option<Self> {
        match self {
            Self::Initialize => None,
            Self::SystemProfile => Some(Self::UserProfile),
            Self::UserProfile => Some(Self::Environment),
            Self::Environment => Some(Self::Command),
            Self::Command | Self::CommandLoop => Some(Self::CommandLoop),
        }
    }
}

enum StartupAdvance {
    Next(StartupTask),
    Finished,
    Exit(Option<crate::status::ExitStatus>),
}

fn advance_after_flow(flow: crate::eval::Flow, next: StartupTask) -> StartupAdvance {
    match flow {
        crate::eval::Flow::Done(_) => StartupAdvance::Next(next),
        crate::eval::Flow::Exit { status } => StartupAdvance::Exit(status),
        control => unreachable!("startup operation returned local control: {control:?}"),
    }
}

// [spec:nsh:req:idiom.jobs-startup-control-flow]
fn run_startup_task(
    sh: &mut Shell,
    argv: &[Vec<u8>],
    task: StartupTask,
) -> Result<StartupAdvance, crate::error::Error> {
    match task {
        StartupTask::Initialize => {
            sh.initialize_from(crate::var::EnvSource::Process)?;
            let next = if crate::options::procargs(sh, argv)? {
                StartupTask::SystemProfile
            } else {
                StartupTask::Environment
            };
            Ok(StartupAdvance::Next(next))
        }
        StartupTask::SystemProfile => Ok(advance_after_flow(
            read_profile(sh, BStr::new(b"/etc/profile"))?,
            StartupTask::UserProfile,
        )),
        StartupTask::UserProfile => Ok(advance_after_flow(
            read_profile(sh, BStr::new(b"$HOME/.profile"))?,
            StartupTask::Environment,
        )),
        StartupTask::Environment => {
            if sh.options.enabled(ShellOption::Interactive)
                && let Some(shinit) = crate::var::lookup_bytes(sh, BStr::new(b"ENV"))
                    .filter(|value| !value.is_empty())
            {
                let flow = read_profile(sh, BStr::new(shinit.as_slice()))?;
                if !matches!(flow, crate::eval::Flow::Done(_)) {
                    return Ok(advance_after_flow(flow, StartupTask::Command));
                }
            }
            Ok(StartupAdvance::Next(StartupTask::Command))
        }
        StartupTask::Command => {
            if let Some(command) = sh.options.minusc.clone() {
                match crate::eval::evalstring(
                    sh,
                    BStr::new(command.as_slice()),
                    if sh.options.enabled(ShellOption::Stdin) {
                        EvalContext::DEFAULT
                    } else {
                        EvalContext::EXITING
                    },
                )? {
                    crate::eval::Flow::Done(status) | crate::eval::Flow::Return { status, .. } => {
                        sh.status = status
                    }
                    crate::eval::Flow::Exit { status } => return Ok(StartupAdvance::Exit(status)),
                    control => {
                        unreachable!("command option returned local control: {control:?}")
                    }
                }
            }
            if sh.options.enabled(ShellOption::Stdin) || sh.options.minusc.is_none() {
                Ok(StartupAdvance::Next(StartupTask::CommandLoop))
            } else {
                Ok(StartupAdvance::Finished)
            }
        }
        StartupTask::CommandLoop => Ok(match cmdloop(sh, 1)? {
            crate::eval::Flow::Done(_) => StartupAdvance::Finished,
            crate::eval::Flow::Exit { status } => StartupAdvance::Exit(status),
            control => unreachable!("command loop returned local control: {control:?}"),
        }),
    }
}

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
// [spec:nsh:req:idiom.evaluator-control-flow]
pub fn main(sh: &mut Shell, argv: &[Vec<u8>]) -> crate::status::ExitStatus {
    /* #if PROFILE: monitor(4, etext, profile_buf, sizeof profile_buf, 50); */
    let mut task = StartupTask::Initialize;
    loop {
        match run_startup_task(sh, argv, task) {
            Ok(StartupAdvance::Next(next)) => task = next,
            Ok(StartupAdvance::Finished) => {
                /* #if PROFILE: monitor(0); */
                /* #if GPROF: _mcleanup(); */
                return crate::trap::exitshell(sh, None);
            }
            Ok(StartupAdvance::Exit(status)) => {
                if let Some(status) = status {
                    sh.status = status;
                }
                sh.clear_evaluation_resources();
                return crate::trap::exitshell(sh, status);
            }
            Err(error) => {
                let interrupted = error.is_interrupt();
                let unrecoverable_read = error.is_unrecoverable_read();
                sh.status = error.status();
                drop(error);
                sh.clear_evaluation_resources();

                // [spec:posix:req:exit.shell-error-consequences]
                // [spec:posix:req:exit.unrecoverable-read-error]
                let recovery = task.recovery();
                if unrecoverable_read
                    || recovery.is_none()
                    || !sh.options.enabled(ShellOption::Interactive)
                    || sh.shell_level != 0
                {
                    return crate::trap::exitshell(sh, None);
                }

                sh.recover_command_loop();
                if interrupted {
                    /* #if ATTY: && (!attyset() || equal(termval(), "emacs")) */
                    let _ = sh.io.stderr().write_all(b"\n");
                }
                crate::error::clear_interrupt_deferral(sh);
                task = recovery.expect("recoverable startup task has a successor");
            }
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
        return crate::status::ExitStatus::from_code(2);
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
    let mut status = crate::status::ExitStatus::SUCCESS;
    let mut numeof: c_int = 0;
    /* `set -i` can change prompting and the other live interactive option
     * effects, but it cannot turn a command file into an interactive input
     * source. Capture that property before the first command can mutate the
     * option table. */
    let interactive_input = sh.options.enabled(ShellOption::Interactive) && top != 0;

    /* TRACE(("cmdloop(%d) called\n", top)); */
    loop {
        /* `setstackmark`/`popstackmark` per iteration: the parse tree and
         * everything the command allocated used to live in the region
         * between them. */
        // [spec:nsh:def:idiom.job-control-model]
        if sh.jobs.jobctl {
            /* An interrupt taken while announcing changed jobs leaves
             * through the read-eval loop, like any other. */
            crate::jobs::showjobs(sh, crate::output::Dest::Stderr, JobDisplay::Changed)?;
        }
        inter = 0;
        if sh.options.enabled(ShellOption::Interactive) && top != 0 {
            inter += 1;
            crate::mail::chkmail(sh);
        }
        let parsed = crate::parser::parsecmd(sh, inter)?;
        /* showtree(n); DEBUG */
        if let crate::parser::ParseResult::Tree(n) = parsed {
            sh.jobs.job_warning = if sh.jobs.job_warning == 2 { 1 } else { 0 };
            numeof = 0;
            let flow = if top != 0 {
                crate::eval::eval_top_level(sh, n.as_ref(), EvalContext::DEFAULT)
            } else {
                crate::eval::evaltree(sh, n.as_ref(), EvalContext::DEFAULT)
            }?;
            match flow {
                crate::eval::Flow::Done(i) => {
                    if n.is_some() {
                        status = i;
                    }
                }
                crate::eval::Flow::Return {
                    status: return_status,
                    ..
                } => {
                    sh.status = return_status;
                    status = return_status;
                    break;
                }
                control => return Ok(control),
            }
        } else {
            // Only the interactive top-level loop may treat EOF as a request
            // for another input record. A command file has ended even when a
            // script enabled the interactive-only `ignoreeof` option.
            if !interactive_input {
                /* Preserve dash's line termination when a command file used
                 * the runtime `set -i` extension: prompting may be live, but
                 * EOF still terminates this non-interactive input source. */
                if !sh.options.enabled(ShellOption::IgnoreEof)
                    && sh.options.enabled(ShellOption::Interactive)
                {
                    let _ = sh.io.stderr().write_all(b"\n");
                }
                break;
            }
            if !sh.options.enabled(ShellOption::IgnoreEof) && numeof >= 50 {
                break;
            }
            if crate::jobs::stoppedjobs(sh) == 0 {
                if !sh.options.enabled(ShellOption::IgnoreEof) {
                    // [spec:nsh:req:compat.smoosh.interactive-job-prompt]
                    // A real terminal needs a line ending after the user's
                    // EOF keystroke. A forced-interactive pipe has no echoed
                    // keystroke to terminate, so the prompt is already the
                    // complete byte stream.
                    if sh.options.enabled(ShellOption::Interactive) && sh.input.stdin_istty != 0 {
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
    }

    Ok(crate::eval::Flow::Done((status).into()))
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
/// child the last disjunct is true: an exit, a `set -e` abort, a diagnostic,
/// or an interrupt all terminate that child. There is no recovery task, so
/// it clears evaluation resources and then runs `exitshell`.
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
    sh.clear_evaluation_resources();
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

    crate::resource::with_resources(sh, |sh, _resources| {
        if !crate::input::setinputfile(
            sh,
            BStr::new(crate::mystring::cstr_prefix(&name)),
            crate::input::INPUT_PUSH_FILE | crate::input::INPUT_NOFILE_OK,
        )? {
            return Ok(crate::eval::Flow::Done((0).into()));
        }

        /* An `exit` in a profile travels out as control flow after the
         * structured input scope has restored the previous frame. */
        cmdloop(sh, 0)
    })
}

/*
 * Read a file containing shell functions.
 */

/// Read and execute a file of commands: the `.` built-in's engine, and
/// how a login shell reads its profile.
// [spec:dash:def:main.readcmdfile-fn]
// [spec:dash:sem:main.readcmdfile-fn]
pub fn readcmdfile(sh: &mut Shell, name: &BStr) -> Result<crate::eval::Flow, crate::error::Error> {
    crate::resource::with_resources(sh, |sh, _resources| {
        crate::input::setinputfile(sh, name, crate::input::INPUT_PUSH_FILE)?;
        cmdloop(sh, 0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_recovery_follows_tasks() {
        assert_eq!(StartupTask::Initialize.recovery(), None);
        assert_eq!(
            StartupTask::SystemProfile.recovery(),
            Some(StartupTask::UserProfile)
        );
        assert_eq!(
            StartupTask::UserProfile.recovery(),
            Some(StartupTask::Environment)
        );
        assert_eq!(
            StartupTask::Environment.recovery(),
            Some(StartupTask::Command)
        );
        assert_eq!(
            StartupTask::Command.recovery(),
            Some(StartupTask::CommandLoop)
        );
        assert_eq!(
            StartupTask::CommandLoop.recovery(),
            Some(StartupTask::CommandLoop)
        );
    }
}

/*
 * Take commands from a file.  To be compatible we should do a path
 * search for the file, which is necessary to find sub-commands.
 */
