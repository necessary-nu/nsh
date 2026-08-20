//! Execute a typed command-line startup request.
//!
//! The command-line crate owns invocation parsing and process-exit policy.
//! This internal module owns only the shell-language startup sequence:
//! profiles, the requested input, interactive recovery, and EXIT traps.
//! Each operation declares where an interactive shell recovers if it fails;
//! no translated label state or non-local jump remains.

// [spec:nsh:req:idiom.operation-modes]
use crate::context::Shell;
use bstr::BStr;
use std::io::Write;

use crate::eval::EvalContext;
use crate::jobs::JobDisplay;
use crate::options::ShellOption;
use crate::source::Startup;
// [spec:nsh:def:idiom.shell-options]

/// Whether this is the top-level shell rather than one of its children.
#[inline]
pub(crate) fn rootshell(sh: &Shell) -> bool {
    sh.shell_level == 0
}

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
    startup: &Startup,
    task: StartupTask,
) -> Result<StartupAdvance, crate::error::Error> {
    match task {
        StartupTask::Initialize => {
            configure_startup(sh, startup)?;
            let next = if startup.login {
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
            if let Some(command) = startup.command_text() {
                match crate::eval::evalstring(
                    sh,
                    command,
                    if startup.reads_stdin() {
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
            if startup.runs_command_loop() {
                Ok(StartupAdvance::Next(StartupTask::CommandLoop))
            } else {
                Ok(StartupAdvance::Finished)
            }
        }
        StartupTask::CommandLoop => Ok(match cmdloop(sh, true)? {
            crate::eval::Flow::Done(_) => StartupAdvance::Finished,
            crate::eval::Flow::Exit { status } => StartupAdvance::Exit(status),
            control => unreachable!("command loop returned local control: {control:?}"),
        }),
    }
}

fn configure_startup(sh: &mut Shell, startup: &Startup) -> Result<(), crate::error::Error> {
    sh.options.command_source = startup.has_command();
    sh.options.set(ShellOption::Stdin, startup.reads_stdin());

    if startup.reads_stdin() && sh.input.stdin_is_tty.is_none() {
        crate::input::input_init(sh);
    }
    if let Some(path) = startup.script_path() {
        crate::input::set_command_input_file(sh, path)?;
    }

    crate::options::optschanged(sh)?;
    crate::trap::refresh_startup_signal_policy(sh);
    Ok(())
}

/// Run one fully parsed startup request on the supplied shell.
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
pub(crate) fn run(sh: &mut Shell, startup: &Startup) -> crate::status::ExitStatus {
    let mut task = StartupTask::Initialize;
    loop {
        match run_startup_task(sh, startup, task) {
            Ok(StartupAdvance::Next(next)) => task = next,
            Ok(StartupAdvance::Finished) => {
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
                    if sh.io.stderr().write_all(b"\n").is_err() {
                        // The interrupt status takes precedence over its courtesy newline.
                    }
                }
                crate::error::clear_interrupt_deferral(&mut sh.interrupt_deferral);
                task = recovery.expect("recoverable startup task has a successor");
            }
        }
    }
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
    top_level: bool,
) -> Result<crate::eval::Flow, crate::error::Error> {
    let mut status = crate::status::ExitStatus::SUCCESS;
    let mut eof_count = 0usize;
    /* `set -i` can change prompting and the other live interactive option
     * effects, but it cannot turn a command file into an interactive input
     * source. Capture that property before the first command can mutate the
     * option table. */
    let interactive_input = sh.options.enabled(ShellOption::Interactive) && top_level;

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
        let interactive = sh.options.enabled(ShellOption::Interactive) && top_level;
        if interactive {
            crate::mail::chkmail(sh)?;
        }
        let parsed = crate::parser::parsecmd(sh, interactive)?;
        if let crate::parser::ParseResult::Tree(n) = parsed {
            sh.jobs.job_warning = sh.jobs.job_warning.advance();
            eof_count = 0;
            let flow = if top_level {
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
                    sh.write_output(crate::output::Dest::Stderr, b"\n")?;
                }
                break;
            }
            if !sh.options.enabled(ShellOption::IgnoreEof) && eof_count >= 50 {
                break;
            }
            if !crate::jobs::stoppedjobs(sh)? {
                if !sh.options.enabled(ShellOption::IgnoreEof) {
                    // [spec:nsh:req:compat.smoosh.interactive-job-prompt]
                    // A real terminal needs a line ending after the user's
                    // EOF keystroke. A forced-interactive pipe has no echoed
                    // keystroke to terminate, so the prompt is already the
                    // complete byte stream.
                    if sh.options.enabled(ShellOption::Interactive)
                        && sh.input.stdin_is_tty == Some(true)
                    {
                        sh.write_output(crate::output::Dest::Stderr, b"\n")?;
                    }
                    break;
                }
                sh.write_output(
                    crate::output::Dest::Stderr,
                    b"\nUse \"exit\" to leave shell.\n",
                )?;
            }
            crate::input::rearm_stdin_after_eof(sh);
            eof_count = eof_count.saturating_add(1);
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
    let name = crate::parser::expandstr(sh, name)?;

    crate::resource::with_resources(sh, |sh, _resources| {
        if !crate::input::setinputfile(
            sh,
            BStr::new(&name),
            crate::input::InputFileOptions::OPTIONAL_PUSHED,
        )? {
            return Ok(crate::eval::Flow::Done((0).into()));
        }

        /* An `exit` in a profile travels out as control flow after the
         * structured input scope has restored the previous frame. */
        cmdloop(sh, false)
    })
}

/*
 * Read a file containing shell functions.
 */

/// Read and execute a file of commands: the `.` built-in's engine, and
/// how a login shell reads its profile.
// [spec:dash:def:main.readcmdfile-fn]
// [spec:dash:sem:main.readcmdfile-fn]
pub(crate) fn readcmdfile(
    sh: &mut Shell,
    name: &BStr,
) -> Result<crate::eval::Flow, crate::error::Error> {
    crate::resource::with_resources(sh, |sh, _resources| {
        crate::input::setinputfile(sh, name, crate::input::InputFileOptions::PUSHED)?;
        cmdloop(sh, false)
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
