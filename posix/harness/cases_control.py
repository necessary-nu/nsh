"""Executable cases for set, trap, the control-flow and variable special
built-ins, and the bg/fg/jobs intrinsics.

Covers posix/docs/spec/builtins-set-trap.md, builtins-control.md,
builtins-variables.md and builtins-jobs.md. Non-executable rules in those
four files are recorded in dispositions.d/control.json.

Interactive cases assert with ``stdout_contains`` and ``status="any"``:
a terminal has one output stream, and dash additionally emits
"Cannot set tty process group" when an interactive shell exits inside the
harness PID namespace, so neither the exact transcript nor the shell's own
wait status is a usable expectation there. Every status these cases care
about is therefore printed by the script itself.
"""

from __future__ import annotations

from model import Case, FileFixture


CASES: tuple[Case, ...] = (
    # ------------------------------------------------------------------
    # set
    # ------------------------------------------------------------------
    # [spec:posix:syn:builtin.set.synopsis/test]
    Case(
        id="set-synopsis-forms",
        rules=("builtin.set.synopsis",),
        script=(
            "set -a\n"
            "set +a\n"
            "set -o >/dev/null\n"
            "set +o >/dev/null\n"
            "set -o noglob\n"
            "set +o noglob\n"
            "set -- alpha beta\n"
            "printf '%s|%s|%s\\n' \"$#\" \"$1\" \"$2\"\n"
        ),
        stdout="2|alpha|beta\n",
    ),
    # [spec:posix:sem:builtin.set.options-and-arguments/test]
    # [spec:posix:req:builtin.set.utility-syntax-guidelines/test]
    Case(
        id="set-options-and-arguments",
        rules=(
            "builtin.set.options-and-arguments",
            "builtin.set.utility-syntax-guidelines",
        ),
        script=(
            "set -fu -- a b\n"
            "case $- in *f*) : ;; *) printf 'NO-F\\n' ;; esac\n"
            "case $- in *u*) : ;; *) printf 'NO-U\\n' ;; esac\n"
            "printf '%s|%s|%s\\n' \"$#\" \"$1\" \"$2\"\n"
            "set +fu\n"
            "case $- in\n"
            "  *[fu]*) printf 'NOT-CLEARED\\n' ;;\n"
            "  *) printf 'cleared\\n' ;;\n"
            "esac\n"
        ),
        stdout="2|a|b\ncleared\n",
    ),
    # [spec:posix:req:builtin.set.no-operands-writes-variables/test]
    # [spec:posix:req:builtin.set.variable-output-reinput/test]
    Case(
        id="set-no-operands-writes-variables",
        rules=(
            "builtin.set.no-operands-writes-variables",
            "builtin.set.variable-output-reinput",
        ),
        script=(
            "zz='a b'\n"
            "aa=1\n"
            "mm=\"it's here\"\n"
            "set >saved\n"
            "grep -E '^(aa|mm|zz)=' saved >mine\n"
            "cut -d= -f1 <mine | tr '\\n' ' '\n"
            "printf '\\n'\n"
            "unset aa mm zz\n"
            ". ./mine\n"
            "printf '[%s][%s][%s]\\n' \"$aa\" \"$mm\" \"$zz\"\n"
        ),
        stdout="aa mm zz \n[1][it's here][a b]\n",
    ),
    # [spec:posix:req:builtin.set.opt-a-allexport/test]
    Case(
        id="set-opt-a-allexport",
        rules=("builtin.set.opt-a-allexport",),
        script=(
            "set -a\n"
            "av=direct\n"
            "printf 'rvalue\\n' >readin\n"
            "read rv <readin\n"
            "set -- -x\n"
            "OPTIND=1\n"
            "getopts x ov\n"
            "sh -c 'printf \"%s|%s|%s\\n\" \"$av\" \"$rv\" \"$ov\"'\n"
        ),
        stdout="direct|rvalue|x\n",
    ),
    # [spec:posix:sem:builtin.set.opt-a-separate-environments/test]
    Case(
        id="set-opt-a-separate-environments",
        rules=("builtin.set.opt-a-separate-environments",),
        script=(
            "set -a\n"
            "fv=bar sh -c 'printf \"%s\\n\" \"$fv\"'\n"
            "printf '%s\\n' \"${fv-unset}\"\n"
        ),
        stdout="bar\nunset\n",
    ),
    # [spec:posix:req:builtin.set.opt-e-errexit/test]
    Case(
        id="set-opt-e-errexit",
        rules=("builtin.set.opt-e-errexit",),
        script=(
            "sh -c 'set -e; false; printf \"BAD\\n\"'\n"
            "printf 'exited=%s\\n' \"$?\"\n"
            "sh -c 'set -e; true | false; printf \"BAD\\n\"'\n"
            "printf 'pipeline=%s\\n' \"$?\"\n"
            "sh -c 'set -e; false | true; printf \"inner-failure-ignored\\n\"'\n"
            "sh -c 'set -e; if false; then :; fi; while false; do :; done;"
            " until true; do :; done; ! false; false || true;"
            " printf \"ignored-contexts\\n\"'\n"
            "sh -c 'set -e; { ! true; }; printf \"compound-exempt\\n\"'\n"
        ),
        stdout=(
            "exited=1\npipeline=1\ninner-failure-ignored\n"
            "ignored-contexts\ncompound-exempt\n"
        ),
    ),
    # [spec:posix:req:builtin.set.opt-e-per-environment/test]
    Case(
        id="set-opt-e-per-environment",
        rules=("builtin.set.opt-e-per-environment",),
        script=(
            "sh -c 'set -e; (false; printf \"one\\n\") | cat; printf \"two\\n\"'\n"
            "sh -c 'set -e; printf \"%s\\n\" $(false; printf \"one\\n\") two'\n"
        ),
        stdout="two\ntwo\n",
    ),
    # [spec:posix:req:builtin.set.opt-f-noglob/test]
    Case(
        id="set-opt-f-noglob",
        rules=("builtin.set.opt-f-noglob",),
        script=(
            ": >afile\n"
            ": >bfile\n"
            "set -f\n"
            "printf '%s\\n' *\n"
            "set +f\n"
            "printf '%s\\n' *\n"
        ),
        stdout="*\nafile\nbfile\n",
    ),
    # [spec:posix:req:builtin.set.opt-n-noexec/test]
    # [spec:posix:def:builtin.set.opt-o-noexec/test]
    Case(
        id="set-opt-n-noexec",
        rules=("builtin.set.opt-n-noexec", "builtin.set.opt-o-noexec"),
        script=(
            "sh -c 'set -n; printf \"BAD1\\n\"'\n"
            "printf 'letter=%s\\n' \"$?\"\n"
            "sh -c 'set -o noexec; printf \"BAD2\\n\"'\n"
            "printf 'longname=%s\\n' \"$?\"\n"
        ),
        stdout="letter=0\nlongname=0\n",
    ),
    # [spec:posix:req:builtin.set.opt-v-verbose/test]
    # [spec:posix:def:builtin.set.opt-o-verbose/test]
    Case(
        id="set-opt-v-verbose",
        rules=("builtin.set.opt-v-verbose", "builtin.set.opt-o-verbose"),
        script=(
            "sh <vscript >vout 2>verr\n"
            "cat vout\n"
            "grep -q 'ECHOED' verr && printf 'input-written-to-stderr\\n'\n"
            "grep -q 'QUIET' verr && printf 'STILL-VERBOSE\\n'\n"
            "sh <voscript >vout2 2>verr2\n"
            "cat vout2\n"
            "grep -q 'LONGNAME' verr2 && printf 'longname-equivalent\\n'\n"
        ),
        files={
            "vscript": FileFixture(
                "set -v\nprintf 'ECHOED\\n'\nset +v\nprintf 'QUIET\\n'\n"
            ),
            "voscript": FileFixture("set -o verbose\nprintf 'LONGNAME\\n'\n"),
        },
        stdout=(
            "ECHOED\nQUIET\ninput-written-to-stderr\n"
            "LONGNAME\nlongname-equivalent\n"
        ),
    ),
    # [spec:posix:req:builtin.set.opt-x-xtrace/test]
    # [spec:posix:def:builtin.set.opt-o-xtrace/test]
    Case(
        id="set-opt-x-xtrace",
        rules=("builtin.set.opt-x-xtrace", "builtin.set.opt-o-xtrace"),
        script=(
            "set -x\n"
            "echo one\n"
            "set +x\n"
            "set -o xtrace\n"
            "echo two\n"
            "set +o xtrace\n"
            "echo three\n"
        ),
        stdout="one\ntwo\nthree\n",
        stderr=None,
        stderr_contains=("echo one", "echo two"),
        stderr_excludes=("echo three",),
    ),
    # [spec:posix:sem:builtin.set.opt-o-report/test]
    Case(
        id="set-opt-o-report",
        rules=("builtin.set.opt-o-report",),
        script=(
            "set -o >oreport 2>oerr\n"
            "printf 'status=%s\\n' \"$?\"\n"
            "test -s oreport && printf 'wrote-standard-output\\n'\n"
            "test -s oerr && printf 'UNEXPECTED-STDERR\\n'\n"
            ":\n"
        ),
        stdout="status=0\nwrote-standard-output\n",
    ),
    # [spec:posix:sem:builtin.set.plus-o-report/test]
    Case(
        id="set-plus-o-report",
        rules=("builtin.set.plus-o-report",),
        script=(
            "set +o >saved\n"
            "set -f -u\n"
            ". ./saved\n"
            "case $- in\n"
            "  *[fu]*) printf 'NOT-RESTORED:%s\\n' \"$-\" ;;\n"
            "  *) printf 'restored\\n' ;;\n"
            "esac\n"
        ),
        stdout="restored\n",
    ),
    # [spec:posix:req:builtin.set.opt-o-option/test]
    # [spec:posix:def:builtin.set.opt-o-allexport/test]
    Case(
        id="set-opt-o-allexport",
        rules=("builtin.set.opt-o-option", "builtin.set.opt-o-allexport"),
        script=(
            "set -o allexport\n"
            "av=1\n"
            "sh -c 'printf \"%s\\n\" \"$av\"'\n"
        ),
        stdout="1\n",
    ),
    # [spec:posix:def:builtin.set.opt-o-errexit/test]
    Case(
        id="set-opt-o-errexit",
        rules=("builtin.set.opt-o-errexit",),
        script=(
            "sh -c 'set -o errexit; false; printf \"BAD\\n\"'\n"
            "printf 'status=%s\\n' \"$?\"\n"
        ),
        stdout="status=1\n",
    ),
    # [spec:posix:def:builtin.set.opt-o-noclobber/test]
    Case(
        id="set-opt-o-noclobber",
        rules=("builtin.set.opt-o-noclobber",),
        script=(
            "set -o noclobber\n"
            "case $- in *C*) printf 'long-on\\n';; esac\n"
            ": >nc\n"
            "if (: >nc) 2>/dev/null; then\n"
            "  printf 'BAD-OVERWRITE\\n'\n"
            "else\n"
            "  printf 'blocked\\n'\n"
            "fi\n"
            "set +C\n"
            ": >nc && printf 'short-off\\n'\n"
            "set -C\n"
            "set +o noclobber\n"
            ": >nc && printf 'long-off\\n'\n"
        ),
        stdout="long-on\nblocked\nshort-off\nlong-off\n",
    ),
    # [spec:posix:def:builtin.set.opt-o-noglob/test]
    Case(
        id="set-opt-o-noglob",
        rules=("builtin.set.opt-o-noglob",),
        script=(": >gfile\nset -o noglob\nprintf '%s\\n' *\n"),
        stdout="*\n",
    ),
    # [spec:posix:def:builtin.set.opt-o-nounset/test]
    Case(
        id="set-opt-o-nounset",
        rules=("builtin.set.opt-o-nounset",),
        script=(
            "if sh -c 'set -o nounset; printf \"%s\" \"${undefinedname}\";"
            " printf \"BAD\\n\"' 2>/dev/null; then\n"
            "  printf 'BAD-ZERO\\n'\n"
            "else\n"
            "  printf 'expansion-failed\\n'\n"
            "fi\n"
        ),
        stdout="expansion-failed\n",
    ),
    # [spec:posix:def:builtin.set.opt-o-notify/test]
    Case(
        id="set-opt-o-notify",
        rules=("builtin.set.opt-o-notify",),
        script=(
            "set -o notify\n"
            "case $- in *b*) printf 'b-set\\n' ;; *) printf 'B-UNSET\\n' ;; esac\n"
            "set +o notify\n"
            "case $- in *b*) printf 'B-STILL\\n' ;; *) printf 'b-cleared\\n' ;; esac\n"
        ),
        stdout="b-set\nb-cleared\n",
        requires=("UP",),
    ),
    # [spec:posix:req:builtin.set.opt-o-nolog/test]
    Case(
        id="set-opt-o-nolog",
        rules=("builtin.set.opt-o-nolog",),
        script=(
            "set -o nolog\n"
            "printf 'on=%s\\n' \"$?\"\n"
            "set +o nolog\n"
            "printf 'off=%s\\n' \"$?\"\n"
        ),
        stdout="on=0\noff=0\n",
        requires=("UP",),
    ),
    # [spec:posix:req:builtin.set.options-default-off/test]
    Case(
        id="set-options-default-off",
        rules=("builtin.set.options-default-off",),
        script=(
            "case $- in\n"
            "  *[abCefmnuvx]*) printf 'NOT-OFF:%s\\n' \"$-\" ;;\n"
            "  *) printf 'defaults-off\\n' ;;\n"
            "esac\n"
        ),
        stdout="defaults-off\n",
    ),
    # [spec:posix:req:builtin.set.stderr-diagnostics-only/test]
    # [spec:posix:req:builtin.set.exit-status/test]
    Case(
        id="set-stderr-and-exit-status",
        rules=(
            "builtin.set.stderr-diagnostics-only",
            "builtin.set.exit-status",
        ),
        script=(
            "set -f 2>err\n"
            "printf 'ok=%s\\n' \"$?\"\n"
            "test -s err && printf 'UNEXPECTED-STDERR\\n'\n"
            "set +f\n"
            "if (set -Z) 2>err2; then\n"
            "  printf 'BAD-ZERO\\n'\n"
            "else\n"
            "  printf 'invalid-option-nonzero\\n'\n"
            "fi\n"
            "test -s err2 || printf 'MISSING-DIAGNOSTIC\\n'\n"
        ),
        stdout="ok=0\ninvalid-option-nonzero\n",
    ),
    # [spec:posix:req:builtin.set.utility-defaults/test]
    Case(
        id="set-utility-defaults",
        rules=("builtin.set.utility-defaults",),
        script=(
            "printf 'from-stdin\\n' | { set -f -- a b; read line;"
            " printf '%s|%s\\n' \"$line\" \"$#\"; }\n"
        ),
        stdout="from-stdin|2\n",
    ),
    # A background job that finishes while a foreground command is still
    # running must not be reported until the foreground command completes
    # when -b is off: "Asynchronous notification shall not be enabled by
    # default."
    # [spec:posix:req:builtin.set.opt-b-notify/test]
    Case(
        id="set-notify-default-deferred",
        rules=("builtin.set.opt-b-notify",),
        script=(
            "(sleep 0.2) &\n"
            "sleep 1.0; printf 'LATER\\n'\n"
            "exit 0\n"
        ),
        mode="interactive",
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        status="any",
        stdout_contains=("LATER\n[1]",),
        timeout=15.0,
        requires=("UP",),
    ),
    # With -b the same completion must be written asynchronously, i.e.
    # before the foreground `sleep` finishes, not deferred to the prompt.
    # [spec:posix:req:builtin.set.opt-b-notify/test]
    Case(
        id="set-notify-async",
        rules=("builtin.set.opt-b-notify",),
        script=(
            "set -b\n"
            "(sleep 0.2) &\n"
            "sleep 1.0; printf 'LATER\\n'\n"
            "exit 0\n"
        ),
        mode="interactive",
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        status="any",
        stdout_contains=("(sleep 0.2)\nLATER",),
        timeout=15.0,
        requires=("UP",),
    ),
    # [spec:posix:req:builtin.set.opt-m-monitor/test]
    # [spec:posix:req:builtin.set.opt-o-monitor/test]
    Case(
        id="set-opt-m-monitor",
        rules=("builtin.set.opt-m-monitor", "builtin.set.opt-o-monitor"),
        script=(
            "case $- in\n"
            "  *m*) printf 'DEFAULT-ON\\n' ;;\n"
            "  *) printf 'DEFAULT-OFF\\n' ;;\n"
            "esac\n"
            "set -o monitor\n"
            "case $- in\n"
            "  *m*) printf 'LONGNAME-ON\\n' ;;\n"
            "  *) printf 'LONGNAME-OFF\\n' ;;\n"
            "esac\n"
            "exit 0\n"
        ),
        mode="interactive",
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        status="any",
        stdout_contains=("DEFAULT-ON\n", "LONGNAME-ON\n"),
        requires=("UP",),
    ),
    # [spec:posix:req:builtin.set.opt-o-vi/test]
    Case(
        id="set-opt-o-vi",
        rules=("builtin.set.opt-o-vi",),
        script=(
            "set -o vi\n"
            "set +o >opts\n"
            "grep -q '^set -o vi$' opts && printf 'VI-ENABLED\\n'\n"
            "if grep -q ' emacs$' opts; then\n"
            "  grep -q '^set +o emacs$' opts && printf 'OTHER-MODE-DISABLED\\n'\n"
            "else\n"
            "  printf 'OTHER-MODE-DISABLED\\n'\n"
            "fi\n"
            "exit 0\n"
        ),
        mode="interactive",
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        status="any",
        stdout_contains=("VI-ENABLED\n", "OTHER-MODE-DISABLED\n"),
        requires=("UP",),
    ),
    # ignoreeof must keep the interactive shell alive across end-of-file so
    # that the user can still type `exit`. Pacing makes every control-D arrive
    # at the start of a fresh input line; 51 of them also prove there is no
    # hidden retry cap after which the shell exits anyway.
    # [spec:posix:req:builtin.set.opt-o-ignoreeof/test]
    Case(
        id="set-opt-o-ignoreeof",
        rules=("builtin.set.opt-o-ignoreeof",),
        script=(
            "set -o ignoreeof\n"
            + "\x04\n" * 51
            + "printf 'STILL-ALIVE\\n'\n"
            + "exit 7\n"
        ),
        mode="interactive",
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        status="any",
        stdout_contains=("STILL-ALIVE\n",),
        timeout=15.0,
        requires=("UP",),
        pace=0.02,
    ),
    # The option is explicitly limited to interactive shells. Enabling it in
    # a script must not rearm a command file after its physical end-of-file,
    # even if the script also toggles dash's runtime `-i` extension.
    # [spec:posix:req:builtin.set.opt-o-ignoreeof/test]
    Case(
        id="set-opt-o-ignoreeof-non-interactive",
        rules=("builtin.set.opt-o-ignoreeof",),
        script="set -i\nset -o ignoreeof\nprintf 'DONE\\n'\n",
        mode="stdin",
        environment={"PS1": "", "PS2": ""},
        stdout="DONE\n",
        stderr="",
        timeout=1.0,
        requires=("UP",),
    ),
    # ------------------------------------------------------------------
    # trap
    # ------------------------------------------------------------------
    # [spec:posix:syn:builtin.trap.synopsis/test]
    Case(
        id="trapx-synopsis-forms",
        rules=("builtin.trap.synopsis",),
        script=(
            "trap 'printf \"action\\n\"' USR1\n"
            "kill -USR1 $$\n"
            "trap -p USR1 >pout 2>perr\n"
            "printf 'p-status=%s\\n' \"$?\"\n"
            "grep -q USR1 pout && printf 'p-lists-condition\\n'\n"
            "trap -p >/dev/null\n"
            "trap >/dev/null\n"
            "trap 0\n"
            "printf 'n-form=%s\\n' \"$?\"\n"
        ),
        stdout="action\np-status=0\np-lists-condition\nn-form=0\n",
    ),
    # [spec:posix:req:builtin.trap.utility-syntax-guidelines/test]
    Case(
        id="trapx-utility-syntax-guidelines",
        rules=("builtin.trap.utility-syntax-guidelines",),
        script=(
            "trap -- 'printf \"double-dash\\n\"' USR1\n"
            "kill -USR1 $$\n"
            "trap -- - USR1\n"
            "trap >lst\n"
            "grep -q USR1 lst && printf 'STILL-SET\\n'\n"
            "printf 'reset\\n'\n"
        ),
        stdout="double-dash\nreset\n",
    ),
    # [spec:posix:req:builtin.trap.operand-interpretation/test]
    # [spec:posix:def:builtin.trap.condition/test]
    Case(
        id="trapx-operand-interpretation",
        rules=("builtin.trap.operand-interpretation", "builtin.trap.condition"),
        script=(
            "trap 'printf \"UNRESET\\n\"' 0\n"
            "trap >l1\n"
            "grep -q EXIT l1 && printf 'zero-is-exit\\n'\n"
            "trap 'printf \"UNRESET\\n\"' USR1 QUIT\n"
            "trap 0 USR1 QUIT\n"
            "trap >l2\n"
            "if test -s l2; then\n"
            "  printf 'NOT-RESET:'\n"
            "  cat l2\n"
            "else\n"
            "  printf 'all-operands-are-conditions\\n'\n"
            "fi\n"
        ),
        stdout="zero-is-exit\nall-operands-are-conditions\n",
    ),
    # [spec:posix:req:builtin.trap.action-values/test]
    Case(
        id="trapx-action-values",
        rules=("builtin.trap.action-values",),
        script=(
            "trap 'printf \"caught\\n\"' USR1\n"
            "kill -USR1 $$\n"
            "trap '' USR1\n"
            "kill -USR1 $$\n"
            "printf 'ignored\\n'\n"
            "trap - USR1\n"
            "trap >lst\n"
            "grep -q USR1 lst && printf 'STILL-LISTED\\n'\n"
            "printf 'default-restored\\n'\n"
        ),
        stdout="caught\nignored\ndefault-restored\n",
    ),
    # [spec:posix:req:builtin.trap.persistence/test]
    Case(
        id="trapx-persistence",
        rules=("builtin.trap.persistence",),
        script=(
            "trap 'printf \"T\\n\"' USR1\n"
            "kill -USR1 $$\n"
            "f() { :; }\n"
            "f\n"
            "for i in 1 2; do :; done\n"
            "( : )\n"
            "kill -USR1 $$\n"
            ". ./dotfile\n"
            "kill -USR1 $$\n"
        ),
        files={"dotfile": FileFixture(":\n")},
        stdout="T\nT\nT\n",
    ),
    # [spec:posix:req:builtin.trap.subshell-reset/test]
    Case(
        id="trapx-subshell-reset",
        rules=("builtin.trap.subshell-reset",),
        script=(
            "trap 'printf \"E\\n\"' EXIT\n"
            "( printf 'in-subshell\\n' )\n"
            "printf 'after\\n'\n"
            "trap 'printf \"PARENT-TRAP\\n\"' USR1\n"
            "( sleep 0.6; printf 'SUB-SURVIVED\\n' ) &\n"
            "p=$!\n"
            "sleep 0.2\n"
            "kill -USR1 $p\n"
            "wait $p\n"
            "s=$?\n"
            "[ \"$s\" -gt 128 ] && printf 'reset-to-default\\n'\n"
            "trap '' USR2\n"
            "( sleep 0.6; printf 'ignore-kept\\n' ) &\n"
            "q=$!\n"
            "sleep 0.2\n"
            "kill -USR2 $q\n"
            "wait $q\n"
            "printf 'ignore-status=%s\\n' \"$?\"\n"
        ),
        stdout=(
            "in-subshell\nafter\nreset-to-default\n"
            "ignore-kept\nignore-status=0\nE\n"
        ),
        timeout=15.0,
    ),
    # [spec:posix:req:builtin.trap.exit-action-environment/test]
    Case(
        id="trapx-exit-action-environment",
        rules=("builtin.trap.exit-action-environment",),
        script=(
            "trap 'printf \"%s|%s|%s\\n\" \"$PWD\" \"$v\" \"$?\"' EXIT\n"
            "v=1\n"
            "mkdir sub\n"
            "cd sub\n"
            "v=2\n"
            "(exit 4)\n"
        ),
        stdout="{ROOT}/sub|2|4\n",
        # The rule fixes the action's environment, including `$?` while it
        # runs; it does not specify whether normal completion of the action
        # replaces the shell's final status.
        status="any",
    ),
    # [spec:posix:req:builtin.trap.signals-ignored-on-entry/test]
    Case(
        id="trapx-signals-ignored-on-entry",
        rules=("builtin.trap.signals-ignored-on-entry",),
        script=("trap '' USR1\nsh ./child.sh\nprintf 'status=%s\\n' \"$?\"\n"),
        files={
            "child.sh": FileFixture(
                "trap 'printf \"CAUGHT\\n\"' USR1\n"
                "kill -USR1 $$\n"
                "printf 'still-ignored\\n'\n"
            )
        },
        stdout="still-ignored\nstatus=0\n",
    ),
    # [spec:posix:req:builtin.trap.xsi-signal-numbers/test]
    Case(
        id="trapx-xsi-signal-numbers",
        rules=("builtin.trap.xsi-signal-numbers",),
        script=(
            "trap 'printf \"term\\n\"' 15\n"
            "kill -TERM $$\n"
            "trap 'printf \"int\\n\"' 2\n"
            "kill -INT $$\n"
            "trap - 15 2\n"
            "printf 'done\\n'\n"
        ),
        stdout="term\nint\ndone\n",
        requires=("XSI",),
    ),
    # [spec:posix:req:builtin.trap.invalid-condition-warning/test]
    # [spec:posix:req:builtin.trap.exit-status/test]
    # [spec:posix:req:builtin.trap.stderr-usage/test]
    Case(
        id="trapx-invalid-condition",
        rules=(
            "builtin.trap.invalid-condition-warning",
            "builtin.trap.exit-status",
            "builtin.trap.stderr-usage",
        ),
        script=(
            "if trap 'printf \"x\\n\"' NOSUCHSIG 2>err; then\n"
            "  printf 'BAD-ZERO\\n'\n"
            "else\n"
            "  printf 'invalid-nonzero\\n'\n"
            "fi\n"
            "test -s err && printf 'warning-on-stderr\\n'\n"
            "printf 'not-aborted\\n'\n"
            "trap 'printf \"y\\n\"' USR1 2>err2\n"
            "printf 'valid=%s\\n' \"$?\"\n"
            "test -s err2 && printf 'UNEXPECTED-STDERR\\n'\n"
            "trap - USR1\n"
        ),
        stdout="invalid-nonzero\nwarning-on-stderr\nnot-aborted\nvalid=0\n",
    ),
    # [spec:posix:req:builtin.trap.utility-defaults/test]
    Case(
        id="trapx-utility-defaults",
        rules=("builtin.trap.utility-defaults",),
        script=(
            "printf 'from-stdin\\n' | { trap 'printf \"T\\n\"' USR1;"
            " read line; printf '%s\\n' \"$line\"; }\n"
        ),
        stdout="from-stdin\n",
    ),
    # ------------------------------------------------------------------
    # 2.15 special built-in utilities
    # ------------------------------------------------------------------
    # [spec:posix:req:builtin.special.supported-and-output/test]
    Case(
        id="ctl-special-supported-and-output",
        rules=("builtin.special.supported-and-output",),
        script=(
            "for u in break : continue . eval exec exit export readonly return"
            " set shift times trap unset; do\n"
            "  command -v \"$u\" >/dev/null 2>&1 || printf 'MISSING:%s\\n' \"$u\"\n"
            "done\n"
            "printf 'all-present\\n'\n"
            "export ev=1\n"
            "export -p >o1\n"
            "grep -q '^export ev=' o1 && printf 'export-redirected\\n'\n"
            "readonly rv=1\n"
            "readonly -p | grep -q '^readonly rv=' && printf 'readonly-piped\\n'\n"
            "set >o2\n"
            "grep -q '^ev=' o2 && printf 'set-redirected\\n'\n"
            "trap 'printf \"t\\n\"' USR1\n"
            "trap >o3\n"
            "grep -q USR1 o3 && printf 'trap-redirected\\n'\n"
            "trap - USR1\n"
            "times >o4\n"
            "test -s o4 && printf 'times-redirected\\n'\n"
        ),
        stdout=(
            "all-present\nexport-redirected\nreadonly-piped\n"
            "set-redirected\ntrap-redirected\ntimes-redirected\n"
        ),
    ),
    # [spec:posix:req:builtin.special.error-may-abort-shell/test]
    Case(
        id="ctl-special-error-may-abort-shell",
        rules=("builtin.special.error-may-abort-shell",),
        script=(
            "if read -Z x </dev/null 2>/dev/null; then\n"
            "  printf 'BAD-REGULAR-ZERO\\n'\n"
            "else\n"
            "  printf 'regular-error-nonzero\\n'\n"
            "fi\n"
            "printf 'regular-did-not-abort\\n'\n"
            "if (set -Z) 2>/dev/null; then\n"
            "  printf 'BAD-SPECIAL-ZERO\\n'\n"
            "else\n"
            "  printf 'special-error-nonzero\\n'\n"
            "fi\n"
        ),
        stdout=(
            "regular-error-nonzero\nregular-did-not-abort\n"
            "special-error-nonzero\n"
        ),
    ),
    # ------------------------------------------------------------------
    # break / colon / continue
    # ------------------------------------------------------------------
    # [spec:posix:syn:builtin.break.syn/test]
    # [spec:posix:def:builtin.break.lexically-enclosing/test]
    Case(
        id="ctl-break-syn-and-lexical",
        rules=("builtin.break.syn", "builtin.break.lexically-enclosing"),
        script=(
            "for i in 1 2; do break; printf 'BAD1\\n'; done\n"
            "printf 'do-group\\n'\n"
            "for i in 1 2; do\n"
            "  for j in a b; do break 2; done\n"
            "  printf 'BAD2\\n'\n"
            "done\n"
            "printf 'nth-loop\\n'\n"
            "while break; do printf 'BAD3\\n'; done\n"
            "printf 'while-condition-list\\n'\n"
            "until break; do printf 'BAD4\\n'; done\n"
            "printf 'until-condition-list\\n'\n"
        ),
        stdout="do-group\nnth-loop\nwhile-condition-list\nuntil-condition-list\n",
    ),
    # [spec:posix:req:builtin.break.exit-status/test]
    # [spec:posix:req:builtin.break.stderr/test]
    # [spec:posix:req:builtin.break.interfaces/test]
    Case(
        id="ctl-break-status-and-io",
        rules=(
            "builtin.break.exit-status",
            "builtin.break.stderr",
            "builtin.break.interfaces",
        ),
        script=(
            "for i in 1; do break >out 2>err; done\n"
            "printf 'ok=%s\\n' \"$?\"\n"
            "test -s out && printf 'UNEXPECTED-STDOUT\\n'\n"
            "test -s err && printf 'UNEXPECTED-STDERR\\n'\n"
            "if (for i in 1; do break 0; done) 2>/dev/null; then\n"
            "  printf 'BAD-ZERO\\n'\n"
            "else\n"
            "  printf 'zero-n-nonzero\\n'\n"
            "fi\n"
            "if (for i in 1; do break x; done) 2>/dev/null; then\n"
            "  printf 'BAD-WORD\\n'\n"
            "else\n"
            "  printf 'non-integer-n-nonzero\\n'\n"
            "fi\n"
            "printf 'from-stdin\\n' | { for i in 1; do break; done;"
            " read line; printf '%s\\n' \"$line\"; }\n"
        ),
        stdout="ok=0\nzero-n-nonzero\nnon-integer-n-nonzero\nfrom-stdin\n",
    ),
    # [spec:posix:syn:builtin.colon.syn/test]
    # [spec:posix:req:builtin.colon.null-utility/test]
    # [spec:posix:req:builtin.colon.exit-status/test]
    # [spec:posix:req:builtin.colon.no-options/test]
    # [spec:posix:req:builtin.colon.interfaces/test]
    Case(
        id="ctl-colon",
        rules=(
            "builtin.colon.syn",
            "builtin.colon.null-utility",
            "builtin.colon.exit-status",
            "builtin.colon.no-options",
            "builtin.colon.interfaces",
        ),
        script=(
            "false\n"
            ":\n"
            "printf 'status=%s\\n' \"$?\"\n"
            ": a b c\n"
            "printf 'operands=%s\\n' \"$?\"\n"
            ": --\n"
            "printf 'double-dash=%s\\n' \"$?\"\n"
            ": -x\n"
            "printf 'option-like=%s\\n' \"$?\"\n"
            "case $- in *x*) printf 'OPTION-HONOURED\\n' ;; esac\n"
            ": >out 2>err\n"
            "test -s out && printf 'UNEXPECTED-STDOUT\\n'\n"
            "test -s err && printf 'UNEXPECTED-STDERR\\n'\n"
            "printf 'from-stdin\\n' | { :; read line; printf '%s\\n' \"$line\"; }\n"
        ),
        stdout=(
            "status=0\noperands=0\ndouble-dash=0\noption-like=0\nfrom-stdin\n"
        ),
    ),
    # [spec:posix:syn:builtin.continue.syn/test]
    # [spec:posix:req:builtin.continue.n-operand/test]
    Case(
        id="ctl-continue",
        rules=("builtin.continue.syn", "builtin.continue.n-operand"),
        script=(
            "for i in 1 2 3; do continue; printf 'BAD1\\n'; done\n"
            "printf 'plain\\n'\n"
            "for i in 1 2; do\n"
            "  for j in a b; do continue 2; printf 'BAD2\\n'; done\n"
            "  printf 'BAD3\\n'\n"
            "done\n"
            "printf 'nth-loop\\n'\n"
            "for i in 1 2; do\n"
            "  for j in a b; do continue 9; printf 'BAD4\\n'; done\n"
            "  printf 'BAD5\\n'\n"
            "done\n"
            "printf 'outermost-loop\\n'\n"
        ),
        stdout="plain\nnth-loop\noutermost-loop\n",
    ),
    # [spec:posix:req:builtin.continue.exit-status/test]
    # [spec:posix:req:builtin.continue.stderr/test]
    # [spec:posix:req:builtin.continue.interfaces/test]
    Case(
        id="ctl-continue-status-and-io",
        rules=(
            "builtin.continue.exit-status",
            "builtin.continue.stderr",
            "builtin.continue.interfaces",
        ),
        script=(
            "for i in 1; do continue >out 2>err; done\n"
            "printf 'ok=%s\\n' \"$?\"\n"
            "test -s out && printf 'UNEXPECTED-STDOUT\\n'\n"
            "test -s err && printf 'UNEXPECTED-STDERR\\n'\n"
            "if (for i in 1; do continue 0; done) 2>/dev/null; then\n"
            "  printf 'BAD-ZERO\\n'\n"
            "else\n"
            "  printf 'zero-n-nonzero\\n'\n"
            "fi\n"
            "printf 'from-stdin\\n' | { for i in 1; do continue; done;"
            " read line; printf '%s\\n' \"$line\"; }\n"
        ),
        stdout="ok=0\nzero-n-nonzero\nfrom-stdin\n",
    ),
    # ------------------------------------------------------------------
    # dot / eval
    # ------------------------------------------------------------------
    # [spec:posix:syn:builtin.dot.syn/test]
    # [spec:posix:req:builtin.dot.utility-syntax-guidelines/test]
    Case(
        id="ctl-dot-syn-and-guidelines",
        rules=("builtin.dot.syn", "builtin.dot.utility-syntax-guidelines"),
        script=(". ./dotted\n. -- ./dotted\n"),
        files={"dotted": FileFixture("printf 'sourced\\n'\n")},
        stdout="sourced\nsourced\n",
    ),
    # [spec:posix:req:builtin.dot.stderr/test]
    # [spec:posix:req:builtin.dot.interfaces/test]
    Case(
        id="ctl-dot-stderr-and-interfaces",
        rules=("builtin.dot.stderr", "builtin.dot.interfaces"),
        script=(
            ". ./emptyfile >out 2>err\n"
            "test -s out && printf 'UNEXPECTED-STDOUT\\n'\n"
            "test -s err && printf 'UNEXPECTED-STDERR\\n'\n"
            "( . ./missingfile ) >/dev/null 2>err2\n"
            "test -s err2 || printf 'MISSING-DIAGNOSTIC\\n'\n"
            "printf 'from-stdin\\n' | { . ./emptyfile; read line;"
            " printf '%s\\n' \"$line\"; }\n"
        ),
        files={"emptyfile": FileFixture("# nothing\n")},
        stdout="from-stdin\n",
    ),
    # [spec:posix:syn:builtin.eval.syn/test]
    # [spec:posix:req:builtin.eval.stderr/test]
    # [spec:posix:req:builtin.eval.interfaces/test]
    Case(
        id="ctl-eval-syn-and-io",
        rules=(
            "builtin.eval.syn",
            "builtin.eval.stderr",
            "builtin.eval.interfaces",
        ),
        script=(
            "eval\n"
            "printf 'no-arguments=%s\\n' \"$?\"\n"
            "eval '' ''\n"
            "printf 'null-arguments=%s\\n' \"$?\"\n"
            "eval \"printf 'evaluated\\n'\"\n"
            "eval ':' >out 2>err\n"
            "test -s out && printf 'UNEXPECTED-STDOUT\\n'\n"
            "test -s err && printf 'UNEXPECTED-STDERR\\n'\n"
            "printf 'from-stdin\\n' | { eval ':'; read line;"
            " printf '%s\\n' \"$line\"; }\n"
        ),
        stdout=(
            "no-arguments=0\nnull-arguments=0\nevaluated\nfrom-stdin\n"
        ),
    ),
    # ------------------------------------------------------------------
    # exec
    # ------------------------------------------------------------------
    # [spec:posix:syn:builtin.exec.syn/test]
    # [spec:posix:req:builtin.exec.no-operands-redirections/test]
    Case(
        id="ctl-exec-syn-and-redirections",
        rules=("builtin.exec.syn", "builtin.exec.no-operands-redirections"),
        script=(
            "exec 3>fd3\n"
            "printf 'via-fd3\\n' >&3\n"
            "exec 3>&-\n"
            "cat fd3\n"
            "( exec >inner; printf 'inner-redirected\\n' )\n"
            "cat inner\n"
        ),
        stdout="via-fd3\ninner-redirected\n",
    ),
    # [spec:posix:req:builtin.exec.failure-non-interactive-exits/test]
    Case(
        id="ctl-exec-failure-non-interactive",
        rules=("builtin.exec.failure-non-interactive-exits",),
        script=(
            "sh -c 'exec ./nosuchutility; printf \"BAD\\n\"' 2>/dev/null\n"
            "printf 'status=%s\\n' \"$?\"\n"
        ),
        stdout="status=127\n",
    ),
    # Guideline 10 / XCU 1.4: a special built-in described as conforming to
    # XBD 12.2 must accept "--" as a first argument and discard it.
    # [spec:posix:req:builtin.exec.utility-syntax-guidelines/test]
    Case(
        id="ctl-exec-utility-syntax-guidelines",
        rules=("builtin.exec.utility-syntax-guidelines",),
        script="exec -- printf 'guideline-ten\\n'\n",
        stdout="guideline-ten\n",
    ),
    # [spec:posix:req:builtin.exec.env-path/test]
    Case(
        id="ctl-exec-env-path",
        rules=("builtin.exec.env-path",),
        script=(
            "( PATH=$PWD/bin1 exec mytool )\n"
            "printf 'status=%s\\n' \"$?\"\n"
        ),
        files={
            "bin1/mytool": FileFixture(
                "#!/bin/sh\nprintf 'found-on-path\\n'\n", 0o755
            )
        },
        stdout="found-on-path\nstatus=0\n",
    ),
    # [spec:posix:req:builtin.exec.stderr/test]
    # [spec:posix:req:builtin.exec.interfaces/test]
    Case(
        id="ctl-exec-stderr-and-interfaces",
        rules=("builtin.exec.stderr", "builtin.exec.interfaces"),
        script=(
            "( exec 3>fd3 2>err; exec 3>&- )\n"
            "test -s err && printf 'UNEXPECTED-STDERR\\n'\n"
            "( exec ./nosuchutility ) >/dev/null 2>err2\n"
            "test -s err2 || printf 'MISSING-DIAGNOSTIC\\n'\n"
            "printf 'from-stdin\\n' | { exec 3>fd4; exec 3>&-; read line;"
            " printf '%s\\n' \"$line\"; }\n"
            "printf 'ok\\n'\n"
        ),
        stdout="from-stdin\nok\n",
    ),
    # An interactive shell whose current environment is not a subshell must
    # survive a failed exec, and the redirections that were made must stay.
    # [spec:posix:req:builtin.exec.failure-interactive-up/test]
    Case(
        id="ctl-exec-failure-interactive",
        rules=("builtin.exec.failure-interactive-up",),
        script=(
            "set -o emacs\n"
            "exec 3>fd3 ./nosuchutility\n"
            "printf 'STILL-ALIVE\\n'\n"
            "printf 'via-fd3\\n' >&3\n"
            "exec 3>&-\n"
            "cat fd3\n"
            "exit 0\n"
        ),
        mode="interactive",
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        status="any",
        stdout_contains=("STILL-ALIVE\n", "via-fd3\n"),
        requires=("UP",),
    ),
    # ------------------------------------------------------------------
    # exit
    # ------------------------------------------------------------------
    # [spec:posix:syn:builtin.exit.syn/test]
    # [spec:posix:req:builtin.exit.default-n/test]
    # [spec:posix:sem:builtin.exit.exit-status/test]
    Case(
        id="ctl-exit-syn-and-default-n",
        rules=(
            "builtin.exit.syn",
            "builtin.exit.default-n",
            "builtin.exit.exit-status",
        ),
        script=(
            "sh -c 'exit'\n"
            "printf 'bare=%s\\n' \"$?\"\n"
            "sh -c 'false; exit'\n"
            "printf 'inherited=%s\\n' \"$?\"\n"
            "sh -c 'exit 5'\n"
            "printf 'operand=%s\\n' \"$?\"\n"
            "sh -c 'trap \"false; exit\" EXIT; true'\n"
            "printf 'trap-preceding=%s\\n' \"$?\"\n"
            "sh -c 'exit 3; printf \"BAD\\n\"'\n"
            "printf 'does-not-return=%s\\n' \"$?\"\n"
        ),
        stdout=(
            "bare=0\ninherited=1\noperand=5\ntrap-preceding=0\n"
            "does-not-return=3\n"
        ),
    ),
    # [spec:posix:req:builtin.exit.stderr/test]
    # [spec:posix:req:builtin.exit.interfaces/test]
    Case(
        id="ctl-exit-stderr-and-interfaces",
        rules=("builtin.exit.stderr", "builtin.exit.interfaces"),
        script=(
            "( exit 0 ) >out 2>err\n"
            "test -s out && printf 'UNEXPECTED-STDOUT\\n'\n"
            "test -s err && printf 'UNEXPECTED-STDERR\\n'\n"
            "printf 'from-stdin\\n' | { ( exit 0 ); read line;"
            " printf '%s\\n' \"$line\"; }\n"
        ),
        stdout="from-stdin\n",
    ),
    # ------------------------------------------------------------------
    # export / readonly
    # ------------------------------------------------------------------
    # [spec:posix:syn:builtin.export.synopsis/test]
    # [spec:posix:req:builtin.export.set-attribute/test]
    Case(
        id="var-export-synopsis-and-attribute",
        rules=("builtin.export.synopsis", "builtin.export.set-attribute"),
        script=(
            "export ev=one\n"
            "sh -c 'printf \"%s\\n\" \"$ev\"'\n"
            "ev2=two\n"
            "export ev2\n"
            "sh -c 'printf \"%s\\n\" \"$ev2\"'\n"
            "export -p >/dev/null\n"
            "printf 'p-form=%s\\n' \"$?\"\n"
        ),
        stdout="one\ntwo\np-form=0\n",
    ),
    # [spec:posix:req:builtin.export.declaration-utility/test]
    Case(
        id="var-export-declaration-utility",
        rules=("builtin.export.declaration-utility",),
        script=(
            "v='a b'\n"
            "export x=$v\n"
            "printf '[%s]\\n' \"$x\"\n"
            ": >aglob\n"
            "p='*'\n"
            "export y=$p\n"
            "printf '[%s]\\n' \"$y\"\n"
            "HOME=/hh\n"
            "export z=~\n"
            "printf '[%s]\\n' \"$z\"\n"
        ),
        stdout="[a b]\n[*]\n[/hh]\n",
    ),
    # [spec:posix:req:builtin.export.utility-syntax-guidelines/test]
    Case(
        id="var-export-utility-syntax-guidelines",
        rules=("builtin.export.utility-syntax-guidelines",),
        script=("export -- gv=1\nsh -c 'printf \"%s\\n\" \"$gv\"'\n"),
        stdout="1\n",
    ),
    # [spec:posix:req:builtin.export.stderr/test]
    # [spec:posix:req:builtin.export.exit-status/test]
    Case(
        id="var-export-stderr-and-exit-status",
        rules=("builtin.export.stderr", "builtin.export.exit-status"),
        script=(
            "export ok1=1 2>err\n"
            "printf 'ok=%s\\n' \"$?\"\n"
            "test -s err && printf 'UNEXPECTED-STDERR\\n'\n"
            "readonly ro=1\n"
            "if (export ro=2) 2>err2; then\n"
            "  printf 'BAD-ZERO\\n'\n"
            "else\n"
            "  printf 'readonly-nonzero\\n'\n"
            "fi\n"
            "test -s err2 || printf 'MISSING-DIAGNOSTIC\\n'\n"
        ),
        stdout="ok=0\nreadonly-nonzero\n",
    ),
    # [spec:posix:sem:builtin.export.utility-defaults/test]
    Case(
        id="var-export-utility-defaults",
        rules=("builtin.export.utility-defaults",),
        script=(
            "printf 'from-stdin\\n' | { export uv=1; read line;"
            " printf '%s\\n' \"$line\"; }\n"
        ),
        stdout="from-stdin\n",
    ),
    # [spec:posix:syn:builtin.readonly.synopsis/test]
    # [spec:posix:def:builtin.readonly.attribute/test]
    Case(
        id="var-readonly-synopsis-and-attribute",
        rules=("builtin.readonly.synopsis", "builtin.readonly.attribute"),
        script=(
            "readonly rv=1\n"
            "readonly -p >/dev/null\n"
            "printf 'p-form=%s\\n' \"$?\"\n"
            "if (rv=2) 2>/dev/null; then printf 'BAD-ASSIGN\\n';"
            " else printf 'assign-blocked\\n'; fi\n"
            "if (export rv=3) 2>/dev/null; then printf 'BAD-EXPORT\\n';"
            " else printf 'export-blocked\\n'; fi\n"
            "if (readonly rv=4) 2>/dev/null; then printf 'BAD-READONLY\\n';"
            " else printf 'readonly-blocked\\n'; fi\n"
            "printf 'x\\n' >inp\n"
            "if (read rv <inp) 2>/dev/null; then printf 'BAD-READ\\n';"
            " else printf 'read-blocked\\n'; fi\n"
            "if (set -- -a; OPTIND=1; getopts a rv) 2>/dev/null; then"
            " printf 'BAD-GETOPTS\\n'; else printf 'getopts-blocked\\n'; fi\n"
            "if (unset rv) 2>/dev/null; then printf 'BAD-UNSET\\n';"
            " else printf 'unset-blocked\\n'; fi\n"
        ),
        stdout=(
            "p-form=0\nassign-blocked\nexport-blocked\nreadonly-blocked\n"
            "read-blocked\ngetopts-blocked\nunset-blocked\n"
        ),
    ),
    # [spec:posix:req:builtin.readonly.declaration-utility/test]
    # [spec:posix:req:builtin.readonly.utility-syntax-guidelines/test]
    Case(
        id="var-readonly-declaration-and-guidelines",
        rules=(
            "builtin.readonly.declaration-utility",
            "builtin.readonly.utility-syntax-guidelines",
        ),
        script=("v='a b'\nreadonly -- x=$v\nprintf '[%s]\\n' \"$x\"\n"),
        stdout="[a b]\n",
    ),
    # [spec:posix:sem:builtin.readonly.p-output-format/test]
    Case(
        id="var-readonly-p-output-format",
        rules=("builtin.readonly.p-output-format",),
        script=(
            "readonly rset='a b'\n"
            "readonly runset\n"
            "readonly -p >out\n"
            "grep -q '^readonly rset=' out && printf 'set-form\\n'\n"
            "grep -q '^readonly runset$' out && printf 'unset-form\\n'\n"
        ),
        stdout="set-form\nunset-form\n",
    ),
    # [spec:posix:req:builtin.readonly.stderr/test]
    # [spec:posix:req:builtin.readonly.exit-status/test]
    # [spec:posix:sem:builtin.readonly.utility-defaults/test]
    Case(
        id="var-readonly-stderr-status-defaults",
        rules=(
            "builtin.readonly.stderr",
            "builtin.readonly.exit-status",
            "builtin.readonly.utility-defaults",
        ),
        script=(
            "readonly ra=1 2>err\n"
            "printf 'ok=%s\\n' \"$?\"\n"
            "test -s err && printf 'UNEXPECTED-STDERR\\n'\n"
            "if (readonly ra=2) 2>err2; then\n"
            "  printf 'BAD-ZERO\\n'\n"
            "else\n"
            "  printf 'modify-nonzero\\n'\n"
            "fi\n"
            "test -s err2 || printf 'MISSING-DIAGNOSTIC\\n'\n"
            "printf 'from-stdin\\n' | { readonly rb=1; read line;"
            " printf '%s\\n' \"$line\"; }\n"
        ),
        stdout="ok=0\nmodify-nonzero\nfrom-stdin\n",
    ),
    # ------------------------------------------------------------------
    # return / shift / times / unset
    # ------------------------------------------------------------------
    # [spec:posix:syn:builtin.return.synopsis/test]
    # [spec:posix:req:builtin.return.stderr/test]
    # [spec:posix:sem:builtin.return.utility-defaults/test]
    Case(
        id="var-return-synopsis-stderr-defaults",
        rules=(
            "builtin.return.synopsis",
            "builtin.return.stderr",
            "builtin.return.utility-defaults",
        ),
        script=(
            "f() { return; }\n"
            "f\n"
            "printf 'bare=%s\\n' \"$?\"\n"
            "g() { return 5; }\n"
            "g >out 2>err\n"
            "printf 'operand=%s\\n' \"$?\"\n"
            "test -s out && printf 'UNEXPECTED-STDOUT\\n'\n"
            "test -s err && printf 'UNEXPECTED-STDERR\\n'\n"
            "printf 'from-stdin\\n' | { h() { return 0; }; h; read line;"
            " printf '%s\\n' \"$line\"; }\n"
        ),
        stdout="bare=0\noperand=5\nfrom-stdin\n",
    ),
    # [spec:posix:syn:builtin.shift.synopsis/test]
    # [spec:posix:req:builtin.shift.stderr/test]
    # [spec:posix:sem:builtin.shift.utility-defaults/test]
    Case(
        id="var-shift-synopsis-stderr-defaults",
        rules=(
            "builtin.shift.synopsis",
            "builtin.shift.stderr",
            "builtin.shift.utility-defaults",
        ),
        script=(
            "set -- a b c\n"
            "shift >out 2>err\n"
            "printf 'bare=%s|%s\\n' \"$#\" \"$1\"\n"
            "test -s out && printf 'UNEXPECTED-STDOUT\\n'\n"
            "test -s err && printf 'UNEXPECTED-STDERR\\n'\n"
            "shift 2\n"
            "printf 'operand=%s\\n' \"$#\"\n"
            "printf 'from-stdin\\n' | { set -- x; shift; read line;"
            " printf '%s\\n' \"$line\"; }\n"
        ),
        stdout="bare=2|b\noperand=0\nfrom-stdin\n",
    ),
    # [spec:posix:syn:builtin.times.synopsis/test]
    # [spec:posix:req:builtin.times.output-format/test]
    # [spec:posix:req:builtin.times.exit-status/test]
    # [spec:posix:req:builtin.times.stderr/test]
    # [spec:posix:sem:builtin.times.utility-defaults/test]
    Case(
        id="var-times-format",
        rules=(
            "builtin.times.synopsis",
            "builtin.times.output-format",
            "builtin.times.exit-status",
            "builtin.times.stderr",
            "builtin.times.utility-defaults",
        ),
        script=(
            "times >out 2>err\n"
            "printf 'status=%s\\n' \"$?\"\n"
            "test -s err && printf 'UNEXPECTED-STDERR\\n'\n"
            "n=0\n"
            "while read -r a b extra; do\n"
            "  n=$((n+1))\n"
            "  case $a in\n"
            "    [0-9]*m[0-9]*.[0-9]*s) : ;;\n"
            "    *) printf 'BAD-FIELD:%s\\n' \"$a\" ;;\n"
            "  esac\n"
            "  case $b in\n"
            "    [0-9]*m[0-9]*.[0-9]*s) : ;;\n"
            "    *) printf 'BAD-FIELD:%s\\n' \"$b\" ;;\n"
            "  esac\n"
            "  test -n \"$extra\" && printf 'EXTRA-FIELD:%s\\n' \"$extra\"\n"
            "done <out\n"
            "printf 'lines=%s\\n' \"$n\"\n"
            "printf 'from-stdin\\n' | { times >/dev/null; read line;"
            " printf '%s\\n' \"$line\"; }\n"
        ),
        stdout="status=0\nlines=2\nfrom-stdin\n",
    ),
    # [spec:posix:req:builtin.times.tms-correspondence/test]
    Case(
        id="var-times-tms-correspondence",
        rules=("builtin.times.tms-correspondence",),
        script=(
            "times >t1\n"
            "{ read -r u1 s1; read -r cu1 cs1; } <t1\n"
            "awk 'BEGIN { x = 0; for (i = 0; i < 3000000; i++) x += i }'\n"
            "times >t2\n"
            "{ read -r u2 s2; read -r cu2 cs2; } <t2\n"
            "if [ \"$cu1\" = \"$cu2\" ]; then\n"
            "  printf 'CHILD-USER-UNCHANGED\\n'\n"
            "else\n"
            "  printf 'children-user-grew\\n'\n"
            "fi\n"
            "i=0\n"
            "while [ \"$i\" -lt 60000 ]; do i=$((i+1)); done\n"
            "times >t3\n"
            "{ read -r u3 s3; read -r cu3 cs3; } <t3\n"
            "if [ \"$u2\" = \"$u3\" ]; then\n"
            "  printf 'SHELL-USER-UNCHANGED\\n'\n"
            "else\n"
            "  printf 'shell-user-grew\\n'\n"
            "fi\n"
        ),
        stdout="children-user-grew\nshell-user-grew\n",
        timeout=30.0,
    ),
    # [spec:posix:syn:builtin.unset.synopsis/test]
    # [spec:posix:req:builtin.unset.unset-names/test]
    # [spec:posix:req:builtin.unset.no-option/test]
    Case(
        id="var-unset-synopsis-and-names",
        rules=(
            "builtin.unset.synopsis",
            "builtin.unset.unset-names",
            "builtin.unset.no-option",
        ),
        script=(
            "export uv=1\n"
            "unset uv\n"
            "sh -c 'printf \"[%s]\\n\" \"${uv-gone}\"'\n"
            "uv=2\n"
            "sh -c 'printf \"[%s]\\n\" \"${uv-gone}\"'\n"
            "n() { printf 'function-kept\\n'; }\n"
            "n=varvalue\n"
            "unset n\n"
            "printf '[%s]\\n' \"${n-gone}\"\n"
            "n\n"
            "unset -v uv\n"
            "unset -f n\n"
            "printf 'ok\\n'\n"
        ),
        stdout="[gone]\n[gone]\n[gone]\nfunction-kept\nok\n",
    ),
    # [spec:posix:req:builtin.unset.utility-syntax-guidelines/test]
    # [spec:posix:sem:builtin.unset.empty-assignment-and-special-parameters/test]
    # [spec:posix:req:builtin.unset.stderr/test]
    # [spec:posix:sem:builtin.unset.utility-defaults/test]
    Case(
        id="var-unset-guidelines-and-io",
        rules=(
            "builtin.unset.utility-syntax-guidelines",
            "builtin.unset.empty-assignment-and-special-parameters",
            "builtin.unset.stderr",
            "builtin.unset.utility-defaults",
        ),
        script=(
            "ev=\n"
            "printf 'empty-assignment=[%s]\\n' \"${ev+SET}\"\n"
            "unset -- ev\n"
            "printf 'after-unset=[%s]\\n' \"${ev+SET}\"\n"
            "unset -- neverset >out 2>err\n"
            "test -s out && printf 'UNEXPECTED-STDOUT\\n'\n"
            "test -s err && printf 'UNEXPECTED-STDERR\\n'\n"
            "printf 'from-stdin\\n' | { unset zz; read line;"
            " printf '%s\\n' \"$line\"; }\n"
        ),
        stdout="empty-assignment=[SET]\nafter-unset=[]\nfrom-stdin\n",
    ),
    # [spec:posix:req:builtin.unset.exit-status/test]
    Case(
        id="var-unset-exit-status",
        rules=("builtin.unset.exit-status",),
        script=(
            "uu=1\n"
            "unset uu\n"
            "printf 'ok=%s\\n' \"$?\"\n"
            "readonly rr=1\n"
            "if (unset rr) 2>/dev/null; then\n"
            "  printf 'BAD-ZERO\\n'\n"
            "else\n"
            "  printf 'readonly-nonzero\\n'\n"
            "fi\n"
        ),
        stdout="ok=0\nreadonly-nonzero\n",
    ),
    # ------------------------------------------------------------------
    # jobs
    # ------------------------------------------------------------------
    # [spec:posix:syn:builtin.jobs.synopsis/test]
    # [spec:posix:req:builtin.jobs.operand-job-id/test]
    # [spec:posix:req:builtin.jobs.utility-syntax-guidelines/test]
    # [spec:posix:req:builtin.jobs.default-display/test]
    # [spec:posix:req:builtin.jobs.display-background-jobs/test]
    Case(
        id="job-jobs-synopsis-and-operands",
        rules=(
            "builtin.jobs.synopsis",
            "builtin.jobs.operand-job-id",
            "builtin.jobs.utility-syntax-guidelines",
            "builtin.jobs.default-display",
            "builtin.jobs.display-background-jobs",
        ),
        script=(
            "sleep 3 &\n"
            "sleep 4 &\n"
            "sleep 0.3\n"
            "jobs %1 >j1\n"
            "jobs >jall\n"
            "jobs -- %2 >j2\n"
            "grep -q 'sleep 3' j1 && printf 'operand-selects\\n'\n"
            "printf 'all=%s\\n' \"$(grep -c . jall)\"\n"
            "grep -q 'sleep 4' j2 && printf 'double-dash\\n'\n"
            "exit 0\n"
        ),
        mode="interactive",
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        status="any",
        stdout_contains=("operand-selects\n", "all=2\n", "double-dash\n"),
        timeout=15.0,
        requires=("UP",),
    ),
    # [spec:posix:req:builtin.jobs.stdout-default-format/test]
    # [spec:posix:req:builtin.jobs.stdout-current-field/test]
    Case(
        id="job-jobs-default-format",
        rules=(
            "builtin.jobs.stdout-default-format",
            "builtin.jobs.stdout-current-field",
        ),
        script=(
            "sleep 3 &\n"
            "sleep 4 &\n"
            "sleep 0.3\n"
            "jobs >jd\n"
            "while read -r l; do\n"
            "  case $l in\n"
            "    '[1] - Running'*'sleep 3') printf 'previous-ok\\n' ;;\n"
            "    '[2] + Running'*'sleep 4') printf 'current-ok\\n' ;;\n"
            "    *) printf 'BAD-LINE:%s\\n' \"$l\" ;;\n"
            "  esac\n"
            "done <jd\n"
            "exit 0\n"
        ),
        mode="interactive",
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        status="any",
        stdout_contains=("current-ok\n", "previous-ok\n"),
        timeout=15.0,
        requires=("UP",),
    ),
    # [spec:posix:req:builtin.jobs.option-l/test]
    # [spec:posix:req:builtin.jobs.stdout-l-format/test]
    # [spec:posix:req:builtin.jobs.option-p/test]
    # [spec:posix:req:builtin.jobs.stdout-p-format/test]
    Case(
        id="job-jobs-l-and-p-formats",
        rules=(
            "builtin.jobs.option-l",
            "builtin.jobs.stdout-l-format",
            "builtin.jobs.option-p",
            "builtin.jobs.stdout-p-format",
        ),
        script=(
            "sleep 3 &\n"
            "sleep 4 &\n"
            "sleep 0.3\n"
            "jobs -l >jl\n"
            "jobs -p >jp\n"
            "lok=0\n"
            "while read -r l; do\n"
            "  case $l in\n"
            "    '['[0-9]'] '?' '[0-9]*' Running'*) lok=$((lok+1)) ;;\n"
            "    *) printf 'BAD-L-LINE:%s\\n' \"$l\" ;;\n"
            "  esac\n"
            "done <jl\n"
            "printf 'l-lines=%s\\n' \"$lok\"\n"
            "pok=0\n"
            "while read -r l; do\n"
            "  pok=$((pok+1))\n"
            "  case $l in *[!0-9]*) printf 'BAD-P-LINE:%s\\n' \"$l\" ;; esac\n"
            "done <jp\n"
            "printf 'p-lines=%s\\n' \"$pok\"\n"
            "exit 0\n"
        ),
        mode="interactive",
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        status="any",
        stdout_contains=("l-lines=2\n", "p-lines=2\n"),
        timeout=15.0,
        requires=("UP",),
    ),
    # A job suspended by SIGSTOP must be shown with one of the <state>
    # strings the standard lists.
    # [spec:posix:def:builtin.jobs.stdout-state-strings/test]
    Case(
        id="job-jobs-state-stopped",
        rules=("builtin.jobs.stdout-state-strings",),
        script=(
            "sleep 5 &\n"
            "sleep 0.3\n"
            "kill -STOP %1\n"
            "sleep 0.5\n"
            "jobs >js\n"
            "read -r line <js\n"
            "body=${line#*'] '}\n"
            "body=${body#? }\n"
            "state=${body%'sleep 5'}\n"
            "while [ \"${state% }\" != \"$state\" ]; do state=${state% }; done\n"
            "case $state in\n"
            "  Stopped|'Stopped (SIGSTOP)'|Suspended|'Suspended (SIGSTOP)')\n"
            "    printf 'stopped-state-ok\\n' ;;\n"
            "  *) printf 'BAD-STATE:[%s]\\n' \"$state\" ;;\n"
            "esac\n"
            "exit 0\n"
        ),
        mode="interactive",
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        status="any",
        stdout_contains=("stopped-state-ok\n",),
        timeout=15.0,
        requires=("UP",),
    ),
    # [spec:posix:def:builtin.jobs.stdout-state-strings/test]
    Case(
        id="job-jobs-state-done",
        rules=("builtin.jobs.stdout-state-strings",),
        script=(
            "set -m\n"
            "(exit 0) &\n"
            "sleep 0.3\n"
            "jobs >j0\n"
            "(exit 7) &\n"
            "sleep 0.3\n"
            "jobs >j7\n"
            "grep -q 'Done' j0 && printf 'done-state\\n'\n"
            "grep -q 'Done(7)' j7 && printf 'done-code-state\\n'\n"
        ),
        stdout="done-state\ndone-code-state\n",
        timeout=15.0,
    ),
    # [spec:posix:req:builtin.jobs.stdout-state-substitution/test]
    Case(
        id="job-jobs-state-terminated",
        rules=("builtin.jobs.stdout-state-substitution",),
        script=(
            "set -m\n"
            "sleep 5 &\n"
            "sleep 0.3\n"
            "kill -TERM %1\n"
            "sleep 0.5\n"
            "jobs >jt\n"
            "read -r line <jt\n"
            "body=${line#*'] '}\n"
            "body=${body#? }\n"
            "state=${body%'sleep 5'}\n"
            "while [ \"${state% }\" != \"$state\" ]; do state=${state% }; done\n"
            "case $state in\n"
            "  Running|Done|'Done('*|Stopped*|Suspended*)\n"
            "    printf 'NOT-DISTINCT:[%s]\\n' \"$state\" ;;\n"
            "  *TERM*|*erminat*) printf 'signal-state-ok\\n' ;;\n"
            "  *) printf 'BAD-STATE:[%s]\\n' \"$state\" ;;\n"
            "esac\n"
        ),
        stdout="signal-state-ok\n",
        timeout=15.0,
    ),
    # [spec:posix:req:builtin.jobs.remove-reported-job/test]
    Case(
        id="job-jobs-remove-reported",
        rules=("builtin.jobs.remove-reported-job",),
        script=(
            "set -m\n"
            "(exit 3) &\n"
            "p=$!\n"
            "sleep 0.3\n"
            "jobs >j1\n"
            "jobs >j2\n"
            "grep -q 'Done(3)' j1 && printf 'reported\\n'\n"
            "test -s j2 && printf 'STILL-LISTED\\n'\n"
            "printf 'removed-from-list\\n'\n"
            "wait $p 2>/dev/null\n"
            "printf 'wait=%s\\n' \"$?\"\n"
        ),
        stdout="reported\nremoved-from-list\nwait=127\n",
        timeout=15.0,
    ),
    # [spec:posix:req:builtin.jobs.stderr/test]
    # [spec:posix:req:builtin.jobs.exit-status/test]
    # [spec:posix:req:builtin.jobs.interfaces/test]
    Case(
        id="job-jobs-stderr-status-interfaces",
        rules=(
            "builtin.jobs.stderr",
            "builtin.jobs.exit-status",
            "builtin.jobs.interfaces",
        ),
        script=(
            "set -m\n"
            "sleep 0.4 &\n"
            "jobs >jo 2>je\n"
            "printf 'status=%s\\n' \"$?\"\n"
            "test -s je && printf 'UNEXPECTED-STDERR\\n'\n"
            "printf 'from-stdin\\n' | { jobs >/dev/null 2>&1; read line;"
            " printf '%s\\n' \"$line\"; }\n"
            "wait\n"
        ),
        stdout="status=0\nfrom-stdin\n",
        timeout=15.0,
    ),
    # The <command> field is required whether or not job control is enabled:
    # the jobs page states the format unconditionally.
    # [spec:posix:req:builtin.jobs.stdout-default-format/test]
    Case(
        id="job-jobs-command-field-without-monitor",
        rules=("builtin.jobs.stdout-default-format",),
        script=(
            "sleep 0.4 &\n"
            "jobs >jn\n"
            "read -r line <jn\n"
            "case $line in\n"
            "  *'sleep 0.4') printf 'command-field-present\\n' ;;\n"
            "  *) printf 'MISSING-COMMAND:[%s]\\n' \"$line\" ;;\n"
            "esac\n"
            "wait\n"
        ),
        stdout="command-field-present\n",
        timeout=15.0,
    ),
    # ------------------------------------------------------------------
    # bg / fg
    # ------------------------------------------------------------------
    # [spec:posix:syn:builtin.bg.synopsis/test]
    # [spec:posix:req:builtin.bg.resume-suspended-jobs/test]
    # [spec:posix:req:builtin.bg.operand-job-id/test]
    # [spec:posix:req:builtin.bg.stdout-format/test]
    # [spec:posix:req:builtin.bg.exit-status/test]
    # [spec:posix:req:builtin.bg.stderr/test]
    Case(
        id="job-bg-resume",
        rules=(
            "builtin.bg.synopsis",
            "builtin.bg.resume-suspended-jobs",
            "builtin.bg.operand-job-id",
            "builtin.bg.stdout-format",
            "builtin.bg.exit-status",
            "builtin.bg.stderr",
        ),
        script=(
            "sleep 5 &\n"
            "sleep 0.3\n"
            "kill -STOP %1\n"
            "sleep 0.5\n"
            "bg >bo 2>be\n"
            "printf 'bg-status=%s\\n' \"$?\"\n"
            "test -s be && printf 'UNEXPECTED-STDERR\\n'\n"
            "read -r bline <bo\n"
            "case $bline in\n"
            "  '[1] '*'sleep 5') printf 'bg-output-ok\\n' ;;\n"
            "  *) printf 'BAD-BG-OUTPUT:[%s]\\n' \"$bline\" ;;\n"
            "esac\n"
            "sleep 0.4\n"
            "jobs >jr\n"
            "grep -q Running jr && printf 'resumed\\n'\n"
            "exit 0\n"
        ),
        mode="interactive",
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        status="any",
        stdout_contains=("bg-status=0\n", "bg-output-ok\n", "resumed\n"),
        timeout=15.0,
        requires=("UP",),
    ),
    # [spec:posix:req:builtin.bg.already-running-no-effect/test]
    Case(
        id="job-bg-already-running",
        rules=("builtin.bg.already-running-no-effect",),
        script=(
            "sleep 3 &\n"
            "sleep 0.3\n"
            "bg %1 >/dev/null 2>&1\n"
            "printf 'status=%s\\n' \"$?\"\n"
            "exit 0\n"
        ),
        mode="interactive",
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        status="any",
        stdout_contains=("status=0\n",),
        timeout=15.0,
        requires=("UP",),
    ),
    # [spec:posix:req:builtin.bg.job-control-disabled/test]
    Case(
        id="job-bg-job-control-disabled",
        rules=("builtin.bg.job-control-disabled",),
        script=(
            "sleep 0.4 &\n"
            "if bg %1 2>/dev/null; then\n"
            "  printf 'BAD-ZERO\\n'\n"
            "else\n"
            "  printf 'error-status\\n'\n"
            "fi\n"
            "wait\n"
            "printf 'done\\n'\n"
        ),
        stdout="error-status\ndone\n",
        timeout=15.0,
    ),
    # [spec:posix:req:builtin.bg.interfaces/test]
    Case(
        id="job-bg-interfaces",
        rules=("builtin.bg.interfaces",),
        script=(
            "sleep 5 &\n"
            "sleep 0.3\n"
            "kill -STOP %1\n"
            "sleep 0.5\n"
            "printf 'from-stdin\\n' | { bg %1 >/dev/null 2>&1; read line;"
            " printf '%s\\n' \"$line\"; }\n"
            "exit 0\n"
        ),
        mode="interactive",
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        status="any",
        stdout_contains=("from-stdin\n",),
        timeout=15.0,
        requires=("UP",),
    ),
    # [spec:posix:syn:builtin.fg.synopsis/test]
    # [spec:posix:req:builtin.fg.move-job-to-foreground/test]
    # [spec:posix:req:builtin.fg.operand-job-id/test]
    # [spec:posix:req:builtin.fg.stdout-format/test]
    # [spec:posix:req:builtin.fg.exit-status/test]
    # [spec:posix:req:builtin.fg.stderr/test]
    Case(
        id="job-fg-foreground",
        rules=(
            "builtin.fg.synopsis",
            "builtin.fg.move-job-to-foreground",
            "builtin.fg.operand-job-id",
            "builtin.fg.stdout-format",
            "builtin.fg.exit-status",
            "builtin.fg.stderr",
        ),
        script=(
            "(sleep 0.3; exit 6) &\n"
            "sleep 0.1\n"
            "fg >fo 2>fe\n"
            "printf 'fg-status=%s\\n' \"$?\"\n"
            "test -s fe && printf 'UNEXPECTED-STDERR\\n'\n"
            "read -r fline <fo\n"
            "case $fline in\n"
            "  *'sleep 0.3'*) printf 'fg-output-ok\\n' ;;\n"
            "  *) printf 'BAD-FG-OUTPUT:[%s]\\n' \"$fline\" ;;\n"
            "esac\n"
            "exit 0\n"
        ),
        mode="interactive",
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        status="any",
        stdout_contains=("fg-status=6\n", "fg-output-ok\n"),
        timeout=15.0,
        requires=("UP",),
    ),
    # [spec:posix:req:builtin.fg.removes-known-process-id/test]
    Case(
        id="job-fg-removes-known-pid",
        rules=("builtin.fg.removes-known-process-id",),
        script=(
            "sleep 0.3 &\n"
            "p=$!\n"
            "sleep 0.1\n"
            "fg >/dev/null 2>&1\n"
            "wait $p 2>/dev/null\n"
            "printf 'wait=%s\\n' \"$?\"\n"
            "exit 0\n"
        ),
        mode="interactive",
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        status="any",
        stdout_contains=("wait=127\n",),
        timeout=15.0,
        requires=("UP",),
    ),
    # [spec:posix:req:builtin.fg.job-control-disabled/test]
    Case(
        id="job-fg-job-control-disabled",
        rules=("builtin.fg.job-control-disabled",),
        script=(
            "sleep 0.4 &\n"
            "if fg %1 2>/dev/null; then\n"
            "  printf 'BAD-ZERO\\n'\n"
            "else\n"
            "  printf 'error-status\\n'\n"
            "fi\n"
            "wait\n"
            "printf 'done\\n'\n"
        ),
        stdout="error-status\ndone\n",
        timeout=15.0,
    ),
    # [spec:posix:req:builtin.fg.interfaces/test]
    Case(
        id="job-fg-interfaces",
        rules=("builtin.fg.interfaces",),
        script=(
            "sleep 0.4 &\n"
            "sleep 0.1\n"
            "printf 'from-stdin\\n' | { fg %1 >/dev/null 2>&1; read line;"
            " printf '%s\\n' \"$line\"; }\n"
            "exit 0\n"
        ),
        mode="interactive",
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        status="any",
        stdout_contains=("from-stdin\n",),
        timeout=15.0,
        requires=("UP",),
    ),
)
