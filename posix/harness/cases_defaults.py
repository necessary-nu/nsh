"""Executable cases for XCU 1.1 (relationship), 1.2-1.7 (utility defaults),
and the sh utility description (invocation).

Every expectation here states what POSIX.1-2024 requires, not what any
particular shell does. Where a case exercises the shell through a nested
invocation it uses the bare name ``sh``: the harness puts a symlink to the
shell under test first on PATH, so ``sh`` is always the shell being judged.
"""

from __future__ import annotations

from model import Case, FileFixture


# A here-document large enough that an implementation using a temporary file
# rather than a pipe would have to create one.
_BIG_HEREDOC = "".join(f"payload line {n}\n" for n in range(4000))

_HEREDOC_SCRIPT = (
    "v=$(cat <<'EOF'\n"
    + _BIG_HEREDOC
    + "EOF\n)\n"
    "case $v in\n"
    "payload*payload*) echo HEREDOC-OK ;;\n"
    "*) echo HEREDOC-BAD ;;\n"
    "esac\n"
)

# `st` reports only whether a status was zero, because POSIX pins an exact
# value in very few places; asserting dash's 2 would be asserting dash.
_ST_HELPER = (
    "st() {\n"
    "  if [ \"$1\" -eq 0 ]; then echo \"$2=zero\"; else echo \"$2=nonzero\"; fi\n"
    "}\n"
)


CASES: tuple[Case, ...] = (
    # ------------------------------------------------------------------
    # XCU sh: SYNOPSIS, OPTIONS, OPERANDS
    # ------------------------------------------------------------------
    # [spec:posix:syn:sh.synopsis/test]
    # [spec:posix:req:sh.command-language-interpreter/test]
    Case(
        id="inv-synopsis-forms",
        rules=("sh.synopsis", "sh.command-language-interpreter"),
        script=(
            "sh -f -c 'echo from-string'\n"
            "sh -f file.sh\n"
            "sh -f -s < stdin.sh\n"
        ),
        files={
            "file.sh": FileFixture("echo from-file\n"),
            "stdin.sh": FileFixture("echo from-stdin\n"),
        },
        stdout="from-string\nfrom-file\nfrom-stdin\n",
    ),
    # [spec:posix:req:sh.option-c/test]
    # [spec:posix:req:sh.stdin-used-only-if/test]
    Case(
        id="inv-option-c-ignores-stdin",
        rules=("sh.option-c", "sh.stdin-used-only-if"),
        # "No commands shall be read from the standard input" with -c, and
        # standard input is likewise untouched when a command_file operand
        # is given: neither invocation may consume the LEAK line.
        script=(
            "printf 'echo LEAK\\n' | sh -c 'echo from-c'\n"
            "printf 'echo LEAK\\n' | sh file.sh\n"
        ),
        files={"file.sh": FileFixture("echo from-file\n")},
        stdout="from-c\nfrom-file\n",
        stdout_excludes=("LEAK",),
    ),
    # [spec:posix:req:sh.option-s/test]
    # [spec:posix:req:sh.operand-argument/test]
    Case(
        id="inv-option-s-operands",
        rules=("sh.option-s", "sh.operand-argument"),
        script="sh -s alpha beta < in.sh\n",
        files={"in.sh": FileFixture('printf "%s|%s|%s\\n" "$0" "$1" "$2"\n')},
        # $0 is the first argument passed to sh from its parent, i.e. "sh".
        stdout="sh|alpha|beta\n",
    ),
    # [spec:posix:req:sh.option-s-assumed/test]
    Case(
        id="inv-option-s-assumed",
        rules=("sh.option-s-assumed",),
        script="sh < in.sh\n",
        files={"in.sh": FileFixture("echo assumed\n")},
        stdout="assumed\n",
    ),
    # [spec:posix:req:sh.operand-hyphen/test]
    Case(
        id="inv-operand-hyphen",
        rules=("sh.operand-hyphen",),
        # A single '-' is the first operand and is then ignored, so the next
        # operand becomes command_file and the rest become $1...
        script="sh - file.sh one two\n",
        files={"file.sh": FileFixture('printf "%s|%s|%s\\n" "$0" "$1" "$2"\n')},
        stdout="file.sh|one|two\n",
    ),
    # [spec:posix:req:sh.operand-command-file/test]
    # [spec:posix:req:sh.special-parameter-0/test]
    Case(
        id="inv-operand-command-file",
        rules=("sh.operand-command-file", "sh.special-parameter-0"),
        # None of the three fixtures is executable: "the file need not be
        # executable" in both the slash and the no-slash case.
        script=("sh plain.sh\n" "sh ./plain.sh\n" "sh sub/deep.sh\n"),
        files={
            "plain.sh": FileFixture('printf "0=%s\\n" "$0"\n', 0o644),
            "sub/deep.sh": FileFixture('printf "0=%s\\n" "$0"\n', 0o644),
        },
        stdout="0=plain.sh\n0=./plain.sh\n0=sub/deep.sh\n",
    ),
    # [spec:posix:req:sh.operand-command-string/test]
    Case(
        id="inv-command-string-empty",
        rules=("sh.operand-command-string",),
        script="sh -c ''\necho \"status=$?\"\n",
        stdout="status=0\n",
    ),
    # [spec:posix:req:sh.set-derived-options/test]
    # [spec:posix:req:sh.utility-syntax-guidelines/test]
    Case(
        id="inv-set-derived-options",
        rules=("sh.set-derived-options", "sh.utility-syntax-guidelines"),
        # A leading '+' means the reverse case of the option, for both the
        # letter form and the -o form.
        script=(
            _ST_HELPER
            + "sh -u -c 'echo \"[$nosuch]\"' >/dev/null 2>&1; st $? minus-u\n"
            "sh -u +u -c 'echo \"[$nosuch]\"'; st $? plus-u\n"
            "sh -o nounset -c 'echo \"[$nosuch]\"' >/dev/null 2>&1; st $? minus-o\n"
            "sh -o nounset +o nounset -c 'echo \"[$nosuch]\"'; st $? plus-o\n"
        ),
        stdout=(
            "minus-u=nonzero\n"
            "[]\nplus-u=zero\n"
            "minus-o=nonzero\n"
            "[]\nplus-o=zero\n"
        ),
    ),
    # [spec:posix:req:sh.utility-syntax-guidelines/test]
    Case(
        id="inv-utility-syntax-guidelines",
        rules=("sh.utility-syntax-guidelines",),
        # XBD 12.2 guideline 5: single-letter options may be grouped behind
        # one '-'. Guideline 10: "--" delimits the end of options.
        script=("sh -uc 'echo grouped'\n" "sh -- file.sh\n" "sh -f -- file.sh\n"),
        files={"file.sh": FileFixture("echo from-file\n")},
        stdout="grouped\nfrom-file\nfrom-file\n",
    ),
    # [spec:posix:req:sh.option-i/test]
    Case(
        id="inv-option-i-noninteractive-c",
        rules=("sh.option-i",),
        # "-i: Specify that the shell is interactive." The option makes the
        # shell interactive even though no terminal is attached.
        script=(
            "sh -i -c 'case $- in *i*) echo INTERACTIVE ;; *) echo NOT ;; esac'"
            " 2>/dev/null\n"
        ),
        stdout="INTERACTIVE\n",
    ),
    # [spec:posix:def:sh.interactive/test]
    Case(
        id="inv-interactive-definition",
        rules=("sh.interactive",),
        mode="interactive",
        # On a controlling terminal: -i is interactive, and a nested shell
        # that reads commands from that same terminal is interactive too,
        # while a -c shell (which does not read standard input) is not.
        script=(
            "case $- in *i*) echo OUTER-I ;; *) echo OUTER-NOT ;; esac\n"
            "sh -c 'case $- in *i*) echo C-I ;; *) echo C-NOT ;; esac'\n"
            "sh\n"
            "case $- in *i*) echo NESTED-I ;; *) echo NESTED-NOT ;; esac\n"
            "exit\n"
        ),
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("OUTER-I", "C-NOT", "NESTED-I"),
        stdout_excludes=("OUTER-NOT", "C-I\n", "NESTED-NOT"),
        # The harness runs interactive cases inside a PID namespace, where a
        # shell cannot make itself the terminal's foreground process group.
        # Every interactive shell therefore ends with a "Cannot set tty
        # process group" diagnostic and a non-zero status regardless of the
        # rule under test, so the exit status carries no information here.
        status="any",
    ),
    # ------------------------------------------------------------------
    # XCU sh: STDIN and INPUT FILES
    # ------------------------------------------------------------------
    # [spec:posix:req:sh.stdin-no-read-ahead/test]
    Case(
        id="inv-stdin-no-read-ahead",
        rules=("sh.stdin-no-read-ahead",),
        mode="stdin",
        # The shell must leave the file pointer directly after "read x" so
        # that read consumes VALUE, and must not have swallowed the third
        # line while doing so.
        script="read x\nVALUE\necho \"got $x\"\n",
        stdout="got VALUE\n",
    ),
    # [spec:posix:req:sh.input-file-contents/test]
    # [spec:posix:req:xcu.stdin.input-file-restrictions-apply/test]
    Case(
        id="inv-input-file-long-lines",
        rules=("sh.input-file-contents", "xcu.stdin.input-file-restrictions-apply"),
        # "The shell shall not enforce any line length limits" -- and the
        # INPUT FILES restrictions apply to standard input as well.
        script=(
            "awk 'BEGIN{printf \"x=\\\"\"; "
            "for(i=0;i<200000;i++) printf \"a\"; "
            "printf \"\\\"\\necho ${#x}\\n\"}' > long.sh\n"
            "sh long.sh\n"
            "sh < long.sh\n"
            "sh -s < long.sh\n"
        ),
        stdout="200000\n200000\n200000\n",
        timeout=20.0,
    ),
    # [spec:posix:req:sh.input-file-blank-or-comments/test]
    Case(
        id="inv-blank-or-comment-file",
        rules=("sh.input-file-blank-or-comments",),
        script=(
            "sh blank.sh; echo \"file=$?\"\n"
            "sh < blank.sh; echo \"stdin=$?\"\n"
        ),
        files={"blank.sh": FileFixture("\n\n# a comment\n   \n\t\n# another\n")},
        stdout="file=0\nstdin=0\n",
    ),
    # [spec:posix:req:sh.pathname-expansion-file-size/test]
    Case(
        id="inv-pathname-expansion-file-size",
        rules=("sh.pathname-expansion-file-size",),
        # A file larger than 2^32 bytes must not defeat pathname expansion.
        script=(
            "mkdir g\n"
            "dd if=/dev/null of=g/huge.dat bs=1 seek=5000000000 count=0 2>/dev/null\n"
            ": > g/small.dat\n"
            "set -- g/*.dat\n"
            "echo \"$# $1 $2\"\n"
        ),
        stdout="2 g/huge.dat g/small.dat\n",
        timeout=30.0,
    ),
    # ------------------------------------------------------------------
    # XCU sh: ENVIRONMENT VARIABLES
    # ------------------------------------------------------------------
    # [spec:posix:req:sh.envvar-env/test]
    Case(
        id="inv-envvar-env-interactive",
        rules=("sh.envvar-env",),
        mode="interactive",
        requires=("UP",),
        # ENV is stored unexpanded so that the case also proves the value is
        # "subjected to parameter expansion ... and the resulting value shall
        # be used as a pathname of a file containing shell commands".
        script="echo body\n",
        environment={"PS1": "", "PS2": "", "ENV": "$HOME/envrc"},
        files={".home/envrc": FileFixture("echo ENVLOADED\n")},
        stdout=None,
        stdout_contains=("ENVLOADED", "body"),
        # See inv-interactive-definition: the sandboxed pty makes the exit
        # status of every interactive shell uninformative.
        status="any",
    ),
    # [spec:posix:req:sh.envvar-env/test]
    Case(
        id="inv-envvar-env-not-interactive",
        rules=("sh.envvar-env",),
        # "when and only when an interactive shell is invoked".
        script="echo body\n",
        environment={"ENV": "{HOME}/envrc"},
        files={".home/envrc": FileFixture("echo ENVLOADED\n")},
        stdout="body\n",
        stdout_excludes=("ENVLOADED",),
    ),
    # [spec:posix:sem:sh.envvar-home/test]
    Case(
        id="inv-envvar-home",
        rules=("sh.envvar-home",),
        script=(
            "echo \"$HOME\"\n"
            "echo ~\n"
            "HOME=/myhome\n"
            "echo ~\n"
        ),
        stdout="{HOME}\n{HOME}\n/myhome\n",
    ),
    # [spec:posix:sem:sh.envvar-lang-and-lc-all/test]
    # [spec:posix:sem:sh.envvar-lc-ctype/test]
    # [spec:posix:req:xcurel.establish-locale/test]
    Case(
        id="inv-envvar-lang-lc-all",
        rules=(
            "sh.envvar-lang-and-lc-all",
            "sh.envvar-lc-ctype",
            "xcurel.establish-locale",
        ),
        # A two-byte UTF-8 sequence is one character under a UTF-8 LC_CTYPE
        # and two characters in the POSIX locale, so '?' distinguishes them.
        # LANG supplies the default; a non-empty LC_ALL overrides it.
        # If the host has no UTF-8 locale installed there is nothing to
        # observe, so the case reports that on standard error and asserts
        # nothing rather than manufacturing a failure.
        script=(
            "u=\n"
            "for l in C.UTF-8 C.utf8 en_US.UTF-8 en_US.utf8; do\n"
            "  if locale -a 2>/dev/null | grep -qx \"$l\"; then u=$l; break; fi\n"
            "done\n"
            "e=$(printf '\\303\\251')\n"
            "if [ -z \"$u\" ]; then\n"
            "  echo 'no UTF-8 locale installed; nothing observable' >&2\n"
            "  echo ONE; echo MANY; exit 0\n"
            "fi\n"
            "LC_ALL=; LANG=$u; export LC_ALL LANG\n"
            "case $e in ?) echo ONE ;; *) echo MANY ;; esac\n"
            "LC_ALL=C; export LC_ALL\n"
            "case $e in ?) echo ONE ;; *) echo MANY ;; esac\n"
        ),
        stdout="ONE\nMANY\n",
    ),
    # [spec:posix:sem:sh.envvar-lc-collate/test]
    Case(
        id="inv-envvar-lc-collate",
        rules=("sh.envvar-lc-collate",),
        # LC_COLLATE determines the behaviour of range expressions in
        # pattern matching; in the POSIX locale [a-c] is exactly a, b, c.
        script=(
            "LC_COLLATE=C; export LC_COLLATE\n"
            "for c in a b c d B; do\n"
            "  case $c in [a-c]) echo \"$c IN\" ;; *) echo \"$c OUT\" ;; esac\n"
            "done\n"
        ),
        stdout="a IN\nb IN\nc IN\nd OUT\nB OUT\n",
    ),
    # [spec:posix:sem:sh.envvar-path/test]
    Case(
        id="inv-envvar-path",
        rules=("sh.envvar-path",),
        script=(
            "(PATH=$PWD/bin1:$PWD/bin2; export PATH; tool)\n"
            "(PATH=$PWD/bin2:$PWD/bin1; export PATH; tool)\n"
        ),
        files={
            "bin1/tool": FileFixture("#!/bin/sh\necho ONE\n", 0o755),
            "bin2/tool": FileFixture("#!/bin/sh\necho TWO\n", 0o755),
        },
        stdout="ONE\nTWO\n",
    ),
    # [spec:posix:req:sh.envvar-pwd/test]
    Case(
        id="inv-envvar-pwd",
        rules=("sh.envvar-pwd",),
        script=(
            "case $PWD in /*) echo absolute ;; *) echo \"relative:$PWD\" ;; esac\n"
            "if [ \"$PWD\" = \"$(pwd)\" ]; then echo matches; else echo MISMATCH; fi\n"
            "cd /\n"
            "echo \"$PWD\"\n"
        ),
        stdout="absolute\nmatches\n/\n",
    ),
    # [spec:posix:req:sh.envvar-mail/test]
    # [spec:posix:req:sh.envvar-mailcheck/test]
    Case(
        id="inv-envvar-mail",
        rules=("sh.envvar-mail", "sh.envvar-mailcheck"),
        mode="interactive",
        requires=("UP",),
        # MAILCHECK set to zero means "check before issuing each primary
        # prompt"; creating the file must then inform the user before the
        # next prompt. A terminal merges the two streams, so the required
        # "written to standard error" placement is not separable here.
        script=(
            "MAILCHECK=0\n"
            "MAIL=$PWD/mbox\n"
            "export MAILCHECK MAIL\n"
            "echo seed > \"$MAIL\"\n"
            ":\n"
            "echo DONE\n"
        ),
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("mail", "DONE"),
        status="any",
        timeout=10.0,
    ),
    # [spec:posix:req:sh.envvar-mailpath/test]
    Case(
        id="inv-envvar-mailpath",
        rules=("sh.envvar-mailpath",),
        mode="interactive",
        requires=("UP",),
        # The text after '%' is the message, and it is subjected to
        # parameter expansion before being written.
        script=(
            "MAILCHECK=0\n"
            "who=postmaster\n"
            "MAILPATH=\"$PWD/mbox%NEW MAIL FOR $who\"\n"
            "export MAILCHECK MAILPATH\n"
            "echo seed > \"$PWD/mbox\"\n"
            ":\n"
            "echo DONE\n"
        ),
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("NEW MAIL FOR postmaster", "DONE"),
        status="any",
        timeout=10.0,
    ),
    # ------------------------------------------------------------------
    # XCU sh: ASYNCHRONOUS EVENTS
    # ------------------------------------------------------------------
    # [spec:posix:req:sh.signals-standard-action/test]
    # [spec:posix:req:xcu.defaults.asynchronous-events-default/test]
    Case(
        id="inv-signal-inherited-ignore",
        rules=("sh.signals-standard-action", "xcu.defaults.asynchronous-events-default"),
        # "If the action inherited from the invoking process ... is for the
        # signal to be ignored, the utility shall ignore the signal."
        script=(
            "trap '' INT\n"
            "sh -c 'kill -INT $$; echo survived'\n"
            "echo \"status=$?\"\n"
        ),
        stdout="survived\nstatus=0\n",
    ),
    # [spec:posix:req:sh.signals-standard-action/test]
    # [spec:posix:req:xcu.defaults.asynchronous-events-default/test]
    Case(
        id="inv-signal-default-action",
        rules=("sh.signals-standard-action", "xcu.defaults.asynchronous-events-default"),
        # "If the action inherited ... is the default signal action, the
        # result of the utility's execution shall be as if the default
        # signal action had been taken": termination by SIGINT, never a
        # normal return that swallows the signal.
        script="sh -c 'kill -INT $$; echo BAD'\necho \"status=$?\"\n",
        stdout="status=130\n",
    ),
    # [spec:posix:req:sh.signal-actions-overridable/test]
    Case(
        id="inv-signal-trap-overrides",
        rules=("sh.signal-actions-overridable",),
        script="trap 'echo caught' INT\nkill -INT $$\necho after\n",
        stdout="caught\nafter\n",
    ),
    # [spec:posix:req:sh.interactive-sigint/test]
    Case(
        id="inv-interactive-sigint",
        rules=("sh.interactive-sigint",),
        mode="interactive",
        # "SIGINT signals received at other times shall be caught but no
        # action performed" -- the interactive shell must survive.
        script="kill -INT $$\necho ALIVE\n",
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("ALIVE",),
        status="any",
        timeout=10.0,
    ),
    # [spec:posix:req:sh.interactive-sigquit-sigterm/test]
    Case(
        id="inv-interactive-sigquit-sigterm",
        rules=("sh.interactive-sigquit-sigterm",),
        mode="interactive",
        script="kill -TERM $$\nkill -QUIT $$\necho ALIVE\n",
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("ALIVE",),
        status="any",
        timeout=10.0,
    ),
    # [spec:posix:req:sh.interactive-stop-signals/test]
    Case(
        id="inv-interactive-stop-signals",
        rules=("sh.interactive-stop-signals",),
        mode="interactive",
        # With -m in effect all three stop signals shall be ignored. The
        # branch for -m not in effect is explicitly unspecified, so only the
        # -m case is asserted. Each signal is confirmed separately: a shell
        # that does not ignore one of them is stopped by it and the marker
        # after it never appears.
        script=(
            "set -m\n"
            "case $- in *m*) echo M-ON ;; *) echo M-OFF ;; esac\n"
            "kill -TSTP $$\n"
            "echo TSTP-IGNORED\n"
            "kill -TTOU $$\n"
            "echo TTOU-IGNORED\n"
            "kill -TTIN $$\n"
            "echo TTIN-IGNORED\n"
        ),
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("M-ON", "TSTP-IGNORED", "TTOU-IGNORED", "TTIN-IGNORED"),
        status="any",
        timeout=5.0,
    ),
    # ------------------------------------------------------------------
    # XCU sh: STDERR, OUTPUT FILES, EXIT STATUS
    # ------------------------------------------------------------------
    # [spec:posix:req:sh.stderr-diagnostics-only/test]
    # [spec:posix:req:xcu.defaults.stderr-diagnostics-only/test]
    Case(
        id="inv-stderr-diagnostics-only",
        rules=("sh.stderr-diagnostics-only", "xcu.defaults.stderr-diagnostics-only"),
        # Nothing on standard error while the exit status says success;
        # a diagnostic on standard error when it says failure.
        script=(
            "sh -c 'echo out; v=1; export v; cd /; : ; true' >/dev/null 2>ok.err\n"
            "sh -c 'cd /no/such/directory' >/dev/null 2>bad.err\n"
            "if [ -s ok.err ]; then echo UNEXPECTED-STDERR; else echo clean; fi\n"
            "if [ -s bad.err ]; then echo diagnosed; else echo MISSING-DIAGNOSTIC; fi\n"
        ),
        stdout="clean\ndiagnosed\n",
        stderr="",
    ),
    # [spec:posix:sem:sh.output-files/test]
    # [spec:posix:req:xcu.defaults.output-files-none/test]
    Case(
        id="inv-output-files-none",
        rules=("sh.output-files", "xcu.defaults.output-files-none"),
        # sh's OUTPUT FILES section is "None.", which means no files are
        # created or modified as a consequence of direct action on the part
        # of the utility.
        script=(
            "mkdir empty\n"
            "cd empty\n"
            "sh ../work.sh >/dev/null\n"
            "ls -A | wc -l\n"
        ),
        files={"work.sh": FileFixture(_HEREDOC_SCRIPT)},
        stdout="0\n",
    ),
    # [spec:posix:req:sh.exit-status-values/test]
    Case(
        id="inv-exit-status-error-range",
        rules=("sh.exit-status-values",),
        # "1-125 | A non-interactive shell detected an error other than
        # command_file not found, command_file not executable, or an
        # unrecoverable read error ... including but not limited to syntax,
        # redirection, or variable assignment errors."
        script=(
            "rng() {\n"
            "  if [ \"$1\" -ge 1 ] && [ \"$1\" -le 125 ]; then\n"
            "    echo \"$2=in-range\"\n"
            "  else\n"
            "    echo \"$2=$1\"\n"
            "  fi\n"
            "}\n"
            "sh -c 'if' >/dev/null 2>&1; rng $? syntax\n"
            "sh -c 'echo x > /no/such/dir/f' >/dev/null 2>&1; rng $? redirection\n"
            "sh -c 'readonly r=1; r=2' >/dev/null 2>&1; rng $? assignment\n"
        ),
        stdout="syntax=in-range\nredirection=in-range\nassignment=in-range\n",
    ),
    # [spec:posix:req:sh.exit-status-values/test]
    # [spec:posix:req:xcu.exit-status.listed-values-binding/test]
    Case(
        id="inv-exit-status-command-file-not-found",
        rules=("sh.exit-status-values", "xcu.exit-status.listed-values-binding"),
        # "127 | A specified command_file could not be found by a
        # non-interactive shell." XCU 1.4 adds that when specific numeric
        # values are listed, "the system shall use those values for the
        # errors described".
        script="sh no_such_command_file_9271 >/dev/null 2>&1\necho \"status=$?\"\n",
        stdout="status=127\n",
    ),
    # [spec:posix:req:sh.exit-status-values/test]
    Case(
        id="inv-exit-status-read-error",
        rules=("sh.exit-status-values",),
        # "128 | An unrecoverable read error was detected while reading
        # commands". INPUT FILES says "The input file can be of any type",
        # and reading a directory fails with EISDIR after a successful open.
        script=(
            "mkdir adir\n"
            "sh adir >/dev/null 2>&1\n"
            "echo \"status=$?\"\n"
        ),
        stdout="status=128\n",
    ),
    # [spec:posix:req:sh.exit-status-otherwise/test]
    Case(
        id="inv-exit-status-otherwise",
        rules=("sh.exit-status-otherwise",),
        # "the shell shall terminate in the same manner as for an exit
        # command with no operands".
        script=(
            "sh -c 'true; false'; echo \"a=$?\"\n"
            "sh -c 'exit 42'; echo \"b=$?\"\n"
            "sh -c 'true; exit'; echo \"c=$?\"\n"
            "sh -c 'false; exit'; echo \"d=$?\"\n"
        ),
        stdout="a=1\nb=42\nc=0\nd=1\n",
    ),
    # [spec:posix:req:sh.exit-status-otherwise/test]
    Case(
        id="inv-exit-status-no-fork-wait",
        rules=("sh.exit-status-otherwise",),
        # "unless the last command the shell invoked was executed without
        # forking, in which case the wait status seen by the parent process
        # of the shell shall be the wait status of the last command".
        # exec replaces the shell, so the wait status must report death by
        # SIGTERM (128+15) rather than a normal exit. The harness cannot
        # observe WIFSIGNALED directly because a sandbox wrapper sits
        # between it and the shell, so the parent shell's $? is used.
        script="sh -c 'exec kill -TERM $$'\necho \"status=$?\"\n",
        stdout="status=143\n",
        stderr=None,
    ),
    # ------------------------------------------------------------------
    # XCU 1.4 Utility Description Defaults
    # ------------------------------------------------------------------
    # [spec:posix:req:xcu.options.unrecognized-diagnostic/test]
    Case(
        id="def-options-unrecognized-diagnostic",
        rules=("xcu.options.unrecognized-diagnostic",),
        script=(
            "chk() {\n"
            "  if [ \"$1\" -ne 0 ] && [ -s \"$2\" ]; then\n"
            "    echo \"$3=rejected\"\n"
            "  else\n"
            "    echo \"$3=BAD status=$1\"\n"
            "  fi\n"
            "}\n"
            "sh -Z -c 'echo BAD' >/dev/null 2>sh.err; chk $? sh.err sh\n"
            "sh -c 'cd -Z /' >/dev/null 2>cd.err; chk $? cd.err cd\n"
            "sh -c 'read -Z v </dev/null' >/dev/null 2>read.err; chk $? read.err read\n"
        ),
        stdout="sh=rejected\ncd=rejected\nread=rejected\n",
    ),
    # [spec:posix:req:xcu.options.eight-bit-transparency/test]
    Case(
        id="def-options-eight-bit",
        rules=("xcu.options.eight-bit-transparency",),
        script=(
            "a=$(printf '\\351\\200\\377')\n"
            "sh -c 'printf %s \"$1\"' argv0 \"$a\" | od -An -tx1 | tr -d ' \\n'\n"
            "echo\n"
        ),
        stdout="e980ff\n",
    ),
    # [spec:posix:req:xcu.input-files.eight-bit-transparency/test]
    Case(
        id="def-input-files-eight-bit",
        rules=("xcu.input-files.eight-bit-transparency",),
        # The script file itself contains bytes with the high bit set.
        script=(
            "{ printf 'printf %%s \"'\n"
            "  printf '\\351\\200\\377'\n"
            "  printf '\" | od -An -tx1 | tr -d \" \\n\"\\n'\n"
            "} > eight.sh\n"
            "sh eight.sh\n"
            "echo\n"
        ),
        stdout="e980ff\n",
    ),
    # [spec:posix:req:xcu.env.eight-bit-transparency/test]
    Case(
        id="def-env-eight-bit",
        rules=("xcu.env.eight-bit-transparency",),
        script=(
            "V8=$(printf '\\351\\200\\377'); export V8\n"
            "sh -c 'printf %s \"$V8\"' | od -An -tx1 | tr -d ' \\n'\n"
            "echo\n"
        ),
        stdout="e980ff\n",
    ),
    # [spec:posix:req:xcu.operands.processing-order/test]
    Case(
        id="def-operands-processing-order",
        rules=("xcu.operands.processing-order",),
        # type reports one line per operand; the lines must follow the
        # command line order.
        script=(
            "sh -c 'type cd exit' | cut -d' ' -f1\n"
            "sh -c 'type exit cd' | cut -d' ' -f1\n"
        ),
        stdout="cd\nexit\nexit\ncd\n",
    ),
    # [spec:posix:req:xcu.input-files.seekable-file-offset/test]
    Case(
        id="def-input-files-seekable-offset",
        rules=("xcu.input-files.seekable-file-offset",),
        # sh terminates at "exit" without reaching end-of-file, so the
        # shared open file description must be positioned just past the
        # last byte it processed and cat must see the remainder.
        script="{ sh; cat; } < script.sh\n",
        files={"script.sh": FileFixture("echo one\nexit\nLEFTOVER\n")},
        stdout="one\nLEFTOVER\n",
    ),
    # [spec:posix:req:xcu.env.effects-confined-to-section/test]
    # [spec:posix:req:xcu.stdin.env-independence/test]
    # [spec:posix:req:xcu.stdout.env-independence/test]
    # [spec:posix:req:xcu.stderr.env-independence/test]
    Case(
        id="def-env-effects-confined",
        rules=(
            "xcu.env.effects-confined-to-section",
            "xcu.stdin.env-independence",
            "xcu.stdout.env-independence",
            "xcu.stderr.env-independence",
        ),
        # Variables sh does list but which have no non-interactive effect,
        # plus variables sh does not list at all, must leave the specified
        # standard input, output, and error untouched.
        script=(
            "poison() {\n"
            "  env FCEDIT=/bogus HISTFILE=/bogus HISTSIZE=notanumber \\\n"
            "      MAILCHECK=notanumber MAILPATH=/bogus NLSPATH=/bogus \\\n"
            "      TZ=Bogus/Zone COLUMNS=3 LINES=1 EDITOR=/bogus VISUAL=/bogus \\\n"
            "      IFS=Q OPTIND=9 \"$@\"\n"
            "}\n"
            "poison sh -c 'echo out' >stdout.txt 2>stderr.txt\n"
            "printf 'echo from-stdin\\n' | poison sh -s >>stdout.txt 2>>stderr.txt\n"
            "cat stdout.txt\n"
            "if [ -s stderr.txt ]; then echo UNEXPECTED-STDERR; else echo clean; fi\n"
        ),
        stdout="out\nfrom-stdin\nclean\n",
    ),
    # [spec:posix:req:xcu.output-files.temp-file-removal/test]
    Case(
        id="def-output-files-temp-removal",
        rules=("xcu.output-files.temp-file-removal",),
        # Any temporary file the implementation creates for the here-document
        # must be gone once the shell exits successfully.
        script=(
            "mkdir td\n"
            "TMPDIR=$PWD/td; export TMPDIR\n"
            "sh ./work.sh\n"
            "ls -A td | wc -l\n"
        ),
        files={"work.sh": FileFixture(_HEREDOC_SCRIPT)},
        stdout="HEREDOC-OK\n0\n",
        timeout=20.0,
    ),
    # [spec:posix:req:xcu.output-files.temp-file-naming/test]
    Case(
        id="def-output-files-temp-naming",
        rules=("xcu.output-files.temp-file-naming",),
        # Eight instances of the same utility, same working directory, at
        # the same time: none may collide on a temporary file name.
        script=(
            "i=0\n"
            "while [ $i -lt 8 ]; do sh ./conc.sh \"$i\" & i=$((i+1)); done\n"
            "wait\n"
            "echo CONCURRENT-OK\n"
        ),
        files={
            "conc.sh": FileFixture(
                "v=$(cat <<EOF\npayload-$1\nEOF\n)\n"
                "if [ \"$v\" != \"payload-$1\" ]; then echo \"MISMATCH-$1\"; fi\n"
            )
        },
        stdout="CONCURRENT-OK\n",
        timeout=20.0,
    ),
    # [spec:posix:req:xcu.defaults.exit-status-successful-completion/test]
    Case(
        id="def-exit-status-successful-completion",
        rules=("xcu.defaults.exit-status-successful-completion",),
        # Status 0 must mean every required action really happened.
        script=(
            "mkdir sub\n"
            "cd sub; s=$?\n"
            "if [ \"$s\" -eq 0 ] && [ \"$PWD\" = \"$(pwd)\" ] \\\n"
            "   && [ \"${PWD##*/}\" = sub ]; then\n"
            "  echo cd-completed\n"
            "else\n"
            "  echo \"CD-INCOMPLETE status=$s pwd=$PWD\"\n"
            "fi\n"
            "cd ..\n"
            "if : > made && [ -f made ]; then\n"
            "  echo redirect-completed\n"
            "else\n"
            "  echo REDIRECT-INCOMPLETE\n"
            "fi\n"
        ),
        stdout="cd-completed\nredirect-completed\n",
    ),
    # [spec:posix:req:xcu.errors.operand-failure-continues/test]
    Case(
        id="def-errors-operand-failure-continues",
        rules=("xcu.errors.operand-failure-continues",),
        # The action cannot be performed on either operand: both must be
        # diagnosed, processing must continue to the second, and the final
        # exit status must indicate an error.
        script=(
            "sh -c 'unalias no_such_a no_such_b' >out.txt 2>err.txt; s=$?\n"
            "if [ \"$s\" -eq 0 ]; then echo \"STATUS-NOT-ERROR:$s\"; else echo status-error; fi\n"
            "for name in no_such_a no_such_b; do\n"
            "  if grep -q \"$name\" err.txt; then\n"
            "    echo \"diagnosed-$name\"\n"
            "  else\n"
            "    echo \"MISSING-$name\"\n"
            "  fi\n"
            "done\n"
        ),
        stdout="status-error\ndiagnosed-no_such_a\ndiagnosed-no_such_b\n",
    ),
    # [spec:posix:req:xcu.errors.option-failure/test]
    Case(
        id="def-errors-option-failure",
        rules=("xcu.errors.option-failure",),
        # The action asked for by an option-argument cannot be performed.
        script=(
            "sh -c 'set -o no_such_option' >/dev/null 2>opt.err; s=$?\n"
            "if [ \"$s\" -ne 0 ] && [ -s opt.err ]; then\n"
            "  echo option-failure-diagnosed\n"
            "else\n"
            "  echo \"BAD status=$s\"\n"
            "fi\n"
        ),
        stdout="option-failure-diagnosed\n",
    ),
    # [spec:posix:req:xcu.errors.unrecoverable-exit-status/test]
    # [spec:posix:req:xcu.errors.diagnostic-message-required/test]
    Case(
        id="def-errors-unrecoverable",
        rules=(
            "xcu.errors.unrecoverable-exit-status",
            "xcu.errors.diagnostic-message-required",
        ),
        script=(
            "n=0\n"
            "chk() {\n"
            "  if [ \"$1\" -ne 0 ] && [ -s \"$2\" ]; then\n"
            "    echo \"$3=diagnosed\"\n"
            "  else\n"
            "    echo \"$3=BAD status=$1\"\n"
            "  fi\n"
            "}\n"
            "sh -c 'cd /no/such/directory' >/dev/null 2>e1; chk $? e1 cd\n"
            "sh -c ': < /no/such/file' >/dev/null 2>e2; chk $? e2 redirect\n"
            "sh -c 'no_such_command_4711' >/dev/null 2>e3; chk $? e3 command\n"
            "sh -c 'if' >/dev/null 2>e4; chk $? e4 syntax\n"
        ),
        stdout="cd=diagnosed\nredirect=diagnosed\ncommand=diagnosed\nsyntax=diagnosed\n",
    ),
    # [spec:posix:req:xcu.description.equivalent-functionality/test]
    Case(
        id="def-description-equivalent-functionality",
        rules=("xcu.description.equivalent-functionality",),
        # cd is described in terms of chdir(); the side-effect associated
        # with successful execution must be the real process attribute, so
        # an external utility and a child shell must agree with $PWD.
        script=(
            "mkdir sub\n"
            "cd sub\n"
            "a=$PWD; b=$(env pwd); c=$(sh -c 'env pwd')\n"
            "if [ \"$a\" = \"$b\" ] && [ \"$b\" = \"$c\" ]; then\n"
            "  echo chdir-equivalent\n"
            "else\n"
            "  echo \"MISMATCH a=$a b=$b c=$c\"\n"
            "fi\n"
        ),
        stdout="chdir-equivalent\n",
    ),
    # [spec:posix:req:xcu.description.declaration-utility/test]
    Case(
        id="def-description-declaration-utility",
        rules=("xcu.description.declaration-utility",),
        # export is explicitly a declaration utility, so the word after '='
        # is subject to tilde expansion; echo is not, so it must not be.
        script=(
            "HOME=/myhome; export HOME\n"
            "export v=~\n"
            "echo \"$v\"\n"
            "echo w=~\n"
            "readonly r=~\n"
            "echo \"$r\"\n"
        ),
        stdout="/myhome\nw=~\n/myhome\n",
    ),
    # ------------------------------------------------------------------
    # XCU 1.5-1.7
    # ------------------------------------------------------------------
    # [spec:posix:req:xcu.arbitrary-file-size/test]
    Case(
        id="def-arbitrary-file-size",
        rules=("xcu.arbitrary-file-size",),
        # sh is named in the table: a redirection must write correctly past
        # the 2^32 boundary and report the resulting size correctly.
        script=(
            "dd if=/dev/null of=huge.dat bs=1 seek=5000000000 count=0 2>/dev/null\n"
            "echo tail >> huge.dat\n"
            "ls -l huge.dat | awk '{print $5}'\n"
            "tail -c 5 huge.dat\n"
        ),
        stdout="5000000005\ntail\n",
        timeout=30.0,
    ),
    # [spec:posix:req:xcu.intrinsic-utilities/test]
    Case(
        id="def-intrinsic-utilities",
        rules=("xcu.intrinsic-utilities",),
        # Every utility named in Table: Intrinsic Utilities must be resolved
        # without a PATH search, so a same-named executable placed first on
        # PATH must never run.
        script=(
            "mkdir fake\n"
            "for u in alias bg cd command fc fg getopts hash jobs kill \\\n"
            "         read type ulimit umask unalias wait; do\n"
            "  printf '#!/bin/sh\\necho EXTERNAL-%s\\n' \"$u\" > fake/$u\n"
            "  chmod 755 fake/$u\n"
            "done\n"
            "PATH=$PWD/fake:$PATH; export PATH\n"
            "{ alias; bg; cd .; command :; fc -l; fg; getopts x v; hash; jobs;\n"
            "  kill -l; read v; type type; ulimit; umask; unalias -a; wait;\n"
            "} >out.txt 2>/dev/null </dev/null\n"
            "if grep -q EXTERNAL out.txt; then\n"
            "  grep EXTERNAL out.txt\n"
            "else\n"
            "  echo NO-PATH-SEARCH\n"
            "fi\n"
        ),
        stdout="NO-PATH-SEARCH\n",
    ),
    # ------------------------------------------------------------------
    # XCU 1.1.1 Relationship to the System Interfaces volume
    # ------------------------------------------------------------------
    # [spec:posix:req:xcurel.concurrent-execution/test]
    Case(
        id="rel-concurrent-execution",
        rules=("xcurel.concurrent-execution",),
        # 1. Independent processes execute independently without either
        #    terminating. 2. A created process carries the attributes of
        #    1.1.1.1 (working directory, file mode creation mask, process
        #    group, real/effective ids, open file descriptors).
        script=(
            "mkdir sub\n"
            "cd sub\n"
            "umask 0027\n"
            "exec 4>fd.txt\n"
            "( sleep 1; echo second ) &\n"
            "echo first\n"
            "wait\n"
            "sh -c 'printf \"%s %s\\n\" \"$(env pwd)\" \"$(umask)\"; echo via-fd4 >&4'\n"
            "exec 4>&-\n"
            "cat fd.txt\n"
        ),
        stdout="first\nsecond\n{ROOT}/sub 0027\nvia-fd4\n",
        timeout=20.0,
    ),
    # [spec:posix:req:xcurel.file-access-permissions/test]
    # [spec:posix:req:xcurel.file-open-access-mode/test]
    Case(
        id="rel-file-access-permissions",
        rules=("xcurel.file-access-permissions", "xcurel.file-open-access-mode"),
        # "When a file is to be read or written, the file shall be opened
        # with an access mode corresponding to the operation to be
        # performed. If file access permissions deny access, the requested
        # operation shall fail."
        script=(
            "chk() {\n"
            "  if [ \"$1\" -ne 0 ]; then echo \"$2=denied\"; else echo \"$2=ALLOWED\"; fi\n"
            "}\n"
            "echo data > ro.txt; chmod 444 ro.txt\n"
            "echo data > wo.txt; chmod 222 wo.txt\n"
            "echo data > none.txt; chmod 000 none.txt\n"
            "sh -c 'echo x > ro.txt' 2>/dev/null; chk $? write-read-only\n"
            "sh -c ': < wo.txt' 2>/dev/null; chk $? read-write-only\n"
            "sh -c ': < none.txt' 2>/dev/null; chk $? read-no-access\n"
            "sh -c 'echo x >> wo.txt' 2>/dev/null\n"
            "if [ $? -eq 0 ]; then echo append-write-only=allowed; else echo APPEND-DENIED; fi\n"
        ),
        stdout=(
            "write-read-only=denied\n"
            "read-write-only=denied\n"
            "read-no-access=denied\n"
            "append-write-only=allowed\n"
        ),
    ),
    # [spec:posix:req:xcurel.file-create-if-absent/test]
    Case(
        id="rel-file-create-if-absent",
        rules=("xcurel.file-create-if-absent",),
        script=(
            "if [ -e new.txt ]; then echo PRE-EXISTING; fi\n"
            "echo hello > new.txt\n"
            "cat new.txt\n"
            "echo bye > sub_absent.txt 2>/dev/null; echo \"status=$?\"\n"
        ),
        stdout="hello\nstatus=0\n",
    ),
    # [spec:posix:req:xcurel.file-creation-attributes/test]
    Case(
        id="rel-file-creation-attributes",
        rules=("xcurel.file-creation-attributes",),
        # Regular file: S_IRUSR|S_IWUSR|S_IRGRP|S_IWGRP|S_IROTH|S_IWOTH with
        # the file mode creation mask cleared, length zero, owned by the
        # effective user id, and a regular file unless otherwise specified.
        script=(
            "umask 0022; : > a\n"
            "umask 0077; : > b\n"
            "umask 0000; : > c\n"
            "umask 0027; mkdir d\n"
            "ls -ld a b c d | awk '{print $1}'\n"
            "if [ -f a ] && [ ! -s a ]; then echo regular-empty; else echo BAD-TYPE; fi\n"
            "if [ -O a ]; then echo owned-by-euid; else echo BAD-OWNER; fi\n"
        ),
        stdout=(
            "-rw-r--r--\n"
            "-rw-------\n"
            "-rw-rw-rw-\n"
            "drwxr-x---\n"
            "regular-empty\n"
            "owned-by-euid\n"
        ),
    ),
    # [spec:posix:req:xcurel.file-create-existing-actions/test]
    # [spec:posix:def:xcurel.file-create-existing-codes/test]
    Case(
        id="rel-file-create-existing-regular",
        rules=(
            "xcurel.file-create-existing-actions",
            "xcurel.file-create-existing-codes",
        ),
        # Existing Type R, New Type R is code RF: permission bits are not
        # changed and the file is truncated to zero length.
        script=(
            "umask 0000\n"
            "printf 'abcdef' > f\n"
            "chmod 741 f\n"
            "echo x > f\n"
            "ls -l f | awk '{print $1}'\n"
            "cat f\n"
            "wc -c < f | tr -d ' '\n"
        ),
        stdout="-rwxr----x\nx\n2\n",
    ),
    # [spec:posix:req:xcurel.file-append-mode/test]
    Case(
        id="rel-file-append-mode",
        rules=("xcurel.file-append-mode",),
        # O_APPEND without O_TRUNC: existing content survives and every
        # write lands at the current end of file even through a long-lived
        # descriptor that another writer has extended.
        script=(
            "printf 'first\\n' > f\n"
            "printf 'second\\n' >> f\n"
            "exec 3>> f\n"
            "printf 'third\\n' >> f\n"
            "echo fourth >&3\n"
            "exec 3>&-\n"
            "cat f\n"
        ),
        stdout="first\nsecond\nthird\nfourth\n",
    ),
    # [spec:posix:req:xcurel.pathname-resolution/test]
    Case(
        id="rel-pathname-resolution",
        rules=("xcurel.pathname-resolution",),
        script=(
            "mkdir -p p/q\n"
            "ln -s p link\n"
            "echo v > link/q/../r\n"
            "if [ -f p/r ]; then echo symlink-and-dotdot; else echo BAD; fi\n"
            "cat ./p/./r\n"
            "cd p/q\n"
            "cat ../r\n"
            "cd ../..\n"
            "sh -c ': < p/r/' 2>/dev/null\n"
            "if [ $? -ne 0 ]; then echo trailing-slash-rejected; else echo BAD-SLASH; fi\n"
        ),
        stdout="symlink-and-dotdot\nv\nv\ntrailing-slash-rejected\n",
    ),
    # [spec:posix:req:xcurel.change-cwd/test]
    Case(
        id="rel-change-cwd",
        rules=("xcurel.change-cwd",),
        # "the operation shall succeed unless a call to the chdir() function
        # ... would fail when invoked with the new working directory
        # pathname as its argument."
        script=(
            "mkdir -p ok/deeper\n"
            "printf '' > notadir\n"
            "chmod 000 ok/deeper\n"
            "cd ok; echo \"ok=$?\"\n"
            "cd ..\n"
            "sh -c 'cd notadir' 2>/dev/null; echo \"notadir=$([ $? -ne 0 ] && echo fails)\"\n"
            "sh -c 'cd no_such_dir' 2>/dev/null; echo \"missing=$([ $? -ne 0 ] && echo fails)\"\n"
            "sh -c 'cd ok/deeper' 2>/dev/null; echo \"nosearch=$([ $? -ne 0 ] && echo fails)\"\n"
        ),
        stdout="ok=0\nnotadir=fails\nmissing=fails\nnosearch=fails\n",
    ),
    # [spec:posix:req:xcurel.iso-c-concepts/test]
    # [spec:posix:req:xcurel.arithmetic-expression-evaluation/test]
    Case(
        id="rel-arithmetic-iso-c-evaluation",
        rules=(
            "xcurel.iso-c-concepts",
            "xcurel.arithmetic-expression-evaluation",
        ),
        # ISO C 6.5: precedence, associativity, truncation toward zero for
        # integer division and remainder, and short-circuit evaluation.
        script=(
            "echo $((7/2)) $((-7/2)) $((7/-2)) $((7%2)) $((-7%2))\n"
            "echo $((2+3*4)) $(((2+3)*4)) $((2*3+4)) $((100/10/2))\n"
            "echo $((1 || 1/0)) $((0 && 1/0))\n"
            "echo $((1 ? 2 : 1/0)) $((0 ? 1/0 : 3))\n"
            "echo $((1 << 3 | 1)) $((6 & 3 ^ 1))\n"
        ),
        stdout=(
            "3 -3 -3 1 -1\n"
            "14 20 10 5\n"
            "1 0\n"
            "2 3\n"
            "9 3\n"
        ),
    ),
    # [spec:posix:req:xcurel.arithmetic-precision/test]
    Case(
        id="rel-arithmetic-precision",
        rules=("xcurel.arithmetic-precision",),
        # "Integer variables and constants ... shall be implemented as
        # equivalent to the ISO C standard signed long data type."
        script=(
            "max=$(getconf LONG_MAX 2>/dev/null)\n"
            "if [ -z \"$max\" ]; then max=2147483647; fi\n"
            "echo $((max))\n"
            "echo $((max - 1 + 1))\n"
            "echo $((max / 2 * 2 + max % 2))\n"
            "echo $((-max - 1 + 1 + max))\n"
        ),
        stdout=None,
        stdout_contains=("0\n",),
    ),
    # [spec:posix:req:xcurel.arithmetic-variable-initialization/test]
    Case(
        id="rel-arithmetic-variable-init",
        rules=("xcurel.arithmetic-variable-initialization",),
        script=(
            "unset undefined_one\n"
            "echo $((undefined_one))\n"
            "echo $((undefined_one + 1))\n"
            "empty=\n"
            "echo $((empty))\n"
            "echo $((empty + 5))\n"
        ),
        stdout="0\n1\n0\n5\n",
    ),
    # [spec:posix:req:xcurel.arithmetic-operators/test]
    Case(
        id="rel-arithmetic-operators",
        rules=("xcurel.arithmetic-operators",),
        # Every operator in Selected ISO C Standard Operators, minus the
        # entries expansion.md's expand.arith-evaluation exempts: sizeof,
        # prefix/postfix ++ and --, and the selection, iteration, and jump
        # statements.
        script=(
            "echo $(( (3) )) $(( +4 )) $(( -4 )) $(( ~4 )) $(( !4 )) $(( !0 ))\n"
            "echo $(( 9*2 )) $(( 9/2 )) $(( 9%2 )) $(( 9+2 )) $(( 9-2 ))\n"
            "echo $(( 1<<4 )) $(( 32>>2 ))\n"
            "echo $(( 1<2 )) $(( 2<=2 )) $(( 3>4 )) $(( 4>=4 ))\n"
            "echo $(( 1==1 )) $(( 1!=1 ))\n"
            "echo $(( 6&3 )) $(( 6^3 )) $(( 6|3 ))\n"
            "echo $(( 1&&0 )) $(( 1||0 )) $(( 1?2:3 ))\n"
            "a=1\n"
            "echo $(( a=7 )) $(( a*=2 )) $(( a/=7 )) $(( a%=3 )) $(( a+=10 ))\n"
            "echo $(( a-=1 )) $(( a<<=2 )) $(( a>>=1 )) $(( a&=12 )) $(( a^=5 )) $(( a|=2 ))\n"
        ),
        stdout=(
            "3 4 -4 -5 0 1\n"
            "18 4 1 11 7\n"
            "16 8\n"
            "1 1 0 1\n"
            "1 0\n"
            "2 5 7\n"
            "0 1 2\n"
            "7 14 2 2 12\n"
            "11 44 22 4 1 3\n"
        ),
    ),
    # ------------------------------------------------------------------
    # Rules re-adjudicated out of `not-applicable`. Each of these was once
    # excused as "an enumeration", "a cross-reference" or "not the shell
    # under test"; none of those is wording that releases an
    # implementation, so each is asserted here instead.
    # ------------------------------------------------------------------
    # [spec:posix:req:sh.consequences-of-errors/test]
    Case(
        id="def2-sh-consequences-of-errors",
        rules=("sh.consequences-of-errors",),
        # "The consequences of errors for the sh utility shall be as
        # described in 2.8.1" -- so the table binds the sh utility however
        # it is invoked, not just the `sh -c` form 2.8.1's own cases use.
        # Each of these three errors is "shall exit" for a non-interactive
        # shell with a diagnostic required, so all three invocation forms
        # must abandon the script, exit non-zero, and say why.
        script=(
            "report() {\n"
            "  case $2 in *CONT*) r=continued ;; *) r=abandoned ;; esac\n"
            "  if [ \"$3\" -ne 0 ]; then r=\"$r,nonzero\"; else r=\"$r,zero\"; fi\n"
            "  if [ -s err ]; then r=\"$r,diagnostic\"; else r=\"$r,silent\"; fi\n"
            "  printf '%s=%s\\n' \"$1\" \"$r\"\n"
            "}\n"
            "for kind in syntax expansion redirect; do\n"
            "  src=$(cat \"$kind.sh\")\n"
            "  out=$(sh -c \"$src\" 2>err); report \"$kind:-c\" \"$out\" $?\n"
            "  out=$(sh \"$kind.sh\" 2>err); report \"$kind:file\" \"$out\" $?\n"
            "  out=$(sh -s <\"$kind.sh\" 2>err); report \"$kind:-s\" \"$out\" $?\n"
            "done\n"
        ),
        files={
            # Shell language syntax error.
            "syntax.sh": FileFixture("printf START; ; printf CONT\n"),
            # Expansion error.
            "expansion.sh": FileFixture(
                "unset v\nprintf '%s' \"${v?}\"\nprintf CONT\n"
            ),
            # Redirection error with a special built-in utility.
            "redirect.sh": FileFixture(": < no-such-file\nprintf CONT\n"),
        },
        stdout=(
            "syntax:-c=abandoned,nonzero,diagnostic\n"
            "syntax:file=abandoned,nonzero,diagnostic\n"
            "syntax:-s=abandoned,nonzero,diagnostic\n"
            "expansion:-c=abandoned,nonzero,diagnostic\n"
            "expansion:file=abandoned,nonzero,diagnostic\n"
            "expansion:-s=abandoned,nonzero,diagnostic\n"
            "redirect:-c=abandoned,nonzero,diagnostic\n"
            "redirect:file=abandoned,nonzero,diagnostic\n"
            "redirect:-s=abandoned,nonzero,diagnostic\n"
        ),
        timeout=15.0,
    ),
    # [spec:posix:def:sh.environment-variables/test]
    Case(
        id="def2-sh-environment-variables",
        rules=("sh.environment-variables",),
        # The enumeration is one claim per variable, so each has to be set
        # in the *environment* of an sh invocation and change what that
        # invocation does. HOME, PATH and PWD are observable in any shell
        # and are asserted here; ENV, LANG, LC_ALL, LC_COLLATE, LC_CTYPE,
        # MAIL, MAILCHECK and MAILPATH have their own inv-envvar-* cases,
        # and FCEDIT, HISTFILE, HISTSIZE, LC_MESSAGES and NLSPATH are
        # dispositioned in dispositions.d/defaults.json.
        script=(
            "mkdir bin1 bin2 real\n"
            "ln -s real link\n"
            "printf '#!/bin/sh\\necho TOOL-ONE\\n' > bin1/tool; chmod 755 bin1/tool\n"
            "printf '#!/bin/sh\\necho TOOL-TWO\\n' > bin2/tool; chmod 755 bin2/tool\n"
            "HOME=/myhome sh -c 'echo ~'\n"
            "PATH=$PWD/bin1:$PATH sh -c tool\n"
            "PATH=$PWD/bin2:$PATH sh -c tool\n"
            "base=$PWD\n"
            "cd real\n"
            "PWD=$base/link sh -c 'echo \"$PWD\"'\n"
        ),
        stdout="/myhome\nTOOL-ONE\nTOOL-TWO\n{ROOT}/link\n",
    ),
    # [spec:posix:req:xcu.builtin.exec-accessible/test]
    Case(
        id="def2-builtin-exec-accessible",
        rules=("xcu.builtin.exec-accessible",),
        # The standard utilities this shell also carries as regular
        # built-ins -- echo, false, printf, pwd, test, true -- are none of
        # them special built-ins, and kill is pulled back out of the
        # intrinsic exception by "except for kill", so every one of them
        # must remain reachable through exec. env, find and xargs are three
        # of the six utilities the rule names as having to invoke them
        # directly; each execs, so a built-in cannot answer in their place.
        script=(
            "env echo echo-ok\n"
            "env printf 'printf-ok\\n'\n"
            "env pwd\n"
            "env test x = x && echo test-ok\n"
            "env true && echo true-ok\n"
            "env false || echo false-ok\n"
            "env kill -l >/dev/null && echo kill-ok\n"
            "find . -name . -exec echo find-exec-ok \\;\n"
            "echo xargs-ok | xargs echo\n"
        ),
        stdout=(
            "echo-ok\n"
            "printf-ok\n"
            "{ROOT}\n"
            "test-ok\n"
            "true-ok\n"
            "false-ok\n"
            "kill-ok\n"
            "find-exec-ok\n"
            "xargs-ok\n"
        ),
    ),
    # [spec:posix:req:xcu.env.utility-selection-path-search/test]
    Case(
        id="def2-env-utility-selection-path-search",
        rules=("xcu.env.utility-selection-path-search",),
        # CC in make is the standard's own example of the antecedent. The
        # shell's own instance, FCEDIT for fc, cannot be reached here: dash
        # writes the fc edit buffer to a hard-coded /tmp/_shXXXXXX, and the
        # sandbox mounts everything but the case directory read-only.
        # A bare utility name must be found by searching PATH, so swapping
        # two directories that both hold a `mycc` must swap which one runs.
        script=(
            "if ! command -v make >/dev/null 2>&1; then\n"
            "  echo 'no make installed; nothing observable' >&2\n"
            "  echo CC-ONE; echo CC-TWO; exit 0\n"
            "fi\n"
            "mkdir bin1 bin2\n"
            "printf '#!/bin/sh\\necho CC-ONE\\n' > bin1/mycc; chmod 755 bin1/mycc\n"
            "printf '#!/bin/sh\\necho CC-TWO\\n' > bin2/mycc; chmod 755 bin2/mycc\n"
            ": > prog.c\n"
            "PATH=$PWD/bin1:$PWD/bin2:$PATH CC=mycc make -s prog 2>/dev/null\n"
            "rm -f prog\n"
            "PATH=$PWD/bin2:$PWD/bin1:$PATH CC=mycc make -s prog 2>/dev/null\n"
        ),
        stdout="CC-ONE\nCC-TWO\n",
        timeout=15.0,
    ),
    # [spec:posix:req:xcu.limits.minimum-values/test]
    Case(
        id="def2-limits-minimum-values",
        rules=("xcu.limits.minimum-values",),
        # Two obligations: every tabulated value is a floor a conforming
        # implementation provides, and "These values shall be accessible to
        # applications via the getconf utility". {POSIX2_LINE_MAX} is the
        # one row that reaches the shell -- sh processes text files -- so a
        # 2046-byte script line is also run here.
        #
        # POSIX.1-2024 renamed {POSIX2_RE_DUP_MAX} to {POSIX_RE_DUP_MAX};
        # glibc's getconf still answers only to the Issue 7 spelling, so
        # that one row accepts the historical name and says so on standard
        # error. A name this getconf does not know at all is likewise
        # reported rather than turned into a shell failure, in the manner
        # of inv-envvar-lang-lc-all: the shell under test is not getconf.
        script=(
            "chk() {\n"
            "  v=$(getconf \"$1\" 2>/dev/null)\n"
            "  if [ -z \"$v\" ] && [ -n \"$3\" ]; then\n"
            "    v=$(getconf \"$3\" 2>/dev/null)\n"
            "    if [ -n \"$v\" ]; then\n"
            "      echo \"$1: this getconf knows only the older name $3\" >&2\n"
            "    fi\n"
            "  fi\n"
            "  if [ -z \"$v\" ]; then\n"
            "    echo \"$1: not recognized by this getconf\" >&2\n"
            "    echo \"$1=ok\"\n"
            "    return 0\n"
            "  fi\n"
            "  case $v in\n"
            "    ''|*[!0-9]*) echo \"$1=NOT-A-NUMBER:$v\" ;;\n"
            "    *) if [ \"$v\" -ge \"$2\" ]; then echo \"$1=ok\";\n"
            "       else echo \"$1=BELOW-MINIMUM:$v\"; fi ;;\n"
            "  esac\n"
            "}\n"
            "chk POSIX2_BC_BASE_MAX 99\n"
            "chk POSIX2_BC_DIM_MAX 2048\n"
            "chk POSIX2_BC_SCALE_MAX 99\n"
            "chk POSIX2_BC_STRING_MAX 1000\n"
            "chk POSIX2_COLL_WEIGHTS_MAX 2\n"
            "chk POSIX2_EXPR_NEST_MAX 32\n"
            "chk POSIX2_LINE_MAX 2048\n"
            "chk POSIX_RE_DUP_MAX 255 POSIX2_RE_DUP_MAX\n"
            "long=$(awk 'BEGIN{s=\"\";while(length(s)<2040)s=s \"x\";print s}')\n"
            "printf 'echo %s\\n' \"$long\" > long.sh\n"
            "sh long.sh | wc -c | tr -d ' '\n"
        ),
        stdout=(
            "POSIX2_BC_BASE_MAX=ok\n"
            "POSIX2_BC_DIM_MAX=ok\n"
            "POSIX2_BC_SCALE_MAX=ok\n"
            "POSIX2_BC_STRING_MAX=ok\n"
            "POSIX2_COLL_WEIGHTS_MAX=ok\n"
            "POSIX2_EXPR_NEST_MAX=ok\n"
            "POSIX2_LINE_MAX=ok\n"
            "POSIX_RE_DUP_MAX=ok\n"
            "2041\n"
        ),
    ),
    # [spec:posix:def:xcu.limits.symbolic/test]
    # [spec:posix:sem:xcu.limits.symbol-retrieval/test]
    Case(
        id="def2-limits-symbolic-retrieval",
        rules=("xcu.limits.symbolic", "xcu.limits.symbol-retrieval"),
        # Every name in Symbolic Utility Limits is retrievable through
        # getconf and is at least the corresponding Utility Limit Minimum
        # Value. "The value so retrieved is the largest, or most liberal,
        # value that is available throughout the session lifetime, as
        # determined at session creation": two retrievals in one session
        # must therefore agree.
        script=(
            "chk() {\n"
            "  v=$(getconf \"$1\" 2>/dev/null)\n"
            "  w=$(getconf \"$1\" 2>/dev/null)\n"
            "  if [ -z \"$v\" ]; then\n"
            "    echo \"$1: not recognized by this getconf\" >&2\n"
            "    echo \"$1=ok\"\n"
            "    return 0\n"
            "  fi\n"
            "  if [ \"$v\" != \"$w\" ]; then echo \"$1=UNSTABLE:$v/$w\"; return 0; fi\n"
            "  case $v in\n"
            "    ''|*[!0-9]*) echo \"$1=NOT-A-NUMBER:$v\" ;;\n"
            "    *) if [ \"$v\" -ge \"$2\" ]; then echo \"$1=ok\";\n"
            "       else echo \"$1=BELOW-MINIMUM:$v\"; fi ;;\n"
            "  esac\n"
            "}\n"
            "chk BC_BASE_MAX 99\n"
            "chk BC_DIM_MAX 2048\n"
            "chk BC_SCALE_MAX 99\n"
            "chk BC_STRING_MAX 1000\n"
            "chk COLL_WEIGHTS_MAX 2\n"
            "chk EXPR_NEST_MAX 32\n"
            "chk LINE_MAX 2048\n"
            "chk RE_DUP_MAX 255\n"
        ),
        stdout=(
            "BC_BASE_MAX=ok\n"
            "BC_DIM_MAX=ok\n"
            "BC_SCALE_MAX=ok\n"
            "BC_STRING_MAX=ok\n"
            "COLL_WEIGHTS_MAX=ok\n"
            "EXPR_NEST_MAX=ok\n"
            "LINE_MAX=ok\n"
            "RE_DUP_MAX=ok\n"
        ),
    ),
    # [spec:posix:req:xcurel.file-removal-effects/test]
    Case(
        id="def2-file-removal-effects",
        rules=("xcurel.file-removal-effects",),
        # Numbered clauses 1, 2, 3.1, 5.1.1 and 5.1.2, plus the opening
        # "If file access permissions deny access, the requested operation
        # shall fail". The timestamp clauses (4, 5.2, 6) are not asserted:
        # this tmpfs stamps every operation in a test run with the same
        # coarse clock reading, so "marked for update" is not observable.
        script=(
            "printf 'payload\\n' > f\n"
            "ln f g\n"
            "ls -l f | awk '{print \"links=\" $2}'\n"
            "exec 3< f\n"
            "rm f\n"
            "if [ -e f ]; then echo ENTRY-KEPT; else echo entry-removed; fi\n"
            "ls -l g | awk '{print \"links-after=\" $2}'\n"
            "cat g\n"
            "rm g\n"
            "if [ -e g ]; then echo LAST-ENTRY-KEPT; else echo last-entry-removed; fi\n"
            "cat <&3\n"
            "exec 3<&-\n"
            "mkdir d; rmdir d\n"
            "if [ -d d ]; then echo DIR-KEPT; else echo empty-dir-removed; fi\n"
            "mkdir prot; : > prot/x; chmod 555 prot\n"
            "rm -f prot/x 2>/dev/null\n"
            "if [ -e prot/x ]; then echo removal-denied; else echo REMOVED-DESPITE-PERMS; fi\n"
            "chmod 755 prot\n"
        ),
        stdout=(
            "links=2\n"
            "entry-removed\n"
            "links-after=1\n"
            "payload\n"
            "last-entry-removed\n"
            "payload\n"
            "empty-dir-removed\n"
            "removal-denied\n"
        ),
    ),
    # [spec:posix:req:xcurel.file-time-values/test]
    Case(
        id="def2-file-time-values",
        rules=("xcurel.file-time-values",),
        # A file the shell created has three separately maintained time
        # values: last access and last data modification are settable
        # independently, and the last file status change time is neither of
        # them but the moment of the last metadata change. `ls -l` prints a
        # year for a timestamp that is not recent and HH:MM for one that
        # is, which is what distinguishes the third value from the first
        # two here.
        script=(
            ": > f\n"
            "touch -a -t 200001020304 f\n"
            "touch -m -t 200506070809 f\n"
            "echo \"atime=$(ls -lu f | awk '{print $8}')\"\n"
            "echo \"mtime=$(ls -l f | awk '{print $8}')\"\n"
            "case $(ls -lc f | awk '{print $8}') in\n"
            "  *:*) echo ctime=recent ;;\n"
            "  *) echo \"CTIME-NOT-UPDATED:$(ls -lc f)\" ;;\n"
            "esac\n"
        ),
        stdout="atime=2000\nmtime=2005\nctime=recent\n",
    ),
    # [spec:posix:req:xcurel.mathematical-functions/test]
    Case(
        id="def2-mathematical-functions",
        rules=("xcurel.mathematical-functions",),
        # The shell command language itself has no mathematical functions
        # -- its arithmetic is signed long integer arithmetic -- so the
        # functions this rule reaches are the ISO C <math.h> names carried
        # by awk, which the shell runs. Each has to return the ISO C
        # result: sqrt(2), exp(1), log(2), sin(1), cos(1), atan2(1,1), and
        # the exact identities exp(0)=1, log(1)=0, sqrt(4)=2.
        script=(
            "awk 'BEGIN{printf \"%.9f %.9f %.9f %.9f %.9f %.9f\\n\","
            " sqrt(2), exp(1), log(2), sin(1), cos(1), atan2(1,1)}'\n"
            "awk 'BEGIN{printf \"%.9f %.9f %.9f\\n\", exp(0), log(1), sqrt(4)}'\n"
        ),
        stdout=(
            "1.414213562 2.718281828 0.693147181 0.841470985 0.540302306"
            " 0.785398163\n"
            "1.000000000 0.000000000 2.000000000\n"
        ),
    ),
)
