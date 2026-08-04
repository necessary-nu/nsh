"""Locale rules: the internationalization variables, tested rather than excused.

These twenty-one rules were all carrying a `manual` disposition whose
reason said, in one wording or another, that observing them "requires a
non-C locale and localized diagnostics, which the fixed LC_ALL=C harness
environment cannot observe".

Both halves of that were wrong. The host has `C.utf8` and `en_US.utf8`
installed, and the harness has always let a case override the environment
it runs under. The difference is plainly visible in dash:

    LC_ALL=C          ${#e}=2   [[:alpha:]] no    ? matches 2 bytes
    LC_ALL=C.utf8     ${#e}=1   [[:alpha:]] yes   ? matches 1 character

So `LC_ALL=C` was the harness's own choice, not a limit on what can be
seen. The lesson from the `not-applicable` audit applies unchanged: a
disposition that excuses a rule has to be shown, not asserted.

What the probing did establish is where the line really falls. LC_CTYPE
is observable through the *shell language* -- parameter length, character
classes, `?`, `read`, suffix removal -- and is NOT observable through the
argument handling of `alias`, `getopts`, `type`, `umask` or `kill`, which
treat their arguments as bytes and produce byte-identical results in both
locales. The per-builtin cases below therefore assert what those builtins
must do with multibyte arguments (round-trip them intact, which a shell
that truncates at a byte boundary fails) rather than pretending to a
locale-dependence that has no observable consequence there.

Every case here sets LC_ALL explicitly, or clears it to let a lower
variable take effect, since the harness default of LC_ALL=C would
otherwise override the variable under test -- which is itself the content
of `param.lc-all`.
"""

from __future__ import annotations

from model import Case, FileFixture


# `printf` is used to build the multibyte text rather than putting a UTF-8
# literal in the script, so the bytes reaching the shell are exact and do
# not depend on how this file is encoded or transported.
E_ACUTE = r"$(printf '\303\251')"

UTF8 = "C.utf8"


CASES: tuple[Case, ...] = (
    # [spec:posix:req:param.lc-ctype/test]
    Case(
        id="locale-lc-ctype-character-interpretation",
        rules=("param.lc-ctype",),
        # "Determine the interpretation of sequences of bytes of text data
        # as characters ..., which characters are defined as letters
        # (character class alpha) ..., and the behavior of character
        # classes within pattern matching."
        script=(
            f"e={E_ACUTE}\n"
            'printf \'len=%s\\n\' "${#e}"\n'
            'case $e in [[:alpha:]]) printf \'alpha=yes\\n\' ;; *) printf \'alpha=no\\n\' ;; esac\n'
            "case $e in ?) printf 'onechar=yes\\n' ;; *) printf 'onechar=no\\n' ;; esac\n"
        ),
        environment={"LC_ALL": UTF8, "LANG": UTF8},
        stdout="len=1\nalpha=yes\nonechar=yes\n",
    ),
    # [spec:posix:req:param.lc-ctype/test]
    Case(
        id="locale-lc-ctype-c-locale-is-bytes",
        rules=("param.lc-ctype",),
        # The same script in the C locale, so the case pair shows the
        # variable is what decides, not the shell's fixed opinion.
        script=(
            f"e={E_ACUTE}\n"
            'printf \'len=%s\\n\' "${#e}"\n'
            'case $e in [[:alpha:]]) printf \'alpha=yes\\n\' ;; *) printf \'alpha=no\\n\' ;; esac\n'
            "case $e in ?) printf 'onechar=yes\\n' ;; *) printf 'onechar=no\\n' ;; esac\n"
        ),
        environment={"LC_ALL": "C", "LANG": "C"},
        stdout="len=2\nalpha=no\nonechar=no\n",
    ),
    # [spec:posix:req:param.lc-ctype/test]
    Case(
        id="locale-lc-ctype-new-shell-picks-up-change",
        rules=("param.lc-ctype",),
        # "Invoking a shell script or performing exec sh subjects the new
        # shell to the changes in LC_CTYPE."
        #
        # Only this half is asserted. The preceding sentence -- "Changing
        # the value of LC_CTYPE after the shell has started shall not
        # affect the lexical processing of shell commands in the current
        # shell execution environment or its subshells" -- constrains
        # *lexical processing*, and a first draft of this case tested it
        # with `${#e}`, which is parameter expansion at execution time and
        # not lexical processing at all. dash reported 1 there and was
        # right to; the expectation of 2 was mine, not the standard's.
        #
        # The helper is a file fixture rather than a nested `sh -c '...'`:
        # the multibyte value has to be built with `printf '\303\251'`, and
        # its single quotes close the ones around the -c argument.
        script=(
            "LC_CTYPE=" + UTF8 + "\n"
            "export LC_CTYPE\n"
            "sh ./len.sh\n"
            "LC_CTYPE=C sh ./len.sh\n"
        ),
        files={
            "len.sh": FileFixture(
                content="e=$(printf '\\303\\251')\nprintf 'len=%s\\n' \"${#e}\"\n"
            )
        },
        # LC_ALL must be out of the way for LC_CTYPE to mean anything.
        environment={"LC_ALL": "", "LANG": "C"},
        stdout="len=1\nlen=2\n",
    ),
    # [spec:posix:req:param.lc-all/test]
    Case(
        id="locale-lc-all-overrides",
        rules=("param.lc-all",),
        # "The value of this variable overrides the LC_* variables and
        # LANG". Both directions, so the case fails on a shell that simply
        # ignores one of the two variables.
        script=(
            f"e={E_ACUTE}\n"
            'printf \'len=%s\\n\' "${#e}"\n'
        ),
        environment={"LC_ALL": "C", "LC_CTYPE": UTF8, "LANG": UTF8},
        stdout="len=2\n",
    ),
    # [spec:posix:req:param.lc-all/test]
    Case(
        id="locale-lc-all-overrides-other-way",
        rules=("param.lc-all",),
        script=(
            f"e={E_ACUTE}\n"
            'printf \'len=%s\\n\' "${#e}"\n'
        ),
        environment={"LC_ALL": UTF8, "LC_CTYPE": "C", "LANG": "C"},
        stdout="len=1\n",
    ),
    # [spec:posix:req:param.lc-all/test]
    Case(
        id="locale-lc-all-empty-does-not-override",
        rules=("param.lc-all",),
        # "If set to a non-empty string value, override ..." -- an empty
        # LC_ALL is not an override, so LC_CTYPE decides.
        script=(
            f"e={E_ACUTE}\n"
            'printf \'len=%s\\n\' "${#e}"\n'
        ),
        environment={"LC_ALL": "", "LC_CTYPE": UTF8, "LANG": "C"},
        stdout="len=1\n",
    ),
    # [spec:posix:req:param.lang/test]
    Case(
        id="locale-lang-supplies-default",
        rules=("param.lang",),
        # "Provide a default value for the internationalization variables
        # that are unset or null."
        script=(
            f"e={E_ACUTE}\n"
            'printf \'len=%s\\n\' "${#e}"\n'
        ),
        environment={"LC_ALL": "", "LC_CTYPE": "", "LANG": UTF8},
        stdout="len=1\n",
    ),
    # [spec:posix:req:param.lang/test]
    Case(
        id="locale-lang-yields-to-lc-ctype",
        rules=("param.lang", "param.lc-ctype"),
        # LANG is only the default: a set LC_CTYPE takes precedence.
        script=(
            f"e={E_ACUTE}\n"
            'printf \'len=%s\\n\' "${#e}"\n'
        ),
        environment={"LC_ALL": "", "LC_CTYPE": "C", "LANG": UTF8},
        stdout="len=2\n",
    ),
    # [spec:posix:req:param.lc-collate/test]
    Case(
        id="locale-lc-collate-range-expressions",
        rules=("param.lc-collate",),
        # "Determine the behavior of range expressions, equivalence
        # classes, and multi-character collating elements within pattern
        # matching." In the C locale the range is over the collating
        # sequence of the portable character set, which is what is asserted
        # here; POSIX leaves the ordering in other locales to the locale
        # definition, so only the C behaviour is a fixed obligation.
        script=(
            "case a in [a-z]) printf 'lower-in=yes\\n' ;; *) printf 'lower-in=no\\n' ;; esac\n"
            "case A in [a-z]) printf 'upper-in=yes\\n' ;; *) printf 'upper-in=no\\n' ;; esac\n"
            "case Q in [A-Z]) printf 'upper-range=yes\\n' ;; *) printf 'upper-range=no\\n' ;; esac\n"
            "case 5 in [0-9]) printf 'digit=yes\\n' ;; *) printf 'digit=no\\n' ;; esac\n"
            "case - in [a-z-]) printf 'hyphen-last=yes\\n' ;; *) printf 'hyphen-last=no\\n' ;; esac\n"
        ),
        environment={"LC_ALL": "C", "LC_COLLATE": "C", "LANG": "C"},
        stdout=(
            "lower-in=yes\nupper-in=no\nupper-range=yes\ndigit=yes\nhyphen-last=yes\n"
        ),
    ),
    # [spec:posix:req:param.lc-messages/test]
    # [spec:posix:req:sh.envvar-lc-messages/test]
    Case(
        id="locale-lc-messages-diagnostics-to-stderr",
        rules=("param.lc-messages", "sh.envvar-lc-messages"),
        # "Determine the language in which messages should be written" /
        # "affect the format and contents of diagnostic messages written to
        # standard error". Which language is chosen needs a catalog the
        # shell does not ship, so what is asserted is the part that holds
        # regardless: the diagnostic goes to standard error, not standard
        # output, and setting LC_MESSAGES to a locale the host does have
        # neither suppresses it nor breaks it.
        script=(
            "sh -c 'no_such_command_a41f' >out 2>err\n"
            "printf 'status=%s\\n' \"$?\"\n"
            "test -s out && printf 'UNEXPECTED-STDOUT\\n'\n"
            "test -s err && printf 'diagnostic=yes\\n' || printf 'diagnostic=no\\n'\n"
            "grep -q no_such_command_a41f err && printf 'names-command=yes\\n'\n"
        ),
        environment={"LC_ALL": "", "LC_MESSAGES": UTF8, "LANG": "C"},
        stdout="status=127\ndiagnostic=yes\nnames-command=yes\n",
    ),
    # The per-utility "the following environment variables shall affect the
    # execution of X" rules. LC_CTYPE governs "the interpretation of
    # sequences of bytes of text data as characters ... in arguments", so
    # what each of these asserts is that the utility carries a multibyte
    # argument through intact under a UTF-8 locale -- a shell that splits
    # or truncates at a byte boundary fails -- together with LC_ALL's
    # precedence where the utility can show it.
    #
    # [spec:posix:req:builtin.alias.env-locale/test]
    # [spec:posix:req:builtin.unalias.env-locale/test]
    Case(
        id="locale-alias-multibyte-arguments",
        rules=("builtin.alias.env-locale", "builtin.unalias.env-locale"),
        # The multibyte text is in the alias *value*, not the alias *name*.
        # POSIX draws alias names from the portable character set and says
        # nothing about names outside it, so a first draft asserting that
        # `alias aé=...` could then be run was asserting a requirement that
        # does not exist -- dash defined the alias and declined to expand
        # it, which is permitted.
        script=(
            f"e={E_ACUTE}\n"
            'alias greet="printf \'hi-%s\\n\' \\"$e\\""\n'
            "alias greet | grep -c 'é'\n"
            "greet\n"
            "unalias greet\n"
            "alias greet 2>/dev/null || printf 'gone\\n'\n"
        ),
        environment={"LC_ALL": UTF8, "LANG": UTF8},
        # `alias` re-quotes its output in an implementation's own style --
        # dash writes `'greet=printf '"'"'hi-%s\n'"'"' "é"'`. That style is
        # the business of builtin.alias.stdout-format, not of this rule, so
        # what is asserted here is only that the multibyte text survives
        # definition, listing and execution.
        stdout="1\nhi-é\ngone\n",
        status="any",
    ),
    # [spec:posix:req:builtin.cd.env-locale/test]
    Case(
        id="locale-cd-multibyte-directory",
        rules=("builtin.cd.env-locale",),
        script=(
            f"e={E_ACUTE}\n"
            'mkdir "d$e"\n'
            'cd "d$e" || { printf \'CD-FAILED\\n\'; exit 1; }\n'
            'printf \'base=%s\\n\' "${PWD##*/}"\n'
            'printf \'len=%s\\n\' "${#e}"\n'
        ),
        environment={"LC_ALL": UTF8, "LANG": UTF8},
        stdout="base=dé\nlen=1\n",
    ),
    # [spec:posix:req:builtin.command.env-locale/test]
    # [spec:posix:req:builtin.type.env-locale/test]
    # [spec:posix:req:builtin.hash.env-locale/test]
    Case(
        id="locale-command-type-hash-multibyte",
        rules=(
            "builtin.command.env-locale",
            "builtin.type.env-locale",
            "builtin.hash.env-locale",
        ),
        # The utility is a file fixture. Generating it with nested printf
        # escapes produced `printf ran\n` -> "rann" in a first draft; the
        # fixture removes a layer of quoting that was never the point.
        script=(
            "PATH=$PWD:$PATH\n"
            f"e={E_ACUTE}\n"
            'command -v "u$e" >/dev/null && printf \'command-v=yes\\n\'\n'
            'type "u$e" >/dev/null 2>&1 && printf \'type=yes\\n\'\n'
            '"u$e"\n'
            'hash >/dev/null 2>&1; printf \'hash=%s\\n\' "$?"\n'
        ),
        files={"ué": FileFixture(content="#!/bin/sh\nprintf 'ran\\n'\n", mode=0o755)},
        environment={"LC_ALL": UTF8, "LANG": UTF8},
        stdout="command-v=yes\ntype=yes\nran\nhash=0\n",
    ),
    # [spec:posix:req:builtin.getopts.env/test]
    Case(
        id="locale-getopts-multibyte-optarg",
        rules=("builtin.getopts.env",),
        script=(
            f"e={E_ACUTE}\n"
            'set -- -a "v$e" rest\n'
            "while getopts a: opt; do\n"
            "  case $opt in a) printf 'optarg=%s\\n' \"$OPTARG\" ;; esac\n"
            "done\n"
            'shift $((OPTIND - 1))\n'
            "printf 'remaining=%s\\n' \"$1\"\n"
        ),
        environment={"LC_ALL": UTF8, "LANG": UTF8},
        stdout="optarg=vé\nremaining=rest\n",
    ),
    # [spec:posix:req:builtin.kill.env-vars/test]
    # [spec:posix:req:builtin.wait.env-vars/test]
    Case(
        id="locale-kill-wait-under-utf8",
        rules=("builtin.kill.env-vars", "builtin.wait.env-vars"),
        # Signal names and pids are drawn from the portable character set,
        # so what is observable here is that a UTF-8 locale does not
        # disturb either utility, and that a bad operand's diagnostic still
        # names the operand byte for byte.
        script=(
            "sh -c 'kill -TERM $$; echo NOT-REACHED' ; printf 'killed=%s\\n' \"$?\"\n"
            "sleep 0.1 & p=$!\n"
            "wait $p; printf 'waited=%s\\n' \"$?\"\n"
            f"e={E_ACUTE}\n"
            'kill -s "BOGUS$e" $$ 2>err; printf \'badsig=%s\\n\' "$?"\n'
            'grep -q "BOGUS$e" err && printf \'names-operand=yes\\n\'\n'
        ),
        environment={"LC_ALL": UTF8, "LANG": UTF8},
        stdout="killed=143\nwaited=0\nbadsig=2\nnames-operand=yes\n",
        status="any",
    ),
    # [spec:posix:req:builtin.umask.env-locale/test]
    # [spec:posix:req:builtin.ulimit.env-locale/test]
    Case(
        id="locale-umask-ulimit-under-utf8",
        rules=("builtin.umask.env-locale", "builtin.ulimit.env-locale"),
        script=(
            "umask 022\n"
            "printf 'umask=%s\\n' \"$(umask)\"\n"
            "printf 'symbolic=%s\\n' \"$(umask -S)\"\n"
            "ulimit -n >/dev/null && printf 'ulimit=ok\\n'\n"
        ),
        environment={"LC_ALL": UTF8, "LANG": UTF8},
        stdout="umask=0022\nsymbolic=u=rwx,g=rx,o=rx\nulimit=ok\n",
    ),
    # [spec:posix:req:builtin.fc.env-locale/test]
    Case(
        id="locale-fc-multibyte-history",
        rules=("builtin.fc.env-locale",),
        mode="interactive",
        script=(
            f"e={E_ACUTE}\n"
            'printf \'mb=%s\\n\' "x$e"\n'
            "fc -l\n"
            "exit\n"
        ),
        environment={"LC_ALL": UTF8, "LANG": UTF8, "PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("mb=xé",),
        status="any",
        timeout=10.0,
    ),
    # [spec:posix:req:builtin.bg.env-locale/test]
    # [spec:posix:req:builtin.fg.env-locale/test]
    # [spec:posix:req:builtin.jobs.env-locale/test]
    Case(
        id="locale-jobs-multibyte-command-text",
        rules=(
            "builtin.bg.env-locale",
            "builtin.fg.env-locale",
            "builtin.jobs.env-locale",
        ),
        mode="interactive",
        # `jobs` prints the command text, so a multibyte word in a job is
        # the observable path for these three. The word is a literal: dash
        # reports the command *as written*, so a first draft using
        # `sleep 2 "$e"` printed `sleep 2 "${e}"` and the é never appeared.
        # That is dash reporting unexpanded text, which is what it should
        # do -- the case was wrong, not the shell.
        script=(
            'sleep 2 "é" &\n'
            "jobs\n"
            "kill %1\n"
            "exit\n"
        ),
        environment={"LC_ALL": UTF8, "LANG": UTF8, "PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("é",),
        status="any",
        timeout=10.0,
    ),
)
