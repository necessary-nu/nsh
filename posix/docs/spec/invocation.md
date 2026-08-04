# Shell Invocation

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

## SYNOPSIS

> [spec:posix:syn:sh.synopsis]
> The sh utility shall be invocable in the following three forms:
>
> `sh [-abCefhimnuvx] [-o option]... [+abCefhimnuvx] [+o option]... [command_file [argument...]]`
>
> `sh -c [-abCefhimnuvx] [-o option]... [+abCefhimnuvx] [+o option]... command_string [command_name [argument...]]`
>
> `sh -s [-abCefhimnuvx] [-o option]... [+abCefhimnuvx] [+o option]... [argument...]`
>
> `[OB]` The `-h` and `+h` option letters are obsolescent in each of these
> synopsis forms.
>
> Source: XCU sh, SYNOPSIS — utilities/sh.html#tag_20_110_02

## DESCRIPTION

> [spec:posix:req:sh.command-language-interpreter]
> The sh utility is a command language interpreter that shall execute commands
> read from a command line string, the standard input, or a specified file. The
> application shall ensure that the commands to be executed are expressed in the
> language described in 2. Shell Command Language.
>
> Source: XCU sh, DESCRIPTION — utilities/sh.html#tag_20_110_03

> [spec:posix:req:sh.pathname-expansion-file-size]
> Pathname expansion shall not fail due to the size of a file.
>
> Source: XCU sh, DESCRIPTION — utilities/sh.html#tag_20_110_03

> [spec:posix:sem:sh.redirection-offset-maximum]
> Shell input and output redirections have an implementation-defined offset
> maximum that is established in the open file description.
>
> Source: XCU sh, DESCRIPTION — utilities/sh.html#tag_20_110_03

## OPTIONS

> [spec:posix:req:sh.utility-syntax-guidelines]
> The sh utility shall conform to XBD 12.2 Utility Syntax Guidelines, with an
> extension for support of a leading <plus-sign> (`'+'`).
>
> Source: XCU sh, OPTIONS — utilities/sh.html#tag_20_110_04

> [spec:posix:req:sh.set-derived-options]
> The -a, -b, -C, -e, -f, -h, -m, -n, -o option, -u, -v, and -x options are
> described as part of the set utility in 2.15 Special Built-In Utilities. The
> option letters derived from the set special built-in shall also be accepted
> with a leading <plus-sign> (`'+'`) instead of a leading <hyphen-minus>
> (meaning the reverse case of the option as described in this volume of
> POSIX.1-2024).
>
> `[OB]` The `-h` and `+h` option letters are obsolescent.
>
> Source: XCU sh, OPTIONS — utilities/sh.html#tag_20_110_04

> [spec:posix:req:sh.option-o-without-option-argument]
> If the -o or +o option is specified without an option-argument, the behavior
> is unspecified.
>
> Source: XCU sh, OPTIONS — utilities/sh.html#tag_20_110_04

> [spec:posix:req:sh.option-c]
> -c: Read commands from the command_string operand. Set the value of special
> parameter 0 (see 2.5.2 Special Parameters) from the value of the command_name
> operand and the positional parameters ($1, $2, and so on) in sequence from the
> remaining argument operands. No commands shall be read from the standard
> input.
>
> Source: XCU sh, OPTIONS — utilities/sh.html#tag_20_110_04

> [spec:posix:req:sh.option-i]
> -i: Specify that the shell is interactive. An implementation may treat
> specifying the -i option as an error if the real user ID of the calling
> process does not equal the effective user ID or if the real group ID does not
> equal the effective group ID.
>
> Source: XCU sh, OPTIONS — utilities/sh.html#tag_20_110_04

> [spec:posix:req:sh.option-s]
> -s: Read commands from the standard input.
>
> Source: XCU sh, OPTIONS — utilities/sh.html#tag_20_110_04

> [spec:posix:req:sh.option-s-assumed]
> If there are no operands and the -c option is not specified, the -s option
> shall be assumed.
>
> Source: XCU sh, OPTIONS — utilities/sh.html#tag_20_110_04

> [spec:posix:def:sh.interactive]
> If the -i option is present, or if the shell reads commands from the standard
> input and the shell's standard input and standard error are attached to a
> terminal, the shell is considered to be interactive.
>
> Source: XCU sh, OPTIONS — utilities/sh.html#tag_20_110_04

## OPERANDS

> [spec:posix:req:sh.operand-hyphen]
> A single <hyphen-minus> shall be treated as the first operand and then
> ignored. If both `'-'` and `"--"` are given as arguments, or if other operands
> precede the single <hyphen-minus>, the results are undefined.
>
> Source: XCU sh, OPERANDS — utilities/sh.html#tag_20_110_05

> [spec:posix:req:sh.operand-argument]
> argument: The positional parameters ($1, $2, and so on) shall be set to
> arguments, if any.
>
> Source: XCU sh, OPERANDS — utilities/sh.html#tag_20_110_05

> [spec:posix:req:sh.operand-command-file]
> command_file: The pathname of a file containing commands. If the pathname
> contains one or more <slash> characters, the implementation attempts to read
> that file; the file need not be executable. If the pathname does not contain a
> <slash> character:
>
> - The implementation shall attempt to read that file from the current working
> directory; the file need not be executable.
>
> - If the file is not in the current working directory, the implementation may
> perform a search for an executable file using the value of PATH, as
> described in 2.9.1.4 Command Search and Execution.
>
> Source: XCU sh, OPERANDS — utilities/sh.html#tag_20_110_05

> [spec:posix:req:sh.special-parameter-0]
> Special parameter 0 (see 2.5.2 Special Parameters) shall be set to the value
> of command_file. If sh is called using a synopsis form that omits
> command_file, special parameter 0 shall be set to the value of the first
> argument passed to sh from its parent (for example, argv[0] for a C program),
> which is normally a pathname used to execute the sh utility.
>
> Source: XCU sh, OPERANDS — utilities/sh.html#tag_20_110_05

> [spec:posix:req:sh.operand-command-name]
> command_name: A string assigned to special parameter 0 when executing the
> commands in command_string. If command_name is not specified, special
> parameter 0 shall be set to the value of the first argument passed to sh from
> its parent (for example, argv[0] for a C program), which is normally a
> pathname used to execute the sh utility.
>
> Source: XCU sh, OPERANDS — utilities/sh.html#tag_20_110_05

> [spec:posix:req:sh.operand-command-string]
> command_string: A string that shall be interpreted by the shell as one or more
> commands, as if the string were the argument to the system() function defined
> in the System Interfaces volume of POSIX.1-2024. If the command_string operand
> is an empty string, sh shall exit with a zero exit status.
>
> Source: XCU sh, OPERANDS — utilities/sh.html#tag_20_110_05

## STDIN

> [spec:posix:req:sh.stdin-used-only-if]
> The standard input shall be used only if one of the following is true:
>
> - The -s option is specified.
>
> - The -c option is not specified and no operands are specified.
>
> - The script executes one or more commands that require input from standard
> input (such as a read command that does not redirect its input).
>
> Source: XCU sh, STDIN — utilities/sh.html#tag_20_110_06

> [spec:posix:req:sh.stdin-no-read-ahead]
> When the shell is using standard input and it invokes a command that also uses
> standard input, the shell shall ensure that the standard input file pointer
> points directly after the command it has read when the command begins
> execution. It shall not read ahead in such a manner that any characters
> intended to be read by the invoked command are consumed by the shell (whether
> interpreted by the shell or not) or that characters that are not read by the
> invoked command are not seen by the shell. When the command expecting to read
> standard input is started asynchronously by an interactive shell, it is
> unspecified whether characters are read by the command or interpreted by the
> shell.
>
> Source: XCU sh, STDIN — utilities/sh.html#tag_20_110_06

> [spec:posix:req:sh.stdin-blocking-reads]
> If the standard input to sh is a FIFO or terminal device and is set to
> non-blocking reads, then sh shall enable blocking reads on standard input.
> This shall remain in effect when the command completes.
>
> Source: XCU sh, STDIN — utilities/sh.html#tag_20_110_06

## INPUT FILES

> [spec:posix:req:sh.input-file-contents]
> The input file can be of any type, but the initial portion of the file
> intended to be parsed according to the shell grammar (see 2.10.2 Shell Grammar
> Rules) shall consist of characters and shall not contain the NUL character.
> The shell shall not enforce any line length limits.
>
> Source: XCU sh, INPUT FILES — utilities/sh.html#tag_20_110_07

> [spec:posix:req:sh.input-file-blank-or-comments]
> If the input file consists solely of zero or more blank lines and comments, sh
> shall exit with a zero exit status.
>
> Source: XCU sh, INPUT FILES — utilities/sh.html#tag_20_110_07

## ENVIRONMENT VARIABLES

> [spec:posix:def:sh.environment-variables]
> The following environment variables shall affect the execution of sh: ENV,
> FCEDIT, HISTFILE, HISTSIZE, HOME, LANG, LC_ALL, LC_COLLATE, LC_CTYPE,
> LC_MESSAGES, MAIL, MAILCHECK, MAILPATH, NLSPATH, PATH, and PWD.
>
> Source: XCU sh, ENVIRONMENT VARIABLES — utilities/sh.html#tag_20_110_08

> [spec:posix:req:sh.envvar-env]
> `[UP]` ENV, when and only when an interactive shell is invoked, shall be subjected to
> parameter expansion (see 2.6.2 Parameter Expansion) by the shell, and the
> resulting value shall be used as a pathname of a file containing shell
> commands to execute in the current environment. The file need not be
> executable. If the expanded value of ENV is not an absolute pathname, the
> results are unspecified. ENV shall be ignored if the real and effective user
> IDs or real and effective group IDs of the process are different. The file
> specified by ENV need not be processed if the file can be written by any user
> other than the user identified by the real (and effective) user ID of the
> shell process.
>
> Source: XCU sh, ENVIRONMENT VARIABLES — utilities/sh.html#tag_20_110_08

> [spec:posix:req:sh.envvar-fcedit]
> `[UP]` FCEDIT, when expanded by the shell, shall determine the default value for the
> -e editor option's editor option-argument. If FCEDIT is null or unset, ed
> shall be used as the editor.
>
> Source: XCU sh, ENVIRONMENT VARIABLES — utilities/sh.html#tag_20_110_08

> [spec:posix:req:sh.envvar-histfile]
> `[UP]` HISTFILE determines a pathname naming a command history file. If the HISTFILE
> variable is not set, the shell may attempt to access or create a file
> .sh_history in the directory referred to by the HOME environment variable. If
> the shell cannot obtain both read and write access to, or create, the history
> file, it shall use an unspecified mechanism that allows the history to operate
> properly. (References to history "file" in this section shall be understood to
> mean this unspecified mechanism in such cases.) An implementation may choose
> to access this variable only when initializing the history file; this
> initialization shall occur when fc or sh first attempt to retrieve entries
> from, or add entries to, the file, as the result of commands issued by the
> user, the file named by the ENV variable, or implementation-defined system
> start-up files. Implementations may choose to disable the history list
> mechanism for users with appropriate privileges who do not set HISTFILE; the
> specific circumstances under which this occurs are implementation-defined. If
> more than one instance of the shell is using the same history file, it is
> unspecified how updates to the history file from those shells interact. As
> entries are deleted from the history file, they shall be deleted oldest first.
> It is unspecified when history file entries are physically removed from the
> history file.
>
> Source: XCU sh, ENVIRONMENT VARIABLES — utilities/sh.html#tag_20_110_08

> [spec:posix:req:sh.envvar-histsize]
> `[UP]` HISTSIZE determines a decimal number representing the limit to the number of
> previous commands that are accessible. If this variable is unset, an
> unspecified default greater than or equal to 128 shall be used. The maximum
> number of commands in the history list is unspecified, but shall be at least
> 128. An implementation may choose to access this variable only when
> initializing the history file, as described under HISTFILE. Therefore, it is
> unspecified whether changes made to HISTSIZE after the history file has been
> initialized are effective.
>
> Source: XCU sh, ENVIRONMENT VARIABLES — utilities/sh.html#tag_20_110_08

> [spec:posix:sem:sh.envvar-home]
> HOME determines the pathname of the user's home directory. The contents of
> HOME are used in tilde expansion as described in 2.6.1 Tilde Expansion.
>
> Source: XCU sh, ENVIRONMENT VARIABLES — utilities/sh.html#tag_20_110_08

> [spec:posix:sem:sh.envvar-lang-and-lc-all]
> LANG provides a default value for the internationalization variables that are
> unset or null. (See XBD 8.2 Internationalization Variables for the precedence
> of internationalization variables used to determine the values of locale
> categories.)
>
> LC_ALL, if set to a non-empty string value, overrides the values of all the
> other internationalization variables.
>
> Source: XCU sh, ENVIRONMENT VARIABLES — utilities/sh.html#tag_20_110_08

> [spec:posix:sem:sh.envvar-lc-collate]
> LC_COLLATE determines the behavior of range expressions, equivalence classes,
> and multi-character collating elements within pattern matching.
>
> Source: XCU sh, ENVIRONMENT VARIABLES — utilities/sh.html#tag_20_110_08

> [spec:posix:sem:sh.envvar-lc-ctype]
> LC_CTYPE determines the locale for the interpretation of sequences of bytes of
> text data as characters (for example, single-byte as opposed to multi-byte
> characters in arguments and input files), which characters are defined as
> letters (character class alpha), and the behavior of character classes within
> pattern matching.
>
> Source: XCU sh, ENVIRONMENT VARIABLES — utilities/sh.html#tag_20_110_08

> [spec:posix:req:sh.envvar-lc-messages]
> LC_MESSAGES determines the locale that should be used to affect the format and
> contents of diagnostic messages written to standard error.
>
> Source: XCU sh, ENVIRONMENT VARIABLES — utilities/sh.html#tag_20_110_08

> [spec:posix:req:sh.envvar-mail]
> `[UP]` MAIL determines a pathname of the user's mailbox file for purposes of incoming
> mail notification. If this variable is set, the shell shall inform the user if
> the file named by the variable is created or if its modification time has
> changed. Informing the user shall be accomplished by writing a string of
> unspecified format to standard error prior to the writing of the next primary
> prompt string. Such check shall be performed only after the completion of the
> interval defined by the MAILCHECK variable after the last such check. The user
> shall be informed only if MAIL is set and MAILPATH is not set.
>
> Source: XCU sh, ENVIRONMENT VARIABLES — utilities/sh.html#tag_20_110_08

> [spec:posix:req:sh.envvar-mailcheck]
> `[UP]` MAILCHECK establishes a decimal integer value that specifies how often (in
> seconds) the shell shall check for the arrival of mail in the files specified
> by the MAILPATH or MAIL variables. The default value shall be 600 seconds. If
> set to zero, the shell shall check before issuing each primary prompt.
>
> Source: XCU sh, ENVIRONMENT VARIABLES — utilities/sh.html#tag_20_110_08

> [spec:posix:req:sh.envvar-mailpath]
> `[UP]` MAILPATH provides a list of pathnames and optional messages separated by
> <colon> characters. If this variable is set, the shell shall inform the user
> if any of the files named by the variable are created or if any of their
> modification times change. (See the preceding entry for MAIL for descriptions
> of mail arrival and user informing.) Each pathname can be followed by `'%'`
> and a string that shall be subjected to parameter expansion and written to
> standard error when the modification time changes. If a `'%'` character in the
> pathname is preceded by a <backslash>, it shall be treated as a literal `'%'`
> in the pathname. The default message is unspecified.
>
> The MAILPATH environment variable takes precedence over the MAIL variable.
>
> Source: XCU sh, ENVIRONMENT VARIABLES — utilities/sh.html#tag_20_110_08

> [spec:posix:sem:sh.envvar-nlspath]
> `[XSI]` NLSPATH determines the location of messages
> objects and message catalogs.
>
> Source: XCU sh, ENVIRONMENT VARIABLES — utilities/sh.html#tag_20_110_08

> [spec:posix:sem:sh.envvar-path]
> PATH establishes a string formatted as described in XBD 8. Environment
> Variables, used to effect command interpretation; see 2.9.1.4 Command Search
> and Execution.
>
> Source: XCU sh, ENVIRONMENT VARIABLES — utilities/sh.html#tag_20_110_08

> [spec:posix:req:sh.envvar-pwd]
> PWD shall represent an absolute pathname of the current working directory.
> Assignments to this variable may be ignored.
>
> Source: XCU sh, ENVIRONMENT VARIABLES — utilities/sh.html#tag_20_110_08

## ASYNCHRONOUS EVENTS

> [spec:posix:req:sh.signals-standard-action]
> The sh utility shall take the standard action for all signals (see 1.4 Utility
> Description Defaults) with the exceptions stated for interactive shells.
>
> Source: XCU sh, ASYNCHRONOUS EVENTS — utilities/sh.html#tag_20_110_09

> [spec:posix:req:sh.interactive-sigint]
> If the shell is interactive, SIGINT signals received during command line
> editing shall be handled as described in the EXTENDED DESCRIPTION, and SIGINT
> signals received at other times shall be caught but no action performed.
>
> Source: XCU sh, ASYNCHRONOUS EVENTS — utilities/sh.html#tag_20_110_09

> [spec:posix:req:sh.interactive-sigquit-sigterm]
> If the shell is interactive, SIGQUIT and SIGTERM signals shall be ignored.
>
> Source: XCU sh, ASYNCHRONOUS EVENTS — utilities/sh.html#tag_20_110_09

> [spec:posix:req:sh.interactive-stop-signals]
> If the shell is interactive:
>
> - If the -m option is in effect, SIGTTIN, SIGTTOU, and SIGTSTP signals shall
> be ignored.
>
> - If the -m option is not in effect, it is unspecified whether SIGTTIN,
> SIGTTOU, and SIGTSTP signals are ignored, set to the default action, or
> caught. If they are caught, the shell shall, in the signal-catching
> function, set the signal to the default action and raise the signal (after
> taking any appropriate steps, such as restoring terminal settings).
>
> Source: XCU sh, ASYNCHRONOUS EVENTS — utilities/sh.html#tag_20_110_09

> [spec:posix:req:sh.signal-actions-overridable]
> The standard actions, and the actions described above for interactive shells,
> can be overridden by use of the trap special built-in utility (see trap and
> 2.12 Signals and Error Handling).
>
> Source: XCU sh, ASYNCHRONOUS EVENTS — utilities/sh.html#tag_20_110_09

## STDOUT / STDERR

> [spec:posix:req:sh.stderr-diagnostics-only]
> Except as otherwise stated (by the descriptions of any invoked utilities or in
> interactive mode), standard error shall be used only for diagnostic messages.
>
> Source: XCU sh, STDERR — utilities/sh.html#tag_20_110_11; the STDOUT
> section (#tag_20_110_10) states only "See the STDERR section."

## OUTPUT FILES

> [spec:posix:sem:sh.output-files]
> None.
>
> Source: XCU sh, OUTPUT FILES — utilities/sh.html#tag_20_110_12

## EXIT STATUS

> [spec:posix:req:sh.exit-status-values]
> The following exit values shall be returned:
>
> | Value | Meaning |
> |---|---|
> | 0 | The script to be executed consisted solely of zero or more blank lines or comments, or both. |
> | 1-125 | A non-interactive shell detected an error other than command_file not found, command_file not executable, or an unrecoverable read error while reading commands (except from the file operand of the dot special built-in); including but not limited to syntax, redirection, or variable assignment errors. |
> | 126 | A specified command_file could not be executed due to an [ENOEXEC] error (see 2.9.1.4 Command Search and Execution, item 2). |
> | 127 | A specified command_file could not be found by a non-interactive shell. |
> | 128 | An unrecoverable read error was detected while reading commands, except from the file operand of the dot special built-in. |
>
> Source: XCU sh, EXIT STATUS — utilities/sh.html#tag_20_110_14

> [spec:posix:req:sh.exit-status-otherwise]
> Otherwise, the shell shall terminate in the same manner as for an exit command
> with no operands, unless the last command the shell invoked was executed
> without forking, in which case the wait status seen by the parent process of
> the shell shall be the wait status of the last command the shell invoked. See
> the exit utility in 2.15 Special Built-In Utilities.
>
> Source: XCU sh, EXIT STATUS — utilities/sh.html#tag_20_110_14

## CONSEQUENCES OF ERRORS

> [spec:posix:req:sh.consequences-of-errors]
> The consequences of errors for the sh utility shall be as described in 2.8.1
> Consequences of Shell Errors.
>
> Source: XCU sh, CONSEQUENCES OF ERRORS — utilities/sh.html#tag_20_110_15
