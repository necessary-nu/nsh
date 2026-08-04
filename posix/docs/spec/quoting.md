# Shell Introduction and Quoting

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
"SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in RFC 2119. This document
reproduces normative text from IEEE Std 1003.1-2024 (POSIX.1-2024),
Copyright © 2001-2024 The IEEE and The Open Group.

## 2.1 Shell Introduction

> [spec:posix:def:shell.command-language-interpreter]
> The shell is a command language interpreter. This chapter describes the syntax
> of that command language as it is used by the sh utility and the system() and
> popen() functions defined in the System Interfaces volume of POSIX.1-2024.
>
> Source: XCU 2.1 Shell Introduction — utilities/V3_chap02.html#tag_19_01

> [spec:posix:sem:shell.input-sources]
> The shell operates according to the following general overview of operations.
> The specific details are included in the cited sections of this chapter.
>
> The shell reads its input from a file (see sh), from the -c option or from the
> system() and popen() functions defined in the System Interfaces volume of
> POSIX.1-2024.
>
> Source: XCU 2.1 Shell Introduction — utilities/V3_chap02.html#tag_19_01

> [spec:posix:req:shell.hashbang-unspecified]
> If the first line of a file of shell commands starts with the characters
> `"#!"`, the results are unspecified.
>
> Source: XCU 2.1 Shell Introduction — utilities/V3_chap02.html#tag_19_01

> [spec:posix:sem:shell.tokenization-and-parsing]
> The shell breaks the input into tokens: words and operators; see 2.3 Token
> Recognition.
>
> The shell parses the input into simple commands (see 2.9.1 Simple Commands)
> and compound commands (see 2.9.4 Compound Commands).
>
> Source: XCU 2.1 Shell Introduction — utilities/V3_chap02.html#tag_19_01

> [spec:posix:sem:shell.word-processing]
> For each word within a command, the shell processes <backslash>-escape
> sequences inside dollar-single-quotes (see 2.2.4 Dollar-Single-Quotes) and
> then performs various word expansions (see 2.6 Word Expansions). In the case
> of a simple command, the results usually include a list of pathnames and
> fields to be treated as a command name and arguments; see 2.9 Shell Commands.
>
> Source: XCU 2.1 Shell Introduction — utilities/V3_chap02.html#tag_19_01

> [spec:posix:sem:shell.redirection-processing]
> The shell performs redirection (see 2.7 Redirection) and removes redirection
> operators and their operands from the parameter list.
>
> Source: XCU 2.1 Shell Introduction — utilities/V3_chap02.html#tag_19_01

> [spec:posix:sem:shell.command-execution]
> The shell executes a function (see 2.9.5 Function Definition Command),
> built-in (see 2.15 Special Built-In Utilities), executable file, or script,
> giving the names of the arguments as positional parameters numbered 1 to n,
> and the name of the command (or in the case of a function within a script, the
> name of the script) as special parameter 0 (see 2.9.1.4 Command Search and
> Execution).
>
> Source: XCU 2.1 Shell Introduction — utilities/V3_chap02.html#tag_19_01

> [spec:posix:sem:shell.exit-status-collection]
> The shell optionally waits for the command to complete and collects the exit
> status (see 2.8.2 Exit Status for Commands).
>
> Source: XCU 2.1 Shell Introduction — utilities/V3_chap02.html#tag_19_01

## 2.2 Quoting

> [spec:posix:def:quote.purpose]
> Quoting is used to remove the special meaning of certain characters or words
> to the shell. Quoting can be used to preserve the literal meaning of the
> special characters in the next paragraph, prevent reserved words from being
> recognized as such, and prevent parameter expansion and command substitution
> within here-document processing (see 2.7.4 Here-Document).
>
> Source: XCU 2.2 Quoting — utilities/V3_chap02.html#tag_19_02

> [spec:posix:req:quote.always-special-characters]
> The application shall quote the following characters if they are to represent
> themselves:
>
> `|` `&` `;` `<` `>` `(` `)` `$` `` ` `` `\` `"` `'` <space> <tab> <newline>
>
> Source: XCU 2.2 Quoting — utilities/V3_chap02.html#tag_19_02

> [spec:posix:req:quote.conditionally-special-characters]
> The following characters might need to be quoted under certain circumstances.
> That is, these characters are sometimes special depending on conditions
> described elsewhere in this volume of POSIX.1-2024:
>
> `*` `?` `[` `]` `^` `-` `!` `#` `~` `=` `%` `{` `,` `}`
>
> Source: XCU 2.2 Quoting — utilities/V3_chap02.html#tag_19_02

> [spec:posix:req:quote.future-special-characters]
> A future version of this standard may extend the conditions under which the
> conditionally-special characters are special. Therefore applications should
> quote them whenever they are intended to represent themselves. This does not
> apply to <hyphen-minus> (`'-'`) since it is in the portable filename character
> set.
>
> Source: XCU 2.2 Quoting — utilities/V3_chap02.html#tag_19_02

> [spec:posix:def:quote.mechanisms]
> The various quoting mechanisms are the escape character, single-quotes,
> double-quotes, and dollar-single-quotes. The here-document represents another
> form of quoting; see 2.7.4 Here-Document.
>
> Source: XCU 2.2 Quoting — utilities/V3_chap02.html#tag_19_02

### 2.2.1 Escape Character (Backslash)

> [spec:posix:req:quote.backslash-literal]
> A <backslash> that is not quoted shall preserve the literal value of the
> following character, with the exception of a <newline>.
>
> Source: XCU 2.2.1 Escape Character (Backslash) — utilities/V3_chap02.html#tag_19_02_01

> [spec:posix:req:quote.backslash-newline]
> If a <newline> immediately follows the <backslash>, the shell shall interpret
> this as line continuation. The <backslash> and <newline> shall be removed
> before splitting the input into tokens. Since the escaped <newline> is removed
> entirely from the input and is not replaced by any white space, it cannot
> serve as a token separator.
>
> Source: XCU 2.2.1 Escape Character (Backslash) — utilities/V3_chap02.html#tag_19_02_01

### 2.2.2 Single-Quotes

> [spec:posix:req:quote.single-quotes]
> Enclosing characters in single-quotes (`''`) shall preserve the literal value
> of each character within the single-quotes. A single-quote cannot occur within
> single-quotes.
>
> Source: XCU 2.2.2 Single-Quotes — utilities/V3_chap02.html#tag_19_02_02

### 2.2.3 Double-Quotes

> [spec:posix:req:quote.double-quotes-literal]
> Enclosing characters in double-quotes (`""`) shall preserve the literal value
> of all characters within the double-quotes, with the exception of the
> characters backquote, <dollar-sign>, and <backslash>, as follows.
>
> Source: XCU 2.2.3 Double-Quotes — utilities/V3_chap02.html#tag_19_02_03

> [spec:posix:req:quote.double-quotes-dollar-sign]
> Within double-quotes, the <dollar-sign> shall retain its special meaning
> introducing parameter expansion (see 2.6.2 Parameter Expansion), a form of
> command substitution (see 2.6.3 Command Substitution), and arithmetic
> expansion (see 2.6.4 Arithmetic Expansion), but shall not retain its special
> meaning introducing the dollar-single-quotes form of quoting (see 2.2.4
> Dollar-Single-Quotes).
>
> Source: XCU 2.2.3 Double-Quotes — utilities/V3_chap02.html#tag_19_02_03

> [spec:posix:req:quote.double-quotes-command-substitution]
> The input characters within the quoted string that are also enclosed between
> `"$("` and the matching `')'` shall not be affected by the double-quotes, but
> rather shall define the command(s) whose output replaces the `"$(...)"` when
> the word is expanded. The tokenizing rules in 2.3 Token Recognition shall be
> applied recursively to find the matching `')'`.
>
> Source: XCU 2.2.3 Double-Quotes — utilities/V3_chap02.html#tag_19_02_03

> [spec:posix:req:quote.double-quotes-substring-parameter-expansion]
> For the four varieties of parameter expansion that provide for substring
> processing (see 2.6.2 Parameter Expansion), within the string of characters
> from an enclosed `"${"` to the matching `'}'`, the double-quotes within which
> the expansion occurs shall have no effect on the handling of any special
> characters.
>
> Source: XCU 2.2.3 Double-Quotes — utilities/V3_chap02.html#tag_19_02_03

> [spec:posix:req:quote.double-quotes-other-parameter-expansion]
> For parameter expansions other than the four varieties that provide for
> substring processing, within the string of characters from an enclosed `"${"`
> to the matching `'}'`, the double-quotes within which the expansion occurs
> shall preserve the literal value of all characters, with the exception of the
> characters double-quote, backquote, <dollar-sign>, and <backslash>. If any
> unescaped double-quote characters occur within the string, other than in
> embedded command substitutions, the behavior is unspecified. The backquote and
> <dollar-sign> characters shall follow the same rules as for characters in
> double-quotes described in this section. The <backslash> character shall
> follow the same rules as for characters in double-quotes described in this
> section except that it shall additionally retain its special meaning as an
> escape character when followed by `'}'` and this shall prevent the escaped
> `'}'` from being considered when determining the matching `'}'` (using the
> rule in 2.6.2 Parameter Expansion).
>
> Source: XCU 2.2.3 Double-Quotes — utilities/V3_chap02.html#tag_19_02_03

> [spec:posix:req:quote.double-quotes-backquote]
> Within double-quotes, the backquote shall retain its special meaning
> introducing the other form of command substitution (see 2.6.3 Command
> Substitution). The portion of the quoted string from the initial backquote and
> the characters up to the next backquote that is not preceded by a <backslash>,
> having escape characters removed, defines that command whose output replaces
> `` `...` `` when the word is expanded.
>
> Source: XCU 2.2.3 Double-Quotes — utilities/V3_chap02.html#tag_19_02_03

> [spec:posix:req:quote.double-quotes-backquote-undefined]
> Either of the following cases produces undefined results:
>
> - A quoted (single-quoted, double-quoted, or dollar-single-quoted) string that
> begins, but does not end, within the `` `...` `` sequence.
>
> - A `` `...` `` sequence that begins, but does not end, within the same
> double-quoted string.
>
> Source: XCU 2.2.3 Double-Quotes — utilities/V3_chap02.html#tag_19_02_03

> [spec:posix:req:quote.double-quotes-backslash]
> Outside of `"$(...)"` and `"${...}"` the <backslash> shall retain its special
> meaning as an escape character (see 2.2.1 Escape Character (Backslash)) only
> when immediately followed by one of the following characters:
>
> `$` `` ` `` `\` <newline>
>
> or by a double-quote character that would otherwise be considered special (see
> 2.6.4 Arithmetic Expansion and 2.7.4 Here-Document).
>
> Source: XCU 2.2.3 Double-Quotes — utilities/V3_chap02.html#tag_19_02_03

> [spec:posix:req:quote.double-quotes-expansion-result]
> When double-quotes are used to quote a parameter expansion, command
> substitution, or arithmetic expansion, the literal value of all characters
> within the result of the expansion shall be preserved.
>
> Source: XCU 2.2.3 Double-Quotes — utilities/V3_chap02.html#tag_19_02_03

> [spec:posix:req:quote.double-quotes-embedded-double-quote]
> The application shall ensure that a double-quote that is not within `"$(...)"`
> nor within `"${...}"` is immediately preceded by a <backslash> in order to be
> included within double-quotes. The parameter `'@'` has special meaning inside
> double-quotes and is described in 2.5.2 Special Parameters.
>
> Source: XCU 2.2.3 Double-Quotes — utilities/V3_chap02.html#tag_19_02_03

### 2.2.4 Dollar-Single-Quotes

> [spec:posix:req:quote.dollar-single-quotes]
> A sequence of characters starting with a <dollar-sign> immediately followed by
> a single-quote (`$'`) shall preserve the literal value of all characters up to
> an unescaped terminating single-quote (`'`), with the exception of certain
> <backslash>-escape sequences.
>
> Source: XCU 2.2.4 Dollar-Single-Quotes — utilities/V3_chap02.html#tag_19_02_04

> [spec:posix:def:quote.dollar-single-quotes-escapes]
> Within dollar-single-quotes the following <backslash>-escape sequences are
> recognized:
>
> - `\"` yields a <quotation-mark> (double-quote) character, but note that
> <quotation-mark> can be included unescaped.
> - `\'` yields an <apostrophe> (single-quote) character.
> - `\\` yields a <backslash> character.
> - `\a` yields an <alert> character.
> - `\b` yields a <backspace> character.
> - `\e` yields an <ESC> character.
> - `\f` yields a <form-feed> character.
> - `\n` yields a <newline> character.
> - `\r` yields a <carriage-return> character.
> - `\t` yields a <tab> character.
> - `\v` yields a <vertical-tab> character.
>
> Source: XCU 2.2.4 Dollar-Single-Quotes — utilities/V3_chap02.html#tag_19_02_04

> [spec:posix:def:quote.dollar-single-quotes-control-escape]
> Within dollar-single-quotes, `\c`X yields the control character listed in the
> Value column of Values for cpio c_mode Field in the OPERANDS section of the
> stty utility when X is one of the characters listed in the ^c column of the
> same table, except that `\c\\` yields the <FS> control character since the
> <backslash> character has to be escaped.
>
> Source: XCU 2.2.4 Dollar-Single-Quotes — utilities/V3_chap02.html#tag_19_02_04

> [spec:posix:def:quote.dollar-single-quotes-hex-escape]
> Within dollar-single-quotes, `\x`XX yields the byte whose value is the
> hexadecimal value XX (one or more hexadecimal digits). If more than two
> hexadecimal digits follow `\x`, the results are unspecified.
>
> Source: XCU 2.2.4 Dollar-Single-Quotes — utilities/V3_chap02.html#tag_19_02_04

> [spec:posix:def:quote.dollar-single-quotes-octal-escape]
> Within dollar-single-quotes, `\`ddd yields the byte whose value is the octal
> value ddd (one to three octal digits).
>
> Source: XCU 2.2.4 Dollar-Single-Quotes — utilities/V3_chap02.html#tag_19_02_04

> [spec:posix:req:quote.dollar-single-quotes-undefined-escape]
> The behavior of an unescaped <backslash> immediately followed by any other
> character, including <newline>, is unspecified.
>
> Source: XCU 2.2.4 Dollar-Single-Quotes — utilities/V3_chap02.html#tag_19_02_04

> [spec:posix:syn:quote.dollar-single-quotes-escape-termination]
> In cases where a variable number of characters can be used to specify an
> escape sequence (`\x`XX and `\`ddd), the escape sequence shall be terminated
> by the first character that is not of the expected type or, for `\`ddd
> sequences, when the maximum number of characters specified has been found,
> whichever occurs first.
>
> Source: XCU 2.2.4 Dollar-Single-Quotes — utilities/V3_chap02.html#tag_19_02_04

> [spec:posix:req:quote.dollar-single-quotes-processing-time]
> These <backslash>-escape sequences shall be processed (replaced with the bytes
> or characters they yield) immediately prior to word expansion (see 2.6 Word
> Expansions) of the word in which the dollar-single-quotes sequence occurs.
>
> Source: XCU 2.2.4 Dollar-Single-Quotes — utilities/V3_chap02.html#tag_19_02_04

> [spec:posix:req:quote.dollar-single-quotes-null-byte]
> If a `\x`XX or `\`ddd escape sequence yields a byte whose value is 0, it is
> unspecified whether that null byte is included in the result or if that byte
> and any following regular characters and escape sequences up to the
> terminating unescaped single-quote are evaluated and discarded.
>
> Source: XCU 2.2.4 Dollar-Single-Quotes — utilities/V3_chap02.html#tag_19_02_04

> [spec:posix:req:quote.dollar-single-quotes-octal-overflow]
> If the octal value specified by `\`ddd will not fit in a byte, the results are
> unspecified.
>
> Source: XCU 2.2.4 Dollar-Single-Quotes — utilities/V3_chap02.html#tag_19_02_04

> [spec:posix:req:quote.dollar-single-quotes-unencodable]
> If a `\e` or `\c`X escape sequence specifies a character that does not have an
> encoding in the locale in effect when these <backslash>-escape sequences are
> processed, the result is implementation-defined. However, implementations
> shall not replace an unsupported character with bytes that do not form valid
> characters in that locale's character set.
>
> Source: XCU 2.2.4 Dollar-Single-Quotes — utilities/V3_chap02.html#tag_19_02_04

> [spec:posix:req:quote.dollar-single-quotes-quote-escape-not-terminator]
> If a <backslash>-escape sequence represents a single-quote character (for
> example `\'`), that sequence shall not terminate the dollar-single-quote
> sequence.
>
> Source: XCU 2.2.4 Dollar-Single-Quotes — utilities/V3_chap02.html#tag_19_02_04
