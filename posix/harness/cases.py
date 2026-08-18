"""Executable cases derived from the wording in posix/docs/spec."""

from __future__ import annotations

from model import Case, FileFixture


CASES: tuple[Case, ...] = (
    # [spec:posix:req:quote.backslash-literal/test]
    # [spec:posix:req:quote.single-quotes/test]
    # [spec:posix:req:quote.double-quotes-literal/test]
    # [spec:posix:req:quote.double-quotes-dollar-sign/test]
    Case(
        id="quoting-basic",
        rules=(
            "quote.backslash-literal",
            "quote.single-quotes",
            "quote.double-quotes-literal",
            "quote.double-quotes-dollar-sign",
        ),
        script="v=world\nprintf '%s\\n' \"hello $v\" 'literal $v' a\\ b\n",
        stdout="hello world\nliteral $v\na b\n",
    ),
    # [spec:posix:req:quote.backslash-newline/test]
    # [spec:posix:req:token.input-lines/test]
    Case(
        id="backslash-newline",
        rules=("quote.backslash-newline", "token.input-lines"),
        script="printf '%s\\n' one\\\ntwo\n",
        stdout="onetwo\n",
    ),
    # [spec:posix:req:quote.dollar-single-quotes/test]
    # [spec:posix:def:quote.dollar-single-quotes-control-escape/test]
    # [spec:posix:def:quote.dollar-single-quotes-hex-escape/test]
    # [spec:posix:def:quote.dollar-single-quotes-octal-escape/test]
    Case(
        id="dollar-single-quotes",
        rules=(
            "quote.dollar-single-quotes",
            "quote.dollar-single-quotes-control-escape",
            "quote.dollar-single-quotes-hex-escape",
            "quote.dollar-single-quotes-octal-escape",
        ),
        script=(
            "printf '%s' $'A\\n\\x42\\103\\cD' "
            "| od -An -tx1 | tr -d ' \\n'\n"
        ),
        stdout="410a424304",
    ),
    # [spec:posix:syn:quote.dollar-single-quotes-escape-termination/test]
    # [spec:posix:req:quote.dollar-single-quotes-quote-escape-not-terminator/test]
    Case(
        id="dollar-single-quote-escape",
        rules=(
            "quote.dollar-single-quotes-escape-termination",
            "quote.dollar-single-quotes-quote-escape-not-terminator",
        ),
        script="printf '%s\\n' $'a\\'b'\n",
        stdout="a'b\n",
    ),
    # [spec:posix:syn:token.comment/test]
    # [spec:posix:syn:token.operator-delimit/test]
    # [spec:posix:syn:token.start-new-operator/test]
    # [spec:posix:syn:token.unquoted-blank-delimits/test]
    Case(
        id="token-comments-operators",
        rules=(
            "token.comment",
            "token.operator-delimit",
            "token.start-new-operator",
            "token.unquoted-blank-delimits",
        ),
        script=(
            "printf A # ignored\n"
            "printf B; printf C && printf D || printf X\n"
            "printf '\\n'\n"
        ),
        stdout="ABCD\n",
    ),
    # [spec:posix:req:token.alias-substitution-conditions/test]
    # [spec:posix:req:token.alias-replacement/test]
    Case(
        id="alias-simple",
        rules=("token.alias-substitution-conditions", "token.alias-replacement"),
        script="alias p='printf alias\\n'\np\n",
        stdout="aliasn",
    ),
    # [spec:posix:req:token.alias-trailing-blank-chaining/test]
    Case(
        id="alias-trailing-blank",
        rules=("token.alias-trailing-blank-chaining",),
        script="alias a='printf \"%s\\\\n\" '\nalias b='CHAIN'\na b\n",
        stdout="CHAIN\n",
    ),
    # [spec:posix:req:param.positional-decimal-digits/test]
    # [spec:posix:syn:param.positional-multi-digit-braces/test]
    # [spec:posix:req:param.special-hash/test]
    # [spec:posix:req:param.special-zero/test]
    Case(
        id="positional-parameters",
        rules=(
            "param.positional-decimal-digits",
            "param.positional-multi-digit-braces",
            "param.special-hash",
            "param.special-zero",
        ),
        script='printf \'%s|%s|%s|%s\\n\' "$#" "$1" "${10}" "$0"\n',
        stdout="10|a|j|audit-zero\n",
        args=("audit-zero", "a", "b", "c", "d", "e", "f", "g", "h", "i", "j"),
    ),
    # [spec:posix:req:param.special-at/test]
    # [spec:posix:req:param.special-at-double-quotes/test]
    Case(
        id="quoted-at",
        rules=("param.special-at", "param.special-at-double-quotes"),
        script="for x in \"$@\"; do printf '<%s>\\n' \"$x\"; done\n",
        stdout="<a b>\n<>\n<c>\n",
        args=("audit-zero", "a b", "", "c"),
    ),
    # [spec:posix:req:param.special-at-no-positional/test]
    Case(
        id="quoted-at-empty",
        rules=("param.special-at-no-positional",),
        script='set --\nn=0; for x in "$@"; do n=$((n+1)); done; echo $n\n',
        stdout="0\n",
    ),
    # [spec:posix:req:param.ifs-unset/test]
    # [spec:posix:req:expand.ifs-unset-default/test]
    # [spec:posix:req:expand.field-splitting-algorithm/test]
    Case(
        id="ifs-unset-default",
        rules=(
            "param.ifs-unset",
            "expand.ifs-unset-default",
            "expand.field-splitting-algorithm",
        ),
        script=(
            "unset IFS\n"
            "x=' a  b '; set -- $x; "
            "printf '%s|%s|%s\\n' \"$#\" \"$1\" \"$2\"\n"
        ),
        stdout="2|a|b\n",
    ),
    # [spec:posix:req:param.lineno/test]
    Case(
        id="lineno-script",
        rules=("param.lineno",),
        script=(
            "printf '%s\\n' \"$LINENO\"\n"
            "printf '%s\\n' \"$LINENO\"\n"
            "printf '%s\\n' \"$LINENO\"\n"
        ),
        stdout="1\n2\n3\n",
    ),
    # [spec:posix:req:param.ps4/test]
    Case(
        id="ps4-expansion",
        rules=("param.ps4",),
        script="PS4='trace:$x ' \nx=one\nset -x\n:\nx=two\n:\n",
        stdout="",
        stderr_contains=("trace:one :", "trace:two x=two", "trace:two :"),
    ),
    # [spec:posix:req:expand.tilde-home/test]
    # [spec:posix:def:expand.tilde-prefix-in-assignment/test]
    # [spec:posix:req:expand.tilde-result-quoted/test]
    Case(
        id="tilde-expansion",
        rules=(
            "expand.tilde-home",
            "expand.tilde-prefix-in-assignment",
            "expand.tilde-result-quoted",
        ),
        script="x=~; printf '%s|%s|%s\\n' ~ \"~\" \"$x\"\n",
        stdout="{HOME}|~|{HOME}\n",
    ),
    # [spec:posix:req:expand.param-use-default/test]
    # [spec:posix:req:expand.param-assign-default/test]
    # [spec:posix:req:expand.param-error-if-unset/test]
    # [spec:posix:req:expand.param-use-alternative/test]
    # [spec:posix:req:expand.param-colon-effect/test]
    Case(
        id="parameter-default-operators",
        rules=(
            "expand.param-use-default",
            "expand.param-assign-default",
            "expand.param-error-if-unset",
            "expand.param-use-alternative",
            "expand.param-colon-effect",
        ),
        script=(
            "unset a b c d; empty=\n"
            "printf '%s|%s|%s|%s|%s\\n' "
            "\"${a-word}\" \"${b:=set}\" \"${c:+alt}\" "
            "\"${b:+alt}\" \"${empty:-fallback}\"\n"
        ),
        stdout="word|set||alt|fallback\n",
    ),
    # [spec:posix:req:expand.param-string-length/test]
    # [spec:posix:req:expand.param-remove-smallest-suffix/test]
    # [spec:posix:req:expand.param-remove-largest-suffix/test]
    # [spec:posix:req:expand.param-remove-smallest-prefix/test]
    # [spec:posix:req:expand.param-remove-largest-prefix/test]
    Case(
        id="parameter-length-removal",
        rules=(
            "expand.param-string-length",
            "expand.param-remove-smallest-suffix",
            "expand.param-remove-largest-suffix",
            "expand.param-remove-smallest-prefix",
            "expand.param-remove-largest-prefix",
        ),
        script=(
            "x=abcabc\n"
            "printf '%s|%s|%s|%s|%s\\n' "
            "\"${#x}\" \"${x%c*}\" \"${x%%c*}\" "
            "\"${x#a*}\" \"${x##a*}\"\n"
        ),
        stdout="6|abcab|ab|bcabc|\n",
    ),
    # [spec:posix:req:expand.cmdsub-semantics/test]
    Case(
        id="command-substitution-newlines",
        rules=("expand.cmdsub-semantics",),
        script=(
            "x=$(printf 'a\\n\\nb\\n\\n'); printf '<%s>\\n' \"$x\"\n"
            "y=`printf 'c\\n\\nd\\n\\n'`; "
            "printf '<%s>\\n' \"$y\"\n"
        ),
        stdout="<a\n\nb>\n<c\n\nd>\n",
    ),
    # [spec:posix:req:expand.cmdsub-no-reexpansion/test]
    Case(
        id="command-substitution-no-reexpansion",
        rules=("expand.cmdsub-no-reexpansion",),
        script="x=$(printf '%s' '$HOME *'); printf '%s\\n' \"$x\"\n",
        stdout="$HOME *\n",
    ),
    # [spec:posix:req:expand.cmdsub-nesting/test]
    # [spec:posix:syn:expand.cmdsub-dollar-paren-extent/test]
    Case(
        id="command-substitution-nesting",
        rules=("expand.cmdsub-nesting", "expand.cmdsub-dollar-paren-extent"),
        script="printf '%s\\n' \"$(printf '<%s>' \"$(printf inner)\")\"\n",
        stdout="<inner>\n",
    ),
    # [spec:posix:req:expand.arith-evaluation/test]
    # [spec:posix:req:expand.arith-variable-changes/test]
    # [spec:posix:req:expand.arith-variable-reference/test]
    Case(
        id="arithmetic-evaluation",
        rules=(
            "expand.arith-evaluation",
            "expand.arith-variable-changes",
            "expand.arith-variable-reference",
        ),
        script=(
            "x=2; printf '%s|' $((x += 3)); "
            "printf '%s|%s\\n' \"$x\" $((x * 2))\n"
        ),
        stdout="5|5|10\n",
    ),
    # [spec:posix:req:builtin.set.opt-u-nounset/test]
    Case(
        id="nounset-arithmetic",
        rules=("builtin.set.opt-u-nounset",),
        script=(
            "set -u\n"
            "printf '%s\\n' $((undefined_name + 1))\n"
            "printf 'SURVIVED\\n'\n"
        ),
        stdout="",
        status="nonzero",
        stderr_contains=("undefined_name",),
    ),
    # [spec:posix:req:expand.field-splitting-applies/test]
    # [spec:posix:req:expand.ifs-delimiters/test]
    # [spec:posix:req:expand.field-splitting-zero-fields/test]
    Case(
        id="field-splitting-custom-ifs",
        rules=(
            "expand.field-splitting-applies",
            "expand.ifs-delimiters",
            "expand.field-splitting-zero-fields",
        ),
        script=(
            "IFS=:; x=':a::b:'; set -- $x; printf '%s' \"$#\"; "
            "for y; do printf '|<%s>' \"$y\"; done; printf '\\n'\n"
        ),
        stdout="4|<>|<a>|<>|<b>\n",
    ),
    # [spec:posix:req:expand.pathname/test]
    # [spec:posix:req:pattern.replacement-sorted/test]
    # [spec:posix:req:pattern.leading-period/test]
    # [spec:posix:req:pattern.no-match-unchanged/test]
    Case(
        id="pathname-expansion",
        rules=(
            "expand.pathname",
            "pattern.replacement-sorted",
            "pattern.leading-period",
            "pattern.no-match-unchanged",
        ),
        script="touch b a .hidden\nprintf '%s\\n' * nomatch-*\n",
        stdout="a\nb\nnomatch-*\n",
    ),
    # [spec:posix:req:expand.quote-removal/test]
    # [spec:posix:sem:expand.quote-removal-quoting-remembered/test]
    Case(
        id="quote-removal",
        rules=("expand.quote-removal", "expand.quote-removal-quoting-remembered"),
        script="printf '<%s>\\n' a\"b\" \"c\"'d'\n",
        stdout="<ab>\n<cd>\n",
    ),
    # [spec:posix:req:redir.output-truncate/test]
    # [spec:posix:req:redir.append/test]
    # [spec:posix:req:redir.input/test]
    Case(
        id="redirection-truncate-append-input",
        rules=("redir.output-truncate", "redir.append", "redir.input"),
        script=(
            "printf one >f; printf two >>f; "
            "x=$(cat <f); printf '%s\\n' \"$x\"\n"
        ),
        stdout="onetwo\n",
    ),
    # [spec:posix:sem:redir.evaluation-order/test]
    # [spec:posix:req:redir.dup-output/test]
    Case(
        id="redirection-order",
        rules=("redir.evaluation-order", "redir.dup-output"),
        script=(
            "{ { printf out; printf err >&2; } 2>&1 >f; } 2>/dev/null\n"
            "printf '|'; cat f; printf '\\n'\n"
        ),
        stdout="err|out\n",
    ),
    # [spec:posix:req:redir.output-noclobber/test]
    # [spec:posix:req:builtin.set.opt-c-noclobber/test]
    Case(
        id="noclobber",
        rules=("redir.output-noclobber", "builtin.set.opt-c-noclobber"),
        script=(
            "printf old >f; set -C; "
            "if printf new >f 2>/dev/null; then echo BAD; else cat f; fi\n"
        ),
        stdout="old",
    ),
    # [spec:posix:req:redir.here-doc-delimiter/test]
    # [spec:posix:req:redir.here-doc-quoted-delimiter/test]
    # [spec:posix:req:redir.here-doc-unquoted-delimiter/test]
    # [spec:posix:req:redir.here-doc-expansion/test]
    Case(
        id="here-doc-quoted-unquoted",
        rules=(
            "redir.here-doc-delimiter",
            "redir.here-doc-quoted-delimiter",
            "redir.here-doc-unquoted-delimiter",
            "redir.here-doc-expansion",
        ),
        script="x=VALUE\ncat <<EOF\n$x\nEOF\ncat <<'EOF'\n$x\nEOF\n",
        stdout="VALUE\n$x\n",
    ),
    # [spec:posix:req:redir.here-doc-tab-strip/test]
    Case(
        id="here-doc-tab-strip",
        rules=("redir.here-doc-tab-strip",),
        script="cat <<-EOF\n\tone\n\tEOF\n",
        stdout="one\n",
    ),
    # [spec:posix:req:redir.dup-input/test]
    # [spec:posix:req:redir.dup-input-close/test]
    # [spec:posix:req:redir.dup-output/test]
    # [spec:posix:req:redir.dup-output-close/test]
    Case(
        id="redirection-dup-close",
        rules=(
            "redir.dup-input",
            "redir.dup-input-close",
            "redir.dup-output",
            "redir.dup-output-close",
        ),
        script=(
            "exec 3>f; printf yes >&3; exec 3>&-; cat f; "
            "if printf no >&3 2>/dev/null; then echo BAD; else echo closed; fi\n"
        ),
        stdout="yesclosed\n",
    ),
    # [spec:posix:req:redir.open-read-write/test]
    Case(
        id="redirection-read-write",
        rules=("redir.open-read-write",),
        script=(
            "printf abc >f; exec 3<>f; read x <&3 || :; "
            "printf '%s\\n' \"$x\"\n"
        ),
        stdout="abc\n",
    ),
    # [spec:posix:req:cmd.no-name-exit-status/test]
    Case(
        id="no-command-name-status",
        rules=("cmd.no-name-exit-status",),
        script=(
            "x=$(exit 42); printf '%s\\n' \"$?\"\n"
            "x=$(exit 3) y=$(exit 7); printf '%s\\n' \"$?\"\n"
        ),
        stdout="42\n7\n",
    ),
    # [spec:posix:req:cmd.assign-exported-to-command/test]
    # [spec:posix:req:shenv.utility-does-not-change-shell-environment/test]
    Case(
        id="assignment-environment",
        rules=(
            "cmd.assign-exported-to-command",
            "shenv.utility-does-not-change-shell-environment",
        ),
        script=(
            "x=outer; x=inner sh -c 'printf \"%s\\n\" \"$x\"'; "
            "printf '%s\\n' \"$x\"\n"
        ),
        stdout="inner\nouter\n",
    ),
    # [spec:posix:req:cmd.search-function/test]
    # [spec:posix:req:cmd.assign-function/test]
    Case(
        id="function-search-and-environment",
        rules=("cmd.search-function", "cmd.assign-function"),
        script=(
            "f(){ printf '%s\\n' \"$x\"; }; "
            "x=outer; x=inner f; printf '%s\\n' \"$x\"\n"
        ),
        stdout="inner\nouter\n",
    ),
    # [spec:posix:req:cmd.pipeline-exit-status/test]
    # [spec:posix:req:cmd.pipeline-bang-subshell-separation/test]
    Case(
        id="pipeline-status-and-bang",
        rules=("cmd.pipeline-exit-status", "cmd.pipeline-bang-subshell-separation"),
        script="true | false; echo $?\n! true | false; echo $?\n",
        stdout="1\n0\n",
    ),
    # [spec:posix:sem:builtin.set.opt-o-pipefail/test]
    # [spec:posix:req:cmd.pipeline-exit-status/test]
    Case(
        id="pipeline-pipefail",
        rules=("builtin.set.opt-o-pipefail", "cmd.pipeline-exit-status"),
        script=(
            "set -o pipefail\n"
            "false | true; echo $?\n"
            "true | false | true; echo $?\n"
        ),
        stdout="1\n1\n",
    ),
    # [spec:posix:req:cmd.pipeline-pipefail-setting-at-start/test]
    Case(
        id="pipefail-setting-at-start",
        rules=("cmd.pipeline-pipefail-setting-at-start",),
        script=(
            "set +o pipefail\n"
            "false | set -o pipefail; echo $?\n"
            "set -o pipefail\n"
            "false | set +o pipefail; echo $?\n"
        ),
        stdout="0\n1\n",
    ),
    # [spec:posix:req:cmd.and-or-precedence/test]
    # [spec:posix:req:cmd.and-list-execution/test]
    # [spec:posix:req:cmd.or-list-execution/test]
    Case(
        id="and-or-precedence",
        rules=(
            "cmd.and-or-precedence",
            "cmd.and-list-execution",
            "cmd.or-list-execution",
        ),
        script=(
            "false && echo BAD || echo fallback\n"
            "true || echo BAD && echo final\n"
        ),
        stdout="fallback\nfinal\n",
    ),
    # [spec:posix:req:cmd.async-process-id-known/test]
    # [spec:posix:req:cmd.async-exit-status/test]
    # [spec:posix:sem:cmd.async-status-via-wait/test]
    Case(
        id="asynchronous-list",
        rules=(
            "cmd.async-process-id-known",
            "cmd.async-exit-status",
            "cmd.async-status-via-wait",
        ),
        script=(
            "(exit 7) & p=$!; test -n \"$p\" || exit 90; "
            "wait \"$p\"; s=$?; printf '%s\\n' \"$s\"\n"
        ),
        stdout="7\n",
    ),
    # [spec:posix:req:cmd.group-subshell/test]
    # [spec:posix:sem:cmd.group-brace-current-environment/test]
    # [spec:posix:req:shenv.subshell-isolation/test]
    Case(
        id="subshell-and-brace-environments",
        rules=(
            "cmd.group-subshell",
            "cmd.group-brace-current-environment",
            "shenv.subshell-isolation",
        ),
        script=(
            "x=outer; (x=sub); printf '%s|' \"$x\"; "
            "{ x=brace; }; printf '%s\\n' \"$x\"\n"
        ),
        stdout="outer|brace\n",
    ),
    # [spec:posix:req:cmd.for-iteration/test]
    # [spec:posix:req:cmd.while-execution/test]
    # [spec:posix:req:cmd.until-execution/test]
    # [spec:posix:req:cmd.case-selection/test]
    Case(
        id="loop-and-case-semantics",
        rules=(
            "cmd.for-iteration",
            "cmd.while-execution",
            "cmd.until-execution",
            "cmd.case-selection",
        ),
        script=(
            "for x in a b; do printf %s \"$x\"; done\n"
            "i=0; while test $i -lt 2; do i=$((i+1)); done\n"
            "until test $i -eq 0; do i=$((i-1)); done\n"
            "case ab in a*) printf '|case:%s\\n' \"$i\";; *) echo BAD;; esac\n"
        ),
        stdout="ab|case:0\n",
    ),
    # [spec:posix:req:cmd.function-invocation-positional-parameters/test]
    # [spec:posix:req:cmd.function-return/test]
    Case(
        id="function-positional-restore",
        rules=(
            "cmd.function-invocation-positional-parameters",
            "cmd.function-return",
        ),
        script=(
            "set -- outer\n"
            "f(){ printf '%s|' \"$1\"; return 7; }; "
            "f inner; s=$?; printf '%s|%s\\n' \"$s\" \"$1\"\n"
        ),
        stdout="inner|7|outer\n",
    ),
    # [spec:posix:req:cmd.function-no-expansion-at-definition/test]
    # [spec:posix:req:grammar.function-body-no-expansion/test]
    Case(
        id="function-body-not-expanded-at-definition",
        rules=(
            "cmd.function-no-expansion-at-definition",
            "grammar.function-body-no-expansion",
        ),
        script='x=old; f(){ printf \'%s\\n\' "$x"; }; x=new; f\n',
        stdout="new\n",
    ),
    # [spec:posix:req:builtin.special.preceding-assignments-persist/test]
    # [spec:posix:req:cmd.assign-special-builtin/test]
    Case(
        id="special-builtin-assignment-persists",
        rules=(
            "builtin.special.preceding-assignments-persist",
            "cmd.assign-special-builtin",
        ),
        script="unset x; x=value export x; printf '%s\\n' \"$x\"\n",
        stdout="value\n",
    ),
    # [spec:posix:req:builtin.break.exit-nth-loop/test]
    # [spec:posix:req:builtin.continue.return-to-top/test]
    Case(
        id="break-continue",
        rules=(
            "builtin.break.exit-nth-loop",
            "builtin.continue.return-to-top",
        ),
        script=(
            "for x in a b; do for y in c d; do "
            "printf %s \"$x$y\"; continue 2; done; done; echo\n"
            "for x in a b; do for y in c d; do "
            "echo break; break 2; done; done\n"
        ),
        stdout="acbc\nbreak\n",
    ),
    # [spec:posix:req:builtin.dot.execute-in-current-environment/test]
    # [spec:posix:req:builtin.dot.exit-status/test]
    Case(
        id="dot-current-environment",
        rules=(
            "builtin.dot.execute-in-current-environment",
            "builtin.dot.exit-status",
        ),
        script='. ./dotfile; printf \'%s|%s\\n\' "$dot_value" "$?"\n',
        stdout="sourced|0\n",
        files={"dotfile": FileFixture("dot_value=sourced\ntrue\n", 0o444)},
    ),
    # [spec:posix:req:builtin.dot.path-search/test]
    Case(
        id="dot-path-skips-unreadable",
        rules=("builtin.dot.path-search",),
        script=(
            "mkdir p1 p2\n"
            "printf 'echo WRONG\\n' >p1/item\n"
            "printf 'echo RIGHT\\n' >p2/item\n"
            "chmod 333 p1/item\n"
            "chmod 444 p2/item\n"
            "PATH=$PWD/p1:$PWD/p2:/usr/bin:/bin\n"
            ". item\n"
        ),
        stdout="RIGHT\n",
    ),
    # [spec:posix:req:builtin.eval.construct-and-execute/test]
    # [spec:posix:req:builtin.eval.exit-status/test]
    Case(
        id="eval",
        rules=("builtin.eval.construct-and-execute", "builtin.eval.exit-status"),
        script="x='printf \"%s\\n\" evaluated'; eval \"$x\"\neval 'exit 9'\n",
        stdout="evaluated\n",
        status=9,
    ),
    # [spec:posix:req:builtin.exec.utility-operand/test]
    # [spec:posix:req:builtin.exec.exit-status/test]
    Case(
        id="exec-utility",
        rules=("builtin.exec.utility-operand", "builtin.exec.exit-status"),
        script="exec printf '%s\\n' replaced\nprintf BAD\n",
        stdout="replaced\n",
    ),
    # [spec:posix:req:builtin.exit.cause-shell-exit/test]
    # [spec:posix:req:builtin.exit.wait-status-from-n/test]
    # [spec:posix:req:builtin.exit.exit-trap/test]
    # [spec:posix:req:builtin.trap.exit-condition/test]
    Case(
        id="exit-trap-and-status",
        rules=(
            "builtin.exit.cause-shell-exit",
            "builtin.exit.wait-status-from-n",
            "builtin.exit.exit-trap",
            "builtin.trap.exit-condition",
        ),
        script="trap 'echo trapped' EXIT\nexit 23\n",
        stdout="trapped\n",
        status=23,
    ),
    # [spec:posix:req:builtin.export.p-output-format/test]
    # [spec:posix:req:builtin.export.p-output-reinput/test]
    Case(
        id="export-print-reinput",
        rules=(
            "builtin.export.p-output-format",
            "builtin.export.p-output-reinput",
        ),
        script=(
            "export x='a b'\n"
            "export -p >exports\n"
            "unset x\n"
            ". ./exports\n"
            "printf '%s\\n' \"$x\"\n"
        ),
        stdout="a b\n",
    ),
    # [spec:posix:req:builtin.readonly.set-attribute/test]
    # [spec:posix:req:builtin.readonly.p-output-reinput/test]
    Case(
        id="readonly",
        rules=(
            "builtin.readonly.set-attribute",
            "builtin.readonly.p-output-reinput",
        ),
        script=(
            "readonly x='a b'\n"
            "readonly -p | grep \"readonly x='a b'\" >/dev/null || exit 8\n"
            "if x=no 2>/dev/null; then exit 9; fi\n"
        ),
        stdout="",
        status="nonzero",
    ),
    # [spec:posix:req:builtin.return.stop-function-or-dot-script/test]
    # [spec:posix:req:builtin.return.exit-status/test]
    Case(
        id="return-status",
        rules=(
            "builtin.return.stop-function-or-dot-script",
            "builtin.return.exit-status",
        ),
        script="f(){ return 37; echo BAD; }; f; printf '%s\\n' \"$?\"\n",
        stdout="37\n",
    ),
    # [spec:posix:req:builtin.shift.positional-parameters/test]
    # [spec:posix:req:builtin.shift.operand-value/test]
    # [spec:posix:req:builtin.shift.exit-status/test]
    Case(
        id="shift",
        rules=(
            "builtin.shift.positional-parameters",
            "builtin.shift.operand-value",
            "builtin.shift.exit-status",
        ),
        script=(
            "set -- a b c; shift 2; printf '%s|%s\\n' \"$#\" \"$1\"; "
            "shift 9 2>/dev/null; printf '%s\\n' \"$?\"\n"
        ),
        stdout="1|c\n",
        status=2,
    ),
    # [spec:posix:req:builtin.unset.v-option/test]
    # [spec:posix:req:builtin.unset.f-option/test]
    # [spec:posix:req:builtin.unset.not-previously-set/test]
    Case(
        id="unset-options",
        rules=(
            "builtin.unset.v-option",
            "builtin.unset.f-option",
            "builtin.unset.not-previously-set",
        ),
        script=(
            "x=1; f(){ :; }; unset -v x; unset -f f; unset -v never_set; "
            "printf '%s|' \"${x-unset}\"; "
            "command -V f >/dev/null 2>&1; echo $?\n"
        ),
        stdout="unset|127\n",
    ),
    # [spec:posix:req:builtin.set.positional-parameters/test]
    # [spec:posix:req:builtin.set.double-hyphen/test]
    Case(
        id="set-positional",
        rules=(
            "builtin.set.positional-parameters",
            "builtin.set.double-hyphen",
        ),
        script="set -- -x 'a b'; printf '%s|%s|%s\\n' \"$#\" \"$1\" \"$2\"\n",
        stdout="2|-x|a b\n",
    ),
    # [spec:posix:req:builtin.set.options-both-forms/test]
    # [spec:posix:req:builtin.set.opt-h/test]
    Case(
        id="set-obsolescent-h",
        rules=("builtin.set.options-both-forms", "builtin.set.opt-h"),
        script=(
            "set -h\n"
            "case $- in *h*) : ;; *) exit 10 ;; esac\n"
            "set +h\n"
            "case $- in *h*) exit 11 ;; *) : ;; esac\n"
            "sh -h -c 'case $- in *h*) : ;; *) exit 12 ;; esac' || exit $?\n"
            "sh +h -c 'case $- in *h*) exit 13 ;; *) : ;; esac' || exit $?\n"
            "printf 'accepted\\n'\n"
        ),
        stdout="accepted\n",
    ),
    # [spec:posix:req:builtin.trap.action-overrides-and-exit-status/test]
    # [spec:posix:req:builtin.trap.action-executed-as-eval/test]
    Case(
        id="trap-preserves-status",
        rules=(
            "builtin.trap.action-overrides-and-exit-status",
            "builtin.trap.action-executed-as-eval",
        ),
        script="trap 'printf \"trap:%s\\n\" \"$?\"; false' EXIT\nfalse\n",
        stdout="trap:1\n",
        status=1,
    ),
    # [spec:posix:req:builtin.trap.list-condition-set/test]
    # [spec:posix:syn:builtin.trap.list-format/test]
    # [spec:posix:req:builtin.trap.list-suitable-for-reinput/test]
    Case(
        id="trap-list-reinput",
        rules=(
            "builtin.trap.list-condition-set",
            "builtin.trap.list-format",
            "builtin.trap.list-suitable-for-reinput",
        ),
        script=(
            "trap 'echo hello world' USR1\n"
            "trap >saved\n"
            "trap - USR1\n"
            ". ./saved\n"
            "kill -USR1 $$\n"
        ),
        stdout="hello world\n",
    ),
    # [spec:posix:req:builtin.trap.opt-p/test]
    # [spec:posix:req:builtin.trap.opt-p-suitable-for-reinput/test]
    Case(
        id="trap-p-option",
        rules=(
            "builtin.trap.opt-p",
            "builtin.trap.opt-p-suitable-for-reinput",
        ),
        script="trap 'echo hello' USR1\ntrap -p USR1\n",
        stdout=None,
        stdout_contains=("USR1", "echo hello"),
    ),
    # [spec:posix:req:builtin.trap.list-in-subshell/test]
    Case(
        id="trap-list-in-subshell",
        rules=("builtin.trap.list-in-subshell",),
        script=(
            "trap 'echo bye' EXIT\n"
            "((trap); trap; trap 'echo current' USR1; trap)\n"
        ),
        stdout=(
            "trap -- 'echo bye' EXIT\n"
            "trap -- 'echo bye' EXIT\n"
            "trap -- 'echo current' USR1\n"
            "bye\n"
        ),
    ),
    # [spec:posix:req:builtin.trap.action-executed-as-eval/test]
    Case(
        id="trap-subshell-command-status",
        rules=("builtin.trap.action-executed-as-eval",),
        script="trap '(false); echo $?' EXIT\n",
        stdout="1\n",
    ),
    # [spec:posix:req:sh.option-c/test]
    # [spec:posix:req:sh.operand-command-name/test]
    # [spec:posix:req:sh.operand-command-string/test]
    # [spec:posix:req:sh.special-parameter-0/test]
    Case(
        id="invocation-c-operands",
        rules=(
            "sh.option-c",
            "sh.operand-command-name",
            "sh.operand-command-string",
            "sh.special-parameter-0",
        ),
        script='printf \'%s|%s|%s\\n\' "$0" "$1" "$2"\n',
        stdout="name|one|two\n",
        args=("name", "one", "two"),
    ),
    # [spec:posix:req:exit.status-command-not-found/test]
    Case(
        id="command-not-found-status",
        rules=("exit.status-command-not-found",),
        script="definitely_no_such_command_8472\n",
        stdout="",
        status=127,
        stderr_contains=("definitely_no_such_command_8472",),
    ),
    # [spec:posix:req:exit.status-not-executable/test]
    # [spec:posix:req:cmd.nonbuiltin-slash-not-found/test]
    Case(
        id="not-executable-status",
        rules=("exit.status-not-executable", "cmd.nonbuiltin-slash-not-found"),
        script="./not-executable\n",
        stdout="",
        status=126,
        stderr_contains=("not-executable",),
        files={
            "not-executable": FileFixture(
                "#!/bin/sh\necho BAD\n",
                0o644,
            )
        },
    ),
    # [spec:posix:sem:pattern.question-mark/test]
    # [spec:posix:sem:pattern.asterisk/test]
    # [spec:posix:syn:pattern.concatenation/test]
    # [spec:posix:sem:pattern.asterisk-longest-match/test]
    Case(
        id="pattern-case",
        rules=(
            "pattern.question-mark",
            "pattern.asterisk",
            "pattern.concatenation",
            "pattern.asterisk-longest-match",
        ),
        script=(
            "case abc123 in a?c*) echo match;; *) echo BAD;; esac\n"
            "x=abcabc; echo \"${x%%a*c}\"\n"
        ),
        stdout="match\n\n",
    ),
    # [spec:posix:syn:pattern.bracket-expression/test]
    Case(
        id="pattern-hyphen-collating",
        rules=("pattern.bracket-expression",),
        script=(
            "touch file- filea\n"
            "printf '%s\\n' file[-123] file[123-] "
            "file[[.-.]] file[[=-=]] file[[:alpha:]]\n"
        ),
        stdout="file-\nfile-\nfile-\nfile-\nfilea\n",
    ),
    # [spec:posix:syn:pattern.bracket-expression/test]
    Case(
        id="pattern-right-bracket-collating",
        rules=("pattern.bracket-expression",),
        script=(
            "touch 'file]' filea\n"
            "printf '%s\\n' file[]123] file[[.].]] "
            "file[[=]=]] file[[:alpha:]]\n"
        ),
        stdout="file]\nfile]\nfile]\nfilea\n",
    ),
    # [spec:posix:req:signal.trap-deferred-until-foreground-command-completes/test]
    Case(
        id="signal-trap-deferred",
        rules=("signal.trap-deferred-until-foreground-command-completes",),
        script=(
            "trap 'echo trapped' USR1\n"
            "(sleep 0.05; kill -USR1 $$) &\n"
            "sleep 0.15\n"
            "echo foreground-done\n"
            "wait\n"
        ),
        stdout="trapped\nforeground-done\n",
    ),

    # ---------------------------------------------------------------
    # XCU 2.9 Shell Commands. Expectations below state what POSIX
    # REQUIRES, not what dash does -- a failure here is a conformance
    # finding, not a broken test.
    # ---------------------------------------------------------------
    # [spec:posix:req:cmd.default-exit-status/test]
    # [spec:posix:req:cmd.simple-processing-order/test]
    Case(
        id="cmd-default-exit-status",
        rules=("cmd.default-exit-status", "cmd.simple-processing-order"),
        script="{ true; false; }\necho $?\n",
        stdout="1\n",
    ),
    # [spec:posix:req:cmd.simple-command-name-determination/test]
    # [spec:posix:req:cmd.simple-argument-expansion/test]
    Case(
        id="cmd-name-determination",
        rules=(
            "cmd.simple-command-name-determination",
            "cmd.simple-argument-expansion",
        ),
        script="c=printf\nf='%s|%s\\n'\nargs='a b'\n$c \"$f\" $args\n",
        stdout="a|b\n",
    ),
    # [spec:posix:req:cmd.simple-assignment-expansion/test]
    Case(
        id="cmd-assignment-expansion",
        rules=("cmd.simple-assignment-expansion",),
        script="HOME=/hm\nv=x\na=~/$v$(echo s)$((1+1))\nprintf '%s\\n' \"$a\"\n",
        stdout="/hm/xs2\n",
    ),
    # [spec:posix:req:cmd.assign-no-command-name/test]
    Case(
        id="cmd-assign-no-name-persists",
        rules=("cmd.assign-no-command-name",),
        script="v=set-in-current\nprintf '%s\\n' \"$v\"\n",
        stdout="set-in-current\n",
    ),
    # [spec:posix:req:cmd.assign-readonly-error/test]
    Case(
        id="cmd-assign-readonly-error",
        rules=("cmd.assign-readonly-error",),
        script="readonly r=1\nr=2 true\necho reached=$?\n",
        status="nonzero",
        stdout="",
    ),
    # [spec:posix:req:cmd.no-name-redirection-failure/test]
    Case(
        id="cmd-no-name-redirect-failure",
        rules=("cmd.no-name-redirection-failure",),
        # The rule requires the COMMAND to fail with a status greater than
        # zero and an error message to be written. It does not require the
        # shell to exit, so assert on $? rather than on the shell status.
        script=(
            "if < /nonexistent/path; then echo zero; else echo nonzero; fi\n"
        ),
        stdout="nonzero\n",
        stderr_contains=("/nonexistent/path",),
    ),
    # [spec:posix:req:cmd.simple-redirections-performed/test]
    Case(
        id="cmd-redirections-performed",
        rules=("cmd.simple-redirections-performed",),
        script="printf 'x\\n' > out\ncat out\n",
        stdout="x\n",
    ),
    # [spec:posix:req:cmd.search-special-builtin/test]
    Case(
        id="cmd-search-special-builtin-first",
        rules=("cmd.search-special-builtin",),
        script="PATH=/nonexistent\nset -- a\nshift\necho $#\n",
        stdout="0\n",
    ),
    # [spec:posix:req:cmd.search-path-unsuccessful/test]
    # [spec:posix:req:cmd.nonbuiltin-path-search-unsuccessful/test]
    Case(
        id="cmd-search-path-unsuccessful",
        rules=(
            "cmd.search-path-unsuccessful",
            "cmd.nonbuiltin-path-search-unsuccessful",
        ),
        script="PATH=/nonexistent\nno_such_utility_xyz\n",
        status=127,
        stderr_contains=("no_such_utility_xyz",),
        stdout="",
    ),
    # [spec:posix:req:cmd.search-name-with-slash/test]
    # [spec:posix:req:cmd.nonbuiltin-slash-execl/test]
    Case(
        id="cmd-name-with-slash",
        rules=("cmd.search-name-with-slash", "cmd.nonbuiltin-slash-execl"),
        script="PATH=/nonexistent\n./prog\n",
        files={"prog": FileFixture("#!/bin/sh\nprintf 'ran\\n'\n", 0o755)},
        stdout="ran\n",
    ),
    # [spec:posix:req:cmd.nonbuiltin-slash-enoexec-script/test]
    # [spec:posix:req:cmd.nonbuiltin-enoexec-script/test]
    Case(
        id="cmd-enoexec-runs-as-script",
        rules=(
            "cmd.nonbuiltin-slash-enoexec-script",
            "cmd.nonbuiltin-enoexec-script",
        ),
        script="./noshebang\n",
        files={"noshebang": FileFixture("printf 'script-ran\\n'\n", 0o755)},
        stdout="script-ran\n",
    ),
    # [spec:posix:req:cmd.nonbuiltin-separate-environment/test]
    Case(
        id="cmd-nonbuiltin-separate-env",
        rules=("cmd.nonbuiltin-separate-environment",),
        script="v=parent\n./child\nprintf '%s\\n' \"$v\"\n",
        files={"child": FileFixture("#!/bin/sh\nv=child\n", 0o755)},
        stdout="parent\n",
    ),
    # [spec:posix:req:cmd.nonbuiltin-exec-replaces-environment/test]
    Case(
        id="cmd-exec-replaces-image",
        rules=("cmd.nonbuiltin-exec-replaces-environment",),
        script="exec printf 'replaced\\n'\nprintf 'unreachable\\n'\n",
        stdout="replaced\n",
    ),
    # [spec:posix:req:cmd.pipeline-connects-stdio/test]
    # [spec:posix:req:cmd.pipeline-foreground-wait/test]
    Case(
        id="cmd-pipeline-connects",
        rules=("cmd.pipeline-connects-stdio", "cmd.pipeline-foreground-wait"),
        script="printf 'a\\nb\\n' | wc -l | tr -d ' '\n",
        stdout="2\n",
    ),
    # [spec:posix:req:cmd.list-separator-semantics/test]
    # [spec:posix:req:cmd.sequential-execution/test]
    # [spec:posix:req:cmd.sequential-exit-status/test]
    Case(
        id="cmd-sequential-lists",
        rules=(
            "cmd.list-separator-semantics",
            "cmd.sequential-execution",
            "cmd.sequential-exit-status",
        ),
        script="echo one; echo two; false\necho $?\n",
        stdout="one\ntwo\n1\n",
    ),
    # [spec:posix:req:cmd.and-list-exit-status/test]
    # [spec:posix:req:cmd.or-list-exit-status/test]
    Case(
        id="cmd-and-or-exit-status",
        rules=("cmd.and-list-exit-status", "cmd.or-list-exit-status"),
        script="true && false; echo $?\nfalse || true; echo $?\nfalse && true; echo $?\n",
        stdout="1\n0\n1\n",
    ),
    # [spec:posix:req:cmd.compound-redirection-scope/test]
    Case(
        id="cmd-compound-redirection-scope",
        rules=("cmd.compound-redirection-scope",),
        script="{ echo a; echo b > inner; echo c; } > outer\ncat outer\ncat inner\n",
        stdout="a\nc\nb\n",
    ),
    # [spec:posix:req:cmd.group-exit-status/test]
    # [spec:posix:req:cmd.compound-list-exit-status/test]
    Case(
        id="cmd-group-exit-status",
        rules=("cmd.group-exit-status", "cmd.compound-list-exit-status"),
        script="{ true; exit 7; }\n",
        status=7,
        stdout="",
    ),
    # [spec:posix:req:cmd.for-omitted-in/test]
    # [spec:posix:req:cmd.for-exit-status/test]
    Case(
        id="cmd-for-omitted-in",
        rules=("cmd.for-omitted-in", "cmd.for-exit-status"),
        script="set -- p q\nfor i do printf '%s.' \"$i\"; done\nprintf '\\n'\nfor i in; do :; done\necho $?\n",
        stdout="p.q.\n0\n",
    ),
    # [spec:posix:req:cmd.case-clause-syntax/test]
    # [spec:posix:req:cmd.case-pattern-expansion/test]
    # [spec:posix:req:cmd.case-exit-status/test]
    Case(
        id="cmd-case-clauses",
        rules=(
            "cmd.case-clause-syntax",
            "cmd.case-pattern-expansion",
            "cmd.case-exit-status",
        ),
        script=(
            "p=b\n"
            "case b in a|$p) echo matched;; *) echo no;; esac\n"
            "case zz in a) :;; esac\necho $?\n"
        ),
        stdout="matched\n0\n",
    ),
    # [spec:posix:req:cmd.if-execution/test]
    # [spec:posix:req:cmd.if-exit-status/test]
    Case(
        id="cmd-if-exit-status",
        rules=("cmd.if-execution", "cmd.if-exit-status"),
        script=(
            "if true; then echo t; fi\n"
            "if false; then echo t; else echo e; fi\n"
            "if false; then :; fi\necho $?\n"
        ),
        stdout="t\ne\n0\n",
    ),
    # [spec:posix:req:cmd.while-exit-status/test]
    # [spec:posix:req:cmd.until-exit-status/test]
    Case(
        id="cmd-while-until-exit-status",
        rules=("cmd.while-exit-status", "cmd.until-exit-status"),
        script=(
            "while false; do :; done\necho $?\n"
            "until true; do :; done\necho $?\n"
            "i=0\nwhile [ $i -lt 2 ]; do i=$((i+1)); false; done\necho $?\n"
        ),
        stdout="0\n0\n1\n",
    ),
    # [spec:posix:req:cmd.function-exit-status/test]
    # [spec:posix:req:cmd.function-name-requirements/test]
    Case(
        id="cmd-function-exit-status",
        rules=("cmd.function-exit-status", "cmd.function-name-requirements"),
        script="f() { return 4; }\necho $?\nf\necho $?\n",
        stdout="0\n4\n",
    ),
    # [spec:posix:req:cmd.async-subshell-background/test]
    # [spec:posix:req:cmd.async-stdin-devnull/test]
    Case(
        id="cmd-async-subshell",
        rules=("cmd.async-subshell-background", "cmd.async-stdin-devnull"),
        script=(
            "v=parent\n"
            "{ v=child; } &\nwait\n"
            "printf '%s\\n' \"$v\"\n"
        ),
        stdout="parent\n",
    ),
    # [spec:posix:req:cmd.search-applies/test]
    # [spec:posix:req:cmd.search-path-non-builtin/test]
    Case(
        id="cmd-search-uses-path",
        rules=("cmd.search-applies", "cmd.search-path-non-builtin"),
        script="PATH=$PWD/bin:$PATH mytool\n",
        files={"bin/mytool": FileFixture("#!/bin/sh\nprintf 'found\\n'\n", 0o755)},
        stdout="found\n",
    ),
    # [spec:posix:req:cmd.simple-step-order-reversal/test]
    Case(
        id="cmd-assign-redirect-order",
        rules=("cmd.simple-step-order-reversal",),
        script="v=$( : ) > out\necho ok\n",
        stdout="ok\n",
    ),
)

# ---------------------------------------------------------------------
# Area modules. Each `cases_<area>.py` exports its own CASES tuple and is
# appended here, so several authors can add coverage at once without
# touching a shared file. Nothing else needs changing to register one.
# ---------------------------------------------------------------------

def _load_area_cases() -> tuple[Case, ...]:
    import importlib
    import pathlib

    found: list[Case] = []
    here = pathlib.Path(__file__).parent
    for path in sorted(here.glob("cases_*.py")):
        module = importlib.import_module(path.stem)
        found.extend(getattr(module, "CASES", ()))
    return tuple(found)


CASES = CASES + _load_area_cases()
