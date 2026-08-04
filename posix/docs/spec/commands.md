# Shell Commands

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
"SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in RFC 2119. This document
reproduces normative text from IEEE Std 1003.1-2024 (POSIX.1-2024),
Copyright © 2001-2024 The IEEE and The Open Group.

## 2.9 Shell Commands

> [spec:posix:syn:cmd.format-descriptions-informal]
> The command descriptions in this section each describe a format of the command
> that is only used to aid the reader in recognizing the command type, and does
> not formally represent the syntax. In particular, the representations include
> spacing between tokens in some places where <blank>s would not be necessary
> (when one of the tokens is an operator). Each description discusses the
> semantics of the command; for a formal definition of the command language,
> consult 2.10 Shell Grammar.
>
> Source: XCU 2.9 Shell Commands — utilities/V3_chap02.html#tag_19_09

> [spec:posix:def:cmd.command-kinds]
> A command is one of the following:
>
> - Simple command (see 2.9.1 Simple Commands)
> - Pipeline (see 2.9.2 Pipelines)
> - List compound-list (see 2.9.3 Lists)
> - Compound command (see 2.9.4 Compound Commands)
> - Function definition (see 2.9.5 Function Definition Command)
>
> Source: XCU 2.9 Shell Commands — utilities/V3_chap02.html#tag_19_09

> [spec:posix:req:cmd.default-exit-status]
> Unless otherwise stated, the exit status of a command shall be that of the
> last simple command executed by the command.
>
> Source: XCU 2.9 Shell Commands — utilities/V3_chap02.html#tag_19_09

> [spec:posix:req:cmd.no-size-limit]
> There shall be no limit on the size of any shell command other than that
> imposed by the underlying system (memory constraints, {ARG_MAX}, and so on).
>
> Source: XCU 2.9 Shell Commands — utilities/V3_chap02.html#tag_19_09

## 2.9.1 Simple Commands

> [spec:posix:def:cmd.simple-definition]
> A "simple command" is a sequence of optional variable assignments and
> redirections, in any sequence, optionally followed by words and redirections.
>
> Source: XCU 2.9.1 Simple Commands — utilities/V3_chap02.html#tag_19_09_01

### 2.9.1.1 Order of Processing

> [spec:posix:req:cmd.simple-processing-order]
> When a given simple command is required to be executed (that is, when any
> conditional construct such as an AND-OR list or a **case** statement has not
> bypassed the simple command), the expansions, assignments, and redirections
> described in steps 1 through 4 of this subclause shall all be performed from
> the beginning of the command text to the end.
>
> Step 1: The words that are recognized as variable assignments or redirections
> according to 2.10.2 Shell Grammar Rules are saved for processing in steps 3
> and 4.
>
> Source: XCU 2.9.1.1 Order of Processing — utilities/V3_chap02.html#tag_19_09_01_01

> [spec:posix:req:cmd.simple-command-name-determination]
> Step 2: The first word (if any) that is not a variable assignment or
> redirection shall be expanded. If any fields remain following its expansion,
> the first field shall be considered the command name. If no fields remain, the
> next word (if any) shall be expanded, and so on, until a command name is found
> or no words remain.
>
> Source: XCU 2.9.1.1 Order of Processing — utilities/V3_chap02.html#tag_19_09_01_01

> [spec:posix:req:cmd.simple-declaration-utility-expansion]
> Step 2 (continued): If there is a command name and it is recognized as a
> declaration utility, then any remaining words after the word that expanded to
> produce the command name, that would be recognized as a variable assignment in
> isolation, shall be expanded as a variable assignment (tilde expansion after
> the first <equals-sign> and after any unquoted <colon>, parameter expansion,
> command substitution, arithmetic expansion, and quote removal, but no field
> splitting or pathname expansion); while remaining words that would not be a
> variable assignment in isolation shall be subject to regular expansion (tilde
> expansion for only a leading <tilde>, parameter expansion, command
> substitution, arithmetic expansion, field splitting, pathname expansion, and
> quote removal).
>
> Source: XCU 2.9.1.1 Order of Processing — utilities/V3_chap02.html#tag_19_09_01_01

> [spec:posix:req:cmd.simple-argument-expansion]
> Step 2 (continued): For all command names other than declaration utilities,
> words after the word that produced the command name shall be subject only to
> regular expansion. All fields resulting from the expansion of the word that
> produced the command name and the subsequent words, except for the field
> containing the command name, shall be the arguments for the command.
>
> Source: XCU 2.9.1.1 Order of Processing — utilities/V3_chap02.html#tag_19_09_01_01

> [spec:posix:req:cmd.simple-redirections-performed]
> Step 3: Redirections shall be performed as described in 2.7 Redirection.
>
> Source: XCU 2.9.1.1 Order of Processing — utilities/V3_chap02.html#tag_19_09_01_01

> [spec:posix:req:cmd.simple-assignment-expansion]
> Step 4: Each variable assignment shall be expanded for tilde expansion,
> parameter expansion, command substitution, arithmetic expansion, and quote
> removal prior to assigning the value.
>
> Source: XCU 2.9.1.1 Order of Processing — utilities/V3_chap02.html#tag_19_09_01_01

> [spec:posix:req:cmd.simple-step-order-reversal]
> The order of steps 3 and 4 may be reversed if no command name results from
> step 2 or if the command name matches the name of a special built-in utility;
> see 2.15 Special Built-In Utilities.
>
> Source: XCU 2.9.1.1 Order of Processing — utilities/V3_chap02.html#tag_19_09_01_01

> [spec:posix:req:cmd.declaration-utility-lexical-analysis]
> When determining whether a command name is a declaration utility, an
> implementation may use only lexical analysis. It is unspecified whether
> assignment context will be used if the command name would only become
> recognized as a declaration utility after word expansions.
>
> Source: XCU 2.9.1.1 Order of Processing — utilities/V3_chap02.html#tag_19_09_01_01

### 2.9.1.2 Variable Assignments

> [spec:posix:req:cmd.assign-no-command-name]
> If no command name results, variable assignments shall affect the current
> execution environment.
>
> Source: XCU 2.9.1.2 Variable Assignments — utilities/V3_chap02.html#tag_19_09_01_02

> [spec:posix:req:cmd.assign-exported-to-command]
> If the command name is not a special built-in utility or function, the
> variable assignments shall be exported for the execution environment of the
> command and shall not affect the current execution environment except as a
> side-effect of the expansions performed in step 4. In this case it is
> unspecified:
>
> - Whether or not the assignments are visible for subsequent expansions in
>   step 4
> - Whether variable assignments made as side-effects of these expansions are
>   visible for subsequent expansions in step 4, or in the current shell
>   execution environment, or both
>
> Source: XCU 2.9.1.2 Variable Assignments — utilities/V3_chap02.html#tag_19_09_01_02

> [spec:posix:req:cmd.assign-standard-utility-as-function]
> If the command name is a standard utility implemented as a function (see XBD
> 4.25 Utility), the effect of variable assignments shall be as if the utility
> was not implemented as a function.
>
> Source: XCU 2.9.1.2 Variable Assignments — utilities/V3_chap02.html#tag_19_09_01_02

> [spec:posix:req:cmd.assign-special-builtin]
> If the command name is a special built-in utility, variable assignments shall
> affect the current execution environment before the utility is executed and
> remain in effect when the command completes; if an assigned variable is further
> modified by the utility, the modifications made by the utility shall persist.
> Unless the *set* **-a** option is on, it is unspecified:
>
> - Whether or not the variables gain the *export* attribute during the
>   execution of the special built-in utility
> - Whether or not *export* attributes gained as a result of the variable
>   assignments persist after the completion of the special built-in utility
>
> Source: XCU 2.9.1.2 Variable Assignments — utilities/V3_chap02.html#tag_19_09_01_02

> [spec:posix:req:cmd.assign-function]
> If the command name is a function that is not a standard utility implemented
> as a function, variable assignments shall affect the current execution
> environment during the execution of the function. It is unspecified:
>
> - Whether or not the variable assignments persist after the completion of the
>   function
> - Whether or not the variables gain the *export* attribute during the
>   execution of the function
> - Whether or not *export* attributes gained as a result of the variable
>   assignments persist after the completion of the function (if variable
>   assignments persist after the completion of the function)
>
> Source: XCU 2.9.1.2 Variable Assignments — utilities/V3_chap02.html#tag_19_09_01_02

> [spec:posix:req:cmd.assign-readonly-error]
> If any of the variable assignments attempt to assign a value to a variable for
> which the *readonly* attribute is set in the current shell environment
> (regardless of whether the assignment is made in that environment), a variable
> assignment error shall occur. See 2.8.1 Consequences of Shell Errors for the
> consequences of these errors.
>
> Source: XCU 2.9.1.2 Variable Assignments — utilities/V3_chap02.html#tag_19_09_01_02

### 2.9.1.3 Commands with no Command Name

> [spec:posix:req:cmd.no-name-redirections-subshell]
> If a simple command has no command name after word expansion (see 2.9.1.1
> Order of Processing), any redirections shall be performed in a subshell
> environment; it is unspecified whether this subshell environment is the same
> one as that used for a command substitution within the command. To affect the
> current execution environment, see the *exec* special built-in.
>
> Source: XCU 2.9.1.3 Commands with no Command Name — utilities/V3_chap02.html#tag_19_09_01_03

> [spec:posix:req:cmd.no-name-redirection-failure]
> If any of the redirections performed in the current shell execution
> environment fail, the command shall immediately fail with an exit status
> greater than zero, and the shell shall write an error message indicating the
> failure. See 2.8.1 Consequences of Shell Errors for the consequences of these
> failures on interactive and non-interactive shells.
>
> Source: XCU 2.9.1.3 Commands with no Command Name — utilities/V3_chap02.html#tag_19_09_01_03

> [spec:posix:req:cmd.no-name-exit-status]
> If there is no command name but the command contains a command substitution,
> the command shall complete with the exit status of the command substitution
> whose exit status was the last to be obtained. Otherwise, the command shall
> complete with a zero exit status.
>
> Source: XCU 2.9.1.3 Commands with no Command Name — utilities/V3_chap02.html#tag_19_09_01_03

### 2.9.1.4 Command Search and Execution

> [spec:posix:req:cmd.search-applies]
> If a simple command has a command name and an optional list of arguments after
> word expansion (see 2.9.1.1 Order of Processing), the actions described in
> this subclause shall be performed. If the command name does not contain any
> <slash> characters, the first successful step in the sequence given by
> `cmd.search-special-builtin`, `cmd.search-unspecified-utility-names`,
> `cmd.search-function`, `cmd.search-intrinsic-utility`, and the *PATH* search
> shall occur.
>
> Source: XCU 2.9.1.4 Command Search and Execution — utilities/V3_chap02.html#tag_19_09_01_04

> [spec:posix:req:cmd.search-special-builtin]
> Step 1a: If the command name matches the name of a special built-in utility,
> that special built-in utility shall be invoked.
>
> Source: XCU 2.9.1.4 Command Search and Execution — utilities/V3_chap02.html#tag_19_09_01_04

> [spec:posix:req:cmd.search-unspecified-utility-names]
> Step 1b: If the command name matches the name of a utility listed in the
> following table, the results are unspecified.
>
> | | | | | |
> |:---|:---|:---|:---|:---|
> | *alloc* | *compcall* | *compvalues* | *history* | *print* |
> | *autoload* | *compctl* | *declare* | *hist* | *pushd* |
> | *bind* | *compdescribe* | *dirs* | *integer* | *readarray* |
> | *bindkey* | *compfiles* | *disable* | *let* | *repeat* |
> | *builtin* | *compgen* | *disown* | *local* | *savehistory* |
> | *bye* | *compgroups* | *dosh* | *login* | *source* |
> | *caller* | *complete* | *echotc* | *logout* | *shopt* |
> | *cap* | *compound* | *echoti* | *map* | *stop* |
> | *chdir* | *compquote* | *enum* | *mapfile* | *suspend* |
> | *clone* | *comptags* | *float* | *nameref* | *typeset* |
> | *comparguments* | *comptry* | *help* | *popd* | *whence* |
>
> Source: XCU 2.9.1.4 Command Search and Execution — utilities/V3_chap02.html#tag_19_09_01_04

> [spec:posix:req:cmd.search-function]
> Step 1c: If the command name matches the name of a function known to this
> shell, the function shall be invoked as described in 2.9.5 Function Definition
> Command. If the implementation has provided a standard utility in the form of
> a function, and that function definition still exists (i.e. has not been
> removed using *unset* **-f** or replaced via another function definition with
> the same name), it shall not be recognized at this point. It shall be invoked
> in conjunction with the path search in step 1e.
>
> Source: XCU 2.9.1.4 Command Search and Execution — utilities/V3_chap02.html#tag_19_09_01_04

> [spec:posix:req:cmd.search-intrinsic-utility]
> Step 1d: If the command name matches the name of an intrinsic utility (see
> XCU 1.7 Intrinsic Utilities), that utility shall be invoked.
>
> Source: XCU 2.9.1.4 Command Search and Execution — utilities/V3_chap02.html#tag_19_09_01_04

> [spec:posix:req:cmd.search-path-associated-builtin]
> Step 1e: Otherwise, the command shall be searched for using the *PATH*
> environment variable as described in XBD 8. Environment Variables. If the
> search is successful and the system has implemented the utility as a built-in
> or as a shell function, and the built-in or function is associated with the
> directory that was most recently tested during the successful *PATH* search,
> that built-in or function shall be invoked.
>
> Source: XCU 2.9.1.4 Command Search and Execution — utilities/V3_chap02.html#tag_19_09_01_04

> [spec:posix:req:cmd.search-path-non-builtin]
> Step 1e (continued): Otherwise, if the *PATH* search is successful, the shell
> shall execute a non-built-in utility as described in 2.9.1.6 Non-built-in
> Utility Execution.
>
> Source: XCU 2.9.1.4 Command Search and Execution — utilities/V3_chap02.html#tag_19_09_01_04

> [spec:posix:req:cmd.search-remembered-location]
> Once a utility has been searched for and found (either as a result of this
> specific search or as part of an unspecified shell start-up activity), an
> implementation may remember its location and need not search for the utility
> again unless the *PATH* variable has been the subject of an assignment. If the
> remembered location fails for a subsequent invocation, the shell shall repeat
> the search to find the new location for the utility, if any.
>
> Source: XCU 2.9.1.4 Command Search and Execution — utilities/V3_chap02.html#tag_19_09_01_04

> [spec:posix:req:cmd.search-path-unsuccessful]
> Step 1e (continued): If the *PATH* search is unsuccessful, the command shall
> fail with an exit status of 127 and the shell shall write an error message.
>
> Source: XCU 2.9.1.4 Command Search and Execution — utilities/V3_chap02.html#tag_19_09_01_04

> [spec:posix:req:cmd.search-name-with-slash]
> Step 2: If the command name contains at least one <slash>, the shell shall
> execute a non-built-in utility as described in 2.9.1.6 Non-built-in Utility
> Execution.
>
> Source: XCU 2.9.1.4 Command Search and Execution — utilities/V3_chap02.html#tag_19_09_01_04

### 2.9.1.5 Standard File Descriptors

> [spec:posix:req:cmd.std-fd-closed]
> If the utility would be executed with file descriptor 0, 1, or 2 closed,
> implementations may execute the utility with the file descriptor open to an
> unspecified file.
>
> Source: XCU 2.9.1.5 Standard File Descriptors — utilities/V3_chap02.html#tag_19_09_01_05

> [spec:posix:req:cmd.std-fd-nonconforming-environment]
> If a standard utility or a conforming application is executed with file
> descriptor 0 not open for reading or with file descriptor 1 or 2 not open for
> writing, the environment in which the utility or application is executed shall
> be deemed non-conforming, and consequently the utility or application might
> not behave as described in this standard.
>
> Source: XCU 2.9.1.5 Standard File Descriptors — utilities/V3_chap02.html#tag_19_09_01_05

### 2.9.1.6 Non-built-in Utility Execution

> [spec:posix:req:cmd.nonbuiltin-separate-environment]
> When the shell executes a non-built-in utility, if the execution is not being
> made via the *exec* special built-in utility, the shell shall execute the
> utility in a separate utility environment (see 2.13 Shell Execution
> Environment).
>
> Source: XCU 2.9.1.6 Non-built-in Utility Execution — utilities/V3_chap02.html#tag_19_09_01_06

> [spec:posix:req:cmd.nonbuiltin-exec-replaces-environment]
> If the execution is being made via the *exec* special built-in utility, the
> shell shall not create a separate utility environment for this execution; the
> new process image shall replace the current shell execution environment. If
> the current shell environment is a subshell environment, the new process image
> shall replace the subshell environment and the shell shall continue in the
> environment from which that subshell environment was invoked.
>
> Source: XCU 2.9.1.6 Non-built-in Utility Execution — utilities/V3_chap02.html#tag_19_09_01_06

> [spec:posix:req:cmd.nonbuiltin-path-search-execl]
> Step 1a: If the command name does not contain any <slash> characters, the
> command name shall be searched for using the *PATH* environment variable as
> described in XBD 8. Environment Variables. If the search is successful, the
> shell shall execute the utility with actions equivalent to calling the
> *execl*() function as defined in the System Interfaces volume of POSIX.1-2024
> with the *path* argument set to the pathname resulting from the search, *arg0*
> set to the command name, and the remaining *execl*() arguments set to the
> command arguments (if any) and the null terminator.
>
> Source: XCU 2.9.1.6 Non-built-in Utility Execution — utilities/V3_chap02.html#tag_19_09_01_06

> [spec:posix:req:cmd.nonbuiltin-enoexec-script]
> Step 1a (continued): If the *execl*() function fails due to an error
> equivalent to the [ENOEXEC] error defined in the System Interfaces volume of
> POSIX.1-2024, the shell shall execute a command equivalent to having a shell
> invoked with the pathname resulting from the search as its first operand, with
> any remaining arguments passed to the new shell, except that the value of
> `"$0"` in the new shell may be set to the command name. The shell may apply a
> heuristic check to determine if the file to be executed could be a script and
> may bypass this command execution if it determines that the file cannot be a
> script. In this case, it shall write an error message, and the command shall
> fail with an exit status of 126.
>
> Note: A common heuristic for rejecting files that cannot be a script is
> locating a NUL byte prior to a <newline> byte within a fixed-length prefix of
> the file. Since *sh* is required to accept input files with unlimited line
> lengths, the heuristic check cannot be based on line length.
>
> Source: XCU 2.9.1.6 Non-built-in Utility Execution — utilities/V3_chap02.html#tag_19_09_01_06

> [spec:posix:req:cmd.nonbuiltin-invalid-name-env-unspecified]
> It is unspecified whether environment variables that were passed to the shell
> when it was invoked, but were not used to initialize shell variables (see
> 2.5.3 Shell Variables) because they had invalid names, are included in the
> environment passed to *execl*() and (if *execl*() fails with an error
> equivalent to [ENOEXEC]) to the new shell. This applies both when the command
> name is found via a *PATH* search and when the command name contains at least
> one <slash>.
>
> Source: XCU 2.9.1.6 Non-built-in Utility Execution — utilities/V3_chap02.html#tag_19_09_01_06

> [spec:posix:req:cmd.nonbuiltin-path-search-unsuccessful]
> Step 1b: If the *PATH* search is unsuccessful, the command shall fail with an
> exit status of 127 and the shell shall write an error message.
>
> Source: XCU 2.9.1.6 Non-built-in Utility Execution — utilities/V3_chap02.html#tag_19_09_01_06

> [spec:posix:req:cmd.nonbuiltin-slash-execl]
> Step 2a: If the command name contains at least one <slash> and the named
> utility exists, the shell shall execute the utility with actions equivalent to
> calling the *execl*() function defined in the System Interfaces volume of
> POSIX.1-2024 with the *path* and *arg0* arguments set to the command name, and
> the remaining *execl*() arguments set to the command arguments (if any) and
> the null terminator.
>
> Source: XCU 2.9.1.6 Non-built-in Utility Execution — utilities/V3_chap02.html#tag_19_09_01_06

> [spec:posix:req:cmd.nonbuiltin-slash-enoexec-script]
> Step 2a (continued): If the *execl*() function fails due to an error
> equivalent to the [ENOEXEC] error, the shell shall execute a command
> equivalent to having a shell invoked with the command name as its first
> operand, with any remaining arguments passed to the new shell. The shell may
> apply a heuristic check to determine if the file to be executed could be a
> script and may bypass this command execution if it determines that the file
> cannot be a script. In this case, it shall write an error message, and the
> command shall fail with an exit status of 126.
>
> Note: A common heuristic for rejecting files that cannot be a script is
> locating a NUL byte prior to a <newline> byte within a fixed-length prefix of
> the file. Since *sh* is required to accept input files with unlimited line
> lengths, the heuristic check cannot be based on line length.
>
> Source: XCU 2.9.1.6 Non-built-in Utility Execution — utilities/V3_chap02.html#tag_19_09_01_06

> [spec:posix:req:cmd.nonbuiltin-slash-not-found]
> Step 2b: If the command name contains at least one <slash> and the named
> utility does not exist, the command shall fail with an exit status of 127 and
> the shell shall write an error message.
>
> Source: XCU 2.9.1.6 Non-built-in Utility Execution — utilities/V3_chap02.html#tag_19_09_01_06

## 2.9.2 Pipelines

> [spec:posix:def:cmd.pipeline-definition]
> A *pipeline* is a sequence of one or more commands separated by the control
> operator `'|'`.
>
> Source: XCU 2.9.2 Pipelines — utilities/V3_chap02.html#tag_19_09_02

> [spec:posix:req:cmd.pipeline-connects-stdio]
> For each command but the last, the shell shall connect the standard output of
> the command to the standard input of the next command as if by creating a pipe
> and passing the write end of the pipe as the standard output of the command
> and the read end of the pipe as the standard input of the next command.
>
> Source: XCU 2.9.2 Pipelines — utilities/V3_chap02.html#tag_19_09_02

> [spec:posix:syn:cmd.pipeline-format]
> The format for a pipeline is:
>
> ```
> [!] command1 [ | command2 ...]
> ```
>
> Source: XCU 2.9.2 Pipelines — utilities/V3_chap02.html#tag_19_09_02

> [spec:posix:req:cmd.pipeline-bang-subshell-separation]
> If the pipeline begins with the reserved word **!** and *command1* is a
> subshell command, the application shall ensure that the **(** operator at the
> beginning of *command1* is separated from the **!** by one or more <blank>
> characters. The behavior of the reserved word **!** immediately followed by
> the **(** operator is unspecified.
>
> Source: XCU 2.9.2 Pipelines — utilities/V3_chap02.html#tag_19_09_02

> [spec:posix:req:cmd.pipeline-assignment-precedes-redirection]
> The standard output of *command1* shall be connected to the standard input of
> *command2*. The standard input, standard output, or both of a command shall be
> considered to be assigned by the pipeline before any redirection specified by
> redirection operators that are part of the command (see 2.7 Redirection).
>
> Source: XCU 2.9.2 Pipelines — utilities/V3_chap02.html#tag_19_09_02

> [spec:posix:req:cmd.pipeline-foreground-wait]
> If the pipeline is not in the background (see 2.9.3.1 Asynchronous AND-OR
> Lists and 2.11 Job Control), the shell shall wait for the last command
> specified in the pipeline to complete, and may also wait for all commands to
> complete.
>
> Source: XCU 2.9.2 Pipelines — utilities/V3_chap02.html#tag_19_09_02

### 2.9.2 Pipelines — Exit Status

> [spec:posix:req:cmd.pipeline-exit-status]
> The exit status of a pipeline shall depend on whether or not the *pipefail*
> option (see *set*) is enabled and whether or not the pipeline begins with the
> **!** reserved word, as described in the following table. The *pipefail*
> option determines which command in the pipeline the exit status is derived
> from; the **!** reserved word causes the exit status to be the logical NOT of
> the exit status of that command.
>
> | **pipefail Enabled** | **Begins with !** | **Exit Status** |
> |:---|:---|:---|
> | no | no | The exit status of the last (rightmost) command specified in the pipeline. |
> | no | yes | Zero, if the last (rightmost) command in the pipeline returned a non-zero exit status; otherwise, 1. |
> | yes | no | Zero, if all commands in the pipeline returned an exit status of 0; otherwise, the exit status of the last (rightmost) command specified in the pipeline that returned a non-zero exit status. |
> | yes | yes | Zero, if any command in the pipeline returned a non-zero exit status; otherwise, 1. |
>
> Source: XCU 2.9.2 Pipelines — Exit Status — utilities/V3_chap02.html#tag_19_09_02_01

> [spec:posix:req:cmd.pipeline-pipefail-setting-at-start]
> The shell shall use the *pipefail* setting at the time it begins execution of
> the pipeline, not the setting at the time it sets the exit status of the
> pipeline. (For example, in `command1 | set -o pipefail` the exit status of
> `command1` has no effect on the exit status of the pipeline, even if the shell
> executes `set -o pipefail` in the current shell environment.)
>
> Source: XCU 2.9.2 Pipelines — Exit Status — utilities/V3_chap02.html#tag_19_09_02_01

## 2.9.3 Lists

> [spec:posix:def:cmd.and-or-list-definition]
> An *AND-OR list* is a sequence of one or more pipelines separated by the
> operators `"&&"` and `"||"`.
>
> Source: XCU 2.9.3 Lists — utilities/V3_chap02.html#tag_19_09_03

> [spec:posix:def:cmd.list-definition]
> A *list* is a sequence of one or more AND-OR lists separated by the operators
> `';'` and `'&'`.
>
> Source: XCU 2.9.3 Lists — utilities/V3_chap02.html#tag_19_09_03

> [spec:posix:req:cmd.and-or-precedence]
> The operators `"&&"` and `"||"` shall have equal precedence and shall be
> evaluated with left associativity. For example, both of the following commands
> write solely **bar** to standard output:
>
> ```
> false && echo foo || echo bar
> true || echo foo && echo bar
> ```
>
> Source: XCU 2.9.3 Lists — utilities/V3_chap02.html#tag_19_09_03

> [spec:posix:req:cmd.list-separator-semantics]
> A `';'` separator or a `';'` or <newline> terminator shall cause the preceding
> AND-OR list to be executed sequentially; an `'&'` separator or terminator
> shall cause asynchronous execution of the preceding AND-OR list.
>
> Source: XCU 2.9.3 Lists — utilities/V3_chap02.html#tag_19_09_03

> [spec:posix:def:cmd.compound-list-definition]
> The term "compound-list" is derived from the grammar in 2.10 Shell Grammar; it
> is equivalent to a sequence of *lists*, separated by <newline> characters,
> that can be preceded or followed by an arbitrary number of <newline>
> characters.
>
> Source: XCU 2.9.3 Lists — utilities/V3_chap02.html#tag_19_09_03

### 2.9.3.1 Asynchronous AND-OR Lists

> [spec:posix:req:cmd.async-subshell-background]
> If an AND-OR list is terminated by the control operator <ampersand> (`'&'`),
> the shell shall execute the AND-OR list asynchronously in a subshell
> environment. This subshell shall execute in the background; that is, the shell
> shall not wait for the subshell to terminate before executing the next command
> (if any); if there are no further commands to execute, the shell shall not
> wait for the subshell to terminate before exiting.
>
> Source: XCU 2.9.3.1 Asynchronous AND-OR Lists — utilities/V3_chap02.html#tag_19_09_03_02

> [spec:posix:req:cmd.async-job-number]
> If job control is enabled (see *set* **-m**), the AND-OR list shall become a
> job-control background job and a job number shall be assigned to it. If job
> control is disabled, the AND-OR list may become a non-job-control background
> job, in which case a job number shall be assigned to it; if no job number is
> assigned it shall become a background command but not a background job.
>
> Source: XCU 2.9.3.1 Asynchronous AND-OR Lists — utilities/V3_chap02.html#tag_19_09_03_02

> [spec:posix:sem:cmd.async-job-control]
> A job-control background job can be controlled as described in 2.11 Job
> Control.
>
> Source: XCU 2.9.3.1 Asynchronous AND-OR Lists — utilities/V3_chap02.html#tag_19_09_03_02

> [spec:posix:req:cmd.async-process-id-known]
> The process ID associated with the asynchronous AND-OR list shall become known
> in the current shell execution environment; see 2.13 Shell Execution
> Environment. This process ID shall remain known until any one of the following
> occurs (and, unless otherwise specified, may continue to remain known after it
> occurs).
>
> - The process terminates and the application waits for the process ID or the
>   corresponding job ID (see *wait*).
> - If the asynchronous AND-OR list did not become a background job: another
>   asynchronous AND-OR list is invoked before `"$!"` (corresponding to the
>   previous asynchronous AND-OR list) is expanded in the current shell
>   execution environment.
> - If the asynchronous AND-OR list became a background job: the *jobs* utility
>   reports the termination status of that job.
> - If the shell is interactive and the asynchronous AND-OR list became a
>   background job: a message indicating completion of the corresponding job is
>   written to standard error. If *set* **-b** is enabled, it is unspecified
>   whether the process ID is removed from the list of known process IDs when
>   the message is written or immediately prior to when the shell writes the
>   next prompt for input.
>
> Source: XCU 2.9.3.1 Asynchronous AND-OR Lists — utilities/V3_chap02.html#tag_19_09_03_02

> [spec:posix:req:cmd.async-known-pid-retention]
> The implementation need not retain more than the {CHILD_MAX} most recent
> entries in its list of known process IDs in the current shell execution
> environment.
>
> Source: XCU 2.9.3.1 Asynchronous AND-OR Lists — utilities/V3_chap02.html#tag_19_09_03_02

> [spec:posix:req:cmd.async-stdin-devnull]
> If, and only if, job control is disabled, the standard input for the subshell
> in which an asynchronous AND-OR list is executed shall initially be assigned
> to an open file description that behaves as if **/dev/null** had been opened
> for reading only. This initial assignment shall be overridden by any explicit
> redirection of standard input within the AND-OR list.
>
> Source: XCU 2.9.3.1 Asynchronous AND-OR Lists — utilities/V3_chap02.html#tag_19_09_03_02

> [spec:posix:req:cmd.async-job-notification-format]
> If the shell is interactive and the asynchronous AND-OR list became a
> background job, the job number and the process ID associated with the job
> shall be written to standard error using the format:
>
> ```
> "[%d] %d\n", <job-number>, <process-id>
> ```
>
> Source: XCU 2.9.3.1 Asynchronous AND-OR Lists — utilities/V3_chap02.html#tag_19_09_03_02

> [spec:posix:req:cmd.async-non-job-pid-message]
> If the shell is interactive and the asynchronous AND-OR list did not become a
> background job, the process ID associated with the asynchronous AND-OR list
> shall be written to standard error in an unspecified format.
>
> Source: XCU 2.9.3.1 Asynchronous AND-OR Lists — utilities/V3_chap02.html#tag_19_09_03_02

### 2.9.3.1 Asynchronous AND-OR Lists — Exit Status

> [spec:posix:req:cmd.async-exit-status]
> The exit status of an asynchronous AND-OR list shall be zero.
>
> Source: XCU 2.9.3.1 Asynchronous AND-OR Lists — Exit Status — utilities/V3_chap02.html#tag_19_09_03_03

> [spec:posix:sem:cmd.async-status-via-wait]
> The exit status of the subshell in which the AND-OR list is asynchronously
> executed can be obtained using the *wait* utility.
>
> Source: XCU 2.9.3.1 Asynchronous AND-OR Lists — Exit Status — utilities/V3_chap02.html#tag_19_09_03_03

### 2.9.3.2 Sequential AND-OR Lists

> [spec:posix:req:cmd.sequential-execution]
> AND-OR lists that are separated by a <semicolon> (`';'`) shall be executed
> sequentially. The format for executing AND-OR lists sequentially shall be:
>
> ```
> aolist1 [; aolist2] ...
> ```
>
> Each AND-OR list shall be expanded and executed in the order specified.
>
> Source: XCU 2.9.3.2 Sequential AND-OR Lists — utilities/V3_chap02.html#tag_19_09_03_04

> [spec:posix:req:cmd.sequential-foreground-job]
> If job control is enabled, the AND-OR lists shall form all or part of a
> foreground job that can be controlled as described in 2.11 Job Control.
>
> Source: XCU 2.9.3.2 Sequential AND-OR Lists — utilities/V3_chap02.html#tag_19_09_03_04

### 2.9.3.2 Sequential AND-OR Lists — Exit Status

> [spec:posix:req:cmd.sequential-exit-status]
> The exit status of a sequential AND-OR list shall be the exit status of the
> last pipeline in the AND-OR list that is executed.
>
> Source: XCU 2.9.3.2 Sequential AND-OR Lists — Exit Status — utilities/V3_chap02.html#tag_19_09_03_05

### 2.9.3.3 AND Lists

> [spec:posix:syn:cmd.and-list-format]
> The control operator `"&&"` denotes an AND list. The format shall be:
>
> ```
> command1 [ && command2] ...
> ```
>
> Source: XCU 2.9.3.3 AND Lists — utilities/V3_chap02.html#tag_19_09_03_06

> [spec:posix:req:cmd.and-list-execution]
> First *command1* shall be executed. If its exit status is zero, *command2*
> shall be executed, and so on, until a command has a non-zero exit status or
> there are no more commands left to execute. The commands are expanded only if
> they are executed.
>
> Source: XCU 2.9.3.3 AND Lists — utilities/V3_chap02.html#tag_19_09_03_06

### 2.9.3.3 AND Lists — Exit Status

> [spec:posix:req:cmd.and-list-exit-status]
> The exit status of an AND list shall be the exit status of the last command
> that is executed in the list.
>
> Source: XCU 2.9.3.3 AND Lists — Exit Status — utilities/V3_chap02.html#tag_19_09_03_07

### 2.9.3.4 OR Lists

> [spec:posix:syn:cmd.or-list-format]
> The control operator `"||"` denotes an OR List. The format shall be:
>
> ```
> command1 [ || command2] ...
> ```
>
> Source: XCU 2.9.3.4 OR Lists — utilities/V3_chap02.html#tag_19_09_03_08

> [spec:posix:req:cmd.or-list-execution]
> First, *command1* shall be executed. If its exit status is non-zero,
> *command2* shall be executed, and so on, until a command has a zero exit
> status or there are no more commands left to execute.
>
> Source: XCU 2.9.3.4 OR Lists — utilities/V3_chap02.html#tag_19_09_03_08

### 2.9.3.4 OR Lists — Exit Status

> [spec:posix:req:cmd.or-list-exit-status]
> The exit status of an OR list shall be the exit status of the last command
> that is executed in the list.
>
> Source: XCU 2.9.3.4 OR Lists — Exit Status — utilities/V3_chap02.html#tag_19_09_03_09

## 2.9.4 Compound Commands

> [spec:posix:def:cmd.compound-definition]
> The shell has several programming constructs that are "compound commands",
> which provide control flow for commands. Each of these compound commands has a
> reserved word or control operator at the beginning, and a corresponding
> terminator reserved word or operator at the end. In addition, each can be
> followed by redirections on the same line as the terminator.
>
> Source: XCU 2.9.4 Compound Commands — utilities/V3_chap02.html#tag_19_09_04

> [spec:posix:req:cmd.compound-redirection-scope]
> Each redirection on a compound command shall apply to all the commands within
> the compound command that do not explicitly override that redirection.
>
> Source: XCU 2.9.4 Compound Commands — utilities/V3_chap02.html#tag_19_09_04

> [spec:posix:req:cmd.compound-list-exit-status]
> Where the exit status of a compound command is stated in terms of the exit
> status of a *compound-list*, the exit status of that *compound-list* shall be
> the value that the special parameter `'?'` (see 2.5.2 Special Parameters)
> would have immediately after execution of the *compound-list*.
>
> Source: XCU 2.9.4 Compound Commands — utilities/V3_chap02.html#tag_19_09_04

### 2.9.4.1 Grouping Commands

> [spec:posix:req:cmd.group-subshell]
> The format for the subshell grouping command is `( compound-list )`. Execute
> *compound-list* in a subshell environment; see 2.13 Shell Execution
> Environment. Variable assignments and built-in commands that affect the
> environment shall not remain in effect after the list finishes.
>
> Source: XCU 2.9.4.1 Grouping Commands — utilities/V3_chap02.html#tag_19_09_04_01

> [spec:posix:req:cmd.group-double-paren-ambiguity]
> If a character sequence beginning with `"(("` would be parsed by the shell as
> an arithmetic expansion if preceded by a `'$'`, shells which implement an
> extension whereby `"((expression))"` is evaluated as an arithmetic expression
> may treat the `"(("` as introducing an arithmetic evaluation instead of a
> grouping command. A conforming application shall ensure that it separates the
> two leading `'('` characters with white space to prevent the shell from
> performing an arithmetic evaluation.
>
> Source: XCU 2.9.4.1 Grouping Commands — utilities/V3_chap02.html#tag_19_09_04_01

> [spec:posix:sem:cmd.group-brace-current-environment]
> The format for the brace grouping command is `{ compound-list ; }`. Execute
> *compound-list* in the current process environment. The semicolon shown here
> is an example of a control operator delimiting the **}** reserved word. Other
> delimiters are possible, as shown in 2.10 Shell Grammar; a <newline> is
> frequently used.
>
> Source: XCU 2.9.4.1 Grouping Commands — utilities/V3_chap02.html#tag_19_09_04_01

### 2.9.4.1 Grouping Commands — Exit Status

> [spec:posix:req:cmd.group-exit-status]
> The exit status of a grouping command shall be the exit status of
> *compound-list*.
>
> Source: XCU 2.9.4.1 Grouping Commands — Exit Status — utilities/V3_chap02.html#tag_19_09_04_02

### 2.9.4.2 The for Loop

> [spec:posix:req:cmd.for-do-done-delimiters]
> The **for** loop shall execute a sequence of commands for each member in a
> list of *items*. The **for** loop requires that the reserved words **do** and
> **done** be used to delimit the sequence of commands.
>
> Source: XCU 2.9.4.2 The for Loop — utilities/V3_chap02.html#tag_19_09_04_03

> [spec:posix:syn:cmd.for-format]
> The format for the **for** loop is as follows:
>
> ```
> for name [ in [word ... ]]
> do
>     compound-list
> done
> ```
>
> Source: XCU 2.9.4.2 The for Loop — utilities/V3_chap02.html#tag_19_09_04_03

> [spec:posix:req:cmd.for-iteration]
> First, the list of words following **in** shall be expanded to generate a list
> of items. Then, the variable *name* shall be set to each item, in turn, and
> the *compound-list* executed each time. If no items result from the expansion,
> the *compound-list* shall not be executed.
>
> Source: XCU 2.9.4.2 The for Loop — utilities/V3_chap02.html#tag_19_09_04_03

> [spec:posix:req:cmd.for-omitted-in]
> Omitting `in word ...` shall be equivalent to `in "$@"`.
>
> Source: XCU 2.9.4.2 The for Loop — utilities/V3_chap02.html#tag_19_09_04_03

### 2.9.4.2 The for Loop — Exit Status

> [spec:posix:req:cmd.for-exit-status]
> If there is at least one item in the list of items, the exit status of a
> **for** command shall be the exit status of the last *compound-list* executed.
> If there are no items, the exit status shall be zero.
>
> Source: XCU 2.9.4.2 The for Loop — Exit Status — utilities/V3_chap02.html#tag_19_09_04_04

### 2.9.4.3 Case Conditional Construct

> [spec:posix:req:cmd.case-selection]
> The conditional construct **case** shall execute the *compound-list*
> corresponding to the first *pattern* (see 2.14 Pattern Matching Notation), if
> any are present, that is matched by the string resulting from the tilde
> expansion, parameter expansion, command substitution, arithmetic expansion,
> and quote removal of the given word.
>
> Source: XCU 2.9.4.3 Case Conditional Construct — utilities/V3_chap02.html#tag_19_09_04_05

> [spec:posix:syn:cmd.case-clause-syntax]
> The reserved word **in** shall denote the beginning of the patterns to be
> matched. Multiple patterns with the same *compound-list* shall be delimited by
> the `'|'` symbol. The control operator `')'` terminates a list of patterns
> corresponding to a given action. The terminated pattern list and the following
> *compound-list* is called a **case** statement *clause*. Each **case**
> statement clause, with the possible exception of the last, shall be terminated
> with either `";;"` or `";&"`. The **case** construct terminates with the
> reserved word **esac** (**case** reversed).
>
> Source: XCU 2.9.4.3 Case Conditional Construct — utilities/V3_chap02.html#tag_19_09_04_05

> [spec:posix:syn:cmd.case-format]
> The format for the **case** construct is as follows:
>
> ```
> case word in
>     [[(] pattern[ | pattern] ... ) compound-list terminator] ...
>     [[(] pattern[ | pattern] ... ) compound-list]
> esac
> ```
>
> Where *terminator* is either `";;"` or `";&"` and is optional for the last
> *compound-list*.
>
> Source: XCU 2.9.4.3 Case Conditional Construct — utilities/V3_chap02.html#tag_19_09_04_05

> [spec:posix:req:cmd.case-pattern-expansion]
> In order from the beginning to the end of the **case** statement, each
> *pattern* that labels a *compound-list* shall be subjected to tilde expansion,
> parameter expansion, command substitution, and arithmetic expansion, and the
> result of these expansions shall be compared against the expansion of *word*,
> according to the rules described in 2.14 Pattern Matching Notation (which also
> describes the effect of quoting parts of the pattern). After the first match,
> no more patterns in the **case** statement shall be expanded, and the
> *compound-list* of the matching clause shall be executed.
>
> Source: XCU 2.9.4.3 Case Conditional Construct — utilities/V3_chap02.html#tag_19_09_04_05

> [spec:posix:req:cmd.case-clause-terminators]
> If the **case** statement clause is terminated by `";;"`, no further clauses
> shall be examined. If the **case** statement clause is terminated by `";&"`,
> then the *compound-list* (if any) of each subsequent clause shall be executed,
> in order, until either a clause terminated by `";;"` is reached and its
> *compound-list* (if any) executed or there are no further clauses in the
> **case** statement.
>
> Source: XCU 2.9.4.3 Case Conditional Construct — utilities/V3_chap02.html#tag_19_09_04_05

> [spec:posix:req:cmd.case-multiple-pattern-order-unspecified]
> The order of expansion and comparison of multiple *pattern*s that label a
> *compound-list* statement is unspecified.
>
> Source: XCU 2.9.4.3 Case Conditional Construct — utilities/V3_chap02.html#tag_19_09_04_05

### 2.9.4.3 Case Conditional Construct — Exit Status

> [spec:posix:req:cmd.case-exit-status]
> The exit status of **case** shall be zero if no patterns are matched.
> Otherwise, the exit status shall be the exit status of the *compound-list* of
> the last clause to be executed.
>
> Source: XCU 2.9.4.3 Case Conditional Construct — Exit Status — utilities/V3_chap02.html#tag_19_09_04_06

### 2.9.4.4 The if Conditional Construct

> [spec:posix:syn:cmd.if-format]
> The format for the **if** construct is as follows:
>
> ```
> if compound-list
> then
>     compound-list
> [elif compound-list
> then
>     compound-list] ...
> [else
>     compound-list]
> fi
> ```
>
> Source: XCU 2.9.4.4 The if Conditional Construct — utilities/V3_chap02.html#tag_19_09_04_07

> [spec:posix:req:cmd.if-execution]
> The **if** command shall execute a *compound-list* and use its exit status to
> determine whether to execute another *compound-list*.
>
> The **if** *compound-list* shall be executed; if its exit status is zero, the
> **then** *compound-list* shall be executed and the command shall complete.
> Otherwise, each **elif** *compound-list* shall be executed, in turn, and if
> its exit status is zero, the **then** *compound-list* shall be executed and
> the command shall complete. Otherwise, the **else** *compound-list* shall be
> executed.
>
> Source: XCU 2.9.4.4 The if Conditional Construct — utilities/V3_chap02.html#tag_19_09_04_07

### 2.9.4.4 The if Conditional Construct — Exit Status

> [spec:posix:req:cmd.if-exit-status]
> The exit status of the **if** command shall be the exit status of the **then**
> or **else** *compound-list* that was executed, or zero, if none was executed.
>
> Note: Although the exit status of the **if** or **elif** *compound-list* is
> ignored when determining the exit status of the **if** command, it is
> available through the special parameter `'?'` (see 2.5.2 Special Parameters)
> during execution of the next **then**, **elif**, or **else** *compound-list*
> (if any is executed) in the normal way.
>
> Source: XCU 2.9.4.4 The if Conditional Construct — Exit Status — utilities/V3_chap02.html#tag_19_09_04_08

### 2.9.4.5 The while Loop

> [spec:posix:syn:cmd.while-format]
> The format of the **while** loop is as follows:
>
> ```
> while compound-list-1
> do
>     compound-list-2
> done
> ```
>
> Source: XCU 2.9.4.5 The while Loop — utilities/V3_chap02.html#tag_19_09_04_09

> [spec:posix:req:cmd.while-execution]
> The **while** loop shall continuously execute one *compound-list* as long as
> another *compound-list* has a zero exit status.
>
> The *compound-list-1* shall be executed, and if it has a non-zero exit status,
> the **while** command shall complete. Otherwise, the *compound-list-2* shall
> be executed, and the process shall repeat.
>
> Source: XCU 2.9.4.5 The while Loop — utilities/V3_chap02.html#tag_19_09_04_09

### 2.9.4.5 The while Loop — Exit Status

> [spec:posix:req:cmd.while-exit-status]
> The exit status of the **while** loop shall be the exit status of the last
> *compound-list-2* executed, or zero if none was executed.
>
> Note: Since the exit status of *compound-list-1* is ignored when determining
> the exit status of the **while** command, it is not possible to obtain the
> status of the command that caused the loop to exit, other than via the special
> parameter `'?'` (see 2.5.2 Special Parameters) during execution of
> *compound-list-1*. The exit status of *compound-list-1* is available through
> the special parameter `'?'` during execution of *compound-list-2*, but is
> known to be zero at that point anyway.
>
> Source: XCU 2.9.4.5 The while Loop — Exit Status — utilities/V3_chap02.html#tag_19_09_04_10

### 2.9.4.6 The until Loop

> [spec:posix:syn:cmd.until-format]
> The format of the **until** loop is as follows:
>
> ```
> until compound-list-1
> do
>     compound-list-2
> done
> ```
>
> Source: XCU 2.9.4.6 The until Loop — utilities/V3_chap02.html#tag_19_09_04_11

> [spec:posix:req:cmd.until-execution]
> The **until** loop shall continuously execute one *compound-list* as long as
> another *compound-list* has a non-zero exit status.
>
> The *compound-list-1* shall be executed, and if it has a zero exit status, the
> **until** command completes. Otherwise, the *compound-list-2* shall be
> executed, and the process repeats.
>
> Source: XCU 2.9.4.6 The until Loop — utilities/V3_chap02.html#tag_19_09_04_11

### 2.9.4.6 The until Loop — Exit Status

> [spec:posix:req:cmd.until-exit-status]
> The exit status of the **until** loop shall be the exit status of the last
> *compound-list-2* executed, or zero if none was executed.
>
> Note: Although the exit status of *compound-list-1* is ignored when
> determining the exit status of the **until** command, it is available through
> the special parameter `'?'` (see 2.5.2 Special Parameters) during execution of
> *compound-list-2* in the normal way.
>
> Source: XCU 2.9.4.6 The until Loop — Exit Status — utilities/V3_chap02.html#tag_19_09_04_12

## 2.9.5 Function Definition Command

> [spec:posix:def:cmd.function-definition-term]
> A function is a user-defined name that is used as a simple command to call a
> compound command with new positional parameters. A function is defined with a
> "function definition command".
>
> Source: XCU 2.9.5 Function Definition Command — utilities/V3_chap02.html#tag_19_09_05

> [spec:posix:syn:cmd.function-format]
> The format of a function definition command is as follows:
>
> ```
> fname ( ) compound-command [io-redirect ...]
> ```
>
> The argument *compound-command* represents a compound command, as described in
> 2.9.4 Compound Commands.
>
> Source: XCU 2.9.5 Function Definition Command — utilities/V3_chap02.html#tag_19_09_05

> [spec:posix:req:cmd.function-name-requirements]
> The function is named *fname*; the application shall ensure that it is a name
> (see XBD 3.216 Name) and that it is not the name of a special built-in
> utility. An implementation may allow other characters in a function name as an
> extension. The implementation shall maintain separate name spaces for
> functions and variables.
>
> Source: XCU 2.9.5 Function Definition Command — utilities/V3_chap02.html#tag_19_09_05

> [spec:posix:req:cmd.function-no-expansion-at-definition]
> When the function is declared, none of the expansions in 2.6 Word Expansions
> shall be performed on the text in *compound-command* or *io-redirect*; all
> expansions shall be performed as normal each time the function is called.
> Similarly, the optional *io-redirect* redirections and any variable
> assignments within *compound-command* shall be performed during the execution
> of the function itself, not the function definition. See 2.8.1 Consequences of
> Shell Errors for the consequences of failures of these operations on
> interactive and non-interactive shells.
>
> Source: XCU 2.9.5 Function Definition Command — utilities/V3_chap02.html#tag_19_09_05

> [spec:posix:req:cmd.function-syntax-error-properties]
> When a function is executed, it shall have the syntax-error properties
> described for special built-in utilities in the first item in the enumerated
> list at the beginning of 2.15 Special Built-In Utilities.
>
> Source: XCU 2.9.5 Function Definition Command — utilities/V3_chap02.html#tag_19_09_05

> [spec:posix:req:cmd.function-invocation-positional-parameters]
> The *compound-command* shall be executed whenever the function name is
> specified as the name of a simple command (see 2.9.1.4 Command Search and
> Execution). The operands to the command temporarily shall become the
> positional parameters during the execution of the *compound-command*; the
> special parameter `'#'` also shall be changed to reflect the number of
> operands. The special parameter 0 shall be unchanged. When the function
> completes, the values of the positional parameters and the special parameter
> `'#'` shall be restored to the values they had before the function was
> executed.
>
> Source: XCU 2.9.5 Function Definition Command — utilities/V3_chap02.html#tag_19_09_05

> [spec:posix:req:cmd.function-return]
> If the special built-in *return* is executed in the *compound-command*, the
> function completes and execution shall resume with the next command after the
> function call.
>
> Source: XCU 2.9.5 Function Definition Command — utilities/V3_chap02.html#tag_19_09_05

### 2.9.5 Function Definition Command — Exit Status

> [spec:posix:req:cmd.function-exit-status]
> The exit status of a function definition shall be zero if the function was
> declared successfully; otherwise, it shall be greater than zero. The exit
> status of a function invocation shall be the exit status of the last command
> executed by the function.
>
> Source: XCU 2.9.5 Function Definition Command — Exit Status — utilities/V3_chap02.html#tag_19_09_05_01
