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

mod children;
pub(crate) use children::ForkedChildren;
mod fork;
pub use fork::fork_and_execute;
pub use fork::fork_shell;
use render::render_command;
pub(crate) use render::write_pipeline;
pub(crate) use terminal::apply_saved_job_terminal_settings;
pub(crate) use terminal::capture_shell_terminal_settings;
pub use terminal::set_job_control;
pub(crate) use terminal::set_terminal_process_group;
pub(crate) use terminal::terminal_settings_error;
pub use wait::has_stopped_jobs;
pub(crate) use wait::reap_children;
pub use wait::wait_for_job;
pub(crate) use wait::wait_for_process_substitution;
pub(crate) use wait::wait_for_this_process_substitution;
mod model;
mod render;
mod terminal;
mod wait;

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

/// Write one command's byte-preserving display text.
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
    let mut column: usize;
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
    let indent = column;

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

    let process_count = shell.jobs[job_id].processes.len();

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
    let mut index: usize;

    index = 0;
    let job_id = loop {
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

/// Every process of a job, in pipeline order, as `${PIPESTATUS[@]}`
/// reports them.
///
/// A process that has not been waited for has no status to report and
/// answers 0, which is what Bash's array holds for a job it has not
/// reaped.
// [spec:nsh:req:compat.bash.builtins-special-variables]
pub(crate) fn pipeline_statuses(
    shell: &crate::context::Shell,
    job_id: JobId,
) -> Vec<crate::status::ExitStatus> {
    shell.jobs[job_id]
        .processes
        .iter()
        .map(|process| match process.status {
            None => crate::status::ExitStatus::SUCCESS,
            Some(ChildStatus::Exited(code)) => crate::status::ExitStatus::from(code),
            Some(ChildStatus::Signaled { signal, .. } | ChildStatus::Stopped(signal)) => {
                crate::status::ExitStatus::from_code(signal.number() + 128)
            }
            Some(ChildStatus::Continued) => crate::status::ExitStatus::SUCCESS,
        })
        .collect()
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

    match status {
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
    }
}

#[cfg(test)]
mod tests;
