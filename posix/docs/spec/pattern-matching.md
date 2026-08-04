# Pattern Matching Notation

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
"SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in RFC 2119. This document
reproduces normative text from IEEE Std 1003.1-2024 (POSIX.1-2024),
Copyright © 2001-2024 The IEEE and The Open Group.

## 2.14 Pattern Matching Notation

> [spec:posix:def:pattern.notation-purpose]
> The pattern matching notation described in this section is used to specify
> patterns for matching character strings in the shell. This notation is also
> used by some other utilities (find, pax, and optionally make) and by some
> system interfaces (fnmatch(), glob(), and wordexp()). Historically, pattern
> matching notation is related to, but slightly different from, the regular
> expression notation described in XBD 9. Regular Expressions. For this reason,
> the description of the rules for this pattern matching notation are based on
> the description of regular expression notation, modified to account for the
> differences.
>
> Source: XCU 2.14 Pattern Matching Notation — utilities/V3_chap02.html#tag_19_14

> [spec:posix:req:pattern.invalid-byte-sequence-unspecified]
> If an attempt is made to use pattern matching notation to match a string that
> contains one or more bytes that do not form part of a valid character, the
> behavior is unspecified. Since pathnames can contain such bytes, portable
> applications need to ensure that the current locale is the C or POSIX locale
> when performing pattern matching (or expansion) on arbitrary pathnames.
>
> Source: XCU 2.14 Pattern Matching Notation — utilities/V3_chap02.html#tag_19_14

## 2.14.1 Patterns Matching a Single Character

> [spec:posix:syn:pattern.single-character-patterns]
> The following patterns shall match a single character: ordinary characters,
> special pattern characters, and pattern bracket expressions. The pattern
> bracket expression also shall match a single collating element.
>
> Source: XCU 2.14.1 Patterns Matching a Single Character — utilities/V3_chap02.html#tag_19_14_01

> [spec:posix:syn:pattern.backslash-escape-with-shell-quoting]
> In a pattern, or part of one, where a shell-quoting <backslash> can be used, a
> <backslash> character shall escape the following character as described in
> 2.2.1 Escape Character (Backslash), regardless of whether or not the
> <backslash> is inside a bracket expression. (The sequence `"\\"` represents one
> literal <backslash>.)
>
> Source: XCU 2.14.1 Patterns Matching a Single Character — utilities/V3_chap02.html#tag_19_14_01

> [spec:posix:syn:pattern.backslash-escape-without-shell-quoting]
> In a pattern, or part of one, where a shell-quoting <backslash> cannot be used
> to preserve the literal value of a character that would otherwise be treated as
> special:
>
> - A <backslash> character that is not inside a bracket expression shall
> preserve the literal value of the following character, unless the following
> character is in a part of the pattern where shell quoting can be used and is a
> shell quoting character, in which case the behavior is unspecified.
> - For the shell only, it is unspecified whether or not a <backslash> character
> inside a bracket expression preserves the literal value of the following
> character.
>
> Source: XCU 2.14.1 Patterns Matching a Single Character — utilities/V3_chap02.html#tag_19_14_01

> [spec:posix:req:pattern.escaping-follows-quoting-rules]
> All of the requirements and effects of quoting on ordinary, shell special, and
> special pattern characters shall apply to escaping in this context, except
> where specified otherwise. Situations where this applies include word
> expansions when a pattern used in pathname expansion is not present in the
> original word but results from an earlier expansion, or the argument to the
> find -name or -path primary as passed to find, or the pattern argument to the
> fnmatch() and glob() functions when FNM_NOESCAPE or GLOB_NOESCAPE is not set in
> flags, respectively.
>
> Source: XCU 2.14.1 Patterns Matching a Single Character — utilities/V3_chap02.html#tag_19_14_01

> [spec:posix:syn:pattern.trailing-backslash-unspecified]
> If a pattern ends with an unescaped <backslash>, the behavior is unspecified.
>
> Source: XCU 2.14.1 Patterns Matching a Single Character — utilities/V3_chap02.html#tag_19_14_01

> [spec:posix:def:pattern.ordinary-character]
> An ordinary character is a pattern that shall match itself. In a pattern, or
> part of one, where a shell-quoting <backslash> can be used, an ordinary
> character can be any character in the supported character set except for NUL,
> those special shell characters in 2.2 Quoting that require quoting, and the
> three special pattern characters described in this section. In a pattern, or
> part of one, where a shell-quoting <backslash> cannot be used to preserve the
> literal value of a character that would otherwise be treated as special, an
> ordinary character can be any character in the supported character set except
> for NUL and the three special pattern characters.
>
> Source: XCU 2.14.1 Patterns Matching a Single Character — utilities/V3_chap02.html#tag_19_14_01

> [spec:posix:req:pattern.match-by-bit-pattern]
> Matching shall be based on the bit pattern used for encoding the character, not
> on the graphic representation of the character.
>
> Source: XCU 2.14.1 Patterns Matching a Single Character — utilities/V3_chap02.html#tag_19_14_01

> [spec:posix:req:pattern.quote-to-match-literally]
> If any character (ordinary, shell special, or pattern special) is quoted, or
> escaped with a <backslash>, that pattern shall match the character itself. The
> application shall ensure that it quotes or escapes any character that would
> otherwise be treated as special, in order for it to be matched as an ordinary
> character.
>
> Source: XCU 2.14.1 Patterns Matching a Single Character — utilities/V3_chap02.html#tag_19_14_01

> [spec:posix:def:pattern.special-pattern-characters]
> When unquoted, unescaped, and not inside a bracket expression, the following
> three characters shall have special meaning in the specification of patterns:
> the <question-mark> (`?`), the <asterisk> (`*`), and the <left-square-bracket>
> (`[`).
>
> Source: XCU 2.14.1 Patterns Matching a Single Character — utilities/V3_chap02.html#tag_19_14_01

> [spec:posix:sem:pattern.question-mark]
> A <question-mark> is a pattern that shall match any character.
>
> Source: XCU 2.14.1 Patterns Matching a Single Character — utilities/V3_chap02.html#tag_19_14_01

> [spec:posix:sem:pattern.asterisk]
> An <asterisk> is a pattern that shall match multiple characters, as described
> in 2.14.2 Patterns Matching Multiple Characters.
>
> Source: XCU 2.14.1 Patterns Matching a Single Character — utilities/V3_chap02.html#tag_19_14_01

> [spec:posix:syn:pattern.bracket-expression]
> A <left-square-bracket> shall introduce a bracket expression if the characters
> following it meet the requirements for bracket expressions stated in XBD 9.3.5
> RE Bracket Expression, except that the <exclamation-mark> character (`'!'`)
> shall replace the <circumflex> character (`'^'`) in its role in a non-matching
> list in the regular expression notation. A bracket expression starting with an
> unquoted <circumflex> character produces unspecified results.
>
> Source: XCU 2.14.1 Patterns Matching a Single Character — utilities/V3_chap02.html#tag_19_14_01

> [spec:posix:sem:pattern.left-bracket-literal]
> A <left-square-bracket> that does not introduce a valid bracket expression
> shall match the character itself.
>
> Source: XCU 2.14.1 Patterns Matching a Single Character — utilities/V3_chap02.html#tag_19_14_01

## 2.14.2 Patterns Matching Multiple Characters

> [spec:posix:sem:pattern.asterisk-matches-any-string]
> The <asterisk> (`'*'`) is a pattern that shall match any string, including the
> null string.
>
> Source: XCU 2.14.2 Patterns Matching Multiple Characters — utilities/V3_chap02.html#tag_19_14_02

> [spec:posix:syn:pattern.concatenation]
> The concatenation of patterns matching a single character is a valid pattern
> that shall match the concatenation of the single characters or collating
> elements matched by each of the concatenated patterns.
>
> Source: XCU 2.14.2 Patterns Matching Multiple Characters — utilities/V3_chap02.html#tag_19_14_02

> [spec:posix:sem:pattern.asterisk-longest-match]
> The concatenation of one or more patterns matching a single character with one
> or more <asterisk> characters is a valid pattern. In such patterns, each
> <asterisk> shall match a string of zero or more characters, matching the
> greatest possible number of characters that still allows the remainder of the
> pattern to match the string.
>
> Source: XCU 2.14.2 Patterns Matching Multiple Characters — utilities/V3_chap02.html#tag_19_14_02

## 2.14.3 Patterns Used for Filename Expansion

> [spec:posix:def:pattern.filename-expansion-qualification]
> The rules described in 2.14.1 Patterns Matching a Single Character and 2.14.2
> Patterns Matching Multiple Characters are qualified by the rules in this
> section that apply when pattern matching notation is used for filename
> expansion.
>
> Source: XCU 2.14.3 Patterns Used for Filename Expansion — utilities/V3_chap02.html#tag_19_14_03

> [spec:posix:req:pattern.slash-explicit-match]
> When pattern matching notation is used for filename expansion, the <slash>
> character in a pathname shall be explicitly matched by using one or more
> <slash> characters in the pattern; it shall neither be matched by the
> <asterisk> or <question-mark> special characters nor by a bracket expression.
>
> Source: XCU 2.14.3 Patterns Used for Filename Expansion — utilities/V3_chap02.html#tag_19_14_03

> [spec:posix:syn:pattern.slash-terminates-bracket]
> <slash> characters in the pattern shall be identified before bracket
> expressions; thus, a <slash> cannot be included in a pattern bracket expression
> used for filename expansion. If a <slash> character is found following an
> unescaped <left-square-bracket> character before a corresponding
> <right-square-bracket> is found, the open bracket shall be treated as an
> ordinary character. For example, the pattern `"a[b/c]d"` does not match such
> pathnames as `abd` or `a/d`. It only matches a pathname of literally `a[b/c]d`.
>
> Source: XCU 2.14.3 Patterns Used for Filename Expansion — utilities/V3_chap02.html#tag_19_14_03

> [spec:posix:req:pattern.leading-period]
> If a filename begins with a <period> (`'.'`), the <period> shall be explicitly
> matched by using a <period> as the first character of the pattern or
> immediately following a <slash> character. The leading <period> shall not be
> matched by:
>
> - The <asterisk> or <question-mark> special characters
> - A bracket expression containing a non-matching list, such as `"[!a]"`, a
> range expression, such as `"[%-0]"`, or a character class expression, such as
> `"[[:punct:]]"`
>
> Source: XCU 2.14.3 Patterns Used for Filename Expansion — utilities/V3_chap02.html#tag_19_14_03

> [spec:posix:req:pattern.leading-period-in-bracket-unspecified]
> It is unspecified whether an explicit <period> in a bracket expression matching
> list, such as `"[.abc]"`, can match a leading <period> in a filename.
>
> Source: XCU 2.14.3 Patterns Used for Filename Expansion — utilities/V3_chap02.html#tag_19_14_03

> [spec:posix:req:pattern.filename-expansion-trigger]
> If a specified pattern contains any `'*'`, `'?'` or `'['` characters that will
> be treated as special (see 2.14.1 Patterns Matching a Single Character), it
> shall be matched against existing filenames and pathnames, as appropriate; if
> directory entries for dot and dot-dot exist, they may be ignored.
>
> Source: XCU 2.14.3 Patterns Used for Filename Expansion — utilities/V3_chap02.html#tag_19_14_03

> [spec:posix:req:pattern.directory-permissions]
> Each component that contains any `'*'`, `'?'` or `'['` characters that will be
> treated as special shall require read permission in the directory containing
> that component. Each component that contains a <backslash> that will be treated
> as special may require read permission in the directory containing that
> component. Any component, except the last, that does not contain any `'*'`,
> `'?'` or `'['` characters that will be treated as special shall require search
> permission.
>
> For example, given the pattern `/foo/bar/x*/bam`, search permission is needed
> for directories `/` and `foo`, search and read permissions are needed for
> directory `bar`, and search permission is needed for each `x*` directory.
>
> Source: XCU 2.14.3 Patterns Used for Filename Expansion — utilities/V3_chap02.html#tag_19_14_03

> [spec:posix:req:pattern.permission-errors-not-fatal]
> If these permissions are denied, or if an attempt to open or search a pathname
> as a directory, or an attempt to read an opened directory, fails because of an
> error condition that is related to file system contents, this shall not be
> considered an error and pathname expansion shall continue as if the pathname
> had named an existing directory which had been successfully opened and read, or
> searched, and no matching directory entries had been found in it. For other
> error conditions it is unspecified whether pathname expansion fails or they are
> treated the same as when permission is denied.
>
> Source: XCU 2.14.3 Patterns Used for Filename Expansion — utilities/V3_chap02.html#tag_19_14_03

> [spec:posix:req:pattern.replacement-sorted]
> If the pattern matches any existing filenames or pathnames, the pattern shall
> be replaced with those filenames and pathnames, sorted according to the
> collating sequence in effect in the current locale. If this collating sequence
> does not have a total ordering of all characters (see XBD 7.3.2 LC_COLLATE),
> any filenames or pathnames that collate equally shall be further compared
> byte-by-byte using the collating sequence for the POSIX locale.
>
> Source: XCU 2.14.3 Patterns Used for Filename Expansion — utilities/V3_chap02.html#tag_19_14_03

> [spec:posix:req:pattern.unmatched-open-bracket-unspecified]
> If the pattern contains an open bracket (`'['`) that does not introduce a
> bracket expression as in XBD 9.3.5 RE Bracket Expression, it is unspecified
> whether other unquoted `'*'`, `'?'`, `'['` or <backslash> characters within the
> same slash-delimited component of the pattern retain their special meanings or
> are treated as ordinary characters. For example, the pattern `"a*[/b*"` may
> match all filenames beginning with `'b'` in the directory `"a*["` or it may
> match all filenames beginning with `'b'` in all directories with names
> beginning with `'a'` and ending with `'['`.
>
> Source: XCU 2.14.3 Patterns Used for Filename Expansion — utilities/V3_chap02.html#tag_19_14_03

> [spec:posix:req:pattern.no-match-unchanged]
> If the pattern does not match any existing filenames or pathnames, the pattern
> string shall be left unchanged.
>
> Source: XCU 2.14.3 Patterns Used for Filename Expansion — utilities/V3_chap02.html#tag_19_14_03

> [spec:posix:req:pattern.no-special-chars-unchanged]
> If a specified pattern does not contain any `'*'`, `'?'` or `'['` characters
> that will be treated as special, the pattern string shall be left unchanged.
>
> Source: XCU 2.14.3 Patterns Used for Filename Expansion — utilities/V3_chap02.html#tag_19_14_03
