# Intrinsic Utilities: kill and wait

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
"SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in RFC 2119. This document
reproduces normative text from IEEE Std 1003.1-2024 (POSIX.1-2024),
Copyright © 2001-2024 The IEEE and The Open Group.

Option-conditional text carries the standard's own margin code inline, at the point the standard shades it:

`[XSI]`
: X/Open System Interfaces. The functionality described is an extension, available on all systems supporting the XSI option.

## kill — SYNOPSIS

> [spec:posix:syn:builtin.kill.synopsis]
> The kill utility has the following forms:
>
> `kill [-s signal_name] pid...`
>
> `kill -l [exit_status]`
>
> Source: XCU kill SYNOPSIS — utilities/kill.html#tag_20_64_02

> [spec:posix:syn:builtin.kill.synopsis-xsi]
> `[XSI]` The kill utility additionally has the following forms:
>
> `kill [-signal_name] pid...`
>
> `kill [-signal_number] pid...`
>
> Source: XCU kill SYNOPSIS — utilities/kill.html#tag_20_64_02

## kill — DESCRIPTION

> [spec:posix:req:builtin.kill.send-signal]
> The kill utility shall send a signal to the process or processes specified by
> each pid operand.
>
> For each pid operand, the kill utility shall perform actions equivalent to the
> kill() function defined in the System Interfaces volume of POSIX.1-2024 called
> with the following arguments:
>
> - The value of the pid operand shall be used as the pid argument.
> - The sig argument is the value specified by the **-s** option,
>   **-**signal_number option, or the **-**signal_name option, or by SIGTERM, if
>   none of these options is specified.
>
> Source: XCU kill DESCRIPTION — utilities/kill.html#tag_20_64_03

## kill — OPTIONS

> [spec:posix:req:builtin.kill.utility-syntax-guidelines]
> The kill utility shall conform to XBD 12.2 Utility Syntax Guidelines, `[XSI]`
> `[Option Start]` except that in the last two SYNOPSIS forms, the
> **-**signal_number and **-**signal_name options are usually more than a single
> character. `[Option End]`
>
> The following options shall be supported:
>
> Source: XCU kill OPTIONS — utilities/kill.html#tag_20_64_04

> [spec:posix:req:builtin.kill.option-l]
> **-l**: (The letter ell.) Write all values of signal_name supported by the
> implementation, if no operand is given. If an exit_status operand is given and
> it is a value of the `'?'` shell special parameter (see 2.5.2 Special
> Parameters and wait) corresponding to a process that was terminated or stopped
> by a signal, the signal_name corresponding to the signal that terminated or
> stopped the process shall be written. If an exit_status operand is given and it
> is the unsigned decimal integer value of a signal number, the signal_name (the
> symbolic constant name without the SIG prefix defined in the Base Definitions
> volume of POSIX.1-2024) corresponding to that signal shall be written.
> Otherwise, the results are unspecified.
>
> Source: XCU kill OPTIONS — utilities/kill.html#tag_20_64_04

> [spec:posix:req:builtin.kill.option-s]
> **-s** signal_name: Specify the signal to send, using one of the symbolic names
> defined in the `<signal.h>` header. Values of signal_name shall be recognized in
> a case-independent fashion, without the SIG prefix. In addition, the symbolic
> name 0 shall be recognized, representing the signal value zero. The
> corresponding signal shall be sent instead of SIGTERM.
>
> Source: XCU kill OPTIONS — utilities/kill.html#tag_20_64_04

> [spec:posix:req:builtin.kill.option-signal-name]
> `[XSI]` **-**signal_name: Equivalent to **-s** signal_name.
>
> Source: XCU kill OPTIONS — utilities/kill.html#tag_20_64_04

> [spec:posix:req:builtin.kill.option-signal-number]
> `[XSI]` **-**signal_number: Specify a non-negative decimal integer,
> signal_number, representing the signal to be used instead of SIGTERM, as the
> sig argument in the effective call to kill(). The correspondence between
> integer values and the sig value used is shown in the following list.
>
> The effects of specifying any signal_number other than those listed below are
> undefined.
>
> | signal_number | sig |
> |---|---|
> | 0 | 0 |
> | 1 | SIGHUP |
> | 2 | SIGINT |
> | 3 | SIGQUIT |
> | 6 | SIGABRT |
> | 9 | SIGKILL |
> | 14 | SIGALRM |
> | 15 | SIGTERM |
>
> Source: XCU kill OPTIONS — utilities/kill.html#tag_20_64_04

> [spec:posix:req:builtin.kill.negative-first-argument]
> `[XSI]` If the first argument is a negative integer, it shall be interpreted as
> a **-**signal_number option, not as a negative pid operand specifying a process
> group.
>
> Source: XCU kill OPTIONS — utilities/kill.html#tag_20_64_04

## kill — OPERANDS

> [spec:posix:req:builtin.kill.operand-pid-number]
> The following operands shall be supported:
>
> pid: A decimal integer specifying a process or process group to be signaled.
> The process or processes selected by positive, negative, and zero values of the
> pid operand shall be as described for the kill() function. If process number 0
> is specified, all processes in the current process group shall be signaled. For
> the effects of negative pid numbers, see the kill() function defined in the
> System Interfaces volume of POSIX.1-2024. If the first pid operand is negative,
> it should be preceded by `"--"` to keep it from being interpreted as an option.
>
> Source: XCU kill OPERANDS — utilities/kill.html#tag_20_64_05

> [spec:posix:def:builtin.kill.operand-pid-job-id]
> pid: A job ID (see XBD 3.182 Job ID) that identifies a process group in the
> case of a job-control background job, or a process ID in the case of a
> non-job-control background job (if supported), to be signaled. The job ID
> notation is applicable only for invocations of kill in the current shell
> execution environment; see 2.13 Shell Execution Environment.
>
> Note: The job ID type of pid is only available on systems supporting the User
> Portability Utilities option or supporting non-job-control background jobs.
>
> Source: XCU kill OPERANDS — utilities/kill.html#tag_20_64_05

> [spec:posix:def:builtin.kill.operand-exit-status]
> exit_status: A decimal integer specifying a signal number or the exit status of
> a process terminated by a signal.
>
> Source: XCU kill OPERANDS — utilities/kill.html#tag_20_64_05

## kill — ENVIRONMENT VARIABLES

> [spec:posix:req:builtin.kill.env-vars]
> The following environment variables shall affect the execution of kill:
>
> LANG: Provide a default value for the internationalization variables that are
> unset or null. (See XBD 8.2 Internationalization Variables for the precedence
> of internationalization variables used to determine the values of locale
> categories.)
>
> LC_ALL: If set to a non-empty string value, override the values of all the
> other internationalization variables.
>
> LC_CTYPE: Determine the locale for the interpretation of sequences of bytes of
> text data as characters (for example, single-byte as opposed to multi-byte
> characters in arguments).
>
> LC_MESSAGES: Determine the locale that should be used to affect the format and
> contents of diagnostic messages written to standard error.
>
> Source: XCU kill ENVIRONMENT VARIABLES — utilities/kill.html#tag_20_64_08

> [spec:posix:sem:builtin.kill.env-nlspath]
> `[XSI]` NLSPATH: Determine the location of messages objects and message
> catalogs.
>
> Source: XCU kill ENVIRONMENT VARIABLES — utilities/kill.html#tag_20_64_08

## kill — STDOUT

> [spec:posix:req:builtin.kill.stdout-unused-without-l]
> When the **-l** option is not specified, the standard output shall not be used.
>
> Source: XCU kill STDOUT — utilities/kill.html#tag_20_64_10

> [spec:posix:req:builtin.kill.stdout-signal-list-format]
> When the **-l** option is specified, the symbolic name of each signal shall be
> written in the following format:
>
> `"%s%c", <signal_name>, <separator>`
>
> where the `<signal_name>` is in uppercase, without the SIG prefix, and the
> `<separator>` shall be either a <newline> or a <space>. For the last signal
> written, `<separator>` shall be a <newline>.
>
> Source: XCU kill STDOUT — utilities/kill.html#tag_20_64_10

> [spec:posix:req:builtin.kill.stdout-exit-status-format]
> When both the **-l** option and exit_status operand are specified, the symbolic
> name of the corresponding signal shall be written in the following format:
>
> `"%s\n", <signal_name>`
>
> Source: XCU kill STDOUT — utilities/kill.html#tag_20_64_10

## kill — STDERR

> [spec:posix:req:builtin.kill.stderr]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU kill STDERR — utilities/kill.html#tag_20_64_11

## kill — Remaining Interfaces

> [spec:posix:req:builtin.kill.interfaces]
> Standard input is not used; there are no input files; asynchronous events are
> handled as for the utility description defaults; there are no output files;
> there is no extended description; and the consequences of errors are as for the
> utility description defaults.
>
> Source: XCU kill — utilities/kill.html#tag_20_64

## kill — EXIT STATUS

> [spec:posix:req:builtin.kill.exit-status]
> The following exit values shall be returned:
>
> | Exit status | Meaning |
> |---|---|
> | 0 | The **-l** option was specified and the output specified in STDOUT was successfully written to standard output; or, the **-l** option was not specified, at least one matching process was found for each pid operand, and the specified signal was successfully processed for at least one matching process. |
> | >0 | An error occurred. |
>
> Source: XCU kill EXIT STATUS — utilities/kill.html#tag_20_64_14

## wait — SYNOPSIS

> [spec:posix:syn:builtin.wait.synopsis]
> The synopsis of the wait utility is `wait [pid...]`.
>
> Source: XCU wait SYNOPSIS — utilities/wait.html#tag_20_147_02

## wait — DESCRIPTION

> [spec:posix:req:builtin.wait.await-children]
> The wait utility shall wait for one or more child processes whose process IDs
> are known in the current shell execution environment (see 2.13 Shell Execution
> Environment) to terminate.
>
> Source: XCU wait DESCRIPTION — utilities/wait.html#tag_20_147_03

> [spec:posix:req:builtin.wait.no-operands]
> If the wait utility is invoked with no operands, it shall wait until all
> process IDs known to the invoking shell have terminated and exit with a zero
> exit status.
>
> Source: XCU wait DESCRIPTION — utilities/wait.html#tag_20_147_03

> [spec:posix:req:builtin.wait.pid-operands]
> If one or more pid operands are specified that represent known process IDs, the
> wait utility shall wait until all of them have terminated. If one or more pid
> operands are specified that represent unknown process IDs, wait shall treat
> them as if they were known process IDs that exited with exit status 127. The
> exit status returned by the wait utility shall be the exit status of the
> process requested by the last pid operand.
>
> Source: XCU wait DESCRIPTION — utilities/wait.html#tag_20_147_03

> [spec:posix:req:builtin.wait.remove-waited-for-pid]
> Once a process ID that is known in the current shell execution environment (see
> 2.13 Shell Execution Environment) has been successfully waited for, it shall be
> removed from the list of process IDs that are known in the current shell
> execution environment. If the process ID is associated with a background job,
> the corresponding job shall also be removed from the list of background jobs.
>
> Source: XCU wait DESCRIPTION — utilities/wait.html#tag_20_147_03

## wait — OPERANDS

> [spec:posix:def:builtin.wait.operand-pid-number]
> The following operand shall be supported:
>
> pid: The unsigned decimal integer process ID of a child process whose
> termination the utility is to wait for.
>
> Source: XCU wait OPERANDS — utilities/wait.html#tag_20_147_05

> [spec:posix:req:builtin.wait.operand-pid-job-id]
> pid: A job ID (see XBD 3.182 Job ID) that identifies a process group in the
> case of a job-control background job, or a process ID in the case of a
> non-job-control background job (if supported), to be waited for. The job ID
> notation is applicable only for invocations of wait in the current shell
> execution environment; see 2.13 Shell Execution Environment. The exit status of
> wait shall be determined by the exit status of the last pipeline to be
> executed.
>
> Note: The job ID type of pid is only available on systems supporting the User
> Portability Utilities option or supporting non-job-control background jobs.
>
> Source: XCU wait OPERANDS — utilities/wait.html#tag_20_147_05

## wait — ENVIRONMENT VARIABLES

> [spec:posix:req:builtin.wait.env-vars]
> The following environment variables shall affect the execution of wait:
>
> LANG: Provide a default value for the internationalization variables that are
> unset or null. (See XBD 8.2 Internationalization Variables for the precedence
> of internationalization variables used to determine the values of locale
> categories.)
>
> LC_ALL: If set to a non-empty string value, override the values of all the
> other internationalization variables.
>
> LC_CTYPE: Determine the locale for the interpretation of sequences of bytes of
> text data as characters (for example, single-byte as opposed to multi-byte
> characters in arguments).
>
> LC_MESSAGES: Determine the locale that should be used to affect the format and
> contents of diagnostic messages written to standard error.
>
> Source: XCU wait ENVIRONMENT VARIABLES — utilities/wait.html#tag_20_147_08

> [spec:posix:sem:builtin.wait.env-nlspath]
> `[XSI]` NLSPATH: Determine the location of messages objects and message
> catalogs.
>
> Source: XCU wait ENVIRONMENT VARIABLES — utilities/wait.html#tag_20_147_08

## wait — STDERR

> [spec:posix:req:builtin.wait.stderr]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU wait STDERR — utilities/wait.html#tag_20_147_11

## wait — Remaining Interfaces

> [spec:posix:req:builtin.wait.interfaces]
> The wait utility has no options. Standard input is not used; there are no input
> files; asynchronous events are handled as for the utility description defaults;
> standard output is not used; there are no output files; there is no extended
> description; and the consequences of errors are as for the utility description
> defaults.
>
> Source: XCU wait — utilities/wait.html#tag_20_147

## wait — EXIT STATUS

> [spec:posix:req:builtin.wait.exit-status-last-operand]
> If one or more operands were specified, all of them have terminated or were not
> known in the invoking shell execution environment, and the status of the last
> operand specified is known, then the exit status of wait shall be the status of
> the last operand specified.
>
> Source: XCU wait EXIT STATUS — utilities/wait.html#tag_20_147_14

> [spec:posix:req:builtin.wait.exit-status-signal]
> If the process terminated abnormally due to the receipt of a signal, the exit
> status shall be greater than 128 and shall be distinct from the exit status
> generated by other signals, but the exact value is unspecified. (See the kill
> **-l** option.)
>
> Source: XCU wait EXIT STATUS — utilities/wait.html#tag_20_147_14

> [spec:posix:req:builtin.wait.exit-status-values]
> Otherwise, the wait utility shall exit with one of the following values:
>
> | Exit status | Meaning |
> |---|---|
> | 0 | The wait utility was invoked with no operands and all process IDs known by the invoking shell have terminated. |
> | 1-126 | The wait utility detected an error. |
> | 127 | The process ID specified by the last pid operand specified is not known in the invoking shell execution environment. |
>
> Source: XCU wait EXIT STATUS — utilities/wait.html#tag_20_147_14
