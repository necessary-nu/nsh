# Special Built-In Utilities: set and trap

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

`[OB]`
: Obsolescent. The functionality may be removed in a future version; strictly conforming applications shall not use it.

## set — SYNOPSIS

> [spec:posix:syn:builtin.set.synopsis]
> The set special built-in has the following forms:
>
> `set [-abCefhmnuvx] [-o option] [argument...]`
>
> `set [+abCefhmnuvx] [+o option] [argument...]`
>
> `set -- [argument...]`
>
> `set -o`
>
> `set +o`
>
> Source: XCU set SYNOPSIS — utilities/V3_chap02.html#tag_19_26_02

## set — DESCRIPTION

> [spec:posix:req:builtin.set.no-operands-writes-variables]
> If no options or arguments are specified, set shall write the names and values
> of all shell variables in the collation sequence of the current locale. Each
> name shall start on a separate line, using the format:
>
> `"%s=%s\n", <name>, <value>`
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:req:builtin.set.variable-output-reinput]
> The value string shall be written with appropriate quoting; see the
> description of shell quoting in 2.2 Quoting. The output shall be suitable for
> reinput to the shell, setting or resetting, as far as possible, the variables
> that are currently set; read-only variables cannot be reset.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:sem:builtin.set.options-and-arguments]
> When options are specified, they shall set or unset attributes of the shell.
> When arguments are specified, they cause positional parameters to be set or
> unset. Setting or unsetting attributes and positional parameters are not
> necessarily related actions, but they can be combined in a single invocation
> of set.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:req:builtin.set.utility-syntax-guidelines]
> The set special built-in shall support XBD 12.2 Utility Syntax Guidelines
> except that options can be specified with either a leading <hyphen-minus>
> (meaning enable the option) or <plus-sign> (meaning disable it) unless
> otherwise specified.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:req:builtin.set.options-both-forms]
> Implementations shall support the options in the following list in both their
> <hyphen-minus> and <plus-sign> forms. These options can also be specified as
> options to sh.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

### set — single-letter options

> [spec:posix:req:builtin.set.opt-a-allexport]
> **-a**: Set the export attribute for all variable assignments. When this
> option is on, whenever a value is assigned to a variable in the current shell
> execution environment, the export attribute shall be set for the variable.
> This applies to all forms of assignment, including those made as a
> side-effect of variable expansions or arithmetic expansions, and those made
> as a result of the operation of the cd, getopts, or read utilities.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:sem:builtin.set.opt-a-separate-environments]
> As discussed in 2.9.1 Simple Commands, not all variable assignments happen in
> the current execution environment. When an assignment happens in a separate
> execution environment the export attribute is still set for the variable, but
> that does not affect the current execution environment.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:req:builtin.set.opt-b-notify]
> **-b**: This option shall be supported if the implementation supports the
> User Portability Utilities option. When job control and **-b** are both
> enabled, the shell shall write asynchronous notifications of background job
> completions (including termination by a signal), and may write asynchronous
> notifications of background job suspensions. See 2.11 Job Control for
> details. When job control is disabled, the **-b** option shall have no
> effect. Asynchronous notification shall not be enabled by default.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:req:builtin.set.opt-c-noclobber]
> **-C**: (Uppercase C.) Prevent existing regular files from being overwritten
> by the shell's `'>'` redirection operator (see 2.7.2 Redirecting Output); the
> `">|"` redirection operator shall override this noclobber option for an
> individual file.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:req:builtin.set.opt-e-errexit]
> **-e**: When this option is on, when any command fails (for any of the
> reasons listed in 2.8.1 Consequences of Shell Errors or by returning an exit
> status greater than zero), the shell immediately shall exit, as if by
> executing the exit special built-in utility with no arguments, with the
> following exceptions:
>
> 1. The failure of any individual command in a multi-command pipeline, or of
> any subshell environments in which command substitution was performed during
> word expansion, shall not cause the shell to exit. Only the failure of the
> pipeline itself shall be considered.
> 2. The **-e** setting shall be ignored when executing the compound list
> following the **while**, **until**, **if**, or **elif** reserved word, a
> pipeline beginning with the **!** reserved word, or any command of an AND-OR
> list other than the last.
> 3. If the exit status of a compound command other than a subshell command was
> the result of a failure while **-e** was being ignored, then **-e** shall not
> apply to this command.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:req:builtin.set.opt-e-per-environment]
> The **-e** requirement applies to the shell environment and each subshell
> environment separately. For example, in `set -e; (false; echo one) | cat;
> echo two` the false command causes the subshell to exit without executing
> `echo one`; however, `echo two` is executed because the exit status of the
> pipeline `(false; echo one) | cat` is zero.
>
> In `set -e; echo $(false; echo one) two` the false command causes the subshell
> in which the command substitution is performed to exit without executing
> `echo one`; the exit status of the subshell is ignored and the shell then
> executes the word-expanded command `echo two`.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:req:builtin.set.opt-f-noglob]
> **-f**: The shell shall disable pathname expansion.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:req:builtin.set.opt-h]
> **-h**: `[OB]` Setting this option may speed up PATH searches (see XBD 8.
> Environment Variables). This option may be enabled by default.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:req:builtin.set.opt-m-monitor]
> **-m**: This option shall be supported if the implementation supports the
> User Portability Utilities option. When this option is enabled, the shell
> shall perform job control actions as described in 2.11 Job Control. This
> option shall be enabled by default for interactive shells.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:req:builtin.set.opt-n-noexec]
> **-n**: The shell shall read commands but does not execute them; this can be
> used to check for shell script syntax errors. Interactive shells and
> subshells of interactive shells, recursively, may ignore this option.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:req:builtin.set.opt-u-nounset]
> **-u**: When the shell tries to expand, in a parameter expansion or an
> arithmetic expansion, an unset parameter other than the `'@'` and `'*'`
> special parameters, it shall write a message to standard error and the
> expansion shall fail with the consequences specified in 2.8.1 Consequences of
> Shell Errors.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:req:builtin.set.opt-v-verbose]
> **-v**: The shell shall write its input to standard error as it is read.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:req:builtin.set.opt-x-xtrace]
> **-x**: The shell shall write to standard error a trace for each command
> after it expands the command and before it executes it. It is unspecified
> whether the command that turns tracing off is traced.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

### set — reporting option settings

> [spec:posix:sem:builtin.set.opt-o-report]
> **-o** (without an option-argument): Write the current settings of the
> options to standard output in an unspecified format.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:sem:builtin.set.plus-o-report]
> **+o** (without an option-argument): Write the current option settings to
> standard output in a format that is suitable for reinput to the shell as
> commands that achieve the same options settings.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

### set — -o option long names

> [spec:posix:req:builtin.set.opt-o-option]
> **-o** *option*: Set various options, many of which shall be equivalent to
> the single option letters. The values of *option* listed below shall be
> supported.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:def:builtin.set.opt-o-allexport]
> **-o** *allexport*: Equivalent to **-a**.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:def:builtin.set.opt-o-errexit]
> **-o** *errexit*: Equivalent to **-e**.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:req:builtin.set.opt-o-ignoreeof]
> **-o** *ignoreeof*: Prevent an interactive shell from exiting on end-of-file.
> This setting prevents accidental logouts when <control>-D is entered. A user
> shall explicitly exit to leave the interactive shell. This option shall be
> supported if the system supports the User Portability Utilities option.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:req:builtin.set.opt-o-monitor]
> **-o** *monitor*: Equivalent to **-m**. This option shall be supported if the
> system supports the User Portability Utilities option.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:def:builtin.set.opt-o-noclobber]
> **-o** *noclobber*: Equivalent to **-C** (uppercase C).
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:def:builtin.set.opt-o-noglob]
> **-o** *noglob*: Equivalent to **-f**.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:def:builtin.set.opt-o-noexec]
> **-o** *noexec*: Equivalent to **-n**.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:req:builtin.set.opt-o-nolog]
> **-o** *nolog*: `[OB]` Prevent the entry of function definitions into the
> command history; see Command History List. This option may have no effect; it
> is kept for compatibility with previous versions of the standard. This option
> shall be supported if the system supports the User Portability Utilities
> option.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:def:builtin.set.opt-o-notify]
> **-o** *notify*: Equivalent to **-b**.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:def:builtin.set.opt-o-nounset]
> **-o** *nounset*: Equivalent to **-u**.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:sem:builtin.set.opt-o-pipefail]
> **-o** *pipefail*: Derive the exit status of a pipeline from the exit statuses
> of all of the commands in the pipeline, not just the last (rightmost)
> command, as described in 2.9.2 Pipelines.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:def:builtin.set.opt-o-verbose]
> **-o** *verbose*: Equivalent to **-v**.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:req:builtin.set.opt-o-vi]
> **-o** *vi*: Allow shell command line editing using the built-in vi editor.
> Enabling vi mode shall disable any other command line editing mode provided
> as an implementation extension. This option shall be supported if the system
> supports the User Portability Utilities option.
>
> It need not be possible to set vi mode on for certain block-mode terminals.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:def:builtin.set.opt-o-xtrace]
> **-o** *xtrace*: Equivalent to **-x**.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

### set — defaults and operands

> [spec:posix:req:builtin.set.options-default-off]
> The default for all these options shall be off (unset) unless stated
> otherwise in the description of the option or unless the shell was invoked
> with them on; see sh.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:req:builtin.set.positional-parameters]
> The remaining arguments shall be assigned in order to the positional
> parameters. The special parameter `'#'` shall be set to reflect the number of
> positional parameters. All positional parameters shall be unset before any
> new values are assigned.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:req:builtin.set.first-argument-hyphen]
> If the first argument is `'-'`, the results are unspecified.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

> [spec:posix:req:builtin.set.double-hyphen]
> The special argument `"--"` immediately following the set command name can be
> used to delimit the arguments if the first argument begins with `'+'` or
> `'-'`, or to prevent inadvertent listing of all shell variables when there
> are no arguments. The command set **--** without argument shall unset all
> positional parameters and set the special parameter `'#'` to zero.
>
> Source: XCU set DESCRIPTION — utilities/V3_chap02.html#tag_19_26_03

## set — input, output, and status

> [spec:posix:req:builtin.set.utility-defaults]
> Standard input is not used by set. set uses no input files, uses no
> environment variables, and creates no output files. Asynchronous events are
> handled as the defaults described in XCU 1.4 Utility Description Defaults, as
> are the consequences of errors. There is no extended description.
>
> Source: XCU set — utilities/V3_chap02.html#tag_19_26

> [spec:posix:req:builtin.set.stderr-diagnostics-only]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU set STDERR — utilities/V3_chap02.html#tag_19_26_11

> [spec:posix:req:builtin.set.exit-status]
> set shall exit with the following status:
>
> | Status | Meaning |
> |---|---|
> | 0 | Successful completion. |
> | >0 | An invalid option was specified, or an error occurred. |
>
> Source: XCU set EXIT STATUS — utilities/V3_chap02.html#tag_19_26_14

## trap — SYNOPSIS

> [spec:posix:syn:builtin.trap.synopsis]
> The trap special built-in has the following forms:
>
> `trap n [condition...]`
>
> `trap -p [condition...]`
>
> `trap [action condition...]`
>
> Source: XCU trap SYNOPSIS — utilities/V3_chap02.html#tag_19_29_02

## trap — DESCRIPTION

> [spec:posix:req:builtin.trap.operand-interpretation]
> If the **-p** option is not specified and the first operand is an unsigned
> decimal integer, the shell shall treat all operands as conditions, and shall
> reset each condition to the default value. Otherwise, if the **-p** option is
> not specified and there are operands, the first operand shall be treated as
> an action and the remaining as conditions.
>
> Source: XCU trap DESCRIPTION — utilities/V3_chap02.html#tag_19_29_03

> [spec:posix:req:builtin.trap.action-values]
> If action is `'-'`, the shell shall reset each condition to the default
> value. If action is null (`""`), the shell shall ignore each specified
> condition if it arises. Otherwise, the argument action shall be read and
> executed by the shell when one of the corresponding conditions arises.
>
> Source: XCU trap DESCRIPTION — utilities/V3_chap02.html#tag_19_29_03

> [spec:posix:req:builtin.trap.action-overrides-and-exit-status]
> The action of trap shall override a previous action (either default action or
> one explicitly set). The value of `"$?"` after the trap action completes shall
> be the value it had before the trap action was executed.
>
> Source: XCU trap DESCRIPTION — utilities/V3_chap02.html#tag_19_29_03

> [spec:posix:def:builtin.trap.condition]
> The condition can be EXIT, 0 (equivalent to EXIT), or a signal specified
> using a symbolic name, without the SIG prefix, as listed in the tables of
> signal names in the `<signal.h>` header defined in XBD 14. Headers; for
> example, HUP, INT, QUIT, TERM.
>
> Source: XCU trap DESCRIPTION — utilities/V3_chap02.html#tag_19_29_03

> [spec:posix:req:builtin.trap.signal-name-extensions]
> Implementations may permit names with the SIG prefix or ignore case in signal
> names as an extension.
>
> Source: XCU trap DESCRIPTION — utilities/V3_chap02.html#tag_19_29_03

> [spec:posix:req:builtin.trap.kill-stop-undefined]
> Setting a trap for SIGKILL or SIGSTOP produces undefined results.
>
> Source: XCU trap DESCRIPTION — utilities/V3_chap02.html#tag_19_29_03

> [spec:posix:req:builtin.trap.exit-condition]
> The EXIT condition shall occur when the shell terminates normally (exits),
> and may occur when the shell terminates abnormally as a result of delivery of
> a signal (other than SIGKILL) whose trap action is the default.
>
> Source: XCU trap DESCRIPTION — utilities/V3_chap02.html#tag_19_29_03

> [spec:posix:req:builtin.trap.exit-action-environment]
> The environment in which the shell executes a trap action on EXIT shall be
> identical to the environment immediately after the last command executed
> before the trap action on EXIT was executed.
>
> Source: XCU trap DESCRIPTION — utilities/V3_chap02.html#tag_19_29_03

> [spec:posix:req:builtin.trap.action-executed-as-eval]
> If action is neither `'-'` nor the empty string, then each time a matching
> condition arises, the action shall be executed in a manner equivalent to
> `eval action`.
>
> Source: XCU trap DESCRIPTION — utilities/V3_chap02.html#tag_19_29_03

> [spec:posix:req:builtin.trap.signals-ignored-on-entry]
> Signals that were ignored on entry to a non-interactive shell cannot be
> trapped or reset, although no error need be reported when attempting to do
> so. An interactive shell may reset or catch signals ignored on entry.
>
> Source: XCU trap DESCRIPTION — utilities/V3_chap02.html#tag_19_29_03

> [spec:posix:req:builtin.trap.persistence]
> Traps shall remain in place for a given shell until explicitly changed with
> another trap command.
>
> Source: XCU trap DESCRIPTION — utilities/V3_chap02.html#tag_19_29_03

> [spec:posix:req:builtin.trap.subshell-reset]
> When a subshell is entered, traps that are not being ignored shall be set to
> the default actions, except in the case of a command substitution containing
> only a single trap command, when the traps need not be altered.
>
> Source: XCU trap DESCRIPTION — utilities/V3_chap02.html#tag_19_29_03

> [spec:posix:req:builtin.trap.subshell-lexical-check]
> Implementations may check for the single-trap-command command substitution
> case using only lexical analysis; for example, if `` `trap` `` and
> `$( trap -- )` do not alter the traps in the subshell, cases such as
> assigning `var=trap` and then using `$($var)` may still alter them. This does
> not imply that the trap command cannot be used within the subshell to set new
> traps.
>
> Source: XCU trap DESCRIPTION — utilities/V3_chap02.html#tag_19_29_03

> [spec:posix:req:builtin.trap.list-condition-set]
> The trap command with no operands shall write to standard output a list of
> commands associated with each of a set of conditions; if the **-p** option is
> not specified, this set shall contain only the conditions that are not in the
> default state (including signals that were ignored on entry to a
> non-interactive shell); if the **-p** option is specified, the set shall
> contain all conditions, except that it is unspecified whether conditions
> corresponding to the SIGKILL and SIGSTOP signals are included in the set.
>
> Source: XCU trap DESCRIPTION — utilities/V3_chap02.html#tag_19_29_03

> [spec:posix:req:builtin.trap.list-in-subshell]
> If the command is executed in a subshell, the implementation does not perform
> the optional lexical check described for a command substitution containing
> only a single trap command, and no trap commands with operands have been
> executed since entry to the subshell, the list shall contain the commands
> that were associated with each condition immediately before the subshell
> environment was entered. Otherwise, the list shall contain the commands
> currently associated with each condition.
>
> Source: XCU trap DESCRIPTION — utilities/V3_chap02.html#tag_19_29_03

> [spec:posix:syn:builtin.trap.list-format]
> The format of the list written by trap shall be:
>
> `"trap -- %s %s ...\n", <action>, <condition> ...`
>
> Source: XCU trap DESCRIPTION — utilities/V3_chap02.html#tag_19_29_03

> [spec:posix:req:builtin.trap.list-suitable-for-reinput]
> The shell shall format the output, including the proper use of quoting, so
> that it is suitable for reinput to the shell as commands that achieve the
> same trapping results for the set of conditions included in the output,
> except for signals that were ignored on entry to the shell. If this set
> includes conditions corresponding to the SIGKILL and SIGSTOP signals, the
> shell shall accept them when the output is reinput to the shell (where
> accepting them means they do not cause a non-zero exit status, a diagnostic
> message, or undefined behavior).
>
> Source: XCU trap DESCRIPTION — utilities/V3_chap02.html#tag_19_29_03

> [spec:posix:req:builtin.trap.xsi-signal-numbers]
> `[XSI]` XSI-conformant systems also allow numeric signal numbers for the
> conditions corresponding to the following signal names:
>
> | Number | Signal |
> |---|---|
> | 1 | SIGHUP |
> | 2 | SIGINT |
> | 3 | SIGQUIT |
> | 6 | SIGABRT |
> | 9 | SIGKILL |
> | 14 | SIGALRM |
> | 15 | SIGTERM |
>
> Source: XCU trap DESCRIPTION — utilities/V3_chap02.html#tag_19_29_03

> [spec:posix:req:builtin.trap.invalid-condition-warning]
> If an invalid signal name `[XSI]` `[Option Start]` or number `[Option End]`
> is specified, the trap utility shall write a warning message to standard
> error.
>
> Source: XCU trap DESCRIPTION — utilities/V3_chap02.html#tag_19_29_03

> [spec:posix:req:builtin.trap.utility-syntax-guidelines]
> The trap special built-in shall conform to XBD 12.2 Utility Syntax
> Guidelines.
>
> Source: XCU trap DESCRIPTION — utilities/V3_chap02.html#tag_19_29_03

## trap — OPTIONS

> [spec:posix:req:builtin.trap.opt-p]
> The following option shall be supported. **-p**: Write to standard output a
> list of commands associated with each condition operand. The behavior when
> there are no operands is specified in the DESCRIPTION section.
>
> Source: XCU trap OPTIONS — utilities/V3_chap02.html#tag_19_29_04

> [spec:posix:req:builtin.trap.opt-p-suitable-for-reinput]
> The shell shall format the output, including the proper use of quoting, so
> that it is suitable for reinput to the shell as commands that achieve the
> same trapping results for the specified set of conditions. If a condition
> operand is a condition corresponding to the SIGKILL or SIGSTOP signal, and
> trap **-p** without any operands would not include it in the set of
> conditions for which it writes output, the behavior is undefined if the
> output is reinput to the shell.
>
> Source: XCU trap OPTIONS — utilities/V3_chap02.html#tag_19_29_04

## trap — input, output, and status

> [spec:posix:req:builtin.trap.utility-defaults]
> Standard input is not used by trap. trap uses no input files, uses no
> environment variables, and creates no output files. Asynchronous events are
> handled as the defaults described in XCU 1.4 Utility Description Defaults, as
> are the consequences of errors. There is no extended description.
>
> Source: XCU trap — utilities/V3_chap02.html#tag_19_29

> [spec:posix:req:builtin.trap.stderr-usage]
> The standard error shall be used only for diagnostic messages and warning
> messages about invalid signal names `[XSI]` `[Option Start]` or numbers.
> `[Option End]`
>
> Source: XCU trap STDERR — utilities/V3_chap02.html#tag_19_29_11

> [spec:posix:req:builtin.trap.exit-status]
> If the trap name `[XSI]` `[Option Start]` or number `[Option End]` is
> invalid, a non-zero exit status shall be returned; otherwise, zero shall be
> returned. For both interactive and non-interactive shells, invalid signal
> names `[XSI]` `[Option Start]` or numbers `[Option End]` shall not be
> considered an error and shall not cause the shell to abort.
>
> Source: XCU trap EXIT STATUS — utilities/V3_chap02.html#tag_19_29_14
