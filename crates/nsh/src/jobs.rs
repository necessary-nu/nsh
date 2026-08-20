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
//!   * `TRACE(...)` compiles to nothing without `DEBUG` and is dropped.

use bstr::{BStr, BString, ByteSlice};
use core::ffi::c_int;
use nsh_platform::{
    ChildStatus, Descriptor, NativeStrExt as _, ProcessGroupId, ProcessGroupState, ProcessId,
    ProcessSelector, ProcessTarget,
};
use std::io::Write as _;

use crate::error::Error;
use crate::fd::LogicalDescriptor;
use crate::nodes::{
    DescriptorRedirectionOperator, DescriptorTarget, FileRedirectionOperator, Node, Redirection,
};
use crate::options::ShellOption;
use crate::output::Dest;
// [spec:nsh:def:idiom.shell-options]

mod model;

pub(crate) use model::{Job, JobId, JobState, JobTable, ProcStat};

/// Append an already-rendered ASCII fragment with `fmtstr`'s historical
/// clamp-to-capacity convention.
fn append_ascii(out: &mut Vec<u8>, capacity: usize, text: &str) -> c_int {
    debug_assert!(text.is_ascii());
    let copied = text.len().min(capacity.saturating_sub(1));
    out.extend_from_slice(&text.as_bytes()[..copied]);
    text.len().min(capacity) as c_int
}

// ---------------------------------------------------------------------
// src/jobs.h
// ---------------------------------------------------------------------

/* Mode argument to forkshell.  Don't change FORK_FG or FORK_BG. */
pub const FORK_FG: c_int = 0;
pub const FORK_BG: c_int = 1;
pub const FORK_NOJOB: c_int = 2;

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

/* mode flags for dowait */
const DOWAIT_NONBLOCK: c_int = 0;
const DOWAIT_BLOCK: c_int = 1;
pub(crate) const DOWAIT_WAITCMD: c_int = 2;
pub(crate) const DOWAIT_WAITCMD_ALL: c_int = 4;

fn notify_completion_now(
    block: c_int,
    state: JobState,
    shell_jobctl: bool,
    notify: bool,
    job_jobctl: bool,
    is_waited_for: bool,
) -> bool {
    block == DOWAIT_BLOCK
        && state == JobState::Done
        && shell_jobctl
        && notify
        && job_jobctl
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
pub(crate) fn ps_pid(sh: &crate::context::Shell, jp: JobId, i: usize) -> Option<ProcessId> {
    sh.jobs[jp].ps.get(i).map(|process| process.pid)
}

fn process_id_text(process: Option<ProcessId>) -> String {
    process.map_or_else(|| "0".to_owned(), |process| process.to_string())
}

#[inline]
fn ps_cmd(sh: &crate::context::Shell, jp: JobId, i: usize) -> &BStr {
    sh.jobs[jp]
        .ps
        .get(i)
        .map_or(BStr::new(b""), |p| p.cmd.as_bstr())
}

/// `%s` of a command text. The bytes are the shell's own — the parser
/// puts control bytes 0x81-0x88 in them — so they go out as bytes and
/// not through a `char *`.
#[inline]
pub(crate) fn outcmd(sh: &mut crate::context::Shell, jp: JobId, i: usize, dest: Dest) {
    /* The lookup is spelled out here rather than going through `ps_cmd`,
     * which is otherwise these same three lines. The two borrows have to
     * be *field*-disjoint: the write takes `sh.io` mutably and the text is
     * read out of `sh.jobs`, and the compiler can see those are different
     * fields only when both are direct field paths. `ps_cmd` borrows the
     * whole shell, so writing through it becomes a conflict the moment
     * `io` becomes a field. It stays because `getjob`'s command-text
     * search still uses it, and that one only reads. */
    let cmd = sh.jobs[jp]
        .ps
        .get(i)
        .map_or(BStr::new(b""), |p| p.cmd.as_bstr());
    let _ = sh.io.get(dest).write_all(cmd);
}

// [spec:dash:def:jobs.onsigchild-fn]
// [spec:dash:sem:jobs.onsigchild-fn]
//
// `STATIC int onsigchild(void);` is declared under `#ifdef SYSV` in
// src/jobs.c (line 117) and is never defined anywhere in the tree — a
// vestige of System V SIGCHLD handling that was removed. There is no
// body to port; this is the annotated placeholder that records the
// omission. `#[cfg(any())]` mirrors the never-satisfied `#ifdef SYSV`.
#[cfg(any())]
fn onsigchild() -> c_int {
    unimplemented!("declared under #ifdef SYSV, never defined in dash")
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

// [spec:dash:def:jobs.xxtcsetpgrp-fn]
// [spec:dash:sem:jobs.xxtcsetpgrp-fn]
pub(crate) fn xxtcsetpgrp(
    sh: &mut crate::context::Shell,
    group: ProcessGroupId,
) -> Result<(), Error> {
    let Some(fd) = sh.jobs.ttyfd.take() else {
        return Ok(());
    };
    let result = xtcsetpgrp(sh, &fd, group.into());
    sh.jobs.ttyfd = Some(fd);
    result
}

// [spec:posix:req:jobctl.save-terminal-settings]
pub(crate) fn capture_shell_terminal_settings(sh: &mut crate::context::Shell) -> Result<(), Error> {
    if !sh.jobs.jobctl || sh.jobs.shell_terminal_settings.is_some() {
        return Ok(());
    }
    let result = {
        let Some(fd) = sh.jobs.ttyfd.as_ref() else {
            return Ok(());
        };
        nsh_platform::TerminalSettings::capture(fd)
    };
    match result {
        Ok(settings) => {
            sh.jobs.shell_terminal_settings = Some(settings);
            Ok(())
        }
        Err(error) => Err(terminal_settings_error(
            sh,
            b"Cannot save shell tty settings",
            error,
        )),
    }
}

pub(crate) fn apply_saved_job_terminal_settings(
    sh: &crate::context::Shell,
    jp: JobId,
) -> std::io::Result<()> {
    let Some(settings) = sh.jobs[jp].terminal_settings.as_ref() else {
        return Ok(());
    };
    let Some(fd) = sh.jobs.ttyfd.as_ref() else {
        return Ok(());
    };
    settings.apply(fd)
}

pub(crate) fn terminal_settings_error(
    sh: &mut crate::context::Shell,
    operation: &[u8],
    error: std::io::Error,
) -> Error {
    let mut message = operation.to_vec();
    message.extend_from_slice(b" (");
    message.extend_from_slice(sh.locale.error_message(&error).as_bytes());
    message.push(b')');
    sh.sh_error_value(&message)
}

fn acquire_control_terminal(sh: &mut crate::context::Shell) -> Result<Option<Descriptor>, Error> {
    let terminal_path = nsh_platform::controlling_terminal_path().to_shell_bytes();
    if let Some(opened) = crate::redir::sh_open(
        sh,
        BStr::new(&terminal_path),
        nsh_platform::OpenMode::ReadWrite,
        1,
    )? {
        return crate::redir::move_fd_above(sh, opened).map(Some);
    }

    let candidate = [
        LogicalDescriptor::STDERR,
        LogicalDescriptor::STDOUT,
        LogicalDescriptor::STDIN,
    ]
    .into_iter()
    .find(|candidate| {
        sh.fds
            .get(*candidate)
            .as_ref()
            .is_some_and(|fd| nsh_platform::is_terminal(fd))
    });
    match candidate {
        Some(candidate) => crate::redir::copy_slot_above(sh, candidate),
        None => Ok(None),
    }
}

fn await_foreground_group(
    sh: &crate::context::Shell,
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
            || !sh.options.enabled(ShellOption::Interactive)
        {
            return Some(group);
        }
        let _ = nsh_platform::send_signal(
            ProcessTarget::CurrentProcessGroup,
            nsh_platform::SignalRequest::Deliver(nsh_platform::terminal_input_signal()),
        );
    }
}

// [spec:dash:def:jobs.setjobctl-fn]
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
/// callers that *are* ordinary code (`set -m`, `exec`, `procargs`) keep
/// dash's behaviour of abandoning the command, and the teardown callers
/// drop it where the C already swallowed it.
pub fn setjobctl(sh: &mut crate::context::Shell, on: c_int) -> Result<(), Error> {
    let enabled = on != 0;
    let process_group: Option<ProcessGroupState>;
    let mut fd: Option<Descriptor>;

    if enabled == sh.jobs.jobctl || crate::shellmain::rootshell(sh) == 0 {
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
    if enabled && !sh.host.may_control_terminal() {
        return Ok(());
    }
    if enabled {
        /* `setjobctl` is reached from `exitshell`'s job-control teardown as
         * well as from `optschanged`, so it stays infallible and bridges:
         * a failure here longjmps exactly as the C's `sh_open` did. Making
         * teardown fallible is the shape docs/errors-are-values.md 4.3
         * argues against. */
        fd = acquire_control_terminal(sh)?;
        let foreground = fd
            .as_ref()
            .and_then(|terminal| await_foreground_group(sh, terminal));
        let terminal_is_accessible = foreground == Some(nsh_platform::current_process_group());
        if !terminal_is_accessible {
            drop(fd.take());
            if sh.options.enabled(ShellOption::Interactive) {
                sh.sh_warnx(b"can't access tty; job control turned off");
                sh.options.set(ShellOption::Monitor, false);
                return Ok(());
            }
        }
        sh.jobs.initialpgrp = foreground;
        process_group = Some(ProcessGroupId::from_leader(sh.root_pid).into());
    } else {
        /* turning job control off */
        fd = sh.jobs.ttyfd.take();
        process_group = sh.jobs.initialpgrp;
    }

    crate::trap::setsignal(sh, nsh_platform::terminal_stop_signal().into());
    crate::trap::setsignal(sh, nsh_platform::terminal_output_signal().into());
    crate::trap::setsignal(sh, nsh_platform::terminal_input_signal().into());
    if let (Some(tty), Some(group)) = (fd.as_ref(), process_group) {
        let _ = nsh_platform::set_process_group(ProcessSelector::CurrentProcess, group);
        xtcsetpgrp(sh, tty, group)?;

        if !enabled {
            drop(fd.take());
        }
    }

    sh.jobs.ttyfd = fd;
    sh.jobs.jobctl = enabled;
    Ok(())
}

// [spec:dash:def:jobs.jobno-fn]
// [spec:dash:sem:jobs.jobno-fn]
//
// The C recovers the index by subtracting `jobtab` from the pointer.
pub(crate) const fn jobno(jp: JobId) -> usize {
    jp.0 + 1
}

// [spec:dash:def:jobs.sprint-status-fn]
// [spec:dash:sem:jobs.sprint-status-fn]
// [spec:posix:def:builtin.jobs.stdout-state-strings]
fn sprint_status(
    locale: &nsh_platform::Locale,
    out: &mut Vec<u8>,
    status: ChildStatus,
    sigonly: c_int,
) -> c_int {
    let start = out.len();
    match status {
        ChildStatus::Exited(code) if sigonly == 0 => {
            if code == 0 {
                append_ascii(out, 5, "Done");
            } else {
                append_ascii(out, 16, &format!("Done({code})"));
            }
        }
        ChildStatus::Stopped(signal) if sigonly == 0 => {
            let signal_name = crate::signames::signal_names
                .get(signal.number() as usize)
                .map_or(BStr::new(b""), |name| BStr::new(name.to_bytes()));
            out.extend_from_slice(b"Stopped");
            if !signal_name.is_empty() {
                out.extend_from_slice(b" (SIG");
                out.extend_from_slice(signal_name);
                out.push(b')');
            }
        }
        ChildStatus::Signaled {
            signal,
            core_dumped,
        } if sigonly == 0
            || (signal != nsh_platform::interrupt_signal()
                && signal != nsh_platform::pipe_signal()) =>
        {
            /* `stpncpy(s, …, 32)` copies at most 32 bytes and NUL-pads
             * the rest of them, which is why the callers' buffers are
             * sized for 32 whatever the signal is called. `strsignal` is
             * locale text, not ASCII, so the bytes are copied rather than
             * routed through `copy_ascii_cstr`. */
            let description = locale.signal_description(signal);
            let name = description.as_slice();
            let n = name.len().min(32);
            out.extend_from_slice(&name[..n]);
            if core_dumped {
                append_ascii(out, 15, " (core dumped)");
            }
        }
        ChildStatus::Continued if sigonly == 0 => {
            append_ascii(out, 8, "Running");
        }
        _ => {}
    }
    (out.len() - start) as c_int
}

// [spec:dash:def:jobs.showjob-fn]
// [spec:dash:sem:jobs.showjob-fn]
// [spec:posix:req:builtin.jobs.remove-reported-job]
// [spec:posix:req:builtin.jobs.stdout-p-format]
// [spec:posix:req:builtin.jobs.stdout-current-field]
// [spec:posix:req:builtin.jobs.stdout-state-substitution]
// [spec:posix:req:builtin.jobs.stdout-l-format]
// [spec:posix:req:builtin.jobs.stdout-default-format]
// [spec:posix:req:jobctl.suspended-job-message]
pub(crate) fn showjob(sh: &mut crate::context::Shell, dest: Dest, jp: JobId, mode: JobDisplay) {
    let psend: usize;
    let mut col: c_int;
    let indent: c_int;
    let mut s: Vec<u8> = Vec::with_capacity(80);

    if matches!(mode, JobDisplay::ProcessGroup) {
        /* just output process (group) id of pipeline */
        /* The pid is read out before the write starts rather than inside
         * its argument list: `ps_pid` borrows the shell and the write
         * borrows `sh.io`, and evaluating one inside the other is the
         * conflict `Dest` exists to keep out of these functions. */
        let pid = process_id_text(ps_pid(sh, jp, 0));
        let _ = writeln!(sh.io.get(dest), "{pid}");
        return;
    }

    let heading = format!("[{}]   ", jobno(jp));
    col = append_ascii(&mut s, 16, &heading);
    indent = col;

    if Some(jp) == sh.jobs.current() {
        s[(col - 2) as usize] = b'+';
    } else if Some(jp) == sh.jobs.previous() {
        s[(col - 2) as usize] = b'-';
    }

    if matches!(mode, JobDisplay::Long) {
        let pid = format!("{} ", process_id_text(ps_pid(sh, jp, 0)));
        col += append_ascii(&mut s, 16, &pid);
    }

    psend = sh.jobs[jp].ps.len();

    if sh.jobs[jp].is_running() {
        /* scopy("Running", s + col) */
        col += append_ascii(&mut s, 8, "Running");
    } else {
        /* `psend[-1]`: a job leaves the running state only through `waitone`,
         * which needs a process to have exited to do it. */
        let mut status = sh.jobs[jp].ps[psend - 1]
            .status
            .expect("a completed job has a child status");
        if sh.jobs[jp].is_stopped() {
            status = sh.jobs[jp]
                .stopstatus
                .expect("a stopped job records its stop status");
        }
        col += sprint_status(&sh.locale, &mut s, status, 0);
    }

    let line_count = if matches!(mode, JobDisplay::Long) {
        psend.max(1)
    } else {
        1
    };
    for ps in 0..line_count {
        let (mut record, line_column) = if ps == 0 {
            (s.clone(), col)
        } else {
            let continuation = format!(
                " |\n{space:>width$}{} ",
                process_id_text(ps_pid(sh, jp, ps)),
                space = ' ',
                width = indent.max(0) as usize,
            );
            let mut prefix = Vec::with_capacity(48);
            let column = append_ascii(&mut prefix, 48, &continuation) - 3;
            (prefix, column)
        };

        let width = (33 - line_column).max(0) as usize;
        record.resize(record.len() + width.max(1), b' ');
        let _ = sh.io.get(dest).write_all(&record);
        outcmd(sh, jp, ps, dest);
    }
    if matches!(mode, JobDisplay::Long) {
        let _ = sh.io.get(dest).write_all(b"\n");
    } else {
        showpipe(sh, jp, dest);
    }

    sh.jobs[jp].changed = false;

    if sh.jobs[jp].is_done() {
        /* TRACE(("showjob: freeing job %d\n", jobno(jp))); */
        freejob(sh, jp);
    }
}

/*
 * Print a list of jobs.  If "change" is nonzero, only print jobs whose
 * statuses have changed since the last call to showjobs.
 */

// [spec:dash:def:jobs.showjobs-fn]
// [spec:dash:sem:jobs.showjobs-fn]
// [spec:posix:req:jobctl.background-job-suspended-message]
// [spec:posix:req:jobctl.background-job-completion-message]
// [spec:posix:req:jobctl.non-interactive-message-timing]
pub(crate) fn showjobs(
    sh: &mut crate::context::Shell,
    dest: Dest,
    mode: JobDisplay,
) -> Result<(), Error> {
    /* TRACE(("showjobs(%x) called\n", mode)); */

    /* If not even one job changed, there is nothing to do */
    /* `DOWAIT_NONBLOCK`, so the wait cannot block and the poll inside it
     * has nothing to notice; the `?` is the type saying so rather than a
     * path anyone expects to take. */
    dowait(sh, DOWAIT_NONBLOCK, None)?;

    /* `showjob` may remove a completed entry, so traverse a stable copy of
     * the explicit presentation order. */
    for i in sh.jobs.order_snapshot() {
        if !matches!(mode, JobDisplay::Changed) || sh.jobs[i].changed {
            showjob(sh, dest, i, mode);
        }
    }
    Ok(())
}

/*
 * Mark a job structure as unused.
 */

// [spec:dash:def:jobs.freejob-fn]
// [spec:dash:sem:jobs.freejob-fn]
fn freejob(sh: &mut crate::context::Shell, jp: JobId) {
    crate::error::with_interrupts_deferred(sh, |sh| {
        /* Taking the occupied slot releases all owned process text and terminal
         * state exactly once, while `JobTable::remove` also repairs ordering. */
        drop(sh.jobs.remove(jp));
    });
}

/// Remove a successfully waited, completed job from both the job list and
/// the set of process IDs known to this shell environment.
// [spec:posix:req:builtin.wait.remove-waited-for-pid]
pub(crate) fn remove_waited_job(sh: &mut crate::context::Shell, jp: JobId) {
    if sh.jobs[jp].is_done() {
        freejob(sh, jp);
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

fn lookup_job(sh: &crate::context::Shell, name: Option<&BStr>) -> Result<JobId, JobLookupError> {
    let current = sh.jobs.current();
    let Some(name) = name else {
        return current.ok_or(JobLookupError::NoCurrent);
    };
    let Some(mut pattern) = name.strip_prefix(b"%") else {
        return Err(JobLookupError::NoSuch);
    };

    match pattern {
        b"" | b"+" | b"%" => return current.ok_or(JobLookupError::NoCurrent),
        b"-" => return sh.jobs.previous().ok_or(JobLookupError::NoPrevious),
        _ => {}
    }

    if let Some(number) = crate::mystring::decimal_digits(BStr::new(pattern))
        && let Ok(number) = usize::try_from(number)
        && let Some(index) = number
            .checked_sub(1)
            .filter(|index| *index < sh.jobs.slots.len())
    {
        return sh.jobs.slots[index]
            .as_ref()
            .map(|_| JobId(index))
            .ok_or(JobLookupError::NoSuch);
    }

    let substring = pattern.first() == Some(&b'?');
    if substring {
        pattern = &pattern[1..];
    }
    let mut found = None;
    for id in sh.jobs.order_snapshot() {
        let command = ps_cmd(sh, id, 0);
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

// [spec:dash:def:jobs.getjob-fn]
// [spec:dash:sem:jobs.getjob-fn]
pub(crate) fn getjob(
    sh: &mut crate::context::Shell,
    name: Option<&BStr>,
    getctl: c_int,
) -> Result<JobId, Error> {
    let result = lookup_job(sh, name).and_then(|id| {
        if getctl != 0 && !sh.jobs[id].jobctl {
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
    Err(sh.sh_error_value(&message))
}

/*
 * Return a new job structure.
 * Called with interrupts off.
 */

// [spec:dash:def:jobs.makejob-fn]
// [spec:dash:sem:jobs.makejob-fn]
// [spec:posix:req:jobctl.job-creation]
// [spec:posix:req:cmd.async-job-number]
// [spec:posix:sem:cmd.async-job-control]
// [spec:posix:req:cmd.async-known-pid-retention]
pub fn makejob(sh: &mut crate::context::Shell, nprocs: c_int) -> JobId {
    let jp: JobId;
    let mut i: usize;

    i = 0;
    jp = loop {
        if i >= sh.jobs.slots.len() {
            break growjobtab(sh);
        }
        let id = JobId(i);
        let Some(job) = sh.jobs.slots[id.0].as_ref() else {
            break id;
        };
        if !job.is_done() || !job.waited {
            i += 1;
            continue;
        }
        if sh.jobs.jobctl {
            i += 1;
            continue;
        }
        freejob(sh, id);
        break id;
    };
    let mut job = Job::new();
    /* The C picks the inline `ps0` for a single process and `ckmalloc`s
     * an array otherwise; all that decided was where the room came from,
     * so it is the capacity here and the processes are pushed as
     * `forkparent` forks them. */
    if nprocs > 0 {
        job.ps.reserve_exact(nprocs as usize);
    }
    if sh.jobs.jobctl {
        job.jobctl = true;
    }
    sh.jobs.occupy_current(jp, job);
    /* TRACE(("makejob(%d) returns %%%d\n", nprocs, jobno(jp))); */
    jp
}

// [spec:dash:def:jobs.growjobtab-fn]
// [spec:dash:sem:jobs.growjobtab-fn]
//
// The C's relocation pass has no counterpart: jobs own their process arrays,
// identities are indices, and ordering contains values rather than pointers.
fn growjobtab(sh: &mut crate::context::Shell) -> JobId {
    let len: usize = sh.jobs.slots.len();

    for _ in 0..4 {
        sh.jobs.slots.push(None);
    }
    JobId(len)
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
/// is what guarantees (see `shellmain::exit_from_child`). The diagnostic
/// has already been written.
#[cold]
fn forkchild_fatal(sh: &mut crate::context::Shell, e: Error) -> ! {
    crate::shellmain::exit_from_child(sh, Err(e))
}

// [spec:dash:def:jobs.forkchild-fn]
// [spec:dash:sem:jobs.forkchild-fn]
// [spec:posix:req:jobctl.pipeline-process-group]
// [spec:posix:req:jobctl.foreground-process-group-assignment]
// [spec:posix:req:signal.async-list-sigint-sigquit-ignored]
// [spec:posix:req:signal.inherited-actions]
// [spec:posix:req:shenv.subshell-creation]
// [spec:posix:req:shenv.subshell-isolation]
// [spec:posix:req:cmd.async-stdin-devnull]
// [spec:nsh:req:idiom.no-raw-fd-core]
fn forkchild(sh: &mut crate::context::Shell, jp: Option<JobId>, n: Option<&Node>, mode: c_int) {
    let oldlvl: c_int;

    /* TRACE(("Child shell %d\n", getpid())); */

    crate::shell::reset_coverage();

    oldlvl = sh.shell_level;
    sh.shell_level += 1;

    sh.prepare_fork_child(if mode == FORK_NOJOB { n } else { None });

    /* do job control only in root shell */
    sh.jobs.jobctl = false;

    /* The C tests `jp->jobctl` without checking `jp`; `jp` is NULL only
     * under FORK_NOJOB, which the first conjunct has already excluded. */
    let ownpgrp = mode != FORK_NOJOB && oldlvl == 0 && jp.is_some_and(|i| sh.jobs[i].jobctl);
    if ownpgrp {
        let process_group: ProcessGroupId;
        let ji: JobId = jp.unwrap();

        if sh.jobs[ji].ps.is_empty() {
            process_group = ProcessGroupId::from_leader(nsh_platform::current_process_id());
        } else {
            process_group = ProcessGroupId::from_leader(sh.jobs[ji].ps[0].pid);
        }
        /* This can fail because we are doing it in the parent also */
        let _ =
            nsh_platform::set_process_group(ProcessSelector::CurrentProcess, process_group.into());
        if mode == FORK_FG {
            xxtcsetpgrp(sh, process_group).unwrap_or_else(|e| forkchild_fatal(sh, e));
        }
        crate::trap::setsignal_in_child(sh, nsh_platform::terminal_stop_signal().into());
        crate::trap::setsignal_in_child(sh, nsh_platform::terminal_output_signal().into());
    } else if mode == FORK_BG {
        crate::trap::ignoresig_in_child(sh, nsh_platform::interrupt_signal().into());
        crate::trap::ignoresig_in_child(sh, nsh_platform::quit_signal().into());
        if jp.map_or(false, |i| sh.jobs[i].ps.is_empty()) {
            /* The C closes descriptor 0 and reopens /dev/null, relying on
             * `open` returning the lowest free descriptor to land back on
             * 0. That only works when the shell's stdin *is* 0, so put it
             * where it belongs when the frontend said otherwise. */
            let null_path = nsh_platform::null_device_path().to_shell_bytes();
            let f = crate::redir::sh_open(
                sh,
                BStr::new(&null_path),
                nsh_platform::OpenMode::ReadOnly,
                0,
            )
            .unwrap_or_else(|e| forkchild_fatal(sh, e))
            .expect("a mandatory open returns a descriptor");
            if let Err(error) = sh.fds.install_owned(LogicalDescriptor::STDIN, f) {
                let error = crate::redir::descriptor_error(sh, LogicalDescriptor::STDIN, error);
                forkchild_fatal(sh, error);
            }
            /* Should call reset_input here, but it's harmless
             * for now.
             */
        }
    }
    if oldlvl == 0 && sh.options.enabled(ShellOption::Interactive) {
        crate::trap::setsignal_in_child(sh, nsh_platform::interrupt_signal().into());
        crate::trap::setsignal_in_child(sh, nsh_platform::quit_signal().into());
        crate::trap::setsignal_in_child(sh, nsh_platform::termination_signal().into());
    }

    let Some(ji) = jp else {
        return;
    };

    freejob(sh, ji);

    if crate::parser::issimplecmd(n, BStr::new(crate::builtins::JOBSCMD.name.to_bytes())) != 0 {
        return;
    }

    for i in sh.jobs.order_snapshot() {
        freejob(sh, i);
    }
}

// [spec:dash:def:jobs.forkparent-fn]
// [spec:dash:sem:jobs.forkparent-fn]
// [spec:posix:req:jobctl.job-number-and-process-id]
// [spec:posix:req:cmd.async-process-id-known]
// [spec:posix:req:cmd.async-job-notification-format]
// [spec:posix:req:cmd.async-non-job-pid-message]
fn forkparent(
    sh: &mut crate::context::Shell,
    jp: Option<JobId>,
    n: Option<&Node>,
    mode: c_int,
    pid: ProcessId,
) {
    /* TRACE(("In parent shell:  child = %d\n", pid)); */
    let Some(ji) = jp else {
        return;
    };
    if mode != FORK_NOJOB && sh.jobs[ji].jobctl {
        let process_group: ProcessGroupId;

        if sh.jobs[ji].ps.is_empty() {
            process_group = ProcessGroupId::from_leader(pid);
        } else {
            process_group = ProcessGroupId::from_leader(sh.jobs[ji].ps[0].pid);
        }
        /* This can fail because we are doing it in the child also */
        let _ =
            nsh_platform::set_process_group(ProcessSelector::Process(pid), process_group.into());
    }
    if mode == FORK_BG {
        sh.backgndpid = Some(pid); /* set $! */
        sh.jobs.position_running(ji);
        if sh.options.enabled(ShellOption::Interactive) {
            let _ = writeln!(sh.io.stderr(), "[{}] {pid}", jobno(ji));
        }
    }
    /* the C's second `if (jp)` is dead after the early return above */
    sh.jobs[ji].ps.push(ProcStat {
        pid,
        status: None,
        cmd: BString::new(Vec::new()),
    });
    if let Some(node) = n {
        let cmd = commandtext(node);
        let last = sh.jobs[ji].ps.len() - 1;
        sh.jobs[ji].ps[last].cmd = cmd;
    }
}

// [spec:dash:def:jobs.forkshell-fn]
// [spec:dash:sem:jobs.forkshell-fn]
// [spec:posix:req:shenv.subshell-contexts]
// [spec:posix:req:xcurel.process-attributes-additional]
// [spec:posix:req:xcurel.concurrent-execution]
pub fn forkshell(
    sh: &mut crate::context::Shell,
    jp: Option<JobId>,
    n: Option<&Node>,
    mode: c_int,
) -> Result<nsh_platform::ForkResult, Error> {
    /* TRACE(("forkshell(%%%d, %p, %d) called\n", jobno(jp), n, mode)); */

    sh.flush_input();

    if mode == FORK_FG && jp.is_some_and(|i| sh.jobs[i].jobctl) {
        capture_shell_terminal_settings(sh)?;
    }

    let fork = match nsh_platform::fork_process() {
        Ok(nsh_platform::ForkResult::Child) => {
            forkchild(sh, jp, n, mode);
            nsh_platform::ForkResult::Child
        }
        Ok(nsh_platform::ForkResult::Parent(pid)) => {
            forkparent(sh, jp, n, mode, pid);
            nsh_platform::ForkResult::Parent(pid)
        }
        Err(_) => {
            if let Some(job) = jp {
                freejob(sh, job);
            }
            return Err(sh.sh_error_value(b"Cannot fork"));
        }
    };

    Ok(fork)
}

// [spec:dash:def:jobs.vforkexec-fn]
// [spec:dash:sem:jobs.vforkexec-fn]
// [spec:posix:req:cmd.nonbuiltin-separate-environment]
/// Fork and immediately execute an external command.
///
/// dash uses `vfork` here. Rust command preparation owns and mutates heap
/// allocations, so sharing the parent's address space is unsound: the
/// second external command returned through a stack corrupted by the first.
/// A regular fork preserves the child-terminus rule without shared memory.
pub fn forkexec(
    sh: &mut crate::context::Shell,
    n: &Node,
    argv: &[&BStr],
    path: &BStr,
    idx: c_int,
) -> Result<JobId, Error> {
    let jp: JobId;
    jp = makejob(sh, 1);

    if sh.jobs[jp].jobctl {
        capture_shell_terminal_settings(sh)?;
    }

    let pid = match nsh_platform::fork_process() {
        Ok(nsh_platform::ForkResult::Child) => {
            forkchild(sh, Some(jp), Some(n), FORK_FG);
            let outcome = crate::exec::shellexec(sh, argv, path, idx);
            crate::shellmain::exit_from_child(sh, outcome);
        }
        Ok(nsh_platform::ForkResult::Parent(pid)) => pid,
        Err(_) => {
            freejob(sh, jp);
            return Err(sh.sh_error_value(b"Cannot fork"));
        }
    };
    forkparent(sh, Some(jp), Some(n), FORK_FG, pid);

    Ok(jp)
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

// [spec:dash:def:jobs.waitforjob-fn]
// [spec:dash:sem:jobs.waitforjob-fn]
// [spec:posix:sem:shell.exit-status-collection]
// [spec:posix:req:jobctl.foreground-process-group-restored]
// [spec:posix:req:signal.trap-deferred-until-foreground-command-completes]
// [spec:posix:sem:cmd.async-status-via-wait]
pub fn waitforjob(
    sh: &mut crate::context::Shell,
    jp: Option<JobId>,
) -> Result<crate::status::ExitStatus, Error> {
    let st: crate::status::ExitStatus;
    let mut terminal_error: Option<(&'static [u8], std::io::Error)> = None;

    /* TRACE(("waitforjob(%%%d) called\n", jp ? jobno(jp) : 0)); */
    dowait(
        sh,
        if jp.is_some() {
            DOWAIT_BLOCK
        } else {
            DOWAIT_NONBLOCK
        },
        jp,
    )?;
    let Some(jp) = jp else {
        return Ok(sh.status);
    };

    st = getstatus(sh, jp);
    if sh.jobs[jp].jobctl {
        if sh.jobs[jp].is_stopped() {
            let result = sh
                .jobs
                .ttyfd
                .as_ref()
                .map(nsh_platform::TerminalSettings::capture);
            match result {
                Some(Ok(settings)) => sh.jobs[jp].terminal_settings = Some(settings),
                Some(Err(error)) => {
                    terminal_error = Some((b"Cannot save job tty settings", error));
                }
                None => {}
            }
        }
        let shell_group = ProcessGroupId::from_leader(sh.root_pid);
        xxtcsetpgrp(sh, shell_group)?;
        if sh.jobs[jp].is_stopped() {
            if let Some(settings) = sh.jobs.shell_terminal_settings.take() {
                let result = sh.jobs.ttyfd.as_ref().map(|fd| settings.apply(fd));
                if let Some(Err(error)) = result {
                    terminal_error = Some((b"Cannot restore shell tty settings", error));
                }
            }
        } else {
            /* A completed foreground utility owns intentional changes made
             * with `stty`. The saved snapshot exists so a suspended job's
             * private modes cannot strand the shell; applying it after a
             * normal exit would erase the utility's successful result. */
            sh.jobs.shell_terminal_settings = None;
        }
        /*
         * This is truly gross.
         * If we're doing job control, then we did a TIOCSPGRP which
         * caused us (the shell) to no longer be in the controlling
         * session -- so we wouldn't have seen any ^C/SIGINT.  So, we
         * intuit from the subprocess exit status whether a SIGINT
         * occurred, and if so interrupt ourselves.  Yuck.  - mycroft
         */
        if sh.jobs[jp].sigint {
            let _ = nsh_platform::raise_signal(nsh_platform::interrupt_signal());
        }
    }
    if sh.jobs[jp].is_done() {
        freejob(sh, jp);
    }
    if let Some((operation, error)) = terminal_error {
        return Err(terminal_settings_error(sh, operation, error));
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
    let child = job.ps.iter_mut().find(|child| child.pid == process)?;
    child.status = Some(status);

    let stopped = job.ps.iter().find_map(|child| match child.status {
        Some(status @ ChildStatus::Stopped(_)) => Some(status),
        _ => None,
    });
    job.stopstatus = stopped;
    if job.ps.iter().any(|child| child.status.is_none()) {
        Some(JobState::Running)
    } else if stopped.is_some() {
        Some(JobState::Stopped)
    } else {
        Some(JobState::Done)
    }
}

// [spec:dash:def:jobs.waitone-fn]
// [spec:dash:sem:jobs.waitone-fn]
// [spec:posix:req:jobctl.suspend-on-catchable-signal]
// [spec:posix:req:jobctl.suspend-on-sigstop]
// [spec:posix:req:builtin.set.opt-b-notify]
fn waitone(
    sh: &mut crate::context::Shell,
    block: c_int,
    jobp: Option<JobId>,
) -> Result<WaitOutcome, Error> {
    let mut thisjob: Option<JobId> = None;
    let mut state = JobState::Running;
    let mut reported_status = None;

    let waited = crate::error::with_interrupts_deferred(sh, |sh| {
        /* TRACE(("dowait(%d) called\n", block)); */
        let waited = waitproc(sh, block)?;
        /* TRACE(("wait returns pid %d, status=%d\n", pid, status)); */
        if let WaitOutcome::Reaped { process, status } = waited {
            reported_status = Some(status);
            for id in sh.jobs.order_snapshot() {
                if sh.jobs[id].is_done() {
                    continue;
                }
                let Some(next_state) = record_child_status(&mut sh.jobs[id], process, status)
                else {
                    continue;
                };
                thisjob = Some(id);
                state = next_state;
                if next_state != JobState::Running {
                    sh.jobs[id].changed = true;
                    if sh.jobs[id].transition_to(next_state) {
                        /* TRACE(("Job %d: changing state from %d to %d\n", ...)); */
                        if next_state == JobState::Stopped {
                            sh.jobs.position_stopped(id);
                        }
                    }
                }
                break;
            }
        }
        Ok::<_, Error>(waited)
    })?;

    if thisjob.is_some() && thisjob == jobp {
        let mut message = Vec::with_capacity(49);
        sprint_status(
            &sh.locale,
            &mut message,
            reported_status.expect("a matched job has a reaped status"),
            1,
        );
        if !message.is_empty() {
            message.push(b'\n');
            let _ = sh.io.stderr().write_all(&message);
        }
    }
    /* A blocking wait can leave an interrupt pending while this structured
     * scope is active. Deliver it only after the caller's prior depth has
     * been restored. */
    if let Some(e) = crate::error::poll_interrupt(sh) {
        return Err(e);
    }
    /* A blocking wait for one foreground job can reap a different,
     * background job first.  `-b` makes that completion observable here,
     * before the wait resumes; non-blocking callers already render changed
     * jobs themselves, and a waited-for job is reported by its caller. */
    if let Some(changed_job) = thisjob {
        if notify_completion_now(
            block,
            state,
            sh.jobs.jobctl,
            sh.options.enabled(ShellOption::Notify),
            sh.jobs[changed_job].jobctl,
            Some(changed_job) == jobp,
        ) {
            showjob(sh, Dest::Stderr, changed_job, JobDisplay::Standard);
        }
    }
    Ok(waited)
}

// [spec:dash:def:jobs.dowait-fn]
// [spec:dash:sem:jobs.dowait-fn]
pub(crate) fn dowait(
    sh: &mut crate::context::Shell,
    block: c_int,
    jp: Option<JobId>,
) -> Result<c_int, Error> {
    let gotchld: c_int = crate::siginbox::signals().child_pending() as c_int;
    let mut wait_completed: c_int;
    let mut waited: WaitOutcome;
    let mut block: c_int = block;

    if jp.is_some_and(|i| !sh.jobs[i].is_running()) {
        block = DOWAIT_NONBLOCK;
    }

    if block == DOWAIT_NONBLOCK && gotchld == 0 {
        return Ok(1);
    }

    wait_completed = 1;

    loop {
        waited = waitone(sh, block, jp)?;
        wait_completed &= (waited != WaitOutcome::Interrupted) as c_int;

        block &= !DOWAIT_WAITCMD_ALL;
        if waited == WaitOutcome::Interrupted || jp.is_some_and(|i| !sh.jobs[i].is_running()) {
            block = DOWAIT_NONBLOCK;
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

// [spec:dash:def:jobs.waitproc-fn]
// [spec:dash:sem:jobs.waitproc-fn]
fn waitproc(sh: &mut crate::context::Shell, block: c_int) -> Result<WaitOutcome, Error> {
    let nonblocking = block != DOWAIT_BLOCK;
    let mut waited: WaitOutcome;

    let signals = crate::siginbox::signals();
    loop {
        signals.set_child_pending(false);
        loop {
            match nsh_platform::wait_for_any_child(nonblocking, sh.jobs.jobctl) {
                Ok(Some((pid, child_status))) => {
                    waited = WaitOutcome::Reaped {
                        process: pid,
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
            if let Some(e) = crate::error::poll_interrupt(sh) {
                return Err(e);
            }
        }

        if waited != WaitOutcome::Interrupted {
            break;
        }
        if block == DOWAIT_NONBLOCK {
            waited = WaitOutcome::Exhausted;
            break;
        }

        let blocked =
            nsh_platform::BlockedSignals::all().expect("blocking signals around child wait failed");

        while !signals.child_pending() && signals.pending_signal().is_none() {
            let _ = blocked.suspend();
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

// [spec:dash:def:jobs.stoppedjobs-fn]
// [spec:dash:sem:jobs.stoppedjobs-fn]
pub fn stoppedjobs(sh: &mut crate::context::Shell) -> c_int {
    if sh.jobs.job_warning != 0 {
        return 0;
    }
    if sh.jobs.current().is_some_and(|id| sh.jobs[id].is_stopped()) {
        let _ = sh.io.stderr().write_all(b"You have stopped jobs.\n");
        sh.jobs.job_warning = 2;
        1
    } else {
        0
    }
}

/*
 * Return a string identifying a command (to be printed by the
 * jobs command).
 */

// [spec:dash:def:jobs.commandtext-fn]
// [spec:dash:sem:jobs.commandtext-fn]
fn commandtext(n: &Node) -> BString {
    let mut text = BString::new(Vec::new());
    cmdtxt(Some(n), &mut text);
    /* `cmdtxt` writes nothing at all for a command with no words — `x=1 &`
     * is one — and the C then hands `savestr` an uninitialised stack block,
     * out of which the reference reads a NUL and prints an empty command
     * text. The empty buffer is that, said on purpose. */
    /* TRACE(("commandtext: name %p, end %p\n", name, cmdnextc)); */
    text
}

// [spec:dash:def:jobs.cmdtxt-fn]
// [spec:dash:sem:jobs.cmdtxt-fn]
// [spec:nsh:req:idiom.structural-ast]
fn cmdtxt(n: Option<&Node>, text: &mut BString) {
    let Some(node) = n else { return };
    match node {
        Node::Sequence(binary) => cmdtxt_binary(binary, b"; ", text),
        Node::And(binary) => cmdtxt_binary(binary, b" && ", text),
        Node::Or(binary) => cmdtxt_binary(binary, b" || ", text),
        Node::Redirect(command) | Node::Background(command) => {
            cmdtxt(Some(command.command.as_ref()), text);
        }
        Node::Not(command) => {
            cmdputs(b"!", text);
            cmdtxt(Some(command.command.as_ref()), text);
        }
        Node::If(command) => {
            cmdputs(b"if ", text);
            cmdtxt(Some(command.condition.as_ref()), text);
            cmdputs(b"; then ", text);
            cmdtxt(Some(command.then_branch.as_ref()), text);
            if command.else_branch.is_some() {
                cmdputs(b"; else ", text);
                cmdtxt(command.else_branch.as_deref(), text);
            }
            cmdputs(b"; fi", text);
        }
        Node::Subshell(command) => {
            cmdputs(b"(", text);
            cmdtxt(Some(command.command.as_ref()), text);
            cmdputs(b")", text);
        }
        Node::While(command) | Node::Until(command) => {
            cmdputs(
                if matches!(node, Node::While(_)) {
                    b"while "
                } else {
                    b"until "
                },
                text,
            );
            cmdtxt(Some(command.left.as_ref()), text);
            cmdputs(b"; do ", text);
            cmdtxt(Some(command.right.as_ref()), text);
            cmdputs(b"; done", text);
        }
        Node::For(command) => {
            cmdputs(b"for ", text);
            cmdputs(command.variable.as_bstr(), text);
            cmdputs(b" in ", text);
            cmdlist(&command.words, 1, text);
            cmdputs(b"; do ", text);
            cmdtxt(Some(command.body.as_ref()), text);
            cmdputs(b"; done", text);
        }
        Node::Function(function) => {
            cmdputs(function.name.as_bstr(), text);
            cmdputs(b"() { ... }", text);
        }
        Node::Command(command) => {
            cmdlist(&command.arguments, 1, text);
            cmdredirs(&command.redirections, text);
        }
        Node::Word(word) => word.word.render(text),
        Node::Case(command) => {
            cmdputs(b"case ", text);
            cmdtxt(Some(command.word.as_ref()), text);
            cmdputs(b" in ", text);
            for clause in &command.clauses {
                /* The C passes the head of the pattern list, so only the
                 * first pattern of a case ever prints. */
                cmdtxt(clause.patterns.first(), text);
                cmdputs(b") ", text);
                cmdtxt(clause.body.as_deref(), text);
                cmdputs(if clause.fallthrough { b";& " } else { b";; " }, text);
            }
            cmdputs(b"esac", text);
        }
        Node::Pipeline(pipeline) => {
            for (index, command) in pipeline.commands.iter().enumerate() {
                if index != 0 {
                    cmdputs(b" | ", text);
                }
                cmdtxt(Some(command), text);
            }
        }
        Node::Bash(_) => cmdputs(b"<bash syntax>", text),
    }
}

fn cmdtxt_binary(command: &crate::nodes::BinaryCommand, separator: &[u8], text: &mut BString) {
    cmdtxt(Some(command.left.as_ref()), text);
    cmdputs(separator, text);
    cmdtxt(Some(command.right.as_ref()), text);
}

// [spec:dash:def:jobs.cmdlist-fn]
// [spec:dash:sem:jobs.cmdlist-fn]
fn cmdlist(np: &[Node], sep: c_int, text: &mut BString) {
    for (i, node) in np.iter().enumerate() {
        if sep == 0 {
            cmdputs(b" ", text);
        }
        cmdtxt(Some(node), text);
        if sep != 0 && i + 1 < np.len() {
            cmdputs(b" ", text);
        }
    }
}

fn cmdredirs(redirections: &[Redirection], text: &mut BString) {
    for redirection in redirections {
        cmdputs(b" ", text);
        match redirection {
            Redirection::File(redirection) => {
                cmdputs(&[redirection.descriptor.as_digit()], text);
                cmdputs(
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
                cmdputs(&[redirection.descriptor.as_digit()], text);
                cmdputs(
                    match redirection.operator {
                        DescriptorRedirectionOperator::Input => b"<&",
                        DescriptorRedirectionOperator::Output => b">&",
                    },
                    text,
                );
                match &redirection.target {
                    DescriptorTarget::Number(descriptor) => cmdputs(&[descriptor.as_digit()], text),
                    DescriptorTarget::Close => cmdputs(b"-", text),
                    DescriptorTarget::Word(word) => word.word.render(text),
                }
            }
            Redirection::HereDocument(_) => cmdputs(b"<<...", text),
        }
    }
}

// [spec:dash:def:jobs.cmdputs-fn]
// [spec:dash:sem:jobs.cmdputs-fn]
fn cmdputs(s: &[u8], text: &mut BString) {
    for &byte in s {
        if matches!(byte, b'\'' | b'\\' | b'"' | b'$') {
            text.push(b'\\');
        }
        text.push(byte);
    }
    /* The C leaves an unadvanced `*nextc = '\0'` for `commandtext` to
     * read as the end of the text. The length is that. */
}

// [spec:dash:def:jobs.showpipe-fn]
// [spec:dash:sem:jobs.showpipe-fn]
pub(crate) fn showpipe(sh: &mut crate::context::Shell, jp: JobId, dest: Dest) {
    let spend: usize = sh.jobs[jp].ps.len();

    for sp in 1..spend {
        let _ = sh.io.get(dest).write_all(b" | ");
        outcmd(sh, jp, sp, dest);
    }
    let _ = sh.io.get(dest).write_all(b"\n");
    sh.io.flushall();
}

// [spec:dash:def:jobs.xtcsetpgrp-fn]
// [spec:dash:sem:jobs.xtcsetpgrp-fn]
fn xtcsetpgrp(
    sh: &mut crate::context::Shell,
    fd: &impl nsh_platform::AsDescriptor,
    group: ProcessGroupState,
) -> Result<(), Error> {
    let blocked = nsh_platform::BlockedSignals::all()
        .expect("blocking signals around terminal handoff failed");
    let result = nsh_platform::set_foreground_process_group(fd, group);
    drop(blocked);

    if let Err(error) = result {
        let mut message = b"Cannot set tty process group (".to_vec();
        message.extend_from_slice(sh.locale.error_message(&error).as_bytes());
        message.push(b')');
        return Err(sh.sh_error_value(&message));
    }
    Ok(())
}

// [spec:dash:def:jobs.getstatus-fn]
// [spec:dash:sem:jobs.getstatus-fn]
// [spec:posix:req:exit.status-normal-termination]
// [spec:posix:req:exit.status-signal-terminated]
pub(crate) fn getstatus(sh: &mut crate::context::Shell, jobp: JobId) -> crate::status::ExitStatus {
    let mut status: ChildStatus;
    let mut ps: usize;

    /* `job->ps + job->nprocs - 1` in C: the bitfield promotes to `int`,
     * so a job that has not forked yet reads `ps[-1]`. It has no status
     * to report; `wait %n` on one answers 0. */
    ps = sh.jobs[jobp].ps.len();
    status = if ps == 0 {
        ChildStatus::Exited(0)
    } else {
        sh.jobs[jobp].ps[ps - 1]
            .status
            .unwrap_or(ChildStatus::Exited(0))
    };
    if sh.options.enabled(ShellOption::Pipefail) {
        loop {
            if status != ChildStatus::Exited(0) {
                break;
            }
            if ps < 2 {
                break;
            }
            ps -= 1;
            status = sh.jobs[jobp].ps[ps - 1]
                .status
                .unwrap_or(ChildStatus::Exited(0));
        }
    }

    let retval = match status {
        ChildStatus::Exited(code) => crate::status::ExitStatus::from(code),
        ChildStatus::Signaled { signal, .. } => {
            if signal == nsh_platform::interrupt_signal() {
                sh.jobs[jobp].sigint = true;
            }
            crate::status::ExitStatus::from_code(signal.number() + 128)
        }
        ChildStatus::Stopped(signal) => crate::status::ExitStatus::from_code(signal.number() + 128),
        ChildStatus::Continued => crate::status::ExitStatus::SUCCESS,
    };
    /* TRACE(("getstatus: job %d, nproc %d, status %x, retval %x\n", ...)); */
    retval
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_status_derives_job_state() {
        let first = ProcessId::new(1).unwrap();
        let second = ProcessId::new(2).unwrap();
        let mut job = Job::new();
        job.ps = vec![
            ProcStat {
                pid: first,
                status: None,
                cmd: BString::default(),
            },
            ProcStat {
                pid: second,
                status: None,
                cmd: BString::default(),
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
            DOWAIT_BLOCK,
            JobState::Done,
            true,
            true,
            true,
            false,
        ));
        assert!(!notify_completion_now(
            DOWAIT_NONBLOCK,
            JobState::Done,
            true,
            true,
            true,
            false,
        ));
        assert!(!notify_completion_now(
            DOWAIT_BLOCK,
            JobState::Stopped,
            true,
            true,
            true,
            false,
        ));
        assert!(!notify_completion_now(
            DOWAIT_BLOCK,
            JobState::Done,
            false,
            true,
            true,
            false,
        ));
        assert!(!notify_completion_now(
            DOWAIT_BLOCK,
            JobState::Done,
            true,
            false,
            true,
            false,
        ));
        assert!(!notify_completion_now(
            DOWAIT_BLOCK,
            JobState::Done,
            true,
            true,
            false,
            false,
        ));
        assert!(!notify_completion_now(
            DOWAIT_BLOCK,
            JobState::Done,
            true,
            true,
            true,
            true,
        ));
    }
}
