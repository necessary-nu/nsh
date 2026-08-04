"""Cases for rules previously dispositioned as non-executable in error.

An earlier pass excused a large block of rules as "headings", "grammar
productions" or "enumerations". That was wrong: a heading like "The
following operands shall be supported:" is an obligation on the shell, and
"The following variables shall affect the execution of sh: ..." is one
testable claim per variable. Anything that constrains what the shell does
is observable if the case is written properly. Only text about the
standard *document* is genuinely out of scope.
"""

from __future__ import annotations

from model import Case, FileFixture

CASES: tuple[Case, ...] = (
    # A pipeline/list/AND-OR/simple/compound command is not just a
    # definition -- the shell either accepts and executes the described
    # form or it does not.
    # [spec:posix:def:cmd.pipeline-definition/test]
    # [spec:posix:syn:cmd.pipeline-format/test]
    Case(
        id="rec-pipeline-form",
        rules=("cmd.pipeline-definition", "cmd.pipeline-format"),
        script="printf 'a\\nb\\n' | sort -r | tr -d '\\n'\n",
        stdout="ba",
    ),
    # [spec:posix:def:cmd.list-definition/test]
    # [spec:posix:def:cmd.and-or-list-definition/test]
    # [spec:posix:syn:cmd.and-list-format/test]
    # [spec:posix:syn:cmd.or-list-format/test]
    Case(
        id="rec-list-forms",
        rules=(
            "cmd.list-definition",
            "cmd.and-or-list-definition",
            "cmd.and-list-format",
            "cmd.or-list-format",
        ),
        script="true && echo A; false || echo B; echo C & wait\n",
        stdout="A\nB\nC\n",
    ),
    # [spec:posix:def:cmd.simple-definition/test]
    Case(
        id="rec-simple-command-form",
        rules=("cmd.simple-definition",),
        # "optional variable assignments and redirections, in any
        # sequence, optionally followed by words". Note $v is NOT used as
        # an argument here: an assignment prefix applies to the command's
        # environment, and argument expansion happens before it takes
        # effect, so "$v" would correctly expand to empty.
        script="> out 2>&1 v=1 printf '%s\\n' hello\ncat out\n",
        stdout="hello\n",
    ),
    # [spec:posix:def:cmd.compound-definition/test]
    # [spec:posix:def:cmd.compound-list-definition/test]
    Case(
        id="rec-compound-forms",
        rules=("cmd.compound-definition", "cmd.compound-list-definition"),
        script=(
            "{ echo brace; }\n(echo subshell)\n"
            "if true; then echo if; fi\n"
            "for i in f; do echo $i; done\n"
            "while false; do :; done; echo while\n"
            "until true; do :; done; echo until\n"
            "case c in c) echo case;; esac\n"
        ),
        stdout="brace\nsubshell\nif\nf\nwhile\nuntil\ncase\n",
    ),
    # [spec:posix:syn:cmd.if-format/test]
    # [spec:posix:syn:cmd.while-format/test]
    # [spec:posix:syn:cmd.until-format/test]
    # [spec:posix:syn:cmd.for-format/test]
    # [spec:posix:syn:cmd.case-format/test]
    # [spec:posix:syn:cmd.function-format/test]
    Case(
        id="rec-compound-formats-elif",
        rules=(
            "cmd.if-format",
            "cmd.while-format",
            "cmd.until-format",
            "cmd.for-format",
            "cmd.case-format",
            "cmd.function-format",
        ),
        # Exercise the full documented forms: elif/else, for over words,
        # case with multiple clauses, and fname() compound-command.
        script=(
            "if false; then echo x; elif true; then echo elif; else echo y; fi\n"
            "i=0; while [ $i -lt 2 ]; do i=$((i+1)); done; echo w$i\n"
            "j=0; until [ $j -ge 2 ]; do j=$((j+1)); done; echo u$j\n"
            "for k in a b; do printf '%s' $k; done; echo\n"
            "case z in a) echo no;; z) echo z;; *) echo star;; esac\n"
            "fn() ( echo fn )\nfn\n"
        ),
        stdout="elif\nw2\nu2\nab\nz\nfn\n",
    ),
    # [spec:posix:req:cmd.for-do-done-delimiters/test]
    Case(
        id="rec-for-requires-do-done",
        rules=("cmd.for-do-done-delimiters",),
        # "requires that the reserved words do and done be used to delimit"
        script="for i in a; echo $i; done\n",
        status="nonzero",
        stdout="",
        stderr_contains=("yntax",),
    ),
    # [spec:posix:def:cmd.command-kinds/test]
    Case(
        id="rec-command-kinds",
        rules=("cmd.command-kinds",),
        # simple, pipeline, list, compound, function definition -- all five.
        script=(
            "echo simple\n"
            "echo p | cat\n"
            "echo l1; echo l2\n"
            "{ echo compound; }\n"
            "g() { echo funcdef; }; g\n"
        ),
        stdout="simple\np\nl1\nl2\ncompound\nfuncdef\n",
    ),
    # [spec:posix:def:cmd.function-definition-term/test]
    Case(
        id="rec-function-new-positional",
        rules=("cmd.function-definition-term",),
        # "call a compound command with new positional parameters"
        script="f() { echo \"$#:$1\"; }\nset -- outer\nf inner extra\necho \"$#:$1\"\n",
        stdout="2:inner\n1:outer\n",
    ),
    # [spec:posix:def:builtin.cd.operand-directory/test]
    Case(
        id="rec-cd-directory-operand",
        rules=("builtin.cd.operand-directory",),
        # "The following operands shall be supported" -- cd must accept an
        # absolute and a relative directory pathname.
        script="mkdir -p sub/deep\ncd sub\nbasename \"$PWD\"\ncd /\necho \"$PWD\"\n",
        stdout="sub\n/\n",
    ),
    # [spec:posix:req:cmd.no-size-limit/test]
    Case(
        id="rec-no-command-size-limit",
        rules=("cmd.no-size-limit",),
        # Falsifiable: a shell with a fixed internal command buffer fails
        # well below the system's own limits. 64 KiB of script and a
        # single ~32 KiB word are far past any such buffer and far below
        # ARG_MAX (typically 2 MiB) since no exec is involved.
        script=(
            "v=$(printf '%032768d' 0)\n"
            "echo ${#v}\n"
            + "".join(f"x{n}=0\n" for n in range(4000))
            + "echo done\n"
        ),
        stdout="32768\ndone\n",
        timeout=20.0,
    ),
    # [spec:posix:def:exit.command-status/test]
    Case(
        id="rec-exit-command-status",
        rules=("exit.command-status",),
        # "Each command has an exit status that can influence the behavior
        # of other shell commands."
        script=(
            "true; echo $?\nfalse; echo $?\n"
            "false && echo unreachable\ntrue || echo unreachable\n"
            "if false; then echo no; else echo influenced; fi\n"
        ),
        stdout="0\n1\ninfluenced\n",
    ),
    # [spec:posix:def:shenv.components/test]
    Case(
        id="rec-shenv-components",
        rules=("shenv.components",),
        # open files, working directory, umask, traps, shell parameters,
        # shell functions and options all belong to the environment and
        # survive within it.
        script=(
            "exec 9>fd9\numask 0027\ncd /\nv=param\nsf() { echo fn; }\nset -f\n"
            "echo \"$PWD\"; umask; echo \"$v\"; sf; case $- in *f*) echo noglob;; esac\n"
            "exec 9>&-\n"
        ),
        stdout="/\n0027\nparam\nfn\nnoglob\n",
    ),
    # [spec:posix:sem:token.categorization/test]
    Case(
        id="rec-token-categorization",
        rules=("token.categorization",),
        # The same characters are categorized differently by position:
        # reserved word vs command name vs operand.
        script="for in in in; do echo \"$in\"; done\nif() { echo notreserved; }\n",
        status="nonzero",
        stdout="in\n",
        stderr_contains=("yntax",),
    ),
    # [spec:posix:req:expand.dollar-invalid-follower/test]
    Case(
        id="rec-dollar-invalid-follower",
        rules=("expand.dollar-invalid-follower",),
        # A '$' not followed by a valid expansion introducer is literal.
        script="printf '%s\\n' \"$ \" '$%' \"$$\" >/dev/null; printf '%s\\n' \"$ \" \"$%\"\n",
        stdout="$ \n$%\n",
    ),
    # [spec:posix:sem:expand.tilde-no-further-expansion/test]
    Case(
        id="rec-tilde-no-further-expansion",
        rules=("expand.tilde-no-further-expansion",),
        # The substituted home directory is not re-expanded: a '$' or '*'
        # inside HOME stays literal.
        script="HOME='/a$b*c'\nprintf '%s\\n' ~\n",
        stdout="/a$b*c\n",
    ),
    # [spec:posix:def:pattern.notation-purpose/test]
    # [spec:posix:def:pattern.filename-expansion-qualification/test]
    Case(
        id="rec-pattern-purpose",
        rules=("pattern.notation-purpose", "pattern.filename-expansion-qualification"),
        # The same notation matches strings in `case` and pathnames in
        # filename expansion, and the filename rules add the slash and
        # leading-period qualifications.
        script=(
            "case abc in a*) echo string-match;; esac\n"
            "touch .hidden vis\n"
            "printf '%s\\n' *\n"
        ),
        stdout="string-match\nvis\n",
    ),
    # [spec:posix:sem:cmd.async-job-control/test]
    Case(
        id="rec-async-job-controllable",
        rules=("cmd.async-job-control",),
        # "A job-control background job CAN BE CONTROLLED as described in
        # 2.11 Job Control" -- an assertion that the job is reachable
        # through the job-control interface, not just that it exists.
        # Needs a controlling terminal, hence interactive mode.
        mode="interactive",
        script=(
            "sleep 5 &\n"
            "jobs\n"
            "kill %1\n"
            "wait\n"
            "jobs\n"
            "echo controlled\n"
            "exit 0\n"
        ),
        stdout=None,
        stdout_contains=("sleep 5", "controlled"),
        status="any",
        timeout=15.0,
    ),
)
