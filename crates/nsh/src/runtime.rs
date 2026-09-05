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

use crate::evaluation::EvaluationContext;
use crate::jobs::JobDisplay;
use crate::options::ShellOption;
use crate::source::Startup;
// [spec:nsh:def:idiom.shell-options]

/// Whether this is the top-level shell rather than one of its children.
#[inline]
pub(crate) fn is_root_shell(shell: &Shell) -> bool {
    shell.shell_level == 0
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

fn advance_after_flow(flow: crate::evaluation::Flow, next: StartupTask) -> StartupAdvance {
    match flow {
        crate::evaluation::Flow::Done(_) => StartupAdvance::Next(next),
        crate::evaluation::Flow::Exit { status } => StartupAdvance::Exit(status),
        control => unreachable!("startup operation returned local control: {control:?}"),
    }
}

// [spec:nsh:req:idiom.jobs-startup-control-flow]
fn run_startup_task(
    shell: &mut Shell,
    startup: &Startup,
    task: StartupTask,
) -> Result<StartupAdvance, crate::error::Error> {
    match task {
        StartupTask::Initialize => {
            configure_startup(shell, startup)?;
            let next = if startup.login {
                StartupTask::SystemProfile
            } else {
                StartupTask::Environment
            };
            Ok(StartupAdvance::Next(next))
        }
        StartupTask::SystemProfile => Ok(advance_after_flow(
            read_profile(shell, BStr::new(b"/etc/profile"))?,
            StartupTask::UserProfile,
        )),
        StartupTask::UserProfile => Ok(advance_after_flow(
            read_profile(shell, BStr::new(b"$HOME/.profile"))?,
            StartupTask::Environment,
        )),
        StartupTask::Environment => {
            if shell.options.enabled(ShellOption::Interactive)
                && let Some(shinit) = crate::variables::lookup_bytes(shell, BStr::new(b"ENV"))
                    .filter(|value| !value.is_empty())
            {
                let flow = read_profile(shell, BStr::new(shinit.as_slice()))?;
                if !matches!(flow, crate::evaluation::Flow::Done(_)) {
                    return Ok(advance_after_flow(flow, StartupTask::Command));
                }
            }
            Ok(StartupAdvance::Next(StartupTask::Command))
        }
        StartupTask::Command => {
            if let Some(command) = startup.command_text() {
                match crate::evaluation::evaluate_string(
                    shell,
                    command,
                    if startup.reads_stdin() {
                        EvaluationContext::DEFAULT
                    } else {
                        EvaluationContext::EXITING
                    },
                )? {
                    crate::evaluation::Flow::Done(status)
                    | crate::evaluation::Flow::Return { status, .. } => shell.status = status,
                    crate::evaluation::Flow::Exit { status } => {
                        return Ok(StartupAdvance::Exit(status));
                    }
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
        StartupTask::CommandLoop => Ok(match command_loop(shell, InputFrame::Stream)? {
            crate::evaluation::Flow::Done(_) => StartupAdvance::Finished,
            crate::evaluation::Flow::Exit { status } => StartupAdvance::Exit(status),
            control => unreachable!("command loop returned local control: {control:?}"),
        }),
    }
}

fn configure_startup(shell: &mut Shell, startup: &Startup) -> Result<(), crate::error::Error> {
    shell.options.command_source = startup.has_command();
    shell.options.set(ShellOption::Stdin, startup.reads_stdin());

    if startup.reads_stdin() && shell.input.standard_input_is_terminal.is_none() {
        crate::input::initialize_input(shell);
    }
    if let Some(path) = startup.script_path() {
        crate::input::set_command_input_file(shell, path)?;
        /* The named command file is the bottom of the call stack Bash's
         * `BASH_SOURCE` and `caller` report; `-c` and standard input
         * have no such frame. */
        // [spec:nsh:req:compat.bash.traps-introspection]
        crate::variables::call_stack::set_script_file(shell, path);
    }

    crate::options::apply_option_changes(shell)?;
    if let Some(text) = startup.command_text() {
        /* After the options are applied, because that is what turns the
         * dialect on and the name belongs to it. */
        // [spec:nsh:req:compat.bash.names.ordinary-state]
        crate::variables::special::set_execution_string(shell, text);
    }
    crate::trap::refresh_startup_signal_policy(shell);
    Ok(())
}

/// Run one fully parsed startup request on the supplied shell.
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
pub(crate) fn run(shell: &mut Shell, startup: &Startup) -> crate::status::ExitStatus {
    let mut task = StartupTask::Initialize;
    loop {
        match run_startup_task(shell, startup, task) {
            Ok(StartupAdvance::Next(next)) => task = next,
            Ok(StartupAdvance::Finished) => {
                return crate::trap::exit_shell(shell, None);
            }
            Ok(StartupAdvance::Exit(status)) => {
                if let Some(status) = status {
                    shell.status = status;
                }
                shell.clear_evaluation_resources();
                return crate::trap::exit_shell(shell, status);
            }
            Err(error) => {
                let interrupted = error.is_interrupt();
                let unrecoverable_read = error.is_unrecoverable_read();
                /* The status a shell leaves when a failure ends it before
                 * its input was read out is the failure's own, which the
                 * dialect stamped where the diagnostic was written -- with
                 * one exception, and the exception belongs to the *frame*
                 * rather than to the failure.
                 *
                 * Measured by
                 * `crates/nsh-cli/tests/bash_expansion_error_status.rs`,
                 * which runs every case below through both shells in all
                 * three invocation shapes and holds no expected values of
                 * its own. In the reference's default mode and under
                 * `--posix` alike: `${x?word}`, `${x:?word}`
                 * and a `set -u` read of an unset name end a `-c` shell
                 * with 127 and end a script file or a standard-input shell
                 * with 1. Bash evaluates a `-c` string through
                 * `parse_and_execute`, whose jump handler answers
                 * `EX_NOTFOUND`, and reads a file or standard input through
                 * a loop that answers the failure's own status. That is why
                 * the test below is on the startup task: the same refusal
                 * inside `eval`, inside `.` and inside a function all leave
                 * through this frame and all answer 127 under `-c`, while a
                 * `( )` subshell and a command substitution never reach it
                 * and already agree at 1 and 0.
                 *
                 * The default dialect is not in this: dash answers 2 for
                 * every invocation shape and so does this shell. */
                // [spec:nsh:req:compat.bash.error-boundary]
                shell.status = if task == StartupTask::Command
                    && error.is_expansion()
                    && shell.options.dialect() == crate::options::Dialect::Bash
                {
                    crate::status::ExitStatus::NOT_FOUND
                } else {
                    error.status()
                };
                drop(error);
                shell.clear_evaluation_resources();

                // [spec:posix:req:exit.shell-error-consequences]
                // [spec:posix:req:exit.unrecoverable-read-error]
                let recovery = task.recovery();
                if unrecoverable_read
                    || recovery.is_none()
                    || !shell.options.enabled(ShellOption::Interactive)
                    || shell.shell_level != 0
                {
                    return crate::trap::exit_shell(shell, None);
                }

                shell.recover_command_loop();
                if interrupted && shell.io.stderr().write_all(b"\n").is_err() {
                    // The interrupt status takes precedence over its courtesy newline.
                }
                crate::error::clear_interrupt_deferral(&mut shell.interrupt_deferral);
                task = recovery.expect("recoverable startup task has a successor");
            }
        }
    }
}

/*
 * Read and execute commands.  "Top" is nonzero for the top level command
 * loop; it turns on prompting if the shell is interactive.
 */

/// Which input a command loop is reading, which decides two independent
/// things.
///
/// The C's `top` asks only whether this loop prompts, and [`Stream`] is
/// that answer. The second question is asked only at end of input: is this
/// loop's input *the shell's own*?
///
/// Running out of the shell's own input with `-i` live writes a newline,
/// because the input the person was typing into has ended. Running out of
/// a `.` operand or a profile has not ended it -- the loop that pushed the
/// frame carries on and prompts again -- so the newline would land between
/// the operand's output and the next prompt.
///
/// [`Stream`]: InputFrame::Stream
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum InputFrame {
    /// Input this loop may prompt for and counts consecutive end-of-file
    /// on for `ignoreeof`. Both of the command-line entry's own inputs are
    /// this, a script operand included, because a script that runs
    /// `set -i` prompts from the line after it.
    Stream,
    /// The shell's own command file, read by a loop that does not prompt.
    CommandFile,
    /// A frame pushed over one of the two above: a `.` or `source`
    /// operand, or a profile.
    Pushed,
}

// [spec:dash:sem:main.cmdloop-fn]
// [spec:posix:req:builtin.set.opt-o-ignoreeof]
pub(crate) fn command_loop(
    shell: &mut Shell,
    frame: InputFrame,
) -> Result<crate::evaluation::Flow, crate::error::Error> {
    let top_level = frame == InputFrame::Stream;
    /* A `.` operand is the only one of the three that is not the shell's
     * own input, and that is the distinction the variable machinery's
     * arithmetic is recovered at. */
    // [spec:nsh:req:compat.bash.error-boundary]
    let record_frame = match frame {
        InputFrame::Stream => crate::evaluation::RecordFrame::Stream,
        InputFrame::CommandFile => crate::evaluation::RecordFrame::CommandFile,
        InputFrame::Pushed => crate::evaluation::RecordFrame::Pushed,
    };
    let mut status = crate::status::ExitStatus::SUCCESS;
    let mut eof_count = 0usize;
    /* `set -i` can change prompting and the other live interactive option
     * effects, but it cannot turn a command file into an interactive input
     * source. Capture that property before the first command can mutate the
     * option table. */
    let interactive_input = shell.options.enabled(ShellOption::Interactive) && top_level;

    loop {
        /* `setstackmark`/`popstackmark` per iteration: the parse tree and
         * everything the command allocated used to live in the region
         * between them. */
        // [spec:nsh:def:idiom.job-control-model]
        if shell.jobs.job_control {
            /* An interrupt taken while announcing changed jobs leaves
             * through the read-eval loop, like any other. */
            crate::jobs::write_jobs(
                shell,
                crate::output::OutputDestination::Stderr,
                JobDisplay::Changed,
            )?;
        }
        let interactive = shell.options.enabled(ShellOption::Interactive) && top_level;
        if interactive {
            crate::mail::check_mail(shell)?;
            crate::prompt::publish_terminal_width(shell);
            /* The last thing before the prompt is drawn, so that what the
             * hook assigns is what `parse_command`'s prompt expansion
             * reads. An `exit` from inside it leaves through here. */
            crate::evaluation::flow!(crate::prompt::run_hook(shell));
        }
        let parsed = crate::parser::parse_command(shell, interactive)?;
        if interactive {
            /* Again, because the window may have been dragged while the
             * shell sat in that read, and the command about to run is
             * what would then read a stale width. */
            crate::prompt::publish_terminal_width(shell);
        }
        if let crate::parser::ParseResult::Tree(command) = parsed {
            shell.jobs.job_warning = shell.jobs.job_warning.advance();
            eof_count = 0;
            /* The shell's own clock, around the record it will report --
             * started after the parse, because the parse is where the
             * shell waited for the person typing. A failed record is
             * still charged what it took. */
            // [spec:nsh:req:interactive.prompt-state]
            let started = crate::prompt::Elapsed::started();
            // [spec:nsh:req:compat.bash.error-boundary]
            let outcome = crate::evaluation::evaluate_record(
                shell,
                command.as_ref(),
                EvaluationContext::DEFAULT,
                record_frame,
            );
            /* A record that parsed to nothing did not run, which is the
             * same distinction the status below draws. */
            if command.is_some() {
                crate::prompt::record_duration(shell, started);
            }
            let flow = outcome?;
            match flow {
                crate::evaluation::Flow::Done(command_status) => {
                    if command.is_some() {
                        status = command_status;
                    }
                }
                crate::evaluation::Flow::Return {
                    status: return_status,
                    ..
                } => {
                    shell.status = return_status;
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
                 * EOF still terminates this non-interactive input source.
                 *
                 * A pushed frame is excluded because the newline belongs to
                 * the input source *ending*, and a `.` operand ending does
                 * not end the source the person is typing into: the loop
                 * that pushed it carries on and prompts again, so the line
                 * would land between the script's output and the next
                 * prompt. dash writes nothing there, and so does the pinned
                 * Bash 5.3 for `.` and `source` alike. */
                if frame != InputFrame::Pushed
                    && !shell.options.enabled(ShellOption::IgnoreEof)
                    && shell.options.enabled(ShellOption::Interactive)
                {
                    shell.write_output(crate::output::OutputDestination::Stderr, b"\n")?;
                }
                break;
            }
            if !shell.options.enabled(ShellOption::IgnoreEof) && eof_count >= 50 {
                break;
            }
            if !crate::jobs::has_stopped_jobs(shell)? {
                if !shell.options.enabled(ShellOption::IgnoreEof) {
                    // [spec:nsh:req:compat.smoosh.interactive-job-prompt]
                    // A real terminal needs a line ending after the user's
                    // EOF keystroke. A forced-interactive pipe has no echoed
                    // keystroke to terminate, so the prompt is already the
                    // complete byte stream.
                    if shell.options.enabled(ShellOption::Interactive)
                        && shell.input.standard_input_is_terminal == Some(true)
                    {
                        shell.write_output(crate::output::OutputDestination::Stderr, b"\n")?;
                    }
                    break;
                }
                shell.write_output(
                    crate::output::OutputDestination::Stderr,
                    b"\nUse \"exit\" to leave shell.\n",
                )?;
            }
            crate::input::rearm_stdin_after_eof(shell);
            eof_count = eof_count.saturating_add(1);
        }
    }

    Ok(crate::evaluation::Flow::Done(status))
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
    shell: &mut Shell,
    outcome: Result<crate::evaluation::Flow, crate::error::Error>,
) -> ! {
    let selected_status = match &outcome {
        Ok(crate::evaluation::Flow::Exit { status }) => *status,
        _ => None,
    };
    /* Same as `main`'s handler: the catch writes the status, because
     * `exitshell` below leaves the process with it. */
    if let Err(error) = &outcome {
        shell.status = error.status();
    }
    drop(outcome);
    if let Some(status) = selected_status {
        shell.status = status;
    }
    shell.clear_evaluation_resources();
    /* `exitshell` returns now, and this is one of the three `_exit`s that
     * stay: it ends a child the library forked, which
     * [dec:nsh:fork-child-is-a-terminus] makes a terminus rather than a
     * frame. Returning from here would carry the child back up through
     * frames the parent owns. */
    let status = crate::trap::exit_shell(shell, selected_status);
    nsh_platform::exit_immediately(status.code().into());
}

/*
 * Read /etc/profile or .profile.  Return on error.
 */

// [spec:dash:sem:main.read-profile-fn]
fn read_profile(
    shell: &mut Shell,
    name: &BStr,
) -> Result<crate::evaluation::Flow, crate::error::Error> {
    let name = crate::parser::expand_string(shell, name)?;

    crate::resource::with_resources(shell, |shell, _resources| {
        if !crate::input::set_input_file(
            shell,
            BStr::new(&name),
            crate::input::InputFileOptions::OPTIONAL_PUSHED,
        )? {
            return Ok(crate::evaluation::Flow::Done((0).into()));
        }

        /* An `exit` in a profile travels out as control flow after the
         * structured input scope has restored the previous frame. */
        command_loop(shell, InputFrame::Pushed)
    })
}

/*
 * Read a file containing shell functions.
 */

/// Read and execute a file of commands: the `.` built-in's engine, and
/// how a login shell reads its profile.
// [spec:dash:sem:main.readcmdfile-fn]
pub(crate) fn read_command_file(
    shell: &mut Shell,
    name: &BStr,
) -> Result<crate::evaluation::Flow, crate::error::Error> {
    crate::resource::with_resources(shell, |shell, _resources| {
        crate::input::set_input_file(shell, name, crate::input::InputFileOptions::PUSHED)?;
        command_loop(shell, InputFrame::Pushed)
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
