# Intrinsic Utilities: cd, umask, and ulimit

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
"SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in RFC 2119. This document
reproduces normative text from IEEE Std 1003.1-2024 (POSIX.1-2024),
Copyright © 2001-2024 The IEEE and The Open Group.

Option-conditional text carries the standard's own margin code inline, at the point the standard shades it:

`[XSI]`
: X/Open System Interfaces. The functionality described is an extension, available on all systems supporting the XSI option.

## cd — SYNOPSIS

> [spec:posix:syn:builtin.cd.syn]
> The synopsis of the cd utility is:
>
> `cd [-L] [directory]`
>
> `cd -P [-e] [directory]`
>
> Source: XCU cd SYNOPSIS — utilities/cd.html#tag_20_14_02

## cd — DESCRIPTION

> [spec:posix:req:builtin.cd.change-working-directory]
> The cd utility shall change the working directory of the current shell
> execution environment (see 2.13 Shell Execution Environment) by executing the
> following steps in sequence.
>
> Source: XCU cd DESCRIPTION — utilities/cd.html#tag_20_14_03

> [spec:posix:def:builtin.cd.curpath]
> In the following steps, the symbol curpath represents an intermediate value
> used to simplify the description of the algorithm used by cd. There is no
> requirement that curpath be made visible to the application.
>
> Source: XCU cd DESCRIPTION — utilities/cd.html#tag_20_14_03

> [spec:posix:req:builtin.cd.step1-no-operand-no-home]
> Step 1: If no directory operand is given and the HOME environment variable is
> empty or undefined, the default behavior is implementation-defined and no
> further steps shall be taken.
>
> Source: XCU cd DESCRIPTION — utilities/cd.html#tag_20_14_03

> [spec:posix:req:builtin.cd.step2-home-as-operand]
> Step 2: If no directory operand is given and the HOME environment variable is
> set to a non-empty value, the cd utility shall behave as if the directory
> named in the HOME environment variable was specified as the directory operand.
>
> Source: XCU cd DESCRIPTION — utilities/cd.html#tag_20_14_03

> [spec:posix:sem:builtin.cd.step3-absolute-operand]
> Step 3: If the directory operand begins with a <slash> character, set curpath
> to the operand and proceed to step 7.
>
> Source: XCU cd DESCRIPTION — utilities/cd.html#tag_20_14_03

> [spec:posix:sem:builtin.cd.step4-dot-or-dot-dot]
> Step 4: If the first component of the directory operand is dot or dot-dot,
> proceed to step 6.
>
> Source: XCU cd DESCRIPTION — utilities/cd.html#tag_20_14_03

> [spec:posix:sem:builtin.cd.step5-cdpath-search]
> Step 5: Starting with the first pathname in the <colon>-separated pathnames of
> CDPATH (see the ENVIRONMENT VARIABLES section) if the pathname is non-null,
> test if the concatenation of that pathname, a <slash> character if that
> pathname did not end with a <slash> character, and the directory operand names
> a directory. If the pathname is null, test if the concatenation of dot, a
> <slash> character, and the operand names a directory. In either case, if the
> resulting string names an existing directory, set curpath to that string and
> proceed to step 7. Otherwise, repeat this step with the next pathname in
> CDPATH until all pathnames have been tested.
>
> Source: XCU cd DESCRIPTION — utilities/cd.html#tag_20_14_03

> [spec:posix:sem:builtin.cd.step6-operand-as-curpath]
> Step 6: Set curpath to the directory operand.
>
> Source: XCU cd DESCRIPTION — utilities/cd.html#tag_20_14_03

> [spec:posix:sem:builtin.cd.step7-prefix-pwd]
> Step 7: If the -P option is in effect, proceed to step 10. If curpath does not
> begin with a <slash> character, set curpath to the string formed by the
> concatenation of the value of PWD, a <slash> character if the value of PWD did
> not end with a <slash> character, and curpath.
>
> Source: XCU cd DESCRIPTION — utilities/cd.html#tag_20_14_03

> [spec:posix:req:builtin.cd.step8-canonical-form-dot]
> Step 8: The curpath value shall then be converted to canonical form as
> follows, considering each component from beginning to end, in sequence:
>
> Dot components and any <slash> characters that separate them from the next
> component shall be deleted.
>
> Source: XCU cd DESCRIPTION — utilities/cd.html#tag_20_14_03

> [spec:posix:req:builtin.cd.step8-canonical-form-dot-dot]
> Step 8, continued: For each dot-dot component, if there is a preceding
> component and it is neither root nor dot-dot, then:
>
> 1. If the preceding component does not refer (in the context of pathname
>    resolution with symbolic links followed) to a directory, then the cd
>    utility shall display an appropriate error message and no further steps
>    shall be taken.
> 2. The preceding component, all <slash> characters separating the preceding
>    component from dot-dot, dot-dot, and all <slash> characters separating
>    dot-dot from the following component (if any) shall be deleted.
>
> Source: XCU cd DESCRIPTION — utilities/cd.html#tag_20_14_03

> [spec:posix:req:builtin.cd.step8-further-simplification]
> Step 8, continued: An implementation may further simplify curpath by removing
> any trailing <slash> characters that are not also leading <slash> characters,
> replacing multiple non-leading consecutive <slash> characters with a single
> <slash>, and replacing three or more leading <slash> characters with a single
> <slash>. If, as a result of this canonicalization, the curpath variable is
> null, no further steps shall be taken.
>
> Source: XCU cd DESCRIPTION — utilities/cd.html#tag_20_14_03

> [spec:posix:req:builtin.cd.step9-path-max-relative]
> Step 9: If curpath is longer than {PATH_MAX} bytes (including the terminating
> null) and the directory operand was not longer than {PATH_MAX} bytes
> (including the terminating null), then curpath shall be converted from an
> absolute pathname to an equivalent relative pathname if possible. This
> conversion shall always be considered possible if the value of PWD, with a
> trailing <slash> added if it does not already have one, is an initial
> substring of curpath. Whether or not it is considered possible under other
> circumstances is unspecified. Implementations may also apply this conversion
> if curpath is not longer than {PATH_MAX} bytes or the directory operand was
> longer than {PATH_MAX} bytes.
>
> Source: XCU cd DESCRIPTION — utilities/cd.html#tag_20_14_03

> [spec:posix:req:builtin.cd.step10-chdir]
> Step 10: The cd utility shall then perform actions equivalent to the chdir()
> function called with curpath as the path argument. If these actions fail for
> any reason, the cd utility shall display an appropriate error message and the
> remainder of this step shall not be executed. If the -P option is not in
> effect, the PWD environment variable shall be set to the value that curpath
> had on entry to step 9 (i.e., before conversion to a relative pathname).
>
> Source: XCU cd DESCRIPTION — utilities/cd.html#tag_20_14_03

> [spec:posix:req:builtin.cd.step10-pwd-physical]
> Step 10, continued: If the -P option is in effect, the PWD environment
> variable shall be set to the string that would be output by pwd -P. If there
> is insufficient permission on the new directory, or on any parent of that
> directory, to determine the current working directory, the value of the PWD
> environment variable is unspecified. If both the -e and the -P options are in
> effect and cd is unable to determine the pathname of the current working
> directory, cd shall complete successfully but return a non-zero exit status.
>
> Source: XCU cd DESCRIPTION — utilities/cd.html#tag_20_14_03

> [spec:posix:req:builtin.cd.oldpwd-set]
> If, during the execution of the above steps, the PWD environment variable is
> set, the OLDPWD shell variable shall also be set to the value of the old
> working directory (that is the current working directory immediately prior to
> the call to cd). It is unspecified whether, when setting OLDPWD, the shell
> also causes it to be exported if it was not already.
>
> Source: XCU cd DESCRIPTION — utilities/cd.html#tag_20_14_03

## cd — OPTIONS

> [spec:posix:req:builtin.cd.utility-syntax-guidelines]
> The cd utility shall conform to XBD 12.2 Utility Syntax Guidelines.
>
> Source: XCU cd OPTIONS — utilities/cd.html#tag_20_14_04

> [spec:posix:req:builtin.cd.opt-e]
> The following options shall be supported by the implementation:
>
> **-e**: If the -P option is in effect, the current working directory is
> successfully changed, and the correct value of the PWD environment variable
> cannot be determined, exit with exit status 1.
>
> Source: XCU cd OPTIONS — utilities/cd.html#tag_20_14_04

> [spec:posix:req:builtin.cd.opt-l]
> **-L**: Handle the operand dot-dot logically; symbolic link components shall
> not be resolved before dot-dot components are processed (see steps 8. and 9.
> in the DESCRIPTION).
>
> Source: XCU cd OPTIONS — utilities/cd.html#tag_20_14_04

> [spec:posix:req:builtin.cd.opt-p]
> **-P**: Handle the operand dot-dot physically; symbolic link components shall
> be resolved before dot-dot components are processed (see step 7. in the
> DESCRIPTION).
>
> Source: XCU cd OPTIONS — utilities/cd.html#tag_20_14_04

> [spec:posix:req:builtin.cd.opt-l-p-last-wins]
> If both -L and -P options are specified, the last of these options shall be
> used and all others ignored. If neither -L nor -P is specified, the operand
> shall be handled dot-dot logically; see the DESCRIPTION.
>
> Source: XCU cd OPTIONS — utilities/cd.html#tag_20_14_04

## cd — OPERANDS

> [spec:posix:def:builtin.cd.operand-directory]
> The following operands shall be supported:
>
> directory: An absolute or relative pathname of the directory that shall become
> the new working directory. The interpretation of a relative pathname by cd
> depends on the -L option and the CDPATH and PWD environment variables.
>
> Source: XCU cd OPERANDS — utilities/cd.html#tag_20_14_05

> [spec:posix:req:builtin.cd.operand-empty-string]
> If directory is an empty string, cd shall write a diagnostic message to
> standard error and exit with non-zero status.
>
> Source: XCU cd OPERANDS — utilities/cd.html#tag_20_14_05

> [spec:posix:req:builtin.cd.operand-hyphen]
> If directory consists of a single `'-'` (<hyphen-minus>) character, the cd
> utility shall behave as if directory contained the value of the OLDPWD
> environment variable, except that after it sets the value of PWD it shall
> write the new value to standard output. The behavior is unspecified if OLDPWD
> does not start with a <slash> character.
>
> Source: XCU cd OPERANDS — utilities/cd.html#tag_20_14_05

## cd — ENVIRONMENT VARIABLES

> [spec:posix:req:builtin.cd.env-cdpath]
> The following environment variables shall affect the execution of cd:
>
> Environment variable CDPATH: A <colon>-separated list of pathnames that refer
> to directories. The cd utility shall use this list in its attempt to change
> the directory, as described in the DESCRIPTION. An empty string in place of a
> directory pathname represents the current directory. If CDPATH is not set, it
> shall be treated as if it were an empty string.
>
> Source: XCU cd ENVIRONMENT VARIABLES — utilities/cd.html#tag_20_14_08

> [spec:posix:def:builtin.cd.env-home]
> Environment variable HOME: The name of the directory, used when no directory
> operand is specified.
>
> Source: XCU cd ENVIRONMENT VARIABLES — utilities/cd.html#tag_20_14_08

> [spec:posix:req:builtin.cd.env-locale]
> - LANG: Provide a default value for the internationalization variables that
>   are unset or null. (See XBD 8.2 Internationalization Variables for the
>   precedence of internationalization variables used to determine the values of
>   locale categories.)
> - LC_ALL: If set to a non-empty string value, override the values of all the
>   other internationalization variables.
> - LC_CTYPE: Determine the locale for the interpretation of sequences of bytes
>   of text data as characters (for example, single-byte as opposed to
>   multi-byte characters in arguments).
> - LC_MESSAGES: Determine the locale that should be used to affect the format
>   and contents of diagnostic messages written to standard error.
>
> Source: XCU cd ENVIRONMENT VARIABLES — utilities/cd.html#tag_20_14_08

> [spec:posix:req:builtin.cd.env-nlspath]
> `[XSI]` Environment variable NLSPATH: Determine the location of messages
> objects and message catalogs.
>
> Source: XCU cd ENVIRONMENT VARIABLES — utilities/cd.html#tag_20_14_08

> [spec:posix:req:builtin.cd.env-oldpwd]
> Environment variable OLDPWD: A pathname of the previous working directory,
> used when the operand is `'-'`. If an application sets or unsets the value of
> OLDPWD, the behavior of cd with a `'-'` operand is unspecified.
>
> Source: XCU cd ENVIRONMENT VARIABLES — utilities/cd.html#tag_20_14_08

> [spec:posix:req:builtin.cd.env-pwd]
> Environment variable PWD: This variable shall be set as specified in the
> DESCRIPTION. If an application sets or unsets the value of PWD, the behavior
> of cd is unspecified.
>
> Source: XCU cd ENVIRONMENT VARIABLES — utilities/cd.html#tag_20_14_08

## cd — STDOUT

> [spec:posix:req:builtin.cd.stdout-new-directory]
> If a non-empty directory name from CDPATH is used, or if the operand `'-'` is
> used, and the absolute pathname of the new working directory can be
> determined, that pathname shall be written to the standard output as follows:
>
> `"%s\n", <new directory>`
>
> Source: XCU cd STDOUT — utilities/cd.html#tag_20_14_10

> [spec:posix:sem:builtin.cd.stdout-undeterminable-pathname]
> If an absolute pathname of the new current working directory cannot be
> determined, it is unspecified whether nothing is written to the standard
> output or the value of curpath used in step 10, followed by a <newline>, is
> written to the standard output.
>
> Source: XCU cd STDOUT — utilities/cd.html#tag_20_14_10

> [spec:posix:req:builtin.cd.stdout-no-output]
> If a non-empty directory name from CDPATH is not used, and the directory
> argument is not `'-'`, there shall be no output.
>
> Source: XCU cd STDOUT — utilities/cd.html#tag_20_14_10

## cd — STDERR, other interfaces, exit status, and errors

> [spec:posix:req:builtin.cd.stderr]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU cd STDERR — utilities/cd.html#tag_20_14_11

> [spec:posix:req:builtin.cd.interfaces]
> Standard input is not used by the cd utility; there are no input files;
> asynchronous events are handled as for the utility description defaults; there
> are no output files; and there is no extended description.
>
> Source: XCU cd — utilities/cd.html#tag_20_14

> [spec:posix:req:builtin.cd.exit-status]
> The following exit values shall be returned:
>
> | Exit status | Meaning |
> |---|---|
> | 0 | The current working directory was successfully changed and the value of the PWD environment variable was set correctly. |
> | 0 | The current working directory was successfully changed, the -e option is not in effect, the -P option is in effect, and the correct value of the PWD environment variable could not be determined. |
> | >0 | Either the -e option or the -P option is not in effect, and an error occurred. |
> | 1 | The current working directory was successfully changed, both the -e and the -P options are in effect, and the correct value of the PWD environment variable could not be determined. |
> | >1 | Both the -e and the -P options are in effect, and an error occurred. |
>
> Source: XCU cd EXIT STATUS — utilities/cd.html#tag_20_14_14

> [spec:posix:req:builtin.cd.consequences-of-errors]
> The working directory shall remain unchanged.
>
> Source: XCU cd CONSEQUENCES OF ERRORS — utilities/cd.html#tag_20_14_15

## umask — SYNOPSIS

> [spec:posix:syn:builtin.umask.syn]
> The synopsis of the umask utility is `umask [-S] [mask]`.
>
> Source: XCU umask SYNOPSIS — utilities/umask.html#tag_20_132_02

## umask — DESCRIPTION

> [spec:posix:req:builtin.umask.set-mask]
> The umask utility shall set the file mode creation mask of the current shell
> execution environment (see 2.13 Shell Execution Environment) to the value
> specified by the mask operand. This mask shall affect the initial value of the
> file permission bits of subsequently created files.
>
> Source: XCU umask DESCRIPTION — utilities/umask.html#tag_20_132_03

> [spec:posix:req:builtin.umask.subshell-no-effect]
> If umask is called in a subshell or separate utility execution environment,
> such as one of the following:
>
> `(umask 002)`
>
> `nohup umask ...`
>
> `find . -exec umask ... \;`
>
> it shall not affect the file mode creation mask of the caller's environment.
>
> Source: XCU umask DESCRIPTION — utilities/umask.html#tag_20_132_03

> [spec:posix:req:builtin.umask.report-when-no-operand]
> If the mask operand is not specified, the umask utility shall write to
> standard output the value of the file mode creation mask of the invoking
> process.
>
> Source: XCU umask DESCRIPTION — utilities/umask.html#tag_20_132_03

## umask — OPTIONS

> [spec:posix:req:builtin.umask.utility-syntax-guidelines]
> The umask utility shall conform to XBD 12.2 Utility Syntax Guidelines.
>
> Source: XCU umask OPTIONS — utilities/umask.html#tag_20_132_04

> [spec:posix:req:builtin.umask.opt-s]
> The following option shall be supported:
>
> **-S**: Produce symbolic output.
>
> Source: XCU umask OPTIONS — utilities/umask.html#tag_20_132_04

> [spec:posix:req:builtin.umask.default-output-style]
> The default output style is unspecified, but shall be recognized on a
> subsequent invocation of umask on the same system as a mask operand to restore
> the previous file mode creation mask.
>
> Source: XCU umask OPTIONS — utilities/umask.html#tag_20_132_04

## umask — OPERANDS

> [spec:posix:def:builtin.umask.operand-mask]
> The following operand shall be supported:
>
> mask: A string specifying the new file mode creation mask. The string is
> treated in the same way as the mode operand described in the EXTENDED
> DESCRIPTION section for chmod.
>
> Source: XCU umask OPERANDS — utilities/umask.html#tag_20_132_05

> [spec:posix:req:builtin.umask.symbolic-mode-complement]
> For a symbolic_mode value, the new value of the file mode creation mask shall
> be the logical complement of the file permission bits portion of the file mode
> specified by the symbolic_mode string.
>
> Source: XCU umask OPERANDS — utilities/umask.html#tag_20_132_05

> [spec:posix:req:builtin.umask.symbolic-op-characters]
> In a symbolic_mode value, the permissions op characters `'+'` and `'-'` shall
> be interpreted relative to the current file mode creation mask; `'+'` shall
> cause the bits for the indicated permissions to be cleared in the mask; `'-'`
> shall cause the bits for the indicated permissions to be set in the mask.
>
> Source: XCU umask OPERANDS — utilities/umask.html#tag_20_132_05

> [spec:posix:sem:builtin.umask.non-permission-bits-unspecified]
> The interpretation of mode values that specify file mode bits other than the
> file permission bits is unspecified.
>
> Source: XCU umask OPERANDS — utilities/umask.html#tag_20_132_05

> [spec:posix:req:builtin.umask.octal-form]
> In the octal integer form of mode, the specified bits are set in the file mode
> creation mask.
>
> The file mode creation mask shall be set to the resulting numeric value.
>
> Source: XCU umask OPERANDS — utilities/umask.html#tag_20_132_05

> [spec:posix:req:builtin.umask.prior-default-output-as-operand]
> The default output of a prior invocation of umask on the same system with no
> operand also shall be recognized as a mask operand.
>
> Source: XCU umask OPERANDS — utilities/umask.html#tag_20_132_05

## umask — ENVIRONMENT VARIABLES

> [spec:posix:req:builtin.umask.env-locale]
> The following environment variables shall affect the execution of umask:
>
> - LANG: Provide a default value for the internationalization variables that
>   are unset or null. (See XBD 8.2 Internationalization Variables for the
>   precedence of internationalization variables used to determine the values of
>   locale categories.)
> - LC_ALL: If set to a non-empty string value, override the values of all the
>   other internationalization variables.
> - LC_CTYPE: Determine the locale for the interpretation of sequences of bytes
>   of text data as characters (for example, single-byte as opposed to
>   multi-byte characters in arguments).
> - LC_MESSAGES: Determine the locale that should be used to affect the format
>   and contents of diagnostic messages written to standard error.
>
> Source: XCU umask ENVIRONMENT VARIABLES — utilities/umask.html#tag_20_132_08

> [spec:posix:req:builtin.umask.env-nlspath]
> `[XSI]` Environment variable NLSPATH: Determine the location of messages
> objects and message catalogs.
>
> Source: XCU umask ENVIRONMENT VARIABLES — utilities/umask.html#tag_20_132_08

## umask — STDOUT

> [spec:posix:req:builtin.umask.stdout-no-operand]
> When the mask operand is not specified, the umask utility shall write a
> message to standard output that can later be used as a umask mask operand.
>
> Source: XCU umask STDOUT — utilities/umask.html#tag_20_132_10

> [spec:posix:req:builtin.umask.stdout-symbolic-format]
> If -S is specified, the message shall be in the following format:
>
> `"u=%s,g=%s,o=%s\n", <owner permissions>, <group permissions>, <other permissions>`
>
> where the three values shall be combinations of letters from the set {r, w,
> x}; the presence of a letter shall indicate that the corresponding bit is
> clear in the file mode creation mask.
>
> Source: XCU umask STDOUT — utilities/umask.html#tag_20_132_10

> [spec:posix:req:builtin.umask.stdout-operand-no-output]
> If a mask operand is specified, there shall be no output written to standard
> output.
>
> Source: XCU umask STDOUT — utilities/umask.html#tag_20_132_10

## umask — STDERR, other interfaces, and exit status

> [spec:posix:req:builtin.umask.stderr]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU umask STDERR — utilities/umask.html#tag_20_132_11

> [spec:posix:req:builtin.umask.interfaces]
> Standard input is not used by the umask utility; there are no input files;
> asynchronous events are handled as for the utility description defaults; there
> are no output files; there is no extended description; and the consequences of
> errors are as for the utility description defaults.
>
> Source: XCU umask — utilities/umask.html#tag_20_132

> [spec:posix:req:builtin.umask.exit-status]
> The following exit values shall be returned:
>
> | Exit status | Meaning |
> |---|---|
> | 0 | The file mode creation mask was successfully changed, or no mask operand was supplied. |
> | >0 | An error occurred. |
>
> Source: XCU umask EXIT STATUS — utilities/umask.html#tag_20_132_14

## ulimit — SYNOPSIS

> [spec:posix:syn:builtin.ulimit.syn]
> The synopsis of the ulimit utility is:
>
> `ulimit [-H|-S] -a`
>
> `[XSI]` `ulimit [-H|-S] [-c|-d|-f|-n|-s|-t|-v] [newlimit]`
>
> Source: XCU ulimit SYNOPSIS — utilities/ulimit.html#tag_20_131_02

## ulimit — DESCRIPTION

> [spec:posix:req:builtin.ulimit.report-or-set]
> The ulimit utility shall report or set the resource limits in effect in the
> process in which it is executed.
>
> Source: XCU ulimit DESCRIPTION — utilities/ulimit.html#tag_20_131_03

> [spec:posix:sem:builtin.ulimit.soft-and-hard-limits]
> Soft limits can be changed by a process to any value that is less than or
> equal to the hard limit. A process can (irreversibly) lower its hard limit to
> any value that is greater than or equal to the soft limit. Only a process with
> appropriate privileges can raise a hard limit.
>
> Source: XCU ulimit DESCRIPTION — utilities/ulimit.html#tag_20_131_03

> [spec:posix:req:builtin.ulimit.unlimited-value]
> The value unlimited for a resource shall be considered to be larger than any
> other limit value. When a resource has this limit value, the implementation
> shall not enforce limits on that resource. In locales other than the POSIX
> locale, ulimit may support additional non-numeric values with the same meaning
> as unlimited.
>
> Source: XCU ulimit DESCRIPTION — utilities/ulimit.html#tag_20_131_03

> [spec:posix:req:builtin.ulimit.limits-exceeded]
> The behavior when resource limits are exceeded shall be as described in the
> System Interfaces volume of POSIX.1-2024 for the setrlimit() function.
>
> Source: XCU ulimit DESCRIPTION — utilities/ulimit.html#tag_20_131_03

## ulimit — OPTIONS

> [spec:posix:req:builtin.ulimit.utility-syntax-guidelines]
> The ulimit utility shall conform to XBD 12.2 Utility Syntax Guidelines, except
> that:
>
> - The order in which options other than -H, -S, and -a are specified may be
>   significant.
> - Conforming applications shall specify each option separately; that is,
>   grouping option letters (for example, -fH) need not be recognized by all
>   implementations.
>
> Source: XCU ulimit OPTIONS — utilities/ulimit.html#tag_20_131_04

> [spec:posix:req:builtin.ulimit.opt-hard]
> The following options shall be supported:
>
> **-H**: Report hard limit(s) or set only a hard limit.
>
> Source: XCU ulimit OPTIONS — utilities/ulimit.html#tag_20_131_04

> [spec:posix:req:builtin.ulimit.opt-soft]
> **-S**: Report soft limit(s) or set only a soft limit.
>
> Source: XCU ulimit OPTIONS — utilities/ulimit.html#tag_20_131_04

> [spec:posix:req:builtin.ulimit.opt-all]
> **-a**: Report the limit value for all of the resources named below and for
> any implementation-specific additional resources.
>
> Source: XCU ulimit OPTIONS — utilities/ulimit.html#tag_20_131_04

> [spec:posix:req:builtin.ulimit.opt-core]
> **-c**: Report, or set if the newlimit operand is present, the core image size
> limit(s) in units of 512 bytes. `[RLIMIT_CORE]`
>
> Source: XCU ulimit OPTIONS — utilities/ulimit.html#tag_20_131_04

> [spec:posix:req:builtin.ulimit.opt-data]
> **-d**: Report, or set if the newlimit operand is present, the data segment
> size limit(s) in units of 1024 bytes. `[RLIMIT_DATA]`
>
> Source: XCU ulimit OPTIONS — utilities/ulimit.html#tag_20_131_04

> [spec:posix:req:builtin.ulimit.opt-fsize]
> **-f**: Report, or set if the newlimit operand is present, the file size
> limit(s) in units of 512 bytes. `[RLIMIT_FSIZE]`
>
> Source: XCU ulimit OPTIONS — utilities/ulimit.html#tag_20_131_04

> [spec:posix:req:builtin.ulimit.opt-nofile]
> **-n**: Report, or set if the newlimit operand is present, the limit(s) on the
> number of open file descriptors, given as a number one greater than the
> maximum value that the system assigns to a newly-created descriptor.
> `[RLIMIT_NOFILE]`
>
> Source: XCU ulimit OPTIONS — utilities/ulimit.html#tag_20_131_04

> [spec:posix:req:builtin.ulimit.opt-stack]
> **-s**: Report, or set if the newlimit operand is present, the stack size
> limit(s) in units of 1024 bytes. `[RLIMIT_STACK]`
>
> Source: XCU ulimit OPTIONS — utilities/ulimit.html#tag_20_131_04

> [spec:posix:req:builtin.ulimit.opt-cpu]
> **-t**: `[XSI]` Report, or set if the newlimit operand is present, the
> per-process CPU time limit(s) in units of seconds. `[RLIMIT_CPU]`
>
> Source: XCU ulimit OPTIONS — utilities/ulimit.html#tag_20_131_04

> [spec:posix:req:builtin.ulimit.opt-as]
> **-v**: Report, or set if the newlimit operand is present, the address space
> size limit(s) in units of 1024 bytes. `[RLIMIT_AS]`
>
> Source: XCU ulimit OPTIONS — utilities/ulimit.html#tag_20_131_04

> [spec:posix:def:builtin.ulimit.rlimit-annotation]
> Where an option description is followed by `[RLIMIT_name]` it indicates which
> resource for the getrlimit() and setrlimit() functions, defined in the System
> Interfaces volume of POSIX.1-2024, the option corresponds to.
>
> Source: XCU ulimit OPTIONS — utilities/ulimit.html#tag_20_131_04

> [spec:posix:req:builtin.ulimit.default-hard-and-soft]
> If neither the -H nor -S option is specified:
>
> - If the newlimit operand is present, it shall be used as the new value for
>   both the hard and soft limits.
> - If the newlimit operand is not present, -S shall be the default.
>
> Source: XCU ulimit OPTIONS — utilities/ulimit.html#tag_20_131_04

> [spec:posix:req:builtin.ulimit.default-f-option]
> If no options other than -H or -S are specified, the behavior shall be as if
> the -f option was (also) specified.
>
> Source: XCU ulimit OPTIONS — utilities/ulimit.html#tag_20_131_04

> [spec:posix:sem:builtin.ulimit.repeated-option-unspecified]
> If any option other than -H or -S is repeated, the behavior is unspecified.
>
> Source: XCU ulimit OPTIONS — utilities/ulimit.html#tag_20_131_04

## ulimit — OPERANDS

> [spec:posix:def:builtin.ulimit.operand-newlimit]
> The following operand shall be supported:
>
> newlimit: Either an integer value to use as the new limit(s) for the specified
> resource, in the units specified in OPTIONS, or a non-numeric string
> indicating no limit, as described in the DESCRIPTION section. Numerals in the
> range 0 to the maximum limit value supported by the implementation for any
> resource shall be syntactically recognized as numeric values.
>
> Source: XCU ulimit OPERANDS — utilities/ulimit.html#tag_20_131_05

## ulimit — ENVIRONMENT VARIABLES

> [spec:posix:req:builtin.ulimit.env-locale]
> The following environment variables shall affect the execution of ulimit:
>
> - LANG: Provide a default value for the internationalization variables that
>   are unset or null. (See XBD 8.2 Internationalization Variables for the
>   precedence of internationalization variables used to determine the values of
>   locale categories.)
> - LC_ALL: If set to a non-empty string value, override the values of all the
>   other internationalization variables.
> - LC_CTYPE: Determine the locale for the interpretation of sequences of bytes
>   of text data as characters (for example, single-byte as opposed to
>   multi-byte characters in arguments).
> - LC_MESSAGES: Determine the locale that should be used to affect the format
>   and contents of diagnostic messages written to standard error.
>
> Source: XCU ulimit ENVIRONMENT VARIABLES — utilities/ulimit.html#tag_20_131_08

> [spec:posix:req:builtin.ulimit.env-nlspath]
> Environment variable NLSPATH: Determine the location of messages objects and
> message catalogs.
>
> Source: XCU ulimit ENVIRONMENT VARIABLES — utilities/ulimit.html#tag_20_131_08

## ulimit — STDOUT

> [spec:posix:req:builtin.ulimit.stdout-used-when-reporting]
> The standard output shall be used when no newlimit operand is present.
>
> Source: XCU ulimit STDOUT — utilities/ulimit.html#tag_20_131_10

> [spec:posix:req:builtin.ulimit.stdout-all-format]
> If the -a option is specified, the output written for each resource shall
> consist of one line that includes:
>
> - A short phrase identifying the resource (for example "file size").
> - An indication of the units used for the resource, if the corresponding
>   option description in OPTIONS specifies the units to be used.
> - The ulimit option used to specify the resource.
> - The limit value.
>
> The format used within each line is unspecified, except that the format used
> for the limit value shall be as described below for the case where a single
> limit value is written.
>
> Source: XCU ulimit STDOUT — utilities/ulimit.html#tag_20_131_10

> [spec:posix:req:builtin.ulimit.stdout-single-limit-format]
> If a single limit value is to be written; that is, the -a option is not
> specified and at most one option other than -H or -S is specified:
>
> - If the resource being reported has a numeric limit, the limit value shall be
>   written in the following format: `"%1d\n", <limit value>`
>   where <limit value> is the value of the limit in the units specified in
>   OPTIONS.
> - If the resource being reported does not have a numeric limit, in the POSIX
>   locale the following format shall be used: `"unlimited\n"`
>
> Source: XCU ulimit STDOUT — utilities/ulimit.html#tag_20_131_10

## ulimit — STDERR, other interfaces, and exit status

> [spec:posix:req:builtin.ulimit.stderr]
> The standard error shall be used only for diagnostic messages.
>
> Source: XCU ulimit STDERR — utilities/ulimit.html#tag_20_131_11

> [spec:posix:req:builtin.ulimit.interfaces]
> Standard input is not used by the ulimit utility; there are no input files;
> asynchronous events are handled as for the utility description defaults; there
> are no output files; there is no extended description; and the consequences of
> errors are as for the utility description defaults.
>
> Source: XCU ulimit — utilities/ulimit.html#tag_20_131

> [spec:posix:req:builtin.ulimit.exit-status]
> The following exit values shall be returned:
>
> | Exit status | Meaning |
> |---|---|
> | 0 | Successful completion. |
> | >0 | A request for a higher limit was rejected or an error occurred. |
>
> Source: XCU ulimit EXIT STATUS — utilities/ulimit.html#tag_20_131_14
