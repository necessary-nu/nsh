# Special Built-In Utilities: Control Flow

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
"SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in RFC 2119. This document
reproduces normative text from IEEE Std 1003.1-2024 (POSIX.1-2024),
Copyright © 2001-2024 The IEEE and The Open Group.

Option-conditional text carries the standard's own margin code inline, at the point the standard shades it:

`[UP]`
: User Portability Utilities. The functionality described is optional.

## 2.15 Special Built-In Utilities

> [spec:posix:req:builtin.special.supported-and-output]
> The following "special built-in" utilities shall be supported in the shell
> command language. The output of each command, if any, shall be written to
> standard output, subject to the normal redirection and piping possible with
> all commands.
>
> Source: XCU 2.15 Special Built-In Utilities — utilities/V3_chap02.html#tag_19_15

> [spec:posix:def:builtin.special.term-built-in]
> The term "built-in" implies that there is no need to execute a separate
> executable file because the utility is implemented in the shell itself. An
> implementation may choose to make any utility a built-in; however, the special
> built-in utilities described here differ from regular built-in utilities in
> two respects: error handling, and the effect of preceding variable
> assignments.
>
> Source: XCU 2.15 Special Built-In Utilities — utilities/V3_chap02.html#tag_19_15

> [spec:posix:req:builtin.special.error-may-abort-shell]
> An error in a special built-in utility may cause a shell executing that
> utility to abort, while an error in a regular built-in utility shall not cause
> a shell executing that utility to abort. (See 2.8.1 Consequences of Shell
> Errors for the consequences of errors on interactive and non-interactive
> shells.) If a special built-in utility encountering an error does not abort
> the shell, its exit value shall be non-zero.
>
> Source: XCU 2.15 Special Built-In Utilities — utilities/V3_chap02.html#tag_19_15

> [spec:posix:req:builtin.special.preceding-assignments-persist]
> As described in 2.9.1 Simple Commands, variable assignments preceding the
> invocation of a special built-in utility affect the current execution
> environment; this shall not be the case with a regular built-in or other
> utility.
>
> Source: XCU 2.15 Special Built-In Utilities — utilities/V3_chap02.html#tag_19_15

> [spec:posix:req:builtin.special.not-exec-accessible]
> The special built-in utilities in this section need not be provided in a
> manner accessible via the exec family of functions defined in the System
> Interfaces volume of POSIX.1-2024.
>
> Source: XCU 2.15 Special Built-In Utilities — utilities/V3_chap02.html#tag_19_15

> [spec:posix:req:builtin.special.utility-syntax-guidelines]
> Some of the special built-ins are described as conforming to XBD 12.2 Utility
> Syntax Guidelines. For those that are not, the requirement in 1.4 Utility
> Description Defaults that `"--"` be recognized as a first argument to be
> discarded does not apply and a conforming application shall not use that
> argument.
>
> Source: XCU 2.15 Special Built-In Utilities — utilities/V3_chap02.html#tag_19_15

## break

> [spec:posix:syn:builtin.break.syn]
> The synopsis of the break utility is `break [n]`.
>
> Source: XCU break SYNOPSIS — utilities/V3_chap02.html#tag_19_16_02

> [spec:posix:req:builtin.break.exit-nth-loop]
> If n is specified, the break utility shall exit from the nth enclosing for,
> while, or until loop. If n is not specified, break shall behave as if n was
> specified as 1. Execution shall continue with the command immediately
> following the exited loop. The application shall ensure that the value of n is
> a positive decimal integer. If n is greater than the number of enclosing
> loops, the outermost enclosing loop shall be exited. If there is no enclosing
> loop, the behavior is unspecified.
>
> Source: XCU break DESCRIPTION — utilities/V3_chap02.html#tag_19_16_03

> [spec:posix:def:builtin.break.lexically-enclosing]
> A loop shall enclose a break or continue command if the loop lexically
> encloses the command. A loop lexically encloses a break or continue command if
> the command is:
>
> - Executing in the same execution environment (see 2.13 Shell Execution
>   Environment) as the compound-list of the loop's do-group (see 2.10.2 Shell
>   Grammar Rules), and
> - Contained in a compound-list associated with the loop (either in the
>   compound-list of the loop's do-group or, if the loop is a while or until
>   loop, in the compound-list following the while or until reserved word), and
> - Not in the body of a function whose function definition command (see 2.9.5
>   Function Definition Command) is contained in a compound-list associated with
>   the loop.
>
> Source: XCU break DESCRIPTION — utilities/V3_chap02.html#tag_19_16_03

> [spec:posix:sem:builtin.break.non-lexical-loop-unspecified]
> If n is greater than the number of lexically enclosing loops and there is a
> non-lexically enclosing loop in progress in the same execution environment as
> the break or continue command, it is unspecified whether that loop encloses
> the command.
>
> Source: XCU break DESCRIPTION — utilities/V3_chap02.html#tag_19_16_03

> [spec:posix:req:builtin.break.stderr]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU break STDERR — utilities/V3_chap02.html#tag_19_16_11

> [spec:posix:req:builtin.break.interfaces]
> The break utility has no options and no operands beyond those described in the
> DESCRIPTION. Standard input is not used; there are no input files; no
> environment variables affect its execution; asynchronous events are handled as
> for the utility description defaults; standard output is not used; there are
> no output files; there is no extended description; and the consequences of
> errors are as for the utility description defaults.
>
> Source: XCU break — utilities/V3_chap02.html#tag_19_16

> [spec:posix:req:builtin.break.exit-status]
> The following exit values shall be returned:
>
> | Exit status | Meaning |
> |---|---|
> | 0 | Successful completion. |
> | >0 | The n value was not an unsigned decimal integer greater than or equal to 1. |
>
> Source: XCU break EXIT STATUS — utilities/V3_chap02.html#tag_19_16_14

## colon (`:`)

> [spec:posix:syn:builtin.colon.syn]
> The synopsis of the null utility is `: [argument...]`.
>
> Source: XCU colon SYNOPSIS — utilities/V3_chap02.html#tag_19_17_02

> [spec:posix:req:builtin.colon.null-utility]
> This utility shall do nothing except return a 0 exit status. It is used when a
> command is needed, as in the then condition of an if command, but nothing is
> to be done by the command.
>
> Source: XCU colon DESCRIPTION — utilities/V3_chap02.html#tag_19_17_03

> [spec:posix:req:builtin.colon.no-options]
> This utility shall not recognize the `"--"` argument in the manner specified
> by Guideline 10 of XBD 12.2 Utility Syntax Guidelines.
>
> Implementations shall not support any options.
>
> Source: XCU colon OPTIONS — utilities/V3_chap02.html#tag_19_17_04

> [spec:posix:req:builtin.colon.interfaces]
> The null utility has no operands beyond those described in the DESCRIPTION.
> Standard input is not used; there are no input files; no environment variables
> affect its execution; asynchronous events are handled as for the utility
> description defaults; standard output is not used; standard error is not used;
> there are no output files; there is no extended description; and there are no
> consequences of errors.
>
> Source: XCU colon — utilities/V3_chap02.html#tag_19_17

> [spec:posix:req:builtin.colon.exit-status]
> The exit status shall be zero.
>
> Source: XCU colon EXIT STATUS — utilities/V3_chap02.html#tag_19_17_14

## continue

> [spec:posix:syn:builtin.continue.syn]
> The synopsis of the continue utility is `continue [n]`.
>
> Source: XCU continue SYNOPSIS — utilities/V3_chap02.html#tag_19_18_02

> [spec:posix:req:builtin.continue.return-to-top]
> If n is specified, the continue utility shall return to the top of the nth
> enclosing for, while, or until loop. If n is not specified, continue shall
> behave as if n was specified as 1. Returning to the top of the loop involves
> repeating the condition list of a while or until loop or performing the next
> assignment of a for loop, and re-executing the loop if appropriate.
>
> Source: XCU continue DESCRIPTION — utilities/V3_chap02.html#tag_19_18_03

> [spec:posix:req:builtin.continue.n-operand]
> The application shall ensure that the value of n is a positive decimal
> integer. If n is greater than the number of enclosing loops, the outermost
> enclosing loop shall be used. If there is no enclosing loop, the behavior is
> unspecified.
>
> The meaning of "enclosing" shall be as specified in the description of the
> break utility.
>
> Source: XCU continue DESCRIPTION — utilities/V3_chap02.html#tag_19_18_03

> [spec:posix:req:builtin.continue.stderr]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU continue STDERR — utilities/V3_chap02.html#tag_19_18_11

> [spec:posix:req:builtin.continue.interfaces]
> The continue utility has no options and no operands beyond those described in
> the DESCRIPTION. Standard input is not used; there are no input files; no
> environment variables affect its execution; asynchronous events are handled as
> for the utility description defaults; standard output is not used; there are
> no output files; there is no extended description; and the consequences of
> errors are as for the utility description defaults.
>
> Source: XCU continue — utilities/V3_chap02.html#tag_19_18

> [spec:posix:req:builtin.continue.exit-status]
> The following exit values shall be returned:
>
> | Exit status | Meaning |
> |---|---|
> | 0 | Successful completion. |
> | >0 | The n value was not an unsigned decimal integer greater than or equal to 1. |
>
> Source: XCU continue EXIT STATUS — utilities/V3_chap02.html#tag_19_18_14

## dot (`.`)

> [spec:posix:syn:builtin.dot.syn]
> The synopsis of the dot utility is `. file`.
>
> Source: XCU dot SYNOPSIS — utilities/V3_chap02.html#tag_19_19_02

> [spec:posix:req:builtin.dot.execute-in-current-environment]
> The shell shall tokenize (see 2.3 Token Recognition) the contents of the file,
> parse the tokens (see 2.10 Shell Grammar), and execute the resulting commands
> in the current environment. It is unspecified whether the commands are parsed
> and executed as a program (as for a shell script) or are parsed as a single
> compound_list that is executed after the entire file has been parsed.
>
> Source: XCU dot DESCRIPTION — utilities/V3_chap02.html#tag_19_19_03

> [spec:posix:req:builtin.dot.path-search]
> If file does not contain a <slash>, the shell shall use the search path
> specified by PATH to find the directory containing file. Unlike normal command
> search, however, the file searched for by the dot utility need not be
> executable. If no readable file is found, a non-interactive shell shall abort;
> an interactive shell shall write a diagnostic message to standard error.
>
> Source: XCU dot DESCRIPTION — utilities/V3_chap02.html#tag_19_19_03

> [spec:posix:req:builtin.dot.utility-syntax-guidelines]
> The dot special built-in shall support XBD 12.2 Utility Syntax Guidelines,
> except for Guidelines 1 and 2.
>
> Source: XCU dot DESCRIPTION — utilities/V3_chap02.html#tag_19_19_03

> [spec:posix:req:builtin.dot.stderr]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU dot STDERR — utilities/V3_chap02.html#tag_19_19_11

> [spec:posix:req:builtin.dot.interfaces]
> The dot utility has no options, and its operands, input files, and the
> environment variables affecting its execution are as described in the
> DESCRIPTION. Standard input is not used; asynchronous events are handled as
> for the utility description defaults; standard output is not used; there are
> no output files; there is no extended description; and the consequences of
> errors are as for the utility description defaults.
>
> Source: XCU dot — utilities/V3_chap02.html#tag_19_19

> [spec:posix:req:builtin.dot.exit-status]
> If no readable file was found or if the commands in the file could not be
> parsed, and the shell is interactive (and therefore does not abort; see 2.8.1
> Consequences of Shell Errors), the exit status shall be non-zero. Otherwise,
> return the value of the last command executed, or a zero exit status if no
> command is executed.
>
> Source: XCU dot EXIT STATUS — utilities/V3_chap02.html#tag_19_19_14

## eval

> [spec:posix:syn:builtin.eval.syn]
> The synopsis of the eval utility is `eval [argument...]`.
>
> Source: XCU eval SYNOPSIS — utilities/V3_chap02.html#tag_19_20_02

> [spec:posix:req:builtin.eval.construct-and-execute]
> The eval utility shall construct a command string by concatenating arguments
> together, separating each with a <space> character. The constructed command
> string shall be tokenized (see 2.3 Token Recognition), parsed (see 2.10 Shell
> Grammar), and executed by the shell in the current environment. It is
> unspecified whether the commands are parsed and executed as a program (as for
> a shell script) or are parsed as a single compound_list that is executed after
> the entire constructed command string has been parsed.
>
> Source: XCU eval DESCRIPTION — utilities/V3_chap02.html#tag_19_20_03

> [spec:posix:req:builtin.eval.stderr]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU eval STDERR — utilities/V3_chap02.html#tag_19_20_11

> [spec:posix:req:builtin.eval.interfaces]
> The eval utility has no options and no operands beyond those described in the
> DESCRIPTION. Standard input is not used; there are no input files; no
> environment variables affect its execution; asynchronous events are handled as
> for the utility description defaults; standard output is not used; there are
> no output files; there is no extended description; and the consequences of
> errors are as for the utility description defaults.
>
> Source: XCU eval — utilities/V3_chap02.html#tag_19_20

> [spec:posix:req:builtin.eval.exit-status]
> If there are no arguments, or only null arguments, eval shall return a zero
> exit status; otherwise, it shall return the exit status of the command defined
> by the string of concatenated arguments separated by <space> characters, or a
> non-zero exit status if the concatenation could not be parsed as a command and
> the shell is interactive (and therefore did not abort).
>
> Source: XCU eval EXIT STATUS — utilities/V3_chap02.html#tag_19_20_14

## exec

> [spec:posix:syn:builtin.exec.syn]
> The synopsis of the exec utility is `exec [utility [argument...]]`.
>
> Source: XCU exec SYNOPSIS — utilities/V3_chap02.html#tag_19_21_02

> [spec:posix:req:builtin.exec.no-operands-redirections]
> If exec is specified with no operands, any redirections associated with the
> exec command shall be made in the current shell execution environment. If any
> file descriptors with numbers greater than 2 are opened by those redirections,
> it is unspecified whether those file descriptors remain open when the shell
> invokes another utility. Scripts concerned that child shells could misuse open
> file descriptors can always close them explicitly. If the result of the
> redirections would be that file descriptor 0, 1, or 2 is closed,
> implementations may open the file descriptor to an unspecified file.
>
> Source: XCU exec DESCRIPTION — utilities/V3_chap02.html#tag_19_21_03

> [spec:posix:req:builtin.exec.utility-operand]
> If exec is specified with a utility operand, the shell shall execute a
> non-built-in utility as described in 2.9.1.6 Non-built-in Utility Execution
> with utility as the command name and the argument operands (if any) as the
> command arguments.
>
> Source: XCU exec DESCRIPTION — utilities/V3_chap02.html#tag_19_21_03

> [spec:posix:req:builtin.exec.failure-non-interactive-exits]
> If the exec command fails, a non-interactive shell shall exit from the current
> shell execution environment.
>
> Source: XCU exec DESCRIPTION — utilities/V3_chap02.html#tag_19_21_03

> [spec:posix:req:builtin.exec.failure-interactive-up]
> `[UP]` When the exec command fails, an interactive shell may exit from a
> subshell environment but shall not exit if the current shell environment is
> not a subshell environment.
>
> If the exec command fails and the shell does not exit, any redirections
> associated with the exec command that were successfully made shall take effect
> in the current shell execution environment.
>
> Source: XCU exec DESCRIPTION — utilities/V3_chap02.html#tag_19_21_03

> [spec:posix:req:builtin.exec.utility-syntax-guidelines]
> The exec special built-in shall support XBD 12.2 Utility Syntax Guidelines.
>
> Source: XCU exec DESCRIPTION — utilities/V3_chap02.html#tag_19_21_03

> [spec:posix:req:builtin.exec.env-path]
> The following environment variable shall affect the execution of exec:
>
> PATH — Determine the search path when looking for the utility given as the
> utility operand; see XBD 8.3 Other Environment Variables.
>
> Source: XCU exec ENVIRONMENT VARIABLES — utilities/V3_chap02.html#tag_19_21_08

> [spec:posix:req:builtin.exec.stderr]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU exec STDERR — utilities/V3_chap02.html#tag_19_21_11

> [spec:posix:req:builtin.exec.interfaces]
> The exec utility has no options and no operands beyond those described in the
> DESCRIPTION. Standard input is not used; there are no input files;
> asynchronous events are handled as for the utility description defaults;
> standard output is not used; there are no output files; there is no extended
> description; and the consequences of errors are as for the utility description
> defaults.
>
> Source: XCU exec — utilities/V3_chap02.html#tag_19_21

> [spec:posix:req:builtin.exec.exit-status]
> If utility is specified and is executed, exec shall not return to the shell;
> rather, the exit status of the current shell execution environment shall be
> the exit status of utility. If utility is specified and an attempt to execute
> it as a non-built-in utility fails, the exit status shall be as described in
> 2.9.1.6 Non-built-in Utility Execution. If a redirection error occurs (see
> 2.8.1 Consequences of Shell Errors), the exit status shall be a value in the
> range 1-125. Otherwise, exec shall return a zero exit status.
>
> Source: XCU exec EXIT STATUS — utilities/V3_chap02.html#tag_19_21_14

## exit

> [spec:posix:syn:builtin.exit.syn]
> The synopsis of the exit utility is `exit [n]`.
>
> Source: XCU exit SYNOPSIS — utilities/V3_chap02.html#tag_19_22_02

> [spec:posix:req:builtin.exit.cause-shell-exit]
> The exit utility shall cause the shell to exit from its current execution
> environment. If the current execution environment is a subshell environment,
> the shell shall exit from the subshell environment and continue in the
> environment from which that subshell environment was invoked; otherwise, the
> shell utility shall terminate. The wait status of the shell or subshell shall
> be determined by the unsigned decimal integer n, if specified.
>
> Source: XCU exit DESCRIPTION — utilities/V3_chap02.html#tag_19_22_03

> [spec:posix:req:builtin.exit.wait-status-from-n]
> If n is specified and has a value between 0 and 255 inclusive, the wait status
> of the shell or subshell shall indicate that it exited with exit status n. If
> n is specified and has a value greater than 256 that corresponds to an exit
> status the shell assigns to commands terminated by a valid signal (see 2.8.2
> Exit Status for Commands), the wait status of the shell or subshell shall
> indicate that it was terminated by that signal. No other actions associated
> with the signal, such as execution of trap actions or creation of a core
> image, shall be performed by the shell.
>
> Source: XCU exit DESCRIPTION — utilities/V3_chap02.html#tag_19_22_03

> [spec:posix:sem:builtin.exit.invalid-n-unspecified]
> If n is specified and is not an unsigned decimal integer, or has a value of
> 256, or has a value greater than 256 but not corresponding to an exit status
> the shell assigns to commands terminated by a valid signal, the wait status of
> the shell or subshell is unspecified.
>
> Source: XCU exit DESCRIPTION — utilities/V3_chap02.html#tag_19_22_03

> [spec:posix:req:builtin.exit.default-n]
> If n is not specified, the result shall be as if n were specified with the
> current value of the special parameter `'?'` (see 2.5.2 Special Parameters),
> except that if the exit command would cause the end of execution of a trap
> action, the value for the special parameter `'?'` that is considered "current"
> shall be the value it had immediately preceding the trap action.
>
> Source: XCU exit DESCRIPTION — utilities/V3_chap02.html#tag_19_22_03

> [spec:posix:req:builtin.exit.exit-trap]
> A trap action on EXIT shall be executed before the shell terminates, except
> when the exit utility is invoked in that trap action itself, in which case the
> shell shall exit immediately. It is unspecified whether setting a new trap
> action on EXIT during execution of a trap action on EXIT will cause the new
> trap action to be executed before the shell terminates.
>
> Source: XCU exit DESCRIPTION — utilities/V3_chap02.html#tag_19_22_03

> [spec:posix:req:builtin.exit.stderr]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU exit STDERR — utilities/V3_chap02.html#tag_19_22_11

> [spec:posix:req:builtin.exit.interfaces]
> The exit utility has no options and no operands beyond those described in the
> DESCRIPTION. Standard input is not used; there are no input files; no
> environment variables affect its execution; asynchronous events are handled as
> for the utility description defaults; standard output is not used; there are
> no output files; there is no extended description; and the consequences of
> errors are as for the utility description defaults.
>
> Source: XCU exit — utilities/V3_chap02.html#tag_19_22

> [spec:posix:sem:builtin.exit.exit-status]
> The exit utility causes the shell to exit from its current execution
> environment, and therefore does not itself return an exit status.
>
> Source: XCU exit EXIT STATUS — utilities/V3_chap02.html#tag_19_22_14
