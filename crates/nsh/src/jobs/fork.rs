//! Forking a child, and what it inherits.
//!
//! Everything a child has to undo before it becomes something else: the
//! job table it must not reap from, the signal dispositions the shell set
//! for its own purposes, and the process group it belongs to -- which
//! both sides set, because whichever runs first must win the same way.

use super::*;

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
    nsh_platform::reset_coverage_counters();

    let parent_shell_level = shell.shell_level;
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
        let active_job: JobId = job_id.unwrap();

        let process_group = if shell.jobs[active_job].processes.is_empty() {
            ProcessGroupId::from_leader(nsh_platform::current_process_id())
        } else {
            ProcessGroupId::from_leader(shell.jobs[active_job].processes[0].process_id)
        };
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
        if job_id.is_some_and(|index| shell.jobs[index].processes.is_empty()) {
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
        let process_group = if shell.jobs[active_job].processes.is_empty() {
            ProcessGroupId::from_leader(process_id)
        } else {
            ProcessGroupId::from_leader(shell.jobs[active_job].processes[0].process_id)
        };
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
    let job_id = create_job(shell, 1);

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
