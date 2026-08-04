# Job Control, Signals, and Execution Environment

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
"SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in RFC 2119. This document
reproduces normative text from IEEE Std 1003.1-2024 (POSIX.1-2024),
Copyright © 2001-2024 The IEEE and The Open Group.

## 2.11 Job Control

> [spec:posix:def:jobctl.definition]
> Job control is defined (see XBD 3.181 Job Control) as a facility that allows
> users selectively to stop (suspend) the execution of processes and continue
> (resume) their execution at a later point. It is jointly supplied by the
> terminal I/O driver and a command interpreter. The shell is one such command
> interpreter and job control in the shell is enabled by `set -m` (which is
> enabled by default in interactive shells).
>
> Requirements relating to background jobs stated in this section only apply to
> job-control background jobs.
>
> Source: XCU 2.11 Job Control — utilities/V3_chap02.html#tag_19_11

> [spec:posix:req:jobctl.initial-foreground-process-group]
> If the shell has a controlling terminal and it is the controlling process for
> the terminal session, it shall initially set the foreground process group ID
> associated with the terminal to its own process group ID. Otherwise, if it has
> a controlling terminal, it shall initially perform the following steps if
> interactive and may perform them if non-interactive:
>
> 1. If its process group is the foreground process group associated with the
>    terminal, the shell shall set its process group ID to its process ID (if
>    they are not already equal) and set the foreground process group ID
>    associated with the terminal to its process group ID.
> 2. If its process group is not the foreground process group associated with
>    the terminal (which would result from it being started by a job-control
>    shell as a background job), the shell shall either stop itself by sending
>    itself a SIGTTIN signal or, if interactive, attempt to read from standard
>    input (which generates a SIGTTIN signal if standard input is the
>    controlling terminal). If it is stopped, then when it continues execution
>    (after receiving a SIGCONT signal) it shall repeat these steps.
>
> Subsequently, the shell shall change the foreground process group associated
> with its controlling terminal when a foreground job is running as noted in the
> description below.
>
> Source: XCU 2.11 Job Control — utilities/V3_chap02.html#tag_19_11

> [spec:posix:req:jobctl.job-creation]
> When job control is enabled, the shell shall create one or more jobs when it
> executes a list (see 2.9.3 Lists) that has one of the following forms:
>
> - A single asynchronous AND-OR list
> - One or more sequentially executed AND-OR lists followed by at most one
>   asynchronous AND-OR list
>
> Source: XCU 2.11 Job Control — utilities/V3_chap02.html#tag_19_11

> [spec:posix:req:jobctl.list-splitting]
> For the purposes of job control, a list that includes more than one
> asynchronous AND-OR list shall be treated as if it were split into multiple
> separate lists, each ending with an asynchronous AND-OR list.
>
> Source: XCU 2.11 Job Control — utilities/V3_chap02.html#tag_19_11

> [spec:posix:def:jobctl.background-job]
> When a job consisting of a single asynchronous AND-OR list is created, it
> shall form a background job and the associated process ID shall be that of a
> child process that is made a process group leader, with all other processes
> (if any) that the shell creates to execute the AND-OR list initially having
> this process ID as their process group ID.
>
> Source: XCU 2.11 Job Control — utilities/V3_chap02.html#tag_19_11

> [spec:posix:def:jobctl.foreground-job]
> For a list consisting of one or more sequentially executed AND-OR lists
> followed by at most one asynchronous AND-OR list, the whole list shall form a
> single foreground job up until the sequentially executed AND-OR lists have all
> completed execution, at which point the asynchronous AND-OR list (if any)
> shall form a background job as described above.
>
> Source: XCU 2.11 Job Control — utilities/V3_chap02.html#tag_19_11

> [spec:posix:req:jobctl.pipeline-process-group]
> For each pipeline in a foreground job, if the pipeline is executed while the
> list is still a foreground job, the set of processes comprising the pipeline,
> and any processes descended from it, shall all be in the same process group,
> unless the shell executes some of the commands in the pipeline in the current
> shell execution environment and others in a subshell environment; in this case
> the process group ID of the current shell need not change (or cannot change if
> it is the session leader), and consequently the process group ID that the
> other processes all share may differ from the process group ID of the current
> shell (which means that a SIGSTOP, SIGTSTP, SIGTTIN, or SIGTTOU signal sent to
> one of those process groups does not cause the whole pipeline to stop).
>
> Source: XCU 2.11 Job Control — utilities/V3_chap02.html#tag_19_11

> [spec:posix:req:jobctl.background-job-brought-to-foreground]
> A background job that was created on execution of an asynchronous AND-OR list
> can be brought into the foreground by means of the `fg` utility (if
> supported); in this case the entire job shall become a single foreground job.
> If a process that the shell subsequently waits for is part of this foreground
> job and is stopped by a signal, the entire job shall become a suspended job
> and the behavior shall be as if the process had been stopped while the job was
> running in the background.
>
> Source: XCU 2.11 Job Control — utilities/V3_chap02.html#tag_19_11

> [spec:posix:req:jobctl.foreground-process-group-assignment]
> When a foreground job is created, or a background job is brought into the
> foreground by the `fg` utility, if the shell has a controlling terminal it
> shall set the foreground process group ID associated with the terminal as
> follows:
>
> - If the job was originally created as a background job, the foreground
>   process group ID shall be set to the process ID of the process that the
>   shell made a process group leader when it executed the asynchronous AND-OR
>   list.
> - If the job was originally created as a foreground job, the foreground
>   process group ID shall be set as follows when each pipeline in the job is
>   executed:
>   - If the shell is not itself executing, in the current shell execution
>     environment, all of the commands in the pipeline, the foreground process
>     group ID shall be set to the process group ID that is shared by the other
>     processes executing the pipeline.
>   - If all of the commands in the pipeline are being executed by the shell
>     itself in the current shell execution environment, the foreground process
>     group ID shall be set to the process group ID of the shell.
>
> Source: XCU 2.11 Job Control — utilities/V3_chap02.html#tag_19_11

> [spec:posix:req:jobctl.foreground-process-group-restored]
> When a foreground job terminates, or becomes a suspended job, if the shell has
> a controlling terminal it shall set the foreground process group ID associated
> with the terminal to the process group ID of the shell.
>
> Source: XCU 2.11 Job Control — utilities/V3_chap02.html#tag_19_11

> [spec:posix:req:jobctl.job-number-and-process-id]
> Each background job (whether suspended or not) shall have associated with it a
> job number and a process ID that is known in the current shell execution
> environment. When a background job is brought into the foreground by means of
> the `fg` utility, the associated job number shall be removed from the shell's
> background jobs list and the associated process ID shall be removed from the
> list of process IDs known in the current shell execution environment.
>
> Source: XCU 2.11 Job Control — utilities/V3_chap02.html#tag_19_11

> [spec:posix:req:jobctl.suspend-on-catchable-signal]
> If a process that the shell is waiting for is part of a foreground job that
> was started as a foreground job and is stopped by a catchable signal (SIGTSTP,
> SIGTTIN, or SIGTTOU):
>
> - If the currently executing AND-OR list within the list comprising the
>   foreground job consists of a single pipeline in which all of the commands
>   are simple commands, the shell shall either create a suspended job
>   consisting of at least that AND-OR list and the remaining (if any) AND-OR
>   lists in the same list, or create a suspended job consisting of just that
>   AND-OR list and discard the remaining (if any) AND-OR lists in the same
>   list.
> - Otherwise, the shell shall create a suspended job consisting of a set of
>   commands, from within the list comprising the foreground job, that is
>   unspecified except that the set shall include at least the pipeline to which
>   the stopped process belongs. Commands in the foreground job that have not
>   already completed and are not included in the suspended job shall be
>   discarded.
>
> Source: XCU 2.11 Job Control — utilities/V3_chap02.html#tag_19_11

> [spec:posix:req:jobctl.suspend-on-sigstop]
> If a process that the shell is waiting for is part of a foreground job that
> was started as a foreground job and is stopped by a SIGSTOP signal, the
> behavior shall be as described above for a catchable signal unless the shell
> was executing a built-in utility in the current shell execution environment
> when the SIGSTOP was delivered, resulting in the shell itself being stopped by
> the signal, in which case if the shell subsequently receives a SIGCONT signal
> and has one or more child processes that remain stopped, the shell shall
> create a suspended job as if only those child processes had been stopped.
>
> Source: XCU 2.11 Job Control — utilities/V3_chap02.html#tag_19_11

> [spec:posix:req:jobctl.suspended-job-message]
> When a suspended job is created as a result of a foreground job being stopped,
> it shall be assigned a job number, and an interactive shell shall write, and a
> non-interactive shell may write, a message to standard error, formatted as
> described by the `jobs` utility (without the `-l` option) for a suspended job.
> The message may indicate that the commands comprising the job include commands
> that have already completed; in this case the completed commands shall not be
> repeated if execution of the job is subsequently continued.
>
> Source: XCU 2.11 Job Control — utilities/V3_chap02.html#tag_19_11

> [spec:posix:req:jobctl.save-terminal-settings]
> When a suspended job is created as a result of a foreground job being stopped,
> if the shell is interactive, it shall save the terminal settings before
> changing them to the settings it needs to read further commands.
>
> Source: XCU 2.11 Job Control — utilities/V3_chap02.html#tag_19_11

> [spec:posix:req:jobctl.background-job-suspended-message]
> When a process associated with a background job is stopped by a SIGSTOP,
> SIGTSTP, SIGTTIN, or SIGTTOU signal, the shell shall convert the
> (non-suspended) background job into a suspended job and an interactive shell
> shall write a message to standard error, formatted as described by the `jobs`
> utility (without the `-l` option) for a suspended job, at the following time:
>
> - If `set -b` is enabled, the message shall be written either immediately
>   after the job became suspended or immediately prior to writing the next
>   prompt for input.
> - If `set -b` is disabled, the message shall be written immediately prior to
>   writing the next prompt for input.
>
> Source: XCU 2.11 Job Control — utilities/V3_chap02.html#tag_19_11

> [spec:posix:req:jobctl.continue-suspended-job]
> Execution of a suspended job can be continued as a foreground job by means of
> the `fg` utility (if supported), or as a (non-suspended) background job either
> by means of the `bg` utility (if supported) or by sending the stopped
> processes a SIGCONT signal. The `fg` and `bg` utilities shall send a SIGCONT
> signal to the process group of the process(es) whose stopped wait status
> caused the shell to suspend the job.
>
> Source: XCU 2.11 Job Control — utilities/V3_chap02.html#tag_19_11

> [spec:posix:req:jobctl.fg-terminal-settings-restore]
> If the shell has a controlling terminal, the `fg` utility shall send the
> SIGCONT signal after it has set the foreground process group ID associated
> with the terminal. If the `fg` utility is used from an interactive shell to
> bring into the foreground a suspended job that was created from a foreground
> job, before it sends the SIGCONT signal the `fg` utility shall restore the
> terminal settings to the ones that the shell saved when the job was suspended.
>
> Source: XCU 2.11 Job Control — utilities/V3_chap02.html#tag_19_11

> [spec:posix:req:jobctl.background-job-completion-message]
> When a background job completes or is terminated by a signal, an interactive
> shell shall write a message to standard error, formatted as described by the
> `jobs` utility (without the `-l` option) for a job that completed or was
> terminated by a signal, respectively, at the following time:
>
> - If `set -b` is enabled, the message shall be written immediately after the
>   job completes or is terminated.
> - If `set -b` is disabled, the message shall be written immediately prior to
>   writing the next prompt for input.
>
> Source: XCU 2.11 Job Control — utilities/V3_chap02.html#tag_19_11

> [spec:posix:req:jobctl.non-interactive-message-timing]
> In each case above where an interactive shell writes a message immediately
> prior to writing the next prompt for input, the same message may also be
> written by a non-interactive shell, at any of the following times:
>
> - After the next time a foreground job terminates or is suspended
> - Before the shell parses further input
> - Before the shell exits
>
> Source: XCU 2.11 Job Control — utilities/V3_chap02.html#tag_19_11

## 2.12 Signals and Error Handling

> [spec:posix:req:signal.async-list-sigint-sigquit-ignored]
> If job control is disabled (see the description of `set -m`) when the shell
> executes an asynchronous AND-OR list, the commands in the list shall inherit
> from the shell a signal action of ignored (SIG_IGN) for the SIGINT and SIGQUIT
> signals.
>
> Source: XCU 2.12 Signals and Error Handling — utilities/V3_chap02.html#tag_19_12

> [spec:posix:req:signal.inherited-actions]
> In all cases other than the commands of an asynchronous AND-OR list executed
> while job control is disabled, commands executed by the shell shall inherit
> the same signal actions as those inherited by the shell from its parent unless
> a signal action is modified by the `trap` special built-in.
>
> Source: XCU 2.12 Signals and Error Handling — utilities/V3_chap02.html#tag_19_12

> [spec:posix:req:signal.trap-deferred-until-foreground-command-completes]
> When a signal for which a trap has been set is received while the shell is
> waiting for the completion of a utility executing a foreground command, the
> trap associated with that signal shall not be executed until after the
> foreground command has completed.
>
> Source: XCU 2.12 Signals and Error Handling — utilities/V3_chap02.html#tag_19_12

> [spec:posix:req:signal.trap-during-wait]
> When the shell is waiting, by means of the `wait` utility, for asynchronous
> commands to complete, the reception of a signal for which a trap has been set
> shall cause the `wait` utility to return immediately with an exit status >128,
> immediately after which the trap associated with that signal shall be taken.
>
> Source: XCU 2.12 Signals and Error Handling — utilities/V3_chap02.html#tag_19_12

> [spec:posix:sem:signal.pending-trap-order]
> If multiple signals are pending for the shell for which there are associated
> trap actions, the order of execution of trap actions is unspecified.
>
> Source: XCU 2.12 Signals and Error Handling — utilities/V3_chap02.html#tag_19_12

## 2.13 Shell Execution Environment

> [spec:posix:def:shenv.components]
> A shell execution environment consists of the following:
>
> - Open files inherited upon invocation of the shell, plus open files
>   controlled by `exec`
> - Working directory as set by `cd`
> - File creation mask set by `umask`
> - File size limit as set by `ulimit`
> - Current traps set by `trap`
> - Shell parameters that are set by variable assignment (see the `set` special
>   built-in) or from the System Interfaces volume of POSIX.1-2024 environment
>   inherited by the shell when it begins (see the `export` special built-in)
> - Shell functions; see 2.9.5 Function Definition Command
> - Options turned on at invocation or by `set`
> - Background jobs and their associated process IDs, and process IDs of child
>   processes created to execute asynchronous AND-OR lists while job control is
>   disabled; together these process IDs constitute the process IDs "known to
>   this shell environment". If the implementation supports non-job-control
>   background jobs, the list of known process IDs and the list of background
>   jobs may form a single list even though this standard describes them as
>   being updated separately. See 2.9.3.1 Asynchronous AND-OR Lists
> - Shell aliases; see 2.3.1 Alias Substitution
>
> Source: XCU 2.13 Shell Execution Environment — utilities/V3_chap02.html#tag_19_13

> [spec:posix:req:shenv.utility-environment]
> Utilities other than the special built-ins (see 2.15 Special Built-In
> Utilities) shall be invoked in a separate environment that consists of the
> following. The initial value of these objects shall be the same as that for
> the parent shell, except as noted below.
>
> - Open files inherited on invocation of the shell, open files controlled by
>   the `exec` special built-in plus any modifications, and additions specified
>   by any redirections to the utility
> - Current working directory
> - File creation mask
> - If the utility is a shell script, traps caught by the shell shall be set to
>   the default values and traps ignored by the shell shall be set to be ignored
>   by the utility; if the utility is not a shell script, the trap actions
>   (default or ignore) shall be mapped into the appropriate signal handling
>   actions for the utility
> - Variables with the `export` attribute, along with those explicitly exported
>   for the duration of the command, shall be passed to the utility environment
>   variables
> - It is unspecified whether environment variables that were passed to the
>   invoking shell when it was invoked itself, but were not used to initialize
>   shell variables (see 2.5.3 Shell Variables) because they had invalid names,
>   are included in the invoked utility's environment.
>
> Source: XCU 2.13 Shell Execution Environment — utilities/V3_chap02.html#tag_19_13

> [spec:posix:req:shenv.utility-does-not-change-shell-environment]
> The environment of the shell process shall not be changed by the utility
> unless explicitly specified by the utility description (for example, `cd` and
> `umask`).
>
> Source: XCU 2.13 Shell Execution Environment — utilities/V3_chap02.html#tag_19_13

> [spec:posix:req:shenv.subshell-creation]
> A subshell environment shall be created as a duplicate of the shell
> environment, except that:
>
> - Unless specified otherwise (see `trap`), traps that are not being ignored
>   shall be set to the default action.
> - If the shell is interactive, the subshell shall behave as a non-interactive
>   shell in all respects except:
>   - The expansion of the special parameter `'-'` may continue to indicate that
>     it is interactive.
>   - The `set -n` option may be ignored.
>
> Source: XCU 2.13 Shell Execution Environment — utilities/V3_chap02.html#tag_19_13

> [spec:posix:req:shenv.subshell-isolation]
> Changes made to the subshell environment shall not affect the shell
> environment.
>
> Source: XCU 2.13 Shell Execution Environment — utilities/V3_chap02.html#tag_19_13

> [spec:posix:req:shenv.subshell-contexts]
> Command substitution, commands that are grouped with parentheses, and
> asynchronous AND-OR lists shall be executed in a subshell environment.
> Additionally, each command of a multi-command pipeline is in a subshell
> environment; as an extension, however, any or all commands in a pipeline may
> be executed in the current environment. Except where otherwise stated, all
> other commands shall be executed in the current shell environment.
>
> Source: XCU 2.13 Shell Execution Environment — utilities/V3_chap02.html#tag_19_13
