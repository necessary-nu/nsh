# Intrinsic Utilities: command, type, and hash

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
"SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in RFC 2119. This document
reproduces normative text from IEEE Std 1003.1-2024 (POSIX.1-2024),
Copyright © 2001-2024 The IEEE and The Open Group.

Option-conditional text carries the standard's own margin code inline, at the point the standard shades it:

`[XSI]`
: X/Open System Interfaces. The functionality described is an extension, available on all systems supporting the XSI option.

## command

### SYNOPSIS

> [spec:posix:syn:builtin.command.synopsis]
> The synopsis of the command utility is:
>
> `command [-p] command_name [argument...]`
>
> `command [-p][-v|-V] command_name`
>
> Source: XCU command SYNOPSIS — utilities/command.html#tag_20_22_02

### DESCRIPTION

> [spec:posix:req:builtin.command.suppress-function-lookup]
> The command utility shall cause the shell to treat the arguments as a simple
> command, suppressing the shell function lookup that is described in 2.9.1.4
> Command Search and Execution, item 1c.
>
> Source: XCU command DESCRIPTION — utilities/command.html#tag_20_22_03

> [spec:posix:req:builtin.command.special-builtin-properties-suppressed]
> If the command_name is the same as the name of one of the special built-in
> utilities, the special properties in the enumerated list at the beginning of
> 2.15 Special Built-In Utilities shall not occur.
>
> Source: XCU command DESCRIPTION — utilities/command.html#tag_20_22_03

> [spec:posix:req:builtin.command.equivalent-to-omitting-command]
> In every other respect, if command_name is not the name of a function, the
> effect of command (with no options) shall be the same as omitting command,
> except that command_name does not appear in the command word position in the
> command command, and consequently is not subject to alias substitution (see
> 2.3.1 Alias Substitution) nor recognized as a reserved word (see 2.4 Reserved
> Words).
>
> Source: XCU command DESCRIPTION — utilities/command.html#tag_20_22_03

> [spec:posix:req:builtin.command.v-options-report-interpretation]
> When the **-v** or **-V** option is used, the command utility shall provide
> information concerning how a command name is interpreted by the shell.
>
> Source: XCU command DESCRIPTION — utilities/command.html#tag_20_22_03

> [spec:posix:req:builtin.command.declaration-utility]
> The command utility shall be treated as a declaration utility if the first
> argument passed to the utility is recognized as a declaration utility. In this
> case, subsequent words of the form name=word shall be expanded in an
> assignment context. See 2.9.1.1 Order of Processing.
>
> Source: XCU command DESCRIPTION — utilities/command.html#tag_20_22_03

### OPTIONS

> [spec:posix:req:builtin.command.utility-syntax-guidelines]
> The command utility shall conform to XBD 12.2 Utility Syntax Guidelines.
>
> The following options shall be supported.
>
> Source: XCU command OPTIONS — utilities/command.html#tag_20_22_04

> [spec:posix:req:builtin.command.opt-p]
> **-p**: Perform the command search using a default value for PATH that is
> guaranteed to find all of the standard utilities.
>
> Source: XCU command OPTIONS — utilities/command.html#tag_20_22_04

> [spec:posix:req:builtin.command.opt-v]
> **-v**: Write a string to standard output that indicates the pathname or
> command that will be used by the shell, in the current shell execution
> environment (see 2.13 Shell Execution Environment), to invoke command_name,
> but do not invoke command_name.
>
> - Executable utilities, regular built-in utilities, command_names including a
>   <slash> character, and any implementation-provided functions that are found
>   using the PATH variable (as described in 2.9.1.4 Command Search and
>   Execution), shall be written as absolute pathnames.
> - Shell functions, special built-in utilities, regular built-in utilities not
>   associated with a PATH search, and shell reserved words shall be written as
>   just their names.
> - An alias shall be written as a command line that represents its alias
>   definition.
> - Otherwise, no output shall be written and the exit status shall reflect that
>   the name was not found.
>
> Source: XCU command OPTIONS — utilities/command.html#tag_20_22_04

> [spec:posix:req:builtin.command.opt-v-uppercase]
> **-V**: Write a string to standard output that indicates how the name given in
> the command_name operand will be interpreted by the shell, in the current
> shell execution environment (see 2.13 Shell Execution Environment), but do not
> invoke command_name. Although the format of this string is unspecified, it
> shall indicate in which of the following categories command_name falls and
> shall include the information stated:
>
> - Executable utilities, regular built-in utilities, and any
>   implementation-provided functions that are found using the PATH variable (as
>   described in 2.9.1.4 Command Search and Execution), shall be identified as
>   such and include the absolute pathname in the string.
> - Other shell functions shall be identified as functions.
> - Aliases shall be identified as aliases and their definitions included in the
>   string.
> - Special built-in utilities shall be identified as special built-in
>   utilities.
> - Regular built-in utilities not associated with a PATH search shall be
>   identified as regular built-in utilities. (The term "regular" need not be
>   used.)
> - Shell reserved words shall be identified as reserved words.
>
> Source: XCU command OPTIONS — utilities/command.html#tag_20_22_04

### OPERANDS

> [spec:posix:def:builtin.command.operands]
> The following operands shall be supported:
>
> argument — One of the strings treated as an argument to command_name.
>
> command_name — The name of a utility or a special built-in utility.
>
> Source: XCU command OPERANDS — utilities/command.html#tag_20_22_05

### ENVIRONMENT VARIABLES

> [spec:posix:req:builtin.command.env-locale]
> The following environment variables shall affect the execution of command:
>
> LANG provides a default value for the internationalization variables that are
> unset or null. (See XBD 8.2 Internationalization Variables for the precedence
> of internationalization variables used to determine the values of locale
> categories.)
>
> LC_ALL, if set to a non-empty string value, overrides the values of all the
> other internationalization variables.
>
> LC_CTYPE determines the locale for the interpretation of sequences of bytes of
> text data as characters (for example, single-byte as opposed to multi-byte
> characters in arguments).
>
> LC_MESSAGES determines the locale that should be used to affect the format and
> contents of diagnostic messages written to standard error and informative
> messages written to standard output.
>
> Source: XCU command ENVIRONMENT VARIABLES — utilities/command.html#tag_20_22_08

> [spec:posix:sem:builtin.command.env-nlspath]
> `[XSI]` NLSPATH determines the location of messages objects and message
> catalogs.
>
> Source: XCU command ENVIRONMENT VARIABLES — utilities/command.html#tag_20_22_08

> [spec:posix:sem:builtin.command.env-path]
> PATH determines the search path used during the command search described in
> 2.9.1.4 Command Search and Execution, except as described under the **-p**
> option.
>
> Source: XCU command ENVIRONMENT VARIABLES — utilities/command.html#tag_20_22_08

### STDOUT

> [spec:posix:req:builtin.command.stdout-format]
> When the **-v** option is specified, standard output shall be formatted as:
>
> `"%s\n", <pathname or command>`
>
> When the **-V** option is specified, standard output shall be formatted as:
>
> `"%s\n", <unspecified>`
>
> Source: XCU command STDOUT — utilities/command.html#tag_20_22_10

### STDERR

> [spec:posix:req:builtin.command.stderr]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU command STDERR — utilities/command.html#tag_20_22_11

### Other interfaces

> [spec:posix:req:builtin.command.interfaces]
> Standard input is not used; there are no input files; asynchronous events are
> handled as for the utility description defaults; there are no output files;
> there is no extended description; and the consequences of errors are as for
> the utility description defaults.
>
> Source: XCU command — utilities/command.html#tag_20_22

### EXIT STATUS

> [spec:posix:req:builtin.command.exit-status-v-options]
> When the **-v** or **-V** options are specified, the following exit values
> shall be returned:
>
> | Exit status | Meaning |
> |---|---|
> | 0 | Successful completion. |
> | >0 | The command_name could not be found or an error occurred. |
>
> Source: XCU command EXIT STATUS — utilities/command.html#tag_20_22_14

> [spec:posix:req:builtin.command.exit-status-invocation]
> Otherwise, the following exit values shall be returned:
>
> | Exit status | Meaning |
> |---|---|
> | 126 | The utility specified by command_name was found but could not be invoked. |
> | 127 | An error occurred in the command utility or the utility specified by command_name could not be found. |
>
> Otherwise, the exit status of command shall be that of the simple command
> specified by the arguments to command.
>
> Source: XCU command EXIT STATUS — utilities/command.html#tag_20_22_14

## type

### SYNOPSIS

> [spec:posix:syn:builtin.type.synopsis]
> `[XSI]` The synopsis of the type utility is `type name...`.
>
> Source: XCU type SYNOPSIS — utilities/type.html#tag_20_130_02

### DESCRIPTION

> [spec:posix:req:builtin.type.indicate-interpretation]
> The type utility shall indicate how each argument would be interpreted if used
> as a command name.
>
> Source: XCU type DESCRIPTION — utilities/type.html#tag_20_130_03

### OPERANDS

> [spec:posix:def:builtin.type.operand-name]
> The following operand shall be supported:
>
> name — A name to be interpreted.
>
> Source: XCU type OPERANDS — utilities/type.html#tag_20_130_05

### ENVIRONMENT VARIABLES

> [spec:posix:req:builtin.type.env-locale]
> The following environment variables shall affect the execution of type:
>
> LANG provides a default value for the internationalization variables that are
> unset or null. (See XBD 8.2 Internationalization Variables for the precedence
> of internationalization variables used to determine the values of locale
> categories.)
>
> LC_ALL, if set to a non-empty string value, overrides the values of all the
> other internationalization variables.
>
> LC_CTYPE determines the locale for the interpretation of sequences of bytes of
> text data as characters (for example, single-byte as opposed to multi-byte
> characters in arguments).
>
> LC_MESSAGES determines the locale that should be used to affect the format and
> contents of diagnostic messages written to standard error.
>
> Source: XCU type ENVIRONMENT VARIABLES — utilities/type.html#tag_20_130_08

> [spec:posix:sem:builtin.type.env-nlspath]
> NLSPATH determines the location of messages objects and message catalogs.
>
> Source: XCU type ENVIRONMENT VARIABLES — utilities/type.html#tag_20_130_08

> [spec:posix:sem:builtin.type.env-path]
> PATH determines the location of name, as described in XBD 8. Environment
> Variables.
>
> Source: XCU type ENVIRONMENT VARIABLES — utilities/type.html#tag_20_130_08

### STDOUT

> [spec:posix:sem:builtin.type.stdout]
> The standard output of type contains information about each operand in an
> unspecified format. The information provided typically identifies the operand
> as a shell built-in, function, alias, or keyword, and where applicable, may
> display the operand's pathname.
>
> Source: XCU type STDOUT — utilities/type.html#tag_20_130_10

### STDERR

> [spec:posix:req:builtin.type.stderr]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU type STDERR — utilities/type.html#tag_20_130_11

### Other interfaces

> [spec:posix:req:builtin.type.interfaces]
> The type utility has no options. Standard input is not used; there are no
> input files; asynchronous events are handled as for the utility description
> defaults; there are no output files; there is no extended description; and the
> consequences of errors are as for the utility description defaults.
>
> Source: XCU type — utilities/type.html#tag_20_130

### EXIT STATUS

> [spec:posix:req:builtin.type.exit-status]
> The following exit values shall be returned:
>
> | Exit status | Meaning |
> |---|---|
> | 0 | Successful completion. |
> | >0 | An error occurred. |
>
> Source: XCU type EXIT STATUS — utilities/type.html#tag_20_130_14

## hash

### SYNOPSIS

> [spec:posix:syn:builtin.hash.synopsis]
> The synopsis of the hash utility is:
>
> `hash [utility...]`
>
> `hash -r`
>
> Source: XCU hash SYNOPSIS — utilities/hash.html#tag_20_56_02

### DESCRIPTION

> [spec:posix:req:builtin.hash.remembered-locations]
> The hash utility shall affect the way the current shell environment remembers
> the locations of utilities found as described in 2.9.1.4 Command Search and
> Execution. Depending on the arguments specified, it shall add utility
> locations to its list of remembered locations or it shall purge the contents
> of the list. When no arguments are specified, it shall report on the contents
> of the list.
>
> Source: XCU hash DESCRIPTION — utilities/hash.html#tag_20_56_03

> [spec:posix:req:builtin.hash.builtins-and-functions-not-reported]
> Utilities provided as built-ins to the shell and functions shall not be
> reported by hash.
>
> Source: XCU hash DESCRIPTION — utilities/hash.html#tag_20_56_03

### OPTIONS

> [spec:posix:req:builtin.hash.utility-syntax-guidelines]
> The hash utility shall conform to XBD 12.2 Utility Syntax Guidelines.
>
> The following option shall be supported.
>
> Source: XCU hash OPTIONS — utilities/hash.html#tag_20_56_04

> [spec:posix:req:builtin.hash.opt-r]
> **-r**: Forget all previously remembered utility locations.
>
> Source: XCU hash OPTIONS — utilities/hash.html#tag_20_56_04

### OPERANDS

> [spec:posix:def:builtin.hash.operand-utility]
> The following operand shall be supported:
>
> utility — The name of a utility to be searched for and added to the list of
> remembered locations.
>
> Source: XCU hash OPERANDS — utilities/hash.html#tag_20_56_05

> [spec:posix:sem:builtin.hash.operand-utility-unspecified]
> If the search does not find utility, it is unspecified whether or not this is
> treated as an error. If utility contains one or more <slash> characters, the
> results are unspecified.
>
> Source: XCU hash OPERANDS — utilities/hash.html#tag_20_56_05

### ENVIRONMENT VARIABLES

> [spec:posix:req:builtin.hash.env-locale]
> The following environment variables shall affect the execution of hash:
>
> LANG provides a default value for the internationalization variables that are
> unset or null. (See XBD 8.2 Internationalization Variables for the precedence
> of internationalization variables used to determine the values of locale
> categories.)
>
> LC_ALL, if set to a non-empty string value, overrides the values of all the
> other internationalization variables.
>
> LC_CTYPE determines the locale for the interpretation of sequences of bytes of
> text data as characters (for example, single-byte as opposed to multi-byte
> characters in arguments).
>
> LC_MESSAGES determines the locale that should be used to affect the format and
> contents of diagnostic messages written to standard error.
>
> Source: XCU hash ENVIRONMENT VARIABLES — utilities/hash.html#tag_20_56_08

> [spec:posix:sem:builtin.hash.env-nlspath]
> `[XSI]` NLSPATH determines the location of messages objects and message
> catalogs.
>
> Source: XCU hash ENVIRONMENT VARIABLES — utilities/hash.html#tag_20_56_08

> [spec:posix:sem:builtin.hash.env-path]
> PATH determines the location of utility, as described in XBD 8. Environment
> Variables.
>
> Source: XCU hash ENVIRONMENT VARIABLES — utilities/hash.html#tag_20_56_08

### STDOUT

> [spec:posix:req:builtin.hash.stdout-report]
> The standard output of hash shall be used when no arguments are specified. Its
> format is unspecified, but includes the pathname of each utility in the list
> of remembered locations for the current shell environment. This list shall
> consist of those utilities named in previous hash invocations that have been
> invoked, and may contain those invoked and found through the normal command
> search process.
>
> Source: XCU hash STDOUT — utilities/hash.html#tag_20_56_10

> [spec:posix:req:builtin.hash.list-cleared-on-path-change]
> This list shall be cleared when the contents of the PATH environment variable
> are changed.
>
> Source: XCU hash STDOUT — utilities/hash.html#tag_20_56_10

### STDERR

> [spec:posix:req:builtin.hash.stderr]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU hash STDERR — utilities/hash.html#tag_20_56_11

### Other interfaces

> [spec:posix:req:builtin.hash.interfaces]
> Standard input is not used; there are no input files; asynchronous events are
> handled as for the utility description defaults; there are no output files;
> there is no extended description; and the consequences of errors are as for
> the utility description defaults.
>
> Source: XCU hash — utilities/hash.html#tag_20_56

### EXIT STATUS

> [spec:posix:req:builtin.hash.exit-status]
> The following exit values shall be returned:
>
> | Exit status | Meaning |
> |---|---|
> | 0 | Successful completion. |
> | >0 | An error occurred. |
>
> Source: XCU hash EXIT STATUS — utilities/hash.html#tag_20_56_14
