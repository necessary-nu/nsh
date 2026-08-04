# Shell Grammar

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
"SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in RFC 2119. This document
reproduces normative text from IEEE Std 1003.1-2024 (POSIX.1-2024),
Copyright © 2001-2024 The IEEE and The Open Group.

## 2.10 Shell Grammar

> [spec:posix:req:grammar.formal-syntax-precedence]
> The following grammar defines the Shell Command Language. This formal syntax
> shall take precedence over the preceding text syntax description.
>
> Source: XCU 2.10 Shell Grammar — utilities/V3_chap02.html#tag_19_10

## 2.10.1 Shell Grammar Lexical Conventions

> [spec:posix:syn:grammar.token-classification]
> The input language to the shell shall be first recognized at the character
> level. The resulting tokens shall be classified by their immediate context
> according to the following rules (applied in order). These rules shall be
> used to determine what a "token" is that is subject to parsing at the token
> level. The rules for token recognition in 2.3 Token Recognition shall apply.
>
> 1. If the token is an operator, the token identifier for that operator shall
> result.
> 2. If the string consists solely of digits and the delimiter character is one
> of `'<'` or `'>'`, the token identifier IO_NUMBER shall result.
> 3. If the string contains at least three characters, begins with a
> <left-curly-bracket> (`'{'`) and ends with a <right-curly-bracket> (`'}'`),
> and the delimiter character is one of `'<'` or `'>'`, the token identifier
> IO_LOCATION may result; if the result is not IO_LOCATION, the token
> identifier TOKEN shall result.
> 4. Otherwise, the token identifier TOKEN shall result.
>
> Source: XCU 2.10.1 Shell Grammar Lexical Conventions — utilities/V3_chap02.html#tag_19_10_01

> [spec:posix:syn:grammar.token-context-dependent-distinction]
> Further distinction on TOKEN is context-dependent. It may be that the same
> TOKEN yields WORD, a NAME, an ASSIGNMENT_WORD, or one of the reserved words
> below, dependent upon the context. Some of the productions in the grammar
> below are annotated with a rule number from the following list. When a TOKEN
> is seen where one of those annotated productions could be used to reduce the
> symbol, the applicable rule shall be applied to convert the token identifier
> type of the TOKEN to:
>
> - The token identifier of the recognized reserved word, for rule 1
> - A token identifier acceptable at that point in the grammar, for all other
> rules
>
> Source: XCU 2.10.1 Shell Grammar Lexical Conventions — utilities/V3_chap02.html#tag_19_10_01

> [spec:posix:req:grammar.highest-numbered-rule-applies]
> The reduction shall then proceed based upon the token identifier type yielded
> by the rule applied. When more than one rule applies, the highest numbered
> rule shall apply (which in turn may refer to another rule). (Note that except
> in rule 7, the presence of an `'='` in the token has no effect.)
>
> Source: XCU 2.10.1 Shell Grammar Lexical Conventions — utilities/V3_chap02.html#tag_19_10_01

> [spec:posix:req:grammar.word-expansion-timing]
> The WORD tokens shall have the word expansion rules applied to them
> immediately before the associated command is executed, not at the time the
> command is parsed.
>
> Source: XCU 2.10.1 Shell Grammar Lexical Conventions — utilities/V3_chap02.html#tag_19_10_01

## 2.10.2 Shell Grammar Rules

> [spec:posix:syn:grammar.command-name]
> Rule 1 [Command Name].
>
> When the TOKEN is exactly a reserved word, the token identifier for that
> reserved word shall result. Otherwise, the token WORD shall be returned.
> Also, if the parser is in any state where only a reserved word could be the
> next correct token, proceed as above.
>
> Note: Because at this point quoting characters (<backslash>, single-quote,
> <quotation-mark>, and the <dollar-sign> single-quote sequence) are retained
> in the token, quoted strings cannot be recognized as reserved words. This
> rule also implies that reserved words are not recognized except in certain
> positions in the input, such as after a <newline> or <semicolon>; the grammar
> presumes that if the reserved word is intended, it is properly delimited by
> the user, and does not attempt to reflect that requirement directly. Also
> note that line joining is done before tokenization, as described in 2.2.1
> Escape Character (Backslash), so escaped <newline> characters are already
> removed at this point.
>
> Rule 1 is not directly referenced in the grammar, but is referred to by other
> rules, or applies globally.
>
> Source: XCU 2.10.2 Shell Grammar Rules — utilities/V3_chap02.html#tag_19_10_02

> [spec:posix:req:grammar.redirection-filename]
> Rule 2 [Redirection to or from filename].
>
> The expansions specified in 2.7 Redirection shall occur. As specified there,
> exactly one field can result (or the result is unspecified), and there are
> additional requirements on pathname expansion.
>
> This rule is applied at the `filename : WORD` production.
>
> Source: XCU 2.10.2 Shell Grammar Rules — utilities/V3_chap02.html#tag_19_10_02

> [spec:posix:req:grammar.here-doc-redirection]
> Rule 3 [Redirection from here-document].
>
> Quote removal shall be applied to the word to determine the delimiter that is
> used to find the end of the here-document that begins after the next
> <newline>.
>
> This rule is applied at the `here_end : WORD` production.
>
> Source: XCU 2.10.2 Shell Grammar Rules — utilities/V3_chap02.html#tag_19_10_02

> [spec:posix:syn:grammar.case-statement-termination]
> Rule 4 [Case statement termination].
>
> When the TOKEN is exactly the reserved word esac, the token identifier for
> esac shall result. Otherwise, the token WORD shall be returned.
>
> This rule is applied at the first `pattern_list : WORD` production only; it
> is not applied to the `'(' WORD` or `pattern_list '|' WORD` productions.
>
> Source: XCU 2.10.2 Shell Grammar Rules — utilities/V3_chap02.html#tag_19_10_02

> [spec:posix:syn:grammar.for-name]
> Rule 5 [NAME in for].
>
> When the TOKEN meets the requirements for a name (see XBD 3.216 Name), the
> token identifier NAME shall result. Otherwise, the token WORD shall be
> returned.
>
> Source: XCU 2.10.2 Shell Grammar Rules — utilities/V3_chap02.html#tag_19_10_02

> [spec:posix:syn:grammar.third-word-of-for-and-case]
> Rule 6 [Third word of for and case].
>
> a. [case only] When the TOKEN is exactly the reserved word in, the token
> identifier for in shall result. Otherwise, the token WORD shall be returned.
>
> b. [for only] When the TOKEN is exactly the reserved word in or do, the token
> identifier for in or do shall result, respectively. Otherwise, the token WORD
> shall be returned.
>
> (For a. and b.: As indicated in the grammar, a linebreak precedes the tokens
> in and do. If <newline> characters are present at the indicated location, it
> is the token after them that is treated in this fashion.)
>
> Source: XCU 2.10.2 Shell Grammar Rules — utilities/V3_chap02.html#tag_19_10_02

> [spec:posix:syn:grammar.assignment-first-word]
> Rule 7 [Assignment preceding command name], a. [When the first word].
>
> If the TOKEN is exactly a reserved word, the token identifier for that
> reserved word shall result. Otherwise, 7b shall be applied.
>
> This rule is applied at the `cmd_name : WORD` production.
>
> Source: XCU 2.10.2 Shell Grammar Rules — utilities/V3_chap02.html#tag_19_10_02

> [spec:posix:syn:grammar.assignment-word-recognition]
> Rule 7 [Assignment preceding command name], b. [Not the first word].
>
> If the TOKEN contains an unquoted (as determined while applying rule 4 from
> 2.3 Token Recognition) <equals-sign> character that is not part of an
> embedded parameter expansion, command substitution, or arithmetic expansion
> construct (as determined while applying rule 5 from 2.3 Token Recognition):
>
> - If the TOKEN begins with `'='`, then the token WORD shall be returned.
> - If all the characters in the TOKEN preceding the first such <equals-sign>
> form a valid name (see XBD 3.216 Name), the token ASSIGNMENT_WORD shall be
> returned.
> - Otherwise, it is implementation-defined whether the token WORD or
> ASSIGNMENT_WORD is returned, or the TOKEN is processed in some other way.
>
> Otherwise, the token WORD shall be returned.
>
> This rule is applied at the `cmd_word : WORD` production.
>
> Source: XCU 2.10.2 Shell Grammar Rules — utilities/V3_chap02.html#tag_19_10_02

> [spec:posix:req:grammar.assignment-word-processing]
> If a returned ASSIGNMENT_WORD token begins with a valid name, assignment of
> the value after the first <equals-sign> to the name shall occur as specified
> in 2.9.1 Simple Commands. If a returned ASSIGNMENT_WORD token does not begin
> with a valid name, the way in which the token is processed is unspecified.
>
> Source: XCU 2.10.2 Shell Grammar Rules — utilities/V3_chap02.html#tag_19_10_02

> [spec:posix:syn:grammar.function-name]
> Rule 8 [NAME in function].
>
> When the TOKEN is exactly a reserved word, the token identifier for that
> reserved word shall result. Otherwise, when the TOKEN meets the requirements
> for a name, the token identifier NAME shall result. Otherwise, rule 7
> applies.
>
> This rule is applied at the `fname : NAME` production.
>
> Source: XCU 2.10.2 Shell Grammar Rules — utilities/V3_chap02.html#tag_19_10_02

> [spec:posix:req:grammar.function-body-no-expansion]
> Rule 9 [Body of function].
>
> Word expansion and assignment shall never occur, even when required by the
> rules above, when this rule is being parsed. Each TOKEN that might either be
> expanded or have assignment applied to it shall instead be returned as a
> single WORD consisting only of characters that are exactly the token
> described in 2.3 Token Recognition.
>
> This rule is applied at both `function_body` productions.
>
> Source: XCU 2.10.2 Shell Grammar Rules — utilities/V3_chap02.html#tag_19_10_02

## The grammar symbols

> [spec:posix:def:grammar.token-symbols]
> The grammar symbols of the Shell Command Language are declared as follows.
>
> ```
> %token  WORD
> %token  ASSIGNMENT_WORD
> %token  NAME
> %token  NEWLINE
> %token  IO_NUMBER
> %token  IO_LOCATION
> ```
>
> Source: XCU 2.10 Shell Grammar — utilities/V3_chap02.html#tag_19_10

> [spec:posix:def:grammar.operator-tokens]
> The following are the operators (see XBD 3.243 Operator) containing more than
> one character.
>
> ```
> %token  AND_IF    OR_IF    DSEMI    SEMI_AND
> /*      '&&'      '||'     ';;'     ';&'   */
>
> %token  DLESS  DGREAT  LESSAND  GREATAND  LESSGREAT  DLESSDASH
> /*      '<<'   '>>'    '<&'     '>&'      '<>'       '<<-'   */
>
> %token  CLOBBER
> /*      '>|'   */
> ```
>
> Source: XCU 2.10 Shell Grammar — utilities/V3_chap02.html#tag_19_10

> [spec:posix:def:grammar.reserved-word-tokens]
> The following are the reserved words.
>
> ```
> %token  If    Then    Else    Elif    Fi    Do    Done
> /*      'if'  'then'  'else'  'elif'  'fi'  'do'  'done'   */
>
> %token  Case    Esac    While    Until    For
> /*      'case'  'esac'  'while'  'until'  'for'   */
> ```
>
> These are reserved words, not operator tokens, and are recognized when
> reserved words are recognized.
>
> ```
> %token  Lbrace    Rbrace    Bang
> /*      '{'       '}'       '!'   */
>
> %token  In
> /*      'in'   */
> ```
>
> Source: XCU 2.10 Shell Grammar — utilities/V3_chap02.html#tag_19_10

## The Grammar

> [spec:posix:syn:grammar.program]
> The start symbol of the grammar is program.
>
> ```
> %start program
> %%
> program          : linebreak complete_commands linebreak
>                  | linebreak
>                  ;
> complete_commands: complete_commands newline_list complete_command
>                  |                                complete_command
>                  ;
> complete_command : list separator_op
>                  | list
>                  ;
> ```
>
> Source: XCU 2.10 Shell Grammar — utilities/V3_chap02.html#tag_19_10

> [spec:posix:syn:grammar.list-and-or]
> ```
> list             : list separator_op and_or
>                  |                   and_or
>                  ;
> and_or           :                         pipeline
>                  | and_or AND_IF linebreak pipeline
>                  | and_or OR_IF  linebreak pipeline
>                  ;
> ```
>
> Source: XCU 2.10 Shell Grammar — utilities/V3_chap02.html#tag_19_10

> [spec:posix:syn:grammar.pipeline]
> ```
> pipeline         :      pipe_sequence
>                  | Bang pipe_sequence
>                  ;
> pipe_sequence    :                             command
>                  | pipe_sequence '|' linebreak command
>                  ;
> ```
>
> Source: XCU 2.10 Shell Grammar — utilities/V3_chap02.html#tag_19_10

> [spec:posix:syn:grammar.command]
> ```
> command          : simple_command
>                  | compound_command
>                  | compound_command redirect_list
>                  | function_definition
>                  ;
> compound_command : brace_group
>                  | subshell
>                  | for_clause
>                  | case_clause
>                  | if_clause
>                  | while_clause
>                  | until_clause
>                  ;
> ```
>
> Source: XCU 2.10 Shell Grammar — utilities/V3_chap02.html#tag_19_10

> [spec:posix:syn:grammar.subshell-and-compound-list]
> ```
> subshell         : '(' compound_list ')'
>                  ;
> compound_list    : linebreak term
>                  | linebreak term separator
>                  ;
> term             : term separator and_or
>                  |                and_or
>                  ;
> ```
>
> Source: XCU 2.10 Shell Grammar — utilities/V3_chap02.html#tag_19_10

> [spec:posix:syn:grammar.for-clause]
> The name production applies rule 5, and the in production applies rule 6. The
> in production is shared with case_clause.
>
> ```
> for_clause       : For name                                      do_group
>                  | For name                       sequential_sep do_group
>                  | For name linebreak in          sequential_sep do_group
>                  | For name linebreak in wordlist sequential_sep do_group
>                  ;
> name             : NAME                     /* Apply rule 5 */
>                  ;
> in               : In                       /* Apply rule 6 */
>                  ;
> wordlist         : wordlist WORD
>                  |          WORD
>                  ;
> ```
>
> Source: XCU 2.10 Shell Grammar — utilities/V3_chap02.html#tag_19_10

> [spec:posix:syn:grammar.case-clause]
> Rule 4 is applied only to the first pattern_list production, as marked.
>
> ```
> case_clause      : Case WORD linebreak in linebreak case_list    Esac
>                  | Case WORD linebreak in linebreak case_list_ns Esac
>                  | Case WORD linebreak in linebreak              Esac
>                  ;
> case_list_ns     : case_list case_item_ns
>                  |           case_item_ns
>                  ;
> case_list        : case_list case_item
>                  |           case_item
>                  ;
> case_item_ns     : pattern_list ')' linebreak
>                  | pattern_list ')' compound_list
>                  ;
> case_item        : pattern_list ')' linebreak     DSEMI linebreak
>                  | pattern_list ')' compound_list DSEMI linebreak
>                  | pattern_list ')' linebreak     SEMI_AND linebreak
>                  | pattern_list ')' compound_list SEMI_AND linebreak
>                  ;
> pattern_list     :                  WORD    /* Apply rule 4 */
>                  |              '(' WORD    /* Do not apply rule 4 */
>                  | pattern_list '|' WORD    /* Do not apply rule 4 */
>                  ;
> ```
>
> Source: XCU 2.10 Shell Grammar — utilities/V3_chap02.html#tag_19_10

> [spec:posix:syn:grammar.if-clause]
> ```
> if_clause        : If compound_list Then compound_list else_part Fi
>                  | If compound_list Then compound_list           Fi
>                  ;
> else_part        : Elif compound_list Then compound_list
>                  | Elif compound_list Then compound_list else_part
>                  | Else compound_list
>                  ;
> ```
>
> Source: XCU 2.10 Shell Grammar — utilities/V3_chap02.html#tag_19_10

> [spec:posix:syn:grammar.while-until-clause]
> ```
> while_clause     : While compound_list do_group
>                  ;
> until_clause     : Until compound_list do_group
>                  ;
> ```
>
> Source: XCU 2.10 Shell Grammar — utilities/V3_chap02.html#tag_19_10

> [spec:posix:syn:grammar.function-definition]
> The function_body productions apply rule 9, and the fname production applies
> rule 8.
>
> ```
> function_definition : fname '(' ')' linebreak function_body
>                  ;
> function_body    : compound_command                /* Apply rule 9 */
>                  | compound_command redirect_list  /* Apply rule 9 */
>                  ;
> fname            : NAME                            /* Apply rule 8 */
>                  ;
> ```
>
> Source: XCU 2.10 Shell Grammar — utilities/V3_chap02.html#tag_19_10

> [spec:posix:syn:grammar.brace-group-and-do-group]
> The do_group production applies rule 6.
>
> ```
> brace_group      : Lbrace compound_list Rbrace
>                  ;
> do_group         : Do compound_list Done           /* Apply rule 6 */
>                  ;
> ```
>
> Source: XCU 2.10 Shell Grammar — utilities/V3_chap02.html#tag_19_10

> [spec:posix:syn:grammar.simple-command]
> The cmd_name production applies rule 7a, and the cmd_word production applies
> rule 7b.
>
> ```
> simple_command   : cmd_prefix cmd_word cmd_suffix
>                  | cmd_prefix cmd_word
>                  | cmd_prefix
>                  | cmd_name cmd_suffix
>                  | cmd_name
>                  ;
> cmd_name         : WORD                   /* Apply rule 7a */
>                  ;
> cmd_word         : WORD                   /* Apply rule 7b */
>                  ;
> cmd_prefix       :            io_redirect
>                  | cmd_prefix io_redirect
>                  |            ASSIGNMENT_WORD
>                  | cmd_prefix ASSIGNMENT_WORD
>                  ;
> cmd_suffix       :            io_redirect
>                  | cmd_suffix io_redirect
>                  |            WORD
>                  | cmd_suffix WORD
>                  ;
> ```
>
> Source: XCU 2.10 Shell Grammar — utilities/V3_chap02.html#tag_19_10

> [spec:posix:syn:grammar.io-redirect]
> The two IO_LOCATION forms of io_redirect are optionally supported.
>
> ```
> redirect_list    :               io_redirect
>                  | redirect_list io_redirect
>                  ;
> io_redirect      :             io_file
>                  | IO_NUMBER   io_file
>                  | IO_LOCATION io_file /* Optionally supported */
>                  |             io_here
>                  | IO_NUMBER   io_here
>                  | IO_LOCATION io_here /* Optionally supported */
>                  ;
> ```
>
> Source: XCU 2.10 Shell Grammar — utilities/V3_chap02.html#tag_19_10

> [spec:posix:syn:grammar.io-file]
> The filename production applies rule 2.
>
> ```
> io_file          : '<'       filename
>                  | LESSAND   filename
>                  | '>'       filename
>                  | GREATAND  filename
>                  | DGREAT    filename
>                  | LESSGREAT filename
>                  | CLOBBER   filename
>                  ;
> filename         : WORD                      /* Apply rule 2 */
>                  ;
> ```
>
> Source: XCU 2.10 Shell Grammar — utilities/V3_chap02.html#tag_19_10

> [spec:posix:syn:grammar.io-here]
> The here_end production applies rule 3.
>
> ```
> io_here          : DLESS     here_end
>                  | DLESSDASH here_end
>                  ;
> here_end         : WORD                      /* Apply rule 3 */
>                  ;
> ```
>
> Source: XCU 2.10 Shell Grammar — utilities/V3_chap02.html#tag_19_10

> [spec:posix:syn:grammar.separators]
> ```
> newline_list     :              NEWLINE
>                  | newline_list NEWLINE
>                  ;
> linebreak        : newline_list
>                  | /* empty */
>                  ;
> separator_op     : '&'
>                  | ';'
>                  ;
> separator        : separator_op linebreak
>                  | newline_list
>                  ;
> sequential_sep   : ';' linebreak
>                  | newline_list
>                  ;
> ```
>
> Source: XCU 2.10 Shell Grammar — utilities/V3_chap02.html#tag_19_10
