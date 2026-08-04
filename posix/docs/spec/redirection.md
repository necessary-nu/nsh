# Redirection

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
"SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in RFC 2119. This document
reproduces normative text from IEEE Std 1003.1-2024 (POSIX.1-2024),
Copyright © 2001-2024 The IEEE and The Open Group.

## 2.7 Redirection

> [spec:posix:def:redir.purpose]
> Redirection is used to open and close files for the current shell execution
> environment (see 2.13 Shell Execution Environment) or for any command.
> Redirection operators can be used with numbers representing file descriptors
> (see XBD 3.141 File Descriptor).
>
> Source: XCU 2.7 Redirection — utilities/V3_chap02.html#tag_19_07

> [spec:posix:syn:redir.format]
> The overall format used for redirection is `[n]redir-op word`.
>
> The number n is an optional one or more digit decimal number designating the
> file descriptor number; the application shall ensure it is delimited from any
> preceding text and immediately precedes the redirection operator redir-op
> (with no intervening <blank> characters allowed).
>
> Source: XCU 2.7 Redirection — utilities/V3_chap02.html#tag_19_07

> [spec:posix:syn:redir.quoting-suppresses-recognition]
> If n is quoted, the number shall not be recognized as part of the redirection
> expression. For example, `echo \2>a` writes the character 2 into file a.
>
> If any part of redir-op is quoted, no redirection expression is recognized.
> For example, `echo 2\>a` writes the characters 2>a to standard output.
>
> Source: XCU 2.7 Redirection — utilities/V3_chap02.html#tag_19_07

> [spec:posix:req:redir.not-in-command-arguments]
> The optional number, redirection operator, and word shall not appear in the
> arguments provided to the command to be executed (if any).
>
> Source: XCU 2.7 Redirection — utilities/V3_chap02.html#tag_19_07

> [spec:posix:req:redir.location-format]
> The shell may support an additional format used for redirection:
> `{location}redir-op word`, where location is non-empty and indicates a
> location where an integer value can be stored, such as the name of a shell
> variable. If this format is supported its behavior is implementation-defined.
>
> Source: XCU 2.7 Redirection — utilities/V3_chap02.html#tag_19_07

> [spec:posix:req:redir.max-fd-number]
> The largest file descriptor number supported in shell redirections is
> implementation-defined; however, all implementations shall support at least
> 0 to 9, inclusive, for use by the application.
>
> Source: XCU 2.7 Redirection — utilities/V3_chap02.html#tag_19_07

> [spec:posix:req:redir.word-expansion]
> If the redirection operator is `"<<"` or `"<<-"`, the word that follows the
> redirection operator shall be subjected to quote removal; it is unspecified
> whether any of the other expansions occur.
>
> For the other redirection operators, the word that follows the redirection
> operator shall be subjected to tilde expansion, parameter expansion, command
> substitution, arithmetic expansion, and quote removal.
>
> Source: XCU 2.7 Redirection — utilities/V3_chap02.html#tag_19_07

> [spec:posix:req:redir.word-pathname-expansion]
> Pathname expansion shall not be performed on the word by a non-interactive
> shell; an interactive shell may perform it, but if the expansion would result
> in more than one word it is unspecified whether the redirection proceeds
> without pathname expansion being performed or the redirection fails.
>
> Source: XCU 2.7 Redirection — utilities/V3_chap02.html#tag_19_07

> [spec:posix:sem:redir.evaluation-order]
> If more than one redirection operator is specified with a command, the order
> of evaluation is from beginning to end.
>
> Source: XCU 2.7 Redirection — utilities/V3_chap02.html#tag_19_07

> [spec:posix:req:redir.open-failure]
> A failure to open or create a file shall cause a redirection to fail.
>
> Source: XCU 2.7 Redirection — utilities/V3_chap02.html#tag_19_07

## 2.7.1 Redirecting Input

> [spec:posix:req:redir.input]
> Input redirection shall cause the file whose name results from the expansion
> of word to be opened for reading on the designated file descriptor, or
> standard input if the file descriptor is not specified.
>
> The general format for redirecting input is `[n]<word`, where the optional n
> represents the file descriptor number. If the number is omitted, the
> redirection shall refer to standard input (file descriptor 0).
>
> Source: XCU 2.7.1 Redirecting Input — utilities/V3_chap02.html#tag_19_07_01

## 2.7.2 Redirecting Output

> [spec:posix:syn:redir.output-format]
> The two general formats for redirecting output are `[n]>word` and
> `[n]>|word`, where the optional n represents the file descriptor number. If
> the number is omitted, the redirection shall refer to standard output (file
> descriptor 1).
>
> Source: XCU 2.7.2 Redirecting Output — utilities/V3_chap02.html#tag_19_07_02

> [spec:posix:req:redir.output-noclobber]
> Output redirection using the `'>'` format shall fail if the noclobber option
> is set (see the description of set -C) and the file named by the expansion of
> word exists and is either a regular file or a symbolic link that resolves to a
> regular file; it may also fail if the file is a symbolic link that does not
> resolve to an existing file.
>
> Source: XCU 2.7.2 Redirecting Output — utilities/V3_chap02.html#tag_19_07_02

> [spec:posix:req:redir.output-noclobber-atomicity]
> The check for existence, file creation, and open operations shall be performed
> atomically as is done by the open() function as defined in the System
> Interfaces volume of POSIX.1-2024 when the O_CREAT and O_EXCL flags are set,
> except that if the file exists and is a symbolic link, the open operation need
> not fail with [EEXIST] unless the symbolic link resolves to an existing
> regular file. Performing these operations atomically ensures that the creation
> of lock files and unique (often temporary) files is reliable.
>
> The check for the type of the file need not be performed atomically with the
> check for existence, file creation, and open operations. If not, there is a
> potential race condition that may result in a misleading shell diagnostic
> message when redirection fails.
>
> Source: XCU 2.7.2 Redirecting Output — utilities/V3_chap02.html#tag_19_07_02

> [spec:posix:req:redir.output-truncate]
> In all other cases (noclobber not set, redirection using `'>'` does not fail
> for the reasons stated above, or redirection using the `">|"` format), output
> redirection shall cause the file whose name results from the expansion of word
> to be opened for output on the designated file descriptor, or standard output
> if none is specified. If the file does not exist, it shall be created as an
> empty file; otherwise, it shall be opened as if the open() function was called
> with the O_TRUNC flag set.
>
> Source: XCU 2.7.2 Redirecting Output — utilities/V3_chap02.html#tag_19_07_02

## 2.7.3 Appending Redirected Output

> [spec:posix:req:redir.append]
> Appended output redirection shall cause the file whose name results from the
> expansion of word to be opened for output on the designated file descriptor.
> The file shall be opened as if the open() function as defined in the System
> Interfaces volume of POSIX.1-2024 was called with the O_APPEND flag set. If
> the file does not exist, it shall be created.
>
> Source: XCU 2.7.3 Appending Redirected Output — utilities/V3_chap02.html#tag_19_07_03

> [spec:posix:syn:redir.append-format]
> The general format for appending redirected output is `[n]>>word`, where the
> optional n represents the file descriptor number. If the number is omitted,
> the redirection refers to standard output (file descriptor 1).
>
> Source: XCU 2.7.3 Appending Redirected Output — utilities/V3_chap02.html#tag_19_07_03

## 2.7.4 Here-Document

> [spec:posix:def:redir.here-doc]
> The redirection operators `"<<"` and `"<<-"` both allow redirection of
> subsequent lines read by the shell to the input of a command. The redirected
> lines are known as a "here-document".
>
> Source: XCU 2.7.4 Here-Document — utilities/V3_chap02.html#tag_19_07_04

> [spec:posix:req:redir.here-doc-delimiter]
> The here-document shall be treated as a single word that begins after the next
> NEWLINE token and continues until there is a line containing only the
> delimiter and a <newline>, with no <blank> characters in between. Then the
> next here-document starts, if there is one.
>
> For the purposes of locating this terminating line, the end of a
> command_string operand (see sh) shall be treated as a <newline> character, and
> the end of the commands string in `$(commands)` and `` `commands` `` may be
> treated as a <newline>. If the end of input is reached without finding the
> terminating line, the shell should, but need not, treat this as a redirection
> error.
>
> Source: XCU 2.7.4 Here-Document — utilities/V3_chap02.html#tag_19_07_04

> [spec:posix:syn:redir.here-doc-format]
> The format of a here-document redirection is `[n]<<word`, followed on
> subsequent lines by the here-document itself and then by a line containing the
> delimiter, where the optional n represents the file descriptor number. If the
> number is omitted, the here-document refers to standard input (file descriptor
> 0).
>
> Source: XCU 2.7.4 Here-Document — utilities/V3_chap02.html#tag_19_07_04

> [spec:posix:sem:redir.here-doc-fd-type]
> It is unspecified whether the file descriptor a here-document is supplied on
> is opened as a regular file or some other type of file. Portable applications
> cannot rely on the file descriptor being seekable (see XSH lseek()).
>
> Source: XCU 2.7.4 Here-Document — utilities/V3_chap02.html#tag_19_07_04

> [spec:posix:req:redir.here-doc-quoted-delimiter]
> If any part of word is quoted, not counting double-quotes outside a command
> substitution if the here-document is inside one, the delimiter shall be formed
> by performing quote removal on word, and the here-document lines shall not be
> expanded.
>
> Source: XCU 2.7.4 Here-Document — utilities/V3_chap02.html#tag_19_07_04

> [spec:posix:req:redir.here-doc-unquoted-delimiter]
> Otherwise (no part of word is quoted, not counting double-quotes outside a
> command substitution if the here-document is inside one), the delimiter shall
> be the word itself.
>
> Source: XCU 2.7.4 Here-Document — utilities/V3_chap02.html#tag_19_07_04

> [spec:posix:req:redir.here-doc-line-continuation]
> If no part of word is quoted, the removal of <backslash><newline> for line
> continuation (see 2.2.1 Escape Character (Backslash)) shall be performed
> during the search for the trailing delimiter. (As a consequence, the trailing
> delimiter is not recognized immediately after a <newline> that was removed by
> line continuation.) It is unspecified whether the line containing the trailing
> delimiter is itself subject to this line continuation.
>
> Source: XCU 2.7.4 Here-Document — utilities/V3_chap02.html#tag_19_07_04

> [spec:posix:req:redir.here-doc-expansion]
> If no part of word is quoted, all lines of the here-document shall be
> expanded, when the redirection operator is evaluated but after the trailing
> delimiter for the here-document has been located, for parameter expansion,
> command substitution, and arithmetic expansion. If the redirection operator is
> never evaluated (because the command it is part of is not executed), the
> here-document shall be read without performing any expansions.
>
> Source: XCU 2.7.4 Here-Document — utilities/V3_chap02.html#tag_19_07_04

> [spec:posix:req:redir.here-doc-backslash]
> If no part of word is quoted, any <backslash> characters in the input shall
> behave as the <backslash> inside double-quotes (see 2.2.3 Double-Quotes).
> However, the double-quote character (`'"'`) shall not be treated specially
> within a here-document, except when the double-quote appears within
> `"$()"`, ``` "``" ```, or `"${}"`.
>
> Source: XCU 2.7.4 Here-Document — utilities/V3_chap02.html#tag_19_07_04

> [spec:posix:req:redir.here-doc-tab-strip]
> If the redirection operator is `"<<-"`, all leading <tab> characters shall be
> stripped from input lines after <backslash><newline> line continuation (when
> it applies) has been performed, and from the line containing the trailing
> delimiter. Stripping of leading <tab> characters shall occur as the
> here-document is read from the shell input (and consequently does not affect
> any <tab> characters that result from expansions).
>
> Source: XCU 2.7.4 Here-Document — utilities/V3_chap02.html#tag_19_07_04

> [spec:posix:req:redir.here-doc-multiple]
> If more than one `"<<"` or `"<<-"` operator is specified on a line, the
> here-document associated with the first operator shall be supplied first by
> the application and shall be read first by the shell.
>
> Source: XCU 2.7.4 Here-Document — utilities/V3_chap02.html#tag_19_07_04

> [spec:posix:req:redir.here-doc-ps2]
> When a here-document is read from a terminal device and the shell is
> interactive, it shall write the contents of the variable PS2, processed as
> described in 2.5.3 Shell Variables, to standard error before reading each line
> of input until the delimiter has been recognized.
>
> Source: XCU 2.7.4 Here-Document — utilities/V3_chap02.html#tag_19_07_04

## 2.7.5 Duplicating an Input File Descriptor

> [spec:posix:req:redir.dup-input]
> The redirection operator `[n]<&word` shall duplicate one input file descriptor
> from another, or shall close one.
>
> If word evaluates to one or more digits, the file descriptor denoted by n, or
> standard input if n is not specified, shall be made to be a copy of the file
> descriptor denoted by word; if the digits in word do not represent an already
> open file descriptor, a redirection error shall result (see 2.8.1 Consequences
> of Shell Errors); if the file descriptor denoted by word represents an open
> file descriptor that is not open for input, a redirection error may result.
>
> Source: XCU 2.7.5 Duplicating an Input File Descriptor — utilities/V3_chap02.html#tag_19_07_05

> [spec:posix:req:redir.dup-input-close]
> If word in `[n]<&word` evaluates to `'-'`, file descriptor n, or standard
> input if n is not specified, shall be closed. Attempts to close a file
> descriptor that is not open shall not constitute an error. If word evaluates
> to something else, the behavior is unspecified.
>
> Source: XCU 2.7.5 Duplicating an Input File Descriptor — utilities/V3_chap02.html#tag_19_07_05

## 2.7.6 Duplicating an Output File Descriptor

> [spec:posix:req:redir.dup-output]
> The redirection operator `[n]>&word` shall duplicate one output file
> descriptor from another, or shall close one.
>
> If word evaluates to one or more digits, the file descriptor denoted by n, or
> standard output if n is not specified, shall be made to be a copy of the file
> descriptor denoted by word; if the digits in word do not represent an already
> open file descriptor, a redirection error shall result (see 2.8.1 Consequences
> of Shell Errors); if the file descriptor denoted by word represents an open
> file descriptor that is not open for output, a redirection error may result.
>
> Source: XCU 2.7.6 Duplicating an Output File Descriptor — utilities/V3_chap02.html#tag_19_07_06

> [spec:posix:req:redir.dup-output-close]
> If word in `[n]>&word` evaluates to `'-'`, file descriptor n, or standard
> output if n is not specified, is closed. Attempts to close a file descriptor
> that is not open shall not constitute an error. If word evaluates to something
> else, the behavior is unspecified.
>
> Source: XCU 2.7.6 Duplicating an Output File Descriptor — utilities/V3_chap02.html#tag_19_07_06

## 2.7.7 Open File Descriptors for Reading and Writing

> [spec:posix:req:redir.open-read-write]
> The redirection operator `[n]<>word` shall cause the file whose name is the
> expansion of word to be opened for both reading and writing on the file
> descriptor denoted by n, or standard input if n is not specified. If the file
> does not exist, it shall be created.
>
> Source: XCU 2.7.7 Open File Descriptors for Reading and Writing — utilities/V3_chap02.html#tag_19_07_07
