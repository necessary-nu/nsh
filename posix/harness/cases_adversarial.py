"""Adversarial cases that push the POSIX.1-2024 wording to its limits.

Every case here targets a corner where the standard's own words are sharper
than the folklore: the boundary between "shall", "unspecified" and
"undefined", the exact tokenisation of an operator, the exact field a
splitting algorithm produces, the exact status a shell leaves behind.

Expectations state what POSIX.1-2024 requires, not what bash or dash
happens to do; where a reference shell disagrees with the standard, the
comment says so. Cases that fail are conformance findings, not bugs in the
suite.

Nothing here asserts behaviour the standard leaves unspecified or
undefined. The corners that were probed and deliberately left untested for
that reason are listed in the comment block below, by rule id.
"""

# ---------------------------------------------------------------------
# Deliberately NOT tested: the standard makes these unspecified,
# undefined, implementation-defined or optional, so any expectation would
# be an invention. Each was probed while writing this file and each is a
# place where nsh, bash and dash can and do differ legitimately.
#
#   token.reserved-words-trailing-colon  "produces unspecified results"
#   token.alias-reserved-word-unspecified whether an alias that would be a
#                                        reserved word is substituted
#   token.alias-change-timing            an alias change need only take
#                                        effect by the end of the current
#                                        complete_command (why every alias
#                                        case below defines on its own line)
#   redir.location-format                "{name}<word" -- "may support";
#                                        "behavior is implementation-defined"
#   grammar.token-classification         IO_LOCATION "may result"
#   redir.dup-output-close               ">&nope": "If word evaluates to
#   redir.dup-input-close                something else, the behavior is
#                                        unspecified"
#   redir.max-fd-number                  beyond fd 9 is implementation-defined
#   redir.word-expansion                 whether expansions other than quote
#                                        removal apply to a here-document
#                                        delimiter word
#   redir.here-doc-delimiter             missing terminator: "should, but
#                                        need not, treat this as an error"
#   redir.here-doc-line-continuation     whether the delimiter line itself
#                                        is subject to line continuation
#   redir.here-doc-fd-type               seekability of the here-document fd
#   pattern.bracket-expression           "[^b]": a bracket expression
#                                        starting with an unquoted
#                                        <circumflex> "produces unspecified
#                                        results"
#   pattern.backslash-escape-without-shell-quoting
#                                        backslash inside a bracket
#                                        expression, for the shell
#   pattern.leading-period-in-bracket-unspecified   "[.abc]"
#   pattern.unmatched-open-bracket-unspecified      "a*[/b*"
#   pattern.trailing-backslash-unspecified          pattern ending in "\"
#   pattern.filename-expansion-trigger   dot and dot-dot "may be ignored",
#                                        so ".*" is never matched here
#   param.positional-zero-not-positional "${00}"
#   param.special-hyphen                 whether -c and -s appear in "$-"
#   param.special-at-double-quotes       "In all other contexts the results
#                                        of the expansion are unspecified"
#   expand.param-word-expansion          word forms with '*' or '@' as the
#   expand.param-string-length           parameter: "${#*}", "${*-x}", ...
#   expand.param-substring-common        "the result ... is unspecified"
#   expand.param-hash-requires-word      "${##}" is ambiguous by design
#   expand.dollar-invalid-follower       "$%" and friends
#   expand.arith-token-expansion         '$(( "x" + 1 ))' -- "a double-quote
#                                        inside the expression is not
#                                        treated specially" cannot be
#                                        reconciled with "quote removal";
#                                        also the empty expression "$(())"
#   expand.arith-evaluation              signed overflow (ISO C undefined)
#                                        and division by zero, which POSIX
#                                        never classifies as an "invalid
#                                        expression"
#   xcurel.arithmetic-operators          the comma operator "need not be
#                                        supported"; ++, -- and sizeof "are
#                                        not required"
#   expand.brace-implementation-defined  brace expansion is optional
#   expand.assignment-redirection-environment  which environment expands the
#                                        assignments and redirections of a
#                                        command
#   cmd.assign-function                  whether assignments preceding a
#                                        function call persist
#   cmd.search-unspecified-utility-names local, typeset, source, ...
#   cmd.case-multiple-pattern-order-unspecified
#   expand.cmdsub-redirections-only      "$( >f )"
#   expand.cmdsub-parsing                incremental vs single compound_list
#   expand.cmdsub-backquote-matching     undefined results
#   quote.double-quotes-backquote-undefined
#   quote.dollar-single-quotes-undefined-escape   "$'\z'"
#   quote.dollar-single-quotes-null-byte, -octal-overflow, -unencodable
#   builtin.exit.invalid-n-unspecified   "exit abc", "exit 256"
#   builtin.trap.kill-stop-undefined     a trap on KILL or STOP
#   builtin.trap.subshell-lexical-check  implementations "may" use lexical
#                                        analysis only
#   builtin.set.first-argument-hyphen    "set -"
#   signal.pending-trap-order
#   shell.hashbang-unspecified
# ---------------------------------------------------------------------

from __future__ import annotations

import textwrap

from model import Case, FileFixture


def _s(text: str) -> str:
    """Turn an indented raw literal into a shell script."""

    return textwrap.dedent(text).lstrip("\n")


TAB = "\t"


CASES: tuple[Case, ...] = (
    # =================================================================
    # 1. Token recognition and operator boundaries
    # =================================================================
    # IO_NUMBER exists only when "the string consists solely of digits and
    # the delimiter character is one of '<' or '>'". A <blank> delimiter
    # therefore leaves the digits as an ordinary word, and a word that is
    # not solely digits is never an IO_NUMBER even when '>' delimits it.
    # [spec:posix:syn:grammar.token-classification/test]
    # [spec:posix:syn:redir.format/test]
    # [spec:posix:syn:token.unquoted-blank-delimits/test]
    Case(
        id="adv-token-io-number-delimiter",
        rules=(
            "grammar.token-classification",
            "redir.format",
            "token.unquoted-blank-delimits",
        ),
        script=_s(
            r"""
            echo 2 > blank
            echo 2> adjacent
            echo a2> word
            printf '[%s][%s][%s]\n' "$(cat blank)" "$(cat adjacent)" "$(cat word)"
            """
        ),
        # `echo 2 > blank`: "2" is delimited by the <blank>, so it is an
        # argument and the file holds it. `echo 2> adjacent`: "2" is an
        # IO_NUMBER, so echo prints an empty line and stderr is redirected.
        # `echo a2> word`: "a2" is not solely digits, so it is an argument.
        # `echo 2> adjacent` still writes echo's empty line to standard output.
        stdout="\n[2][][a2]\n",
    ),
    # The standard gives both examples verbatim: "echo \2>a writes the
    # character 2 into file a" and "echo 2\>a writes the characters 2>a to
    # standard output". A double-quoted n is quoted just as a backslashed
    # one is: the token retains its quoting characters, so it is not
    # "solely digits".
    # [spec:posix:syn:redir.quoting-suppresses-recognition/test]
    # [spec:posix:req:quote.backslash-literal/test]
    Case(
        id="adv-token-quoted-io-number",
        rules=("redir.quoting-suppresses-recognition", "quote.backslash-literal"),
        script=_s(
            r"""
            echo \2>a
            echo "2">b
            echo 2\>c
            printf '[%s][%s]\n' "$(cat a)" "$(cat b)"
            """
        ),
        stdout="2>c\n[2][2]\n",
    ),
    # Rule 2 of token recognition appends a character to an operator only
    # if the result "can be used with the previous characters to form an
    # operator". ">>|" is not an operator, so ">>" is delimited and '|'
    # starts a new operator -- leaving a redirection with no filename,
    # which is a syntax error. A shell that instead munched "> >|" or
    # ">|" would run the command.
    # [spec:posix:syn:token.operator-continue/test]
    # [spec:posix:syn:token.operator-delimit/test]
    # [spec:posix:syn:token.start-new-operator/test]
    Case(
        id="adv-token-operator-munch-limit",
        rules=(
            "token.operator-continue",
            "token.operator-delimit",
            "token.start-new-operator",
        ),
        script="echo hi >>|f; echo reached\n",
        stdout="",
        status="nonzero",
    ),
    # Reserved words are recognised only in the listed positions, and only
    # when no character of the word is quoted. '}' as an argument is an
    # ordinary word; "then" as the first word of a command is a reserved
    # word (hence a syntax error on its own); a backslash anywhere in the
    # word prevents recognition entirely.
    # [spec:posix:req:token.reserved-word-recognition-contexts/test]
    # [spec:posix:syn:grammar.command-name/test]
    Case(
        id="adv-token-reserved-word-positions",
        rules=("token.reserved-word-recognition-contexts", "grammar.command-name"),
        script=_s(
            r"""
            echo } '{' done esac
            for i in in; do echo "[$i]"; done
            case done in done) echo third-word-not-reserved ;; esac
            """
        ),
        stdout="} { done esac\n[in]\nthird-word-not-reserved\n",
    ),
    # "then" alone is a reserved word in command position: a shell language
    # syntax error, which a non-interactive shell shall diagnose and exit
    # for.
    # [spec:posix:req:token.reserved-word-recognition-contexts/test]
    # [spec:posix:req:exit.shell-error-consequences/test]
    Case(
        id="adv-token-reserved-word-alone",
        rules=("token.reserved-word-recognition-contexts", "exit.shell-error-consequences"),
        script="then\n",
        stdout="",
        status="nonzero",
    ),
    # Rule 8 (append to word) is applied before rule 9 (comment), so a '#'
    # that is not the first character of a token is ordinary. A '#' after
    # an unquoted operator or <blank> does start a comment, and the
    # <newline> ending the line is not part of it.
    # [spec:posix:syn:token.comment/test]
    # [spec:posix:syn:token.append-to-word/test]
    Case(
        id="adv-token-comment-boundaries",
        rules=("token.comment", "token.append-to-word"),
        script=_s(
            r"""
            echo a#b
            echo 'x'#y
            echo keep #dropped
            echo tail;#dropped
            echo end
            """
        ),
        stdout="a#b\nx#y\nkeep\ntail\nend\n",
    ),
    # A reserved word has to be a token of its own: "{echo" is a single
    # word, so the brace group never opens and the '}' is a syntax error.
    # '(' is an operator, so "(echo hi)" needs no <blank>.
    # [spec:posix:def:token.reserved-words/test]
    # [spec:posix:syn:token.start-new-operator/test]
    Case(
        id="adv-token-brace-needs-delimiting",
        rules=("token.reserved-words", "token.start-new-operator"),
        script="{echo hi; }\n",
        stdout="",
        status="nonzero",
    ),
    # "once a complete_command has been recognized by the grammar, the
    # complete_command shall be executed before the next complete_command
    # is tokenized and parsed": the first line runs even though the second
    # line is a syntax error.
    # [spec:posix:req:token.incremental-execution/test]
    Case(
        id="adv-token-incremental-execution",
        rules=("token.incremental-execution",),
        script="echo first\nfi\n",
        stdout="first\n",
        status="nonzero",
    ),
    # Alias substitution: the replacement is re-tokenised, but a TOKEN that
    # resulted from an alias substitution of the same name at an earlier
    # recursion level is not substituted again, so "echo x y" results
    # rather than an infinite expansion. bash performs no alias
    # substitution at all in a non-interactive shell, which is a documented
    # bash deviation, not the standard's behaviour.
    # [spec:posix:req:token.alias-substitution-conditions/test]
    # [spec:posix:req:token.alias-replacement/test]
    Case(
        id="adv-alias-recursion-guard",
        rules=("token.alias-substitution-conditions", "token.alias-replacement"),
        # The definition and the use are on separate lines: a change to an
        # alias need only take effect by the end of the current
        # complete_command, so using it in the same list is unspecified.
        script="alias echo='echo x'\necho y\n",
        stdout="x y\n",
    ),
    # An alias value ending in an unquoted <blank> makes the next word a
    # candidate for alias substitution too; one that does not, does not.
    # [spec:posix:req:token.alias-trailing-blank-chaining/test]
    Case(
        id="adv-alias-trailing-blank-chaining",
        rules=("token.alias-trailing-blank-chaining",),
        script=_s(
            r"""
            alias chain='echo '
            alias nochain='echo'
            alias word=SUBSTITUTED
            chain word
            nochain word
            """
        ),
        stdout="SUBSTITUTED\nword\n",
    ),
    # "The TOKEN does not contain any quoting characters" -- a single
    # backslash disqualifies the whole token, so the alias is not applied
    # and the command is not found.
    # [spec:posix:req:token.alias-substitution-conditions/test]
    # [spec:posix:req:token.alias-not-inherited/test]
    Case(
        id="adv-alias-quoted-and-inherited",
        rules=("token.alias-substitution-conditions", "token.alias-not-inherited"),
        script=_s(
            r"""
            alias aliased=echo
            \aliased quoted 2>/dev/null
            printf 'quoted=%s\n' "$?"
            sh -c 'aliased inherited' 2>/dev/null
            printf 'inherited=%s\n' "$?"
            """
        ),
        stdout="quoted=127\ninherited=127\n",
    ),

    # "Any non-NEWLINE tokens (including more io_here tokens) that are
    # recognized while searching for the next NEWLINE token shall be saved
    # for processing after the here-document has been parsed", and the
    # here-document of the first operator is read first.
    # [spec:posix:req:token.here-document-mode/test]
    # [spec:posix:req:redir.here-doc-multiple/test]
    # [spec:posix:syn:redir.here-doc-format/test]
    Case(
        id="adv-token-heredoc-saved-tokens",
        rules=("token.here-document-mode", "redir.here-doc-multiple", "redir.here-doc-format"),
        script=_s(
            r"""
            cat <<A; cat <<B
            first
            A
            second
            B
            cat <<C <<D
            ignored
            C
            used
            D
            """
        ),
        # Two operators on one command: the here-document for the first is
        # supplied first, but only the last one is left on fd 0.
        stdout="first\nsecond\nused\n",
    ),

    # =================================================================
    # 2. Quoting
    # =================================================================
    # Inside double-quotes the <backslash> is an escape character ONLY
    # before '$', '`', '\' and <newline> (and a '"' that would otherwise be
    # special). Before anything else it is an ordinary character and both
    # characters survive.
    # [spec:posix:req:quote.double-quotes-backslash/test]
    # [spec:posix:req:quote.double-quotes-literal/test]
    Case(
        id="adv-quote-dq-backslash-scope",
        rules=("quote.double-quotes-backslash", "quote.double-quotes-literal"),
        script=_s(
            r"""
            v=EXPANDED
            printf '[%s]' "a\b" "a\'b" "a\$v" "a\\b" "a\"b" "a\`b" "$v"
            printf '\n'
            """
        ),
        stdout=r"""[a\b][a\'b][a$v][a\b][a"b][a`b][EXPANDED]""" + "\n",
    ),
    # A <backslash><newline> is line continuation inside double-quotes (the
    # <backslash> retains its escape meaning before a <newline>) but is two
    # literal characters inside single-quotes, which "preserve the literal
    # value of each character".
    # [spec:posix:req:quote.backslash-newline/test]
    # [spec:posix:req:quote.single-quotes/test]
    Case(
        id="adv-quote-line-continuation-contexts",
        rules=("quote.backslash-newline", "quote.single-quotes"),
        script='printf \'[%s]\' "ab\\\ncd" \'ab\\\ncd\' ef\\\ngh\nprintf \'\\n\'\n',
        stdout="[abcd][ab\\\ncd][efgh]\n",
    ),
    # POSIX.1-2024: within double-quotes the <dollar-sign> "shall not
    # retain its special meaning introducing the dollar-single-quotes form
    # of quoting", so "$'x'" is four literal characters.
    # [spec:posix:req:quote.double-quotes-dollar-sign/test]
    # [spec:posix:req:quote.dollar-single-quotes/test]
    Case(
        id="adv-quote-dollar-single-inside-dq",
        rules=("quote.double-quotes-dollar-sign", "quote.dollar-single-quotes"),
        script="printf '[%s][%s]\\n' \"$'x'\" $'x'\n",
        stdout="[$'x'][x]\n",
    ),
    # Dollar-single-quote escapes: \cX yields the control character, and
    # "\c\\ yields the <FS> control character since the <backslash> has to
    # be escaped"; \e yields <ESC>. An escape sequence with a variable
    # number of characters ends at "the first character that is not of the
    # expected type or, for \ddd sequences, when the maximum number of
    # characters specified has been found" -- so \1010 is <A> then '0' and
    # \x9z is <tab> then 'z'.
    # [spec:posix:def:quote.dollar-single-quotes-control-escape/test]
    # [spec:posix:def:quote.dollar-single-quotes-hex-escape/test]
    # [spec:posix:def:quote.dollar-single-quotes-octal-escape/test]
    # [spec:posix:syn:quote.dollar-single-quotes-escape-termination/test]
    Case(
        id="adv-quote-dsq-escape-termination",
        rules=(
            "quote.dollar-single-quotes-control-escape",
            "quote.dollar-single-quotes-hex-escape",
            "quote.dollar-single-quotes-octal-escape",
            "quote.dollar-single-quotes-escape-termination",
        ),
        script=_s(
            r"""
            printf '%s' $'\cA' | od -An -c | tr -d ' \n'
            printf '\n'
            printf '%s' $'\c\\' | od -An -c | tr -d ' \n'
            printf '\n'
            printf '[%s][%s][%s][%s]\n' $'\x41' $'\101' $'\1010' $'\x9z'
            """
        ),
        stdout="001\n034\n[A][A][A0][\t" + "z]\n",
    ),
    # "If a <backslash>-escape sequence represents a single-quote character
    # (for example \'), that sequence shall not terminate the
    # dollar-single-quote sequence."
    # [spec:posix:req:quote.dollar-single-quotes-quote-escape-not-terminator/test]
    # [spec:posix:req:quote.dollar-single-quotes-processing-time/test]
    Case(
        id="adv-quote-dsq-quote-not-terminator",
        rules=(
            "quote.dollar-single-quotes-quote-escape-not-terminator",
            "quote.dollar-single-quotes-processing-time",
        ),
        script=_s(
            r"""
            IFS=:
            v=NOT-EXPANDED
            set -- $'a\'b'
            printf '[%s]' "$1"
            set -- $'x:y'
            printf '(%d)[%s]' "$#" "$1"
            q=$'x:y'
            set -- $q
            printf '(%d)[%s][%s]' "$#" "$1" "$2"
            printf '[%s]\n' $'$v'
            """
        ),
        # The escapes are processed "immediately prior to word expansion",
        # but dollar-single-quotes is a quoting mechanism: the ':' it yields
        # is quoted word text, not the result of an expansion, so it does not
        # split the word. Reaching field splitting through a variable does
        # split it. The '$' it yields never introduces an expansion.
        stdout="[a'b](1)[x:y](2)[x][y][$v]\n",
    ),
    # The four substring varieties are the only ones where "the
    # double-quotes within which the expansion occurs shall have no effect
    # on the handling of any special characters": the quotes around 'a' are
    # real quoting there. For every other variety the enclosing
    # double-quotes "preserve the literal value of all characters" with
    # only '"', '`', '$' and '\' excepted -- so the single-quotes are
    # literal text of the word.
    # [spec:posix:req:quote.double-quotes-substring-parameter-expansion/test]
    # [spec:posix:req:quote.double-quotes-other-parameter-expansion/test]
    Case(
        id="adv-quote-dq-inside-braces",
        rules=(
            "quote.double-quotes-substring-parameter-expansion",
            "quote.double-quotes-other-parameter-expansion",
        ),
        script=_s(
            r"""
            x=abc
            unset u
            printf '[%s]' "${x#'a'}" "${x#'*'}" "${u-'a b'}" "${u-a\}b}"
            printf '\n'
            """
        ),
        # "${x#'a'}": quoted pattern 'a' matches literally -> bc.
        # "${x#'*'}": the '*' is quoted, so it matches no prefix -> abc.
        # "${u-'a b'}": the quotes are literal characters of the word.
        # "${u-a\}b}": the <backslash> "shall additionally retain its
        # special meaning as an escape character when followed by '}'".
        stdout="[bc][abc]['a b'][a}b]\n",
    ),

    # "After quote removal the shell still remembers which characters were
    # quoted. This is necessary for purposes such as matching patterns in a
    # case conditional construct." The pattern text is identical in all
    # three arms; only the quoting differs.
    # [spec:posix:sem:expand.quote-removal-quoting-remembered/test]
    # [spec:posix:req:expand.quote-removal/test]
    # [spec:posix:req:cmd.case-pattern-expansion/test]
    Case(
        id="adv-quote-removal-remembered",
        rules=(
            "expand.quote-removal-quoting-remembered",
            "expand.quote-removal",
            "cmd.case-pattern-expansion",
        ),
        script=_s(
            r"""
            star='*'
            case 'x'  in "$star") printf 'A';; *) printf 'a';; esac
            case '*'  in "$star") printf 'B';; *) printf 'b';; esac
            case 'x'  in $star)   printf 'C';; *) printf 'c';; esac
            case 'a*' in a\*)     printf 'D';; *) printf 'd';; esac
            case 'ab' in a\*)     printf 'E';; *) printf 'e';; esac
            printf '\n'
            """
        ),
        stdout="aBCDe\n",
    ),

    # =================================================================
    # 3. Parameter expansion
    # =================================================================
    # The whole <colon> table from 2.6.2, in one case: with the colon the
    # test is "unset or null", without it the test is "only unset".
    # [spec:posix:req:expand.param-colon-effect/test]
    # [spec:posix:req:expand.param-use-default/test]
    # [spec:posix:req:expand.param-use-alternative/test]
    Case(
        id="adv-param-colon-table",
        rules=(
            "expand.param-colon-effect",
            "expand.param-use-default",
            "expand.param-use-alternative",
        ),
        script=_s(
            r"""
            v=value; n=; unset u
            printf '[%s]' "${v:-w}" "${n:-w}" "${u:-w}"
            printf '[%s]' "${v-w}" "${n-w}" "${u-w}"
            printf '[%s]' "${v:+w}" "${n:+w}" "${u:+w}"
            printf '[%s]' "${v+w}" "${n+w}" "${u+w}"
            printf '\n'
            """
        ),
        stdout="[value][w][w][value][][w][w][][][w][w][]\n",
    ),
    # "${parameter:=[word]}": quote removal is performed on the expansion
    # of word and the result assigned. "${parameter?}" with a null (not
    # unset) parameter substitutes null and is not an error. "If word is
    # not needed, it shall not be expanded" -- the command substitution in
    # the unused word must not run.
    # [spec:posix:req:expand.param-assign-default/test]
    # [spec:posix:req:expand.param-word-expansion/test]
    Case(
        id="adv-param-assign-and-unused-word",
        rules=("expand.param-assign-default", "expand.param-word-expansion"),
        script=_s(
            r"""
            unset x
            y=${x:='a b'}
            printf '[%s][%s]' "$x" "$y"
            n=
            printf '[%s]' "${n?}"
            set=here
            printf '[%s]' "${set:-$(echo SIDE-EFFECT >&2; echo w)}"
            printf '\n'
            """
        ),
        stdout="[a b][a b][][here]\n",
        stderr="",
    ),
    # "${parameter:?[word]}": the expansion of word "shall be written to
    # standard error and the shell exits with a non-zero exit status".
    # [spec:posix:req:expand.param-error-if-unset/test]
    # [spec:posix:req:exit.shell-error-consequences/test]
    Case(
        id="adv-param-error-if-unset-exits",
        rules=("expand.param-error-if-unset", "exit.shell-error-consequences"),
        script=_s(
            r"""
            unset x
            echo "${x:?diagnostic-text}"
            echo reached
            """
        ),
        stdout="",
        stderr_contains=("diagnostic-text",),
        status="nonzero",
    ),
    # "The digits denoting the positional parameters shall always be
    # interpreted as a decimal value, even if there is a leading zero", and
    # "$10" is $1 followed by '0', not ${10}.
    # [spec:posix:req:param.positional-decimal-digits/test]
    # [spec:posix:syn:param.positional-multi-digit-braces/test]
    Case(
        id="adv-param-positional-digits",
        rules=("param.positional-decimal-digits", "param.positional-multi-digit-braces"),
        script=_s(
            r"""
            set -- 1 2 3 4 5 6 7 8 9 TEN
            printf '[%s]' "$8" "${8}" "${08}" "${008}" "${10}" "$10"
            printf '\n'
            """
        ),
        stdout="[8][8][8][8][TEN][10]\n",
    ),
    # "if the parameter being expanded was embedded within a word, the
    # first field shall be joined with the beginning part of the original
    # word and the last field shall be joined with the end part".
    # With no positional parameters '@' "shall generate zero fields, even
    # when '@' is within double-quotes".
    # [spec:posix:req:param.special-at-double-quotes/test]
    # [spec:posix:req:param.special-at-no-positional/test]
    Case(
        id="adv-param-at-embedded-in-word",
        rules=("param.special-at-double-quotes", "param.special-at-no-positional"),
        script=_s(
            r"""
            set -- a b
            printf '[%s]' pre"$@"post
            printf '\n'
            set --
            for w in "$@"; do printf '[%s]' "$w"; done
            printf 'END\n'
            """
        ),
        stdout="[prea][bpost]\nEND\n",
    ),
    # '*' joined outside a field-splitting context: separated by the first
    # character of IFS, by <space> if IFS is unset, with no separation if
    # IFS is null. '@' in double-quotes keeps its fields even when IFS is
    # null, because field splitting "would be performed if the expansion
    # were not within double-quotes ... regardless of whether field
    # splitting would have any effect".
    # [spec:posix:req:param.special-asterisk/test]
    Case(
        id="adv-param-star-joining",
        rules=("param.special-asterisk",),
        script=_s(
            r"""
            set -- a b c
            IFS=,;   printf '[%s]' "$*"
            IFS=xy;  printf '[%s]' "$*"
            IFS=;    printf '[%s]' "$*"
            unset IFS; printf '[%s]' "$*"
            set -- a b
            IFS=; printf '[%s]' "$@"
            printf '\n'
            """
        ),
        stdout="[a,b,c][axbxc][abc][a b c][a][b]\n",
    ),
    # set -u applies to "an unset parameter other than the '@' and '*'
    # special parameters", so an empty parameter list is not an error;
    # ${parameter-word} does not expand an unset parameter at all; but
    # "${#parameter}" with an unset parameter "shall fail" under set -u.
    # [spec:posix:req:builtin.set.opt-u-nounset/test]
    # [spec:posix:req:expand.param-string-length/test]
    Case(
        id="adv-param-nounset-boundaries",
        rules=("builtin.set.opt-u-nounset", "expand.param-string-length"),
        script=_s(
            r"""
            set -u
            set --
            unset x
            len=abcd
            printf '[%s]' "$@" "$*" "${x-default}" "${#len}"
            printf 'OK\n'
            echo "${#x}"
            echo reached
            """
        ),
        stdout="[][default][4]OK\n",
        status="nonzero",
    ),
    # "Any '}' escaped by a <backslash> or within a quoted string, and
    # characters in embedded arithmetic expansions, command substitutions,
    # and variable expansions, shall not be examined in determining the
    # matching '}'."
    # [spec:posix:syn:expand.param-format/test]
    Case(
        id="adv-param-matching-brace",
        rules=("expand.param-format",),
        script=_s(
            r"""
            unset x y
            printf '[%s]' ${x-'}'} "${x-$(printf '%s' '}')}" "${x-${y-inner}}" "${x-$((1+1))}"
            printf '\n'
            """
        ),
        stdout="[}][}][inner][2]\n",
    ),

    # =================================================================
    # 4. Field splitting and IFS
    # =================================================================
    # An IFS character that is not IFS white space delimits a field even
    # when the candidate is empty: ":a::b:" is four fields, and the
    # trailing delimiter does not create a fifth because "Once the input is
    # empty, the candidate shall become an output field if and only if it
    # is not empty".
    # [spec:posix:req:expand.field-splitting-algorithm/test]
    # [spec:posix:req:expand.ifs-delimiters/test]
    Case(
        id="adv-ifs-non-whitespace-empties",
        rules=("expand.field-splitting-algorithm", "expand.ifs-delimiters"),
        script=_s(
            r"""
            IFS=:
            x=':a::b:'
            set -- $x
            printf '(%d)' "$#"
            printf '[%s]' "$@"
            printf '\n'
            """
        ),
        stdout="(4)[][a][][b]\n",
    ),
    # IFS white space is absorbed in runs, leading and trailing, and a run
    # of IFS white space around a non-white-space delimiter does not add
    # fields. An input field that is entirely IFS white space yields zero
    # fields, not one empty field.
    # [spec:posix:def:expand.ifs-white-space/test]
    # [spec:posix:req:expand.field-splitting-zero-fields/test]
    Case(
        id="adv-ifs-whitespace-runs",
        rules=("expand.ifs-white-space", "expand.field-splitting-zero-fields"),
        script=_s(
            r"""
            IFS=' :'
            a='  x  y  '; set -- $a; printf '(%d)' "$#"; printf '[%s]' "$@"
            b='x : y';    set -- $b; printf '(%d)' "$#"; printf '[%s]' "$@"
            c='x::y';     set -- $c; printf '(%d)' "$#"; printf '[%s]' "$@"
            d='   ';      set -- $d; printf '(%d)' "$#"
            e=' : ';      set -- $e; printf '(%d)' "$#"; printf '[%s]' "$@"
            printf '\n'
            """
        ),
        stdout="(2)[x][y](2)[x][y](3)[x][][y](0)(1)[]\n",
    ),
    # "Field splitting only ever alters those parts of the field which are
    # present as the result of an expansion": the literal colons in the
    # word are not delimiters, so p:a and b:q survive as single fields.
    # [spec:posix:req:expand.field-splitting-unexpanded-fields/test]
    # [spec:posix:def:expand.field-splitting-results-of-expansion/test]
    Case(
        id="adv-ifs-literal-separator-not-split",
        rules=(
            "expand.field-splitting-unexpanded-fields",
            "expand.field-splitting-results-of-expansion",
        ),
        script=_s(
            r"""
            IFS=:
            x='a:b'
            set -- p:$x:q
            printf '(%d)' "$#"
            printf '[%s]' "$@"
            printf '\n'
            """
        ),
        stdout="(2)[p:a][b:q]\n",
    ),
    # IFS set to the null string: no splitting happens, but "if an input
    # field which contained the results of an expansion is entirely empty,
    # it shall be removed". IFS unset behaves as <space><tab><newline>
    # "without altering the value of the variable".
    # [spec:posix:req:expand.field-splitting-empty-ifs/test]
    # [spec:posix:req:expand.ifs-unset-default/test]
    Case(
        id="adv-ifs-empty-versus-unset",
        rules=("expand.field-splitting-empty-ifs", "expand.ifs-unset-default"),
        script=_s(
            r"""
            IFS=
            keep=' a b '
            set -- $keep; printf '(%d)[%s]' "$#" "$1"
            empty=
            set -- $empty tail; printf '(%d)[%s]' "$#" "$1"
            unset IFS
            mixed=$(printf ' a\tb\nc ')
            set -- $mixed; printf '(%d)' "$#"; printf '[%s]' "$@"
            printf '[%s]\n' "${IFS-UNSET}"
            """
        ),
        stdout="(1)[ a b ](1)[tail](3)[a][b][c][UNSET]\n",
    ),
    # An unquoted '*' expansion is split like any other expansion result,
    # and a quoted one is not. The joining uses the first IFS character
    # even when the splitting that follows uses all of them.
    # [spec:posix:req:expand.field-splitting-applies/test]
    # [spec:posix:req:expand.field-splitting-order/test]
    Case(
        id="adv-ifs-split-order-preserved",
        rules=("expand.field-splitting-applies", "expand.field-splitting-order"),
        script=_s(
            r"""
            IFS=:
            a='1:2'
            b='3:4'
            set -- $a X $b
            printf '(%d)' "$#"
            printf '[%s]' "$@"
            printf '\n'
            """
        ),
        stdout="(5)[1][2][X][3][4]\n",
    ),

    # =================================================================
    # 5. Pattern matching
    # =================================================================
    # XBD 9.3.5 as adopted by 2.14.1: a ']' first in the list (or first
    # after '!') is literal. A shell-quoting <backslash> escapes the
    # following character "regardless of whether or not the <backslash> is
    # inside a bracket expression", so [ab\]] is the set {a,b,]}.
    # [spec:posix:syn:pattern.bracket-expression/test]
    # [spec:posix:syn:pattern.backslash-escape-with-shell-quoting/test]
    Case(
        id="adv-pattern-bracket-rbracket",
        rules=("pattern.bracket-expression", "pattern.backslash-escape-with-shell-quoting"),
        script=_s(
            r"""
            case ']' in []a]) printf 'A';; *) printf 'a';; esac
            case 'a' in []a]) printf 'B';; *) printf 'b';; esac
            case ']' in [!]a]) printf 'C';; *) printf 'c';; esac
            case 'z' in [!]a]) printf 'D';; *) printf 'd';; esac
            case ']' in [ab\]]) printf 'E';; *) printf 'e';; esac
            printf '\n'
            """
        ),
        stdout="ABcDE\n",
    ),
    # Character classes, equivalence classes and collating symbols are part
    # of the RE bracket expression syntax that 2.14.1 adopts.
    # [spec:posix:syn:pattern.bracket-expression/test]
    # [spec:posix:syn:pattern.single-character-patterns/test]
    Case(
        id="adv-pattern-bracket-classes",
        rules=("pattern.bracket-expression", "pattern.single-character-patterns"),
        script=_s(
            r"""
            case '5' in [[:digit:]]) printf 'A';; *) printf 'a';; esac
            case '5' in [x[:digit:]]) printf 'B';; *) printf 'b';; esac
            case '.' in [[:punct:]]) printf 'C';; *) printf 'c';; esac
            case 'a' in [[=a=]]) printf 'D';; *) printf 'd';; esac
            case 'a' in [[.a.]]) printf 'E';; *) printf 'e';; esac
            case '-' in [a-]) printf 'F';; *) printf 'f';; esac
            case 'b' in [a-c]) printf 'G';; *) printf 'g';; esac
            printf '\n'
            """
        ),
        stdout="ABCDEFG\n",
    ),
    # "A <left-square-bracket> that does not introduce a valid bracket
    # expression shall match the character itself", and a quoted special
    # pattern character matches itself.
    # [spec:posix:sem:pattern.left-bracket-literal/test]
    # [spec:posix:req:pattern.quote-to-match-literally/test]
    Case(
        id="adv-pattern-literal-bracket-and-quoting",
        rules=("pattern.left-bracket-literal", "pattern.quote-to-match-literally"),
        script=_s(
            r"""
            case '[x' in [x) printf 'A';; *) printf 'a';; esac
            case '[' in [) printf 'B';; *) printf 'b';; esac
            case '*' in "*") printf 'C';; *) printf 'c';; esac
            case 'zz' in "*") printf 'D';; *) printf 'd';; esac
            case '?' in \?) printf 'E';; *) printf 'e';; esac
            printf '\n'
            """
        ),
        stdout="ABCdE\n",
    ),
    # A leading <period> is matched only by an explicit <period>: not by
    # '*', not by '?', and not by a non-matching list. The replacement list
    # is sorted by the collating sequence of the current locale (LC_ALL=C
    # here, so by byte: 'B' 0x42, '_' 0x5F, 'a' 0x61).
    # [spec:posix:req:pattern.leading-period/test]
    # [spec:posix:req:pattern.replacement-sorted/test]
    Case(
        id="adv-pattern-leading-period-and-order",
        rules=("pattern.leading-period", "pattern.replacement-sorted"),
        script=_s(
            r"""
            mkdir sub && cd sub || exit 1
            : > .hidden
            : > B
            : > _
            : > a
            printf '[%s]' *
            printf '\n'
            printf '[%s]' [!x]*
            printf '\n'
            printf '[%s]' .h*
            printf '\n'
            """
        ),
        stdout="[B][_][a]\n[B][_][a]\n[.hidden]\n",
    ),
    # A <slash> "shall be explicitly matched": neither '?' nor a bracket
    # expression can match it, and a '/' found before the closing ']'
    # makes the '[' an ordinary character. A pattern that matches nothing
    # is left unchanged.
    # [spec:posix:req:pattern.slash-explicit-match/test]
    # [spec:posix:syn:pattern.slash-terminates-bracket/test]
    # [spec:posix:req:pattern.no-match-unchanged/test]
    Case(
        id="adv-pattern-slash-boundaries",
        rules=(
            "pattern.slash-explicit-match",
            "pattern.slash-terminates-bracket",
            "pattern.no-match-unchanged",
        ),
        script=_s(
            r"""
            mkdir sub && cd sub || exit 1
            mkdir q
            : > q/c
            printf '[%s]' q?c
            printf '[%s]' 'a[b/c]d'
            printf '[%s]' nomatch*
            printf '[%s]' q/*
            printf '\n'
            """
        ),
        stdout="[q?c][a[b/c]d][nomatch*][q/c]\n",
    ),

    # =================================================================
    # 6. Redirection
    # =================================================================
    # noclobber: '>' shall fail on an existing regular file, ">|" overrides
    # it for that file, ">>" is unaffected, and a file that does not exist
    # is still created.
    # [spec:posix:req:redir.output-noclobber/test]
    # [spec:posix:req:builtin.set.opt-c-noclobber/test]
    # [spec:posix:req:redir.append/test]
    Case(
        id="adv-redir-noclobber-matrix",
        rules=("redir.output-noclobber", "builtin.set.opt-c-noclobber", "redir.append"),
        script=_s(
            r"""
            set -C
            echo first > f
            echo second > f 2>/dev/null
            # POSIX requires only that the redirection fail; the particular
            # non-zero status is not specified (nsh and dash use 2, bash 1).
            printf 'clobber=%s ' "$([ "$?" -gt 0 ] && echo failed)"
            echo third >| f
            printf 'override=%s ' "$(cat f)"
            echo fourth >> f
            printf 'append=%s ' "$(tr '\n' ',' < f)"
            echo new > fresh
            printf 'created=%s\n' "$(cat fresh)"
            """
        ),
        stdout="clobber=failed override=third append=third,fourth, created=new\n",
        status=0,
    ),
    # 2.8.1: a redirection error with a special built-in shall exit a
    # non-interactive shell; the same error with a compound command or a
    # function shall not. bash outside POSIX mode continues in both cases.
    # [spec:posix:req:exit.shell-error-consequences/test]
    # [spec:posix:req:redir.open-failure/test]
    Case(
        id="adv-redir-error-special-builtin-exits",
        rules=("exit.shell-error-consequences", "redir.open-failure"),
        script=_s(
            r"""
            f() { echo function-body; }
            { echo group-body; } < missing-file
            printf 'group=%s ' "$?"
            f < missing-file
            printf 'function=%s\n' "$?"
            : < missing-file
            echo reached
            """
        ),
        stdout_contains=("group=", "function="),
        stdout_excludes=("reached",),
        stdout=None,
        status="nonzero",
    ),
    # "[n]<>word" opens for reading and writing and creates the file if it
    # does not exist; "[n]>&-" closes; "Attempts to close a file descriptor
    # that is not open shall not constitute an error."
    # [spec:posix:req:redir.open-read-write/test]
    # [spec:posix:req:redir.dup-output-close/test]
    # [spec:posix:req:redir.dup-input-close/test]
    Case(
        id="adv-redir-open-rw-and-close",
        rules=("redir.open-read-write", "redir.dup-output-close", "redir.dup-input-close"),
        script=_s(
            r"""
            exec 3<> created
            echo written >&3
            exec 3>&-
            printf 'rw=%s ' "$(cat created)"
            exec 9>&-
            printf 'close-unopened=%s ' "$?"
            exec 8<&-
            printf 'close-unopened-input=%s\n' "$?"
            """
        ),
        stdout="rw=written close-unopened=0 close-unopened-input=0\n",
    ),
    # "If word evaluates to one or more digits, the file descriptor denoted
    # by word shall be made to be a copy of the file descriptor denoted by
    # word" -- "03" is one or more digits denoting file descriptor 3, and
    # IO_NUMBER is likewise "a string solely of digits". dash rejects both
    # with "Bad fd number".
    # [spec:posix:req:redir.dup-output/test]
    # [spec:posix:req:redir.dup-input/test]
    Case(
        id="adv-redir-dup-leading-zero",
        rules=("redir.dup-output", "redir.dup-input"),
        script=_s(
            r"""
            exec 03> out
            echo written >&03
            exec 3>&-
            printf 'out=%s ' "$(cat out)"
            printf 'line\n' > in
            exec 3< in
            read got <&03
            exec 3<&-
            printf 'in=%s\n' "$got"
            """
        ),
        stdout="out=written in=line\n",
    ),
    # "Pathname expansion shall not be performed on the word by a
    # non-interactive shell", and the redirection operator and its word
    # "shall not appear in the arguments provided to the command".
    # Redirections are evaluated beginning to end, so "2>&1 >f" leaves
    # stderr on the original standard output.
    # [spec:posix:req:redir.word-pathname-expansion/test]
    # [spec:posix:req:redir.not-in-command-arguments/test]
    # [spec:posix:sem:redir.evaluation-order/test]
    Case(
        id="adv-redir-word-and-order",
        rules=(
            "redir.word-pathname-expansion",
            "redir.not-in-command-arguments",
            "redir.evaluation-order",
        ),
        script=_s(
            r"""
            : > aa
            echo x > a*
            if [ -e 'a*' ]; then printf 'literal '; else printf 'globbed '; fi
            echo one > args two
            printf 'args=%s ' "$(cat args)"
            sh -c 'echo to-stderr >&2' 2>&1 > swallowed
            printf 'swallowed=%s\n' "$(cat swallowed)"
            """
        ),
        # "2>&1 > swallowed" duplicates the standard output that was in
        # effect when 2>&1 was evaluated, so the diagnostic lands on the
        # original standard output and only fd 1 is redirected afterwards.
        stdout="literal args=one two to-stderr\nswallowed=\n",
        stderr="",
    ),
    # Here-document line continuation: with an unquoted delimiter word,
    # <backslash><newline> removal happens "during the search for the
    # trailing delimiter", so the delimiter is not recognised on a line
    # joined to the previous one. With a quoted delimiter word no such
    # removal occurs and the lines are not expanded.
    # [spec:posix:req:redir.here-doc-line-continuation/test]
    # [spec:posix:req:redir.here-doc-quoted-delimiter/test]
    # [spec:posix:req:redir.here-doc-unquoted-delimiter/test]
    Case(
        id="adv-redir-heredoc-continuation",
        rules=(
            "redir.here-doc-line-continuation",
            "redir.here-doc-quoted-delimiter",
            "redir.here-doc-unquoted-delimiter",
        ),
        script=_s(
            r"""
            v=EXPANDED
            cat <<EOF
            joined\
            EOF
            tail $v
            EOF
            cat <<'EOF'
            kept\
            $v
            EOF
            """
        ),
        stdout="joinedEOF\ntail EXPANDED\nkept\\\n$v\n",
    ),
    # "<<-" strips leading <tab> characters -- only tabs, and also from the
    # line containing the trailing delimiter -- as the here-document is read
    # from the shell input, so tabs produced by expansions survive.
    # [spec:posix:req:redir.here-doc-tab-strip/test]
    # [spec:posix:req:redir.here-doc-expansion/test]
    Case(
        id="adv-redir-heredoc-tab-strip",
        rules=("redir.here-doc-tab-strip", "redir.here-doc-expansion"),
        script=(
            "v='\tfrom-expansion'\n"
            "cat <<-EOF\n"
            "\tone\n"
            "\t\ttwo\n"
            "  three\n"
            "\t$v\n"
            "\tEOF\n"
            "echo done\n"
        ),
        stdout="one\ntwo\n  three\n\tfrom-expansion\ndone\n",
    ),
    # In an unquoted here-document "any <backslash> characters in the input
    # shall behave as the <backslash> inside double-quotes", except that
    # '"' is not special there -- so \$ and \\ are escapes while \z and \"
    # keep both characters.
    # [spec:posix:req:redir.here-doc-backslash/test]
    # [spec:posix:req:redir.here-doc-delimiter/test]
    Case(
        id="adv-redir-heredoc-backslash",
        rules=("redir.here-doc-backslash", "redir.here-doc-delimiter"),
        script=_s(
            r"""
            v=EXPANDED
            cat <<EOF
            $v \$v \\ \z \" "q"
            EOF x
            EOF
            echo done
            """
        ),
        # "EOF x" is not the delimiter: the terminating line contains only
        # the delimiter and a <newline>, "with no <blank> characters in
        # between".
        stdout='EXPANDED $v \\ \\z \\" "q"\nEOF x\ndone\n',
    ),

    # =================================================================
    # 7. Command search and execution
    # =================================================================
    # "If the command name is a special built-in utility, variable
    # assignments shall affect the current execution environment before the
    # utility is executed and remain in effect when the command completes."
    # For any other utility they "shall not affect the current execution
    # environment". bash outside POSIX mode does not keep the special
    # built-in assignment.
    # [spec:posix:req:cmd.assign-special-builtin/test]
    # [spec:posix:req:cmd.assign-exported-to-command/test]
    # [spec:posix:req:builtin.special.preceding-assignments-persist/test]
    Case(
        id="adv-exec-assignment-persistence",
        rules=(
            "cmd.assign-special-builtin",
            "cmd.assign-exported-to-command",
            "builtin.special.preceding-assignments-persist",
        ),
        script=_s(
            r"""
            v=outer
            v=inner :
            printf 'special=%s ' "$v"
            v=outer
            v=inner true
            printf 'utility=%s ' "$v"
            v=exported sh -c 'printf "child=%s " "$v"'
            printf '\n'
            """
        ),
        stdout="special=inner utility=outer child=exported \n",
    ),
    # "command" suppresses the special-built-in properties: the shell shall
    # not exit on an error, and the preceding assignments do not persist.
    # The same error without "command" shall exit a non-interactive shell.
    # [spec:posix:req:builtin.command.special-builtin-properties-suppressed/test]
    # [spec:posix:req:exit.shell-error-consequences/test]
    Case(
        id="adv-exec-command-suppresses-special",
        rules=(
            "builtin.command.special-builtin-properties-suppressed",
            "exit.shell-error-consequences",
        ),
        script=_s(
            r"""
            v=outer
            v=inner command :
            printf 'assign=%s ' "$v"
            command set -o no-such-option 2>/dev/null
            printf 'survived=%s\n' "$?"
            set -o no-such-option 2>/dev/null
            echo reached
            """
        ),
        stdout_contains=("assign=outer", "survived="),
        stdout_excludes=("reached",),
        stdout=None,
        status="nonzero",
    ),
    # Search order: a function is found at step 1c, before the intrinsic
    # utility of the same name at step 1d; "command" suppresses the
    # function lookup so the intrinsic utility runs.
    # [spec:posix:req:cmd.search-function/test]
    # [spec:posix:req:cmd.search-intrinsic-utility/test]
    # [spec:posix:req:builtin.command.suppress-function-lookup/test]
    Case(
        id="adv-exec-function-before-intrinsic",
        rules=(
            "cmd.search-function",
            "cmd.search-intrinsic-utility",
            "builtin.command.suppress-function-lookup",
        ),
        script=_s(
            r"""
            start=$PWD
            mkdir target
            cd() { printf 'function '; }
            cd target
            printf 'still=%s ' "$([ "$PWD" = "$start" ] && echo yes)"
            command cd target
            printf 'builtin=%s\n' "${PWD##*/}"
            """
        ),
        stdout="function still=yes builtin=target\n",
    ),
    # "a variable assignment error shall occur" when an assignment targets a
    # readonly variable, and 2.8.1 makes that fatal for a non-interactive
    # shell -- including when the assignment is a prefix to another utility
    # and so is not even made in the current environment.
    # [spec:posix:req:cmd.assign-readonly-error/test]
    # [spec:posix:req:exit.shell-error-consequences/test]
    Case(
        id="adv-exec-readonly-assignment-error",
        rules=("cmd.assign-readonly-error", "exit.shell-error-consequences"),
        script=_s(
            r"""
            readonly r=1
            r=2 true
            echo reached
            """
        ),
        stdout="",
        status="nonzero",
    ),
    # 127 for "not found", 126 for "found, but it is not an executable
    # utility". The PATH search failure must also produce a diagnostic.
    # [spec:posix:req:exit.status-command-not-found/test]
    # [spec:posix:req:exit.status-not-executable/test]
    # [spec:posix:req:cmd.search-path-unsuccessful/test]
    Case(
        id="adv-exec-status-126-127",
        rules=(
            "exit.status-command-not-found",
            "exit.status-not-executable",
            "cmd.search-path-unsuccessful",
        ),
        files={"plain": FileFixture("not executable\n", 0o644)},
        script=_s(
            r"""
            no-such-utility-xyz 2>/dev/null
            printf 'notfound=%s ' "$?"
            ./plain 2>/dev/null
            printf 'notexec=%s ' "$?"
            ./ 2>/dev/null
            printf 'directory=%s\n' "$?"
            """
        ),
        stdout="notfound=127 notexec=126 directory=126\n",
    ),
    # "If there is no command name but the command contains a command
    # substitution, the command shall complete with the exit status of the
    # command substitution whose exit status was the last to be obtained."
    # Redirections without a command name are performed in a subshell, so
    # they do not change the current environment.
    # [spec:posix:req:cmd.no-name-exit-status/test]
    # [spec:posix:req:cmd.no-name-redirections-subshell/test]
    # [spec:posix:sem:param.special-question-assignment/test]
    Case(
        id="adv-exec-no-command-name",
        rules=(
            "cmd.no-name-exit-status",
            "cmd.no-name-redirections-subshell",
            "param.special-question-assignment",
        ),
        script=_s(
            r"""
            printf 'first\nsecond\n' > data
            v=$(sh -c 'exit 3')$(sh -c 'exit 7')
            printf 'status=%s ' "$?"
            > empty
            printf 'plain=%s ' "$?"
            < data
            read line
            printf 'read=%s line=[%s]\n' "$([ "$?" -gt 0 ] && echo eof)" "$line"
            """
        ),
        # The `< data` command has no command name, so the redirection
        # happens in a subshell; the shell's own standard input is
        # untouched and the following read sees end-of-file.
        stdout="status=7 plain=0 read=eof line=[]\n",
    ),
    # The dot special built-in is a special built-in: failing to find the
    # file is a special-built-in error, which shall exit a non-interactive
    # shell.
    # [spec:posix:req:builtin.dot.path-search/test]
    # [spec:posix:req:builtin.dot.exit-status/test]
    Case(
        id="adv-exec-dot-not-found-exits",
        rules=("builtin.dot.path-search", "builtin.dot.exit-status"),
        script=_s(
            r"""
            . no-such-dot-file-xyz 2>/dev/null
            echo reached
            """
        ),
        stdout="",
        status="nonzero",
    ),

    # =================================================================
    # 8. Exit status and set -e
    # =================================================================
    # Exception 2 of set -e: the setting "shall be ignored when executing
    # the compound list following the while, until, if, or elif reserved
    # word, a pipeline beginning with the ! reserved word, or any command
    # of an AND-OR list other than the last".
    # [spec:posix:req:builtin.set.opt-e-errexit/test]
    Case(
        id="adv-errexit-suppressed-contexts",
        rules=("builtin.set.opt-e-errexit",),
        script=_s(
            r"""
            set -e
            if false; then :; fi;            printf 'if '
            if false; then :; elif false; then :; fi; printf 'elif '
            while false; do :; done;         printf 'while '
            until true; do :; done;          printf 'until '
            ! false;                         printf 'bang '
            false && echo no;                printf 'and '
            false || true;                   printf 'or '
            false | true;                    printf 'pipeline '
            printf 'done\n'
            """
        ),
        stdout="if elif while until bang and or pipeline done\n",
    ),
    # The complement: -e applies to the last command of an AND-OR list, to
    # the pipeline's own status, and inside loop and case bodies.
    # [spec:posix:req:builtin.set.opt-e-errexit/test]
    # [spec:posix:req:cmd.pipeline-exit-status/test]
    Case(
        id="adv-errexit-applies-contexts",
        rules=("builtin.set.opt-e-errexit", "cmd.pipeline-exit-status"),
        script=_s(
            r"""
            (set -e; true && false; echo no) ; printf 'andor=%s ' "$?"
            (set -e; true | false; echo no)  ; printf 'pipeline=%s ' "$?"
            (set -e; for i in 1; do false; done; echo no); printf 'for=%s ' "$?"
            (set -e; case a in a) false ;; esac; echo no); printf 'case=%s\n' "$?"
            """
        ),
        stdout="andor=1 pipeline=1 for=1 case=1\n",
    ),
    # Both examples are given verbatim in the description of set -e:
    # "set -e; (false; echo one) | cat; echo two" prints two, and
    # "set -e; echo $(false; echo one) two" prints two -- the subshell in
    # which the command substitution is performed exits without executing
    # "echo one". bash prints "one two" for the second, contradicting the
    # standard's own worked example.
    # [spec:posix:req:builtin.set.opt-e-per-environment/test]
    Case(
        id="adv-errexit-per-environment-examples",
        rules=("builtin.set.opt-e-per-environment",),
        script=_s(
            r"""
            set -e
            (false; echo one) | cat
            echo two
            echo $(false; echo one) three
            """
        ),
        stdout="two\nthree\n",
    ),
    # A utility's exit status is "the value obtained by the equivalent of
    # the WEXITSTATUS macro", i.e. modulo 256; a command killed by a signal
    # gets a status greater than 128; and "!" yields 0 or 1 only, never the
    # negated pipeline's own status.
    # [spec:posix:req:exit.status-normal-termination/test]
    # [spec:posix:req:exit.status-signal-terminated/test]
    # [spec:posix:req:cmd.pipeline-exit-status/test]
    Case(
        id="adv-status-wait-status-mapping",
        rules=(
            "exit.status-normal-termination",
            "exit.status-signal-terminated",
            "cmd.pipeline-exit-status",
        ),
        script=_s(
            r"""
            sh -c 'exit 300'
            printf 'mod=%s ' "$?"
            sh -c 'kill -TERM $$' 2>/dev/null
            printf 'signal=%s ' "$([ "$?" -gt 128 ] && echo gt128)"
            ! sh -c 'exit 3'
            printf 'bang=%s ' "$?"
            ! true
            printf 'bangtrue=%s\n' "$?"
            """
        ),
        stdout="mod=44 signal=gt128 bang=0 bangtrue=1\n",
    ),
    # "When a subshell environment is created, the value of the special
    # parameter '?' from the invoking shell environment shall be preserved
    # in the subshell", and an assignment whose value comes from a command
    # substitution takes that substitution's status.
    # [spec:posix:req:param.special-question/test]
    Case(
        id="adv-status-question-preserved",
        rules=("param.special-question",),
        script=_s(
            r"""
            sh -c 'exit 5'
            (printf 'subshell=%s ' "$?")
            printf 'current=%s ' "$?"
            v=$(sh -c 'exit 7')
            printf 'assign=%s\n' "$?"
            """
        ),
        stdout="subshell=5 current=0 assign=7\n",
    ),
    # With set -e the shell exits "as if by executing the exit special
    # built-in utility with no arguments", i.e. with the status of the
    # failed command -- and the EXIT trap that runs first must not change
    # it: "The value of "$?" after the trap action completes shall be the
    # value it had before the trap action was executed."
    # [spec:posix:req:builtin.set.opt-e-errexit/test]
    # [spec:posix:req:builtin.trap.action-overrides-and-exit-status/test]
    Case(
        id="adv-errexit-exit-trap-status",
        rules=(
            "builtin.set.opt-e-errexit",
            "builtin.trap.action-overrides-and-exit-status",
        ),
        script=_s(
            r"""
            set -e
            trap 'echo trapped' EXIT
            sh -c 'exit 6'
            echo reached
            """
        ),
        stdout="trapped\n",
        status=6,
    ),

    # =================================================================
    # 9. Traps and signals
    # =================================================================
    # Reaching end of input terminates the shell "in the same manner as for
    # an exit command with no operands", whose n is "the current value of
    # the special parameter '?'" -- and 2.15's trap description fixes that
    # value across the trap action: "The value of "$?" after the trap
    # action completes shall be the value it had before the trap action was
    # executed."
    # [spec:posix:req:builtin.trap.action-overrides-and-exit-status/test]
    # [spec:posix:req:sh.exit-status-otherwise/test]
    # [spec:posix:req:builtin.exit.default-n/test]
    Case(
        id="adv-trap-exit-preserves-status",
        rules=(
            "builtin.trap.action-overrides-and-exit-status",
            "sh.exit-status-otherwise",
            "builtin.exit.default-n",
        ),
        script=_s(
            r"""
            trap 'printf "seen=%s\n" "$?"' EXIT
            sh -c 'exit 4'
            """
        ),
        stdout="seen=4\n",
        status=4,
    ),
    # The same requirement one level down: a subshell exits with the status
    # its last command left, not with the status of its EXIT trap action.
    # [spec:posix:req:builtin.trap.action-overrides-and-exit-status/test]
    # [spec:posix:req:builtin.exit.exit-trap/test]
    Case(
        id="adv-trap-exit-status-subshell",
        rules=(
            "builtin.trap.action-overrides-and-exit-status",
            "builtin.exit.exit-trap",
        ),
        script=_s(
            r"""
            (trap 'true' EXIT; sh -c 'exit 3')
            printf 'subshell=%s ' "$?"
            (trap 'sh -c "exit 9"' EXIT; true)
            printf 'action-ignored=%s\n' "$?"
            """
        ),
        stdout="subshell=3 action-ignored=0\n",
    ),
    # A signal trap action likewise leaves "$?" as it was: the action here
    # ends with a command whose status is 9, which must not be visible.
    # [spec:posix:req:builtin.trap.action-executed-as-eval/test]
    # [spec:posix:req:builtin.trap.action-overrides-and-exit-status/test]
    Case(
        id="adv-trap-signal-status-restored",
        rules=(
            "builtin.trap.action-executed-as-eval",
            "builtin.trap.action-overrides-and-exit-status",
        ),
        script=_s(
            r"""
            trap 'printf "in=%s " "$((1+1))"; sh -c "exit 9"' USR1
            kill -USR1 $$
            printf 'after=%s\n' "$?"
            """
        ),
        # "after" is the status of the kill, not of the trap action.
        stdout="in=2 after=0\n",
    ),
    # "When a subshell is entered, traps that are not being ignored shall be
    # set to the default actions" -- so the EXIT trap fires once, in the
    # parent, after the subshell has already finished.
    # [spec:posix:req:builtin.trap.subshell-reset/test]
    # [spec:posix:req:builtin.trap.exit-condition/test]
    Case(
        id="adv-trap-subshell-reset",
        rules=("builtin.trap.subshell-reset", "builtin.trap.exit-condition"),
        script=_s(
            r"""
            trap 'echo exit-trap' EXIT
            (true)
            echo after-subshell
            """
        ),
        stdout="after-subshell\nexit-trap\n",
    ),
    # POSIX.1-2024 tightened this: when trap is run in a subshell and no
    # trap command with operands has been executed since entering it, "the
    # list shall contain the commands that were associated with each
    # condition immediately before the subshell environment was entered".
    # The format is fixed: "trap -- %s %s ...\n" with the condition named
    # as 2.15 defines it -- "a symbolic name, without the SIG prefix". dash
    # prints nothing at all for either of these; bash prints SIGUSR1, a
    # spelling the standard only permits an implementation to ACCEPT as an
    # extension.
    # [spec:posix:req:builtin.trap.list-in-subshell/test]
    # [spec:posix:syn:builtin.trap.list-format/test]
    # [spec:posix:req:builtin.trap.list-condition-set/test]
    Case(
        id="adv-trap-list-in-subshell",
        rules=(
            "builtin.trap.list-in-subshell",
            "builtin.trap.list-format",
            "builtin.trap.list-condition-set",
        ),
        script=_s(
            r"""
            trap 'echo "a b"' USR1
            trap | cat
            printf '[%s]\n' "$(trap)"
            (trap)
            """
        ),
        stdout=(
            "trap -- 'echo \"a b\"' USR1\n"
            "[trap -- 'echo \"a b\"' USR1]\n"
            "trap -- 'echo \"a b\"' USR1\n"
        ),
    ),
    # "If the -p option is not specified and the first operand is an
    # unsigned decimal integer, the shell shall treat all operands as
    # conditions, and shall reset each condition to the default value" --
    # so `trap 0` resets the EXIT trap rather than setting an action named
    # "0". EXIT and 0 are the same condition.
    # [spec:posix:req:builtin.trap.operand-interpretation/test]
    # [spec:posix:def:builtin.trap.condition/test]
    Case(
        id="adv-trap-numeric-first-operand",
        rules=("builtin.trap.operand-interpretation", "builtin.trap.condition"),
        script=_s(
            r"""
            trap 'echo should-not-run' EXIT
            trap 0
            printf 'reset=%s\n' "$(trap)"
            trap 'echo zero-condition' 0
            """
        ),
        stdout="reset=\nzero-condition\n",
    ),
    # "Signals that were ignored on entry to a non-interactive shell cannot
    # be trapped or reset, although no error need be reported when
    # attempting to do so" -- the inner shell's trap action must not run.
    # [spec:posix:req:builtin.trap.signals-ignored-on-entry/test]
    # [spec:posix:req:signal.inherited-actions/test]
    Case(
        id="adv-trap-ignored-on-entry",
        rules=("builtin.trap.signals-ignored-on-entry", "signal.inherited-actions"),
        script=_s(
            r"""
            trap '' USR1
            sh -c 'trap "echo caught" USR1; kill -USR1 $$; echo child-alive'
            printf 'parent=%s\n' "$?"
            """
        ),
        stdout="child-alive\nparent=0\n",
    ),
    # "For both interactive and non-interactive shells, invalid signal
    # names shall not be considered an error and shall not cause the shell
    # to abort", but trap "shall write a warning message" and return
    # non-zero.
    # [spec:posix:req:builtin.trap.exit-status/test]
    # [spec:posix:req:builtin.trap.invalid-condition-warning/test]
    Case(
        id="adv-trap-invalid-condition",
        rules=("builtin.trap.exit-status", "builtin.trap.invalid-condition-warning"),
        script=_s(
            r"""
            trap 'echo x' NO-SUCH-SIGNAL 2>diag
            printf 'status=%s ' "$([ "$?" -gt 0 ] && echo nonzero)"
            printf 'diagnosed=%s ' "$([ -s diag ] && echo yes)"
            printf 'alive=yes\n'
            """
        ),
        stdout="status=nonzero diagnosed=yes alive=yes\n",
    ),
    # "If job control is disabled when the shell executes an asynchronous
    # AND-OR list, the commands in the list shall inherit from the shell a
    # signal action of ignored (SIG_IGN) for the SIGINT and SIGQUIT
    # signals." The sleep gives the child time to reach the ignoring state
    # before the signals arrive; a race here would be a bug in the case,
    # not a finding.
    # [spec:posix:req:signal.async-list-sigint-sigquit-ignored/test]
    Case(
        id="adv-signal-async-ignores-int-quit",
        rules=("signal.async-list-sigint-sigquit-ignored",),
        script=_s(
            r"""
            (sleep 1; echo survived) &
            child=$!
            sleep 0.3
            kill -INT "$child"
            kill -QUIT "$child"
            wait "$child"
            printf 'status=%s\n' "$?"
            """
        ),
        stdout="survived\nstatus=0\n",
        timeout=15.0,
    ),
    # "the reception of a signal for which a trap has been set shall cause
    # the wait utility to return immediately with an exit status >128,
    # immediately after which the trap associated with that signal shall be
    # taken".
    # [spec:posix:req:signal.trap-during-wait/test]
    Case(
        id="adv-signal-trap-during-wait",
        rules=("signal.trap-during-wait",),
        script=_s(
            r"""
            trap 'echo trap-taken' USR1
            sleep 5 &
            slow=$!
            (sleep 0.3; kill -USR1 $$) &
            wait "$slow"
            status=$?
            printf 'gt128=%s\n' "$([ "$status" -gt 128 ] && echo yes)"
            kill "$slow" 2>/dev/null
            """
        ),
        stdout="trap-taken\ngt128=yes\n",
        timeout=15.0,
    ),

    # =================================================================
    # 10. Arithmetic expansion
    # =================================================================
    # "Only the decimal-constant, octal-constant, and hexadecimal-constant
    # constants specified in the ISO C standard ... are required to be
    # recognized": 010 is octal, 0x1f and 0X1F are hexadecimal. "08" is not
    # a valid octal-constant, so the expression is invalid and "the
    # expansion fails and the shell shall write a diagnostic message".
    # [spec:posix:req:expand.arith-evaluation/test]
    # [spec:posix:req:expand.arith-invalid-expression/test]
    Case(
        id="adv-arith-constant-bases",
        rules=("expand.arith-evaluation", "expand.arith-invalid-expression"),
        script=_s(
            r"""
            printf '[%s]' "$((010))" "$((0x1f))" "$((0X1F))" "$((0))" "$((9223372036854775807))"
            printf '\n'
            echo "$((08))"
            echo reached
            """
        ),
        stdout="[8][31][31][0][9223372036854775807]\n",
        status="nonzero",
    ),
    # XCU 1.1.2.1: "All variables shall be initialized to zero if they are
    # not otherwise assigned by the input to the application" -- but under
    # set -u the expansion of an unset parameter in an arithmetic expansion
    # "shall fail" (dash returns 0 here instead).
    # [spec:posix:req:xcurel.arithmetic-variable-initialization/test]
    # [spec:posix:req:builtin.set.opt-u-nounset/test]
    Case(
        id="adv-arith-unset-is-zero",
        rules=(
            "xcurel.arithmetic-variable-initialization",
            "builtin.set.opt-u-nounset",
        ),
        script=_s(
            r"""
            unset x
            printf '[%s]' "$((x))" "$((x+1))"
            empty=
            printf '[%s]' "$((empty+2))"
            printf '\n'
            set -u
            echo "$((x))"
            echo reached
            """
        ),
        stdout="[0][1][2]\n",
        status="nonzero",
    ),
    # Operator semantics "shall be equivalent to that described in Section
    # 6.5, Expressions, of the ISO C standard": precedence, C truncation
    # toward zero for / and %, the value of a relational operator, and
    # short-circuit evaluation that skips the assignment in the dead
    # operand.
    # [spec:posix:req:xcurel.arithmetic-operators/test]
    # [spec:posix:req:xcurel.arithmetic-expression-evaluation/test]
    Case(
        id="adv-arith-operator-semantics",
        rules=(
            "xcurel.arithmetic-operators",
            "xcurel.arithmetic-expression-evaluation",
        ),
        script=_s(
            r"""
            printf '[%s]' "$((2+3*4))" "$(((1+2)*3))" "$((-3/2))" "$((-3%2))" \
                          "$((1<<3))" "$((~0))" "$((!0))" "$((!1+1))" \
                          "$((1<2))" "$((2<1))" "$((1?2:3))"
            printf '\n'
            unset z
            printf '[%s][%s]' "$((0 && (z=1)))" "${z-untouched}"
            printf '[%s][%s]' "$((1 || (z=2)))" "${z-untouched}"
            printf '\n'
            """
        ),
        stdout="[14][9][-1][-1][8][-1][1][1][1][0][2]\n[0][untouched][1][untouched]\n",
    ),
    # "All changes to variables in an arithmetic expression shall be in
    # effect after the arithmetic expansion", including the compound
    # assignment operators.
    # [spec:posix:req:expand.arith-variable-changes/test]
    # [spec:posix:req:expand.arith-token-expansion/test]
    Case(
        id="adv-arith-assignment-effects",
        rules=("expand.arith-variable-changes", "expand.arith-token-expansion"),
        script=_s(
            r"""
            unset a b
            : $((a = 5))
            b=2
            : $((b *= 3))
            printf '[%s][%s]' "$a" "$b"
            printf '[%s]' "$((c = d = 4))"
            printf '[%s][%s]' "$c" "$d"
            n=7
            printf '[%s]' "$(( $n + 1 ))"
            printf '\n'
            """
        ),
        stdout="[5][6][4][4][4][8]\n",
    ),
    # "If the shell variable x contains a value that forms a valid integer
    # constant, optionally including a leading <plus-sign> or
    # <hyphen-minus>, then the arithmetic expansions "$((x))" and "$(($x))"
    # shall return the same value" -- including for the signed and
    # hexadecimal forms.
    # [spec:posix:req:expand.arith-variable-reference/test]
    Case(
        id="adv-arith-variable-reference",
        rules=("expand.arith-variable-reference",),
        script=_s(
            r"""
            for v in 5 +5 -5 0x10 010; do
                a=$v
                printf '[%s=%s]' "$((a))" "$(($a))"
            done
            printf '\n'
            """
        ),
        stdout="[5=5][5=5][-5=-5][16=16][8=8]\n",
    ),
    # An assignment inside an arithmetic expansion is still a variable
    # assignment: targeting a readonly variable is a variable assignment
    # error, which shall exit a non-interactive shell.
    # [spec:posix:req:exit.shell-error-consequences/test]
    # [spec:posix:def:builtin.readonly.attribute/test]
    Case(
        id="adv-arith-readonly-assignment",
        rules=("exit.shell-error-consequences", "builtin.readonly.attribute"),
        script=_s(
            r"""
            readonly r=1
            echo "$((r = 2))"
            echo reached
            """
        ),
        stdout="",
        status="nonzero",
    ),
)
