//! The controlling terminal: who owns it, and what it was set to.
//!
//! Job control is mostly this. A foreground job has to own the terminal
//! while it runs and hand it back afterwards, and its terminal settings
//! have to be captured when it stops so they can be restored when it is
//! resumed -- a shell that gets this wrong leaves the user's terminal in
//! whatever mode the last job left it.

use super::*;

/*
 * Turn job control on and off.
 *
 * Note:  This code assumes that the third arg to ioctl is a character
 * pointer, which is true on Berkeley systems but not System V.  Since
 * System V doesn't have job control yet, this isn't a problem now.
 *
 * Called with interrupts off.
 */

// [spec:dash:sem:jobs.xxtcsetpgrp-fn]
pub(crate) fn set_terminal_process_group(
    shell: &mut crate::context::Shell,
    group: ProcessGroupId,
) -> Result<(), Error> {
    let Some(descriptor) = shell.jobs.terminal.take() else {
        return Ok(());
    };
    let result = set_terminal_process_group_on(shell, &descriptor, group.into());
    shell.jobs.terminal = Some(descriptor);
    result
}

// [spec:posix:req:jobctl.save-terminal-settings]
pub(crate) fn capture_shell_terminal_settings(
    shell: &mut crate::context::Shell,
) -> Result<(), Error> {
    if !shell.jobs.job_control || shell.jobs.shell_terminal_settings.is_some() {
        return Ok(());
    }
    let result = {
        let Some(descriptor) = shell.jobs.terminal.as_ref() else {
            return Ok(());
        };
        nsh_platform::TerminalSettings::capture(descriptor)
    };
    match result {
        Ok(settings) => {
            shell.jobs.shell_terminal_settings = Some(settings);
            Ok(())
        }
        Err(error) => Err(terminal_settings_error(
            shell,
            b"Cannot save shell tty settings",
            error,
        )),
    }
}

pub(crate) fn apply_saved_job_terminal_settings(
    shell: &crate::context::Shell,
    job_id: JobId,
) -> std::io::Result<()> {
    let Some(settings) = shell.jobs[job_id].terminal_settings.as_ref() else {
        return Ok(());
    };
    let Some(descriptor) = shell.jobs.terminal.as_ref() else {
        return Ok(());
    };
    settings.apply(descriptor)
}

pub(crate) fn terminal_settings_error(
    shell: &mut crate::context::Shell,
    operation: &[u8],
    error: std::io::Error,
) -> Error {
    let mut message = operation.to_vec();
    message.extend_from_slice(b" (");
    message.extend_from_slice(shell.locale.error_message(&error).as_bytes());
    message.push(b')');
    shell.diagnostics().shell_error(&message)
}

fn acquire_control_terminal(
    shell: &mut crate::context::Shell,
) -> Result<Option<Descriptor>, Error> {
    let terminal_path = nsh_platform::controlling_terminal_path().to_shell_bytes();
    if let Some(opened) = crate::redirection::open_file(
        shell,
        BStr::new(&terminal_path),
        nsh_platform::OpenMode::ReadWrite,
        true,
    )? {
        return crate::redirection::move_descriptor_above(shell, opened).map(Some);
    }

    let candidate = [
        LogicalDescriptor::STDERR,
        LogicalDescriptor::STDOUT,
        LogicalDescriptor::STDIN,
    ]
    .into_iter()
    .find(|candidate| {
        shell
            .descriptors
            .get(*candidate)
            .as_ref()
            .is_some_and(nsh_platform::is_terminal)
    });
    match candidate {
        Some(candidate) => crate::redirection::copy_slot_above(shell, candidate),
        None => Ok(None),
    }
}

fn await_foreground_group(
    shell: &crate::context::Shell,
    terminal: &Descriptor,
) -> Option<ProcessGroupState> {
    loop {
        let group = loop {
            match nsh_platform::foreground_process_group(terminal) {
                Ok(group) => break Some(group),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break None,
            }
        }?;
        if group == nsh_platform::current_process_group()
            || !shell.options.enabled(ShellOption::Interactive)
        {
            return Some(group);
        }
        if nsh_platform::send_signal(
            ProcessTarget::CurrentProcessGroup,
            nsh_platform::SignalRequest::Deliver(nsh_platform::terminal_input_signal()),
        )
        .is_err()
        {
            // A failed self-stop means this shell cannot acquire the terminal.
            return None;
        }
    }
}

// [spec:dash:sem:jobs.setjobctl-fn]
// [spec:posix:def:jobctl.definition]
// [spec:posix:req:jobctl.initial-foreground-process-group]
// [spec:nsh:def:idiom.logical-descriptors]
/// Turn job control on or off.
///
/// Returns its diagnostic rather than raising it. Two of its three
/// callers are teardown -- `exitshell`, and `optschanged` when
/// `poplocalvars` restores a `local -` option set -- and 4.3's rule is
/// that teardown does not become fallible; the `Result` is here so the
/// callers that *are* ordinary code (`set -m`, `exec`, startup) keep
/// dash's behaviour of abandoning the command, and the teardown callers
/// drop it where the C already swallowed it.
pub fn set_job_control(shell: &mut crate::context::Shell, enabled: bool) -> Result<(), Error> {
    let process_group: Option<ProcessGroupState>;
    let mut descriptor: Option<Descriptor>;

    if enabled == shell.jobs.job_control || !crate::runtime::is_root_shell(shell) {
        return Ok(());
    }
    /* Turning job control *on* is three operations on the host's process:
     * `setpgid(0, rootpid)` and `tcsetpgrp` below, and on the way there
     * possibly a `killpg(0, SIGTTIN)` that stops the host and every
     * sibling with it. [dec:nsh:host-owns-signals] is the same argument
     * that put dispositions behind the host, so the grant lives in the
     * same place rather than in a second one -- see
     * `Host::may_control_terminal`, which answers `docs/api-design.md`
     * §11.5's open question about granularity.
     *
     * Turning it *off* is never gated: `exitshell` and a forked child both
     * do it, and a shell that never had it gives nothing up.
     *
     * One test is enough for the whole feature because the interlock was
     * already there: `xxtcsetpgrp` returns `Ok(())` when `ttyfd < 0`, and
     * `setjobctl` is the only thing that ever sets `ttyfd`. So refusing
     * here also gates `forkchild`'s handoff, `waitforjob`'s hand-back and
     * `fg`'s. */
    if enabled && !shell.host.may_control_terminal() {
        return Ok(());
    }
    if enabled {
        /* `setjobctl` is reached from `exitshell`'s job-control teardown as
         * well as from `optschanged`, so it stays infallible and bridges:
         * a failure here longjmps exactly as the C's `sh_open` did. Making
         * teardown fallible is the shape docs/errors-are-values.md 4.3
         * argues against. */
        descriptor = acquire_control_terminal(shell)?;
        let foreground = descriptor
            .as_ref()
            .and_then(|terminal| await_foreground_group(shell, terminal));
        let terminal_is_accessible = foreground == Some(nsh_platform::current_process_group());
        if !terminal_is_accessible {
            drop(descriptor.take());
            if shell.options.enabled(ShellOption::Interactive) {
                shell
                    .diagnostics()
                    .shell_warning(b"can't access tty; job control turned off");
                shell.options.set(ShellOption::Monitor, false);
                return Ok(());
            }
        }
        shell.jobs.initial_process_group = foreground;
        process_group = Some(ProcessGroupId::from_leader(shell.root_pid).into());
    } else {
        /* turning job control off */
        descriptor = shell.jobs.terminal.take();
        process_group = shell.jobs.initial_process_group;
    }

    crate::trap::configure_signal(shell, nsh_platform::terminal_stop_signal().into());
    crate::trap::configure_signal(shell, nsh_platform::terminal_output_signal().into());
    crate::trap::configure_signal(shell, nsh_platform::terminal_input_signal().into());
    if let (Some(tty), Some(group)) = (descriptor.as_ref(), process_group) {
        let already_in_group = nsh_platform::current_process_group() == group;
        if !already_in_group
            && let Err(error) =
                nsh_platform::set_process_group(ProcessSelector::CurrentProcess, group)
        {
            let mut message = b"Cannot set process group (".to_vec();
            message.extend_from_slice(shell.locale.error_message(&error).as_bytes());
            message.push(b')');
            return Err(shell.diagnostics().shell_error(&message));
        }
        set_terminal_process_group_on(shell, tty, group)?;

        if !enabled {
            drop(descriptor.take());
        }
    }

    shell.jobs.terminal = descriptor;
    shell.jobs.job_control = enabled;
    Ok(())
}

// [spec:dash:sem:jobs.xtcsetpgrp-fn]
fn set_terminal_process_group_on(
    shell: &mut crate::context::Shell,
    descriptor: &impl nsh_platform::AsDescriptor,
    group: ProcessGroupState,
) -> Result<(), Error> {
    let blocked = nsh_platform::BlockedSignals::all()
        .expect("blocking signals around terminal handoff failed");
    let result = nsh_platform::set_foreground_process_group(descriptor, group);
    drop(blocked);

    if let Err(error) = result {
        let mut message = b"Cannot set tty process group (".to_vec();
        message.extend_from_slice(shell.locale.error_message(&error).as_bytes());
        message.push(b')');
        return Err(shell.diagnostics().shell_error(&message));
    }
    Ok(())
}
