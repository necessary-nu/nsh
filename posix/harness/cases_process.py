"""Executable cases for cd, umask, ulimit, kill, wait, and shell exit status.

Covers the rule corpus in posix/docs/spec/builtins-process.md,
builtins-signals.md, and exit-status.md. Expectations state what POSIX.1-2024
requires; where this shell disagrees the case is left failing on purpose.
"""

from __future__ import annotations

from model import Case


# 100 bytes; used to grow PWD past {PATH_MAX} for the cd step 9 case.
_LONG = "a" * 100


CASES: tuple[Case, ...] = (
    # ------------------------------------------------------------------
    # cd
    # ------------------------------------------------------------------
    # [spec:posix:syn:builtin.cd.syn/test]
    Case(
        id="proc-cd-syn",
        rules=("builtin.cd.syn",),
        script=(
            "mkdir a b || exit 1\n"
            "cd a || exit 1\n"
            "cd .. || exit 1\n"
            "cd -L b || exit 1\n"
            "cd .. || exit 1\n"
            "cd -P a || exit 1\n"
            "cd || exit 1\n"
            "printf 'forms-accepted\\n'\n"
        ),
        stdout="forms-accepted\n",
    ),
    # [spec:posix:req:builtin.cd.change-working-directory/test]
    # [spec:posix:req:builtin.cd.step10-chdir/test]
    # [spec:posix:req:builtin.cd.env-pwd/test]
    Case(
        id="proc-cd-change-working-directory",
        rules=(
            "builtin.cd.change-working-directory",
            "builtin.cd.step10-chdir",
            "builtin.cd.env-pwd",
        ),
        script=(
            "mkdir sub || exit 1\n"
            "cd sub || exit 1\n"
            "printf 'pwd=%s\\n' \"$PWD\"\n"
            ": > marker\n"
            "cd .. || exit 1\n"
            "if [ -f sub/marker ]; then printf 'cwd-changed\\n'; fi\n"
        ),
        stdout="pwd={ROOT}/sub\ncwd-changed\n",
    ),
    # [spec:posix:req:builtin.cd.step2-home-as-operand/test]
    # [spec:posix:def:builtin.cd.env-home/test]
    Case(
        id="proc-cd-home-default",
        rules=("builtin.cd.step2-home-as-operand", "builtin.cd.env-home"),
        script="cd || exit 1\nprintf '%s\\n' \"$PWD\"\n",
        stdout="{HOME}\n",
    ),
    # [spec:posix:sem:builtin.cd.step3-absolute-operand/test]
    Case(
        id="proc-cd-absolute-operand",
        rules=("builtin.cd.step3-absolute-operand",),
        script=(
            "root=$PWD\n"
            "mkdir -p base/sub sub || exit 1\n"
            "CDPATH=$root/base\n"
            "export CDPATH\n"
            "cd \"$root/sub\" || exit 1\n"
            "printf 'pwd=%s\\n' \"$PWD\"\n"
        ),
        # An absolute operand goes straight to step 7, so CDPATH is never
        # searched and nothing is written to standard output.
        stdout="pwd={ROOT}/sub\n",
    ),
    # [spec:posix:sem:builtin.cd.step4-dot-or-dot-dot/test]
    Case(
        id="proc-cd-dot-first-component",
        rules=("builtin.cd.step4-dot-or-dot-dot",),
        script=(
            "root=$PWD\n"
            "mkdir -p base/sub sub || exit 1\n"
            "CDPATH=$root/base\n"
            "export CDPATH\n"
            "cd ./sub || exit 1\n"
            "printf 'dot=%s\\n' \"$PWD\"\n"
            "cd \"$root/base\" || exit 1\n"
            "cd ../sub || exit 1\n"
            "printf 'dotdot=%s\\n' \"$PWD\"\n"
        ),
        stdout="dot={ROOT}/sub\ndotdot={ROOT}/sub\n",
    ),
    # [spec:posix:sem:builtin.cd.step5-cdpath-search/test]
    # [spec:posix:req:builtin.cd.env-cdpath/test]
    # [spec:posix:req:builtin.cd.stdout-new-directory/test]
    Case(
        id="proc-cd-cdpath-search",
        rules=(
            "builtin.cd.step5-cdpath-search",
            "builtin.cd.env-cdpath",
            "builtin.cd.stdout-new-directory",
        ),
        script=(
            "root=$PWD\n"
            "mkdir -p base/sub work/sub || exit 1\n"
            "cd \"$root/work\" || exit 1\n"
            "CDPATH=$root/base\n"
            "export CDPATH\n"
            "cd sub || exit 1\n"
            "printf 'hit=%s\\n' \"$PWD\"\n"
            "cd \"$root/work\" || exit 1\n"
            "CDPATH=:$root/base\n"
            "cd sub || exit 1\n"
            "printf 'null=%s\\n' \"$PWD\"\n"
        ),
        # The CDPATH hit writes the new directory; the null CDPATH entry
        # stands for the current directory and writes nothing.
        stdout="{ROOT}/base/sub\nhit={ROOT}/base/sub\nnull={ROOT}/work/sub\n",
    ),
    # [spec:posix:sem:builtin.cd.step6-operand-as-curpath/test]
    # [spec:posix:req:builtin.cd.stdout-no-output/test]
    Case(
        id="proc-cd-cdpath-miss",
        rules=("builtin.cd.step6-operand-as-curpath", "builtin.cd.stdout-no-output"),
        script=(
            "root=$PWD\n"
            "mkdir -p base other || exit 1\n"
            "CDPATH=$root/base\n"
            "export CDPATH\n"
            "cd other || exit 1\n"
            "printf 'pwd=%s\\n' \"$PWD\"\n"
        ),
        stdout="pwd={ROOT}/other\n",
    ),
    # [spec:posix:sem:builtin.cd.step7-prefix-pwd/test]
    # [spec:posix:req:builtin.cd.opt-l/test]
    Case(
        id="proc-cd-logical-pwd-prefix",
        rules=("builtin.cd.step7-prefix-pwd", "builtin.cd.opt-l"),
        script=(
            "mkdir real || exit 1\n"
            "ln -s real sym || exit 1\n"
            "cd -L sym || exit 1\n"
            "printf '%s\\n' \"$PWD\"\n"
        ),
        stdout="{ROOT}/sym\n",
    ),
    # [spec:posix:req:builtin.cd.step8-canonical-form-dot/test]
    Case(
        id="proc-cd-canonical-dot",
        rules=("builtin.cd.step8-canonical-form-dot",),
        script=(
            "mkdir -p a/b || exit 1\n"
            "cd ./a/./b || exit 1\n"
            "printf '%s\\n' \"$PWD\"\n"
        ),
        stdout="{ROOT}/a/b\n",
    ),
    # [spec:posix:req:builtin.cd.step8-canonical-form-dot-dot/test]
    # [spec:posix:req:builtin.cd.opt-p/test]
    Case(
        id="proc-cd-canonical-dot-dot",
        rules=("builtin.cd.step8-canonical-form-dot-dot", "builtin.cd.opt-p"),
        script=(
            "root=$PWD\n"
            "mkdir -p real/deep || exit 1\n"
            "ln -s real/deep sym || exit 1\n"
            "cd -L sym || exit 1\n"
            "cd .. || exit 1\n"
            "printf 'logical=%s\\n' \"$PWD\"\n"
            "cd \"$root\" || exit 1\n"
            "cd -P sym || exit 1\n"
            "cd .. || exit 1\n"
            "printf 'physical=%s\\n' \"$PWD\"\n"
        ),
        stdout="logical={ROOT}\nphysical={ROOT}/real\n",
    ),
    # [spec:posix:req:builtin.cd.step8-canonical-form-dot-dot/test]
    Case(
        id="proc-cd-dot-dot-after-non-directory",
        rules=("builtin.cd.step8-canonical-form-dot-dot",),
        # "If the preceding component does not refer ... to a directory, then
        # the cd utility shall display an appropriate error message and no
        # further steps shall be taken."
        script=(
            ": > regfile || exit 1\n"
            "err=$(cd regfile/.. 2>&1 >/dev/null)\n"
            "st=$?\n"
            "if [ \"$st\" -ne 0 ]; then printf 'rejected\\n';"
            " else printf 'accepted\\n'; fi\n"
            "if [ -n \"$err\" ]; then printf 'diagnostic\\n';"
            " else printf 'no-diagnostic\\n'; fi\n"
        ),
        stdout="rejected\ndiagnostic\n",
    ),
    # [spec:posix:req:builtin.cd.step9-path-max-relative/test]
    Case(
        id="proc-cd-path-max-relative",
        rules=("builtin.cd.step9-path-max-relative",),
        # PWD is always an initial substring of curpath here, so the standard
        # says the conversion to a relative pathname "shall always be
        # considered possible" and cd must succeed.
        script=(
            f"name={_LONG}\n"
            "i=0\n"
            "while [ ${#PWD} -lt 4300 ] && [ $i -lt 80 ]; do\n"
            "  mkdir \"$name\" || { printf 'mkdir-failed len=%s\\n' \"${#PWD}\"; exit 1; }\n"
            "  cd \"$name\" || { printf 'cd-failed len=%s\\n' \"${#PWD}\"; exit 1; }\n"
            "  i=$((i+1))\n"
            "done\n"
            "if [ ${#PWD} -gt 4096 ]; then printf 'deep-cd-ok\\n'; fi\n"
        ),
        stdout="deep-cd-ok\n",
        timeout=30.0,
    ),
    # [spec:posix:req:builtin.cd.step10-pwd-physical/test]
    Case(
        id="proc-cd-pwd-physical",
        rules=("builtin.cd.step10-pwd-physical",),
        script=(
            "mkdir real || exit 1\n"
            "ln -s real sym || exit 1\n"
            "cd -P sym || exit 1\n"
            "printf 'pwd=%s\\n' \"$PWD\"\n"
            "if [ \"$PWD\" = \"$(pwd -P)\" ]; then printf 'matches-pwd-P\\n'; fi\n"
        ),
        stdout="pwd={ROOT}/real\nmatches-pwd-P\n",
    ),
    # [spec:posix:req:builtin.cd.oldpwd-set/test]
    Case(
        id="proc-cd-oldpwd-set",
        rules=("builtin.cd.oldpwd-set",),
        script=(
            "mkdir a || exit 1\n"
            "cd a || exit 1\n"
            "printf 'oldpwd=%s\\n' \"$OLDPWD\"\n"
            "printf 'pwd=%s\\n' \"$PWD\"\n"
        ),
        stdout="oldpwd={ROOT}\npwd={ROOT}/a\n",
    ),
    # [spec:posix:req:builtin.cd.operand-hyphen/test]
    # [spec:posix:req:builtin.cd.env-oldpwd/test]
    Case(
        id="proc-cd-hyphen-operand",
        rules=("builtin.cd.operand-hyphen", "builtin.cd.env-oldpwd"),
        script=(
            "mkdir a || exit 1\n"
            "cd a || exit 1\n"
            "cd - || exit 1\n"
            "printf 'pwd=%s\\n' \"$PWD\"\n"
        ),
        # cd '-' writes the new working directory to standard output.
        stdout="{ROOT}\npwd={ROOT}\n",
    ),
    # [spec:posix:req:builtin.cd.opt-l-p-last-wins/test]
    Case(
        id="proc-cd-l-p-last-wins",
        rules=("builtin.cd.opt-l-p-last-wins",),
        script=(
            "root=$PWD\n"
            "mkdir -p real/deep || exit 1\n"
            "ln -s real/deep sym || exit 1\n"
            "cd -P -L sym || exit 1\n"
            "printf 'last-L=%s\\n' \"$PWD\"\n"
            "cd \"$root\" || exit 1\n"
            "cd -L -P sym || exit 1\n"
            "printf 'last-P=%s\\n' \"$PWD\"\n"
            "cd \"$root\" || exit 1\n"
            "cd sym || exit 1\n"
            "printf 'neither=%s\\n' \"$PWD\"\n"
        ),
        stdout=(
            "last-L={ROOT}/sym\n"
            "last-P={ROOT}/real/deep\n"
            "neither={ROOT}/sym\n"
        ),
    ),
    # [spec:posix:req:builtin.cd.utility-syntax-guidelines/test]
    Case(
        id="proc-cd-utility-syntax-guidelines",
        rules=("builtin.cd.utility-syntax-guidelines",),
        script=(
            "mkdir a || exit 1\n"
            "cd -- a || exit 1\n"
            "printf 'delimiter=%s\\n' \"$PWD\"\n"
            "cd .. || exit 1\n"
            "cd -L -- a || exit 1\n"
            "printf 'option-then-delimiter=%s\\n' \"$PWD\"\n"
        ),
        stdout="delimiter={ROOT}/a\noption-then-delimiter={ROOT}/a\n",
    ),
    # [spec:posix:req:builtin.cd.opt-e/test]
    Case(
        id="proc-cd-opt-e",
        rules=("builtin.cd.opt-e",),
        # -e is listed under "The following options shall be supported by the
        # implementation"; with -P and a determinable PWD it must succeed.
        script=(
            "mkdir a || exit 1\n"
            "cd -P -e a 2>/dev/null\n"
            "printf 'status=%s\\n' \"$?\"\n"
            "printf 'pwd=%s\\n' \"$PWD\"\n"
        ),
        stdout="status=0\npwd={ROOT}/a\n",
    ),
    # [spec:posix:req:builtin.cd.operand-empty-string/test]
    Case(
        id="proc-cd-empty-operand",
        rules=("builtin.cd.operand-empty-string",),
        script=(
            "err=$(cd \"\" 2>&1 >/dev/null)\n"
            "st=$?\n"
            "if [ \"$st\" -ne 0 ]; then printf 'nonzero\\n';"
            " else printf 'zero\\n'; fi\n"
            "if [ -n \"$err\" ]; then printf 'diagnostic\\n';"
            " else printf 'no-diagnostic\\n'; fi\n"
        ),
        stdout="nonzero\ndiagnostic\n",
    ),
    # [spec:posix:req:builtin.cd.stderr/test]
    # [spec:posix:req:builtin.cd.interfaces/test]
    Case(
        id="proc-cd-stderr-and-interfaces",
        rules=("builtin.cd.stderr", "builtin.cd.interfaces"),
        script=(
            "mkdir a || exit 1\n"
            "cd a\n"
            "read v\n"
            "printf 'read=%s\\n' \"$v\"\n"
        ),
        stdin="LINE\n",
        stdout="read=LINE\n",
        stderr="",
    ),
    # [spec:posix:req:builtin.cd.exit-status/test]
    # [spec:posix:req:builtin.cd.consequences-of-errors/test]
    Case(
        id="proc-cd-exit-status",
        rules=("builtin.cd.exit-status", "builtin.cd.consequences-of-errors"),
        script=(
            "root=$PWD\n"
            "mkdir a || exit 1\n"
            "cd a\n"
            "printf 'success=%s\\n' \"$?\"\n"
            "cd \"$root/nosuchdir\" 2>/dev/null\n"
            "s=$?\n"
            "if [ \"$s\" -gt 0 ]; then printf 'failure-nonzero\\n';"
            " else printf 'failure-zero\\n'; fi\n"
            "printf 'pwd=%s\\n' \"$PWD\"\n"
        ),
        stdout="success=0\nfailure-nonzero\npwd={ROOT}/a\n",
    ),
    # ------------------------------------------------------------------
    # umask
    # ------------------------------------------------------------------
    # [spec:posix:syn:builtin.umask.syn/test]
    Case(
        id="proc-umask-syn",
        rules=("builtin.umask.syn",),
        script=(
            "umask 022 || exit 1\n"
            "umask >/dev/null || exit 1\n"
            "umask -S >/dev/null || exit 1\n"
            "printf 'forms-accepted\\n'\n"
        ),
        stdout="forms-accepted\n",
    ),
    # [spec:posix:req:builtin.umask.set-mask/test]
    # [spec:posix:req:builtin.umask.octal-form/test]
    # [spec:posix:def:builtin.umask.operand-mask/test]
    Case(
        id="proc-umask-set-mask",
        rules=(
            "builtin.umask.set-mask",
            "builtin.umask.octal-form",
            "builtin.umask.operand-mask",
        ),
        script=(
            "umask 077\n"
            ": > f1\n"
            "umask 000\n"
            ": > f2\n"
            "umask 026\n"
            ": > f3\n"
            "ls -l f1 | cut -c1-10\n"
            "ls -l f2 | cut -c1-10\n"
            "ls -l f3 | cut -c1-10\n"
        ),
        stdout="-rw-------\n-rw-rw-rw-\n-rw-r-----\n",
    ),
    # [spec:posix:req:builtin.umask.subshell-no-effect/test]
    Case(
        id="proc-umask-subshell-no-effect",
        rules=("builtin.umask.subshell-no-effect",),
        script=(
            "umask 022\n"
            "(umask 002)\n"
            ": > f\n"
            "ls -l f | cut -c1-10\n"
        ),
        stdout="-rw-r--r--\n",
    ),
    # [spec:posix:req:builtin.umask.report-when-no-operand/test]
    # [spec:posix:req:builtin.umask.stdout-no-operand/test]
    # [spec:posix:req:builtin.umask.default-output-style/test]
    # [spec:posix:req:builtin.umask.prior-default-output-as-operand/test]
    Case(
        id="proc-umask-report-round-trip",
        rules=(
            "builtin.umask.report-when-no-operand",
            "builtin.umask.stdout-no-operand",
            "builtin.umask.default-output-style",
            "builtin.umask.prior-default-output-as-operand",
        ),
        script=(
            "umask 037\n"
            "saved=$(umask)\n"
            "if [ -n \"$saved\" ]; then printf 'reported\\n';"
            " else printf 'no-report\\n'; fi\n"
            "umask 000\n"
            "umask \"$saved\" || exit 1\n"
            ": > f\n"
            "ls -l f | cut -c1-10\n"
        ),
        stdout="reported\n-rw-r-----\n",
    ),
    # [spec:posix:req:builtin.umask.opt-s/test]
    # [spec:posix:req:builtin.umask.stdout-symbolic-format/test]
    Case(
        id="proc-umask-symbolic-output",
        rules=("builtin.umask.opt-s", "builtin.umask.stdout-symbolic-format"),
        script=(
            "umask 027\n"
            "umask -S\n"
            "umask 000\n"
            "umask -S\n"
            "umask 777\n"
            "umask -S\n"
        ),
        stdout="u=rwx,g=rx,o=\nu=rwx,g=rwx,o=rwx\nu=,g=,o=\n",
    ),
    # [spec:posix:req:builtin.umask.symbolic-mode-complement/test]
    Case(
        id="proc-umask-symbolic-complement",
        rules=("builtin.umask.symbolic-mode-complement", "builtin.umask.operand-mask"),
        script=(
            "umask 000\n"
            "umask u=rwx,g=rx,o=rx\n"
            "umask -S\n"
            ": > f\n"
            "ls -l f | cut -c1-10\n"
        ),
        stdout="u=rwx,g=rx,o=rx\n-rw-r--r--\n",
    ),
    # [spec:posix:req:builtin.umask.symbolic-op-characters/test]
    Case(
        id="proc-umask-symbolic-ops",
        rules=("builtin.umask.symbolic-op-characters",),
        script=(
            "umask 777\n"
            "umask a+rx\n"
            "umask -S\n"
            "umask 000\n"
            "umask a-w\n"
            "umask -S\n"
        ),
        # '+' clears the named bits in the mask, '-' sets them.
        stdout="u=rx,g=rx,o=rx\nu=rx,g=rx,o=rx\n",
    ),
    # [spec:posix:req:builtin.umask.stdout-operand-no-output/test]
    Case(
        id="proc-umask-operand-no-output",
        rules=("builtin.umask.stdout-operand-no-output",),
        script=(
            "out=$(umask 022)\n"
            "if [ -z \"$out\" ]; then printf 'silent\\n';"
            " else printf 'output=%s\\n' \"$out\"; fi\n"
            "out=$(umask u=rwx,g=,o=)\n"
            "if [ -z \"$out\" ]; then printf 'silent-symbolic\\n';"
            " else printf 'output=%s\\n' \"$out\"; fi\n"
        ),
        stdout="silent\nsilent-symbolic\n",
    ),
    # [spec:posix:req:builtin.umask.utility-syntax-guidelines/test]
    Case(
        id="proc-umask-utility-syntax-guidelines",
        rules=("builtin.umask.utility-syntax-guidelines",),
        script=(
            "umask 000\n"
            "umask -- 022 || exit 1\n"
            ": > f\n"
            "ls -l f | cut -c1-10\n"
        ),
        stdout="-rw-r--r--\n",
    ),
    # [spec:posix:req:builtin.umask.stderr/test]
    # [spec:posix:req:builtin.umask.interfaces/test]
    Case(
        id="proc-umask-stderr-and-interfaces",
        rules=("builtin.umask.stderr", "builtin.umask.interfaces"),
        script=(
            "umask 022\n"
            "umask >/dev/null\n"
            "umask -S >/dev/null\n"
            "read v\n"
            "printf 'read=%s\\n' \"$v\"\n"
        ),
        stdin="LINE\n",
        stdout="read=LINE\n",
        stderr="",
    ),
    # [spec:posix:req:builtin.umask.exit-status/test]
    Case(
        id="proc-umask-exit-status",
        rules=("builtin.umask.exit-status",),
        script=(
            "umask 022\n"
            "printf 'set=%s\\n' \"$?\"\n"
            "umask >/dev/null\n"
            "printf 'report=%s\\n' \"$?\"\n"
            "(umask 999) 2>/dev/null\n"
            "s=$?\n"
            "if [ \"$s\" -gt 0 ]; then printf 'error-nonzero\\n';"
            " else printf 'error-zero\\n'; fi\n"
        ),
        stdout="set=0\nreport=0\nerror-nonzero\n",
    ),
    # ------------------------------------------------------------------
    # ulimit
    # ------------------------------------------------------------------
    # [spec:posix:syn:builtin.ulimit.syn/test]
    Case(
        id="proc-ulimit-syn",
        rules=("builtin.ulimit.syn",),
        script=(
            "ulimit -H -a >/dev/null || exit 1\n"
            "ulimit -S -a >/dev/null || exit 1\n"
            "ulimit -H -f >/dev/null || exit 1\n"
            "( ulimit -f 1000 && ulimit -f )\n"
        ),
        stdout="1000\n",
        requires=("XSI",),
    ),
    # [spec:posix:req:builtin.ulimit.report-or-set/test]
    # [spec:posix:def:builtin.ulimit.operand-newlimit/test]
    # [spec:posix:req:builtin.ulimit.stdout-single-limit-format/test]
    Case(
        id="proc-ulimit-report-or-set",
        rules=(
            "builtin.ulimit.report-or-set",
            "builtin.ulimit.operand-newlimit",
            "builtin.ulimit.stdout-single-limit-format",
        ),
        script=(
            "( ulimit -f 1000; ulimit -f )\n"
            "( ulimit -f 2000; ulimit -f )\n"
            "( ulimit -f unlimited; ulimit -f )\n"
        ),
        stdout="1000\n2000\nunlimited\n",
    ),
    # [spec:posix:sem:builtin.ulimit.soft-and-hard-limits/test]
    # [spec:posix:req:builtin.ulimit.opt-hard/test]
    # [spec:posix:req:builtin.ulimit.opt-soft/test]
    Case(
        id="proc-ulimit-soft-and-hard",
        rules=(
            "builtin.ulimit.soft-and-hard-limits",
            "builtin.ulimit.opt-hard",
            "builtin.ulimit.opt-soft",
        ),
        script=(
            "(\n"
            "ulimit -f 1000 || exit 1\n"
            "ulimit -S -f 500 || exit 1\n"
            "printf 'soft=%s\\n' \"$(ulimit -S -f)\"\n"
            "printf 'hard=%s\\n' \"$(ulimit -H -f)\"\n"
            "if ulimit -S -f 2000 2>/dev/null; then printf 'soft-above-hard\\n';"
            " else printf 'soft-capped-by-hard\\n'; fi\n"
            "if ulimit -H -f 2000 2>/dev/null; then printf 'hard-raised\\n';"
            " else printf 'hard-lowering-irreversible\\n'; fi\n"
            ")\n"
        ),
        stdout=(
            "soft=500\nhard=1000\nsoft-capped-by-hard\nhard-lowering-irreversible\n"
        ),
    ),
    # [spec:posix:req:builtin.ulimit.unlimited-value/test]
    Case(
        id="proc-ulimit-unlimited",
        rules=("builtin.ulimit.unlimited-value",),
        script=(
            "( ulimit -f unlimited || exit 1\n"
            "  printf 'reported=%s\\n' \"$(ulimit -f)\"\n"
            "  dd if=/dev/zero of=big bs=1024 count=8 2>/dev/null\n"
            "  printf 'size=%s\\n' \"$(wc -c < big)\" )\n"
            "( ulimit -f 1000\n"
            "  if ulimit -S -f unlimited 2>/dev/null;"
            " then printf 'unlimited-under-hard\\n';"
            " else printf 'unlimited-exceeds-hard\\n'; fi )\n"
        ),
        stdout="reported=unlimited\nsize=8192\nunlimited-exceeds-hard\n",
    ),
    # [spec:posix:req:builtin.ulimit.limits-exceeded/test]
    # [spec:posix:req:builtin.ulimit.opt-fsize/test]
    Case(
        id="proc-ulimit-fsize-limit",
        rules=("builtin.ulimit.limits-exceeded", "builtin.ulimit.opt-fsize"),
        script=(
            "ulimit -c 0\n"
            "( ulimit -f 1; exec dd if=/dev/zero of=big bs=1024 count=4 2>/dev/null )\n"
            "s=$?\n"
            "if [ \"$s\" -gt 128 ]; then printf 'sigxfsz\\n';"
            " else printf 'status=%s\\n' \"$s\"; fi\n"
            "printf 'size=%s\\n' \"$(wc -c < big)\"\n"
        ),
        # RLIMIT_FSIZE is expressed in 512-byte units, and setrlimit() requires
        # SIGXFSZ when a write would exceed the soft limit.
        stdout="sigxfsz\nsize=512\n",
    ),
    # [spec:posix:req:builtin.ulimit.utility-syntax-guidelines/test]
    Case(
        id="proc-ulimit-utility-syntax-guidelines",
        rules=("builtin.ulimit.utility-syntax-guidelines",),
        script=(
            "( ulimit -f -- 1000 || exit 1; printf 'delimiter=%s\\n' \"$(ulimit -f)\" )\n"
            "( ulimit -S -f 1000 || exit 1; printf 'separate=%s\\n' \"$(ulimit -S -f)\" )\n"
        ),
        stdout="delimiter=1000\nseparate=1000\n",
    ),
    # [spec:posix:req:builtin.ulimit.opt-all/test]
    Case(
        id="proc-ulimit-opt-all",
        rules=("builtin.ulimit.opt-all",),
        script=(
            "ulimit -a > all.txt || exit 1\n"
            "n=$(wc -l < all.txt)\n"
            "if [ \"$n\" -ge 7 ]; then printf 'reports-all\\n';"
            " else printf 'lines=%s\\n' \"$n\"; fi\n"
        ),
        stdout="reports-all\n",
    ),
    # [spec:posix:req:builtin.ulimit.stdout-all-format/test]
    Case(
        id="proc-ulimit-all-format",
        rules=("builtin.ulimit.stdout-all-format",),
        # Each -a line must include "The ulimit option used to specify the
        # resource".
        script=(
            "ulimit -a > all.txt || exit 1\n"
            "for o in c d f n s t v; do\n"
            "  grep -q -e \"-$o\" all.txt || printf 'missing-option-%s\\n' \"$o\"\n"
            "done\n"
            "printf 'checked\\n'\n"
        ),
        stdout="checked\n",
    ),
    # [spec:posix:req:builtin.ulimit.opt-core/test]
    # [spec:posix:req:builtin.ulimit.opt-data/test]
    # [spec:posix:req:builtin.ulimit.opt-nofile/test]
    # [spec:posix:req:builtin.ulimit.opt-stack/test]
    # [spec:posix:req:builtin.ulimit.opt-as/test]
    Case(
        id="proc-ulimit-resource-options",
        rules=(
            "builtin.ulimit.opt-core",
            "builtin.ulimit.opt-data",
            "builtin.ulimit.opt-nofile",
            "builtin.ulimit.opt-stack",
            "builtin.ulimit.opt-as",
        ),
        script=(
            "( ulimit -c 100 || exit 1; printf 'c=%s\\n' \"$(ulimit -c)\" )\n"
            "( ulimit -d 100000 || exit 1; printf 'd=%s\\n' \"$(ulimit -d)\" )\n"
            "( ulimit -n 200 || exit 1; printf 'n=%s\\n' \"$(ulimit -n)\" )\n"
            "( ulimit -s 8000 || exit 1; printf 's=%s\\n' \"$(ulimit -s)\" )\n"
            "( ulimit -v 4194304 || exit 1; printf 'v=%s\\n' \"$(ulimit -v)\" )\n"
        ),
        stdout="c=100\nd=100000\nn=200\ns=8000\nv=4194304\n",
    ),
    # [spec:posix:req:builtin.ulimit.opt-cpu/test]
    Case(
        id="proc-ulimit-opt-cpu",
        rules=("builtin.ulimit.opt-cpu",),
        script="( ulimit -t 3600 || exit 1; printf 't=%s\\n' \"$(ulimit -t)\" )\n",
        stdout="t=3600\n",
        requires=("XSI",),
    ),
    # [spec:posix:req:builtin.ulimit.default-hard-and-soft/test]
    Case(
        id="proc-ulimit-default-hard-and-soft",
        rules=("builtin.ulimit.default-hard-and-soft",),
        script=(
            "( ulimit -f 1000 || exit 1\n"
            "  printf 'both=%s/%s\\n' \"$(ulimit -S -f)\" \"$(ulimit -H -f)\"\n"
            "  ulimit -S -f 500 || exit 1\n"
            "  printf 'default-report=%s\\n' \"$(ulimit -f)\" )\n"
        ),
        # No -H/-S with a newlimit sets both; with no newlimit -S is default.
        stdout="both=1000/1000\ndefault-report=500\n",
    ),
    # [spec:posix:req:builtin.ulimit.default-f-option/test]
    Case(
        id="proc-ulimit-default-f-option",
        rules=("builtin.ulimit.default-f-option",),
        script=(
            "( ulimit 1000 || exit 1\n"
            "  printf 'f=%s\\n' \"$(ulimit -f)\"\n"
            "  printf 'plain=%s\\n' \"$(ulimit)\" )\n"
        ),
        stdout="f=1000\nplain=1000\n",
    ),
    # [spec:posix:req:builtin.ulimit.stdout-used-when-reporting/test]
    Case(
        id="proc-ulimit-stdout-when-reporting",
        rules=("builtin.ulimit.stdout-used-when-reporting",),
        script=(
            "out=$( (ulimit -f 1000) )\n"
            "if [ -z \"$out\" ]; then printf 'set-silent\\n';"
            " else printf 'set-output=%s\\n' \"$out\"; fi\n"
            "out=$(ulimit -f)\n"
            "if [ -n \"$out\" ]; then printf 'report-on-stdout\\n';"
            " else printf 'report-empty\\n'; fi\n"
        ),
        stdout="set-silent\nreport-on-stdout\n",
    ),
    # [spec:posix:req:builtin.ulimit.stderr/test]
    # [spec:posix:req:builtin.ulimit.interfaces/test]
    Case(
        id="proc-ulimit-stderr-and-interfaces",
        rules=("builtin.ulimit.stderr", "builtin.ulimit.interfaces"),
        script=(
            "ulimit -f >/dev/null\n"
            "ulimit -a >/dev/null\n"
            "read v\n"
            "printf 'read=%s\\n' \"$v\"\n"
        ),
        stdin="LINE\n",
        stdout="read=LINE\n",
        stderr="",
    ),
    # [spec:posix:req:builtin.ulimit.exit-status/test]
    Case(
        id="proc-ulimit-exit-status",
        rules=("builtin.ulimit.exit-status",),
        script=(
            "(\n"
            "ulimit -f 1000\n"
            "printf 'set=%s\\n' \"$?\"\n"
            "ulimit -f >/dev/null\n"
            "printf 'report=%s\\n' \"$?\"\n"
            "ulimit -f 2000 2>/dev/null\n"
            "s=$?\n"
            "if [ \"$s\" -gt 0 ]; then printf 'rejected-nonzero\\n';"
            " else printf 'rejected-zero\\n'; fi\n"
            ")\n"
        ),
        stdout="set=0\nreport=0\nrejected-nonzero\n",
    ),
    # ------------------------------------------------------------------
    # kill
    # ------------------------------------------------------------------
    # [spec:posix:syn:builtin.kill.synopsis/test]
    Case(
        id="sig-kill-synopsis",
        rules=("builtin.kill.synopsis",),
        script=(
            "trap 'printf \"trapped\\n\"' TERM\n"
            "kill -s TERM $$ || exit 1\n"
            "kill -l >/dev/null || exit 1\n"
            "printf 'forms-accepted\\n'\n"
        ),
        stdout="trapped\nforms-accepted\n",
    ),
    # [spec:posix:syn:builtin.kill.synopsis-xsi/test]
    # [spec:posix:req:builtin.kill.option-signal-name/test]
    # [spec:posix:req:builtin.kill.option-signal-number/test]
    # [spec:posix:req:builtin.kill.negative-first-argument/test]
    Case(
        id="sig-kill-xsi-signal-forms",
        rules=(
            "builtin.kill.synopsis-xsi",
            "builtin.kill.option-signal-name",
            "builtin.kill.option-signal-number",
            "builtin.kill.negative-first-argument",
        ),
        # "-14" is a negative integer first argument: it must select SIGALRM,
        # not a process group operand.
        script=(
            "trap 'printf \"by-name\\n\"' HUP\n"
            "kill -HUP $$ || exit 1\n"
            "trap 'printf \"by-number\\n\"' ALRM\n"
            "kill -14 $$ || exit 1\n"
            "printf 'done\\n'\n"
        ),
        stdout="by-name\nby-number\ndone\n",
        requires=("XSI",),
    ),
    # [spec:posix:req:builtin.kill.send-signal/test]
    # [spec:posix:req:builtin.kill.operand-pid-number/test]
    Case(
        id="sig-kill-send-signal",
        rules=("builtin.kill.send-signal", "builtin.kill.operand-pid-number"),
        # The child is signalled before any TERM trap exists in this shell, so
        # the observation is about kill and not about trap inheritance.
        script=(
            "sleep 2 &\n"
            "p=$!\n"
            "kill -s TERM \"$p\" || exit 1\n"
            "wait \"$p\"\n"
            "s=$?\n"
            "if [ \"$s\" -gt 128 ]; then printf 'child-signaled\\n';"
            " else printf 'child-status=%s\\n' \"$s\"; fi\n"
            "trap 'printf \"default-term\\n\"' TERM\n"
            "kill $$ || exit 1\n"
            "trap 'printf \"process-group\\n\"' ALRM\n"
            "kill -s ALRM 0 || exit 1\n"
        ),
        stdout="child-signaled\ndefault-term\nprocess-group\n",
        timeout=15.0,
    ),
    # [spec:posix:req:builtin.kill.option-s/test]
    Case(
        id="sig-kill-option-s",
        rules=("builtin.kill.option-s",),
        script=(
            "trap 'printf \"upper\\n\"' ALRM\n"
            "kill -s ALRM $$ || exit 1\n"
            "trap 'printf \"lower\\n\"' HUP\n"
            "kill -s hup $$ || exit 1\n"
            "kill -s 0 $$ || exit 1\n"
            "printf 'signal-zero\\n'\n"
        ),
        stdout="upper\nlower\nsignal-zero\n",
    ),
    # [spec:posix:req:builtin.kill.option-l/test]
    # [spec:posix:req:builtin.kill.stdout-signal-list-format/test]
    Case(
        id="sig-kill-option-l-list",
        rules=("builtin.kill.option-l", "builtin.kill.stdout-signal-list-format"),
        script=(
            "kill -l > list.txt || exit 1\n"
            "for n in HUP INT QUIT KILL ALRM TERM; do\n"
            "  grep -q -x \"$n\" list.txt || printf 'missing-%s\\n' \"$n\"\n"
            "done\n"
            "if grep -q '^SIG' list.txt; then printf 'has-sig-prefix\\n'; fi\n"
            "if grep -q '[a-z]' list.txt; then printf 'has-lowercase\\n'; fi\n"
            "body=$(cat list.txt)\n"
            "n1=$(wc -c < list.txt)\n"
            "n2=$(printf '%s' \"$body\" | wc -c)\n"
            "if [ \"$n1\" -eq \"$((n2 + 1))\" ]; then printf 'newline-terminated\\n';"
            " else printf 'trailer=%s/%s\\n' \"$n1\" \"$n2\"; fi\n"
        ),
        stdout="newline-terminated\n",
    ),
    # [spec:posix:req:builtin.kill.stdout-exit-status-format/test]
    # [spec:posix:def:builtin.kill.operand-exit-status/test]
    Case(
        id="sig-kill-option-l-exit-status",
        rules=(
            "builtin.kill.stdout-exit-status-format",
            "builtin.kill.operand-exit-status",
        ),
        script=(
            "kill -l 9\n"
            "kill -l 15\n"
            "sh -c 'kill -9 $$'\n"
            "kill -l $?\n"
        ),
        stdout="KILL\nTERM\nKILL\n",
    ),
    # [spec:posix:def:builtin.kill.operand-pid-job-id/test]
    Case(
        id="sig-kill-job-id-operand",
        rules=("builtin.kill.operand-pid-job-id",),
        # Without job control the background job is a plain process ID, so the
        # job ID must resolve to that process ID.
        script=(
            "sleep 2 &\n"
            "p=$!\n"
            "if kill -s TERM %1 2>/dev/null; then printf 'signaled\\n';"
            " else printf 'kill-failed\\n'; fi\n"
            "wait \"$p\"\n"
            "s=$?\n"
            "if [ \"$s\" -gt 128 ]; then printf 'terminated-by-signal\\n';"
            " else printf 'status=%s\\n' \"$s\"; fi\n"
        ),
        stdout="signaled\nterminated-by-signal\n",
        timeout=15.0,
    ),
    Case(
        id="sig-kill-job-id-process-group",
        rules=("builtin.kill.operand-pid-job-id",),
        mode="interactive",
        # With job control, the job ID identifies the whole process group.
        # A positive PID would signal only the first sleep and leave the
        # pipeline's last process (and therefore wait) running to timeout.
        script=(
            "set -m\n"
            "sleep 10 | sleep 10 &\n"
            "p=$!\n"
            "if kill -s TERM %1 2>/dev/null; then printf 'group-signaled\\n';"
            " else printf 'kill-failed\\n'; fi\n"
            "wait \"$p\"\n"
            "s=$?\n"
            "if [ \"$s\" -gt 128 ]; then printf 'group-terminated\\n';"
            " else printf 'status=%s\\n' \"$s\"; fi\n"
            "exit\n"
        ),
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("group-signaled\n", "group-terminated\n"),
        status="any",
        timeout=15.0,
    ),
    # [spec:posix:req:builtin.kill.stdout-unused-without-l/test]
    Case(
        id="sig-kill-stdout-unused",
        rules=("builtin.kill.stdout-unused-without-l",),
        script=(
            "out=$(kill -s 0 $$)\n"
            "if [ -z \"$out\" ]; then printf 'silent-success\\n';"
            " else printf 'output=%s\\n' \"$out\"; fi\n"
            "sh -c 'exit 0' &\n"
            "p=$!\n"
            "wait \"$p\"\n"
            "out=$(kill -s TERM \"$p\" 2>/dev/null)\n"
            "if [ -z \"$out\" ]; then printf 'silent-error\\n';"
            " else printf 'output=%s\\n' \"$out\"; fi\n"
        ),
        stdout="silent-success\nsilent-error\n",
    ),
    # [spec:posix:req:builtin.kill.stderr/test]
    # [spec:posix:req:builtin.kill.interfaces/test]
    Case(
        id="sig-kill-stderr-and-interfaces",
        rules=("builtin.kill.stderr", "builtin.kill.interfaces"),
        script=(
            "kill -s 0 $$\n"
            "kill -l >/dev/null\n"
            "read v\n"
            "printf 'read=%s\\n' \"$v\"\n"
        ),
        stdin="LINE\n",
        stdout="read=LINE\n",
        stderr="",
    ),
    # [spec:posix:req:builtin.kill.exit-status/test]
    # [spec:posix:req:builtin.kill.utility-syntax-guidelines/test]
    Case(
        id="sig-kill-exit-status",
        rules=("builtin.kill.exit-status", "builtin.kill.utility-syntax-guidelines"),
        script=(
            "kill -l >/dev/null\n"
            "printf 'list=%s\\n' \"$?\"\n"
            "kill -s 0 -- $$\n"
            "printf 'delimiter=%s\\n' \"$?\"\n"
            "sh -c 'exit 0' &\n"
            "p=$!\n"
            "wait \"$p\"\n"
            "kill -s TERM \"$p\" 2>/dev/null\n"
            "s=$?\n"
            "if [ \"$s\" -gt 0 ]; then printf 'error-nonzero\\n';"
            " else printf 'error-zero\\n'; fi\n"
        ),
        stdout="list=0\ndelimiter=0\nerror-nonzero\n",
    ),
    # ------------------------------------------------------------------
    # wait
    # ------------------------------------------------------------------
    # [spec:posix:syn:builtin.wait.synopsis/test]
    Case(
        id="sig-wait-synopsis",
        rules=("builtin.wait.synopsis",),
        script=(
            "sh -c 'exit 0' &\n"
            "wait || exit 1\n"
            "sh -c 'exit 0' &\n"
            "p=$!\n"
            "wait \"$p\" || exit 1\n"
            "printf 'forms-accepted\\n'\n"
        ),
        stdout="forms-accepted\n",
    ),
    # [spec:posix:req:builtin.wait.await-children/test]
    # [spec:posix:req:builtin.wait.no-operands/test]
    Case(
        id="sig-wait-no-operands",
        rules=("builtin.wait.await-children", "builtin.wait.no-operands"),
        script=(
            "( sleep 0.3; : > slow ) &\n"
            "( sleep 0.1; : > quick ) &\n"
            "wait\n"
            "printf 'status=%s\\n' \"$?\"\n"
            "if [ -f slow ] && [ -f quick ]; then printf 'all-terminated\\n'; fi\n"
        ),
        stdout="status=0\nall-terminated\n",
        timeout=15.0,
    ),
    # [spec:posix:req:builtin.wait.pid-operands/test]
    # [spec:posix:req:builtin.wait.exit-status-last-operand/test]
    # [spec:posix:def:builtin.wait.operand-pid-number/test]
    Case(
        id="sig-wait-pid-operands",
        rules=(
            "builtin.wait.pid-operands",
            "builtin.wait.exit-status-last-operand",
            "builtin.wait.operand-pid-number",
        ),
        script=(
            "sh -c 'exit 3' &\n"
            "a=$!\n"
            "sh -c 'exit 5' &\n"
            "b=$!\n"
            "wait \"$a\" \"$b\"\n"
            "printf 'last=%s\\n' \"$?\"\n"
            "sh -c 'exit 6' &\n"
            "c=$!\n"
            "wait 999999 \"$c\" 2>/dev/null\n"
            "printf 'unknown-first=%s\\n' \"$?\"\n"
        ),
        stdout="last=5\nunknown-first=6\n",
        timeout=15.0,
    ),
    # [spec:posix:req:builtin.wait.pid-operands/test]
    # [spec:posix:req:builtin.wait.exit-status-last-operand/test]
    # [spec:posix:req:builtin.wait.exit-status-values/test]
    Case(
        id="sig-wait-unknown-last-operand",
        rules=(
            "builtin.wait.pid-operands",
            "builtin.wait.exit-status-last-operand",
            "builtin.wait.exit-status-values",
        ),
        # An unknown pid operand is treated as a known pid that exited 127, and
        # the status of wait is the status of the last operand specified.
        script=(
            "sh -c 'exit 6' &\n"
            "c=$!\n"
            "wait \"$c\" 999999 2>/dev/null\n"
            "printf 'unknown-last=%s\\n' \"$?\"\n"
        ),
        stdout="unknown-last=127\n",
        timeout=15.0,
    ),
    # [spec:posix:req:builtin.wait.remove-waited-for-pid/test]
    Case(
        id="sig-wait-removes-waited-pid",
        rules=("builtin.wait.remove-waited-for-pid",),
        script=(
            "sh -c 'exit 7' &\n"
            "p=$!\n"
            "wait \"$p\"\n"
            "printf 'first=%s\\n' \"$?\"\n"
            "wait \"$p\" 2>/dev/null\n"
            "printf 'second=%s\\n' \"$?\"\n"
        ),
        # The pid is removed once waited for, so the second wait sees an
        # unknown process ID and must report 127.
        stdout="first=7\nsecond=127\n",
    ),
    # [spec:posix:req:builtin.wait.exit-status-values/test]
    Case(
        id="sig-wait-exit-status-values",
        rules=("builtin.wait.exit-status-values",),
        script=(
            "wait 999999 2>/dev/null\n"
            "printf 'unknown=%s\\n' \"$?\"\n"
            "sh -c 'exit 0' &\n"
            "wait\n"
            "printf 'none=%s\\n' \"$?\"\n"
            "sh -c 'exit 9' &\n"
            "p=$!\n"
            "wait \"$p\"\n"
            "printf 'known=%s\\n' \"$?\"\n"
        ),
        stdout="unknown=127\nnone=0\nknown=9\n",
    ),
    # [spec:posix:req:builtin.wait.exit-status-signal/test]
    Case(
        id="sig-wait-exit-status-signal",
        rules=("builtin.wait.exit-status-signal",),
        script=(
            "sh -c 'kill -9 $$' &\n"
            "a=$!\n"
            "wait \"$a\"\n"
            "x=$?\n"
            "sh -c 'kill -15 $$' &\n"
            "b=$!\n"
            "wait \"$b\"\n"
            "y=$?\n"
            "if [ \"$x\" -gt 128 ] && [ \"$y\" -gt 128 ];"
            " then printf 'above-128\\n'; else printf 'x=%s y=%s\\n' \"$x\" \"$y\"; fi\n"
            "if [ \"$x\" -ne \"$y\" ]; then printf 'distinct\\n';"
            " else printf 'not-distinct\\n'; fi\n"
        ),
        stdout="above-128\ndistinct\n",
        timeout=15.0,
    ),
    # [spec:posix:req:builtin.wait.operand-pid-job-id/test]
    Case(
        id="sig-wait-job-id-operand",
        rules=("builtin.wait.operand-pid-job-id",),
        script=(
            "sh -c 'exit 4' &\n"
            "wait %1\n"
            "printf 'status=%s\\n' \"$?\"\n"
        ),
        stdout="status=4\n",
    ),
    # [spec:posix:req:builtin.wait.stderr/test]
    # [spec:posix:req:builtin.wait.interfaces/test]
    Case(
        id="sig-wait-stderr-and-interfaces",
        rules=("builtin.wait.stderr", "builtin.wait.interfaces"),
        script=(
            "sh -c 'exit 0' &\n"
            "wait > out.txt\n"
            "if [ -s out.txt ]; then printf 'stdout-used\\n';"
            " else printf 'no-stdout\\n'; fi\n"
            "read v\n"
            "printf 'read=%s\\n' \"$v\"\n"
        ),
        stdin="LINE\n",
        stdout="no-stdout\nread=LINE\n",
        stderr="",
    ),
    # ------------------------------------------------------------------
    # exit status and shell errors
    # ------------------------------------------------------------------
    # [spec:posix:req:exit.shell-error-consequences/test]
    Case(
        id="exit-noninteractive-error-consequences",
        rules=("exit.shell-error-consequences",),
        script=(
            "check() {\n"
            "  out=$(sh -c \"$2\" 2>/dev/null)\n"
            "  case $out in\n"
            "    *CONT*) printf '%s=continued\\n' \"$1\" ;;\n"
            "    *) printf '%s=abandoned\\n' \"$1\" ;;\n"
            "  esac\n"
            "}\n"
            "check syntax 'printf START; ; printf CONT'\n"
            "check special-builtin 'set -o nosuchopt; printf CONT'\n"
            "check special-builtin-via-command 'command set -o nosuchopt; printf CONT'\n"
            "check other-utility 'ls /nonexistent-xyz; printf CONT'\n"
            "check redirect-special-builtin ': < /nonexistent-xyz; printf CONT'\n"
            "check redirect-compound '{ :; } < /nonexistent-xyz; printf CONT'\n"
            "check redirect-function 'f() { :; }; f < /nonexistent-xyz; printf CONT'\n"
            "check redirect-other-utility 'true < /nonexistent-xyz; printf CONT'\n"
            "check assignment 'readonly v=1; v=2; printf CONT'\n"
            "check expansion 'printf \"%s\" \"${x!y}\"; printf CONT'\n"
        ),
        stdout=(
            "syntax=abandoned\n"
            "special-builtin=abandoned\n"
            "special-builtin-via-command=continued\n"
            "other-utility=continued\n"
            "redirect-special-builtin=abandoned\n"
            "redirect-compound=continued\n"
            "redirect-function=continued\n"
            "redirect-other-utility=continued\n"
            "assignment=abandoned\n"
            "expansion=abandoned\n"
        ),
        timeout=15.0,
    ),
    # [spec:posix:req:exit.shell-error-consequences/test]
    Case(
        id="exit-shell-error-diagnostics",
        rules=("exit.shell-error-consequences",),
        script=(
            "diag() {\n"
            "  err=$(sh -c \"$2\" 2>&1 >/dev/null)\n"
            "  if [ -n \"$err\" ]; then printf '%s=diagnostic\\n' \"$1\";\n"
            "  else printf '%s=silent\\n' \"$1\"; fi\n"
            "}\n"
            "diag syntax 'printf START; ; printf CONT'\n"
            "diag redirect-special-builtin ': < /nonexistent-xyz'\n"
            "diag redirect-compound '{ :; } < /nonexistent-xyz'\n"
            "diag redirect-function 'f() { :; }; f < /nonexistent-xyz'\n"
            "diag redirect-other-utility 'true < /nonexistent-xyz'\n"
            "diag assignment 'readonly v=1; v=2'\n"
            "diag expansion 'printf \"%s\" \"${x!y}\"'\n"
            "diag command-not-found 'nosuchcmd-xyz'\n"
        ),
        stdout=(
            "syntax=diagnostic\n"
            "redirect-special-builtin=diagnostic\n"
            "redirect-compound=diagnostic\n"
            "redirect-function=diagnostic\n"
            "redirect-other-utility=diagnostic\n"
            "assignment=diagnostic\n"
            "expansion=diagnostic\n"
            "command-not-found=diagnostic\n"
        ),
        timeout=15.0,
    ),
    # [spec:posix:def:exit.expansion-error/test]
    Case(
        id="exit-expansion-error",
        rules=("exit.expansion-error",),
        script=(
            "out=$(sh -c 'printf \"%s\" \"${x!y}\"; printf CONT' 2>/dev/null)\n"
            "st=$?\n"
            "printf 'out=[%s]\\n' \"$out\"\n"
            "if [ \"$st\" -ne 0 ]; then printf 'nonzero\\n';"
            " else printf 'zero\\n'; fi\n"
        ),
        stdout="out=[]\nnonzero\n",
    ),
    # [spec:posix:req:exit.subshell-error-exit/test]
    Case(
        id="exit-subshell-error-exit",
        rules=("exit.subshell-error-exit",),
        script=(
            "( readonly v=1; v=2; printf 'inner-continued\\n' ) 2>/dev/null\n"
            "s=$?\n"
            "if [ \"$s\" -ne 0 ]; then printf 'subshell-nonzero\\n';"
            " else printf 'subshell-zero\\n'; fi\n"
            "( printf '%s' \"${x!y}\"; printf 'inner-continued\\n' ) 2>/dev/null\n"
            "s=$?\n"
            "if [ \"$s\" -ne 0 ]; then printf 'expansion-nonzero\\n';"
            " else printf 'expansion-zero\\n'; fi\n"
            "printf 'outer-continues\\n'\n"
        ),
        stdout="subshell-nonzero\nexpansion-nonzero\nouter-continues\n",
    ),
    # [spec:posix:req:exit.unrecoverable-read-error/test]
    # [spec:posix:req:exit.shell-error-consequences/test]
    Case(
        id="exit-unrecoverable-read-error-stdin",
        rules=("exit.unrecoverable-read-error", "exit.shell-error-consequences"),
        # Reading commands from a directory descriptor fails with EISDIR; the
        # table requires the shell to exit and to write a diagnostic.
        script=(
            "mkdir d || exit 1\n"
            "err=$(sh < d 2>&1 >/dev/null)\n"
            "st=$?\n"
            "if [ \"$st\" -ne 0 ]; then printf 'exit-nonzero\\n';"
            " else printf 'exit-zero\\n'; fi\n"
            "if [ -n \"$err\" ]; then printf 'diagnostic\\n';"
            " else printf 'no-diagnostic\\n'; fi\n"
        ),
        stdout="exit-nonzero\ndiagnostic\n",
    ),
    # [spec:posix:req:exit.unrecoverable-read-error/test]
    Case(
        id="exit-unrecoverable-read-error-dot",
        rules=("exit.unrecoverable-read-error",),
        # "An unrecoverable read error while reading from the file operand of
        # the dot special built-in shall be treated as a special built-in
        # utility error", which a non-interactive shell must exit on.
        script=(
            "mkdir d || exit 1\n"
            "out=$(sh -c '. ./d; printf CONT' 2>/dev/null)\n"
            "case $out in\n"
            "  *CONT*) printf 'continued\\n' ;;\n"
            "  *) printf 'abandoned\\n' ;;\n"
            "esac\n"
        ),
        stdout="abandoned\n",
    ),
    # [spec:posix:req:exit.interactive-abandons-command/test]
    # [spec:posix:req:exit.shell-error-consequences/test]
    Case(
        id="exit-interactive-abandons-command",
        rules=("exit.interactive-abandons-command", "exit.shell-error-consequences"),
        mode="interactive",
        environment={"PS1": "", "PS2": ""},
        script=(
            "printf 'ONE\\n'; printf '%s' \"${x!y}\"; printf 'TWO\\n'\n"
            "printf 'THREE\\n'\n"
        ),
        stdout=None,
        status="any",
        stdout_contains=("THREE\n",),
        stdout_excludes=("TWO",),
    ),
    # [spec:posix:req:exit.shell-error-consequences/test]
    Case(
        id="exit-interactive-survives-errors",
        rules=("exit.shell-error-consequences",),
        mode="interactive",
        environment={"PS1": "", "PS2": ""},
        script=(
            "set -o nosuchopt\n"
            ": < /nonexistent-xyz\n"
            "readonly rv=1\n"
            "rv=2\n"
            "; printf 'BAD\\n'\n"
            "printf 'STILL-ALIVE\\n'\n"
        ),
        stdout=None,
        status="any",
        stdout_contains=("STILL-ALIVE\n",),
        stdout_excludes=("BAD\n",),
    ),
    # [spec:posix:req:exit.status-signal-terminated/test]
    Case(
        id="exit-status-signal-terminated",
        rules=("exit.status-signal-terminated",),
        script=(
            "sh -c 'kill -9 $$'\n"
            "x=$?\n"
            "sh -c 'kill -15 $$'\n"
            "y=$?\n"
            "if [ \"$x\" -gt 128 ] && [ \"$y\" -gt 128 ];"
            " then printf 'above-128\\n'; else printf 'x=%s y=%s\\n' \"$x\" \"$y\"; fi\n"
            "if [ \"$x\" -ne \"$y\" ]; then printf 'identifies-signal\\n';"
            " else printf 'not-distinct\\n'; fi\n"
        ),
        stdout="above-128\nidentifies-signal\n",
        timeout=15.0,
    ),
    # [spec:posix:req:exit.status-normal-termination/test]
    Case(
        id="exit-status-normal-termination",
        rules=("exit.status-normal-termination",),
        script=(
            "sh -c 'exit 42'\n"
            "printf '%s\\n' \"$?\"\n"
            "sh -c 'exit 0'\n"
            "printf '%s\\n' \"$?\"\n"
            "sh -c 'exit 300'\n"
            "printf '%s\\n' \"$?\"\n"
        ),
        # WEXITSTATUS, i.e. the value modulo 256 for a C program.
        stdout="42\n0\n44\n",
    ),
)
