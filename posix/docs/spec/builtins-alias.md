# Intrinsic Utilities: alias, unalias, and fc

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

The alias and unalias pages carry no `[UP]` shading — Issue 7 moved both from the
User Portability Utilities option to the Base — so the only option-conditional
text on those two pages is the `[XSI]` NLSPATH entry. The fc page has its entire
SYNOPSIS shaded `[UP]` `[Option Start]` … `[Option End]`, which by the standard's
own convention ("Where applicable, utilities are marked with the UP margin
legend in the SYNOPSIS section") makes the whole fc reference page conditional
on the User Portability Utilities option; every fc rule below therefore leads
with `[UP]`, and the fc NLSPATH rule carries `[UP]` `[XSI]` — the standard's
space-separated notation for a feature requiring support of both options.

## alias

### SYNOPSIS

> [spec:posix:syn:builtin.alias.synopsis]
> The alias utility shall be invocable in the following form:
>
> `alias [alias-name[=string]...]`
>
> Source: XCU alias SYNOPSIS — utilities/alias.html#tag_20_02_02

### DESCRIPTION

> [spec:posix:req:builtin.alias.create-or-display]
> The alias utility shall create or redefine alias definitions or write the
> values of existing alias definitions to standard output.
>
> Source: XCU alias DESCRIPTION — utilities/alias.html#tag_20_02_03

> [spec:posix:def:builtin.alias.definition]
> An alias definition provides a string value that shall replace a command name
> when it is encountered. For information on valid string values, and the
> processing involved, see 2.3.1 Alias Substitution.
>
> Source: XCU alias DESCRIPTION — utilities/alias.html#tag_20_02_03

> [spec:posix:req:builtin.alias.execution-environment]
> An alias definition shall affect the current shell execution environment and
> the execution environments of the subshells of the current shell. When used as
> specified by this volume of POSIX.1-2024, the alias definition shall not
> affect the parent process of the current shell nor any utility environment
> invoked by the shell; see 2.13 Shell Execution Environment.
>
> Source: XCU alias DESCRIPTION — utilities/alias.html#tag_20_02_03

### OPERANDS

> [spec:posix:req:builtin.alias.operands]
> The following operands shall be supported:
>
> alias-name — Write the alias definition to standard output.
>
> alias-name=string — Assign the value of string to the alias alias-name.
>
> If no operands are given, all alias definitions shall be written to standard
> output.
>
> Source: XCU alias OPERANDS — utilities/alias.html#tag_20_02_05

### ENVIRONMENT VARIABLES

> [spec:posix:req:builtin.alias.env-locale]
> The following environment variables shall affect the execution of alias:
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
> Source: XCU alias ENVIRONMENT VARIABLES — utilities/alias.html#tag_20_02_08

> [spec:posix:sem:builtin.alias.env-nlspath]
> `[XSI]` NLSPATH determines the location of messages objects and message
> catalogs.
>
> Source: XCU alias ENVIRONMENT VARIABLES — utilities/alias.html#tag_20_02_08

### STDOUT

> [spec:posix:req:builtin.alias.stdout-format]
> The format for displaying aliases (when no operands or only name operands are
> specified) shall be:
>
> `"%s=%s\n", name, value`
>
> The value string shall be written with appropriate quoting so that it is
> suitable for reinput to the shell. See the description of shell quoting in 2.2
> Quoting.
>
> Source: XCU alias STDOUT — utilities/alias.html#tag_20_02_10

### STDERR

> [spec:posix:req:builtin.alias.stderr]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU alias STDERR — utilities/alias.html#tag_20_02_11

### Other interfaces

> [spec:posix:req:builtin.alias.interfaces]
> The alias utility has no options. Standard input is not used; there are no
> input files; asynchronous events are handled as for the utility description
> defaults; there are no output files; there is no extended description; and the
> consequences of errors are as for the utility description defaults.
>
> Source: XCU alias — utilities/alias.html#tag_20_02

### EXIT STATUS

> [spec:posix:req:builtin.alias.exit-status]
> The following exit values shall be returned:
>
> | Exit status | Meaning |
> |---|---|
> | 0 | Successful completion. |
> | >0 | One of the name operands specified did not have an alias definition, or an error occurred. |
>
> Source: XCU alias EXIT STATUS — utilities/alias.html#tag_20_02_14

## unalias

### SYNOPSIS

> [spec:posix:syn:builtin.unalias.synopsis]
> The unalias utility shall be invocable in the following two forms:
>
> `unalias alias-name...`
>
> `unalias -a`
>
> Source: XCU unalias SYNOPSIS — utilities/unalias.html#tag_20_133_02

### DESCRIPTION

> [spec:posix:req:builtin.unalias.remove-definitions]
> The unalias utility shall remove the definition for each alias name specified.
> See 2.3.1 Alias Substitution. The aliases shall be removed from the current
> shell execution environment; see 2.13 Shell Execution Environment.
>
> Source: XCU unalias DESCRIPTION — utilities/unalias.html#tag_20_133_03

### OPTIONS

> [spec:posix:req:builtin.unalias.utility-syntax-guidelines]
> The unalias utility shall conform to XBD 12.2 Utility Syntax Guidelines.
>
> Source: XCU unalias OPTIONS — utilities/unalias.html#tag_20_133_04

> [spec:posix:req:builtin.unalias.opt-a]
> The following option shall be supported:
>
> **-a** — Remove all alias definitions from the current shell execution
> environment.
>
> Source: XCU unalias OPTIONS — utilities/unalias.html#tag_20_133_04

### OPERANDS

> [spec:posix:req:builtin.unalias.operand-alias-name]
> The following operand shall be supported:
>
> alias-name — The name of an alias to be removed.
>
> Source: XCU unalias OPERANDS — utilities/unalias.html#tag_20_133_05

### ENVIRONMENT VARIABLES

> [spec:posix:req:builtin.unalias.env-locale]
> The following environment variables shall affect the execution of unalias:
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
> Source: XCU unalias ENVIRONMENT VARIABLES — utilities/unalias.html#tag_20_133_08

> [spec:posix:sem:builtin.unalias.env-nlspath]
> `[XSI]` NLSPATH determines the location of messages objects and message
> catalogs.
>
> Source: XCU unalias ENVIRONMENT VARIABLES — utilities/unalias.html#tag_20_133_08

### STDERR

> [spec:posix:req:builtin.unalias.stderr]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU unalias STDERR — utilities/unalias.html#tag_20_133_11

### Other interfaces

> [spec:posix:req:builtin.unalias.interfaces]
> Standard input is not used; there are no input files; asynchronous events are
> handled as for the utility description defaults; standard output is not used;
> there are no output files; there is no extended description; and the
> consequences of errors are as for the utility description defaults.
>
> Source: XCU unalias — utilities/unalias.html#tag_20_133

### EXIT STATUS

> [spec:posix:req:builtin.unalias.exit-status]
> The following exit values shall be returned:
>
> | Exit status | Meaning |
> |---|---|
> | 0 | Successful completion. |
> | >0 | One of the alias-name operands specified did not represent a valid alias definition, or an error occurred. |
>
> Source: XCU unalias EXIT STATUS — utilities/unalias.html#tag_20_133_14

## fc

### SYNOPSIS

> [spec:posix:syn:builtin.fc.synopsis]
> `[UP]` The fc utility shall be invocable in the following three forms:
>
> `fc [-r] [-e editor] [first [last]]`
>
> `fc -l [-nr] [first [last]]`
>
> `fc -s [old=new] [first]`
>
> Source: XCU fc SYNOPSIS — utilities/fc.html#tag_20_44_02

### DESCRIPTION

> [spec:posix:req:builtin.fc.list-or-edit]
> `[UP]` The fc utility shall list, or shall edit and re-execute, commands
> previously entered to an interactive sh.
>
> Source: XCU fc DESCRIPTION — utilities/fc.html#tag_20_44_03

> [spec:posix:req:builtin.fc.history-numbering]
> `[UP]` The command history list shall reference commands by number. The first
> number in the list is selected arbitrarily. The relationship of a number to
> its command shall not change except when the user logs in and no other process
> is accessing the list, at which time the system may reset the numbering to
> start the oldest retained command at another number (usually 1).
>
> Source: XCU fc DESCRIPTION — utilities/fc.html#tag_20_44_03

> [spec:posix:req:builtin.fc.history-number-wrap]
> `[UP]` When the number reaches an implementation-defined upper limit, which
> shall be no smaller than the value in HISTSIZE or 32767 (whichever is
> greater), the shell may wrap the numbers, starting the next command with a
> lower number (usually 1). However, despite this optional wrapping of numbers,
> fc shall maintain the time-ordering sequence of the commands. For example, if
> four commands in sequence are given the numbers 32766, 32767, 1 (wrapped), and
> 2 as they are executed, command 32767 is considered the command previous to 1,
> even though its number is higher.
>
> Source: XCU fc DESCRIPTION — utilities/fc.html#tag_20_44_03

> [spec:posix:req:builtin.fc.edit-and-reexecute]
> `[UP]` When commands are edited (when the **-l** option is not specified), the
> resulting lines shall be entered at the end of the history list and then
> re-executed by sh. The fc command that caused the editing shall not be entered
> into the history list. If the editor returns a non-zero exit status, this shall
> suppress the entry into the history list and the command re-execution. Any
> command line variable assignments or redirection operators used with fc shall
> affect both the fc command itself as well as the command that results; for
> example:
>
> `fc -s -- -1 2>/dev/null`
>
> reinvokes the previous command, suppressing standard error for both fc and the
> previous command.
>
> Source: XCU fc DESCRIPTION — utilities/fc.html#tag_20_44_03

### OPTIONS

> [spec:posix:req:builtin.fc.utility-syntax-guidelines]
> `[UP]` The fc utility shall conform to XBD 12.2 Utility Syntax Guidelines.
>
> Source: XCU fc OPTIONS — utilities/fc.html#tag_20_44_04

> [spec:posix:req:builtin.fc.opt-e]
> `[UP]` **-e** editor — Use the editor named by editor to edit the commands. The
> editor string is a utility name, subject to search via the PATH variable (see
> XBD 8. Environment Variables). The value in the FCEDIT variable shall be used
> as a default when **-e** is not specified. If FCEDIT is null or unset, ed shall
> be used as the editor.
>
> Source: XCU fc OPTIONS — utilities/fc.html#tag_20_44_04

> [spec:posix:req:builtin.fc.opt-l]
> `[UP]` **-l** — (The letter ell.) List the commands rather than invoking an
> editor on them. The commands shall be written in the sequence indicated by the
> first and last operands, as affected by **-r**, with each command preceded by
> the command number.
>
> Source: XCU fc OPTIONS — utilities/fc.html#tag_20_44_04

> [spec:posix:req:builtin.fc.opt-n]
> `[UP]` **-n** — Suppress command numbers when listing with **-l**.
>
> Source: XCU fc OPTIONS — utilities/fc.html#tag_20_44_04

> [spec:posix:req:builtin.fc.opt-r]
> `[UP]` **-r** — Reverse the order of the commands listed (with **-l**) or
> edited (with neither **-l** nor **-s**).
>
> Source: XCU fc OPTIONS — utilities/fc.html#tag_20_44_04

> [spec:posix:req:builtin.fc.opt-s]
> `[UP]` **-s** — Re-execute the command without invoking an editor.
>
> Source: XCU fc OPTIONS — utilities/fc.html#tag_20_44_04

### OPERANDS

> [spec:posix:syn:builtin.fc.operand-first-last]
> `[UP]` The following operands shall be supported:
>
> first, last — Select the commands to list or edit. The number of previous
> commands that can be accessed shall be determined by the value of the HISTSIZE
> variable. The value of first or last or both shall be one of the following:
>
> `[+]number` — A positive number representing a command number; command numbers
> can be displayed with the **-l** option.
>
> `-number` — A negative decimal number representing the command that was
> executed number of commands previously. For example, -1 is the immediately
> previous command.
>
> string — A string indicating the most recently entered command that begins
> with that string. If the old=new operand is not also specified with **-s**, the
> string form of the first operand cannot contain an embedded <equals-sign>.
>
> Source: XCU fc OPERANDS — utilities/fc.html#tag_20_44_05

> [spec:posix:req:builtin.fc.operand-default-s]
> `[UP]` When the synopsis form with **-s** is used, if first is omitted, the
> previous command shall be used.
>
> Source: XCU fc OPERANDS — utilities/fc.html#tag_20_44_05

> [spec:posix:req:builtin.fc.operand-defaults-no-s]
> `[UP]` For the synopsis forms without **-s**:
>
> - If last is omitted, last shall default to the previous command when **-l**
>   is specified; otherwise, it shall default to first.
> - If first and last are both omitted, the previous 16 commands shall be listed
>   or the previous single command shall be edited (based on the **-l** option).
>
> Source: XCU fc OPERANDS — utilities/fc.html#tag_20_44_05

> [spec:posix:req:builtin.fc.operand-range]
> `[UP]` If first and last are both present, all of the commands from first to
> last shall be edited (without **-l**) or listed (with **-l**). Editing multiple
> commands shall be accomplished by presenting to the editor all of the commands
> at one time, each command starting on a new line. If first represents a newer
> command than last, the commands shall be listed or edited in reverse sequence,
> equivalent to using **-r**. For example, the following commands on the first
> line are equivalent to the corresponding commands on the second:
>
> `fc -r 10 20    fc    30 40`
>
> `fc    20 10    fc -r 40 30`
>
> Source: XCU fc OPERANDS — utilities/fc.html#tag_20_44_05

> [spec:posix:req:builtin.fc.operand-range-clamping]
> `[UP]` When a range of commands is used, it shall not be an error to specify
> first or last values that are not in the history list; fc shall substitute the
> value representing the oldest or newest command in the list, as appropriate.
> For example, if there are only ten commands in the history list, numbered 1 to
> 10:
>
> `fc -l`
>
> `fc 1 99`
>
> shall list and edit, respectively, all ten commands.
>
> Source: XCU fc OPERANDS — utilities/fc.html#tag_20_44_05

> [spec:posix:req:builtin.fc.operand-old-new]
> `[UP]` old=new — Replace the first occurrence of string old in the commands to
> be re-executed by the string new.
>
> Source: XCU fc OPERANDS — utilities/fc.html#tag_20_44_05

### ENVIRONMENT VARIABLES

> [spec:posix:req:builtin.fc.env-fcedit]
> `[UP]` The following environment variables shall affect the execution of fc:
>
> FCEDIT — This variable, when expanded by the shell, shall determine the default
> value for the **-e** editor option's editor option-argument. If FCEDIT is null
> or unset, ed shall be used as the editor.
>
> Source: XCU fc ENVIRONMENT VARIABLES — utilities/fc.html#tag_20_44_08

> [spec:posix:req:builtin.fc.env-histfile]
> `[UP]` HISTFILE — Determine a pathname naming a command history file. If the
> HISTFILE variable is not set, the shell may attempt to access or create a file
> .sh_history in the directory referred to by the HOME environment variable. If
> the shell cannot obtain both read and write access to, or create, the history
> file, it shall use an unspecified mechanism that allows the history to operate
> properly. (References to history "file" in this section shall be understood to
> mean this unspecified mechanism in such cases.)
>
> Source: XCU fc ENVIRONMENT VARIABLES — utilities/fc.html#tag_20_44_08

> [spec:posix:req:builtin.fc.env-histfile-initialization]
> `[UP]` An implementation may choose to access the HISTFILE variable only when
> initializing the history file; this initialization shall occur when fc or sh
> first attempt to retrieve entries from, or add entries to, the file, as the
> result of commands issued by the user, the file named by the ENV variable, or
> implementation-defined system start-up files. In some historical shells, the
> history file is initialized just after the ENV file has been processed.
> Therefore, it is implementation-defined whether changes made to HISTFILE after
> the history file has been initialized are effective. Implementations may choose
> to disable the history list mechanism for users with appropriate privileges who
> do not set HISTFILE; the specific circumstances under which this occurs are
> implementation-defined.
>
> Source: XCU fc ENVIRONMENT VARIABLES — utilities/fc.html#tag_20_44_08

> [spec:posix:req:builtin.fc.env-histfile-sharing-and-deletion]
> `[UP]` If more than one instance of the shell is using the same history file,
> it is unspecified how updates to the history file from those shells interact.
> As entries are deleted from the history file, they shall be deleted oldest
> first. It is unspecified when history file entries are physically removed from
> the history file.
>
> Source: XCU fc ENVIRONMENT VARIABLES — utilities/fc.html#tag_20_44_08

> [spec:posix:req:builtin.fc.env-histsize]
> `[UP]` HISTSIZE — Determine a decimal number representing the limit to the
> number of previous commands that are accessible. If this variable is unset, an
> unspecified default greater than or equal to 128 shall be used. The maximum
> number of commands in the history list is unspecified, but shall be at least
> 128. An implementation may choose to access this variable only when
> initializing the history file, as described under HISTFILE. Therefore, it is
> unspecified whether changes made to HISTSIZE after the history file has been
> initialized are effective.
>
> Source: XCU fc ENVIRONMENT VARIABLES — utilities/fc.html#tag_20_44_08

> [spec:posix:req:builtin.fc.env-locale]
> `[UP]` LANG provides a default value for the internationalization variables
> that are unset or null. (See XBD 8.2 Internationalization Variables for the
> precedence of internationalization variables used to determine the values of
> locale categories.)
>
> LC_ALL, if set to a non-empty string value, overrides the values of all the
> other internationalization variables.
>
> LC_CTYPE determines the locale for the interpretation of sequences of bytes of
> text data as characters (for example, single-byte as opposed to multi-byte
> characters in arguments and input files).
>
> LC_MESSAGES determines the locale that should be used to affect the format and
> contents of diagnostic messages written to standard error.
>
> Source: XCU fc ENVIRONMENT VARIABLES — utilities/fc.html#tag_20_44_08

> [spec:posix:sem:builtin.fc.env-nlspath]
> `[UP]` `[XSI]` NLSPATH determines the location of messages objects and message
> catalogs.
>
> Source: XCU fc ENVIRONMENT VARIABLES — utilities/fc.html#tag_20_44_08

### STDOUT

> [spec:posix:req:builtin.fc.stdout-list-format]
> `[UP]` When the **-l** option is used to list commands, the format of each
> command in the list shall be as follows:
>
> `"%d\t%s\n", <line number>, <command>`
>
> If both the **-l** and **-n** options are specified, the format of each command
> shall be:
>
> `"\t%s\n", <command>`
>
> If the <command> consists of more than one line, the lines after the first
> shall be displayed as:
>
> `"\t%s\n", <continued-command>`
>
> Source: XCU fc STDOUT — utilities/fc.html#tag_20_44_10

### STDERR

> [spec:posix:req:builtin.fc.stderr]
> `[UP]` The standard error shall be used only for diagnostic messages.
>
> Source: XCU fc STDERR — utilities/fc.html#tag_20_44_11

### Other interfaces

> [spec:posix:req:builtin.fc.interfaces]
> `[UP]` Standard input is not used; there are no input files; asynchronous
> events are handled as for the utility description defaults; there are no output
> files; there is no extended description; and the consequences of errors are as
> for the utility description defaults.
>
> Source: XCU fc — utilities/fc.html#tag_20_44

### EXIT STATUS

> [spec:posix:req:builtin.fc.exit-status]
> `[UP]` The following exit values shall be returned:
>
> | Exit status | Meaning |
> |---|---|
> | 0 | Successful completion of the listing. |
> | >0 | An error occurred. |
>
> Otherwise, the exit status shall be that of the commands executed by fc.
>
> Source: XCU fc EXIT STATUS — utilities/fc.html#tag_20_44_14
