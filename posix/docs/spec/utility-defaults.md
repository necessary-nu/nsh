# Utility Description Defaults and Built-In Utilities

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
"SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in RFC 2119. This document
reproduces normative text from IEEE Std 1003.1-2024 (POSIX.1-2024),
Copyright © 2001-2024 The IEEE and The Open Group.

Option-conditional text carries the standard's own margin code inline, at the point the standard shades it:

`[XSI]`
: X/Open System Interfaces. The functionality described is an extension, available on all systems supporting the XSI option.

## 1.2 Utility Limits

> This section lists magnitude limitations imposed by a specific
> implementation. The braces notation, {LIMIT}, is used in this volume of
> POSIX.1-2024 to indicate these values, but the braces are not part of the
> name.
>
> Source: XCU 1.2 Utility Limits — utilities/V3_chap01.html#tag_18_02

> [spec:posix:req:xcu.limits.minimum-values]
> The values specified in Utility Limit Minimum Values represent the lowest
> values conforming implementations shall provide and, consequently, the
> largest values on which an application can rely without further enquiries.
> These values shall be accessible to applications via the getconf utility.
>
> Table: Utility Limit Minimum Values
>
> | Name | Description | Value |
> |---|---|---|
> | {POSIX2_BC_BASE_MAX} | The maximum obase value allowed by the bc utility. | 99 |
> | {POSIX2_BC_DIM_MAX} | The maximum number of elements permitted in an array by the bc utility. | 2048 |
> | {POSIX2_BC_SCALE_MAX} | The maximum scale value allowed by the bc utility. | 99 |
> | {POSIX2_BC_STRING_MAX} | The maximum length of a string constant accepted by the bc utility. | 1000 |
> | {POSIX2_COLL_WEIGHTS_MAX} | The maximum number of weights that can be assigned to an entry of the LC_COLLATE order keyword in the locale definition file; see the **border_start** [sic] keyword in XBD 7.3.2 LC_COLLATE. | 2 |
> | {POSIX2_EXPR_NEST_MAX} | The maximum number of expressions that can be nested within parentheses by the expr utility. | 32 |
> | {POSIX2_LINE_MAX} | Unless otherwise noted, the maximum length, in bytes, of the input line of a utility (either standard input or another file), when the utility is described as processing text files. The length includes room for the trailing <newline>. | 2048 |
> | {POSIX_RE_DUP_MAX} | Maximum number of repeated occurrences of a BRE or ERE interval expression; see XBD 9.3.6 BREs Matching Multiple Characters and 9.4.6 EREs Matching Multiple Characters. | 255 |
>
> Source: XCU 1.2 Utility Limits — utilities/V3_chap01.html#tag_18_02

> [spec:posix:req:xcu.limits.more-liberal-values]
> Implementations may provide more liberal, or less restrictive, values than
> shown in Utility Limit Minimum Values. These possibly more liberal values are
> accessible using the symbols in Symbolic Utility Limits.
>
> Source: XCU 1.2 Utility Limits — utilities/V3_chap01.html#tag_18_02

> [spec:posix:sem:xcu.limits.symbol-retrieval]
> The sysconf() function defined in the System Interfaces volume of
> POSIX.1-2024 or the getconf utility return the value of each symbol on each
> specific implementation. The value so retrieved is the largest, or most
> liberal, value that is available throughout the session lifetime, as
> determined at session creation. The literal names shown in the table apply
> only to the getconf utility; the high-level language binding describes the
> exact form of each name to be used by the interfaces in that binding.
>
> Source: XCU 1.2 Utility Limits — utilities/V3_chap01.html#tag_18_02

> All numeric limits defined by the System Interfaces volume of POSIX.1-2024,
> such as {PATH_MAX}, shall also apply to this volume of POSIX.1-2024. All the
> utilities defined by this volume of POSIX.1-2024 are implicitly limited by
> these values, unless otherwise noted in the utility descriptions.
>
> Source: XCU 1.2 Utility Limits — utilities/V3_chap01.html#tag_18_02

> [spec:posix:sem:xcu.limits.reachability-not-guaranteed]
> It is not guaranteed that the application can actually reach the specified
> limit of an implementation in any given case, or at all, as a lack of virtual
> memory or other resources may prevent this. The limit value indicates only
> that the implementation does not specifically impose any arbitrary, more
> restrictive limit.
>
> Source: XCU 1.2 Utility Limits — utilities/V3_chap01.html#tag_18_02

> [spec:posix:def:xcu.limits.symbolic]
> Table: Symbolic Utility Limits
>
> | Name | Description | Minimum Value |
> |---|---|---|
> | {BC_BASE_MAX} | The maximum obase value allowed by the bc utility. | {POSIX2_BC_BASE_MAX} |
> | {BC_DIM_MAX} | The maximum number of elements permitted in an array by the bc utility. | {POSIX2_BC_DIM_MAX} |
> | {BC_SCALE_MAX} | The maximum scale value allowed by the bc utility. | {POSIX2_BC_SCALE_MAX} |
> | {BC_STRING_MAX} | The maximum length of a string constant accepted by the bc utility. | {POSIX2_BC_STRING_MAX} |
> | {COLL_WEIGHTS_MAX} | The maximum number of weights that can be assigned to an entry of the LC_COLLATE order keyword in the locale definition file; see the **order_start** keyword in XBD 7.3.2 LC_COLLATE. | {POSIX2_COLL_WEIGHTS_MAX} |
> | {EXPR_NEST_MAX} | The maximum number of expressions that can be nested within parentheses by the expr utility. | {POSIX2_EXPR_NEST_MAX} |
> | {LINE_MAX} | Unless otherwise noted, the maximum length, in bytes, of the input line of a utility (either standard input or another file), when the utility is described as processing text files. The length includes room for the trailing <newline>. | {POSIX2_LINE_MAX} |
> | {RE_DUP_MAX} | Maximum number of repeated occurrences of a BRE or ERE interval expression; see XBD 9.3.6 BREs Matching Multiple Characters and 9.4.6 EREs Matching Multiple Characters. | {POSIX_RE_DUP_MAX} |
>
> Source: XCU 1.2 Utility Limits — utilities/V3_chap01.html#tag_18_02

> [spec:posix:def:xcu.limits.posix2-symlinks]
> The following value may be a constant within an implementation or may vary
> from one pathname to another.
>
> {POSIX2_SYMLINKS}: When referring to a directory, the system supports the
> creation of symbolic links within that directory; for non-directory files,
> the meaning of {POSIX2_SYMLINKS} is undefined.
>
> Source: XCU 1.2 Utility Limits — utilities/V3_chap01.html#tag_18_02

## 1.3 Grammar Conventions

> Portions of this volume of POSIX.1-2024 are expressed in terms of a special
> grammar notation. It is used to portray the complex syntax of certain program
> input. The grammar is based on the syntax used by the yacc utility. However,
> it does not represent fully functional yacc input, suitable for program use;
> the lexical processing and all semantic requirements are described only in
> textual form. The grammar is not based on source used in any traditional
> implementation and has not been tested with the semantic code that would
> normally be required to accompany it. Furthermore, there is no implication
> that the partial yacc code presented represents the most efficient, or only,
> means of supporting the complex syntax within the utility.
>
> Source: XCU 1.3 Grammar Conventions — utilities/V3_chap01.html#tag_18_03

> [spec:posix:req:xcu.grammar-notation.implementation-freedom]
> Implementations may use other programming languages or algorithms, as long as
> the syntax supported is the same as that represented by the grammar.
>
> Source: XCU 1.3 Grammar Conventions — utilities/V3_chap01.html#tag_18_03

> The following typographical conventions are used in the grammar; they have no
> significance except to aid in reading.
>
> - The identifiers for the reserved words of the language are shown with a
>   leading capital letter. (These are terminals in the grammar; for example,
>   **While**, **Case**.)
> - The identifiers for terminals in the grammar are all named with uppercase
>   letters and underscores; for example, **NEWLINE**, **ASSIGN_OP**, **NAME**.
> - The identifiers for non-terminals are all lowercase.
>
> Source: XCU 1.3 Grammar Conventions — utilities/V3_chap01.html#tag_18_03

## 1.4 Utility Description Defaults

> This section describes all of the subsections used within the utility
> descriptions, including:
>
> - Intended usage of the section
> - Global defaults that affect all the standard utilities
> - The meanings of notations used in this volume of POSIX.1-2024 that are
>   specific to individual utility sections
>
> Source: XCU 1.4 Utility Description Defaults — utilities/V3_chap01.html#tag_18_04

### NAME

> This section gives the name or names of the utility and briefly states its
> purpose.
>
> Source: XCU 1.4 Utility Description Defaults, NAME — utilities/V3_chap01.html#tag_18_04

### SYNOPSIS

> The SYNOPSIS section summarizes the syntax of the calling sequence for the
> utility, including options, option-arguments, and operands. Standards for
> utility naming are described in XBD 12.2 Utility Syntax Guidelines; for
> describing the utility's arguments in XBD 12.1 Utility Argument Syntax.
>
> Source: XCU 1.4 Utility Description Defaults, SYNOPSIS — utilities/V3_chap01.html#tag_18_04

### DESCRIPTION

> The DESCRIPTION section describes the actions of the utility. If the utility
> has a very complex set of subcommands or its own procedural language, an
> EXTENDED DESCRIPTION section is also provided. Most explanations of optional
> functionality are omitted here, as they are usually explained in the OPTIONS
> section.
>
> Source: XCU 1.4 Utility Description Defaults, DESCRIPTION — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.description.equivalent-functionality]
> As stated in 1.1.1.11 Actions Equivalent to Functions, some functions are
> described in terms of equivalent functionality. When specific functions are
> cited, the implementation shall provide equivalent functionality including
> side-effects associated with successful execution of the function. The
> treatment of errors and intermediate results from the individual functions
> cited is generally not specified by this volume of POSIX.1-2024. See the
> utility's EXIT STATUS and CONSEQUENCES OF ERRORS sections for all actions
> associated with errors encountered by the utility.
>
> Source: XCU 1.4 Utility Description Defaults, DESCRIPTION — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.description.declaration-utility]
> A standard utility shall not be treated as a declaration utility unless
> explicitly stated in this section.
>
> Source: XCU 1.4 Utility Description Defaults, DESCRIPTION — utilities/V3_chap01.html#tag_18_04

### OPTIONS

> The OPTIONS section describes the utility options and option-arguments, and
> how they modify the actions of the utility. Standard utilities that have
> options either fully comply with XBD 12.2 Utility Syntax Guidelines or
> describe all deviations.
>
> Source: XCU 1.4 Utility Description Defaults, OPTIONS — utilities/V3_chap01.html#tag_18_04

> Apparent disagreements between functionality descriptions in the OPTIONS and
> DESCRIPTION (or EXTENDED DESCRIPTION) sections are always resolved in favor
> of the OPTIONS section.
>
> Source: XCU 1.4 Utility Description Defaults, OPTIONS — utilities/V3_chap01.html#tag_18_04

> Each OPTIONS section that uses the phrase "The ... utility shall conform to
> the Utility Syntax Guidelines ..." refers only to the use of the utility as
> specified by this volume of POSIX.1-2024; implementation extensions should
> also conform to the guidelines, but may allow exceptions for historical
> practice.
>
> Source: XCU 1.4 Utility Description Defaults, OPTIONS — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.options.unrecognized-diagnostic]
> Unless otherwise stated in the utility description, when given an option
> unrecognized by the implementation, or when a required option-argument is not
> provided, standard utilities shall issue a diagnostic message to standard
> error and exit with an exit status that indicates an error occurred.
>
> Source: XCU 1.4 Utility Description Defaults, OPTIONS — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.options.eight-bit-transparency]
> All utilities in this volume of POSIX.1-2024 shall be capable of processing
> arguments using eight-bit transparency.
>
> Source: XCU 1.4 Utility Description Defaults, OPTIONS — utilities/V3_chap01.html#tag_18_04

> When this section is listed as "None.", it means that the implementation need
> not support any options. Standard utilities that do not accept options, but
> that do accept operands, shall recognize `"--"` as a first argument to be
> discarded.
>
> Source: XCU 1.4 Utility Description Defaults, OPTIONS — utilities/V3_chap01.html#tag_18_04

### OPERANDS

> The OPERANDS section describes the utility operands, and how they affect the
> actions of the utility.
>
> Source: XCU 1.4 Utility Description Defaults, OPERANDS — utilities/V3_chap01.html#tag_18_04

> Apparent disagreements between functionality descriptions in the OPERANDS and
> DESCRIPTION (or EXTENDED DESCRIPTION) sections shall be resolved in favor of
> the OPERANDS section.
>
> Source: XCU 1.4 Utility Description Defaults, OPERANDS — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.operands.hyphen-means-stdin]
> If an operand naming a file can be specified as `'-'`, which means to use the
> standard input instead of a named file, this is explicitly stated in this
> section. Unless otherwise stated, the use of multiple instances of `'-'` to
> mean standard input in a single command produces unspecified results.
>
> Source: XCU 1.4 Utility Description Defaults, OPERANDS — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.operands.processing-order]
> Unless otherwise stated, the standard utilities that accept operands shall
> process those operands in the order specified in the command line.
>
> Source: XCU 1.4 Utility Description Defaults, OPERANDS — utilities/V3_chap01.html#tag_18_04

> When this section is listed as "None.", it means that the implementation need
> not support any operands.
>
> Source: XCU 1.4 Utility Description Defaults, OPERANDS — utilities/V3_chap01.html#tag_18_04

### STDIN

> The STDIN section describes the standard input of the utility. This section
> is frequently merely a reference to the following section, as many utilities
> treat standard input and input files in the same manner.
>
> Source: XCU 1.4 Utility Description Defaults, STDIN — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.stdin.input-file-restrictions-apply]
> Unless otherwise stated, all restrictions described in the INPUT FILES
> section shall apply to this section as well.
>
> Source: XCU 1.4 Utility Description Defaults, STDIN — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.stdin.terminal-background]
> Use of a terminal for standard input can cause any of the standard utilities
> that read standard input to stop when used in the background. For this
> reason, applications should not use interactive features in scripts to be
> placed in the background.
>
> Source: XCU 1.4 Utility Description Defaults, STDIN — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.stdin.env-independence]
> The specified standard input format of the standard utilities shall not
> depend on the existence or value of the environment variables defined in this
> volume of POSIX.1-2024, except as provided by this volume of POSIX.1-2024.
>
> Source: XCU 1.4 Utility Description Defaults, STDIN — utilities/V3_chap01.html#tag_18_04

> When this section is listed as "Not used.", it means that the standard input
> shall not be read when the utility is used as described by this volume of
> POSIX.1-2024.
>
> Source: XCU 1.4 Utility Description Defaults, STDIN — utilities/V3_chap01.html#tag_18_04

### INPUT FILES

> The INPUT FILES section describes the files, other than the standard input,
> used as input by the utility. It includes files named as operands and
> option-arguments as well as other files that are referred to, such as
> start-up and initialization files, databases, and so on. Commonly-used files
> are generally described in one place and cross-referenced by other utilities.
>
> Source: XCU 1.4 Utility Description Defaults, INPUT FILES — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.input-files.eight-bit-transparency]
> All utilities in this volume of POSIX.1-2024 shall be capable of processing
> input files using eight-bit transparency.
>
> Source: XCU 1.4 Utility Description Defaults, INPUT FILES — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.input-files.seekable-file-offset]
> When a standard utility reads a seekable input file and terminates without an
> error before it reaches end-of-file, the utility shall ensure that the file
> offset in the open file description is properly positioned just past the last
> byte processed by the utility. For files that are not seekable, the state of
> the file offset in the open file description for that file is unspecified. A
> conforming application shall not assume that the following three commands are
> equivalent:
>
> `tail -n +2 file`
>
> `(sed -n 1q; cat) < file`
>
> `cat file | (sed -n 1q; cat)`
>
> The second command is equivalent to the first only when the file is seekable.
> The third command leaves the file offset in the open file description in an
> unspecified state. Other utilities, such as head, read, and sh, have similar
> properties.
>
> Source: XCU 1.4 Utility Description Defaults, INPUT FILES — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.input-files.document-size-limits]
> Some of the standard utilities, such as filters, process input files a line
> or a block at a time and have no restrictions on the maximum input file size.
> Some utilities may have size limitations that are not as obvious as file
> space or memory limitations. Such limitations should reflect resource
> limitations of some sort, not arbitrary limits set by implementors.
> Implementations shall document those utilities that are limited by
> constraints other than file system space, available memory, and other limits
> specifically cited by this volume of POSIX.1-2024, and identify what the
> constraint is and indicate a way of estimating when the constraint would be
> reached. Similarly, some utilities descend the directory tree (recursively).
> Implementations shall also document any limits that they may have in
> descending the directory tree that are beyond limits cited by this volume of
> POSIX.1-2024.
>
> Source: XCU 1.4 Utility Description Defaults, INPUT FILES — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.input-files.text-file-and-line-continuation]
> When an input file is described as a "text file", the utility produces
> undefined results if given input that is not from a text file, unless
> otherwise stated. Some utilities (for example, make, read, sh) allow for
> continued input lines using an escaped <newline> convention; unless otherwise
> stated, the utility need not be able to accumulate more than {LINE_MAX} bytes
> from a set of multiple, continued input lines. Thus, for a conforming
> application the total of all the continued lines in a set cannot exceed
> {LINE_MAX}. If a utility using the escaped <newline> convention detects an
> end-of-file condition immediately after an escaped <newline>, the results are
> unspecified.
>
> Source: XCU 1.4 Utility Description Defaults, INPUT FILES — utilities/V3_chap01.html#tag_18_04

> Record formats are described in a notation similar to that used by the
> C-language function, printf(). See XBD 5. File Format Notation for a
> description of this notation. The format description is intended to be
> sufficiently rigorous to allow other applications to generate these input
> files. However, since <blank>s can legitimately be included in some of the
> fields described by the standard utilities, particularly in locales other
> than the POSIX locale, this intent is not always realized.
>
> The same notation is used for the record formats described in the STDOUT and
> OUTPUT FILES sections.
>
> Source: XCU 1.4 Utility Description Defaults, INPUT FILES — utilities/V3_chap01.html#tag_18_04

> When this section is listed as "None.", it means that no input files are
> required to be supplied when the utility is used as described by this volume
> of POSIX.1-2024.
>
> Source: XCU 1.4 Utility Description Defaults, INPUT FILES — utilities/V3_chap01.html#tag_18_04

### ENVIRONMENT VARIABLES

> The ENVIRONMENT VARIABLES section lists what variables affect the utility's
> execution.
>
> Source: XCU 1.4 Utility Description Defaults, ENVIRONMENT VARIABLES — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.env.effects-confined-to-section]
> The entire manner in which environment variables described in this volume of
> POSIX.1-2024 affect the behavior of each utility is described in the
> ENVIRONMENT VARIABLES section for that utility, in conjunction with the
> global effects of the LANG, LC_ALL, and `[XSI]` `[Option Start]` NLSPATH
> `[Option End]` environment variables described in XBD 8. Environment
> Variables. The existence or value of environment variables described in this
> volume of POSIX.1-2024 shall not otherwise affect the specified behavior of
> the standard utilities. Any effects of the existence or value of environment
> variables not described by this volume of POSIX.1-2024 upon the standard
> utilities are unspecified.
>
> Source: XCU 1.4 Utility Description Defaults, ENVIRONMENT VARIABLES — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.env.utility-selection-path-search]
> For those standard utilities that use environment variables as a means for
> selecting a utility to execute (such as CC in make), the string provided to
> the utility is subjected to the path search described for PATH in XBD 8.
> Environment Variables.
>
> Source: XCU 1.4 Utility Description Defaults, ENVIRONMENT VARIABLES — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.env.eight-bit-transparency]
> All utilities in this volume of POSIX.1-2024 shall be capable of processing
> environment variable names and values using eight-bit transparency.
>
> Source: XCU 1.4 Utility Description Defaults, ENVIRONMENT VARIABLES — utilities/V3_chap01.html#tag_18_04

> When this section is listed as "None.", it means that the behavior of the
> utility is not directly affected by environment variables described by this
> volume of POSIX.1-2024 when the utility is used as described by this volume
> of POSIX.1-2024.
>
> Source: XCU 1.4 Utility Description Defaults, ENVIRONMENT VARIABLES — utilities/V3_chap01.html#tag_18_04

### ASYNCHRONOUS EVENTS

> The ASYNCHRONOUS EVENTS section lists how the utility reacts to such events
> as signals and what signals are caught.
>
> Source: XCU 1.4 Utility Description Defaults, ASYNCHRONOUS EVENTS — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.defaults.asynchronous-events-default]
> When this section is listed as "Default.", or it refers to "the standard
> action" for any signal, it means that the action taken as a result of the
> signal shall be as follows:
>
> - If the action inherited from the invoking process, according to the rules
>   of inheritance of signal actions defined in the System Interfaces volume of
>   POSIX.1-2024, is for the signal to be ignored, the utility shall ignore the
>   signal.
> - If the action inherited from the invoking process, according to the rules
>   of inheritance of signal actions defined in System Interfaces volume of
>   POSIX.1-2024, is the default signal action, the result of the utility's
>   execution shall be as if the default signal action had been taken.
>
> Source: XCU 1.4 Utility Description Defaults, ASYNCHRONOUS EVENTS — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.async.may-catch-and-resignal]
> When the required action is for the signal to terminate the utility, the
> utility may catch the signal, perform some additional processing (such as
> deleting temporary files), restore the default signal action, and resignal
> itself.
>
> Source: XCU 1.4 Utility Description Defaults, ASYNCHRONOUS EVENTS — utilities/V3_chap01.html#tag_18_04

### STDOUT

> The STDOUT section completely describes the standard output of the utility.
> This section is frequently merely a reference to the following section,
> OUTPUT FILES, because many utilities treat standard output and output files
> in the same manner.
>
> Source: XCU 1.4 Utility Description Defaults, STDOUT — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.stdout.terminal-background]
> Use of a terminal for standard output may cause any of the standard utilities
> that write standard output to stop when used in the background. For this
> reason, applications should not use interactive features in scripts to be
> placed in the background.
>
> Source: XCU 1.4 Utility Description Defaults, STDOUT — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.stdout.env-independence]
> The specified standard output of the standard utilities shall not depend on
> the existence or value of the environment variables defined in this volume of
> POSIX.1-2024, except as provided by this volume of POSIX.1-2024.
>
> Source: XCU 1.4 Utility Description Defaults, STDOUT — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.stdout.display-verb]
> Some of the standard utilities describe their output using the verb display,
> defined in XBD 3.107 Display. Output described in the STDOUT sections of such
> utilities may be produced using means other than standard output. When
> standard output is directed to a terminal, the output described shall be
> written directly to the terminal. Otherwise, the results are undefined.
>
> Source: XCU 1.4 Utility Description Defaults, STDOUT — utilities/V3_chap01.html#tag_18_04

> When this section is listed as "Not used.", it means that the standard output
> shall not be written when the utility is used as described by this volume of
> POSIX.1-2024.
>
> Source: XCU 1.4 Utility Description Defaults, STDOUT — utilities/V3_chap01.html#tag_18_04

### STDERR

> The STDERR section describes the standard error output of the utility. Only
> those messages that are purposely sent by the utility are described.
>
> Source: XCU 1.4 Utility Description Defaults, STDERR — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.stderr.terminal-background]
> Use of a terminal for standard error may cause any of the standard utilities
> that write standard error output to stop when used in the background. For
> this reason, applications should not use interactive features in scripts to
> be placed in the background.
>
> Source: XCU 1.4 Utility Description Defaults, STDERR — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.stderr.message-language]
> The format of diagnostic messages for most utilities is unspecified, but the
> language and cultural conventions of diagnostic and informative messages
> whose format is unspecified by this volume of POSIX.1-2024 should be affected
> by the setting of LC_MESSAGES and `[XSI]` `[Option Start]` NLSPATH
> `[Option End]`.
>
> Source: XCU 1.4 Utility Description Defaults, STDERR — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.stderr.env-independence]
> The specified standard error output of standard utilities shall not depend on
> the existence or value of the environment variables defined in this volume of
> POSIX.1-2024, except as provided by this volume of POSIX.1-2024.
>
> Source: XCU 1.4 Utility Description Defaults, STDERR — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.defaults.stderr-diagnostics-only]
> When this section is listed as "The standard error shall be used only for
> diagnostic messages.", it means that, unless otherwise stated, the diagnostic
> messages shall be sent to the standard error only when the exit status
> indicates that an error occurred and the utility is used as described by this
> volume of POSIX.1-2024.
>
> Source: XCU 1.4 Utility Description Defaults, STDERR — utilities/V3_chap01.html#tag_18_04

> When this section is listed as "Not used.", it means that the standard error
> shall not be used when the utility is used as described in this volume of
> POSIX.1-2024.
>
> Source: XCU 1.4 Utility Description Defaults, STDERR — utilities/V3_chap01.html#tag_18_04

### OUTPUT FILES

> The OUTPUT FILES section completely describes the files created or modified
> by the utility. Temporary or system files that are created for internal usage
> by this utility or other parts of the implementation (for example, spool,
> log, and audit files) are not described in this, or any, section. The
> utilities creating such files and the names of such files are unspecified.
>
> Source: XCU 1.4 Utility Description Defaults, OUTPUT FILES — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.output-files.tmpdir]
> If applications are written to use temporary or intermediate files, they
> should use the TMPDIR environment variable, if it is set and represents an
> accessible directory, to select the location of temporary files.
>
> Source: XCU 1.4 Utility Description Defaults, OUTPUT FILES — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.output-files.temp-file-naming]
> Implementations shall ensure that temporary files, when used by the standard
> utilities, are named so that different utilities or multiple instances of the
> same utility can operate simultaneously without regard to their working
> directories, or any other process characteristic other than process ID. There
> are two exceptions to this rule:
>
> 1. Resources for temporary files other than the name space (for example, disk
>    space, available directory entries, or number of processes allowed) are
>    not guaranteed.
> 2. Certain standard utilities generate output files that are intended as
>    input for other utilities (for example, lex generates lex.yy.c), and these
>    cannot have unique names. These cases are explicitly identified in the
>    descriptions of the respective utilities.
>
> Source: XCU 1.4 Utility Description Defaults, OUTPUT FILES — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.output-files.temp-file-removal]
> Any temporary file created by the implementation shall be removed by the
> implementation upon a utility's successful exit, exit because of errors, or
> before termination by any of the SIGHUP, SIGINT, or SIGTERM signals, unless
> specified otherwise by the utility description.
>
> Source: XCU 1.4 Utility Description Defaults, OUTPUT FILES — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.output-files.sigquit-bypasses-recovery]
> Receipt of the SIGQUIT signal should generally cause termination (unless in
> some debugging mode) that would bypass any attempted recovery actions.
>
> Source: XCU 1.4 Utility Description Defaults, OUTPUT FILES — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.defaults.output-files-none]
> When this section is listed as "None.", it means that no files are created or
> modified as a consequence of direct action on the part of the utility when
> the utility is used as described by this volume of POSIX.1-2024. However, the
> utility may create or modify system files, such as log files, that are
> outside the utility's normal execution environment.
>
> Source: XCU 1.4 Utility Description Defaults, OUTPUT FILES — utilities/V3_chap01.html#tag_18_04

### EXTENDED DESCRIPTION

> The EXTENDED DESCRIPTION section provides a place for describing the actions
> of very complicated utilities, such as text editors or language processors,
> which typically have elaborate command languages.
>
> Source: XCU 1.4 Utility Description Defaults, EXTENDED DESCRIPTION — utilities/V3_chap01.html#tag_18_04

> When this section is listed as "None.", no further description is necessary.
>
> Source: XCU 1.4 Utility Description Defaults, EXTENDED DESCRIPTION — utilities/V3_chap01.html#tag_18_04

### EXIT STATUS

> The EXIT STATUS section describes the values the utility shall return to the
> calling program, or shell, and the conditions that cause these values to be
> returned. Usually, utilities return zero for successful completion and values
> greater than zero for various error conditions.
>
> Source: XCU 1.4 Utility Description Defaults, EXIT STATUS — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.exit-status.listed-values-binding]
> If specific numeric values are listed in this section, the system shall use
> those values for the errors described. In some cases, status values are
> listed more loosely, such as >0. A strictly conforming application shall not
> rely on any specific value in the range shown and shall be prepared to
> receive any value in the range.
>
> Source: XCU 1.4 Utility Description Defaults, EXIT STATUS — utilities/V3_chap01.html#tag_18_04

> For example, a utility may list zero as a successful return, 1 as a failure
> for a specific reason, and >1 as "an error occurred". In this case,
> unspecified conditions may cause a 2 or 3, or other value, to be returned. A
> conforming application should be written so that it tests for successful exit
> status values (zero in this case), rather than relying upon the single
> specific error value listed in this volume of POSIX.1-2024. In that way, it
> has maximum portability, even on implementations with extensions.
>
> Source: XCU 1.4 Utility Description Defaults, EXIT STATUS — utilities/V3_chap01.html#tag_18_04

> Unspecified error conditions may be represented by specific values not listed
> in this volume of POSIX.1-2024.
>
> Source: XCU 1.4 Utility Description Defaults, EXIT STATUS — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.defaults.exit-status-successful-completion]
> When the description of exit status 0 is "Successful completion", it means
> that exit status 0 shall indicate that all of the actions the utility is
> required to perform were completed successfully.
>
> Source: XCU 1.4 Utility Description Defaults, EXIT STATUS — utilities/V3_chap01.html#tag_18_04

### CONSEQUENCES OF ERRORS

> The CONSEQUENCES OF ERRORS section describes the effects on the environment,
> file systems, process state, and so on, when error conditions occur. It does
> not describe error messages produced or exit status values used.
>
> Source: XCU 1.4 Utility Description Defaults, CONSEQUENCES OF ERRORS — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.errors.failure-reasons-unspecified]
> The many reasons for failure of a utility are generally not specified by the
> utility descriptions. Utilities may terminate prematurely if they encounter:
> invalid usage of options, arguments, or environment variables; invalid usage
> of the complex syntaxes expressed in EXTENDED DESCRIPTION sections; resource
> exhaustion; difficulties accessing, creating, reading, or writing files; or
> difficulties associated with the privileges of the process.
>
> Source: XCU 1.4 Utility Description Defaults, CONSEQUENCES OF ERRORS — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.errors.operand-failure-continues]
> The following shall apply to each utility, unless otherwise stated: if the
> requested action cannot be performed on an operand representing a file,
> directory, user, process, and so on, the utility shall issue a diagnostic
> message to standard error and continue processing the next operand in
> sequence, but the final exit status shall be one that indicates an error
> occurred.
>
> For a utility that recursively traverses a file hierarchy (such as find or
> chown -R), if the requested action cannot be performed on a file or directory
> encountered in the hierarchy, the utility shall issue a diagnostic message to
> standard error and continue processing the remaining files in the hierarchy,
> but the final exit status shall be one that indicates an error occurred.
>
> **Note:** If the requested action is to write one or more pathnames in a
> format that has <newline> as a terminator or separator, and a pathname to be
> written contains any bytes that have the encoded value of a <newline>
> character, this should be treated as an action that cannot be performed. A
> future version of this standard may require that utilities treat this as an
> error.
>
> Source: XCU 1.4 Utility Description Defaults, CONSEQUENCES OF ERRORS — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.errors.option-failure]
> The following shall apply to each utility, unless otherwise stated: if the
> requested action characterized by an option or option-argument cannot be
> performed, the utility shall issue a diagnostic message to standard error and
> the exit status returned shall be one that indicates an error occurred.
>
> Source: XCU 1.4 Utility Description Defaults, CONSEQUENCES OF ERRORS — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.errors.unrecoverable-exit-status]
> The following shall apply to each utility, unless otherwise stated: when an
> unrecoverable error condition is encountered, the utility shall exit with an
> exit status that indicates an error occurred.
>
> Source: XCU 1.4 Utility Description Defaults, CONSEQUENCES OF ERRORS — utilities/V3_chap01.html#tag_18_04

> [spec:posix:req:xcu.errors.diagnostic-message-required]
> The following shall apply to each utility, unless otherwise stated: a
> diagnostic message shall be written to standard error whenever an error
> condition occurs.
>
> Source: XCU 1.4 Utility Description Defaults, CONSEQUENCES OF ERRORS — utilities/V3_chap01.html#tag_18_04

> When a utility encounters an error condition several actions are possible,
> depending on the severity of the error and the state of the utility. Included
> in the possible actions of various utilities are: deletion of temporary or
> intermediate work files; deletion of incomplete files; validity checking of
> the file system or directory.
>
> Source: XCU 1.4 Utility Description Defaults, CONSEQUENCES OF ERRORS — utilities/V3_chap01.html#tag_18_04

> When this section is listed as "Default.", it means that any changes to the
> environment, file systems, process state, and so on are unspecified.
>
> Source: XCU 1.4 Utility Description Defaults, CONSEQUENCES OF ERRORS — utilities/V3_chap01.html#tag_18_04

### Informative sections

> The APPLICATION USAGE, EXAMPLES, RATIONALE, FUTURE DIRECTIONS, SEE ALSO, and
> CHANGE HISTORY sections are informative.
>
> - The APPLICATION USAGE section gives advice to the application programmer or
>   user about the way the utility should be used.
> - The EXAMPLES section gives one or more examples of usage, where
>   appropriate.
> - The RATIONALE section contains historical information concerning the
>   contents of this volume of POSIX.1-2024 and why features were included or
>   discarded by the standard developers.
> - The FUTURE DIRECTIONS section should be used as a guide to current
>   thinking; there is not necessarily a commitment to implement all of these
>   future directions in their entirety.
> - The SEE ALSO section lists related entries.
> - The CHANGE HISTORY section shows the derivation of the entry and any
>   significant changes that have been made to it.
>
> Source: XCU 1.4 Utility Description Defaults — utilities/V3_chap01.html#tag_18_04

> In the event of conflict between an example and a normative part of the
> specification, the normative material is to be taken as correct.
>
> Source: XCU 1.4 Utility Description Defaults, EXAMPLES — utilities/V3_chap01.html#tag_18_04

> Certain of the standard utilities describe how they can invoke other
> utilities or applications, such as by passing a command string to the command
> interpreter. The external influences (STDIN, ENVIRONMENT VARIABLES, and so
> on) and external effects (STDOUT, CONSEQUENCES OF ERRORS, and so on) of such
> invoked utilities are not described in the section concerning the standard
> utility that invokes them.
>
> Source: XCU 1.4 Utility Description Defaults — utilities/V3_chap01.html#tag_18_04

## 1.5 Considerations for Utilities in Support of Files of Arbitrary Size

> [spec:posix:req:xcu.arbitrary-file-size]
> The following utilities support files of any size up to the maximum that can
> be created by the implementation. This support includes correct writing of
> file size-related values (such as file sizes and offsets, line numbers, and
> block counts) and correct interpretation of command line arguments that
> contain such values.
>
> | | | | |
> |---|---|---|---|
> | basename | cat | cd | chgrp |
> | chmod | chown | cksum | cmp |
> | cp | dd | df | dirname |
> | du | find | ln | ls |
> | mkdir | mv | pathchk | pwd |
> | rm | rmdir | sh | test |
> | touch | ulimit | | |
>
> Of these, cd, sh, and ulimit are within the scope of this corpus. The
> standard pairs each name with its one-line purpose; those are the utilities'
> own NAME descriptions and are omitted here.
>
> Source: XCU 1.5 Considerations for Utilities in Support of Files of Arbitrary Size — utilities/V3_chap01.html#tag_18_05

## 1.6 Built-In Utilities

> [spec:posix:req:xcu.builtin.regular-permitted]
> Any of the standard utilities may be implemented as regular built-in
> utilities within the command language interpreter. This is usually done to
> increase the performance of frequently used utilities or to achieve
> functionality that would be more difficult in a separate environment. The
> intrinsic utilities described in 1.7 Intrinsic Utilities are frequently
> provided as regular built-ins.
>
> Source: XCU 1.6 Built-In Utilities — utilities/V3_chap01.html#tag_18_06

> [spec:posix:req:xcu.builtin.exec-accessible]
> However, all of the standard utilities other than:
>
> - The special built-ins described in 2.15 Special Built-In Utilities
> - The intrinsic utilities named in Intrinsic Utilities, except for kill
>
> shall be implemented, regardless of whether they are also implemented as
> regular built-ins, in a manner so that they can be accessed via the exec
> family of functions as defined in the System Interfaces volume of
> POSIX.1-2024 and can be invoked directly by those standard utilities that
> require it (env, find, nice, nohup, time, xargs).
>
> Source: XCU 1.6 Built-In Utilities — utilities/V3_chap01.html#tag_18_06

## 1.7 Intrinsic Utilities

> [spec:posix:req:xcu.intrinsic-utilities]
> As described in 2.9.1.4 Command Search and Execution, intrinsic utilities are
> not subject to a PATH search during command search and execution. The
> utilities named in Intrinsic Utilities shall be intrinsic utilities.
>
> Table: Intrinsic Utilities
>
> | Intrinsic utility |
> |---|
> | alias |
> | bg |
> | cd |
> | command |
> | fc |
> | fg |
> | getopts |
> | hash |
> | jobs |
> | kill |
> | read |
> | type |
> | ulimit |
> | umask |
> | unalias |
> | wait |
>
> Source: XCU 1.7 Intrinsic Utilities — utilities/V3_chap01.html#tag_18_07

> [spec:posix:req:xcu.intrinsic.additional-implementation-defined]
> Whether any additional utility is considered an intrinsic utility is
> implementation-defined. Because applications are unable to override an
> intrinsic utility with a utility from PATH, implementations should not make
> any utility an intrinsic utility beyond the utilities in Intrinsic Utilities.
>
> Source: XCU 1.7 Intrinsic Utilities — utilities/V3_chap01.html#tag_18_07
