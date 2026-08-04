"""Executable cases for read/getopts, alias/unalias/fc, and command/type/hash.

Covers the rules in posix/docs/spec/builtins-input.md, builtins-alias.md and
builtins-command.md. Expectations state what POSIX.1-2024 requires, not what
dash currently does; several cases fail deliberately and are called out in the
area report.
"""

from __future__ import annotations

from model import Case, FileFixture


TOOL = FileFixture("#!/bin/sh\nprintf 'ran\\n'\n", 0o755)


CASES: tuple[Case, ...] = (
    # ------------------------------------------------------------------
    # read
    # ------------------------------------------------------------------
    # [spec:posix:syn:builtin.read.syn/test]
    Case(
        id="read-synopsis-forms",
        rules=("builtin.read.syn",),
        script=(
            "read -r one\n"
            "read two three\n"
            "printf '[%s][%s][%s]\\n' \"$one\" \"$two\" \"$three\"\n"
        ),
        stdin="alpha\nbeta gamma\n",
        stdout="[alpha][beta][gamma]\n",
    ),
    # [spec:posix:syn:builtin.read.syn/test]
    # [spec:posix:req:builtin.read.option-d/test]
    Case(
        id="read-opt-d-delimiter",
        rules=("builtin.read.syn", "builtin.read.option-d"),
        script=(
            "read -d : x < single\n"
            "printf '[%s]' \"$x\"\n"
            "read -d '' y < nul\n"
            "printf '[%s]\\n' \"$y\"\n"
        ),
        files={
            "single": FileFixture("abc:def\n"),
            "nul": FileFixture("ab\x00cd\x00"),
        },
        stdout="[abc][ab]\n",
    ),
    # [spec:posix:req:builtin.read.logical-line/test]
    # [spec:posix:req:builtin.read.terminating-delimiter-removed/test]
    Case(
        id="read-logical-line",
        rules=(
            "builtin.read.logical-line",
            "builtin.read.terminating-delimiter-removed",
        ),
        script="read a\nread b\nprintf '[%s][%s]\\n' \"$a\" \"$b\"\n",
        stdin="first line\nsecond line\n",
        stdout="[first line][second line]\n",
    ),
    # [spec:posix:req:builtin.read.backslash-escape/test]
    Case(
        id="read-backslash-escape",
        rules=("builtin.read.backslash-escape",),
        script=(
            "read x y\n"
            "read z\n"
            "printf '[%s][%s][%s]\\n' \"$x\" \"$y\" \"$z\"\n"
        ),
        # Input lines are:  a\ b c   and   x\\y
        stdin="a\\ b c\nx\\\\y\n",
        stdout="[a b][c][x\\y]\n",
    ),
    # [spec:posix:req:builtin.read.backslash-line-continuation/test]
    Case(
        id="read-backslash-continuation",
        rules=("builtin.read.backslash-line-continuation",),
        # First logical line is  one\<newline>two ; the escaped newline and its
        # backslash are removed, so a single field "onetwo" results.
        script="read x\nread y\nprintf '[%s][%s]\\n' \"$x\" \"$y\"\n",
        stdin="one\\\ntwo\nnext\n",
        stdout="[onetwo][next]\n",
    ),
    # [spec:posix:req:builtin.read.option-r/test]
    Case(
        id="read-opt-r",
        rules=("builtin.read.option-r",),
        script=(
            "read -r x y\n"
            "read -r z\n"
            "read -r w\n"
            "printf '[%s][%s][%s][%s]\\n' \"$x\" \"$y\" \"$z\" \"$w\"\n"
        ),
        # Lines:  a\ b   /   p\   /   continued
        stdin="a\\ b\np\\\ncontinued\n",
        stdout="[a\\][b][p\\][continued]\n",
    ),
    # [spec:posix:req:builtin.read.ifs-empty/test]
    Case(
        id="read-ifs-empty",
        rules=("builtin.read.ifs-empty",),
        script=(
            "IFS=\n"
            "read x y z\n"
            "printf '[%s][%s][%s]\\n' \"$x\" \"$y\" \"$z\"\n"
        ),
        stdin="  a b  c \n",
        stdout="[  a b  c ][][]\n",
    ),
    # [spec:posix:req:builtin.read.field-splitting-modified/test]
    # [spec:posix:req:builtin.read.var-assignment-order/test]
    # [spec:posix:req:builtin.read.env/test]
    Case(
        id="read-field-splitting",
        rules=(
            "builtin.read.field-splitting-modified",
            "builtin.read.var-assignment-order",
            "builtin.read.env",
        ),
        script=(
            "IFS=:\n"
            "read a b c\n"
            "printf '[%s][%s][%s];' \"$a\" \"$b\" \"$c\"\n"
            "IFS=' '\n"
            "read d e\n"
            "printf '[%s][%s]\\n' \"$d\" \"$e\"\n"
        ),
        stdin="one::three\nfour five six\n",
        stdout="[one][][three];[four][five six]\n",
    ),
    # [spec:posix:req:builtin.read.field-splitting-leftover/test]
    Case(
        id="read-field-splitting-leftover",
        rules=("builtin.read.field-splitting-leftover",),
        script="read x y\nprintf '[%s][%s]\\n' \"$x\" \"$y\"\n",
        stdin=" a  b  c d \n",
        stdout="[a][b  c d]\n",
    ),
    # [spec:posix:thm:builtin.read.single-var-unsplit/test]
    Case(
        id="read-single-var-unsplit",
        rules=("builtin.read.single-var-unsplit",),
        script="read x\nprintf '[%s]\\n' \"$x\"\n",
        stdin="   a  b   \n",
        stdout="[a  b]\n",
    ),
    # [spec:posix:req:builtin.read.unprocessed-vars-empty/test]
    Case(
        id="read-unprocessed-vars-empty",
        rules=("builtin.read.unprocessed-vars-empty",),
        script=(
            "y=preset\n"
            "z=preset\n"
            "read x y z\n"
            "printf '[%s][%s][%s]\\n' \"$x\" \"$y\" \"$z\"\n"
        ),
        stdin="solo\n",
        stdout="[solo][][]\n",
    ),
    # [spec:posix:req:builtin.read.affects-current-environment/test]
    Case(
        id="read-affects-current-environment",
        rules=("builtin.read.affects-current-environment",),
        script=(
            "x=orig\n"
            "(read x; printf '<%s>' \"$x\")\n"
            "printf '[%s]' \"$x\"\n"
            "read x\n"
            "printf '[%s]\\n' \"$x\"\n"
        ),
        stdin="sub\nmain\n",
        stdout="<sub>[orig][main]\n",
    ),
    # [spec:posix:req:builtin.read.variable-set-error/test]
    Case(
        id="read-variable-set-error",
        rules=("builtin.read.variable-set-error",),
        script=(
            "readonly b=keep\n"
            "read a b c\n"
            "st=$?\n"
            "[ \"$st\" -gt 1 ] && printf 'gt1;'\n"
            "printf '[%s][%s]\\n' \"$a\" \"$b\"\n"
        ),
        stdin="1 2 3\n",
        stdout="gt1;[1][keep]\n",
    ),
    # [spec:posix:req:builtin.read.end-of-file/test]
    # [spec:posix:req:builtin.read.exit-status/test]
    Case(
        id="read-end-of-file",
        rules=("builtin.read.end-of-file", "builtin.read.exit-status"),
        script=(
            "read x\n"
            "printf 'ok=%s;' \"$?\"\n"
            "read y\n"
            "printf 'eof=%s,[%s];' \"$?\" \"$y\"\n"
            "read z\n"
            "printf 'empty=%s,[%s]\\n' \"$?\" \"$z\"\n"
        ),
        stdin="complete\npartial",
        stdout="ok=0;eof=1,[partial];empty=1,[]\n",
    ),
    # [spec:posix:req:builtin.read.utility-syntax-guidelines/test]
    Case(
        id="read-utility-syntax-guidelines",
        rules=("builtin.read.utility-syntax-guidelines",),
        script=(
            "read -- x\n"
            "read -r -- y\n"
            "printf '[%s][%s]\\n' \"$x\" \"$y\"\n"
        ),
        stdin="one\ntw\\o\n",
        stdout="[one][tw\\o]\n",
    ),
    # [spec:posix:def:builtin.read.operand-var/test]
    Case(
        id="read-operand-var",
        rules=("builtin.read.operand-var",),
        script=(
            "existing=old\n"
            "read existing fresh\n"
            "printf '[%s][%s]\\n' \"$existing\" \"$fresh\"\n"
        ),
        stdin="new value\n",
        stdout="[new][value]\n",
    ),
    # [spec:posix:req:builtin.read.stdin/test]
    Case(
        id="read-stdin-arbitrary-bytes",
        rules=("builtin.read.stdin",),
        script=(
            "printf '\\200\\201\\n' > bytes\n"
            "read x < bytes\n"
            "printf '%s' \"$x\" | od -An -tx1 | tr -d ' \\n'\n"
            "printf ';'\n"
            "printf '' > empty\n"
            "read y < empty\n"
            "printf 'eof=%s\\n' \"$?\"\n"
        ),
        stdout="8081;eof=1\n",
    ),
    # [spec:posix:req:builtin.read.stderr/test]
    # [spec:posix:req:builtin.read.interfaces/test]
    Case(
        id="read-stderr-and-interfaces",
        rules=("builtin.read.stderr", "builtin.read.interfaces"),
        script=(
            "read x >out 2>err\n"
            "printf '[%s]' \"$x\"\n"
            "[ -s out ] || printf '[nostdout]'\n"
            "[ -s err ] || printf '[noerr]'\n"
            "readonly r=keep\n"
            "read r >out2 2>err2\n"
            "[ -s out2 ] || printf '[nostdout2]'\n"
            "[ -s err2 ] && printf '[diag]'\n"
            "printf '\\n'\n"
        ),
        stdin="value\nsecond\n",
        stdout="[value][nostdout][noerr][nostdout2][diag]\n",
    ),
    # [spec:posix:req:builtin.read.continuation-prompt/test]
    # [spec:posix:req:builtin.read.env-ps2/test]
    Case(
        id="read-continuation-prompt",
        rules=("builtin.read.continuation-prompt", "builtin.read.env-ps2"),
        mode="interactive",
        script=(
            "PS2='CONT-PROMPT> '\n"
            "read x\n"
            "first\\\n"
            "second\n"
            "printf 'got=[%s]\\n' \"$x\"\n"
            "exit 0\n"
        ),
        stdout=None,
        status="any",
        stdout_contains=("CONT-PROMPT> ", "got=[firstsecond]"),
    ),
    # ------------------------------------------------------------------
    # getopts
    # ------------------------------------------------------------------
    # [spec:posix:syn:builtin.getopts.syn/test]
    # [spec:posix:req:builtin.getopts.retrieve-options/test]
    # [spec:posix:def:builtin.getopts.operand-name/test]
    # [spec:posix:def:builtin.getopts.operand-optstring/test]
    # [spec:posix:req:builtin.getopts.exit-status/test]
    Case(
        id="getopts-basic-loop",
        rules=(
            "builtin.getopts.syn",
            "builtin.getopts.retrieve-options",
            "builtin.getopts.operand-name",
            "builtin.getopts.operand-optstring",
            "builtin.getopts.exit-status",
        ),
        script=(
            "set -- -a -b -f value operand\n"
            "getopts abf: opt; printf 'r1=%s,%s;' \"$?\" \"$opt\"\n"
            "getopts abf: opt; printf 'r2=%s,%s;' \"$?\" \"$opt\"\n"
            "getopts abf: opt; printf 'r3=%s,%s,%s;' \"$?\" \"$opt\" \"$OPTARG\"\n"
            "getopts abf: opt; printf 'r4=%s,%s;' \"$?\" \"$opt\"\n"
            "shift $((OPTIND-1))\n"
            "printf 'rest=%s;' \"$*\"\n"
            "( getopts; printf 'usage=%s\\n' \"$?\" ) 2>/dev/null\n"
        ),
        stdout="r1=0,a;r2=0,b;r3=0,f,value;r4=1,?;rest=operand;usage=2\n",
    ),
    # [spec:posix:req:builtin.getopts.optind-initialized/test]
    # [spec:posix:req:builtin.getopts.optind-after-invocation/test]
    # [spec:posix:req:builtin.getopts.end-of-options/test]
    Case(
        id="getopts-optind-progression",
        rules=(
            "builtin.getopts.optind-initialized",
            "builtin.getopts.optind-after-invocation",
            "builtin.getopts.end-of-options",
        ),
        script=(
            "printf 'init=%s;' \"$OPTIND\"\n"
            "set -- -f val -a rest\n"
            "getopts f:a o; printf 'o=%s,ind=%s;' \"$o\" \"$OPTIND\"\n"
            "getopts f:a o; printf 'o=%s,ind=%s;' \"$o\" \"$OPTIND\"\n"
            "getopts f:a o; printf 'st=%s,o=%s,ind=%s\\n' \"$?\" \"$o\" \"$OPTIND\"\n"
        ),
        stdout="init=1;o=f,ind=3;o=a,ind=4;st=1,o=?,ind=4\n",
    ),
    # [spec:posix:req:builtin.getopts.optarg-content/test]
    # [spec:posix:req:builtin.getopts.optstring-separate-arguments/test]
    Case(
        id="getopts-optarg-content",
        rules=(
            "builtin.getopts.optarg-content",
            "builtin.getopts.optstring-separate-arguments",
        ),
        script=(
            "( set -- -f val; getopts f: o; printf 'sep=[%s];' \"$OPTARG\" )\n"
            "( set -- -fval; getopts f: o; printf 'att=[%s]\\n' \"$OPTARG\" )\n"
        ),
        stdout="sep=[val];att=[val]\n",
    ),
    # [spec:posix:req:builtin.getopts.optarg/test]
    Case(
        id="getopts-optarg-unset",
        rules=("builtin.getopts.optarg",),
        script=(
            "( set -- -a; getopts ab o; printf 'noarg=[%s];' \"${OPTARG-UNSET}\" )\n"
            "( set -- -f v x; getopts f: o; getopts f: o;"
            " printf 'end=[%s]\\n' \"${OPTARG-UNSET}\" )\n"
        ),
        stdout="noarg=[UNSET];end=[UNSET]\n",
    ),
    # [spec:posix:req:builtin.getopts.unknown-option/test]
    # [spec:posix:sem:builtin.getopts.optstring-first-character/test]
    Case(
        id="getopts-unknown-option",
        rules=(
            "builtin.getopts.unknown-option",
            "builtin.getopts.optstring-first-character",
        ),
        script=(
            "( set -- -x; getopts ab o 2>e1; printf 'loud=st%s,[%s],[%s],' \"$?\" \"$o\""
            " \"${OPTARG-UNSET}\"; [ -s e1 ] && printf 'diag;' )\n"
            "( set -- -x; getopts :ab o 2>e2; printf 'quiet=st%s,[%s],[%s],' \"$?\" \"$o\""
            " \"${OPTARG-UNSET}\"; [ -s e2 ] || printf 'nodiag\\n' )\n"
        ),
        stdout="loud=st0,[?],[UNSET],diag;quiet=st0,[?],[x],nodiag\n",
    ),
    # [spec:posix:req:builtin.getopts.missing-option-argument/test]
    Case(
        id="getopts-missing-option-argument",
        rules=("builtin.getopts.missing-option-argument",),
        script=(
            "( set -- -f; getopts f: o 2>e1; printf 'loud=st%s,[%s],[%s],' \"$?\" \"$o\""
            " \"${OPTARG-UNSET}\"; [ -s e1 ] && printf 'diag;' )\n"
            "( set -- -f; getopts :f: o 2>e2; printf 'quiet=st%s,[%s],[%s],' \"$?\" \"$o\""
            " \"${OPTARG-UNSET}\"; [ -s e2 ] || printf 'nodiag\\n' )\n"
        ),
        stdout="loud=st0,[?],[UNSET],diag;quiet=st0,[:],[f],nodiag\n",
    ),
    # [spec:posix:def:builtin.getopts.end-of-options-identification/test]
    Case(
        id="getopts-end-of-options-identification",
        rules=("builtin.getopts.end-of-options-identification",),
        script=(
            "( set -- -a -- -b; getopts ab o; getopts ab o;"
            " printf 'dashdash=st%s,ind%s;' \"$?\" \"$OPTIND\" )\n"
            "( set -- -a plain -b; getopts ab o; getopts ab o;"
            " printf 'operand=st%s,ind%s\\n' \"$?\" \"$OPTIND\" )\n"
        ),
        stdout="dashdash=st1,ind3;operand=st1,ind2\n",
    ),
    # [spec:posix:req:builtin.getopts.no-export/test]
    Case(
        id="getopts-no-export",
        rules=("builtin.getopts.no-export",),
        script=(
            "set -- -f val\n"
            "getopts f: o\n"
            "env | sed -n 's/^\\(OPTIND\\|OPTARG\\)=.*/exported:&/p'\n"
            "printf 'done\\n'\n"
        ),
        stdout="done\n",
    ),
    # [spec:posix:req:builtin.getopts.variable-set-error/test]
    Case(
        id="getopts-variable-set-error",
        rules=("builtin.getopts.variable-set-error",),
        script=(
            "readonly o\n"
            "set -- -a\n"
            "getopts a o 2>/dev/null\n"
            "st=$?\n"
            "[ \"$st\" -gt 1 ] && printf 'gt1\\n'\n"
        ),
        stdout="gt1\n",
    ),
    # [spec:posix:sem:builtin.getopts.affects-current-environment/test]
    Case(
        id="getopts-affects-current-environment",
        rules=("builtin.getopts.affects-current-environment",),
        script=(
            "set -- -f val\n"
            "getopts f: o\n"
            "printf '[%s][%s][%s]\\n' \"$o\" \"$OPTARG\" \"$OPTIND\"\n"
        ),
        stdout="[f][val][3]\n",
    ),
    # [spec:posix:sem:builtin.getopts.reset/test]
    # [spec:posix:req:builtin.getopts.env-optind/test]
    Case(
        id="getopts-optind-reset",
        rules=("builtin.getopts.reset", "builtin.getopts.env-optind"),
        script=(
            "set -- -a -b\n"
            "getopts ab o\n"
            "printf 'first=%s;' \"$o\"\n"
            "OPTIND=1\n"
            "getopts ab o\n"
            "printf 'second=%s,ind=%s\\n' \"$o\" \"$OPTIND\"\n"
        ),
        stdout="first=a;second=a,ind=2\n",
    ),
    # [spec:posix:req:builtin.getopts.operand-param/test]
    Case(
        id="getopts-operand-param",
        rules=("builtin.getopts.operand-param",),
        script=(
            "( set -- -z; getopts ab o -a -b; printf 'params=%s,%s;' \"$o\" \"$OPTIND\" )\n"
            "( getopts ab o operand; printf 'operand=st%s,%s;' \"$?\" \"$OPTIND\" )\n"
            "( getopts ab o -a; getopts ab o -a;"
            " printf 'exhausted=st%s,%s\\n' \"$?\" \"$OPTIND\" )\n"
        ),
        stdout="params=a,2;operand=st1,1;exhausted=st1,2\n",
    ),
    # [spec:posix:req:builtin.getopts.stderr-diagnostic/test]
    Case(
        id="getopts-stderr-diagnostic",
        rules=("builtin.getopts.stderr-diagnostic",),
        args=("mygetoptsprog", "-x"),
        script=(
            "getopts ab o 2>err\n"
            "set -- -f\n"
            "getopts f: o 2>>err\n"
            "cat err\n"
        ),
        stdout=None,
        stdout_contains=("mygetoptsprog", "x", "f"),
    ),
    # [spec:posix:req:builtin.getopts.interfaces/test]
    Case(
        id="getopts-interfaces",
        rules=("builtin.getopts.interfaces",),
        script=(
            "set -- -a\n"
            "printf 'DATA\\n' | { getopts a o >out; read line;"
            " printf 'read=[%s],out=[%s]\\n' \"$line\" \"$(cat out)\"; }\n"
        ),
        stdout="read=[DATA],out=[]\n",
    ),
    # ------------------------------------------------------------------
    # alias
    # ------------------------------------------------------------------
    # [spec:posix:syn:builtin.alias.synopsis/test]
    # [spec:posix:req:builtin.alias.create-or-display/test]
    # [spec:posix:req:builtin.alias.operands/test]
    Case(
        id="alias-create-and-display",
        rules=(
            "builtin.alias.synopsis",
            "builtin.alias.create-or-display",
            "builtin.alias.operands",
        ),
        script=(
            "alias one=ONE two=TWO\n"
            "printf 'named=%s;' \"$(alias one | grep -c ONE)\"\n"
            "printf 'all=%s;' \"$(alias | grep -c .)\"\n"
            "alias one=REDEFINED\n"
            "printf 'redef=%s\\n' \"$(alias one | grep -c REDEFINED)\"\n"
        ),
        stdout="named=1;all=2;redef=1\n",
    ),
    # [spec:posix:req:builtin.alias.stdout-format/test]
    Case(
        id="alias-stdout-format",
        rules=("builtin.alias.stdout-format",),
        script=(
            "alias sample=value\n"
            "alias sample > out\n"
            "while IFS= read -r line; do\n"
            "  case $line in\n"
            "  sample=*) printf 'name-then-equals\\n' ;;\n"
            "  *) printf 'unexpected:%s\\n' \"$line\" ;;\n"
            "  esac\n"
            "done < out\n"
        ),
        stdout="name-then-equals\n",
    ),
    # [spec:posix:req:builtin.alias.stdout-format/test]
    Case(
        id="alias-stdout-reinput",
        rules=("builtin.alias.stdout-format",),
        script=(
            "alias q=\"printf '%s\\\\n' 'a b'\"\n"
            "def=$(alias q)\n"
            "unalias -a\n"
            "eval \"alias $def\"\n"
            "q\n"
        ),
        stdout="a b\n",
    ),
    # [spec:posix:def:builtin.alias.definition/test]
    Case(
        id="alias-definition-replaces-command-name",
        rules=("builtin.alias.definition",),
        script=(
            "alias greet=\"printf 'GREETED\\n'\"\n"
            "greet\n"
        ),
        stdout="GREETED\n",
    ),
    # [spec:posix:req:builtin.alias.execution-environment/test]
    Case(
        id="alias-execution-environment",
        rules=("builtin.alias.execution-environment",),
        script=(
            "alias hi=\"printf 'SUB\\n'\"\n"
            "( hi )\n"
            "sh -c 'hi' 2>/dev/null || printf 'not-in-utility-env\\n'\n"
        ),
        stdout="SUB\nnot-in-utility-env\n",
    ),
    # [spec:posix:req:builtin.alias.stderr/test]
    Case(
        id="alias-stderr",
        rules=("builtin.alias.stderr",),
        script=(
            "alias ok=1 2>err\n"
            "[ -s err ] || printf 'no-diag;'\n"
            "alias missingname >out2 2>err2\n"
            "[ -s out2 ] || printf 'no-stdout;'\n"
            "[ -s err2 ] && printf 'diag-on-stderr\\n'\n"
        ),
        stdout="no-diag;no-stdout;diag-on-stderr\n",
    ),
    # [spec:posix:req:builtin.alias.interfaces/test]
    Case(
        id="alias-interfaces",
        rules=("builtin.alias.interfaces",),
        script=(
            "printf 'DATA\\n' | { alias a=1; read line; printf 'read=[%s];' \"$line\"; }\n"
            "alias -x >out 2>err\n"
            "printf 'dashx=%s;' \"$?\"\n"
            "[ -s out ] || printf 'no-stdout\\n'\n"
        ),
        stdout="read=[DATA];dashx=1;no-stdout\n",
    ),
    # [spec:posix:req:builtin.alias.exit-status/test]
    Case(
        id="alias-exit-status",
        rules=("builtin.alias.exit-status",),
        script=(
            "alias good=1\n"
            "alias good >/dev/null && printf 'zero;'\n"
            "alias nosuch >/dev/null 2>&1 || printf 'nonzero\\n'\n"
        ),
        stdout="zero;nonzero\n",
    ),
    # ------------------------------------------------------------------
    # unalias
    # ------------------------------------------------------------------
    # [spec:posix:syn:builtin.unalias.synopsis/test]
    # [spec:posix:req:builtin.unalias.remove-definitions/test]
    # [spec:posix:req:builtin.unalias.operand-alias-name/test]
    Case(
        id="unalias-remove-definitions",
        rules=(
            "builtin.unalias.synopsis",
            "builtin.unalias.remove-definitions",
            "builtin.unalias.operand-alias-name",
        ),
        script=(
            "alias gone=\"printf 'GONE\\n'\"\n"
            "alias kept=\"printf 'KEPT\\n'\"\n"
            "gone\n"
            "unalias gone\n"
            "gone 2>/dev/null || printf 'substitution-removed\\n'\n"
            "kept\n"
        ),
        stdout="GONE\nsubstitution-removed\nKEPT\n",
    ),
    # [spec:posix:syn:builtin.unalias.synopsis/test]
    # [spec:posix:req:builtin.unalias.opt-a/test]
    Case(
        id="unalias-opt-a",
        rules=("builtin.unalias.synopsis", "builtin.unalias.opt-a"),
        script=(
            "alias a=1 b=2\n"
            "unalias -a\n"
            "printf 'st=%s;' \"$?\"\n"
            "alias > out\n"
            "[ -s out ] || printf 'empty\\n'\n"
        ),
        stdout="st=0;empty\n",
    ),
    # [spec:posix:req:builtin.unalias.utility-syntax-guidelines/test]
    Case(
        id="unalias-utility-syntax-guidelines",
        rules=("builtin.unalias.utility-syntax-guidelines",),
        script=(
            "alias keep=1\n"
            "unalias -- keep\n"
            "printf 'st=%s;' \"$?\"\n"
            "alias keep >/dev/null 2>&1 || printf 'gone\\n'\n"
        ),
        stdout="st=0;gone\n",
    ),
    # [spec:posix:req:builtin.unalias.stderr/test]
    # [spec:posix:req:builtin.unalias.interfaces/test]
    Case(
        id="unalias-stderr-and-interfaces",
        rules=("builtin.unalias.stderr", "builtin.unalias.interfaces"),
        script=(
            "alias k=1\n"
            "unalias k >out 2>err\n"
            "[ -s out ] || printf 'no-stdout;'\n"
            "[ -s err ] || printf 'no-diag;'\n"
            "unalias nosuch >out2 2>err2\n"
            "[ -s out2 ] || printf 'no-stdout2;'\n"
            "[ -s err2 ] && printf 'diag-on-stderr;'\n"
            "printf 'DATA\\n' | { unalias -a; read line; printf 'read=[%s]\\n' \"$line\"; }\n"
        ),
        stdout="no-stdout;no-diag;no-stdout2;diag-on-stderr;read=[DATA]\n",
    ),
    # [spec:posix:req:builtin.unalias.exit-status/test]
    Case(
        id="unalias-exit-status",
        rules=("builtin.unalias.exit-status",),
        script=(
            "alias k=1\n"
            "unalias k && printf 'zero;'\n"
            "unalias nosuch 2>/dev/null || printf 'nonzero\\n'\n"
        ),
        stdout="zero;nonzero\n",
    ),
    # ------------------------------------------------------------------
    # fc (User Portability Utilities option; needs an interactive shell)
    # ------------------------------------------------------------------
    # [spec:posix:syn:builtin.fc.synopsis/test]
    # [spec:posix:req:builtin.fc.list-or-edit/test]
    # [spec:posix:req:builtin.fc.opt-l/test]
    Case(
        id="fc-list-and-reexecute",
        rules=(
            "builtin.fc.synopsis",
            "builtin.fc.list-or-edit",
            "builtin.fc.opt-l",
        ),
        mode="interactive",
        requires=("UP",),
        script=(
            "printf 'ONCE\\n' >> tally\n"
            "printf 'listed=%s\\n' \"$(fc -l -n 1 1 | tr -d '\\t' | tr -d '\\n')\"\n"
            "fc -s 1 >/dev/null 2>&1\n"
            "printf 'tally=%s\\n' \"$(wc -l < tally | tr -d ' ')\"\n"
            "exit 0\n"
        ),
        stdout=None,
        status="any",
        stdout_contains=("listed=printf 'ONCE", "tally=2"),
    ),
    # [spec:posix:req:builtin.fc.history-numbering/test]
    Case(
        id="fc-history-numbering",
        rules=("builtin.fc.history-numbering",),
        mode="interactive",
        requires=("UP",),
        script=(
            ": ALPHA\n"
            ": BETA\n"
            "fc -l 1 2 > f1\n"
            "fc -l 1 2 > f2\n"
            "cmp -s f1 f2 && printf 'STABLE\\n'\n"
            "printf 'number=%s\\n' \"$(fc -l 1 1 | tr -cd '0-9')\"\n"
            "exit 0\n"
        ),
        stdout=None,
        status="any",
        stdout_contains=("STABLE", "number=1"),
    ),
    # [spec:posix:req:builtin.fc.utility-syntax-guidelines/test]
    Case(
        id="fc-utility-syntax-guidelines",
        rules=("builtin.fc.utility-syntax-guidelines",),
        mode="interactive",
        requires=("UP",),
        script=(
            ": ALPHA\n"
            "printf 'sep=%s\\n' \"$(fc -l -n 1 1 | tr -d '\\t' | tr -d '\\n')\"\n"
            "printf 'grp=%s\\n' \"$(fc -ln 1 1 | tr -d '\\t' | tr -d '\\n')\"\n"
            "exit 0\n"
        ),
        stdout=None,
        status="any",
        stdout_contains=("sep=: ALPHA", "grp=: ALPHA"),
    ),
    # [spec:posix:req:builtin.fc.env-histfile/test]
    Case(
        id="blt2-fc-histfile-names-history-file",
        rules=("builtin.fc.env-histfile",),
        mode="interactive",
        requires=("UP",),
        # "HISTFILE - Determine a pathname naming a command history file."
        # The escape clause covers only a shell that cannot obtain read and
        # write access to, or create, that file; HISTFILE here names a fresh
        # pathname in the case's own writable directory, so the shell must
        # use it. An inner interactive shell runs and exits first, so a shell
        # that only flushes its history at exit is still judged fairly.
        environment={"HISTFILE": "{ROOT}/histfile"},
        script=(
            "sh -i\n"
            ": HISTORY_MARKER\n"
            "exit\n"
            'test -s "$HISTFILE" && printf \'histfile=used\\n\'\n'
            "exit 0\n"
        ),
        stdout=None,
        status="any",
        stdout_contains=("histfile=used",),
        timeout=8.0,
    ),
    # [spec:posix:req:builtin.fc.opt-n/test]
    Case(
        id="fc-opt-n",
        rules=("builtin.fc.opt-n",),
        mode="interactive",
        requires=("UP",),
        script=(
            ": ALPHA\n"
            "printf 'numbered=%s\\n' \"$(fc -l 1 1 | tr -cd '0-9')\"\n"
            "printf 'suppressed=[%s]\\n' \"$(fc -l -n 1 1 | tr -cd '0-9')\"\n"
            "exit 0\n"
        ),
        stdout=None,
        status="any",
        stdout_contains=("numbered=1", "suppressed=[]"),
    ),
    # [spec:posix:req:builtin.fc.opt-r/test]
    Case(
        id="fc-opt-r",
        rules=("builtin.fc.opt-r",),
        mode="interactive",
        requires=("UP",),
        script=(
            ": ALPHA\n"
            ": BETA\n"
            "printf 'rev=%s\\n' \"$(fc -l -n -r 1 2 | tr -d '\\t' | tr '\\n' '|')\"\n"
            "exit 0\n"
        ),
        stdout=None,
        status="any",
        stdout_contains=("rev=: BETA|: ALPHA|",),
    ),
    # [spec:posix:req:builtin.fc.opt-s/test]
    # [spec:posix:req:builtin.fc.operand-default-s/test]
    Case(
        id="fc-opt-s-reexecutes-previous",
        rules=("builtin.fc.opt-s", "builtin.fc.operand-default-s"),
        mode="interactive",
        requires=("UP",),
        script=(
            "printf 'X\\n' >> tally\n"
            "fc -s >/dev/null 2>&1\n"
            "printf 'tally=%s\\n' \"$(wc -l < tally | tr -d ' ')\"\n"
            "exit 0\n"
        ),
        stdout=None,
        status="any",
        stdout_contains=("tally=2",),
    ),
    # [spec:posix:req:builtin.fc.operand-old-new/test]
    Case(
        id="fc-operand-old-new",
        rules=("builtin.fc.operand-old-new",),
        mode="interactive",
        requires=("UP",),
        script=(
            "printf 'aaa\\n'\n"
            "fc -s aaa=bbb 2>/dev/null\n"
            "exit 0\n"
        ),
        stdout=None,
        status="any",
        stdout_contains=("bbb\n",),
    ),
    # [spec:posix:syn:builtin.fc.operand-first-last/test]
    Case(
        id="fc-operand-first-last",
        rules=("builtin.fc.operand-first-last",),
        mode="interactive",
        requires=("UP",),
        script=(
            "ZED=1\n"
            ": ALPHA\n"
            "printf 'num=%s\\n' \"$(fc -l -n 2 2 | tr -d '\\t' | tr -d '\\n')\"\n"
            "printf 'str=%s\\n' \"$(fc -l -n ZED ZED | tr -d '\\t' | tr -d '\\n')\"\n"
            "printf 'neg=%s\\n' \"$(fc -l -n -1 -1 | tr -d '\\t' | tr -d '\\n')\"\n"
            "exit 0\n"
        ),
        stdout=None,
        status="any",
        stdout_contains=("num=: ALPHA", "str=ZED=1", "neg=printf 'str="),
    ),
    # [spec:posix:req:builtin.fc.operand-defaults-no-s/test]
    Case(
        id="fc-operand-defaults-no-s",
        rules=("builtin.fc.operand-defaults-no-s",),
        mode="interactive",
        requires=("UP",),
        script=(
            ": ALPHA\n"
            ": BETA\n"
            "printf 'none=%s\\n' \"$(fc -l -n | tr -d '\\t' | tr '\\n' '|')\"\n"
            "printf 'first=%s\\n' \"$(fc -l -n 1 | tr -d '\\t' | tr '\\n' '|')\"\n"
            "exit 0\n"
        ),
        stdout=None,
        status="any",
        stdout_contains=("none=: ALPHA|: BETA|", "first=: ALPHA|: BETA|"),
    ),
    # [spec:posix:req:builtin.fc.operand-range/test]
    Case(
        id="fc-operand-range",
        rules=("builtin.fc.operand-range",),
        mode="interactive",
        requires=("UP",),
        script=(
            ": ALPHA\n"
            ": BETA\n"
            ": GAMMA\n"
            "printf 'fwd=%s\\n' \"$(fc -l -n 2 3 | tr -d '\\t' | tr '\\n' '|')\"\n"
            "printf 'rev=%s\\n' \"$(fc -l -n 3 2 | tr -d '\\t' | tr '\\n' '|')\"\n"
            "exit 0\n"
        ),
        stdout=None,
        status="any",
        stdout_contains=("fwd=: BETA|: GAMMA|", "rev=: GAMMA|: BETA|"),
    ),
    # [spec:posix:req:builtin.fc.operand-range-clamping/test]
    Case(
        id="fc-operand-range-clamping",
        rules=("builtin.fc.operand-range-clamping",),
        mode="interactive",
        requires=("UP",),
        script=(
            ": ALPHA\n"
            ": BETA\n"
            "fc -l -n 1 99 > listed 2>err\n"
            "st=$?\n"
            "[ -s err ] && diag=yes || diag=no\n"
            "body=$(tr -d '\\t' < listed | tr '\\n' '|')\n"
            "printf 'st=%s;diag=%s;listed=%s\\n' \"$st\" \"$diag\" \"$body\"\n"
            "exit 0\n"
        ),
        stdout=None,
        status="any",
        stdout_contains=("st=0;diag=no;listed=: ALPHA|: BETA|",),
    ),
    # [spec:posix:req:builtin.fc.edit-and-reexecute/test]
    Case(
        id="fc-reexecuted-line-enters-history",
        rules=("builtin.fc.edit-and-reexecute",),
        mode="interactive",
        requires=("UP",),
        script=(
            ": aaa\n"
            "fc -s >/dev/null 2>&1\n"
            "printf 'hist=%s\\n' \"$(fc -l -n | tr -d '\\t' | tr '\\n' '|')\"\n"
            "exit 0\n"
        ),
        stdout=None,
        status="any",
        stdout_contains=("hist=: aaa|: aaa|",),
    ),
    # [spec:posix:req:builtin.fc.edit-and-reexecute/test]
    Case(
        id="fc-redirection-affects-both",
        rules=("builtin.fc.edit-and-reexecute",),
        mode="interactive",
        requires=("UP",),
        script=(
            "printf 'REDIRTEST\\n'\n"
            "fc -s > out 2> err\n"
            "printf 'out=%s,err=%s\\n' \"$(grep -c REDIRTEST out)\" \"$(grep -c REDIRTEST err)\"\n"
            "exit 0\n"
        ),
        stdout=None,
        status="any",
        stdout_contains=("out=1,err=1",),
    ),
    # [spec:posix:req:builtin.fc.stdout-list-format/test]
    Case(
        id="fc-stdout-list-format",
        rules=("builtin.fc.stdout-list-format",),
        mode="interactive",
        requires=("UP",),
        script=(
            ": ALPHA\n"
            "printf 'numbered=[%s]\\n' \"$(fc -l 1 1 | tr '\\t' 'T' | tr -d '\\n')\"\n"
            "printf 'plain=[%s]\\n' \"$(fc -l -n 1 1 | tr '\\t' 'T' | tr -d '\\n')\"\n"
            "exit 0\n"
        ),
        stdout=None,
        status="any",
        stdout_contains=("numbered=[1T: ALPHA]", "plain=[T: ALPHA]"),
    ),
    # [spec:posix:req:builtin.fc.env-histsize/test]
    Case(
        id="fc-env-histsize-default",
        rules=("builtin.fc.env-histsize",),
        mode="interactive",
        requires=("UP",),
        timeout=30.0,
        script=(
            ": MARKER1\n"
            + "".join(": filler%d\n" % n for n in range(2, 128))
            + "printf 'oldest=%s\\n' \"$(fc -l -n 1 1 | head -1 | tr -d '\\t')\"\n"
            "exit 0\n"
        ),
        stdout=None,
        status="any",
        stdout_contains=("oldest=: MARKER1",),
    ),
    # [spec:posix:req:builtin.fc.stderr/test]
    Case(
        id="fc-stderr",
        rules=("builtin.fc.stderr",),
        mode="interactive",
        requires=("UP",),
        script=(
            ": ALPHA\n"
            "printf 'listing=[%s]\\n' \"$(fc -l -n 2>/dev/null | tr -d '\\t' | tr '\\n' '|')\"\n"
            "printf 'diagnostic=[%s]\\n' \"$(fc -l NOSUCHPREFIX 2>/dev/null)\"\n"
            "exit 0\n"
        ),
        stdout=None,
        status="any",
        stdout_contains=("listing=[: ALPHA|]", "diagnostic=[]"),
    ),
    # [spec:posix:req:builtin.fc.interfaces/test]
    Case(
        id="fc-interfaces",
        rules=("builtin.fc.interfaces",),
        mode="interactive",
        requires=("UP",),
        script=(
            ": ALPHA\n"
            "printf 'DATA\\n' | { fc -l -n >/dev/null 2>&1; read line;"
            " printf 'read=[%s]\\n' \"$line\"; }\n"
            "exit 0\n"
        ),
        stdout=None,
        status="any",
        stdout_contains=("read=[DATA]",),
    ),
    # [spec:posix:req:builtin.fc.exit-status/test]
    Case(
        id="fc-exit-status-list",
        rules=("builtin.fc.exit-status",),
        mode="interactive",
        requires=("UP",),
        script=(
            ": ALPHA\n"
            "fc -l >/dev/null 2>&1\n"
            "listing=$?\n"
            "fc -l NOSUCHPREFIX >/dev/null 2>&1\n"
            "failure=$?\n"
            "[ \"$failure\" -gt 0 ] && failure=nonzero\n"
            "printf 'list=%s;error=%s\\n' \"$listing\" \"$failure\"\n"
            "exit 0\n"
        ),
        stdout=None,
        status="any",
        stdout_contains=("list=0;error=nonzero",),
    ),
    # [spec:posix:req:builtin.fc.exit-status/test]
    Case(
        id="fc-exit-status-reexecuted",
        rules=("builtin.fc.exit-status",),
        mode="interactive",
        requires=("UP",),
        script=(
            "( exit 3 )\n"
            "fc -s >/dev/null 2>&1\n"
            "printf 'reexec=%s\\n' \"$?\"\n"
            "exit 0\n"
        ),
        stdout=None,
        status="any",
        stdout_contains=("reexec=3",),
    ),
    # ------------------------------------------------------------------
    # command
    # ------------------------------------------------------------------
    # [spec:posix:syn:builtin.command.synopsis/test]
    # [spec:posix:def:builtin.command.operands/test]
    Case(
        id="command-synopsis-forms",
        rules=("builtin.command.synopsis", "builtin.command.operands"),
        script=(
            "command printf '%s\\n' hello\n"
            "command -p printf '%s\\n' with-p\n"
            "command -v printf >/dev/null && printf 'v-ok\\n'\n"
            "command -V printf >/dev/null && printf 'bigv-ok\\n'\n"
            "command -p -v printf >/dev/null && printf 'pv-ok\\n'\n"
        ),
        stdout="hello\nwith-p\nv-ok\nbigv-ok\npv-ok\n",
    ),
    # [spec:posix:req:builtin.command.utility-syntax-guidelines/test]
    Case(
        id="command-utility-syntax-guidelines",
        rules=("builtin.command.utility-syntax-guidelines",),
        script=(
            "command -- printf '%s\\n' delimited\n"
            "command -pv printf >/dev/null && printf 'grouped\\n'\n"
        ),
        stdout="delimited\ngrouped\n",
    ),
    # [spec:posix:req:builtin.command.suppress-function-lookup/test]
    Case(
        id="command-suppress-function-lookup",
        rules=("builtin.command.suppress-function-lookup",),
        script=(
            "greet() { printf 'FUNCTION\\n'; }\n"
            "greet\n"
            "command greet 2>/dev/null || printf 'function-not-searched\\n'\n"
        ),
        stdout="FUNCTION\nfunction-not-searched\n",
    ),
    # [spec:posix:req:builtin.command.special-builtin-properties-suppressed/test]
    Case(
        id="command-special-builtin-properties-suppressed",
        rules=("builtin.command.special-builtin-properties-suppressed",),
        script=(
            "v=one :\n"
            "printf 'plain=[%s];' \"$v\"\n"
            "w=two command :\n"
            "printf 'wrapped=[%s];' \"$w\"\n"
            "command set -o nosuchoption 2>/dev/null\n"
            "printf 'survived\\n'\n"
        ),
        stdout="plain=[one];wrapped=[];survived\n",
    ),
    # [spec:posix:req:builtin.command.equivalent-to-omitting-command/test]
    Case(
        id="command-equivalent-to-omitting-command",
        rules=("builtin.command.equivalent-to-omitting-command",),
        script=(
            "alias true='printf ALIASED\\n'\n"
            "command true\n"
            "printf 'no-alias-substitution;'\n"
            "command for 2>/dev/null || printf 'no-reserved-word\\n'\n"
        ),
        stdout="no-alias-substitution;no-reserved-word\n",
    ),
    # [spec:posix:req:builtin.command.declaration-utility/test]
    Case(
        id="command-declaration-utility",
        rules=("builtin.command.declaration-utility",),
        script=(
            "HOME=/decl-home\n"
            "command export dv=~\n"
            "printf 'export=[%s];' \"$dv\"\n"
            "command readonly rv=~\n"
            "printf 'readonly=[%s];' \"$rv\"\n"
            "control=$(printf '%s' w=~)\n"
            "printf 'control=[%s]\\n' \"$control\"\n"
        ),
        stdout="export=[/decl-home];readonly=[/decl-home];control=[w=~]\n",
    ),
    # [spec:posix:req:builtin.command.v-options-report-interpretation/test]
    # [spec:posix:req:builtin.command.opt-v/test]
    # [spec:posix:req:builtin.command.stdout-format/test]
    Case(
        id="command-opt-v-categories",
        rules=(
            "builtin.command.v-options-report-interpretation",
            "builtin.command.opt-v",
            "builtin.command.stdout-format",
        ),
        files={"tools/mytool": TOOL},
        environment={"PATH": "{ROOT}/tools:/usr/bin:/bin"},
        script=(
            "myfunc() { :; }\n"
            "command -v myfunc\n"
            "command -v while\n"
            "command -v set\n"
            "command -v cd\n"
            "command -v mytool\n"
            "command -v nosuchutility && printf 'BAD\\n' || printf 'not-found\\n'\n"
        ),
        stdout="myfunc\nwhile\nset\ncd\n{ROOT}/tools/mytool\nnot-found\n",
    ),
    # [spec:posix:req:builtin.command.opt-v/test]
    Case(
        id="command-opt-v-alias",
        rules=("builtin.command.opt-v",),
        script=(
            "alias al='printf ALIASRAN\\n'\n"
            "command -v al > out\n"
            "unalias -a\n"
            "eval \"$(cat out)\"\n"
            "alias al >/dev/null && printf 'alias-restored\\n'\n"
        ),
        stdout="alias-restored\n",
    ),
    # [spec:posix:req:builtin.command.opt-v/test]
    Case(
        id="command-opt-v-slash-absolute",
        rules=("builtin.command.opt-v",),
        files={"tools/mytool": TOOL},
        script="command -v ./tools/mytool\n",
        stdout="{ROOT}/tools/mytool\n",
    ),
    # [spec:posix:req:builtin.command.opt-v-uppercase/test]
    Case(
        id="command-opt-v-uppercase",
        rules=("builtin.command.opt-v-uppercase",),
        files={"tools/mytool": TOOL},
        environment={"PATH": "{ROOT}/tools:/usr/bin:/bin"},
        script=(
            "myfunc() { :; }\n"
            "alias myalias='printf hi'\n"
            "command -V myfunc\n"
            "command -V myalias\n"
            "command -V set\n"
            "command -V cd\n"
            "command -V mytool\n"
            "command -V while > reserved\n"
            "[ -s reserved ] && printf 'reserved-reported\\n'\n"
        ),
        stdout=None,
        stdout_contains=(
            "function",
            "alias",
            "printf hi",
            "special",
            "builtin",
            "{ROOT}/tools/mytool",
            "reserved-reported",
        ),
    ),
    # [spec:posix:req:builtin.command.opt-p/test]
    Case(
        id="command-opt-p",
        rules=("builtin.command.opt-p",),
        script=(
            "PATH=/nonexistent-directory\n"
            "command cat /dev/null 2>/dev/null || printf 'unset-path-fails;'\n"
            "command -p cat /dev/null && printf 'default-path-finds-it\\n'\n"
        ),
        stdout="unset-path-fails;default-path-finds-it\n",
    ),
    # [spec:posix:sem:builtin.command.env-path/test]
    Case(
        id="command-env-path",
        rules=("builtin.command.env-path",),
        files={"tools/mytool": TOOL},
        script=(
            "PATH=$PWD/tools\n"
            "command -v mytool\n"
            "PATH=/nonexistent-directory\n"
            "command -v mytool >/dev/null 2>&1 || printf 'gone\\n'\n"
        ),
        stdout="{ROOT}/tools/mytool\ngone\n",
    ),
    # [spec:posix:req:builtin.command.stdout-format/test]
    Case(
        id="command-stdout-format",
        rules=("builtin.command.stdout-format",),
        script=(
            "command -v cd\n"
            "printf 'bigv-lines=%s\\n' \"$(command -V cd | wc -l | tr -d ' ')\"\n"
        ),
        stdout="cd\nbigv-lines=1\n",
    ),
    # [spec:posix:req:builtin.command.stderr/test]
    Case(
        id="command-stderr",
        rules=("builtin.command.stderr",),
        script=(
            "command -v cd >out 2>err\n"
            "[ -s err ] || printf 'no-diag;'\n"
            "command -v nosuchutility >out2 2>err2\n"
            "[ -s out2 ] || printf 'no-stdout;'\n"
            "command nosuchutility >out3 2>err3\n"
            "[ -s err3 ] && printf 'diag-on-stderr\\n'\n"
        ),
        stdout="no-diag;no-stdout;diag-on-stderr\n",
    ),
    # [spec:posix:req:builtin.command.interfaces/test]
    Case(
        id="command-interfaces",
        rules=("builtin.command.interfaces",),
        script=(
            "printf 'DATA\\n' | { command -v cd >/dev/null; read line;"
            " printf 'read=[%s]\\n' \"$line\"; }\n"
        ),
        stdout="read=[DATA]\n",
    ),
    # [spec:posix:req:builtin.command.exit-status-v-options/test]
    Case(
        id="command-exit-status-v-options",
        rules=("builtin.command.exit-status-v-options",),
        script=(
            "command -v cd >/dev/null; printf 'v=%s;' \"$?\"\n"
            "command -V cd >/dev/null; printf 'bigv=%s;' \"$?\"\n"
            "command -v nosuch >/dev/null 2>&1 || printf 'v-nonzero;'\n"
            "command -V nosuch >/dev/null 2>&1 || printf 'bigv-nonzero\\n'\n"
        ),
        stdout="v=0;bigv=0;v-nonzero;bigv-nonzero\n",
    ),
    # [spec:posix:req:builtin.command.exit-status-invocation/test]
    Case(
        id="command-exit-status-invocation",
        rules=("builtin.command.exit-status-invocation",),
        files={"blocked/.keep": FileFixture("x"), "tools/mytool": TOOL},
        script=(
            "command nosuchutility 2>/dev/null; printf '%s;' \"$?\"\n"
            "command ./blocked 2>/dev/null; printf '%s;' \"$?\"\n"
            "command ./tools/mytool >/dev/null; printf '%s;' \"$?\"\n"
            "command false; printf '%s\\n' \"$?\"\n"
        ),
        stdout="127;126;0;1\n",
    ),
    # ------------------------------------------------------------------
    # type
    # ------------------------------------------------------------------
    # [spec:posix:syn:builtin.type.synopsis/test]
    Case(
        id="type-synopsis",
        rules=("builtin.type.synopsis",),
        requires=("XSI",),
        script="type cd printf >/dev/null && printf 'accepted\\n'\n",
        stdout="accepted\n",
    ),
    # [spec:posix:req:builtin.type.indicate-interpretation/test]
    # [spec:posix:def:builtin.type.operand-name/test]
    # [spec:posix:sem:builtin.type.stdout/test]
    Case(
        id="type-indicate-interpretation",
        rules=(
            "builtin.type.indicate-interpretation",
            "builtin.type.operand-name",
            "builtin.type.stdout",
        ),
        files={"tools/mytool": TOOL},
        environment={"PATH": "{ROOT}/tools:/usr/bin:/bin"},
        script=(
            "myfunc() { :; }\n"
            "alias myalias='printf hi'\n"
            "type myfunc\n"
            "type myalias\n"
            "type cd\n"
            "type while\n"
            "type mytool\n"
        ),
        stdout=None,
        stdout_contains=(
            "myfunc",
            "myalias",
            "cd",
            "while",
            "{ROOT}/tools/mytool",
        ),
    ),
    # [spec:posix:sem:builtin.type.env-path/test]
    Case(
        id="type-env-path",
        rules=("builtin.type.env-path",),
        files={"tools/mytool": TOOL},
        script=(
            "PATH=$PWD/tools\n"
            "type mytool\n"
            "PATH=/nonexistent-directory\n"
            "type mytool >/dev/null 2>&1 || printf 'gone\\n'\n"
        ),
        stdout=None,
        stdout_contains=("{ROOT}/tools/mytool", "gone"),
    ),
    # [spec:posix:req:builtin.type.stderr/test]
    # [spec:posix:req:builtin.type.interfaces/test]
    Case(
        id="type-stderr-and-interfaces",
        rules=("builtin.type.stderr", "builtin.type.interfaces"),
        script=(
            "type cd >out 2>err\n"
            "[ -s out ] && printf 'stdout-used;'\n"
            "[ -s err ] || printf 'no-diag;'\n"
            "printf 'DATA\\n' | { type cd >/dev/null 2>&1; read line;"
            " printf 'read=[%s]\\n' \"$line\"; }\n"
        ),
        stdout="stdout-used;no-diag;read=[DATA]\n",
    ),
    # [spec:posix:req:builtin.type.exit-status/test]
    Case(
        id="type-exit-status",
        rules=("builtin.type.exit-status",),
        script=(
            "type cd >/dev/null && printf 'zero;'\n"
            "type nosuchname >/dev/null 2>&1 || printf 'nonzero\\n'\n"
        ),
        stdout="zero;nonzero\n",
    ),
    # ------------------------------------------------------------------
    # hash
    # ------------------------------------------------------------------
    # [spec:posix:syn:builtin.hash.synopsis/test]
    # [spec:posix:req:builtin.hash.remembered-locations/test]
    # [spec:posix:def:builtin.hash.operand-utility/test]
    # [spec:posix:req:builtin.hash.stdout-report/test]
    Case(
        id="hash-remembered-locations",
        rules=(
            "builtin.hash.synopsis",
            "builtin.hash.remembered-locations",
            "builtin.hash.operand-utility",
            "builtin.hash.stdout-report",
        ),
        files={"tools/mytool": TOOL},
        environment={"PATH": "{ROOT}/tools:/usr/bin:/bin"},
        script="hash mytool\nhash\n",
        stdout="{ROOT}/tools/mytool\n",
    ),
    # [spec:posix:syn:builtin.hash.synopsis/test]
    # [spec:posix:req:builtin.hash.opt-r/test]
    Case(
        id="hash-opt-r",
        rules=("builtin.hash.synopsis", "builtin.hash.opt-r"),
        files={"tools/mytool": TOOL},
        environment={"PATH": "{ROOT}/tools:/usr/bin:/bin"},
        script=(
            "hash mytool\n"
            "hash > before\n"
            "[ -s before ] && printf 'remembered;'\n"
            "hash -r\n"
            "hash > after\n"
            "[ -s after ] || printf 'forgotten\\n'\n"
        ),
        stdout="remembered;forgotten\n",
    ),
    # [spec:posix:req:builtin.hash.builtins-and-functions-not-reported/test]
    Case(
        id="hash-builtins-and-functions-not-reported",
        rules=("builtin.hash.builtins-and-functions-not-reported",),
        script=(
            "myfunc() { :; }\n"
            "myfunc\n"
            "hash cd 2>/dev/null\n"
            "hash > out\n"
            "[ -s out ] || printf 'nothing-reported\\n'\n"
        ),
        stdout="nothing-reported\n",
    ),
    # [spec:posix:req:builtin.hash.utility-syntax-guidelines/test]
    Case(
        id="hash-utility-syntax-guidelines",
        rules=("builtin.hash.utility-syntax-guidelines",),
        files={"tools/mytool": TOOL},
        environment={"PATH": "{ROOT}/tools:/usr/bin:/bin"},
        script="hash -- mytool\nhash\n",
        stdout="{ROOT}/tools/mytool\n",
    ),
    # [spec:posix:req:builtin.hash.list-cleared-on-path-change/test]
    Case(
        id="hash-list-cleared-on-path-change",
        rules=("builtin.hash.list-cleared-on-path-change",),
        files={"tools/mytool": TOOL},
        environment={"PATH": "{ROOT}/tools:/usr/bin:/bin"},
        script=(
            "hash mytool\n"
            "hash > before\n"
            "[ -s before ] && printf 'remembered;'\n"
            "PATH=/usr/bin:/bin:$PWD/tools\n"
            "hash > after\n"
            "[ -s after ] || printf 'cleared\\n'\n"
        ),
        stdout="remembered;cleared\n",
    ),
    # [spec:posix:sem:builtin.hash.env-path/test]
    Case(
        id="hash-env-path",
        rules=("builtin.hash.env-path",),
        files={"tools/mytool": TOOL},
        script=(
            "PATH=/nonexistent-directory\n"
            "hash mytool 2>/dev/null || printf 'not-found;'\n"
            "PATH=$PWD/tools\n"
            "hash mytool && printf 'found;'\n"
            "hash\n"
        ),
        stdout="not-found;found;{ROOT}/tools/mytool\n",
    ),
    # [spec:posix:req:builtin.hash.stderr/test]
    # [spec:posix:req:builtin.hash.interfaces/test]
    Case(
        id="hash-stderr-and-interfaces",
        rules=("builtin.hash.stderr", "builtin.hash.interfaces"),
        files={"tools/mytool": TOOL},
        environment={"PATH": "{ROOT}/tools:/usr/bin:/bin"},
        script=(
            "hash mytool >out 2>err\n"
            "[ -s out ] || printf 'no-stdout;'\n"
            "[ -s err ] || printf 'no-diag;'\n"
            "hash nosuchutility >out2 2>err2\n"
            "[ -s err2 ] && printf 'diag-on-stderr;'\n"
            "printf 'DATA\\n' | { hash >/dev/null; read line;"
            " printf 'read=[%s]\\n' \"$line\"; }\n"
        ),
        stdout="no-stdout;no-diag;diag-on-stderr;read=[DATA]\n",
    ),
    # [spec:posix:req:builtin.hash.exit-status/test]
    Case(
        id="hash-exit-status",
        rules=("builtin.hash.exit-status",),
        files={"tools/mytool": TOOL},
        environment={"PATH": "{ROOT}/tools:/usr/bin:/bin"},
        script=(
            "hash mytool && printf 'add=zero;'\n"
            "hash -r && printf 'clear=zero;'\n"
            "hash >/dev/null && printf 'report=zero\\n'\n"
        ),
        stdout="add=zero;clear=zero;report=zero\n",
    ),
)
