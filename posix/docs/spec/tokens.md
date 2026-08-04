# Token Recognition and Reserved Words

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
"SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in RFC 2119. This document
reproduces normative text from IEEE Std 1003.1-2024 (POSIX.1-2024),
Copyright © 2001-2024 The IEEE and The Open Group.

## 2.3 Token Recognition

> [spec:posix:req:token.input-lines]
> The shell shall read its input in terms of lines. (For details about how the
> shell reads its input, see the description of sh.) The input lines can be of
> unlimited length. These lines shall be parsed using two major modes: ordinary
> token recognition and processing of here-documents.
>
> Source: XCU 2.3 Token Recognition — utilities/V3_chap02.html#tag_19_03

> [spec:posix:req:token.here-document-mode]
> When an io_here token has been recognized by the grammar (see 2.10 Shell
> Grammar), one or more of the subsequent lines immediately following the next
> NEWLINE token form the body of a here-document and shall be parsed according
> to the rules of 2.7.4 Here-Document. Any non-NEWLINE tokens (including more
> io_here tokens) that are recognized while searching for the next NEWLINE token
> shall be saved for processing after the here-document has been parsed. If a
> saved token is an io_here token, the corresponding here-document shall start
> on the line immediately following the line containing the trailing delimiter
> of the previous here-document. If any saved token includes a <newline>
> character, the behavior is unspecified.
>
> Source: XCU 2.3 Token Recognition — utilities/V3_chap02.html#tag_19_03

> [spec:posix:syn:token.recognition-algorithm]
> When it is not processing an io_here, the shell shall break its input into
> tokens by applying the first applicable rule below to each character in turn
> in its input. At the start of input or after a previous token has just been
> delimited, the first or next token, respectively, shall start with the first
> character that has not already been included in a token and is not discarded
> according to the rules below. Once a token has started, zero or more
> characters from the input shall be appended to the token until the end of the
> token is delimited according to one of the rules below. When both the start
> and end of a token have been delimited, the characters forming the token shall
> be exactly those in the input between the two delimiters, including any
> quoting characters. If a rule below indicates that a token is delimited, and
> no characters have been included in the token, that empty token shall be
> discarded.
>
> The numbered rules are those given by `[spec:posix:syn:token.delimit-at-end-of-input]`
> through `[spec:posix:syn:token.start-new-word]`, applied in that order.
>
> Source: XCU 2.3 Token Recognition — utilities/V3_chap02.html#tag_19_03

> [spec:posix:syn:token.delimit-at-end-of-input]
> If the end of input is recognized, the current token (if any) shall be
> delimited.
>
> Source: XCU 2.3 Token Recognition, rule 1 — utilities/V3_chap02.html#tag_19_03

> [spec:posix:syn:token.operator-continue]
> If the previous character was used as part of an operator and the current
> character is not quoted and can be used with the previous characters to form
> an operator, it shall be used as part of that (operator) token.
>
> Source: XCU 2.3 Token Recognition, rule 2 — utilities/V3_chap02.html#tag_19_03

> [spec:posix:syn:token.operator-delimit]
> If the previous character was used as part of an operator and the current
> character cannot be used with the previous characters to form an operator, the
> operator containing the previous character shall be delimited.
>
> Source: XCU 2.3 Token Recognition, rule 3 — utilities/V3_chap02.html#tag_19_03

> [spec:posix:syn:token.quoting-characters]
> If the current character is an unquoted <backslash>, single-quote, or
> double-quote or is the first character of an unquoted <dollar-sign>
> single-quote sequence, it shall affect quoting for subsequent characters up to
> the end of the quoted text. The rules for quoting are as described in
> 2.2 Quoting. During token recognition no substitutions shall be actually
> performed, and the result token shall contain exactly the characters that
> appear in the input unmodified, including any embedded or enclosing quotes or
> substitution operators, between the start and the end of the quoted text. The
> token shall not be delimited by the end of the quoted field.
>
> Source: XCU 2.3 Token Recognition, rule 4 — utilities/V3_chap02.html#tag_19_03

> [spec:posix:syn:token.expansion-candidates]
> If the current character is an unquoted `'$'` or `` '`' ``, the shell shall
> identify the start of any candidates for parameter expansion (2.6.2 Parameter
> Expansion), command substitution (2.6.3 Command Substitution), or arithmetic
> expansion (2.6.4 Arithmetic Expansion) from their introductory unquoted
> character sequences: `'$'` or `"${"`, `"$("` or `` '`' ``, and `"$(("`,
> respectively. The shell shall read sufficient input to determine the end of
> the unit to be expanded (as explained in the cited sections). While processing
> the characters, if instances of expansions or quoting are found nested within
> the substitution, the shell shall recursively process them in the manner
> specified for the construct that is found. For `"$("` and `` '`' `` only, if
> instances of io_here tokens are found nested within the substitution, they
> shall be parsed according to the rules of 2.7.4 Here-Document; if the
> terminating `')'` or `` '`' `` of the substitution occurs before the NEWLINE
> token marking the start of the here-document, the behavior is unspecified. The
> characters found from the beginning of the substitution to its end, allowing
> for any recursion necessary to recognize embedded constructs, shall be
> included unmodified in the result token, including any embedded or enclosing
> substitution operators or quotes. The token shall not be delimited by the end
> of the substitution.
>
> Source: XCU 2.3 Token Recognition, rule 5 — utilities/V3_chap02.html#tag_19_03

> [spec:posix:syn:token.start-new-operator]
> If the current character is not quoted and can be used as the first character
> of a new operator, the current token (if any) shall be delimited. The current
> character shall be used as the beginning of the next (operator) token.
>
> Source: XCU 2.3 Token Recognition, rule 6 — utilities/V3_chap02.html#tag_19_03

> [spec:posix:syn:token.unquoted-blank-delimits]
> If the current character is an unquoted <blank>, any token containing the
> previous character is delimited and the current character shall be discarded.
>
> Source: XCU 2.3 Token Recognition, rule 7 — utilities/V3_chap02.html#tag_19_03

> [spec:posix:syn:token.append-to-word]
> If the previous character was part of a word, the current character shall be
> appended to that word.
>
> Source: XCU 2.3 Token Recognition, rule 8 — utilities/V3_chap02.html#tag_19_03

> [spec:posix:syn:token.comment]
> If the current character is a `'#'`, it and all subsequent characters up to,
> but excluding, the next <newline> shall be discarded as a comment. The
> <newline> that ends the line is not considered part of the comment.
>
> Source: XCU 2.3 Token Recognition, rule 9 — utilities/V3_chap02.html#tag_19_03

> [spec:posix:syn:token.start-new-word]
> The current character is used as the start of a new word.
>
> Source: XCU 2.3 Token Recognition, rule 10 — utilities/V3_chap02.html#tag_19_03

> [spec:posix:sem:token.categorization]
> Once a token is delimited, it is categorized as required by the grammar in
> 2.10 Shell Grammar.
>
> Source: XCU 2.3 Token Recognition — utilities/V3_chap02.html#tag_19_03

> [spec:posix:req:token.incremental-execution]
> In situations where the shell parses its input as a program, once a
> complete_command has been recognized by the grammar (see 2.10 Shell Grammar),
> the complete_command shall be executed before the next complete_command is
> tokenized and parsed.
>
> Source: XCU 2.3 Token Recognition — utilities/V3_chap02.html#tag_19_03

## 2.3.1 Alias Substitution

> [spec:posix:req:token.alias-substitution-conditions]
> After a token has been categorized as type TOKEN (see 2.10.1 Shell Grammar
> Lexical Conventions), including (recursively) any token resulting from an
> alias substitution, the TOKEN shall be subject to alias substitution if all of
> the following conditions are true:
>
> - The TOKEN does not contain any quoting characters.
> - The TOKEN is a valid alias name (see XBD 3.10 Alias Name).
> - An alias with that name is in effect.
> - The TOKEN did not either fully or, optionally, partially result from an
> alias substitution of the same alias name at any earlier recursion level.
> - Either the TOKEN is being considered for alias substitution because it
> follows an alias substitution whose replacement value ended with a <blank>
> or the TOKEN could be parsed as the command name word of a simple command
> (see 2.10 Shell Grammar), based on this TOKEN and the tokens (if any) that
> preceded it, but ignoring whether any subsequent characters would allow that.
>
> Source: XCU 2.3.1 Alias Substitution — utilities/V3_chap02.html#tag_19_03_01

> [spec:posix:req:token.alias-reserved-word-unspecified]
> If the TOKEN meets the conditions for alias substitution and would be
> recognized as a reserved word (see 2.4 Reserved Words) if it occurred in an
> appropriate place in the input, it is unspecified whether the TOKEN is subject
> to alias substitution.
>
> Source: XCU 2.3.1 Alias Substitution — utilities/V3_chap02.html#tag_19_03_01

> [spec:posix:req:token.alias-replacement]
> When a TOKEN is subject to alias substitution, the value of the alias shall be
> processed as if it had been read from the input instead of the TOKEN, with
> token recognition (see 2.3 Token Recognition) resuming at the start of the
> alias value. When the end of the alias value is reached, the shell may behave
> as if an additional <space> character had been read from the input after the
> TOKEN that was replaced. If it does not add this <space>, it is unspecified
> whether the current token is delimited before token recognition is applied to
> the character (if any) that followed the TOKEN in the input.
>
> Note: A future version of this standard may disallow adding this <space>.
>
> Source: XCU 2.3.1 Alias Substitution — utilities/V3_chap02.html#tag_19_03_01

> [spec:posix:req:token.alias-trailing-blank-chaining]
> If the value of the alias replacing the TOKEN ends in a <blank> that would be
> unquoted after substitution, and optionally if it ends in a <blank> that would
> be quoted after substitution, the shell shall check the next token in the
> input, if it is a TOKEN, for alias substitution; this process shall continue
> until a TOKEN is found that is not a valid alias or an alias value does not
> end in such a <blank>.
>
> Source: XCU 2.3.1 Alias Substitution — utilities/V3_chap02.html#tag_19_03_01

> [spec:posix:req:token.alias-change-timing]
> An implementation may defer the effect of a change to an alias but the change
> shall take effect no later than the completion of the currently executing
> complete_command (see 2.10 Shell Grammar). Changes to aliases shall not take
> effect out of order. Implementations may provide predefined aliases that are
> in effect when the shell is invoked.
>
> Source: XCU 2.3.1 Alias Substitution — utilities/V3_chap02.html#tag_19_03_01

> [spec:posix:req:token.alias-not-inherited]
> When used as specified by this volume of POSIX.1-2024, alias definitions shall
> not be inherited by separate invocations of the shell or by the utility
> execution environments invoked by the shell; see 2.13 Shell Execution
> Environment.
>
> Source: XCU 2.3.1 Alias Substitution — utilities/V3_chap02.html#tag_19_03_01

## 2.4 Reserved Words

> [spec:posix:def:token.reserved-words]
> Reserved words are words that have special meaning to the shell; see
> 2.9 Shell Commands. The following words shall be recognized as reserved words:
>
> | | | | |
> |---|---|---|---|
> | `!` | `do` | `esac` | `in` |
> | `{` | `done` | `fi` | `then` |
> | `}` | `elif` | `for` | `until` |
> | `case` | `else` | `if` | `while` |
>
> Source: XCU 2.4 Reserved Words — utilities/V3_chap02.html#tag_19_04

> [spec:posix:req:token.reserved-word-recognition-contexts]
> This recognition shall only occur when none of the characters is quoted and
> when the word is used as:
>
> - The first word of a command
> - The first word following one of the reserved words other than case, for, or in
> - The third word in a case command (only in is valid in this case)
> - The third word in a for command (only in and do are valid in this case)
>
> See the grammar in 2.10 Shell Grammar.
>
> Source: XCU 2.4 Reserved Words — utilities/V3_chap02.html#tag_19_04

> [spec:posix:def:token.reserved-words-optional]
> When used in circumstances where reserved words are recognized (described in
> `[spec:posix:req:token.reserved-word-recognition-contexts]`), the following
> words may be recognized as reserved words, in which case the results are
> unspecified except as described below for time:
>
> | | | | | | |
> |---|---|---|---|---|---|
> | `[[` | `]]` | `function` | `namespace` | `select` | `time` |
>
> Source: XCU 2.4 Reserved Words — utilities/V3_chap02.html#tag_19_04

> [spec:posix:req:token.reserved-word-time]
> When the word time is recognized as a reserved word in circumstances where it
> would, if it were not a reserved word, be the command name (see 2.9.1.1 Order
> of Processing) of a simple command that would execute the time utility in a
> manner other than one for which time states that the results are unspecified,
> the behavior shall be as specified for the time utility.
>
> Source: XCU 2.4 Reserved Words — utilities/V3_chap02.html#tag_19_04

> [spec:posix:def:token.reserved-words-trailing-colon]
> When used in circumstances where reserved words are recognized (described in
> `[spec:posix:req:token.reserved-word-recognition-contexts]`), all words whose
> final character is a <colon> (`':'`) are reserved; their use in those
> circumstances produces unspecified results.
>
> Source: XCU 2.4 Reserved Words — utilities/V3_chap02.html#tag_19_04
