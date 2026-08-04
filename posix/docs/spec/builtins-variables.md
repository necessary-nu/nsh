# Special Built-In Utilities: Variables and Positional Parameters

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
"SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in RFC 2119. This document
reproduces normative text from IEEE Std 1003.1-2024 (POSIX.1-2024),
Copyright © 2001-2024 The IEEE and The Open Group.

## export

### SYNOPSIS

> [spec:posix:syn:builtin.export.synopsis]
> `export name[=word]...`
>
> `export -p`
>
> Source: XCU export SYNOPSIS — utilities/V3_chap02.html#tag_19_23_02

### DESCRIPTION

> [spec:posix:req:builtin.export.set-attribute]
> The shell shall give the export attribute to the variables corresponding to
> the specified names, which shall cause them to be in the environment of
> subsequently executed commands. If the name of a variable is followed by
> =word, then the value of that variable shall be set to word.
>
> Source: XCU export DESCRIPTION — utilities/V3_chap02.html#tag_19_23_03

> [spec:posix:req:builtin.export.declaration-utility]
> The export special built-in shall be a declaration utility. Therefore, if
> export is recognized as the command name of a simple command, then subsequent
> words of the form name=word shall be expanded in an assignment context. See
> 2.9.1.1 Order of Processing.
>
> Source: XCU export DESCRIPTION — utilities/V3_chap02.html#tag_19_23_03

> [spec:posix:req:builtin.export.utility-syntax-guidelines]
> The export special built-in shall support XBD 12.2 Utility Syntax Guidelines.
>
> Source: XCU export DESCRIPTION — utilities/V3_chap02.html#tag_19_23_03

> [spec:posix:req:builtin.export.p-output-format]
> When -p is specified, export shall write to the standard output the names and
> values of all exported variables, in the following format:
>
> `"export %s=%s\n", <name>, <value>`
>
> if name is set, and:
>
> `"export %s\n", <name>`
>
> if name is unset.
>
> Source: XCU export DESCRIPTION — utilities/V3_chap02.html#tag_19_23_03

> [spec:posix:req:builtin.export.p-output-reinput]
> The shell shall format the output, including the proper use of quoting, so
> that it is suitable for reinput to the shell as commands that achieve the same
> exporting results, except:
>
> 1. Read-only variables with values cannot be reset.
> 2. Variables that were unset at the time they were output need not be reset to
> the unset state if a value is assigned to the variable between the time the
> state was saved and the time at which the saved output is reinput to the
> shell.
>
> Source: XCU export DESCRIPTION — utilities/V3_chap02.html#tag_19_23_03

> [spec:posix:sem:builtin.export.no-arguments]
> When no arguments are given, the results are unspecified.
>
> Source: XCU export DESCRIPTION — utilities/V3_chap02.html#tag_19_23_03

### STDERR

> [spec:posix:req:builtin.export.stderr]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU export STDERR — utilities/V3_chap02.html#tag_19_23_11

### EXIT STATUS

> [spec:posix:req:builtin.export.exit-status]
> The following exit values shall be returned:
>
> | Exit status | Condition |
> |---|---|
> | 0 | Successful completion. |
> | greater than 0 | At least one operand could not be processed as requested, such as a name operand that could not be exported or an attempt to modify a readonly variable using a name=word operand, or the -p option was specified and a write error occurred. |
>
> Source: XCU export EXIT STATUS — utilities/V3_chap02.html#tag_19_23_14

### Utility description defaults

> [spec:posix:sem:builtin.export.utility-defaults]
> Standard input is not used by export. There are no input files, no
> environment variables that affect its execution, and no output files. There
> is no extended description. Asynchronous events and the consequences of
> errors are the default ones described in XCU 1.4 Utility Description Defaults.
> The options and operands of export are described in the DESCRIPTION, and its
> standard output is as described in the DESCRIPTION.
>
> Source: XCU export STDIN, INPUT FILES, ENVIRONMENT VARIABLES, ASYNCHRONOUS EVENTS, OUTPUT FILES, EXTENDED DESCRIPTION, CONSEQUENCES OF ERRORS — utilities/V3_chap02.html#tag_19_23_06

## readonly

### SYNOPSIS

> [spec:posix:syn:builtin.readonly.synopsis]
> `readonly name[=word]...`
>
> `readonly -p`
>
> Source: XCU readonly SYNOPSIS — utilities/V3_chap02.html#tag_19_24_02

### DESCRIPTION

> [spec:posix:req:builtin.readonly.set-attribute]
> The variables whose names are specified shall be given the readonly attribute.
> If the name of a variable is followed by =word, then the value of that
> variable shall be set to word.
>
> Source: XCU readonly DESCRIPTION — utilities/V3_chap02.html#tag_19_24_03

> [spec:posix:def:builtin.readonly.attribute]
> The values of variables with the readonly attribute cannot be changed by
> subsequent assignment or use of the export, getopts, readonly, or read
> utilities, nor can those variables be unset by the unset utility.
>
> Source: XCU readonly DESCRIPTION — utilities/V3_chap02.html#tag_19_24_03

> [spec:posix:req:builtin.readonly.application-constraint]
> As described in XBD 8.1 Environment Variable Definition, conforming
> applications shall not request to mark a variable as readonly if it is
> documented as being manipulated by a shell built-in utility, as it may render
> those utilities unable to complete successfully.
>
> Source: XCU readonly DESCRIPTION — utilities/V3_chap02.html#tag_19_24_03

> [spec:posix:req:builtin.readonly.declaration-utility]
> The readonly special built-in shall be a declaration utility. Therefore, if
> readonly is recognized as the command name of a simple command, then
> subsequent words of the form name=word shall be expanded in an assignment
> context. See 2.9.1.1 Order of Processing.
>
> Source: XCU readonly DESCRIPTION — utilities/V3_chap02.html#tag_19_24_03

> [spec:posix:req:builtin.readonly.utility-syntax-guidelines]
> The readonly special built-in shall support XBD 12.2 Utility Syntax
> Guidelines.
>
> Source: XCU readonly DESCRIPTION — utilities/V3_chap02.html#tag_19_24_03

> [spec:posix:sem:builtin.readonly.p-output-format]
> When -p is specified, readonly writes to the standard output the names and
> values of all read-only variables, in the following format:
>
> `"readonly %s=%s\n", <name>, <value>`
>
> if name is set, and
>
> `"readonly %s\n", <name>`
>
> if name is unset.
>
> Source: XCU readonly DESCRIPTION — utilities/V3_chap02.html#tag_19_24_03

> [spec:posix:req:builtin.readonly.p-output-reinput]
> The shell shall format the output, including the proper use of quoting, so
> that it is suitable for reinput to the shell as commands that achieve the same
> value and readonly attribute-setting results in a shell execution environment
> in which:
>
> 1. Variables with values at the time they were output do not have the readonly
> attribute set.
> 2. Variables that were unset at the time they were output do not have a value
> at the time at which the saved output is reinput to the shell.
>
> Source: XCU readonly DESCRIPTION — utilities/V3_chap02.html#tag_19_24_03

> [spec:posix:sem:builtin.readonly.no-arguments]
> When no arguments are given, the results are unspecified.
>
> Source: XCU readonly DESCRIPTION — utilities/V3_chap02.html#tag_19_24_03

### STDERR

> [spec:posix:req:builtin.readonly.stderr]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU readonly STDERR — utilities/V3_chap02.html#tag_19_24_11

### EXIT STATUS

> [spec:posix:req:builtin.readonly.exit-status]
> The following exit values shall be returned:
>
> | Exit status | Condition |
> |---|---|
> | 0 | Successful completion. |
> | greater than 0 | At least one operand could not be processed as requested, such as a name operand that could not be marked readonly or an attempt to modify an already readonly variable using a name=word operand, or the -p option was specified and a write error occurred. |
>
> Source: XCU readonly EXIT STATUS — utilities/V3_chap02.html#tag_19_24_14

### Utility description defaults

> [spec:posix:sem:builtin.readonly.utility-defaults]
> Standard input is not used by readonly. There are no input files, no
> environment variables that affect its execution, and no output files. There
> is no extended description. Asynchronous events and the consequences of
> errors are the default ones described in XCU 1.4 Utility Description Defaults.
> The options and operands of readonly are described in the DESCRIPTION, and its
> standard output is as described in the DESCRIPTION.
>
> Source: XCU readonly STDIN, INPUT FILES, ENVIRONMENT VARIABLES, ASYNCHRONOUS EVENTS, OUTPUT FILES, EXTENDED DESCRIPTION, CONSEQUENCES OF ERRORS — utilities/V3_chap02.html#tag_19_24_06

## return

### SYNOPSIS

> [spec:posix:syn:builtin.return.synopsis]
> `return [n]`
>
> Source: XCU return SYNOPSIS — utilities/V3_chap02.html#tag_19_25_02

### DESCRIPTION

> [spec:posix:req:builtin.return.stop-function-or-dot-script]
> The return utility shall cause the shell to stop executing the current
> function or dot script. If the shell is not currently executing a function or
> dot script, the results are unspecified.
>
> Source: XCU return DESCRIPTION — utilities/V3_chap02.html#tag_19_25_03

### STDERR

> [spec:posix:req:builtin.return.stderr]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU return STDERR — utilities/V3_chap02.html#tag_19_25_11

### EXIT STATUS

> [spec:posix:req:builtin.return.exit-status]
> The exit status shall be n, if specified, except that the behavior is
> unspecified if n is not an unsigned decimal integer or is greater than 255. If
> n is not specified, the result shall be as if n were specified with the
> current value of the special parameter `'?'`, except that if the return
> command would cause the end of execution of a trap action, the value for the
> special parameter `'?'` that is considered "current" shall be the value it had
> immediately preceding the trap action.
>
> Source: XCU return EXIT STATUS — utilities/V3_chap02.html#tag_19_25_14

### Utility description defaults

> [spec:posix:sem:builtin.return.utility-defaults]
> The return utility has no options; its operand is described in the
> DESCRIPTION. Standard input, standard output, and output files are not used.
> There are no input files and no environment variables that affect its
> execution. There is no extended description. Asynchronous events and the
> consequences of errors are the default ones described in XCU 1.4 Utility
> Description Defaults.
>
> Source: XCU return OPTIONS, STDIN, INPUT FILES, ENVIRONMENT VARIABLES, ASYNCHRONOUS EVENTS, STDOUT, OUTPUT FILES, EXTENDED DESCRIPTION, CONSEQUENCES OF ERRORS — utilities/V3_chap02.html#tag_19_25_04

## shift

### SYNOPSIS

> [spec:posix:syn:builtin.shift.synopsis]
> `shift [n]`
>
> Source: XCU shift SYNOPSIS — utilities/V3_chap02.html#tag_19_27_02

### DESCRIPTION

> [spec:posix:req:builtin.shift.positional-parameters]
> The positional parameters shall be shifted. Positional parameter 1 shall be
> assigned the value of parameter (1+n), parameter 2 shall be assigned the value
> of parameter (2+n), and so on. The parameters represented by the numbers
> `"$#"` down to `"$#-n+1"` shall be unset, and the parameter `'#'` is updated
> to reflect the new number of positional parameters.
>
> Source: XCU shift DESCRIPTION — utilities/V3_chap02.html#tag_19_27_03

> [spec:posix:req:builtin.shift.operand-value]
> The value n shall be an unsigned decimal integer less than or equal to the
> value of the special parameter `'#'`. If n is not given, it shall be assumed
> to be 1. If n is 0, the positional and special parameters are not changed.
>
> Source: XCU shift DESCRIPTION — utilities/V3_chap02.html#tag_19_27_03

### STDERR

> [spec:posix:req:builtin.shift.stderr]
> The standard error shall be used only for diagnostic messages and the warning
> message specified in EXIT STATUS.
>
> Source: XCU shift STDERR — utilities/V3_chap02.html#tag_19_27_11

### EXIT STATUS

> [spec:posix:req:builtin.shift.exit-status]
> If the n operand is invalid or is greater than `"$#"`, this may be treated as
> an error and a non-interactive shell may exit; if the shell does not exit in
> this case, a non-zero exit status shall be returned and a warning message
> shall be written to standard error. Otherwise, zero shall be returned.
>
> Source: XCU shift EXIT STATUS — utilities/V3_chap02.html#tag_19_27_14

### Utility description defaults

> [spec:posix:sem:builtin.shift.utility-defaults]
> The shift utility has no options; its operand is described in the DESCRIPTION.
> Standard input, standard output, and output files are not used. There are no
> input files and no environment variables that affect its execution. There is
> no extended description. Asynchronous events and the consequences of errors
> are the default ones described in XCU 1.4 Utility Description Defaults.
>
> Source: XCU shift OPTIONS, STDIN, INPUT FILES, ENVIRONMENT VARIABLES, ASYNCHRONOUS EVENTS, STDOUT, OUTPUT FILES, EXTENDED DESCRIPTION, CONSEQUENCES OF ERRORS — utilities/V3_chap02.html#tag_19_27_04

## times

### SYNOPSIS

> [spec:posix:syn:builtin.times.synopsis]
> `times`
>
> Source: XCU times SYNOPSIS — utilities/V3_chap02.html#tag_19_28_02

### DESCRIPTION

> [spec:posix:req:builtin.times.output-format]
> The times utility shall write the accumulated user and system times for the
> shell and for all of its child processes, in the following POSIX locale
> format:
>
> `"%dm%fs %dm%fs\n%dm%fs %dm%fs\n", <shell user minutes>, <shell user seconds>, <shell system minutes>, <shell system seconds>, <children user minutes>, <children user seconds>, <children system minutes>, <children system seconds>`
>
> Source: XCU times DESCRIPTION — utilities/V3_chap02.html#tag_19_28_03

> [spec:posix:req:builtin.times.tms-correspondence]
> The four pairs of times shall correspond to the members of the `<sys/times.h>`
> tms structure (defined in XBD 14. Headers) as returned by times(): tms_utime,
> tms_stime, tms_cutime, and tms_cstime, respectively.
>
> Source: XCU times DESCRIPTION — utilities/V3_chap02.html#tag_19_28_03

### STDERR

> [spec:posix:req:builtin.times.stderr]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU times STDERR — utilities/V3_chap02.html#tag_19_28_11

### EXIT STATUS

> [spec:posix:req:builtin.times.exit-status]
> The following exit values shall be returned:
>
> | Exit status | Condition |
> |---|---|
> | 0 | Successful completion. |
> | greater than 0 | An error occurred. |
>
> Source: XCU times EXIT STATUS — utilities/V3_chap02.html#tag_19_28_14

### Utility description defaults

> [spec:posix:sem:builtin.times.utility-defaults]
> The times utility has no options and no operands. Standard input is not used,
> and its standard output is as described in the DESCRIPTION. There are no input
> files, no environment variables that affect its execution, and no output
> files. There is no extended description. Asynchronous events and the
> consequences of errors are the default ones described in XCU 1.4 Utility
> Description Defaults.
>
> Source: XCU times OPTIONS, OPERANDS, STDIN, INPUT FILES, ENVIRONMENT VARIABLES, ASYNCHRONOUS EVENTS, OUTPUT FILES, EXTENDED DESCRIPTION, CONSEQUENCES OF ERRORS — utilities/V3_chap02.html#tag_19_28_04

## unset

### SYNOPSIS

> [spec:posix:syn:builtin.unset.synopsis]
> `unset [-fv] name...`
>
> Source: XCU unset SYNOPSIS — utilities/V3_chap02.html#tag_19_30_02

### DESCRIPTION

> [spec:posix:req:builtin.unset.unset-names]
> The unset utility shall unset each variable or function definition specified
> by name that does not have the readonly attribute and remove any attributes
> other than readonly that have been given to name (see 2.15 Special Built-In
> Utilities, export and readonly).
>
> Source: XCU unset DESCRIPTION — utilities/V3_chap02.html#tag_19_30_03

> [spec:posix:req:builtin.unset.v-option]
> If -v is specified, name refers to a variable name and the shell shall unset
> it and remove it from the environment. Read-only variables cannot be unset.
>
> Source: XCU unset DESCRIPTION — utilities/V3_chap02.html#tag_19_30_03

> [spec:posix:req:builtin.unset.f-option]
> If -f is specified, name refers to a function and the shell shall unset the
> function definition.
>
> Source: XCU unset DESCRIPTION — utilities/V3_chap02.html#tag_19_30_03

> [spec:posix:req:builtin.unset.no-option]
> If neither -f nor -v is specified, name refers to a variable; if a variable by
> that name does not exist, it is unspecified whether a function by that name,
> if any, shall be unset.
>
> Source: XCU unset DESCRIPTION — utilities/V3_chap02.html#tag_19_30_03

> [spec:posix:req:builtin.unset.not-previously-set]
> Unsetting a variable or function that was not previously set shall not be
> considered an error and does not cause the shell to abort.
>
> Source: XCU unset DESCRIPTION — utilities/V3_chap02.html#tag_19_30_03

> [spec:posix:req:builtin.unset.utility-syntax-guidelines]
> The unset special built-in shall support XBD 12.2 Utility Syntax Guidelines.
>
> Source: XCU unset DESCRIPTION — utilities/V3_chap02.html#tag_19_30_03

> [spec:posix:sem:builtin.unset.empty-assignment-and-special-parameters]
> Note that `VARIABLE=` is not equivalent to an unset of VARIABLE; in the
> example, VARIABLE is set to `""`. Also, the variables that can be unset should
> not be misinterpreted to include the special parameters (see 2.5.2 Special
> Parameters).
>
> Source: XCU unset DESCRIPTION — utilities/V3_chap02.html#tag_19_30_03

### STDERR

> [spec:posix:req:builtin.unset.stderr]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU unset STDERR — utilities/V3_chap02.html#tag_19_30_11

### EXIT STATUS

> [spec:posix:req:builtin.unset.exit-status]
> The following exit values shall be returned:
>
> | Exit status | Condition |
> |---|---|
> | 0 | All name operands were successfully unset. |
> | greater than 0 | At least one name could not be unset. |
>
> Source: XCU unset EXIT STATUS — utilities/V3_chap02.html#tag_19_30_14

### Utility description defaults

> [spec:posix:sem:builtin.unset.utility-defaults]
> The options and operands of unset are described in the DESCRIPTION. Standard
> input, standard output, and output files are not used. There are no input
> files and no environment variables that affect its execution. There is no
> extended description. Asynchronous events and the consequences of errors are
> the default ones described in XCU 1.4 Utility Description Defaults.
>
> Source: XCU unset STDIN, INPUT FILES, ENVIRONMENT VARIABLES, ASYNCHRONOUS EVENTS, STDOUT, OUTPUT FILES, EXTENDED DESCRIPTION, CONSEQUENCES OF ERRORS — utilities/V3_chap02.html#tag_19_30_06
