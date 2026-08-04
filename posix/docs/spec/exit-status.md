# Exit Status and Errors

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
"SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in RFC 2119. This document
reproduces normative text from IEEE Std 1003.1-2024 (POSIX.1-2024),
Copyright © 2001-2024 The IEEE and The Open Group.

## 2.8.1 Consequences of Shell Errors

> [spec:posix:req:exit.shell-error-consequences]
> Certain errors shall cause the shell to write a diagnostic message to standard
> error and exit as shown in the following table:
>
> | Error | Non-Interactive Shell | Interactive Shell | Shell Diagnostic Message Required |
> |---|---|---|---|
> | Shell language syntax error | shall exit | shall not exit | yes |
> | Special built-in utility error | shall exit<sup>1</sup> | shall not exit | no<sup>2</sup> |
> | Other utility (not a special built-in) error | shall not exit | shall not exit | no<sup>3</sup> |
> | Redirection error with special built-in utilities | shall exit | shall not exit | yes |
> | Redirection error with compound commands | shall not exit | shall not exit | yes |
> | Redirection error with function execution | shall not exit | shall not exit | yes |
> | Redirection error with other utilities (not special built-ins) | shall not exit | shall not exit | yes |
> | Variable assignment error | shall exit | shall not exit | yes |
> | Expansion error | shall exit | shall not exit | yes |
> | Command not found | may exit | shall not exit | yes |
> | Unrecoverable read error when reading commands | shall exit<sup>4</sup> | shall exit<sup>4</sup> | yes |
>
> Notes:
>
> 1. The shell shall exit only if the special built-in utility is executed
> directly. If it is executed via the `command` utility, the shell shall not
> exit.
> 2. Although special built-ins are part of the shell, a diagnostic message
> written by a special built-in is not considered to be a shell diagnostic
> message, and can be redirected like any other utility.
> 3. The shell is not required to write a diagnostic message, but the utility
> itself shall write a diagnostic message if required to do so.
> 4. If an unrecoverable read error occurs when reading commands, other than
> from the file operand of the `dot` special built-in, the shell shall execute
> no further commands (including any already successfully read but not yet
> executed) other than any specified in a previously defined EXIT `trap` action.
> An unrecoverable read error while reading from the file operand of the `dot`
> special built-in shall be treated as a special built-in utility error.
>
> Source: XCU 2.8.1 Consequences of Shell Errors — utilities/V3_chap02.html#tag_19_08_01

> [spec:posix:req:exit.unrecoverable-read-error]
> If an unrecoverable read error occurs when reading commands, other than from
> the file operand of the `dot` special built-in, the shell shall execute no
> further commands (including any already successfully read but not yet
> executed) other than any specified in a previously defined EXIT `trap` action.
>
> An unrecoverable read error while reading from the file operand of the `dot`
> special built-in shall be treated as a special built-in utility error.
>
> Source: XCU 2.8.1 Consequences of Shell Errors — utilities/V3_chap02.html#tag_19_08_01

> [spec:posix:def:exit.expansion-error]
> An expansion error is one that occurs when the shell expansions defined in
> 2.6 Word Expansions are carried out (for example, `"${x!y}"`, because `'!'` is
> not a valid operator); an implementation may treat these as syntax errors if
> it is able to detect them during tokenization, rather than during expansion.
>
> Source: XCU 2.8.1 Consequences of Shell Errors — utilities/V3_chap02.html#tag_19_08_01

> [spec:posix:req:exit.subshell-error-exit]
> If any of the errors shown as "shall exit" or "may exit" occur in a subshell
> environment, the shell shall (respectively, may) exit from the subshell
> environment with a non-zero status and continue in the environment from which
> that subshell environment was invoked.
>
> Source: XCU 2.8.1 Consequences of Shell Errors — utilities/V3_chap02.html#tag_19_08_01

> [spec:posix:req:exit.interactive-abandons-command]
> In all of the cases shown in the table where an interactive shell is required
> not to exit and a non-interactive shell is required to exit, an interactive
> shell shall not perform any further processing of the command in which the
> error occurred.
>
> Source: XCU 2.8.1 Consequences of Shell Errors — utilities/V3_chap02.html#tag_19_08_01

## 2.8.2 Exit Status for Commands

> [spec:posix:def:exit.command-status]
> Each command has an exit status that can influence the behavior of other shell
> commands. The exit status of commands that are not utilities is documented in
> this section. The exit status of the standard utilities is documented in their
> respective sections.
>
> Source: XCU 2.8.2 Exit Status for Commands — utilities/V3_chap02.html#tag_19_08_02

> [spec:posix:req:exit.status-command-not-found]
> The exit status of a command shall be determined as follows: if the command is
> not found, the exit status shall be 127.
>
> Source: XCU 2.8.2 Exit Status for Commands — utilities/V3_chap02.html#tag_19_08_02

> [spec:posix:req:exit.status-not-executable]
> Otherwise, if the command name is found, but it is not an executable utility,
> the exit status shall be 126.
>
> Source: XCU 2.8.2 Exit Status for Commands — utilities/V3_chap02.html#tag_19_08_02

> [spec:posix:req:exit.status-signal-terminated]
> Otherwise, if the command terminated due to the receipt of a signal, the shell
> shall assign it an exit status greater than 128. The exit status shall
> identify, in an implementation-defined manner, which signal terminated the
> command. Note that shell implementations are permitted to assign an exit
> status greater than 255 if a command terminates due to a signal.
>
> Source: XCU 2.8.2 Exit Status for Commands — utilities/V3_chap02.html#tag_19_08_02

> [spec:posix:req:exit.status-normal-termination]
> Otherwise, the exit status shall be the value obtained by the equivalent of
> the WEXITSTATUS macro applied to the status obtained by the `wait()` function
> (as defined in the System Interfaces volume of POSIX.1-2024). Note that for C
> programs, this value is equal to the result of performing a modulo 256
> operation on the value passed to `_Exit()`, `_exit()`, or `exit()` or returned
> from `main()`.
>
> Source: XCU 2.8.2 Exit Status for Commands — utilities/V3_chap02.html#tag_19_08_02
