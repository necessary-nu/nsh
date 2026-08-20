//! Job control and child-process accounting.
//! Rules: `docs/spec/port/src/jobs.md`.
//!
//! Translation notes:
//!   * Jobs use typed identities and states; C bitfields become `bool`
//!     properties, and completed jobs cannot transition back to active.
//!   * The job table stores `Option<Job>` slots and a separate ordering of
//!     live `JobId`s. Vacancy and current/previous ordering are structural;
//!     no sentinel fields or pointer-link emulation remain. The C's
//!     `growjobtab` relocation pass has nothing left to relocate.
//!   * Command rendering and job lookup are expressed as ordinary iteration
//!     and typed results rather than translated label blocks.

use bstr::{BStr, BString, ByteSlice};
use nsh_platform::{
    ChildStatus, Descriptor, NativeStrExt as _, ProcessGroupId, ProcessGroupState, ProcessId,
    ProcessSelector, ProcessTarget,
};

use crate::descriptors::LogicalDescriptor;
use crate::error::Error;
use crate::nodes::{
    DescriptorRedirectionOperator, DescriptorTarget, FileRedirectionOperator, Node, Redirection,
};
use crate::options::ShellOption;
use crate::output::OutputDestination;
// [spec:nsh:def:idiom.shell-options]

mod model;

pub(crate) use model::{Job, JobId, JobState, JobTable, JobWarning, ProcessRecord};

// ---------------------------------------------------------------------
// src/jobs.h
// ---------------------------------------------------------------------

/// How a newly forked shell participates in job control.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ForkMode {
    Foreground,
    Background,
    WithoutJob,
}

/// The four supported job-list presentations.
///
/// These are alternatives, not composable bits: `-p` replaces the ordinary
/// record with a process-group id, `-l` adds process ids, and asynchronous
/// notification filters the standard presentation to changed jobs.
// [spec:nsh:req:idiom.operation-modes]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum JobDisplay {
    #[default]
    Standard,
    Long,
    ProcessGroup,
    Changed,
}

// ---------------------------------------------------------------------
// src/jobs.c module state
// ---------------------------------------------------------------------

/// How child collection waits when no status is immediately available.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WaitMode {
    Poll,
    Block,
    Command,
    CommandAll,
}

impl WaitMode {
    fn kernel_nonblocking(self) -> bool {
        !matches!(self, Self::Block)
    }

    fn suspends_for_signal(self) -> bool {
        !matches!(self, Self::Poll)
    }

    fn after_observation(self) -> Self {
        if matches!(self, Self::CommandAll) {
            Self::Poll
        } else {
            self
        }
    }
}

fn notify_completion_now(
    mode: WaitMode,
    state: JobState,
    shell_job_control: bool,
    notify: bool,
    job_job_control: bool,
    is_waited_for: bool,
) -> bool {
    mode == WaitMode::Block
        && state == JobState::Done
        && shell_job_control
        && notify
        && job_job_control
        && !is_waited_for
}

/// A job that has not forked yet has no `ProcStat` at all; the C reads
/// its zeroed inline `ps0`. That is reachable: `evalpipe` calls
/// `makejob` before it opens the pipe, so a failing `pipe(2)` leaves a
/// used, zero-process job on the current-job chain for `jobs`, `kill`
/// and `wait` to find. Every reader the C writes as an unconditional
/// `ps[i]` goes through these two. `ps_pid` makes the absence explicit;
/// `ps_cmd` answers with the empty text, where the C reads `ps0.cmd`, a
/// null pointer it then hands to `%s`.
#[inline]
pub(crate) fn process_id(
    shell: &crate::context::Shell,
    job_id: JobId,
    index: usize,
) -> Option<ProcessId> {
    shell.jobs[job_id]
        .processes
        .get(index)
        .map(|process| process.process_id)
}

fn process_id_text(process: Option<ProcessId>) -> String {
    process.map_or_else(|| "0".to_owned(), |process| process.to_string())
}

#[inline]
fn process_command_text(shell: &crate::context::Shell, job_id: JobId, index: usize) -> &BStr {
    shell.jobs[job_id]
        .processes
        .get(index)
        .map_or(BStr::new(b""), |process| process.command_text.as_bstr())
}

/// `%s` of a command text. The bytes are the shell's own — the parser
/// puts control bytes 0x81-0x88 in them — so they go out as bytes and
/// not through a `char *`.
#[inline]
pub(crate) fn write_command_text(
    shell: &mut crate::context::Shell,
    job_id: JobId,
    index: usize,
    destination: OutputDestination,
) -> Result<(), Error> {
    /* The lookup is spelled out here rather than going through `ps_cmd`,
     * which is otherwise these same three lines. The two borrows have to
     * be *field*-disjoint: the write takes `sh.io` mutably and the text is
     * read out of `sh.jobs`, and the compiler can see those are different
     * fields only when both are direct field paths. `ps_cmd` borrows the
     * whole shell, so writing through it becomes a conflict the moment
     * `io` becomes a field. It stays because `getjob`'s command-text
     * search still uses it, and that one only reads. */
    let command_text = shell.jobs[job_id]
        .processes
        .get(index)
        .map_or_else(BString::default, |process| process.command_text.clone());
    shell.write_output(destination, &command_text)
}

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
            .is_some_and(|descriptor| nsh_platform::is_terminal(descriptor))
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
        if let Err(error) = nsh_platform::set_process_group(ProcessSelector::CurrentProcess, group)
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

// [spec:dash:sem:jobs.jobno-fn]
//
// The C recovers the index by subtracting `jobtab` from the pointer.
pub(crate) const fn job_number(job_id: JobId) -> usize {
    job_id.0 + 1
}

// [spec:dash:sem:jobs.sprint-status-fn]
// [spec:posix:def:builtin.jobs.stdout-state-strings]
// [spec:nsh:req:idiom.no-artificial-limits]
fn format_process_status(
    locale: &nsh_platform::Locale,
    output: &mut Vec<u8>,
    status: ChildStatus,
    signal_only: bool,
) -> usize {
    let start = output.len();
    match status {
        ChildStatus::Exited(code) if !signal_only => {
            if code == 0 {
                output.extend_from_slice(b"Done");
            } else {
                output.extend_from_slice(format!("Done({code})").as_bytes());
            }
        }
        ChildStatus::Stopped(signal) if !signal_only => {
            let signal_name = crate::signal_names::SIGNAL_NAMES
                .get(signal.number() as usize)
                .map_or(BStr::new(b""), |name| BStr::new(name.to_bytes()));
            output.extend_from_slice(b"Stopped");
            if !signal_name.is_empty() {
                output.extend_from_slice(b" (SIG");
                output.extend_from_slice(signal_name);
                output.push(b')');
            }
        }
        ChildStatus::Signaled {
            signal,
            core_dumped,
        } if !signal_only
            || (signal != nsh_platform::interrupt_signal()
                && signal != nsh_platform::pipe_signal()) =>
        {
            let description = locale.signal_description(signal);
            output.extend_from_slice(&description);
            if core_dumped {
                output.extend_from_slice(b" (core dumped)");
            }
        }
        ChildStatus::Continued if !signal_only => {
            output.extend_from_slice(b"Running");
        }
        _ => {}
    }
    output.len() - start
}

// [spec:dash:sem:jobs.showjob-fn]
// [spec:posix:req:builtin.jobs.remove-reported-job]
// [spec:posix:req:builtin.jobs.stdout-p-format]
// [spec:posix:req:builtin.jobs.stdout-current-field]
// [spec:posix:req:builtin.jobs.stdout-state-substitution]
// [spec:posix:req:builtin.jobs.stdout-l-format]
// [spec:posix:req:builtin.jobs.stdout-default-format]
// [spec:posix:req:jobctl.suspended-job-message]
pub(crate) fn write_job(
    shell: &mut crate::context::Shell,
    destination: OutputDestination,
    job_id: JobId,
    mode: JobDisplay,
) -> Result<(), Error> {
    let process_count: usize;
    let mut column: usize;
    let indent: usize;
    let mut summary = Vec::new();

    if matches!(mode, JobDisplay::ProcessGroup) {
        /* just output process (group) id of pipeline */
        /* The pid is read out before the write starts rather than inside
         * its argument list: `ps_pid` borrows the shell and the write
         * borrows `sh.io`, and evaluating one inside the other is the
         * conflict `Dest` exists to keep out of these functions. */
        let process_id = process_id_text(process_id(shell, job_id, 0));
        shell.write_output_fmt(destination, format_args!("{process_id}\n"))?;
        return Ok(());
    }

    let heading = format!("[{}]   ", job_number(job_id));
    summary.extend_from_slice(heading.as_bytes());
    column = summary.len();
    indent = column;

    if Some(job_id) == shell.jobs.current() {
        summary[column - 2] = b'+';
    } else if Some(job_id) == shell.jobs.previous() {
        summary[column - 2] = b'-';
    }

    if matches!(mode, JobDisplay::Long) {
        let process_id = format!("{} ", process_id_text(process_id(shell, job_id, 0)));
        summary.extend_from_slice(process_id.as_bytes());
        column = summary.len();
    }

    process_count = shell.jobs[job_id].processes.len();

    if shell.jobs[job_id].is_running() {
        /* scopy("Running", s + col) */
        summary.extend_from_slice(b"Running");
        column = summary.len();
    } else {
        /* `psend[-1]`: a job leaves the running state only through `waitone`,
         * which needs a process to have exited to do it. */
        let mut status = shell.jobs[job_id].processes[process_count - 1]
            .status
            .expect("a completed job has a child status");
        if shell.jobs[job_id].is_stopped() {
            status = shell.jobs[job_id]
                .stop_status
                .expect("a stopped job records its stop status");
        }
        column += format_process_status(&shell.locale, &mut summary, status, false);
    }

    let line_count = if matches!(mode, JobDisplay::Long) {
        process_count.max(1)
    } else {
        1
    };
    for process_index in 0..line_count {
        let (mut record, line_column) = if process_index == 0 {
            (summary.clone(), column)
        } else {
            let continuation = format!(
                " |\n{space:>width$}{} ",
                process_id_text(process_id(shell, job_id, process_index)),
                space = ' ',
                width = indent,
            );
            let prefix = continuation.into_bytes();
            let column = prefix.len() - 3;
            (prefix, column)
        };

        let width = 33usize.saturating_sub(line_column);
        record.resize(record.len() + width.max(1), b' ');
        shell.write_output(destination, &record)?;
        write_command_text(shell, job_id, process_index, destination)?;
    }
    if matches!(mode, JobDisplay::Long) {
        shell.write_output(destination, b"\n")?;
    } else {
        write_pipeline(shell, job_id, destination)?;
    }

    shell.jobs[job_id].changed = false;

    if shell.jobs[job_id].is_done() {
        remove_job(&mut shell.interrupt_deferral, &mut shell.jobs, job_id);
    }
    Ok(())
}

/*
 * Print a list of jobs.  If "change" is nonzero, only print jobs whose
 * statuses have changed since the last call to showjobs.
 */

// [spec:dash:sem:jobs.showjobs-fn]
// [spec:posix:req:jobctl.background-job-suspended-message]
// [spec:posix:req:jobctl.background-job-completion-message]
// [spec:posix:req:jobctl.non-interactive-message-timing]
pub(crate) fn write_jobs(
    shell: &mut crate::context::Shell,
    destination: OutputDestination,
    mode: JobDisplay,
) -> Result<(), Error> {
    /* If not even one job changed, there is nothing to do */
    /* `DOWAIT_NONBLOCK`, so the wait cannot block and the poll inside it
     * has nothing to notice; the `?` is the type saying so rather than a
     * path anyone expects to take. */
    reap_children(shell, WaitMode::Poll, None)?;

    /* `showjob` may remove a completed entry, so traverse a stable copy of
     * the explicit presentation order. */
    for index in shell.jobs.order_snapshot() {
        if !matches!(mode, JobDisplay::Changed) || shell.jobs[index].changed {
            write_job(shell, destination, index, mode)?;
        }
    }
    Ok(())
}

/*
 * Mark a job structure as unused.
 */

// [spec:dash:sem:jobs.freejob-fn]
fn remove_job(
    interrupts: &mut crate::error::InterruptDeferral,
    jobs: &mut JobTable,
    job_id: JobId,
) {
    interrupts.run_with(jobs, |jobs| {
        /* Taking the occupied slot releases all owned process text and terminal
         * state exactly once, while `JobTable::remove` also repairs ordering. */
        drop(jobs.remove(job_id));
    });
}

/// Remove a successfully waited, completed job from both the job list and
/// the set of process IDs known to this shell environment.
// [spec:posix:req:builtin.wait.remove-waited-for-pid]
pub(crate) fn remove_waited_job(
    interrupts: &mut crate::error::InterruptDeferral,
    jobs: &mut JobTable,
    job_id: JobId,
) {
    if jobs[job_id].is_done() {
        remove_job(interrupts, jobs, job_id);
    }
}

/*
 * Convert a job name to a job structure.
 */

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum JobLookupError {
    NoSuch,
    NoPrevious,
    Ambiguous,
    NoCurrent,
    NoControl,
}

fn lookup_job(shell: &crate::context::Shell, name: Option<&BStr>) -> Result<JobId, JobLookupError> {
    let current = shell.jobs.current();
    let Some(name) = name else {
        return current.ok_or(JobLookupError::NoCurrent);
    };
    // [spec:dash:sem:mystring.prefix-fn]
    let Some(mut pattern) = name.strip_prefix(b"%") else {
        return Err(JobLookupError::NoSuch);
    };

    match pattern {
        b"" | b"+" | b"%" => return current.ok_or(JobLookupError::NoCurrent),
        b"-" => return shell.jobs.previous().ok_or(JobLookupError::NoPrevious),
        _ => {}
    }

    if let Some(number) = crate::number::parse_decimal(BStr::new(pattern))
        && let Ok(number) = usize::try_from(number)
        && let Some(index) = number
            .checked_sub(1)
            .filter(|index| *index < shell.jobs.slots.len())
    {
        return shell.jobs.slots[index]
            .as_ref()
            .map(|_| JobId(index))
            .ok_or(JobLookupError::NoSuch);
    }

    let substring = pattern.first() == Some(&b'?');
    if substring {
        pattern = &pattern[1..];
    }
    let mut found = None;
    for id in shell.jobs.order_snapshot() {
        let command = process_command_text(shell, id, 0);
        let matches = if substring {
            command.contains_str(pattern)
        } else {
            command.starts_with(pattern)
        };
        if matches && found.replace(id).is_some() {
            return Err(JobLookupError::Ambiguous);
        }
    }
    found.ok_or(JobLookupError::NoSuch)
}

// [spec:dash:sem:jobs.getjob-fn]
pub(crate) fn resolve_job(
    shell: &mut crate::context::Shell,
    name: Option<&BStr>,
    require_control: bool,
) -> Result<JobId, Error> {
    let result = lookup_job(shell, name).and_then(|id| {
        if require_control && !shell.jobs[id].job_control {
            Err(JobLookupError::NoControl)
        } else {
            Ok(id)
        }
    });
    let job_error = match result {
        Ok(id) => return Ok(id),
        Err(error) => error,
    };
    let mut message = Vec::new();
    match job_error {
        JobLookupError::NoSuch => {
            message.extend_from_slice(b"No such job: ");
            message.extend_from_slice(name.unwrap_or(BStr::new(b"(null)")));
        }
        JobLookupError::NoPrevious => message.extend_from_slice(b"No previous job"),
        JobLookupError::Ambiguous => {
            message.extend_from_slice(name.unwrap_or(BStr::new(b"(null)")));
            message.extend_from_slice(b": ambiguous");
        }
        JobLookupError::NoCurrent => message.extend_from_slice(b"No current job"),
        JobLookupError::NoControl => {
            message.extend_from_slice(b"job ");
            message.extend_from_slice(name.unwrap_or(BStr::new(b"(null)")));
            message.extend_from_slice(b" not created under job control");
        }
    }
    Err(shell.diagnostics().shell_error(&message))
}

/*
 * Return a new job structure.
 * Called with interrupts off.
 */

// [spec:dash:sem:jobs.makejob-fn]
// [spec:posix:req:jobctl.job-creation]
// [spec:posix:req:cmd.async-job-number]
// [spec:posix:sem:cmd.async-job-control]
// [spec:posix:req:cmd.async-known-pid-retention]
pub fn create_job(shell: &mut crate::context::Shell, process_capacity: usize) -> JobId {
    let job_id: JobId;
    let mut index: usize;

    index = 0;
    job_id = loop {
        if index >= shell.jobs.slots.len() {
            break reserve_job_slot(&mut shell.jobs);
        }
        let id = JobId(index);
        let Some(job) = shell.jobs.slots[id.0].as_ref() else {
            break id;
        };
        if !job.is_done() || !job.waited {
            index += 1;
            continue;
        }
        if shell.jobs.job_control {
            index += 1;
            continue;
        }
        remove_job(&mut shell.interrupt_deferral, &mut shell.jobs, id);
        break id;
    };
    let mut job = Job::new();
    /* The C picks the inline `ps0` for a single process and `ckmalloc`s
     * an array otherwise; all that decided was where the room came from,
     * so it is the capacity here and the processes are pushed as
     * `forkparent` forks them. */
    if process_capacity > 0 {
        job.processes.reserve_exact(process_capacity);
    }
    if shell.jobs.job_control {
        job.job_control = true;
    }
    shell.jobs.occupy_current(job_id, job);
    job_id
}

// [spec:dash:sem:jobs.growjobtab-fn]
//
// The C's relocation pass has no counterpart: jobs own their process arrays,
// identities are indices, and ordering contains values rather than pointers.
fn reserve_job_slot(jobs: &mut JobTable) -> JobId {
    let first_new_slot = jobs.slots.len();

    for _ in 0..4 {
        jobs.slots.push(None);
    }
    JobId(first_new_slot)
}

/*
 * Fork off a subshell.  If we are doing job control, give the subshell its
 * own process group.  Jp is a job structure that the job is to be added to.
 * N is the command that will be evaluated by the child.  Both jp and n may
 * be NULL.  The mode parameter can be one of the following:
 *	FORK_FG - Fork off a foreground process.
 *	FORK_BG - Fork off a background process.
 *	FORK_NOJOB - Like FORK_FG, but don't give the process its own
 *		     process group even if job control is on.
 *
 * When job control is turned off, background processes have their standard
 * input redirected to /dev/null (except for the second and later processes
 * in a pipeline).
 *
 * Called with interrupts off.
 */

/// What `forkchild` does with a diagnostic it cannot return.
///
/// `forkchild` runs in the child. An `Err` returned from here would travel
/// through frames copied from the parent and resume work the child must
/// never resume, so this is a terminus. The child ends the way `main`'s
/// handler ends every forked child, which `forkchild`'s own `shlvl += 1`
/// is what guarantees (see `runtime::exit_from_child`). The diagnostic
/// has already been written.
#[cold]
// [spec:dash:sem:jobs.forkchild-fn]
// [spec:posix:req:jobctl.pipeline-process-group]
// [spec:posix:req:jobctl.foreground-process-group-assignment]
// [spec:posix:req:signal.async-list-sigint-sigquit-ignored]
// [spec:posix:req:signal.inherited-actions]
// [spec:posix:req:shenv.subshell-creation]
// [spec:posix:req:shenv.subshell-isolation]
// [spec:posix:req:cmd.async-stdin-devnull]
// [spec:nsh:req:idiom.no-raw-fd-core]
fn initialize_child_process(
    shell: &mut crate::context::Shell,
    job_id: Option<JobId>,
    node: Option<&Node>,
    mode: ForkMode,
) {
    let parent_shell_level: usize;

    nsh_platform::reset_coverage_counters();

    parent_shell_level = shell.shell_level;
    shell.shell_level += 1;

    shell.prepare_fork_child(if mode == ForkMode::WithoutJob {
        node
    } else {
        None
    });

    /* do job control only in root shell */
    shell.jobs.job_control = false;

    /* The C tests `jp->jobctl` without checking `jp`; `jp` is NULL only
     * under FORK_NOJOB, which the first conjunct has already excluded. */
    let controls_process_group = mode != ForkMode::WithoutJob
        && parent_shell_level == 0
        && job_id.is_some_and(|index| shell.jobs[index].job_control);
    if controls_process_group {
        let process_group: ProcessGroupId;
        let active_job: JobId = job_id.unwrap();

        if shell.jobs[active_job].processes.is_empty() {
            process_group = ProcessGroupId::from_leader(nsh_platform::current_process_id());
        } else {
            process_group =
                ProcessGroupId::from_leader(shell.jobs[active_job].processes[0].process_id);
        }
        /* This can fail because we are doing it in the parent also */
        if nsh_platform::set_process_group(ProcessSelector::CurrentProcess, process_group.into())
            .is_err()
        {
            // The parent performs the same race-safe process-group assignment.
        }
        if mode == ForkMode::Foreground {
            set_terminal_process_group(shell, process_group)
                .unwrap_or_else(|error| crate::runtime::exit_from_child(shell, Err(error)));
        }
        crate::trap::configure_signal_in_child(shell, nsh_platform::terminal_stop_signal().into());
        crate::trap::configure_signal_in_child(
            shell,
            nsh_platform::terminal_output_signal().into(),
        );
    } else if mode == ForkMode::Background {
        crate::trap::ignore_signal_in_child(shell, nsh_platform::interrupt_signal().into());
        crate::trap::ignore_signal_in_child(shell, nsh_platform::quit_signal().into());
        if job_id.map_or(false, |index| shell.jobs[index].processes.is_empty()) {
            /* The C closes descriptor 0 and reopens /dev/null, relying on
             * `open` returning the lowest free descriptor to land back on
             * 0. That only works when the shell's stdin *is* 0, so put it
             * where it belongs when the frontend said otherwise. */
            let null_path = nsh_platform::null_device_path().to_shell_bytes();
            let null_descriptor = crate::redirection::open_file(
                shell,
                BStr::new(&null_path),
                nsh_platform::OpenMode::ReadOnly,
                false,
            )
            .unwrap_or_else(|error| crate::runtime::exit_from_child(shell, Err(error)))
            .expect("a mandatory open returns a descriptor");
            if let Err(error) = shell
                .descriptors
                .install_owned(LogicalDescriptor::STDIN, null_descriptor)
            {
                let error =
                    crate::redirection::descriptor_error(shell, LogicalDescriptor::STDIN, error);
                crate::runtime::exit_from_child(shell, Err(error));
            }
            /* Should call reset_input here, but it's harmless
             * for now.
             */
        }
    }
    if parent_shell_level == 0 && shell.options.enabled(ShellOption::Interactive) {
        crate::trap::configure_signal_in_child(shell, nsh_platform::interrupt_signal().into());
        crate::trap::configure_signal_in_child(shell, nsh_platform::quit_signal().into());
        crate::trap::configure_signal_in_child(shell, nsh_platform::termination_signal().into());
    }

    let Some(active_job) = job_id else {
        return;
    };

    remove_job(&mut shell.interrupt_deferral, &mut shell.jobs, active_job);

    if crate::parser::is_simple_command(node, BStr::new(b"jobs")) {
        return;
    }

    for index in shell.jobs.order_snapshot() {
        remove_job(&mut shell.interrupt_deferral, &mut shell.jobs, index);
    }
}

// [spec:dash:sem:jobs.forkparent-fn]
// [spec:posix:req:jobctl.job-number-and-process-id]
// [spec:posix:req:cmd.async-process-id-known]
// [spec:posix:req:cmd.async-job-notification-format]
// [spec:posix:req:cmd.async-non-job-pid-message]
fn record_forked_child(
    shell: &mut crate::context::Shell,
    job_id: Option<JobId>,
    node: Option<&Node>,
    mode: ForkMode,
    process_id: ProcessId,
) -> Result<(), Error> {
    let Some(active_job) = job_id else {
        return Ok(());
    };
    if mode != ForkMode::WithoutJob && shell.jobs[active_job].job_control {
        let process_group: ProcessGroupId;

        if shell.jobs[active_job].processes.is_empty() {
            process_group = ProcessGroupId::from_leader(process_id);
        } else {
            process_group =
                ProcessGroupId::from_leader(shell.jobs[active_job].processes[0].process_id);
        }
        /* This can fail because we are doing it in the child also */
        if nsh_platform::set_process_group(
            ProcessSelector::Process(process_id),
            process_group.into(),
        )
        .is_err()
        {
            // The child performs the same race-safe process-group assignment.
        }
    }
    if mode == ForkMode::Background {
        shell.background_process = Some(process_id); /* set $! */
        shell.jobs.position_running(active_job);
        if shell.options.enabled(ShellOption::Interactive) {
            shell.write_output_fmt(
                OutputDestination::Stderr,
                format_args!("[{}] {process_id}\n", job_number(active_job)),
            )?;
        }
    }
    /* the C's second `if (jp)` is dead after the early return above */
    shell.jobs[active_job].processes.push(ProcessRecord {
        process_id,
        status: None,
        command_text: BString::new(Vec::new()),
    });
    if let Some(node) = node {
        let command_text = render_command(node);
        let last = shell.jobs[active_job].processes.len() - 1;
        shell.jobs[active_job].processes[last].command_text = command_text;
    }
    Ok(())
}

// [spec:dash:sem:jobs.forkshell-fn]
// [spec:posix:req:shenv.subshell-contexts]
// [spec:posix:req:xcurel.process-attributes-additional]
// [spec:posix:req:xcurel.concurrent-execution]
pub fn fork_shell(
    shell: &mut crate::context::Shell,
    job_id: Option<JobId>,
    node: Option<&Node>,
    mode: ForkMode,
) -> Result<nsh_platform::ForkResult, Error> {
    shell.flush_input();

    if mode == ForkMode::Foreground && job_id.is_some_and(|index| shell.jobs[index].job_control) {
        capture_shell_terminal_settings(shell)?;
    }

    let fork = match nsh_platform::fork_process() {
        Ok(nsh_platform::ForkResult::Child) => {
            initialize_child_process(shell, job_id, node, mode);
            nsh_platform::ForkResult::Child
        }
        Ok(nsh_platform::ForkResult::Parent(process_id)) => {
            record_forked_child(shell, job_id, node, mode, process_id)?;
            nsh_platform::ForkResult::Parent(process_id)
        }
        Err(_) => {
            if let Some(job) = job_id {
                remove_job(&mut shell.interrupt_deferral, &mut shell.jobs, job);
            }
            return Err(shell.diagnostics().shell_error(b"Cannot fork"));
        }
    };

    Ok(fork)
}

// [spec:dash:sem:jobs.vforkexec-fn]
// [spec:posix:req:cmd.nonbuiltin-separate-environment]
/// Fork and immediately execute an external command.
///
/// dash uses `vfork` here. Rust command preparation owns and mutates heap
/// allocations, so sharing the parent's address space is unsound: the
/// second external command returned through a stack corrupted by the first.
/// A regular fork preserves the child-terminus rule without shared memory.
pub fn fork_and_execute(
    shell: &mut crate::context::Shell,
    node: &Node,
    arguments: &[&BStr],
    path: &BStr,
    path_index: Option<usize>,
) -> Result<JobId, Error> {
    let job_id: JobId;
    job_id = create_job(shell, 1);

    if shell.jobs[job_id].job_control {
        capture_shell_terminal_settings(shell)?;
    }

    let process_id = match nsh_platform::fork_process() {
        Ok(nsh_platform::ForkResult::Child) => {
            initialize_child_process(shell, Some(job_id), Some(node), ForkMode::Foreground);
            let outcome =
                crate::execution::execute_external_command(shell, arguments, path, path_index);
            crate::runtime::exit_from_child(shell, outcome);
        }
        Ok(nsh_platform::ForkResult::Parent(process_id)) => process_id,
        Err(_) => {
            remove_job(&mut shell.interrupt_deferral, &mut shell.jobs, job_id);
            return Err(shell.diagnostics().shell_error(b"Cannot fork"));
        }
    };
    record_forked_child(
        shell,
        Some(job_id),
        Some(node),
        ForkMode::Foreground,
        process_id,
    )?;

    Ok(job_id)
}

/*
 * Wait for job to finish.
 *
 * Under job control we have the problem that while a child process is
 * running interrupts generated by the user are sent to the child but not
 * to the shell.  This means that an infinite loop started by an inter-
 * active user may be hard to kill.  With job control turned off, an
 * interactive user may place an interactive program inside a loop.  If
 * the interactive program catches interrupts, the user doesn't want
 * these interrupts to also abort the loop.  The approach we take here
 * is to have the shell ignore interrupt signals while waiting for a
 * forground process to terminate, and then send itself an interrupt
 * signal if the child process was terminated by an interrupt signal.
 * Unfortunately, some programs want to do a bit of cleanup and then
 * exit on interrupt; unless these processes terminate themselves by
 * sending a signal to themselves (instead of calling exit) they will
 * confuse this approach.
 *
 * Called with interrupts off.
 */

// [spec:dash:sem:jobs.waitforjob-fn]
// [spec:posix:sem:shell.exit-status-collection]
// [spec:posix:req:jobctl.foreground-process-group-restored]
// [spec:posix:req:signal.trap-deferred-until-foreground-command-completes]
// [spec:posix:sem:cmd.async-status-via-wait]
pub fn wait_for_job(
    shell: &mut crate::context::Shell,
    job_id: Option<JobId>,
) -> Result<crate::status::ExitStatus, Error> {
    let st: crate::status::ExitStatus;
    let mut terminal_error: Option<(&'static [u8], std::io::Error)> = None;

    reap_children(
        shell,
        if job_id.is_some() {
            WaitMode::Block
        } else {
            WaitMode::Poll
        },
        job_id,
    )?;
    let Some(job_id) = job_id else {
        return Ok(shell.status);
    };

    st = job_exit_status(shell, job_id);
    if shell.jobs[job_id].job_control {
        if shell.jobs[job_id].is_stopped() {
            let result = shell
                .jobs
                .terminal
                .as_ref()
                .map(nsh_platform::TerminalSettings::capture);
            match result {
                Some(Ok(settings)) => shell.jobs[job_id].terminal_settings = Some(settings),
                Some(Err(error)) => {
                    terminal_error = Some((b"Cannot save job tty settings", error));
                }
                None => {}
            }
        }
        let shell_group = ProcessGroupId::from_leader(shell.root_pid);
        set_terminal_process_group(shell, shell_group)?;
        if shell.jobs[job_id].is_stopped() {
            if let Some(settings) = shell.jobs.shell_terminal_settings.take() {
                let result = shell
                    .jobs
                    .terminal
                    .as_ref()
                    .map(|descriptor| settings.apply(descriptor));
                if let Some(Err(error)) = result {
                    terminal_error = Some((b"Cannot restore shell tty settings", error));
                }
            }
        } else {
            /* A completed foreground utility owns intentional changes made
             * with `stty`. The saved snapshot exists so a suspended job's
             * private modes cannot strand the shell; applying it after a
             * normal exit would erase the utility's successful result. */
            shell.jobs.shell_terminal_settings = None;
        }
        /*
         * This is truly gross.
         * If we're doing job control, then we did a TIOCSPGRP which
         * caused us (the shell) to no longer be in the controlling
         * session -- so we wouldn't have seen any ^C/SIGINT.  So, we
         * intuit from the subprocess exit status whether a SIGINT
         * occurred, and if so interrupt ourselves.  Yuck.  - mycroft
         */
        if shell.jobs[job_id].interrupted {
            if let Err(error) = nsh_platform::raise_signal(nsh_platform::interrupt_signal()) {
                let mut message = b"Cannot raise interrupt (".to_vec();
                message.extend_from_slice(shell.locale.error_message(&error).as_bytes());
                message.push(b')');
                return Err(shell.diagnostics().shell_error(&message));
            }
        }
    }
    if shell.jobs[job_id].is_done() {
        remove_job(&mut shell.interrupt_deferral, &mut shell.jobs, job_id);
    }
    if let Some((operation, error)) = terminal_error {
        return Err(terminal_settings_error(shell, operation, error));
    }
    Ok(st)
}

/*
 * Wait for a process to terminate.
 */

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WaitOutcome {
    Reaped {
        process: ProcessId,
        status: ChildStatus,
    },
    Interrupted,
    Exhausted,
}

fn record_child_status(job: &mut Job, process: ProcessId, status: ChildStatus) -> Option<JobState> {
    let child = job
        .processes
        .iter_mut()
        .find(|child| child.process_id == process)?;
    child.status = Some(status);

    let stopped = job.processes.iter().find_map(|child| match child.status {
        Some(status @ ChildStatus::Stopped(_)) => Some(status),
        _ => None,
    });
    job.stop_status = stopped;
    if job.processes.iter().any(|child| child.status.is_none()) {
        Some(JobState::Running)
    } else if stopped.is_some() {
        Some(JobState::Stopped)
    } else {
        Some(JobState::Done)
    }
}

// [spec:dash:sem:jobs.waitone-fn]
// [spec:posix:req:jobctl.suspend-on-catchable-signal]
// [spec:posix:req:jobctl.suspend-on-sigstop]
// [spec:posix:req:builtin.set.opt-b-notify]
fn wait_once(
    shell: &mut crate::context::Shell,
    mode: WaitMode,
    job_id: Option<JobId>,
) -> Result<WaitOutcome, Error> {
    let mut thisjob: Option<JobId> = None;
    let mut state = JobState::Running;
    let mut reported_status = None;

    let waited = crate::error::with_interrupts_deferred(shell, |shell| {
        let waited = wait_for_process(shell, mode)?;
        if let WaitOutcome::Reaped { process, status } = waited {
            reported_status = Some(status);
            for id in shell.jobs.order_snapshot() {
                if shell.jobs[id].is_done() {
                    continue;
                }
                let Some(next_state) = record_child_status(&mut shell.jobs[id], process, status)
                else {
                    continue;
                };
                thisjob = Some(id);
                state = next_state;
                if next_state != JobState::Running {
                    shell.jobs[id].changed = true;
                    if shell.jobs[id].transition_to(next_state) {
                        if next_state == JobState::Stopped {
                            shell.jobs.position_stopped(id);
                        }
                    }
                }
                break;
            }
        }
        Ok::<_, Error>(waited)
    })?;

    if thisjob.is_some() && thisjob == job_id {
        let mut message = Vec::new();
        format_process_status(
            &shell.locale,
            &mut message,
            reported_status.expect("a matched job has a reaped status"),
            true,
        );
        if !message.is_empty() {
            message.push(b'\n');
            shell.write_output(OutputDestination::Stderr, &message)?;
        }
    }
    /* A blocking wait can leave an interrupt pending while this structured
     * scope is active. Deliver it only after the caller's prior depth has
     * been restored. */
    if let Some(error) = crate::error::poll_interrupt(shell.interrupt_context()) {
        return Err(error);
    }
    /* A blocking wait for one foreground job can reap a different,
     * background job first.  `-b` makes that completion observable here,
     * before the wait resumes; non-blocking callers already render changed
     * jobs themselves, and a waited-for job is reported by its caller. */
    if let Some(changed_job) = thisjob {
        if notify_completion_now(
            mode,
            state,
            shell.jobs.job_control,
            shell.options.enabled(ShellOption::Notify),
            shell.jobs[changed_job].job_control,
            Some(changed_job) == job_id,
        ) {
            write_job(
                shell,
                OutputDestination::Stderr,
                changed_job,
                JobDisplay::Standard,
            )?;
        }
    }
    Ok(waited)
}

// [spec:dash:sem:jobs.dowait-fn]
pub(crate) fn reap_children(
    shell: &mut crate::context::Shell,
    mode: WaitMode,
    job_id: Option<JobId>,
) -> Result<bool, Error> {
    let child_pending = crate::signal_inbox::signals().child_pending();
    let mut wait_completed: bool;
    let mut waited: WaitOutcome;
    let mut mode = mode;

    if job_id.is_some_and(|index| !shell.jobs[index].is_running()) {
        mode = WaitMode::Poll;
    }

    if mode == WaitMode::Poll && !child_pending {
        return Ok(true);
    }

    wait_completed = true;

    loop {
        waited = wait_once(shell, mode, job_id)?;
        wait_completed &= waited != WaitOutcome::Interrupted;

        mode = mode.after_observation();
        if waited == WaitOutcome::Interrupted
            || job_id.is_some_and(|index| !shell.jobs[index].is_running())
        {
            mode = WaitMode::Poll;
        }
        if waited == WaitOutcome::Exhausted {
            break;
        }
    }

    Ok(wait_completed)
}

/*
 * Do a wait system call.  If block is zero, we return -1 rather than
 * blocking.  If block is DOWAIT_WAITCMD, we return 0 when a signal
 * other than SIGCHLD interrupted the wait.
 *
 * We use sigsuspend in conjunction with a non-blocking wait in
 * order to ensure that waitcmd exits promptly upon the reception
 * of a signal.
 *
 * For code paths other than waitcmd we either use a blocking wait
 * or a non-blocking wait.  For the latter case the caller of dowait
 * must ensure that it is called over and over again until all dead
 * children have been reaped.  Otherwise zombies may linger.
 */

// [spec:dash:sem:jobs.waitproc-fn]
fn wait_for_process(
    shell: &mut crate::context::Shell,
    mode: WaitMode,
) -> Result<WaitOutcome, Error> {
    let nonblocking = mode.kernel_nonblocking();
    let mut waited: WaitOutcome;

    let signals = crate::signal_inbox::signals();
    loop {
        signals.set_child_pending(false);
        loop {
            match nsh_platform::wait_for_any_child(nonblocking, shell.jobs.job_control) {
                Ok(Some((process_id, child_status))) => {
                    waited = WaitOutcome::Reaped {
                        process: process_id,
                        status: child_status,
                    };
                    break;
                }
                Ok(None) => {
                    waited = WaitOutcome::Interrupted;
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => {
                    waited = WaitOutcome::Exhausted;
                    break;
                }
            }
            /* One of the three EINTR sites the C retries blindly, and the
             * one that matters for a ^C during a foreground command that
             * does not itself die of it. */
            if let Some(error) = crate::error::poll_interrupt(shell.interrupt_context()) {
                return Err(error);
            }
        }

        if waited != WaitOutcome::Interrupted {
            break;
        }
        if !mode.suspends_for_signal() {
            waited = WaitOutcome::Exhausted;
            break;
        }

        let blocked =
            nsh_platform::BlockedSignals::all().expect("blocking signals around child wait failed");

        while !signals.child_pending() && signals.pending_signal().is_none() {
            if let Err(error) = blocked.suspend() {
                let mut message = b"Cannot wait for signal (".to_vec();
                message.extend_from_slice(shell.locale.error_message(&error).as_bytes());
                message.push(b')');
                return Err(shell.diagnostics().shell_error(&message));
            }
        }

        drop(blocked);

        if !signals.child_pending() {
            break;
        }
    }

    Ok(waited)
}

/*
 * return 1 if there are stopped jobs, otherwise 0
 */

// [spec:dash:sem:jobs.stoppedjobs-fn]
pub fn has_stopped_jobs(shell: &mut crate::context::Shell) -> Result<bool, Error> {
    if shell.jobs.job_warning != JobWarning::Ready {
        return Ok(false);
    }
    if shell
        .jobs
        .current()
        .is_some_and(|id| shell.jobs[id].is_stopped())
    {
        shell.write_output(OutputDestination::Stderr, b"You have stopped jobs.\n")?;
        shell.jobs.job_warning = JobWarning::Reported;
        Ok(true)
    } else {
        Ok(false)
    }
}

/*
 * Return a string identifying a command (to be printed by the
 * jobs command).
 */

// [spec:dash:sem:jobs.commandtext-fn]
// [spec:posix:req:builtin.jobs.stdout-default-format]
// [spec:nsh:sem:idiom.specified-defects+1]
fn render_command(node: &Node) -> BString {
    let mut text = BString::new(Vec::new());
    render_node(Some(node), &mut text);
    text
}

// [spec:dash:sem:jobs.cmdtxt-fn]
// [spec:nsh:req:idiom.structural-ast]
fn render_node(node: Option<&Node>, text: &mut BString) {
    let Some(node) = node else { return };
    match node {
        Node::Sequence(binary) => render_binary_command(binary, b"; ", text),
        Node::And(binary) => render_binary_command(binary, b" && ", text),
        Node::Or(binary) => render_binary_command(binary, b" || ", text),
        Node::Redirect(command) => {
            render_node(Some(command.command.as_ref()), text);
            render_redirections(&command.redirections, text);
        }
        Node::Background(command) => {
            render_node(Some(command.command.as_ref()), text);
        }
        Node::Not(command) => {
            push_command_text(b"!", text);
            render_node(Some(command.command.as_ref()), text);
        }
        Node::If(command) => {
            push_command_text(b"if ", text);
            render_node(Some(command.condition.as_ref()), text);
            push_command_text(b"; then ", text);
            render_node(Some(command.then_branch.as_ref()), text);
            if command.else_branch.is_some() {
                push_command_text(b"; else ", text);
                render_node(command.else_branch.as_deref(), text);
            }
            push_command_text(b"; fi", text);
        }
        Node::Subshell(command) => {
            push_command_text(b"(", text);
            render_node(Some(command.command.as_ref()), text);
            push_command_text(b")", text);
            render_redirections(&command.redirections, text);
        }
        Node::While(command) | Node::Until(command) => {
            push_command_text(
                if matches!(node, Node::While(_)) {
                    b"while "
                } else {
                    b"until "
                },
                text,
            );
            render_node(Some(command.left.as_ref()), text);
            push_command_text(b"; do ", text);
            render_node(Some(command.right.as_ref()), text);
            push_command_text(b"; done", text);
        }
        Node::For(command) => {
            push_command_text(b"for ", text);
            push_command_text(command.variable.as_bstr(), text);
            push_command_text(b" in ", text);
            render_command_list(&command.words, true, text);
            push_command_text(b"; do ", text);
            render_node(Some(command.body.as_ref()), text);
            push_command_text(b"; done", text);
        }
        Node::Function(function) => {
            push_command_text(function.name.as_bstr(), text);
            push_command_text(b"() { ... }", text);
        }
        Node::Command(command) => {
            render_command_list(&command.assignments, true, text);
            if !command.assignments.is_empty() && !command.arguments.is_empty() {
                push_command_text(b" ", text);
            }
            render_command_list(&command.arguments, true, text);
            render_redirections(&command.redirections, text);
        }
        Node::Word(word) => word.word.render(text),
        Node::Case(command) => {
            push_command_text(b"case ", text);
            render_node(Some(command.word.as_ref()), text);
            push_command_text(b" in ", text);
            for clause in &command.clauses {
                for (index, pattern) in clause.patterns.iter().enumerate() {
                    if index != 0 {
                        push_command_text(b"|", text);
                    }
                    render_node(Some(pattern), text);
                }
                push_command_text(b") ", text);
                render_node(clause.body.as_deref(), text);
                push_command_text(if clause.fallthrough { b";& " } else { b";; " }, text);
            }
            push_command_text(b"esac", text);
        }
        Node::Pipeline(pipeline) => {
            for (index, command) in pipeline.commands.iter().enumerate() {
                if index != 0 {
                    push_command_text(b" | ", text);
                }
                render_node(Some(command), text);
            }
        }
        Node::Bash(_) => push_command_text(b"<bash syntax>", text),
    }
}

fn render_binary_command(
    command: &crate::nodes::BinaryCommand,
    separator: &[u8],
    text: &mut BString,
) {
    render_node(Some(command.left.as_ref()), text);
    push_command_text(separator, text);
    render_node(Some(command.right.as_ref()), text);
}

// [spec:dash:sem:jobs.cmdlist-fn]
fn render_command_list(nodes: &[Node], space_between: bool, text: &mut BString) {
    for (index, node) in nodes.iter().enumerate() {
        if !space_between {
            push_command_text(b" ", text);
        }
        render_node(Some(node), text);
        if space_between && index + 1 < nodes.len() {
            push_command_text(b" ", text);
        }
    }
}

fn render_redirections(redirections: &[Redirection], text: &mut BString) {
    for redirection in redirections {
        push_command_text(b" ", text);
        match redirection {
            Redirection::File(redirection) => {
                push_command_text(&[redirection.descriptor.as_digit()], text);
                push_command_text(
                    match redirection.operator {
                        FileRedirectionOperator::Write => b">",
                        FileRedirectionOperator::Clobber => b">|",
                        FileRedirectionOperator::Read => b"<",
                        FileRedirectionOperator::ReadWrite => b"<>",
                        FileRedirectionOperator::Append => b">>",
                    },
                    text,
                );
                redirection.target.word.render(text);
            }
            Redirection::Descriptor(redirection) => {
                push_command_text(&[redirection.descriptor.as_digit()], text);
                push_command_text(
                    match redirection.operator {
                        DescriptorRedirectionOperator::Input => b"<&",
                        DescriptorRedirectionOperator::Output => b">&",
                    },
                    text,
                );
                match &redirection.target {
                    DescriptorTarget::Number(descriptor) => {
                        push_command_text(&[descriptor.as_digit()], text)
                    }
                    DescriptorTarget::Close => push_command_text(b"-", text),
                    DescriptorTarget::Word(word) => word.word.render(text),
                }
            }
            Redirection::HereDocument(_) => push_command_text(b"<<...", text),
        }
    }
}

// [spec:dash:sem:jobs.cmdputs-fn]
fn push_command_text(s: &[u8], text: &mut BString) {
    for &byte in s {
        if matches!(byte, b'\'' | b'\\' | b'"' | b'$') {
            text.push(b'\\');
        }
        text.push(byte);
    }
    /* The C leaves an unadvanced `*nextc = '\0'` for `commandtext` to
     * read as the end of the text. The length is that. */
}

// [spec:dash:sem:jobs.showpipe-fn]
pub(crate) fn write_pipeline(
    shell: &mut crate::context::Shell,
    job_id: JobId,
    destination: OutputDestination,
) -> Result<(), Error> {
    let process_count: usize = shell.jobs[job_id].processes.len();

    for process_index in 1..process_count {
        shell.write_output(destination, b" | ")?;
        write_command_text(shell, job_id, process_index, destination)?;
    }
    shell.write_output(destination, b"\n")?;
    shell.flush_output()
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

// [spec:dash:sem:jobs.getstatus-fn]
// [spec:posix:req:exit.status-normal-termination]
// [spec:posix:req:exit.status-signal-terminated]
pub(crate) fn job_exit_status(
    shell: &mut crate::context::Shell,
    job_id: JobId,
) -> crate::status::ExitStatus {
    /* `job->ps + job->nprocs - 1` in C: the bitfield promotes to `int`,
     * so a job that has not forked yet reads `ps[-1]`. It has no status
     * to report; `wait %n` on one answers 0. */
    let mut remaining_processes = shell.jobs[job_id].processes.len();
    let mut status = remaining_processes
        .checked_sub(1)
        .and_then(|last| shell.jobs[job_id].processes[last].status);
    if shell.options.enabled(ShellOption::Pipefail) {
        while matches!(status, None | Some(ChildStatus::Exited(0))) && remaining_processes >= 2 {
            remaining_processes -= 1;
            status = shell.jobs[job_id].processes[remaining_processes - 1].status;
        }
    }

    let status = match status {
        // A job with no completed process has no failure status to report.
        None => crate::status::ExitStatus::SUCCESS,
        Some(ChildStatus::Exited(code)) => crate::status::ExitStatus::from(code),
        Some(ChildStatus::Signaled { signal, .. }) => {
            if signal == nsh_platform::interrupt_signal() {
                shell.jobs[job_id].interrupted = true;
            }
            crate::status::ExitStatus::from_code(signal.number() + 128)
        }
        Some(ChildStatus::Stopped(signal)) => {
            crate::status::ExitStatus::from_code(signal.number() + 128)
        }
        Some(ChildStatus::Continued) => crate::status::ExitStatus::SUCCESS,
    };
    status
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::{CaseClause, CaseCommand, SimpleCommand, WordNode};
    use crate::word::ParsedWord;

    fn word(text: &[u8]) -> Node {
        Node::Word(WordNode {
            word: ParsedWord::literal(BString::from(text)),
        })
    }

    #[test]
    fn child_status_derives_job_state() {
        let first = ProcessId::new(1).unwrap();
        let second = ProcessId::new(2).unwrap();
        let mut job = Job::new();
        job.processes = vec![
            ProcessRecord {
                process_id: first,
                status: None,
                command_text: BString::default(),
            },
            ProcessRecord {
                process_id: second,
                status: None,
                command_text: BString::default(),
            },
        ];

        assert_eq!(
            record_child_status(&mut job, first, ChildStatus::Exited(0)),
            Some(JobState::Running)
        );
        assert_eq!(
            record_child_status(
                &mut job,
                second,
                ChildStatus::Stopped(nsh_platform::terminal_stop_signal()),
            ),
            Some(JobState::Stopped)
        );
        assert_eq!(
            record_child_status(&mut job, second, ChildStatus::Exited(0)),
            Some(JobState::Done)
        );
    }

    #[test]
    fn immediate_notification_gates() {
        assert!(notify_completion_now(
            WaitMode::Block,
            JobState::Done,
            true,
            true,
            true,
            false,
        ));
        assert!(!notify_completion_now(
            WaitMode::Poll,
            JobState::Done,
            true,
            true,
            true,
            false,
        ));
        assert!(!notify_completion_now(
            WaitMode::Block,
            JobState::Stopped,
            true,
            true,
            true,
            false,
        ));
        assert!(!notify_completion_now(
            WaitMode::Block,
            JobState::Done,
            false,
            true,
            true,
            false,
        ));
        assert!(!notify_completion_now(
            WaitMode::Block,
            JobState::Done,
            true,
            false,
            true,
            false,
        ));
        assert!(!notify_completion_now(
            WaitMode::Block,
            JobState::Done,
            true,
            true,
            false,
            false,
        ));
        assert!(!notify_completion_now(
            WaitMode::Block,
            JobState::Done,
            true,
            true,
            true,
            true,
        ));
    }

    // [spec:posix:req:builtin.jobs.stdout-default-format/test]
    // [spec:nsh:sem:idiom.specified-defects+1/test]
    #[test]
    fn job_text_includes_assignment_only_commands() {
        let command = Node::Command(SimpleCommand {
            line: 1,
            assignments: vec![word(b"answer=42")],
            arguments: Vec::new(),
            redirections: Vec::new(),
        });

        assert_eq!(render_command(&command), BString::from(b"answer=42"));
    }

    // [spec:posix:req:builtin.jobs.stdout-default-format/test]
    // [spec:nsh:sem:idiom.specified-defects+1/test]
    #[test]
    fn job_text_includes_every_case_pattern() {
        let command = Node::Case(CaseCommand {
            line: 1,
            word: Box::new(word(b"value")),
            clauses: vec![CaseClause {
                patterns: vec![word(b"first"), word(b"second")],
                body: Some(Box::new(Node::Command(SimpleCommand {
                    line: 1,
                    assignments: Vec::new(),
                    arguments: vec![word(b"echo")],
                    redirections: Vec::new(),
                }))),
                fallthrough: false,
            }],
        });

        assert_eq!(
            render_command(&command),
            BString::from(b"case value in first|second) echo;; esac")
        );
    }
}
