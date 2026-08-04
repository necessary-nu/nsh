# Intrinsic Utilities: bg, fg, and jobs

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
"SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in RFC 2119. This document
reproduces normative text from IEEE Std 1003.1-2024 (POSIX.1-2024),
Copyright © 2001-2024 The IEEE and The Open Group.

Option-conditional text carries the standard's own margin code inline, at the point the standard shades it:

`[UP]`
: User Portability Utilities. The functionality described is optional.

`[XSI]`
: X/Open System Interfaces. The functionality described is an extension, available on all systems supporting the XSI option.

On each of these three pages the `UP` shading covers the SYNOPSIS box and
nothing else: the `[Option Start]` marker opens immediately before the utility
name and `[Option End]` closes immediately after the last operand. That single
bracket is how the standard marks the utility as a whole as part of the User
Portability Utilities option, so the `UP` code appears here on the synopsis
rules. The DESCRIPTION, OPTIONS, OPERANDS, STDOUT, EXIT STATUS, and
CONSEQUENCES OF ERRORS sections of all three pages are printed unshaded and are
reproduced unshaded below. The only other shaded span on these pages is the
`XSI` bracket around the NLSPATH description.

The job-control machinery these utilities drive — creation of background and
suspended jobs, process-group and controlling-terminal handling, the SIGCONT
delivery that resumes a suspended job, and the asynchronous status messages the
shell writes — is specified in XCU 2.11 Job Control and is covered by the
`jobctl.` rules in `execution.md`. Those rules are not restated here.

## bg

### bg SYNOPSIS

> [spec:posix:syn:builtin.bg.synopsis]
> `[UP]` The synopsis of the bg utility is `bg [job_id...]`.
>
> Source: XCU bg SYNOPSIS — utilities/bg.html#tag_20_10_02

### bg DESCRIPTION

The resumption itself — sending SIGCONT to the process group of the stopped
processes — is required by `[spec:posix:req:jobctl.continue-suspended-job]` in
`execution.md`.

> [spec:posix:req:builtin.bg.resume-suspended-jobs]
> If job control is enabled (see the description of set -m), the shell is
> interactive, and the current shell execution environment (see 2.13 Shell
> Execution Environment) is not a subshell environment, the bg utility shall
> resume suspended jobs from the current shell execution environment by running
> them as background jobs, as described in 2.11 Job Control; it may also do so
> if the shell is non-interactive or the current shell execution environment is
> a subshell environment.
>
> Source: XCU bg DESCRIPTION — utilities/bg.html#tag_20_10_03

> [spec:posix:req:builtin.bg.already-running-no-effect]
> If the job specified by job_id is already a running background job, the bg
> utility shall have no effect and shall exit successfully.
>
> Source: XCU bg DESCRIPTION — utilities/bg.html#tag_20_10_03

### bg OPERANDS

> [spec:posix:req:builtin.bg.operand-job-id]
> The following operand shall be supported:
>
> job_id — Specify the job to be resumed as a background job. If no job_id
> operand is given, the most recently suspended job shall be used. The format of
> job_id is described in XBD 3.182 Job ID.
>
> Source: XCU bg OPERANDS — utilities/bg.html#tag_20_10_05

### bg ENVIRONMENT VARIABLES

> [spec:posix:req:builtin.bg.env-locale]
> The following environment variables shall affect the execution of bg:
>
> LANG — Provide a default value for the internationalization variables that are
> unset or null. (See XBD 8.2 Internationalization Variables for the precedence
> of internationalization variables used to determine the values of locale
> categories.)
>
> LC_ALL — If set to a non-empty string value, override the values of all the
> other internationalization variables.
>
> LC_CTYPE — Determine the locale for the interpretation of sequences of bytes
> of text data as characters (for example, single-byte as opposed to multi-byte
> characters in arguments).
>
> LC_MESSAGES — Determine the locale that should be used to affect the format
> and contents of diagnostic messages written to standard error.
>
> Source: XCU bg ENVIRONMENT VARIABLES — utilities/bg.html#tag_20_10_08

> [spec:posix:sem:builtin.bg.env-nlspath]
> `[XSI]` NLSPATH determines the location of messages objects and message
> catalogs.
>
> Source: XCU bg ENVIRONMENT VARIABLES — utilities/bg.html#tag_20_10_08

### bg STDOUT

> [spec:posix:req:builtin.bg.stdout-format]
> The output of bg shall consist of a line in the format:
>
> `"[%d] %s\n", <job-number>, <command>`
>
> where the fields are as follows:
>
> `<job-number>` — A number that can be used to identify the job to the wait,
> fg, and kill utilities. Using these utilities, the job can be identified by
> prefixing the job number with `'%'`.
>
> `<command>` — The associated command that was given to the shell.
>
> Source: XCU bg STDOUT — utilities/bg.html#tag_20_10_10

### bg STDERR

> [spec:posix:req:builtin.bg.stderr]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU bg STDERR — utilities/bg.html#tag_20_10_11

### bg EXIT STATUS

> [spec:posix:req:builtin.bg.exit-status]
> The following exit values shall be returned:
>
> | Exit status | Meaning |
> |---|---|
> | 0 | Successful completion. |
> | >0 | An error occurred. |
>
> Source: XCU bg EXIT STATUS — utilities/bg.html#tag_20_10_14

### bg CONSEQUENCES OF ERRORS

> [spec:posix:req:builtin.bg.job-control-disabled]
> If job control is disabled, the bg utility shall exit with an error and no job
> shall be placed in the background.
>
> Source: XCU bg CONSEQUENCES OF ERRORS — utilities/bg.html#tag_20_10_15

### bg — sections with no additional requirements

> [spec:posix:req:builtin.bg.interfaces]
> The bg utility has no options. Standard input is not used; there are no input
> files; asynchronous events are handled as for the utility description
> defaults; there are no output files; and there is no extended description.
>
> Source: XCU bg — utilities/bg.html#tag_20_10

## fg

### fg SYNOPSIS

> [spec:posix:syn:builtin.fg.synopsis]
> `[UP]` The synopsis of the fg utility is `fg [job_id]`.
>
> Source: XCU fg SYNOPSIS — utilities/fg.html#tag_20_45_02

### fg DESCRIPTION

The process-group and controlling-terminal work fg performs, including the
requirement that it send SIGCONT after setting the foreground process group ID
and that it restore saved terminal settings, is stated by
`[spec:posix:req:jobctl.fg-terminal-settings-restore]` in `execution.md`.

> [spec:posix:req:builtin.fg.move-job-to-foreground]
> If job control is enabled (see the description of set -m), the shell is
> interactive, and the current shell execution environment (see 2.13 Shell
> Execution Environment) is not a subshell environment, the fg utility shall
> move a background job in the current execution environment into the
> foreground, as described in 2.11 Job Control; it may also do so if the shell
> is non-interactive or the current shell execution environment is a subshell
> environment.
>
> Source: XCU fg DESCRIPTION — utilities/fg.html#tag_20_45_03

> [spec:posix:req:builtin.fg.removes-known-process-id]
> Using fg to place a job into the foreground shall remove its process ID from
> the list of those "known in the current shell execution environment"; see
> 2.9.3.1 Asynchronous AND-OR Lists.
>
> Source: XCU fg DESCRIPTION — utilities/fg.html#tag_20_45_03

### fg OPERANDS

> [spec:posix:req:builtin.fg.operand-job-id]
> The following operand shall be supported:
>
> job_id — Specify the job to be run as a foreground job. If no job_id operand
> is given, the job_id for the job that was most recently suspended, placed in
> the background, or run as a background job shall be used. The format of job_id
> is described in XBD 3.182 Job ID.
>
> Source: XCU fg OPERANDS — utilities/fg.html#tag_20_45_05

### fg ENVIRONMENT VARIABLES

> [spec:posix:req:builtin.fg.env-locale]
> The following environment variables shall affect the execution of fg:
>
> LANG — Provide a default value for the internationalization variables that are
> unset or null. (See XBD 8.2 Internationalization Variables for the precedence
> of internationalization variables used to determine the values of locale
> categories.)
>
> LC_ALL — If set to a non-empty string value, override the values of all the
> other internationalization variables.
>
> LC_CTYPE — Determine the locale for the interpretation of sequences of bytes
> of text data as characters (for example, single-byte as opposed to multi-byte
> characters in arguments).
>
> LC_MESSAGES — Determine the locale that should be used to affect the format
> and contents of diagnostic messages written to standard error.
>
> Source: XCU fg ENVIRONMENT VARIABLES — utilities/fg.html#tag_20_45_08

> [spec:posix:sem:builtin.fg.env-nlspath]
> `[XSI]` NLSPATH determines the location of messages objects and message
> catalogs.
>
> Source: XCU fg ENVIRONMENT VARIABLES — utilities/fg.html#tag_20_45_08

### fg STDOUT

> [spec:posix:req:builtin.fg.stdout-format]
> The fg utility shall write the command line of the job to standard output in
> the following format:
>
> `"%s\n", <command>`
>
> Source: XCU fg STDOUT — utilities/fg.html#tag_20_45_10

### fg STDERR

> [spec:posix:req:builtin.fg.stderr]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU fg STDERR — utilities/fg.html#tag_20_45_11

### fg EXIT STATUS

> [spec:posix:req:builtin.fg.exit-status]
> If the fg utility succeeds, it does not return an exit status. Instead, the
> shell waits for the job that fg moved into the foreground.
>
> If fg does not move a job into the foreground, the following exit value shall
> be returned:
>
> | Exit status | Meaning |
> |---|---|
> | >0 | An error occurred. |
>
> Source: XCU fg EXIT STATUS — utilities/fg.html#tag_20_45_14

### fg CONSEQUENCES OF ERRORS

> [spec:posix:req:builtin.fg.job-control-disabled]
> If job control is disabled, the fg utility shall exit with an error and no job
> shall be placed in the foreground.
>
> Source: XCU fg CONSEQUENCES OF ERRORS — utilities/fg.html#tag_20_45_15

### fg — sections with no additional requirements

> [spec:posix:req:builtin.fg.interfaces]
> The fg utility has no options. Standard input is not used; there are no input
> files; asynchronous events are handled as for the utility description
> defaults; there are no output files; and there is no extended description.
>
> Source: XCU fg — utilities/fg.html#tag_20_45

## jobs

### jobs SYNOPSIS

> [spec:posix:syn:builtin.jobs.synopsis]
> `[UP]` The synopsis of the jobs utility is `jobs [-l|-p] [job_id...]`.
>
> Source: XCU jobs SYNOPSIS — utilities/jobs.html#tag_20_62_02

### jobs DESCRIPTION

The format in which the shell itself reports suspended, continued, and
completed background jobs is defined by reference to the jobs output format
without the -l option; see `[spec:posix:req:jobctl.suspended-job-message]` and
`[spec:posix:req:jobctl.background-job-completion-message]` in `execution.md`.

> [spec:posix:req:builtin.jobs.display-background-jobs]
> If the current shell execution environment (see 2.13 Shell Execution
> Environment) is not a subshell environment, the jobs utility shall display the
> status of background jobs that were created in the current shell execution
> environment; it may also do so if the current shell execution environment is a
> subshell environment.
>
> Source: XCU jobs DESCRIPTION — utilities/jobs.html#tag_20_62_03

> [spec:posix:req:builtin.jobs.remove-reported-job]
> When jobs reports the termination status of a job, the shell shall remove the
> job from the background jobs list and the associated process ID from the list
> of those "known in the current shell execution environment"; see 2.9.3.1
> Asynchronous AND-OR Lists. If a write error occurs when jobs writes to
> standard output, some process IDs might have been removed from the list but
> not successfully reported.
>
> Source: XCU jobs DESCRIPTION — utilities/jobs.html#tag_20_62_03

### jobs OPTIONS

> [spec:posix:req:builtin.jobs.utility-syntax-guidelines]
> The jobs utility shall conform to XBD 12.2 Utility Syntax Guidelines.
>
> Source: XCU jobs OPTIONS — utilities/jobs.html#tag_20_62_04

> [spec:posix:req:builtin.jobs.option-l]
> The following option shall be supported:
>
> -l — (The letter ell.) Provide more information about each job listed. See
> STDOUT for details.
>
> Source: XCU jobs OPTIONS — utilities/jobs.html#tag_20_62_04

> [spec:posix:req:builtin.jobs.option-p]
> The following option shall be supported:
>
> -p — Display only the process IDs for the process group leaders of
> job-control background jobs and the process IDs associated with
> non-job-control background jobs (if supported).
>
> Source: XCU jobs OPTIONS — utilities/jobs.html#tag_20_62_04

> [spec:posix:req:builtin.jobs.default-display]
> By default, the jobs utility shall display the status of all background jobs,
> both running and suspended, and all jobs whose status has changed and have not
> been reported by the shell.
>
> Source: XCU jobs OPTIONS — utilities/jobs.html#tag_20_62_04

### jobs OPERANDS

> [spec:posix:req:builtin.jobs.operand-job-id]
> The following operand shall be supported:
>
> job_id — Specifies the jobs for which the status is to be displayed. If no
> job_id is given, the status information for all jobs shall be displayed. The
> format of job_id is described in XBD 3.182 Job ID.
>
> Source: XCU jobs OPERANDS — utilities/jobs.html#tag_20_62_05

### jobs ENVIRONMENT VARIABLES

> [spec:posix:req:builtin.jobs.env-locale]
> The following environment variables shall affect the execution of jobs:
>
> LANG — Provide a default value for the internationalization variables that are
> unset or null. (See XBD 8.2 Internationalization Variables for the precedence
> of internationalization variables used to determine the values of locale
> categories.)
>
> LC_ALL — If set to a non-empty string value, override the values of all the
> other internationalization variables.
>
> LC_CTYPE — Determine the locale for the interpretation of sequences of bytes
> of text data as characters (for example, single-byte as opposed to multi-byte
> characters in arguments).
>
> LC_MESSAGES — Determine the locale that should be used to affect the format
> and contents of diagnostic messages written to standard error and informative
> messages written to standard output.
>
> Source: XCU jobs ENVIRONMENT VARIABLES — utilities/jobs.html#tag_20_62_08

> [spec:posix:sem:builtin.jobs.env-nlspath]
> `[XSI]` NLSPATH determines the location of messages objects and message
> catalogs.
>
> Source: XCU jobs ENVIRONMENT VARIABLES — utilities/jobs.html#tag_20_62_08

### jobs STDOUT

> [spec:posix:req:builtin.jobs.stdout-p-format]
> If the -p option is specified, the output shall consist of one line for each
> process ID:
>
> `"%d\n", <process ID>`
>
> Source: XCU jobs STDOUT — utilities/jobs.html#tag_20_62_10

> [spec:posix:req:builtin.jobs.stdout-default-format]
> Otherwise, if the -l option is not specified, the output shall be a series of
> lines of the form:
>
> `"[%d] %c %s %s\n", <job-number>, <current>, <state>, <command>`
>
> where the fields shall be as follows:
>
> `<job-number>` — A number that can be used to identify the job to the wait,
> fg, bg, and kill utilities. Using these utilities, the job can be identified
> by prefixing the job number with `'%'`.
>
> `<command>` — The associated command that was given to the shell.
>
> Source: XCU jobs STDOUT — utilities/jobs.html#tag_20_62_10

> [spec:posix:req:builtin.jobs.stdout-current-field]
> `<current>` — The character `'+'` identifies the job that would be used as a
> default for the fg or bg utilities; this job can also be specified using the
> job_id %+ or `"%%"`. The character `'-'` identifies the job that would become
> the default if the current default job were to exit; this job can also be
> specified using the job_id %-. For other jobs, this field is a <space>. At
> most one job can be identified with `'+'` and at most one job can be
> identified with `'-'`. If there is any suspended job, then the current job
> shall be a suspended job. If there are at least two suspended jobs, then the
> previous job also shall be a suspended job.
>
> Source: XCU jobs STDOUT — utilities/jobs.html#tag_20_62_10

> [spec:posix:def:builtin.jobs.stdout-state-strings]
> `<state>` — One of the following strings (in the POSIX locale):
>
> | String | Meaning |
> |---|---|
> | Running | Indicates that the job has not been suspended by a signal and has not exited. |
> | Done | Indicates that the job completed and returned exit status zero. |
> | Done(code) | Indicates that the job completed normally and that it exited with the specified non-zero exit status, code, expressed as a decimal number. |
> | Stopped | Indicates that the job was suspended by the SIGTSTP signal. |
> | Stopped (SIGTSTP) | Indicates that the job was suspended by the SIGTSTP signal. |
> | Stopped (SIGSTOP) | Indicates that the job was suspended by the SIGSTOP signal. |
> | Stopped (SIGTTIN) | Indicates that the job was suspended by the SIGTTIN signal. |
> | Stopped (SIGTTOU) | Indicates that the job was suspended by the SIGTTOU signal. |
>
> Source: XCU jobs STDOUT — utilities/jobs.html#tag_20_62_10

> [spec:posix:req:builtin.jobs.stdout-state-substitution]
> The implementation may substitute the string Suspended in place of Stopped. If
> the job was terminated by a signal, the format of `<state>` is unspecified, but
> it shall be visibly distinct from all of the other `<state>` formats shown here
> and shall indicate the name or description of the signal causing the
> termination.
>
> Source: XCU jobs STDOUT — utilities/jobs.html#tag_20_62_10

> [spec:posix:req:builtin.jobs.stdout-l-format]
> If the -l option is specified:
>
> - For job-control background jobs, a field containing the process group ID
>   shall be inserted before the `<state>` field. Also, more processes in a
>   process group may be output on separate lines, using only the process ID and
>   `<command>` fields.
> - For non-job-control background jobs (if supported), a field containing the
>   process ID associated with the job shall be inserted before the `<state>`
>   field. Also, more processes created to execute the job may be output on
>   separate lines, using only the process ID and `<command>` fields.
>
> Source: XCU jobs STDOUT — utilities/jobs.html#tag_20_62_10

### jobs STDERR

> [spec:posix:req:builtin.jobs.stderr]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU jobs STDERR — utilities/jobs.html#tag_20_62_11

### jobs EXIT STATUS

> [spec:posix:req:builtin.jobs.exit-status]
> The following exit values shall be returned:
>
> | Exit status | Meaning |
> |---|---|
> | 0 | The output specified in STDOUT was successfully written to standard output. |
> | >0 | An error occurred. |
>
> Source: XCU jobs EXIT STATUS — utilities/jobs.html#tag_20_62_14

### jobs — sections with no additional requirements

> [spec:posix:req:builtin.jobs.interfaces]
> Standard input is not used; there are no input files; asynchronous events are
> handled as for the utility description defaults; there are no output files;
> there is no extended description; and the consequences of errors are as for
> the utility description defaults.
>
> Source: XCU jobs — utilities/jobs.html#tag_20_62
