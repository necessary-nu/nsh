# Parameters and Variables

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

## 2.5 Parameters and Variables

> [spec:posix:def:param.denotation]
> A parameter can be denoted by a name, a number, or one of the special
> characters listed in 2.5.2 Special Parameters. A variable is a parameter
> denoted by a name.
>
> Source: XCU 2.5 Parameters and Variables — utilities/V3_chap02.html#tag_19_05

> [spec:posix:def:param.set-state]
> A parameter is set if it has an assigned value (null is a valid value). Once a
> variable is set, it can only be unset by using the unset special built-in
> command.
>
> Source: XCU 2.5 Parameters and Variables — utilities/V3_chap02.html#tag_19_05

> [spec:posix:req:param.byte-values]
> Parameters can contain arbitrary byte sequences, except for the null byte. The
> shell shall process their values as characters only when performing operations
> that are described in this standard in terms of characters.
>
> Source: XCU 2.5 Parameters and Variables — utilities/V3_chap02.html#tag_19_05

## 2.5.1 Positional Parameters

> [spec:posix:def:param.positional-definition]
> A positional parameter is a parameter denoted by a decimal representation of a
> positive integer.
>
> Source: XCU 2.5.1 Positional Parameters — utilities/V3_chap02.html#tag_19_05_01

> [spec:posix:req:param.positional-decimal-digits]
> The digits denoting the positional parameters shall always be interpreted as a
> decimal value, even if there is a leading zero.
>
> For example, `"$8"`, `"${8}"`, `"${08}"`, and `"${008}"` all expand to the
> value of the eighth positional parameter.
>
> Source: XCU 2.5.1 Positional Parameters — utilities/V3_chap02.html#tag_19_05_01

> [spec:posix:syn:param.positional-multi-digit-braces]
> When a positional parameter with more than one digit is specified, the
> application shall enclose the digits in braces (see 2.6.2 Parameter Expansion).
>
> For example, `"${10}"` expands to the value of the tenth positional parameter,
> whereas `"$10"` expands to the value of the first positional parameter followed
> by the character '0'.
>
> Source: XCU 2.5.1 Positional Parameters — utilities/V3_chap02.html#tag_19_05_01

> [spec:posix:sem:param.positional-zero-not-positional]
> 0 is a special parameter, not a positional parameter, and therefore the results
> of expanding `${00}` are unspecified.
>
> Source: XCU 2.5.1 Positional Parameters — utilities/V3_chap02.html#tag_19_05_01

> [spec:posix:sem:param.positional-assignment]
> Positional parameters are initially assigned when the shell is invoked (see
> sh), temporarily replaced when a shell function is invoked (see 2.9.5 Function
> Definition Command), and can be reassigned with the set special built-in
> command.
>
> Source: XCU 2.5.1 Positional Parameters — utilities/V3_chap02.html#tag_19_05_01

## 2.5.2 Special Parameters

> [spec:posix:def:param.special-parameters]
> The special parameters are `@`, `*`, `#`, `?`, `-` (hyphen), `$`, `!`, and 0
> (zero). Each has a value to which it shall expand, as specified in the rules
> that follow. Only the values of the special parameters are specified there; see
> 2.6 Word Expansions for a detailed summary of all the stages involved in
> expanding words.
>
> Source: XCU 2.5.2 Special Parameters — utilities/V3_chap02.html#tag_19_05_02

> [spec:posix:req:param.special-at]
> Special parameter `@`: Expands to the positional parameters, starting from one,
> initially producing one field for each positional parameter that is set. When
> the expansion occurs in a context where field splitting will be performed, any
> empty fields may be discarded and each of the non-empty fields shall be further
> split as described in 2.6.5 Field Splitting.
>
> Source: XCU 2.5.2 Special Parameters — utilities/V3_chap02.html#tag_19_05_02

> [spec:posix:req:param.special-at-double-quotes]
> When the expansion of the special parameter `@` occurs within double-quotes,
> the behavior is unspecified unless one of the following is true:
>
> - Field splitting as described in 2.6.5 Field Splitting would be performed if
> the expansion were not within double-quotes (regardless of whether field
> splitting would have any effect; for example, if IFS is null).
> - The double-quotes are within the word of a `${parameter:-word}` or a
> `${parameter:+word}` expansion (with or without the <colon>; see 2.6.2
> Parameter Expansion) which would have been subject to field splitting if
> parameter had been expanded instead of word.
>
> If one of these conditions is true, the initial fields shall be retained as
> separate fields, except that if the parameter being expanded was embedded
> within a word, the first field shall be joined with the beginning part of the
> original word and the last field shall be joined with the end part of the
> original word. In all other contexts the results of the expansion are
> unspecified.
>
> Source: XCU 2.5.2 Special Parameters — utilities/V3_chap02.html#tag_19_05_02

> [spec:posix:req:param.special-at-no-positional]
> If there are no positional parameters, the expansion of `'@'` shall generate
> zero fields, even when `'@'` is within double-quotes; however, if the expansion
> is embedded within a word which contains one or more other parts that expand to
> a quoted null string, these null string(s) shall still produce an empty field,
> except that if the other parts are all within the same double-quotes as the
> `'@'`, it is unspecified whether the result is zero fields or one empty field.
>
> Source: XCU 2.5.2 Special Parameters — utilities/V3_chap02.html#tag_19_05_02

> [spec:posix:req:param.special-asterisk]
> Special parameter `*`: Expands to the positional parameters, starting from one,
> initially producing one field for each positional parameter that is set. When
> the expansion occurs in a context where field splitting will be performed, any
> empty fields may be discarded and each of the non-empty fields shall be further
> split as described in 2.6.5 Field Splitting. When the expansion occurs in a
> context where field splitting will not be performed, the initial fields shall
> be joined to form a single field with the value of each parameter separated by
> the first character of the IFS variable if IFS contains at least one character,
> or separated by a <space> if IFS is unset, or with no separation if IFS is set
> to a null string.
>
> Source: XCU 2.5.2 Special Parameters — utilities/V3_chap02.html#tag_19_05_02

> [spec:posix:req:param.special-hash]
> Special parameter `#`: Expands to the shortest representation of the decimal
> number of positional parameters. The command name (parameter 0) shall not be
> counted in the number given by `'#'` because it is a special parameter, not a
> positional parameter.
>
> Source: XCU 2.5.2 Special Parameters — utilities/V3_chap02.html#tag_19_05_02

> [spec:posix:req:param.special-question]
> Special parameter `?`: Expands to the shortest representation of the decimal
> exit status (see 2.8.2 Exit Status for Commands) of the pipeline (see 2.9.2
> Pipelines) executed from the current shell execution environment (not a
> subshell environment) that most recently either terminated or, optionally but
> only if the shell is interactive and job control is enabled, was stopped by a
> signal. If this pipeline terminated, the status value shall be its exit status;
> otherwise, the status value shall be the same as the exit status that would
> have resulted if the pipeline had been terminated by a signal with the same
> number as the signal that stopped it. The value of the special parameter `'?'`
> shall be set to 0 during initialization of the shell. When a subshell
> environment is created, the value of the special parameter `'?'` from the
> invoking shell environment shall be preserved in the subshell.
>
> Source: XCU 2.5.2 Special Parameters — utilities/V3_chap02.html#tag_19_05_02

> [spec:posix:sem:param.special-question-assignment]
> In `var=$(some_command); echo $?` the output is the exit status of
> some_command, which is executed in a subshell environment, but this is because
> its exit status becomes the exit status of the assignment command
> `var=$(some_command)` (see 2.9.1 Simple Commands) and this assignment command
> is the most recently completed pipeline. Likewise for any pipeline consisting
> entirely of a simple command that has no command word, but contains one or more
> command substitutions.
>
> Source: XCU 2.5.2 Special Parameters — utilities/V3_chap02.html#tag_19_05_02

> [spec:posix:req:param.special-hyphen]
> Special parameter `-` (hyphen): Expands to the current option flags (the
> single-letter option names concatenated into a string) as specified on
> invocation, by the set special built-in command, or implicitly by the shell. It
> is unspecified whether the -c and -s options are included in the expansion of
> `"$-"`. The -i option shall be included in `"$-"` if the shell is interactive,
> regardless of whether it was specified on invocation.
>
> Source: XCU 2.5.2 Special Parameters — utilities/V3_chap02.html#tag_19_05_02

> [spec:posix:req:param.special-dollar]
> Special parameter `$`: Expands to the shortest representation of the decimal
> process ID of the invoked shell. In a subshell (see 2.13 Shell Execution
> Environment), `'$'` shall expand to the same value as that of the current
> shell.
>
> Source: XCU 2.5.2 Special Parameters — utilities/V3_chap02.html#tag_19_05_02

> [spec:posix:req:param.special-bang]
> Special parameter `!`: Expands to the shortest representation of the decimal
> process ID associated with the most recent asynchronous AND-OR list (see 2.9.3.1
> Asynchronous AND-OR Lists) executed from the current shell execution
> environment, or to the shortest representation of the decimal process ID of the
> last command specified in the currently executing pipeline in the job-control
> background job that most recently resumed execution through the use of bg,
> whichever is the most recent.
>
> Source: XCU 2.5.2 Special Parameters — utilities/V3_chap02.html#tag_19_05_02

> [spec:posix:req:param.special-zero]
> Special parameter 0 (zero): Expands to the name of the shell or shell script.
> See sh for a detailed description of how this name is derived.
>
> Source: XCU 2.5.2 Special Parameters — utilities/V3_chap02.html#tag_19_05_02

## 2.5.3 Shell Variables

> [spec:posix:req:param.variable-environment-initialization]
> Variables shall be initialized from the environment (as defined by XBD 8.
> Environment Variables and the exec function in the System Interfaces volume of
> POSIX.1-2024) and can be given new values with variable assignment commands.
> Shell variables shall be initialized only from environment variables that have
> valid names. If a variable is initialized from the environment, it shall be
> marked for export immediately; see the export special built-in.
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03

> [spec:posix:sem:param.variable-creation]
> New variables can be defined and initialized with variable assignments, with
> the read or getopts utilities, with the name parameter in a for loop, with the
> `${name=word}` expansion, or with other mechanisms provided as implementation
> extensions.
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03

> [spec:posix:def:param.shell-variables]
> The following variables shall affect the execution of the shell: ENV, HOME,
> IFS, LANG, LC_ALL, LC_COLLATE, LC_CTYPE, LC_MESSAGES, LINENO, NLSPATH, PATH,
> PPID, PS1, PS2, PS4, and PWD.
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03

> [spec:posix:req:param.env]
> `[UP]` The processing of the ENV shell variable shall be supported if the
> system supports the User Portability Utilities option.
>
> This variable, when and only when an interactive shell is invoked, shall be
> subjected to parameter expansion (see 2.6.2 Parameter Expansion) by the shell
> and the resulting value shall be used as a pathname of a file. Before any
> interactive commands are read, the shell shall tokenize (see 2.3 Token
> Recognition) the contents of the file, parse the tokens as a program (see 2.10
> Shell Grammar), and execute the resulting commands in the current environment.
> (In other words, the contents of the ENV file are not parsed as a single
> compound_list. This distinction matters because it influences when aliases take
> effect.) The file need not be executable. If the expanded value of ENV is not
> an absolute pathname, the results are unspecified. ENV shall be ignored if the
> user's real and effective user IDs or real and effective group IDs are
> different.
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03

> [spec:posix:def:param.home]
> Shell variable HOME: The pathname of the user's home directory. The contents of
> HOME are used in tilde expansion (see 2.6.1 Tilde Expansion).
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03

> [spec:posix:def:param.ifs]
> Shell variable IFS: A string treated as a list of characters that is used for
> field splitting, expansion of the `'*'` special parameter, and to split lines
> into fields with the read utility. If the value of IFS includes any bytes that
> do not form part of a valid character, the results of field splitting,
> expansion of `'*'`, and use of the read utility are unspecified.
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03

> [spec:posix:req:param.ifs-unset]
> If IFS is not set, it shall behave as normal for an unset variable, except that
> field splitting by the shell and line splitting by the read utility shall be
> performed as if the value of IFS is <space><tab><newline>; see 2.6.5 Field
> Splitting.
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03

> [spec:posix:req:param.ifs-initial-value]
> The shell shall set IFS to <space><tab><newline> when it is invoked.
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03

> [spec:posix:req:param.lang]
> Shell variable LANG: Provide a default value for the internationalization
> variables that are unset or null. (See XBD 8.2 Internationalization Variables
> for the precedence of internationalization variables used to determine the
> values of locale categories.)
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03

> [spec:posix:req:param.lc-all]
> Shell variable LC_ALL: The value of this variable overrides the LC_* variables
> and LANG, as described in XBD 8. Environment Variables.
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03

> [spec:posix:req:param.lc-collate]
> Shell variable LC_COLLATE: Determine the behavior of range expressions,
> equivalence classes, and multi-character collating elements within pattern
> matching.
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03

> [spec:posix:req:param.lc-ctype]
> Shell variable LC_CTYPE: Determine the interpretation of sequences of bytes of
> text data as characters (for example, single-byte as opposed to multi-byte
> characters), which characters are defined as letters (character class alpha)
> and <blank> characters (character class blank), and the behavior of character
> classes within pattern matching. Changing the value of LC_CTYPE after the shell
> has started shall not affect the lexical processing of shell commands in the
> current shell execution environment or its subshells. Invoking a shell script
> or performing exec sh subjects the new shell to the changes in LC_CTYPE.
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03

> [spec:posix:req:param.lc-messages]
> Shell variable LC_MESSAGES: Determine the language in which messages should be
> written.
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03

> [spec:posix:req:param.lineno]
> `[UP]` The processing of the LINENO shell variable shall be supported if the
> system supports the User Portability Utilities option.
>
> Set by the shell to a decimal number representing the current sequential line
> number (numbered starting with 1) within a script or function before it
> executes each command. If the user unsets or resets LINENO, the variable may
> lose its special meaning for the life of the shell. If the shell is not
> currently executing a script or function, the value of LINENO is unspecified.
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03

> [spec:posix:req:param.nlspath]
> `[XSI]` Determine the location of message catalogs for the processing of
> LC_MESSAGES.
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03

> [spec:posix:def:param.path]
> Shell variable PATH: A string formatted as described in XBD 8. Environment
> Variables, used to effect command interpretation; see 2.9.1.4 Command Search
> and Execution.
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03

> [spec:posix:req:param.ppid]
> Shell variable PPID: Set by the shell to the decimal value of its parent
> process ID during initialization of the shell. In a subshell (see 2.13 Shell
> Execution Environment), PPID shall be set to the same value as that of the
> parent of the current shell. For example, `echo $PPID` and `(echo $PPID)` would
> produce the same value.
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03

> [spec:posix:req:param.ps1]
> `[UP]` The processing of the PS1 shell variable shall be supported if the
> system supports the User Portability Utilities option.
>
> Each time an interactive shell is ready to read a command, the value of this
> variable shall be subjected to parameter expansion (see 2.6.2 Parameter
> Expansion) and exclamation-mark expansion. Whether the value is also subjected
> to command substitution (see 2.6.3 Command Substitution) or arithmetic
> expansion (see 2.6.4 Arithmetic Expansion) or both is unspecified. After
> expansion, the value shall be written to standard error.
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03

> [spec:posix:req:param.ps1-two-pass]
> `[UP]` The PS1 expansions shall be performed in two passes, where the result of the
> first pass is input to the second pass. One of the passes shall perform only
> the exclamation-mark expansion described in `[spec:posix:req:param.ps1-exclamation-expansion]`.
> The other pass shall perform the other expansion(s) according to the rules in
> 2.6 Word Expansions. Which of the two passes is performed first is unspecified.
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03

> [spec:posix:req:param.ps1-default]
> `[UP]` The default value of PS1 shall be `"$ "`. For users who have specific
> additional implementation-defined privileges, the default may be another,
> implementation-defined value.
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03

> [spec:posix:req:param.ps1-exclamation-expansion]
> `[UP]` Exclamation-mark expansion: The shell shall replace each instance of the
> <exclamation-mark> character (`'!'`) with the history file number (see Command
> History List) of the next command to be typed. An <exclamation-mark> character
> escaped by another <exclamation-mark> character (that is, `"!!"`) shall expand
> to a single <exclamation-mark> character.
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03

> [spec:posix:req:param.ps2]
> `[UP]` The processing of the PS2 shell variable shall be supported if the
> system supports the User Portability Utilities option.
>
> Each time the user enters a <newline> prior to completing a command line in an
> interactive shell, the value of this variable shall be subjected to parameter
> expansion (see 2.6.2 Parameter Expansion). Whether the value is also subjected
> to command substitution (see 2.6.3 Command Substitution) or arithmetic
> expansion (see 2.6.4 Arithmetic Expansion) or both is unspecified. After
> expansion, the value shall be written to standard error. The default value
> shall be `"> "`.
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03

> [spec:posix:req:param.ps4]
> `[UP]` The processing of the PS4 shell variable shall be supported if the
> system supports the User Portability Utilities option.
>
> When an execution trace (set -x) is being performed, before each line in the
> execution trace, the value of this variable shall be subjected to parameter
> expansion (see 2.6.2 Parameter Expansion). Whether the value is also subjected
> to command substitution (see 2.6.3 Command Substitution) or arithmetic
> expansion (see 2.6.4 Arithmetic Expansion) or both is unspecified. After
> expansion, the value shall be written to standard error. The default value
> shall be `"+ "`.
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03

> [spec:posix:req:param.pwd]
> Shell variable PWD: Set by the shell and by the cd utility. In the shell the
> value shall be initialized from the environment as follows. If a value for PWD
> is passed to the shell in the environment when it is executed, the value is an
> absolute pathname of the current working directory that is no longer than
> {PATH_MAX} bytes including the terminating null byte, and the value does not
> contain any components that are dot or dot-dot, then the shell shall set PWD to
> the value from the environment. Otherwise, if a value for PWD is passed to the
> shell in the environment when it is executed, the value is an absolute pathname
> of the current working directory, and the value does not contain any components
> that are dot or dot-dot, then it is unspecified whether the shell sets PWD to
> the value from the environment or sets PWD to the pathname that would be output
> by pwd -P. Otherwise, the sh utility sets PWD to the pathname that would be
> output by pwd -P.
>
> In cases where PWD is set to the value from the environment, the value can
> contain components that refer to files of type symbolic link. In cases where
> PWD is set to the pathname that would be output by pwd -P, if there is
> insufficient permission on the current working directory, or on any parent of
> that directory, to determine what that pathname would be, the value of PWD is
> unspecified.
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03

> [spec:posix:req:param.pwd-assignment]
> Assignments to the PWD variable may be ignored. If an application sets or
> unsets the value of PWD, the behaviors of the cd and pwd utilities are
> unspecified.
>
> Source: XCU 2.5.3 Shell Variables — utilities/V3_chap02.html#tag_19_05_03
