"""Executable cases for the shell language core.

Covers the rule wording in posix/docs/spec/{expansion,parameters,grammar,
quoting,execution,pattern-matching,redirection,tokens}.md plus the last
uncovered rules of commands.md.

Every expectation below states what POSIX.1-2024 requires, not what any
particular shell happens to do. Cases that fail are conformance findings.
"""

from __future__ import annotations

import textwrap

from model import Case, FileFixture


def _s(text: str) -> str:
    """Turn an indented raw literal into a shell script.

    Scripts are written as raw strings so that a <backslash> in the shell
    source stays a <backslash>; mis-escaping is the commonest way to write a
    case that fails for the wrong reason.
    """

    return textwrap.dedent(text).lstrip("\n")


TAB = "\t"


CASES: tuple[Case, ...] = (
    # -----------------------------------------------------------------
    # 2.1 Shell Introduction
    # -----------------------------------------------------------------
    # [spec:posix:sem:shell.input-sources/test]
    Case(
        id="lang-shell-input-sources",
        rules=("shell.input-sources",),
        script=_s(
            r"""
            printf 'printf FILE\n' > s.sh
            sh s.sh
            printf 'printf STDIN\n' | sh
            printf 'CMD\n'
            """
        ),
        stdout="FILESTDINCMD\n",
    ),
    # [spec:posix:sem:shell.tokenization-and-parsing/test]
    Case(
        id="lang-shell-tokenization",
        rules=("shell.tokenization-and-parsing",),
        script=_s(
            r"""
            if true; then printf 'A'; fi
            { printf 'B'; }
            printf 'C\n'
            """
        ),
        stdout="ABC\n",
    ),
    # [spec:posix:sem:shell.word-processing/test]
    # [spec:posix:req:quote.dollar-single-quotes-processing-time/test]
    Case(
        id="lang-shell-word-processing",
        rules=("shell.word-processing", "quote.dollar-single-quotes-processing-time"),
        script=_s(
            r"""
            v=X
            printf '%s\n' $'\x24v' "$v"
            """
        ),
        stdout="$v\nX\n",
    ),
    # [spec:posix:sem:shell.redirection-processing/test]
    # [spec:posix:req:redir.not-in-command-arguments/test]
    Case(
        id="lang-shell-redirection-processing",
        rules=("shell.redirection-processing", "redir.not-in-command-arguments"),
        script=_s(
            r"""
            printf '%s\n' A >out B
            cat out
            """
        ),
        stdout="A\nB\n",
    ),
    # [spec:posix:sem:shell.command-execution/test]
    Case(
        id="lang-shell-command-execution",
        rules=("shell.command-execution",),
        script=_s(
            r"""
            cat > s.sh <<'EOS'
            f() { printf '%s|%s|%s\n' "$0" "$1" "$2"; }
            f A B
            EOS
            sh s.sh
            """
        ),
        stdout="s.sh|A|B\n",
    ),
    # [spec:posix:sem:shell.exit-status-collection/test]
    Case(
        id="lang-shell-exit-status-collection",
        rules=("shell.exit-status-collection",),
        script=_s(
            r"""
            (exit 4)
            printf '%s\n' "$?"
            """
        ),
        stdout="4\n",
    ),
    # -----------------------------------------------------------------
    # 2.2 Quoting
    # -----------------------------------------------------------------
    # [spec:posix:def:quote.purpose/test]
    Case(
        id="lang-quote-purpose",
        rules=("quote.purpose",),
        script=_s(
            r"""
            v=X
            cat <<'E'
            $v
            E
            ( "if" ) 2>/dev/null
            printf '%s\n' "$?"
            """
        ),
        stdout="$v\n127\n",
    ),
    # [spec:posix:req:quote.always-special-characters/test]
    Case(
        id="lang-quote-always-special",
        rules=("quote.always-special-characters",),
        script=(
            "printf '[%s]' '|' '&' ';' '<' '>' '(' ')' '$' '`' '\\' '\"' \\'"
            " ' ' '\t' '\n'\n"
            "printf '\\n'\n"
        ),
        stdout="[|][&][;][<][>][(][)][$][`][\\][\"]['][ ][\t][\n]\n",
    ),
    # [spec:posix:req:quote.conditionally-special-characters/test]
    Case(
        id="lang-quote-conditionally-special",
        rules=("quote.conditionally-special-characters",),
        script=_s(
            r"""
            : > afile
            printf '[%s]' '*' '?' '[' ']' '^' '-' '!' '#' '~' '=' '%' '{' ',' '}'
            printf '\n'
            """
        ),
        stdout="[*][?][[][]][^][-][!][#][~][=][%][{][,][}]\n",
    ),
    # [spec:posix:def:quote.mechanisms/test]
    Case(
        id="lang-quote-mechanisms",
        rules=("quote.mechanisms",),
        script=_s(
            r"""
            printf '%s|' a\ b 'c d' "e f" $'g\th'
            printf '\n'
            """
        ),
        stdout="a b|c d|e f|g\th|\n",
    ),
    # [spec:posix:req:quote.double-quotes-command-substitution/test]
    Case(
        id="lang-quote-double-cmdsub",
        rules=("quote.double-quotes-command-substitution",),
        script=_s(
            r"""
            printf '[%s]\n' "$(printf '%s' "p  q")"
            printf '[%s]\n' "$(echo 'a b')"
            """
        ),
        stdout="[p  q]\n[a b]\n",
    ),
    # [spec:posix:req:quote.double-quotes-substring-parameter-expansion/test]
    # [spec:posix:req:expand.param-substring-common/test]
    Case(
        id="lang-quote-double-substring-pe",
        rules=(
            "quote.double-quotes-substring-parameter-expansion",
            "expand.param-substring-common",
        ),
        script=_s(
            r"""
            v=abc
            printf '[%s]' "${v##*}" "${v##'*'}" "${v#}" "${v%}"
            printf '\n'
            sh -c 'set -u; unset u; printf "%s" "${u#x}"' 2>/dev/null
            printf 'nz=%s\n' "$(( $? != 0 ))"
            """
        ),
        stdout="[][abc][abc][abc]\nnz=1\n",
    ),
    # [spec:posix:req:quote.double-quotes-other-parameter-expansion/test]
    # [spec:posix:syn:expand.param-format/test]
    Case(
        id="lang-quote-double-other-pe",
        rules=(
            "quote.double-quotes-other-parameter-expansion",
            "expand.param-format",
        ),
        script=_s(
            r"""
            unset u
            w=W
            printf '[%s]' "${u:-a\}b}" "${u:-x$(printf '}')y}" "${u:-$w}"
            printf '[%s]' "${u:-`printf Z`}" "${u:-p  q}"
            printf '\n'
            """
        ),
        stdout="[a}b][x}y][W][Z][p  q]\n",
    ),
    # [spec:posix:req:quote.double-quotes-backquote/test]
    Case(
        id="lang-quote-double-backquote",
        rules=("quote.double-quotes-backquote",),
        script=_s(
            r"""
            printf '%s\n' "`printf hi`" "`printf x\`printf y\``"
            """
        ),
        stdout="hi\nxy\n",
    ),
    # [spec:posix:req:quote.double-quotes-backslash/test]
    # [spec:posix:req:quote.double-quotes-embedded-double-quote/test]
    Case(
        id="lang-quote-double-backslash",
        rules=(
            "quote.double-quotes-backslash",
            "quote.double-quotes-embedded-double-quote",
        ),
        script=_s(
            r"""
            printf '%s\n' "a\$b" "a\\b" "a\qb" "a\"b" "a\
            b"
            """
        ),
        stdout='a$b\na\\b\na\\qb\na"b\nab\n',
    ),
    # [spec:posix:req:quote.double-quotes-expansion-result/test]
    Case(
        id="lang-quote-double-expansion-result",
        rules=("quote.double-quotes-expansion-result",),
        script=_s(
            r"""
            : > zfile
            v='* a  b'
            printf '[%s]\n' "$v"
            set -- "$v"
            printf 'n=%s\n' "$#"
            """
        ),
        stdout="[* a  b]\nn=1\n",
    ),
    # [spec:posix:def:quote.dollar-single-quotes-escapes/test]
    Case(
        id="lang-quote-dollar-single-escapes",
        rules=("quote.dollar-single-quotes-escapes",),
        script=_s(
            r"""
            printf '%s' $'\"\'\\\a\b\e\f\n\r\t\v' | od -An -tx1 | tr -d ' \n'
            printf '\n'
            """
        ),
        stdout="22275c07081b0c0a0d090b\n",
    ),
    # -----------------------------------------------------------------
    # 2.3 Token Recognition / 2.4 Reserved Words
    # -----------------------------------------------------------------
    # [spec:posix:req:token.here-document-mode/test]
    # [spec:posix:req:redir.here-doc-multiple/test]
    Case(
        id="lang-token-here-doc-mode",
        rules=("token.here-document-mode", "redir.here-doc-multiple"),
        script=_s(
            r"""
            cat <<A; cat <<B
            p
            A
            q
            B
            { cat; cat <&3; } <<C 3<<D
            one
            C
            two
            D
            """
        ),
        stdout="p\nq\none\ntwo\n",
    ),
    # [spec:posix:syn:token.recognition-algorithm/test]
    # [spec:posix:syn:token.quoting-characters/test]
    # [spec:posix:syn:token.append-to-word/test]
    # [spec:posix:syn:token.start-new-word/test]
    # [spec:posix:syn:token.delimit-at-end-of-input/test]
    Case(
        id="lang-token-recognition",
        rules=(
            "token.recognition-algorithm",
            "token.quoting-characters",
            "token.append-to-word",
            "token.start-new-word",
            "token.delimit-at-end-of-input",
        ),
        script="printf '[%s]' a'b c'd    e \"\"\nprintf '[%s]' end",
        stdout="[ab cd][e][][end]",
    ),
    # [spec:posix:syn:token.operator-continue/test]
    Case(
        id="lang-token-operator-continue",
        rules=("token.operator-continue",),
        script=_s(
            r"""
            printf a > f
            printf b >> f
            cat f
            printf '\n'
            true && printf 'X'
            false || printf 'Y'
            printf '\n'
            """
        ),
        stdout="ab\nXY\n",
    ),
    # [spec:posix:syn:token.expansion-candidates/test]
    Case(
        id="lang-token-expansion-candidates",
        rules=("token.expansion-candidates",),
        script=_s(
            r"""
            printf '[%s]\n' x$(printf ab)y
            printf '[%s]\n' "$(printf '%s' "$(printf in)")"
            printf '[%s]\n' x`printf ab`y
            """
        ),
        stdout="[xaby]\n[in]\n[xaby]\n",
    ),
    # [spec:posix:req:token.incremental-execution/test]
    Case(
        id="lang-token-incremental-execution",
        rules=("token.incremental-execution",),
        script=_s(
            r"""
            cat > s.sh <<'EOS'
            printf 'ONE\n'
            if
            EOS
            sh s.sh 2>/dev/null
            printf 'nz=%s\n' "$(( $? != 0 ))"
            """
        ),
        stdout="ONE\nnz=1\n",
    ),
    # [spec:posix:req:token.alias-change-timing/test]
    Case(
        id="lang-token-alias-change-timing",
        rules=("token.alias-change-timing",),
        script=_s(
            r"""
            cat > s.sh <<'EOS'
            alias a='printf X'
            a
            alias a='printf Y'
            a
            printf '\n'
            EOS
            sh s.sh
            """
        ),
        stdout="XY\n",
    ),
    # [spec:posix:req:token.alias-not-inherited/test]
    Case(
        id="lang-token-alias-not-inherited",
        rules=("token.alias-not-inherited",),
        script=_s(
            r"""
            alias foo='printf A'
            sh -c foo 2>/dev/null
            printf 'st=%s\n' "$?"
            """
        ),
        stdout="st=127\n",
    ),
    # [spec:posix:def:token.reserved-words/test]
    # [spec:posix:def:grammar.reserved-word-tokens/test]
    Case(
        id="lang-token-reserved-words",
        rules=("token.reserved-words", "grammar.reserved-word-tokens"),
        script=_s(
            r"""
            if true; then printf 'a'; elif false; then printf 'b'; else printf 'c'; fi
            for i in x; do printf 'd'; done
            while false; do printf 'w'; done
            until true; do printf 'u'; done
            case x in x) printf 'e';; esac
            { printf 'f'; }
            ! false && printf 'g'
            printf '\n'
            """
        ),
        stdout="adefg\n",
    ),
    # [spec:posix:req:token.reserved-word-recognition-contexts/test]
    # [spec:posix:syn:grammar.command-name/test]
    # [spec:posix:syn:grammar.third-word-of-for-and-case/test]
    Case(
        id="lang-token-reserved-contexts",
        rules=(
            "token.reserved-word-recognition-contexts",
            "grammar.command-name",
            "grammar.third-word-of-for-and-case",
        ),
        script=_s(
            r"""
            printf '%s|' if then fi done esac
            printf '\n'
            for in in a; do printf '%s\n' "$in"; done
            case in in in) printf 'c\n';; esac
            set -- p q
            for x do printf '%s\n' "$x"; done
            ( "if" ) 2>/dev/null
            printf 'st=%s\n' "$?"
            """
        ),
        stdout="if|then|fi|done|esac|\na\nc\np\nq\nst=127\n",
    ),
    # -----------------------------------------------------------------
    # 2.10 Shell Grammar
    # -----------------------------------------------------------------
    # [spec:posix:syn:grammar.token-classification/test]
    # [spec:posix:syn:redir.format/test]
    Case(
        id="lang-grammar-token-classification",
        rules=("grammar.token-classification", "redir.format"),
        script=_s(
            r"""
            printf '%s' abc 1>f1
            printf '[%s]' "$(cat f1)"
            printf '%s' abc1>f2
            printf '[%s]' "$(cat f2)"
            printf '%s|%s' x 1 >f3
            printf '[%s]' "$(cat f3)"
            printf '\n'
            """
        ),
        stdout="[abc][abc1][x|1]\n",
    ),
    # [spec:posix:req:grammar.word-expansion-timing/test]
    Case(
        id="lang-grammar-word-expansion-timing",
        rules=("grammar.word-expansion-timing",),
        script=_s(
            r"""
            v=1
            while [ "$v" -lt 3 ]; do printf '%s' "$v"; v=$((v+1)); done
            printf '\n'
            """
        ),
        stdout="12\n",
    ),
    # [spec:posix:req:grammar.redirection-filename/test]
    # [spec:posix:req:redir.word-expansion/test]
    Case(
        id="lang-grammar-redirection-filename",
        rules=("grammar.redirection-filename", "redir.word-expansion"),
        script=_s(
            r"""
            HOME=$PWD
            d=out
            printf A > $d
            printf B > $(printf 'g')
            printf C > x$((1+1))
            printf D > ~/tilde
            printf '[%s][%s][%s][%s]\n' "$(cat out)" "$(cat g)" "$(cat x2)" "$(cat tilde)"
            """
        ),
        stdout="[A][B][C][D]\n",
    ),
    # [spec:posix:req:grammar.here-doc-redirection/test]
    Case(
        id="lang-grammar-here-doc-word",
        rules=("grammar.here-doc-redirection",),
        script=_s(
            r"""
            v=V
            cat <<"EO"F
            $v
            EOF
            """
        ),
        stdout="$v\n",
    ),
    # [spec:posix:syn:grammar.case-statement-termination/test]
    Case(
        id="lang-grammar-case-termination",
        rules=("grammar.case-statement-termination",),
        script=_s(
            r"""
            case x in esac
            printf 'A\n'
            case esac in (esac) printf 'B\n';; esac
            """
        ),
        stdout="A\nB\n",
    ),
    # [spec:posix:syn:grammar.for-name/test]
    Case(
        id="lang-grammar-for-name",
        rules=("grammar.for-name",),
        script=_s(
            r"""
            for x in 1 2; do printf '%s' "$x"; done
            printf '\n'
            sh -c 'for 1x in 1; do :; done' 2>/dev/null
            printf 'nz=%s\n' "$(( $? != 0 ))"
            """
        ),
        stdout="12\nnz=1\n",
    ),
    # [spec:posix:syn:grammar.assignment-first-word/test]
    # [spec:posix:syn:grammar.assignment-word-recognition/test]
    # [spec:posix:req:grammar.assignment-word-processing/test]
    Case(
        id="lang-grammar-assignment-recognition",
        rules=(
            "grammar.assignment-first-word",
            "grammar.assignment-word-recognition",
            "grammar.assignment-word-processing",
        ),
        script=_s(
            r"""
            a=1 b=2
            printf '%s%s\n' "$a" "$b"
            printf '%s\n' a=b
            a\=b 2>/dev/null
            printf '%s\n' "$?"
            =x 2>/dev/null
            printf '%s\n' "$?"
            unset v
            ${v=printf} '%s\n' ok
            """
        ),
        stdout="12\na=b\n127\n127\nok\n",
    ),
    # [spec:posix:req:grammar.highest-numbered-rule-applies/test]
    Case(
        id="lang-grammar-equals-sign-effect",
        rules=("grammar.highest-numbered-rule-applies",),
        script=_s(
            r"""
            case x=y in x=y) printf 'A\n';; esac
            printf '%s\n' q=r
            for i in m=n; do printf '%s\n' "$i"; done
            sh -c 'for x=y in 1; do :; done' 2>/dev/null
            printf 'nz=%s\n' "$(( $? != 0 ))"
            """
        ),
        stdout="A\nq=r\nm=n\nnz=1\n",
    ),
    # [spec:posix:syn:grammar.function-name/test]
    # [spec:posix:syn:grammar.function-definition/test]
    Case(
        id="lang-grammar-function-name",
        rules=("grammar.function-name", "grammar.function-definition"),
        script=_s(
            r"""
            f() { printf 'F'; } > out
            f
            printf '[%s]\n' "$(cat out)"
            sh -c 'if() { :; }' 2>/dev/null
            printf 'nz=%s\n' "$(( $? != 0 ))"
            """
        ),
        stdout="[F]\nnz=1\n",
    ),
    # [spec:posix:syn:grammar.program/test]
    # The production has two alternatives and complete_commands is
    # left-recursive over newline_list, so all three have to be reached:
    #
    #   program           : linebreak complete_commands linebreak
    #                     | linebreak
    #   complete_commands : complete_commands newline_list complete_command
    #                     | complete_command
    #
    # This case used to be `script="\n\n# only a comment\n\n"` asserting
    # `stdout=""`, which covers the empty alternative and nothing else --
    # and "produces no output and exits 0" is satisfied by a shell that
    # does not read its input at all. It was the single case in the suite
    # that passed against `/bin/true`.
    Case(
        id="lang-grammar-program",
        rules=("grammar.program",),
        script=(
            "\n"
            "\n"
            "# a comment in the leading linebreak\n"
            "\n"
            "printf one\n"
            "\n"
            "\n"
            "# a newline_list, then another complete_command\n"
            "printf two\n"
            "printf '\\n'\n"
            "sh -c '\n\n# a program that is only a linebreak\n\n'\n"
            "printf 'empty=%s\\n' \"$?\"\n"
            "\n"
            "# a trailing linebreak\n"
            "\n"
        ),
        stdout="onetwo\nempty=0\n",
    ),
    # [spec:posix:syn:grammar.list-and-or/test]
    # [spec:posix:syn:grammar.separators/test]
    Case(
        id="lang-grammar-list-and-or",
        rules=("grammar.list-and-or", "grammar.separators"),
        script=_s(
            r"""
            printf a; printf b
            true &&
            printf c
            false ||
            printf d
            printf '\n'
            printf e & wait
            printf '\n'
            """
        ),
        stdout="abcd\ne\n",
    ),
    # [spec:posix:syn:grammar.pipeline/test]
    Case(
        id="lang-grammar-pipeline",
        rules=("grammar.pipeline",),
        script=_s(
            r"""
            ! false
            printf '%s\n' "$?"
            printf 'x' |
            cat
            printf '\n'
            """
        ),
        stdout="0\nx\n",
    ),
    # [spec:posix:syn:grammar.command/test]
    # [spec:posix:syn:grammar.subshell-and-compound-list/test]
    # [spec:posix:syn:grammar.brace-group-and-do-group/test]
    Case(
        id="lang-grammar-command-forms",
        rules=(
            "grammar.command",
            "grammar.subshell-and-compound-list",
            "grammar.brace-group-and-do-group",
        ),
        script=_s(
            r"""
            { printf 'a'; printf 'b'
            } > out
            printf '[%s]' "$(cat out)"
            ( printf 'c'; printf 'd' )
            printf '\n'
            for i in 1; do printf 'e'; done
            printf '\n'
            """
        ),
        stdout="[ab]cd\ne\n",
    ),
    # [spec:posix:syn:grammar.for-clause/test]
    Case(
        id="lang-grammar-for-clause",
        rules=("grammar.for-clause",),
        script=_s(
            r"""
            set -- p q
            for a do printf '%s' "$a"; done
            printf '|'
            for b; do printf '%s' "$b"; done
            printf '|'
            for c in; do printf 'X'; done
            printf '|'
            for d in 1 2; do printf '%s' "$d"; done
            printf '\n'
            """
        ),
        stdout="pq|pq||12\n",
    ),
    # [spec:posix:syn:grammar.case-clause/test]
    Case(
        id="lang-grammar-case-clause",
        rules=("grammar.case-clause",),
        script=_s(
            r"""
            case abc in
            (a*) printf 'A';;
            esac
            case b in
            a|b) printf 'B';;
            esac
            case z in
            z) printf 'C'
            esac
            printf '\n'
            """
        ),
        stdout="ABC\n",
    ),
    # [spec:posix:syn:grammar.case-clause/test]
    Case(
        id="lang-grammar-case-pattern-reserved-words",
        rules=("grammar.case-clause",),
        script=_s(
            r"""
            case in in
            (in) printf 'paren\n';;
            esac
            case esac in
            x|esac) printf 'pipe\n';;
            esac
            """
        ),
        stdout="paren\npipe\n",
    ),
    # [spec:posix:syn:grammar.case-clause/test]
    Case(
        id="lang-grammar-case-first-pattern-rule-four",
        rules=("grammar.case-clause",),
        script="case x in esac) :;; esac\n",
        stdout="",
        status="nonzero",
        stderr_contains=("Syntax error",),
    ),
    # [spec:posix:syn:grammar.if-clause/test]
    Case(
        id="lang-grammar-if-clause",
        rules=("grammar.if-clause",),
        script=_s(
            r"""
            if false; then printf 'X'
            elif false; then printf 'Y'
            elif true; then printf 'A'
            else printf 'Z'
            fi
            if true; then printf 'B'; fi
            if false; then :; else printf 'C'; fi
            printf '\n'
            """
        ),
        stdout="ABC\n",
    ),
    # [spec:posix:syn:grammar.while-until-clause/test]
    Case(
        id="lang-grammar-while-until",
        rules=("grammar.while-until-clause",),
        script=_s(
            r"""
            i=0
            while [ "$i" -lt 2 ]; do printf 'w'; i=$((i+1)); done
            j=0
            until [ "$j" -ge 2 ]; do printf 'u'; j=$((j+1)); done
            printf '\n'
            """
        ),
        stdout="wwuu\n",
    ),
    # [spec:posix:syn:grammar.simple-command/test]
    Case(
        id="lang-grammar-simple-command",
        rules=("grammar.simple-command",),
        script=_s(
            r"""
            a=1 b=2 printf '%s|%s\n' x y
            > out printf '%s\n' z
            printf '[%s]\n' "$(cat out)"
            v=9
            printf '%s\n' "$v"
            """
        ),
        stdout="x|y\n[z]\n9\n",
    ),
    # [spec:posix:syn:grammar.io-redirect/test]
    # [spec:posix:syn:grammar.io-file/test]
    # [spec:posix:def:redir.purpose/test]
    # [spec:posix:syn:redir.output-format/test]
    # [spec:posix:syn:redir.append-format/test]
    # [spec:posix:req:redir.max-fd-number/test]
    # [spec:posix:def:grammar.operator-tokens/test]
    Case(
        id="lang-grammar-io-file",
        rules=(
            "grammar.io-redirect",
            "grammar.io-file",
            "redir.purpose",
            "redir.output-format",
            "redir.append-format",
            "redir.max-fd-number",
            "grammar.operator-tokens",
        ),
        script=_s(
            r"""
            printf A > f1
            printf B >> f1
            printf C >| f2
            printf D 2> f3 1>&2
            exec 9> f4
            printf E >&9
            exec 9>&-
            : <> f5
            exec 0< f1
            cat
            printf '[%s][%s][%s]' "$(cat f2)" "$(cat f3)" "$(cat f4)"
            printf '\n'
            true && false || true
            case x in x) :;; esac
            """
        ),
        stdout="AB[C][D][E]\n",
    ),
    # [spec:posix:syn:grammar.io-here/test]
    # [spec:posix:def:redir.here-doc/test]
    # [spec:posix:syn:redir.here-doc-format/test]
    Case(
        id="lang-grammar-io-here",
        rules=("grammar.io-here", "redir.here-doc", "redir.here-doc-format"),
        script=(
            "cat <<E1\n"
            "one\n"
            "E1\n"
            "cat <<-E2\n"
            + TAB + "two\n"
            + TAB + "E2\n"
            "sh -c 'read x <&3; printf \"%s\\n\" \"$x\"' 3<<E3\n"
            "three\n"
            "E3\n"
        ),
        stdout="one\ntwo\nthree\n",
    ),
    # -----------------------------------------------------------------
    # 2.7 Redirection
    # -----------------------------------------------------------------
    # [spec:posix:syn:redir.quoting-suppresses-recognition/test]
    Case(
        id="lang-redir-quoting-suppresses",
        rules=("redir.quoting-suppresses-recognition",),
        script=_s(
            r"""
            echo \2>a
            echo 2\>b
            printf '[%s]\n' "$(cat a)"
            """
        ),
        stdout="2>b\n[2]\n",
    ),
    # [spec:posix:req:redir.word-pathname-expansion/test]
    Case(
        id="lang-redir-word-pathname",
        rules=("redir.word-pathname-expansion",),
        script=_s(
            r"""
            : > g1
            printf x > g*
            if [ -e 'g*' ]; then printf 'literal\n'; else printf 'globbed\n'; fi
            """
        ),
        stdout="literal\n",
    ),
    # [spec:posix:req:redir.open-failure/test]
    Case(
        id="lang-redir-open-failure",
        rules=("redir.open-failure",),
        script=_s(
            r"""
            printf x 2> /dev/null > nodir/f
            printf 'nz=%s\n' "$(( $? != 0 ))"
            """
        ),
        stdout="nz=1\n",
    ),
    # [spec:posix:req:redir.here-doc-line-continuation/test]
    Case(
        id="lang-redir-here-doc-continuation",
        rules=("redir.here-doc-line-continuation",),
        script=_s(
            r"""
            cat <<EOF
            a\
            b
            EOF
            cat <<EOF
            x\
            EOF
            y
            EOF
            """
        ),
        stdout="ab\nxEOF\ny\n",
    ),
    # [spec:posix:req:redir.here-doc-backslash/test]
    Case(
        id="lang-redir-here-doc-backslash",
        rules=("redir.here-doc-backslash",),
        script=_s(
            r"""
            v=V
            cat <<EOF
            a\$b "c" \\d \q
            $v
            EOF
            """
        ),
        stdout='a$b "c" \\d \\q\nV\n',
    ),
    # [spec:posix:req:redir.here-doc-ps2/test]
    # [spec:posix:req:param.ps2/test]
    Case(
        id="lang-redir-here-doc-ps2",
        rules=("redir.here-doc-ps2", "param.ps2"),
        mode="interactive",
        requires=("UP",),
        script=_s(
            r"""
            cat <<EOF
            hi
            EOF
            exit
            """
        ),
        stdout=None,
        status="any",
        stdout_contains=("> ", "hi\n"),
    ),
    # -----------------------------------------------------------------
    # 2.14 Pattern Matching Notation
    # -----------------------------------------------------------------
    # [spec:posix:syn:pattern.single-character-patterns/test]
    # [spec:posix:def:pattern.ordinary-character/test]
    # [spec:posix:def:pattern.special-pattern-characters/test]
    # [spec:posix:sem:pattern.asterisk-matches-any-string/test]
    Case(
        id="lang-pattern-single-char",
        rules=(
            "pattern.single-character-patterns",
            "pattern.ordinary-character",
            "pattern.special-pattern-characters",
            "pattern.asterisk-matches-any-string",
        ),
        script=_s(
            r"""
            case abc in a?c) printf 'A';; esac
            case a in [ab]) printf 'B';; esac
            case abc in abc) printf 'C';; esac
            case '' in *) printf 'D';; esac
            case abc in a*c) printf 'E';; esac
            printf '\n'
            """
        ),
        stdout="ABCDE\n",
    ),
    # [spec:posix:syn:pattern.backslash-escape-with-shell-quoting/test]
    # [spec:posix:syn:pattern.backslash-escape-without-shell-quoting/test]
    # [spec:posix:req:pattern.escaping-follows-quoting-rules/test]
    # [spec:posix:req:pattern.quote-to-match-literally/test]
    Case(
        id="lang-pattern-escapes",
        rules=(
            "pattern.backslash-escape-with-shell-quoting",
            "pattern.backslash-escape-without-shell-quoting",
            "pattern.escaping-follows-quoting-rules",
            "pattern.quote-to-match-literally",
        ),
        script=_s(
            r"""
            case '*' in \*) printf 'A';; esac
            case '?' in "?") printf 'B';; esac
            case ']' in [\]]) printf 'C';; esac
            case '\' in \\) printf 'D';; esac
            p='\*'
            case '*' in $p) printf 'E';; esac
            case 'x' in $p) printf 'BAD';; esac
            printf '\n'
            """
        ),
        stdout="ABCDE\n",
    ),
    # [spec:posix:sem:pattern.left-bracket-literal/test]
    Case(
        id="lang-pattern-left-bracket-literal",
        rules=("pattern.left-bracket-literal",),
        script=_s(
            r"""
            case 'a[b' in a[b) printf 'A';; esac
            printf '\n'
            """
        ),
        stdout="A\n",
    ),
    # [spec:posix:req:pattern.match-by-bit-pattern/test]
    Case(
        id="lang-pattern-bit-pattern",
        rules=("pattern.match-by-bit-pattern",),
        script=_s(
            r"""
            s=$(printf 'a\200b')
            case $s in a?b) printf 'A';; esac
            case $s in *$(printf '\200')*) printf 'B';; esac
            case $s in a?c) printf 'BAD';; esac
            printf '\n'
            """
        ),
        stdout="AB\n",
    ),
    # [spec:posix:req:pattern.filename-expansion-trigger/test]
    # [spec:posix:req:pattern.no-special-chars-unchanged/test]
    # [spec:posix:req:pattern.slash-explicit-match/test]
    # [spec:posix:syn:pattern.slash-terminates-bracket/test]
    Case(
        id="lang-pattern-filename",
        rules=(
            "pattern.filename-expansion-trigger",
            "pattern.no-special-chars-unchanged",
            "pattern.slash-explicit-match",
            "pattern.slash-terminates-bracket",
        ),
        script=_s(
            r"""
            mkdir d
            : > d/f
            : > abd
            printf '%s\n' *
            printf '%s\n' */*
            printf '%s\n' d?f
            printf '%s\n' nosuchfile
            printf '%s\n' a[b/c]d
            """
        ),
        stdout="abd\nd\nd/f\nd?f\nnosuchfile\na[b/c]d\n",
    ),
    # [spec:posix:req:pattern.directory-permissions/test]
    # [spec:posix:req:pattern.permission-errors-not-fatal/test]
    Case(
        id="lang-pattern-permissions",
        rules=("pattern.directory-permissions", "pattern.permission-errors-not-fatal"),
        script=_s(
            r"""
            mkdir -p a/b
            : > a/b/x
            chmod 111 a/b
            printf '%s\n' a/b/*
            printf 'st=%s\n' "$?"
            printf '%s\n' a/b/x
            chmod 755 a/b
            printf '%s\n' a/b/*
            """
        ),
        stdout="a/b/*\nst=0\na/b/x\na/b/x\n",
    ),
    # -----------------------------------------------------------------
    # 2.6 Word Expansions
    # -----------------------------------------------------------------
    # [spec:posix:req:expand.order/test]
    Case(
        id="lang-expand-order",
        rules=("expand.order",),
        script=_s(
            r"""
            mkdir d
            : > d/f1
            : > d/f2
            v='d/f*'
            set -- $v
            printf '%s|%s|%s\n' "$#" "$1" "$2"
            set -f
            set -- $v
            printf '%s|%s\n' "$#" "$1"
            set +f
            printf '[%s]\n' 'a b'
            """
        ),
        stdout="2|d/f1|d/f2\n1|d/f*\n[a b]\n",
    ),
    # [spec:posix:req:expand.single-field/test]
    # [spec:posix:def:expand.field-splitting-results-of-expansion/test]
    # [spec:posix:req:expand.field-splitting-unexpanded-fields/test]
    Case(
        id="lang-expand-single-field",
        rules=(
            "expand.single-field",
            "expand.field-splitting-results-of-expansion",
            "expand.field-splitting-unexpanded-fields",
        ),
        script=_s(
            r"""
            HOME='/a b'
            set -- ~/x
            printf 'n=%s|%s\n' "$#" "$1"
            IFS=:
            set -- a:b $(printf 'c:d')
            printf 'n=%s|%s|%s|%s\n' "$#" "$1" "$2" "$3"
            """
        ),
        stdout="n=1|/a b/x\nn=3|a:b|c|d\n",
    ),
    # [spec:posix:req:expand.execution-environment/test]
    Case(
        id="lang-expand-execution-environment",
        rules=("expand.execution-environment",),
        script=_s(
            r"""
            F=x
            F=bar printf '[%s]\n' "$F"
            printf '[%s]\n' "$F"
            """
        ),
        stdout="[x]\n[x]\n",
    ),
    # [spec:posix:def:expand.dollar-introducer/test]
    Case(
        id="lang-expand-dollar-introducer",
        rules=("expand.dollar-introducer",),
        script=_s(
            r"""
            x=1
            printf '%s%s%s\n' $x $(printf c) $((1+1))
            """
        ),
        stdout="1c2\n",
    ),
    # [spec:posix:req:expand.dollar-literal/test]
    Case(
        id="lang-expand-dollar-literal",
        rules=("expand.dollar-literal",),
        script="printf '%s\\n' \"a $ b\"\nprintf '%s\\n' a$\nprintf '%s\\n' \"t$\tu\"\n",
        stdout="a $ b\na$\nt$\tu\n",
    ),
    # [spec:posix:def:expand.tilde-prefix/test]
    # [spec:posix:def:param.home/test]
    Case(
        id="lang-expand-tilde-prefix",
        rules=("expand.tilde-prefix", "param.home"),
        script=_s(
            r"""
            HOME=/h
            printf '%s\n' ~/x ~ a~ "~"
            """
        ),
        stdout="/h/x\n/h\na~\n~\n",
    ),
    # [spec:posix:req:expand.tilde-login-name/test]
    # [spec:posix:req:expand.tilde-replacement-pathname/test]
    Case(
        id="lang-expand-tilde-login-name",
        rules=("expand.tilde-login-name", "expand.tilde-replacement-pathname"),
        script=_s(
            r"""
            h=$(awk -F: '$1 == "root" { print $6; exit }' /etc/passwd)
            g=$(printf '%s' ~root)
            if [ "$g" = "$h" ]; then printf 'match\n'; else printf 'no|%s|%s\n' "$g" "$h"; fi
            """
        ),
        stdout="match\n",
    ),
    # [spec:posix:req:expand.param-simple/test]
    # [spec:posix:syn:expand.param-braces-optional/test]
    # [spec:posix:syn:expand.param-unbraced-resolution/test]
    Case(
        id="lang-expand-param-basics",
        rules=(
            "expand.param-simple",
            "expand.param-braces-optional",
            "expand.param-unbraced-resolution",
        ),
        script=_s(
            r"""
            v=abc
            printf '[%s]' "${v}" "${v}B"
            set -- 1 2 3 4 5 6 7 8 9 ten eleven
            printf '[%s]' "${11}" "$11"
            foo=A
            foobar=B
            printf '[%s]' "$foobar"
            unset foobar
            printf '[%s]' "$foobar"
            printf '\n'
            """
        ),
        stdout="[abc][abcB][eleven][11][B][]\n",
    ),
    # [spec:posix:req:expand.param-word-expansion/test]
    Case(
        id="lang-expand-param-word",
        rules=("expand.param-word-expansion",),
        script=_s(
            r"""
            HOME=/h
            unset u
            printf '[%s]' ${u:-~/w} "${u:-$(printf c)}" "${u:-$((2+3))}"
            v=set
            : ${v:-$(printf marker > marker)}
            if [ -e marker ]; then printf '[BAD]'; else printf '[ok]'; fi
            printf '\n'
            """
        ),
        stdout="[/h/w][c][5][ok]\n",
    ),
    # [spec:posix:def:expand.cmdsub-forms/test]
    # [spec:posix:req:expand.cmdsub-parsing/test]
    Case(
        id="lang-expand-cmdsub-forms",
        rules=("expand.cmdsub-forms", "expand.cmdsub-parsing"),
        script=_s(
            r"""
            printf '%s%s\n' $(printf a) `printf b`
            x=$(if true; then printf yes; fi)
            printf '%s\n' "$x"
            """
        ),
        stdout="ab\nyes\n",
    ),
    # [spec:posix:req:expand.cmdsub-backquote-backslash/test]
    Case(
        id="lang-expand-cmdsub-backquote-backslash",
        rules=("expand.cmdsub-backquote-backslash",),
        script=_s(
            r"""
            v=VAL
            printf '%s\n' `printf '%s' 'a\qb'`
            printf '%s\n' `printf '%s' 'a\\b'`
            printf '%s\n' `printf '%s' \$v`
            """
        ),
        stdout="a\\qb\na\\b\nVAL\n",
    ),
    # [spec:posix:req:expand.cmdsub-backquote-matching/test]
    Case(
        id="lang-expand-cmdsub-backquote-matching",
        rules=("expand.cmdsub-backquote-matching",),
        script=_s(
            r"""
            printf '%s\n' `printf x``printf y`
            printf '%s\n' `printf w\`printf z\``
            """
        ),
        stdout="xy\nwz\n",
    ),
    # [spec:posix:req:expand.cmdsub-arith-ambiguity/test]
    Case(
        id="lang-expand-cmdsub-arith-ambiguity",
        rules=("expand.cmdsub-arith-ambiguity",),
        script=_s(
            r"""
            printf '%s\n' $((1+1))
            printf '%s\n' $( (printf x) )
            printf '\n'
            sh -c 'printf %s $((1+1)' 2>/dev/null
            printf 'nz=%s\n' "$(( $? != 0 ))"
            """
        ),
        stdout="2\nx\n\nnz=1\n",
    ),
    # [spec:posix:syn:expand.arith-format/test]
    # [spec:posix:req:expand.arith-token-expansion/test]
    Case(
        id="lang-expand-arith",
        rules=("expand.arith-format", "expand.arith-token-expansion"),
        script=_s(
            r"""
            printf '%s\n' $((2+3))
            a=2
            printf '%s\n' $(( a + $(printf 3) ))
            printf '%s\n' "$((1+2))"
            """
        ),
        stdout="5\n5\n3\n",
    ),
    # [spec:posix:req:expand.arith-invalid-expression/test]
    Case(
        id="lang-expand-arith-invalid",
        rules=("expand.arith-invalid-expression",),
        script=_s(
            r"""
            sh -c 'printf %s $((1 +))' 2>err
            printf 'nz=%s' "$(( $? != 0 ))"
            if [ -s err ]; then printf ' diag\n'; else printf ' nodiag\n'; fi
            sh -c 'v=abc; printf %s $((v))' 2>err2
            printf 'nz=%s' "$(( $? != 0 ))"
            if [ -s err2 ]; then printf ' diag\n'; else printf ' nodiag\n'; fi
            """
        ),
        stdout="nz=1 diag\nnz=1 diag\n",
    ),
    # [spec:posix:req:expand.field-splitting-empty-ifs/test]
    Case(
        id="lang-expand-field-splitting-empty-ifs",
        rules=("expand.field-splitting-empty-ifs",),
        script=_s(
            r"""
            IFS=
            v='a b'
            unset u
            set -- $v $u x
            printf '%s|%s|%s\n' "$#" "$1" "$2"
            """
        ),
        stdout="2|a b|x\n",
    ),
    # [spec:posix:req:expand.field-splitting-order/test]
    # [spec:posix:def:expand.ifs-white-space/test]
    Case(
        id="lang-expand-field-splitting-order",
        rules=("expand.field-splitting-order", "expand.ifs-white-space"),
        script=_s(
            r"""
            IFS=:
            v='a:b'
            set -- $v c
            printf '%s|%s|%s|%s\n' "$#" "$1" "$2" "$3"
            IFS=' '
            w='  p  q  '
            set -- $w
            printf '%s|%s|%s\n' "$#" "$1" "$2"
            """
        ),
        stdout="3|a|b|c\n2|p|q\n",
    ),
    # [spec:posix:req:expand.field-splitting-output-replaces-input/test]
    Case(
        id="lang-expand-field-splitting-output",
        rules=("expand.field-splitting-output-replaces-input",),
        script=_s(
            r"""
            set -- a b c
            set -- $(printf '   ')
            printf 'n=%s\n' "$#"
            """
        ),
        stdout="n=0\n",
    ),
    # [spec:posix:sem:expand.field-splitting-arbitrary-bytes/test]
    Case(
        id="lang-expand-field-splitting-bytes",
        rules=("expand.field-splitting-arbitrary-bytes",),
        script=_s(
            r"""
            IFS=:
            v=$(printf 'a\200:b')
            set -- $v
            printf 'n=%s\n' "$#"
            printf '%s' "$1" | od -An -tx1 | tr -d ' \n'
            printf '\n'
            """
        ),
        stdout="n=2\n6180\n",
    ),
    # -----------------------------------------------------------------
    # 2.5 Parameters and Variables
    # -----------------------------------------------------------------
    # [spec:posix:def:param.denotation/test]
    # [spec:posix:def:param.positional-definition/test]
    # [spec:posix:def:param.special-parameters/test]
    Case(
        id="lang-param-denotation",
        rules=(
            "param.denotation",
            "param.positional-definition",
            "param.special-parameters",
        ),
        args=("lang0",),
        script=_s(
            r"""
            v=1
            set -- a b c d e f g h i j k
            printf '[%s]' "$v" "$1" "${10}" "$#" "$?" "$0"
            case $- in *[!a-z]*) printf '[BAD]';; *) printf '[opt]';; esac
            case $$ in ''|*[!0-9]*) printf '[BAD]';; *) printf '[pid]';; esac
            printf '[%s]' "$*"
            printf '\n'
            """
        ),
        stdout="[1][a][j][11][0][lang0][opt][pid][a b c d e f g h i j k]\n",
    ),
    # [spec:posix:def:param.set-state/test]
    Case(
        id="lang-param-set-state",
        rules=("param.set-state",),
        script=_s(
            r"""
            v=
            printf '[%s]' "${v+set}"
            unset v
            printf '[%s]' "${v+set}"
            w=x
            w=
            printf '[%s][%s]' "${w+set}" "$w"
            printf '\n'
            """
        ),
        stdout="[set][][set][]\n",
    ),
    # [spec:posix:req:param.byte-values/test]
    Case(
        id="lang-param-byte-values",
        rules=("param.byte-values",),
        script=_s(
            r"""
            v=$(printf 'a\200b')
            printf '%s' "$v" | od -An -tx1 | tr -d ' \n'
            printf '\n'
            """
        ),
        stdout="618062\n",
    ),
    # [spec:posix:sem:param.positional-assignment/test]
    Case(
        id="lang-param-positional-assignment",
        rules=("param.positional-assignment",),
        args=("lang0", "A", "B"),
        script=_s(
            r"""
            printf '[%s][%s][%s]' "$0" "$1" "$2"
            f() { printf '[%s][%s]' "$1" "$#"; }
            f Z
            printf '[%s]' "$1"
            set -- P Q
            printf '[%s][%s]' "$1" "$2"
            printf '\n'
            """
        ),
        stdout="[lang0][A][B][Z][1][A][P][Q]\n",
    ),
    # [spec:posix:req:param.special-asterisk/test]
    Case(
        id="lang-param-special-asterisk",
        rules=("param.special-asterisk",),
        script=_s(
            r"""
            set -- a b c
            IFS=:
            printf '%s\n' "$*"
            IFS=
            printf '%s\n' "$*"
            unset IFS
            printf '%s\n' "$*"
            IFS=:
            set -- x y
            set -- $*
            printf 'n=%s|%s|%s\n' "$#" "$1" "$2"
            """
        ),
        stdout="a:b:c\nabc\na b c\nn=2|x|y\n",
    ),
    # [spec:posix:req:param.special-question/test]
    Case(
        id="lang-param-special-question",
        rules=("param.special-question",),
        script=_s(
            r"""
            printf '%s\n' "$?"
            (exit 3)
            printf '%s\n' "$?"
            false | true
            printf '%s\n' "$?"
            (exit 7)
            ( printf '%s\n' "$?" )
            """
        ),
        stdout="0\n3\n0\n7\n",
    ),
    # [spec:posix:sem:param.special-question-assignment/test]
    Case(
        id="lang-param-question-assignment",
        rules=("param.special-question-assignment",),
        script=_s(
            r"""
            var=$(exit 5)
            printf '%s\n' "$?"
            """
        ),
        stdout="5\n",
    ),
    # [spec:posix:req:param.special-hyphen/test]
    Case(
        id="lang-param-special-hyphen",
        rules=("param.special-hyphen",),
        script=_s(
            r"""
            set -f
            case $- in *f*) printf 'f\n';; *) printf 'no\n';; esac
            set +f
            case $- in *f*) printf 'stillf\n';; *) printf 'nof\n';; esac
            """
        ),
        stdout="f\nnof\n",
    ),
    # [spec:posix:req:param.special-dollar/test]
    Case(
        id="lang-param-special-dollar",
        rules=("param.special-dollar",),
        script=_s(
            r"""
            p=$$
            ( if [ "$$" = "$p" ]; then printf 'same\n'; fi )
            case $$ in ''|*[!0-9]*) printf 'bad\n';; *) printf 'num\n';; esac
            kill -0 "$$" && printf 'alive\n'
            """
        ),
        stdout="same\nnum\nalive\n",
    ),
    # [spec:posix:req:param.special-bang/test]
    Case(
        id="lang-param-special-bang",
        rules=("param.special-bang",),
        script=_s(
            r"""
            sh -c 'printf "%s\n" "$$" > pidfile' &
            p=$!
            wait
            read v < pidfile
            if [ "$v" = "$p" ]; then printf 'match\n'; else printf 'no|%s|%s\n' "$v" "$p"; fi
            """
        ),
        stdout="match\n",
    ),
    # [spec:posix:req:param.variable-environment-initialization/test]
    Case(
        id="lang-param-env-init",
        rules=("param.variable-environment-initialization",),
        environment={"LANGFOO": "bar"},
        script=_s(
            r"""
            printf '%s\n' "$LANGFOO"
            LANGFOO=baz
            sh -c 'printf "%s\n" "$LANGFOO"'
            """
        ),
        stdout="bar\nbaz\n",
    ),
    # [spec:posix:sem:param.variable-creation/test]
    Case(
        id="lang-param-variable-creation",
        rules=("param.variable-creation",),
        script=_s(
            r"""
            a=1
            read b <<'E'
            BB
            E
            for c in 3; do :; done
            : ${d=4}
            printf '%s|%s|%s|%s\n' "$a" "$b" "$c" "$d"
            """
        ),
        stdout="1|BB|3|4\n",
    ),
    # [spec:posix:def:param.ifs/test]
    # [spec:posix:req:param.ifs-initial-value/test]
    Case(
        id="lang-param-ifs",
        rules=("param.ifs", "param.ifs-initial-value"),
        environment={"IFS": "XYZ"},
        script=_s(
            r"""
            printf '%s' "$IFS" | od -An -tx1 | tr -d ' \n'
            printf '\n'
            IFS=:
            set -- $(printf 'a:b')
            printf 'n=%s\n' "$#"
            read x y <<'E'
            p:q
            E
            printf '%s|%s\n' "$x" "$y"
            set -- m n
            printf '%s\n' "$*"
            """
        ),
        stdout="20090a\nn=2\np|q\nm:n\n",
    ),
    # [spec:posix:req:param.ppid/test]
    Case(
        id="lang-param-ppid",
        rules=("param.ppid",),
        script=_s(
            r"""
            case $PPID in ''|*[!0-9]*) printf 'bad\n';; *) printf 'num\n';; esac
            q=$PPID
            ( if [ "$PPID" = "$q" ]; then printf 'same\n'; fi )
            if [ "$PPID" != "$$" ]; then printf 'differs\n'; fi
            """
        ),
        stdout="num\nsame\ndiffers\n",
    ),
    # [spec:posix:req:param.pwd/test]
    Case(
        id="lang-param-pwd",
        rules=("param.pwd",),
        script=_s(
            r"""
            mkdir real
            ln -s real link
            cd link
            printf '%s\n' "$PWD"
            sh -c 'printf "%s\n" "$PWD"'
            """
        ),
        stdout="{ROOT}/link\n{ROOT}/link\n",
    ),
    # [spec:posix:req:param.env/test]
    Case(
        id="lang-param-env",
        rules=("param.env",),
        mode="interactive",
        requires=("UP",),
        files={"envrc": FileFixture("printf 'ENVFILE\\n'\n")},
        environment={"ENV": "{ROOT}/envrc"},
        script="printf 'DONE\\n'\nexit\n",
        stdout=None,
        status="any",
        stdout_contains=("ENVFILE\n", "DONE\n"),
    ),
    # [spec:posix:req:param.ps1/test]
    # [spec:posix:req:param.ps1-default/test]
    Case(
        id="lang-param-ps1",
        rules=("param.ps1", "param.ps1-default"),
        mode="interactive",
        requires=("UP",),
        script="printf 'A\\n'\nPS1='<$X>'\nX=Q\nprintf 'B\\n'\nexit\n",
        stdout=None,
        status="any",
        stdout_contains=("$ ", "<Q>"),
    ),
    # [spec:posix:req:param.ps1-exclamation-expansion/test]
    # [spec:posix:req:param.ps1-two-pass/test]
    Case(
        id="lang-param-ps1-exclamation",
        rules=("param.ps1-exclamation-expansion", "param.ps1-two-pass"),
        mode="interactive",
        requires=("UP",),
        environment={"PS1": "[!!]"},
        script="printf 'A\\n'\nexit\n",
        stdout=None,
        status="any",
        stdout_contains=("[!]",),
        stdout_excludes=("[!!]",),
    ),
    # -----------------------------------------------------------------
    # 2.11 Job Control / 2.12 Signals / 2.13 Shell Execution Environment
    # -----------------------------------------------------------------
    # [spec:posix:def:jobctl.definition/test]
    # [spec:posix:req:jobctl.job-creation/test]
    # [spec:posix:req:jobctl.list-splitting/test]
    Case(
        id="lang-jobctl-job-creation",
        rules=("jobctl.definition", "jobctl.job-creation", "jobctl.list-splitting"),
        script=_s(
            r"""
            set -m
            case $- in *m*) printf 'm\n';; *) printf 'nom\n';; esac
            sleep 30 & sleep 30 &
            jobs > jf
            wc -l < jf
            kill %1 %2 2>/dev/null
            wait 2>/dev/null
            """
        ),
        stdout="m\n2\n",
        timeout=15.0,
    ),
    # [spec:posix:def:jobctl.background-job/test]
    Case(
        id="lang-jobctl-background-job",
        rules=("jobctl.background-job",),
        script=_s(
            r"""
            set -m
            sleep 30 &
            p=$!
            g=$(ps -o pgid= -p "$p" | tr -d ' ')
            if [ "$g" = "$p" ]; then printf 'leader\n'; else printf 'no|%s|%s\n' "$g" "$p"; fi
            kill "$p" 2>/dev/null
            wait 2>/dev/null
            """
        ),
        stdout="leader\n",
        timeout=15.0,
    ),
    # [spec:posix:req:jobctl.pipeline-process-group/test]
    Case(
        id="lang-jobctl-pipeline-process-group",
        rules=("jobctl.pipeline-process-group",),
        script=_s(
            r"""
            set -m
            sh -c 'ps -o pgid= -p $$' | { sh -c 'ps -o pgid= -p $$'; cat; } |
            tr -d ' ' | sort -u | wc -l
            """
        ),
        stdout="1\n",
        timeout=15.0,
    ),
    # [spec:posix:req:signal.async-list-sigint-sigquit-ignored/test]
    Case(
        id="lang-signal-async-ignored",
        rules=("signal.async-list-sigint-sigquit-ignored",),
        script=_s(
            r"""
            sh -c 'kill -INT $$; kill -QUIT $$; printf "survived\n"' &
            wait
            """
        ),
        stdout="survived\n",
        timeout=15.0,
    ),
    # [spec:posix:req:signal.inherited-actions/test]
    Case(
        id="lang-signal-inherited-actions",
        rules=("signal.inherited-actions",),
        script=_s(
            r"""
            trap '' INT
            sh -c 'kill -INT $$; printf "ignored\n"'
            trap - INT
            sh -c 'kill -INT $$; printf "unreached\n"'
            printf 'st=%s\n' "$?"
            """
        ),
        stdout="ignored\nst=130\n",
        timeout=15.0,
    ),
    # [spec:posix:req:signal.trap-during-wait/test]
    Case(
        id="lang-signal-trap-during-wait",
        rules=("signal.trap-during-wait",),
        script=_s(
            r"""
            trap 'printf "trapped\n"' USR1
            sleep 30 &
            s=$!
            ( sleep 1; kill -USR1 $$ ) &
            wait "$s"
            printf 'gt128=%s\n' "$(( $? > 128 ))"
            kill "$s" 2>/dev/null
            """
        ),
        stdout="trapped\ngt128=1\n",
        timeout=20.0,
    ),
    # [spec:posix:req:shenv.utility-environment/test]
    Case(
        id="lang-shenv-utility-environment",
        rules=("shenv.utility-environment",),
        script=_s(
            r"""
            umask 077
            LANGE=v
            export LANGE
            trap 'printf "PARENT\n"' USR1
            cat > s.sh <<'EOS'
            printf '%s\n' "$LANGE"
            umask
            kill -USR1 $$
            printf 'unreached\n'
            EOS
            sh s.sh 2>/dev/null
            printf 'st=%s\n' "$?"
            """
        ),
        stdout="v\n0077\nst=138\n",
        timeout=15.0,
    ),
    # [spec:posix:req:shenv.subshell-creation/test]
    Case(
        id="lang-shenv-subshell-creation",
        rules=("shenv.subshell-creation",),
        script=_s(
            r"""
            v=1
            f() { printf 'F'; }
            set -f
            ( printf '%s' "$v"; f; case $- in *f*) printf 'g';; esac; v=9 )
            printf '|%s\n' "$v"
            trap 'printf "T\n"' EXIT
            ( : )
            printf 'end\n'
            """
        ),
        stdout="1Fg|1\nend\nT\n",
    ),
    # [spec:posix:req:shenv.subshell-contexts/test]
    Case(
        id="lang-shenv-subshell-contexts",
        rules=("shenv.subshell-contexts",),
        script=_s(
            r"""
            v=1
            x=$(v=2; printf '%s' "$v")
            printf '%s|%s\n' "$x" "$v"
            ( v=3 )
            printf '%s\n' "$v"
            v=4 & wait
            printf '%s\n' "$v"
            """
        ),
        stdout="2|1\n1\n1\n",
        timeout=15.0,
    ),
    # -----------------------------------------------------------------
    # 2.9 Shell Commands (remaining rules)
    # -----------------------------------------------------------------
    # [spec:posix:req:cmd.simple-declaration-utility-expansion/test]
    Case(
        id="lang-cmd-declaration-utility",
        rules=("cmd.simple-declaration-utility-expansion",),
        script=_s(
            r"""
            HOME=/h
            export V=~/a:~/b
            printf '%s\n' "$V"
            readonly R=~/c
            printf '%s\n' "$R"
            IFS=:
            export W=$(printf 'a:b')
            printf '%s\n' "$W"
            """
        ),
        stdout="/h/a:/h/b\n/h/c\na:b\n",
    ),
    # [spec:posix:req:cmd.no-name-redirections-subshell/test]
    Case(
        id="lang-cmd-no-name-redirections",
        rules=("cmd.no-name-redirections-subshell",),
        script=_s(
            r"""
            > f
            printf 'stdout\n'
            if [ -e f ]; then printf 'created\n'; fi
            v=1 > g
            printf '%s\n' "$v"
            """
        ),
        stdout="stdout\ncreated\n1\n",
    ),
    # [spec:posix:req:cmd.search-intrinsic-utility/test]
    Case(
        id="lang-cmd-intrinsic-utility",
        rules=("cmd.search-intrinsic-utility",),
        script=_s(
            r"""
            (PATH=; cd /)
            printf '%s\n' "$?"
            (PATH=; false)
            printf '%s\n' "$?"
            (PATH=; true)
            printf '%s\n' "$?"
            (PATH=; nosuchthingxyz) 2>/dev/null
            printf '%s\n' "$?"
            """
        ),
        stdout="0\n1\n0\n127\n",
    ),
    # [spec:posix:req:cmd.nonbuiltin-path-search-execl/test]
    # [spec:posix:def:param.path/test]
    Case(
        id="lang-cmd-nonbuiltin-execl",
        rules=("cmd.nonbuiltin-path-search-execl", "param.path"),
        files={
            "bin/langtool": FileFixture(
                "#!/bin/sh\nprintf '%s\\n' \"$0\"\n", 0o755
            )
        },
        script=_s(
            r"""
            PATH=$PWD/bin:$PATH
            langtool
            """
        ),
        stdout="{ROOT}/bin/langtool\n",
    ),
    # [spec:posix:req:cmd.pipeline-assignment-precedes-redirection/test]
    Case(
        id="lang-cmd-pipeline-redirection",
        rules=("cmd.pipeline-assignment-precedes-redirection",),
        script=_s(
            r"""
            printf 'p\n' | cat > out
            printf '[%s]\n' "$(cat out)"
            printf 'q\n' > in
            printf 'r\n' | cat < in
            """
        ),
        stdout="[p]\nq\n",
    ),
    # [spec:posix:req:cmd.case-clause-terminators/test]
    # [spec:posix:syn:grammar.case-clause/test]
    # [spec:posix:def:grammar.operator-tokens/test]
    Case(
        id="lang-cmd-case-semi-and",
        rules=(
            "cmd.case-clause-terminators",
            "grammar.case-clause",
            "grammar.operator-tokens",
        ),
        script=_s(
            r"""
            case a in
            a) printf '1';&
            never) printf '2';&
            still-never) printf '3';;
            c) printf '4';;
            esac
            printf '\n'
            """
        ),
        stdout="123\n",
    ),
    # -----------------------------------------------------------------
    # Rules formerly excused as `not-applicable` on the grounds that they
    # were "a heading", "an enumeration", "a %token list" or "a chapter
    # introduction". Each states an obligation on the shell, so each is
    # tested here instead.
    # -----------------------------------------------------------------
    # [spec:posix:def:shell.command-language-interpreter/test]
    Case(
        id="lang2-command-language-interpreter",
        rules=("shell.command-language-interpreter",),
        script=_s(
            r"""
            printf 'if true; then printf "[then]"; fi\n' > s.sh
            sh -c 'for w in a b; do printf "[%s]" "$w"; done'
            sh s.sh
            printf 'printf "[%%s]" "$((1+2))"\n' | sh
            printf '\n'
            """
        ),
        stdout="[a][b][then][3]\n",
    ),
    # The %token list is a claim about which terminal symbols the shell
    # recognizes: ASSIGNMENT_WORD, WORD, NAME, NEWLINE and IO_NUMBER are
    # each required. IO_LOCATION is not, because redir.location-format
    # only says the shell "may support" that format.
    # [spec:posix:def:grammar.token-symbols/test]
    Case(
        id="lang2-grammar-token-symbols",
        rules=("grammar.token-symbols",),
        script=_s(
            r"""
            name=VALUE
            printf '[%s]' "$name"
            for name in ITEM; do printf '[%s]' "$name"; done
            printf '[%s]' 2>err
            printf '[%s]' 2 >out
            printf '%s' "$(cat out)"
            printf '\n'
            """
        ),
        stdout="[VALUE][ITEM][][2]\n",
    ),
    # The same TOKEN `for` yields the reserved word For, then a NAME,
    # then a WORD, then an ASSIGNMENT_WORD, purely from context.
    # [spec:posix:syn:grammar.token-context-dependent-distinction/test]
    Case(
        id="lang2-token-context-dependent",
        rules=("grammar.token-context-dependent-distinction",),
        script=_s(
            r"""
            for for in for; do printf '[%s]' "$for"; done
            for=ASSIGNED
            printf '[%s]' "$for"
            printf '[%s]' for
            printf '\n'
            """
        ),
        stdout="[for][ASSIGNED][for]\n",
    ),
    # [spec:posix:def:expand.field-splitting-delimited/test]
    Case(
        id="lang2-field-splitting-delimited",
        rules=("expand.field-splitting-delimited",),
        script=_s(
            r"""
            IFS=:
            v='a::b'
            set -- $v
            printf '%s' "$#"
            printf '[%s]' "$1" "$2" "$3"
            unset IFS
            w='  p q  '
            set -- $w
            printf '%s' "$#"
            printf '[%s]' "$1" "$2"
            printf '\n'
            """
        ),
        stdout="3[a][][b]2[p][q]\n",
    ),
    # With word supplied, `${#-word}` and `${#+word}` are the parameter
    # expansion of `#`, not the string length of some other parameter.
    # [spec:posix:req:expand.param-hash-requires-word/test]
    Case(
        id="lang2-param-hash-requires-word",
        rules=("expand.param-hash-requires-word",),
        script=_s(
            r"""
            set -- alpha beta
            printf '[%s]' "${#-WORD}"
            printf '[%s]' "${#+WORD}"
            printf '[%s]' "${#}"
            set --
            printf '[%s]' "${#-WORD}"
            printf '\n'
            """
        ),
        stdout="[2][WORD][2][0]\n",
    ),
    # expand.cmdsub-parsing leaves the choice of parsing strategy
    # unspecified, so establish which one this shell made before
    # asserting the obligation that follows from it: a trailing syntax
    # error that suppresses an earlier command means the whole string
    # was parsed before anything in it ran.
    # [spec:posix:req:expand.cmdsub-alias-substitution/test]
    Case(
        id="lang2-cmdsub-alias-substitution",
        rules=("expand.cmdsub-alias-substitution",),
        script=_s(
            r"""
            rm -f marker
            ( eval 'x=$(: > marker; done)' ) 2>/dev/null
            alias outer='printf OUTER'
            v=$(unalias outer
            outer)
            { w=$(alias inner='printf INNER'
            inner); } 2>/dev/null
            if [ -f marker ]; then
                printf 'ok\n'
            elif [ "$v" = OUTER ] && [ -z "$w" ]; then
                printf 'ok\n'
            else
                printf 'alias-took-effect v=[%s] w=[%s]\n' "$v" "$w"
            fi
            """
        ),
        stdout="ok\n",
    ),
    # [spec:posix:req:cmd.assign-standard-utility-as-function/test]
    Case(
        id="lang2-assign-standard-utility-as-function",
        rules=("cmd.assign-standard-utility-as-function",),
        script=_s(
            r"""
            V=outer
            true() { printf 'in=[%s]' "$V"; }
            V=inner true
            printf 'after=[%s]' "$V"
            V=inner env > envout
            printf 'exported=[%s]' "$(grep -c '^V=inner$' envout)"
            printf 'after=[%s]\n' "$V"
            """
        ),
        stdout="in=[inner]after=[outer]exported=[1]after=[outer]\n",
    ),
    # A syntax error met while a function runs has the syntax-error
    # properties of a special built-in: the shell may abort, but if it
    # does not abort the exit status must still be non-zero.
    # [spec:posix:req:cmd.function-syntax-error-properties/test]
    Case(
        id="lang2-function-syntax-error-properties",
        rules=("cmd.function-syntax-error-properties",),
        script=_s(
            r"""
            f() { eval 'for'; printf 'CONTINUED'; }
            ( f; printf 'status=%s' "$?" ) > out 2>/dev/null
            outer=$?
            body=$(cat out)
            if [ -n "$body" ]; then
                case "$body" in
                *status=0*) printf 'zero-status-without-abort\n' ;;
                *) printf 'ok\n' ;;
                esac
            elif [ "$outer" -ne 0 ]; then
                printf 'ok\n'
            else
                printf 'zero-status-after-abort\n'
            fi
            """
        ),
        stdout="ok\n",
    ),
    # Step 1e: what runs is decided by the PATH search, and the earliest
    # directory that tests successfully is the one that supplies it.
    # [spec:posix:req:cmd.search-path-associated-builtin/test]
    Case(
        id="lang2-search-path-associated-builtin",
        rules=("cmd.search-path-associated-builtin",),
        files={
            "d1/langpath": FileFixture("#!/bin/sh\nprintf 'D1'\n", 0o755),
            "d2/langpath": FileFixture("#!/bin/sh\nprintf 'D2'\n", 0o755),
        },
        script=_s(
            r"""
            base=$PWD
            PATH=$base/d1:$base/d2
            langpath
            PATH=$base/d2:$base/d1
            langpath
            PATH=$base/d2
            langpath
            # echo, not printf: PATH here deliberately holds only d2, so
            # the probe has to be a builtin to survive it.
            echo
            """
        ),
        stdout="D1D2D2\n",
    ),
    # token.reserved-words-optional makes recognizing `time` optional.
    # Either way the observable is the same: `time utility` runs the
    # utility, passes its standard output through, and exits with its
    # status, writing any timing to standard error.
    # [spec:posix:req:token.reserved-word-time/test]
    Case(
        id="lang2-reserved-word-time",
        rules=("token.reserved-word-time",),
        files={
            "bin/time": FileFixture(
                '#!/bin/sh\n"$@"\nstatus=$?\nprintf \'real 0.00\\n\' >&2\n'
                'exit "$status"\n',
                0o755,
            )
        },
        script=_s(
            r"""
            PATH=$PWD/bin:$PATH
            time printf 'OUT\n'
            printf 'status=%s\n' "$?"
            time false
            printf 'status=%s\n' "$?"
            """
        ),
        stdout="OUT\nstatus=0\nstatus=1\n",
    ),
    # "The following variables shall affect the execution of the shell":
    # the seven below are observable without a terminal or a second
    # installed locale.
    # [spec:posix:def:param.shell-variables/test]
    Case(
        id="lang2-shell-variables",
        rules=("param.shell-variables",),
        files={"bin/langvar": FileFixture("#!/bin/sh\nprintf 'PATH-OK'\n", 0o755)},
        script=_s(
            r"""
            base=$PWD
            mkdir langhome
            HOME=$base/langhome
            cd
            printf 'HOME=[%s]' "${PWD##*/}"
            cd "$base"
            IFS=:
            v=x:y
            set -- $v
            printf 'IFS=[%s%s]' "$1" "$2"
            unset IFS
            PATH=$base/bin:$PATH
            printf 'PATH=[%s]' "$(langvar)"
            case $PPID in
            ''|*[!0-9]*) printf 'PPID=[bad]' ;;
            *) printf 'PPID=[ok]' ;;
            esac
            first=$LINENO
            second=$LINENO
            printf 'LINENO=[%s]' "$((second - first))"
            (PS4='@@ '; set -x; :) 2>ps4out
            printf 'PS4=[%s]' "$(grep -c '^@@ ' ps4out)"
            if [ "$PWD" = "$(pwd)" ]; then
                printf 'PWD=[same]\n'
            else
                printf 'PWD=[differs]\n'
            fi
            """
        ),
        stdout="HOME=[langhome]IFS=[xy]PATH=[PATH-OK]PPID=[ok]LINENO=[1]PS4=[1]PWD=[same]\n",
    ),
)
