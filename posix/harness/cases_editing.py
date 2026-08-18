"""Executable cases for POSIX.1-2024 `sh` command history and vi-mode line editing.

Every rule in `posix/docs/spec/line-editing.md` is inside the single `[UP]`
(User Portability Utilities) bracket that opens the sh EXTENDED DESCRIPTION, so
every case here declares ``requires=("UP",)``.

How these cases are driven
--------------------------

`mode="interactive"` runs the shell on a real controlling terminal, so raw key
sequences can be sent and the resulting command line observed. The terminal
transcript interleaves the editor's own redraw output with command output, and
cursor motion is therefore not directly observable. Each case is instead built
so that the *executed command line* reveals the edit:

* An executable ``r`` is installed on PATH which prints ``R:<args>``. The
  edited text never contains the string it will print, so a ``stdout_contains``
  assertion on ``R:...`` cannot be satisfied by the editor's echo of the line.
* A session ends by typing ``; exit`` onto the same line that was edited, so
  the whole test is one editor read wherever the rule allows it.
* Where a rule inherently spans several lines (history recall, search, `#`),
  extra lines are typed and the session is ended with a separate ``exit``.

Two environmental facts, neither of which is a shell conformance result:

* `TERM=xterm` is set explicitly. The harness default is `TERM=dumb`, which
  some editors treat as "no editing possible"; `edit.block-mode-terminals`
  makes that a permitted outcome, so a terminal that can support editing is
  used instead in order to test the requirements rather than the exemption.
* Interactive cases run inside the harness sandbox's PID namespace while the
  pty's foreground process group belongs to the wrapper outside it, so the
  shell always reports "Cannot set tty process group" and exits 2 when it
  leaves. That is an artefact of the sandbox, identical for every shell, so
  these cases assert ``status="any"`` and carry their weight in the content
  assertions. A hang is still caught: the executor records a separate
  "timed out" reason regardless of the status expectation.
"""

from __future__ import annotations

from model import Case, FileFixture


ESC = "\x1b"
BEL = "\x07"
DEL = "\x7f"
KILL = "\x15"  # <control>-U
WERASE = "\x17"  # <control>-W
LNEXT = "\x16"  # <control>-V
CAN = "\x18"  # <control>-X, used as a non-default stty erase character

VI = "set -o vi\n"

# `r` prints its arguments with a marker that never appears in the text typed
# to produce it, so the assertion cannot be satisfied by the editor's echo.
R = {".bin/r": FileFixture("#!/bin/sh\nprintf 'R:%s\\n' \"$*\"\n", 0o755)}

GLOB = dict(R)
GLOB.update(
    {
        "alpha1": FileFixture("x"),
        "alpha2": FileFixture("x"),
        "beta": FileFixture("x"),
    }
)

TERMINAL = {"PS1": "$ ", "PS2": "> ", "TERM": "xterm"}

TIMEOUT = 4.0


CASES: tuple[Case, ...] = (
    # ------------------------------------------------------------------
    # Command history list and enabling vi-mode editing
    # ------------------------------------------------------------------
    # [spec:posix:req:edit.history-list/test]
    Case(
        id="edit-history-list-histfile",
        rules=("edit.history-list",),
        script=VI + 'test -s "$HISTFILE" && printf "R:histfile\\n"; exit\n',
        stdout=None,
        stdout_contains=("R:histfile\n",),
        mode="interactive",
        environment={**TERMINAL, "HISTFILE": "{ROOT}/histfile"},
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.set-o-vi/test]
    # [spec:posix:req:edit.vi-mode-editing/test]
    Case(
        id="edit-set-o-vi-enables",
        rules=("edit.set-o-vi", "edit.vi-mode-editing"),
        script=VI + "set -o | grep '^vi' | grep -q on && printf 'R:vion\\n'; exit\n",
        stdout=None,
        stdout_contains=("R:vion\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.set-o-vi/test]
    Case(
        id="edit-set-plus-o-vi-disables",
        rules=("edit.set-o-vi",),
        script=VI
        + "set +o vi; set -o | grep '^vi' | grep -q off && printf 'R:vioff\\n'; exit\n",
        stdout=None,
        stdout_contains=("R:vioff\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # ------------------------------------------------------------------
    # vi Line Editing Insert Mode
    # ------------------------------------------------------------------
    # [spec:posix:req:edit.insert-mode-default/test]
    # [spec:posix:req:edit.insert-mode-special-characters/test]
    Case(
        id="edit-insert-mode-default",
        rules=("edit.insert-mode-default", "edit.insert-mode-special-characters"),
        script=VI + "r abc '~#|<>$'; exit\n",
        stdout=None,
        stdout_contains=("R:abc ~#|<>$\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.insert-newline/test]
    Case(
        id="edit-insert-newline-executes",
        rules=("edit.insert-newline",),
        script=VI + "r abc; exit\n",
        stdout=None,
        stdout_contains=("R:abc\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.insert-newline/test]
    Case(
        id="edit-insert-newline-enters-history",
        rules=("edit.insert-newline",),
        script=VI
        + "r one\n"
        + "fc -l | grep -q 'r one' && printf 'R:inhistory\\n'\n"
        + "exit\n",
        stdout=None,
        stdout_contains=("R:one\n", "R:inhistory\n"),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.insert-deletion/test]
    Case(
        id="edit-insert-erase-kill-werase",
        rules=("edit.insert-deletion",),
        script=VI + "junk" + KILL + "r abcX" + DEL + " zzz" + WERASE + "; exit\n",
        stdout=None,
        stdout_contains=("R:abc\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.stty-characters/test]
    Case(
        id="edit-stty-erase-character",
        rules=("edit.stty-characters",),
        script="stty erase '" + CAN + "'\n" + VI + "r abcQ" + CAN + "; exit\n",
        stdout=None,
        stdout_contains=("R:abc\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:sem:edit.insert-literal-next/test]
    Case(
        id="edit-insert-literal-next",
        rules=("edit.insert-literal-next",),
        script=VI + "r a" + LNEXT + WERASE + "b; exit\n",
        stdout=None,
        stdout_contains=("R:a" + WERASE + "b\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
        pace=0.02,
    ),
    # [spec:posix:req:edit.insert-end-of-file/test]
    Case(
        id="edit-insert-end-of-file",
        rules=("edit.insert-end-of-file",),
        # The executor appends end-of-file twice after the script, so the
        # editor receives it at the beginning of an input line.
        script="trap 'printf \"R:eof\\n\"' EXIT\n" + VI,
        stdout=None,
        stdout_contains=("R:eof\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=2.0,
        requires=("UP",),
        pace=0.02,
    ),
    # [spec:posix:sem:edit.insert-escape/test]
    # [spec:posix:req:edit.escape-to-command-mode/test]
    Case(
        id="edit-escape-enters-command-mode",
        rules=("edit.insert-escape", "edit.escape-to-command-mode"),
        script=VI + "r abcdef" + ESC + "xA; exit\n",
        stdout=None,
        stdout_contains=("R:abcde\n",),
        stdout_excludes=("R:abcdef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.escape-to-command-mode/test]
    Case(
        id="edit-command-mode-unknown-alerts",
        rules=("edit.escape-to-command-mode",),
        # <control>-^ is not an editing command: alert, and leave the line be.
        script=VI + "r abc" + ESC + "\x1e" + "A; exit\n",
        stdout=None,
        stdout_contains=(BEL, "R:abc\n"),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # ------------------------------------------------------------------
    # vi Line Editing Command Mode — general commands
    # ------------------------------------------------------------------
    # [spec:posix:req:edit.command-newline/test]
    Case(
        id="edit-command-newline-executes",
        rules=("edit.command-newline",),
        script=VI + "r abc; exit" + ESC + "\n",
        stdout=None,
        stdout_contains=("R:abc\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.command-comment/test]
    Case(
        id="edit-command-comment",
        rules=("edit.command-comment",),
        script=VI
        + "r abc"
        + ESC
        + "#\n"
        + "fc -l | grep -q '#r abc' && printf 'R:commented\\n'\n"
        + "r zzz; exit\n",
        stdout=None,
        stdout_contains=("R:commented\n", "R:zzz\n"),
        stdout_excludes=("R:abc\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.command-display-expansions/test]
    Case(
        id="edit-command-display-expansions",
        rules=("edit.command-display-expansions",),
        # No '?', '*' or '[' in the bigword, so '*' is implicitly assumed.
        script=VI + "r alph" + ESC + "=A; exit\n",
        stdout=None,
        stdout_contains=("alpha1", "alpha2", "R:alph\n"),
        mode="interactive",
        environment=TERMINAL,
        files=GLOB,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.command-complete-unique/test]
    Case(
        id="edit-command-complete-unique",
        rules=("edit.command-complete-unique",),
        script=VI + "r bet" + ESC + "\\" + "tail\n" + "exit\n",
        stdout=None,
        stdout_contains=("R:beta tail\n",),
        mode="interactive",
        environment=TERMINAL,
        files=GLOB,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.command-expand-all/test]
    Case(
        id="edit-command-expand-all",
        rules=("edit.command-expand-all",),
        script=VI + "r alph" + ESC + "*" + "tail\n" + "exit\n",
        stdout=None,
        stdout_contains=("R:alpha1 alpha2tail\n",),
        mode="interactive",
        environment=TERMINAL,
        files=GLOB,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.command-alias-insert/test]
    Case(
        id="edit-command-alias-insert",
        rules=("edit.command-alias-insert",),
        script="alias _z='r zzz; exit'\n" + VI + ESC + "@z\n" + "exit\n",
        stdout=None,
        stdout_contains=("R:zzz\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.command-case-toggle/test]
    Case(
        id="edit-command-case-toggle",
        rules=("edit.command-case-toggle",),
        script=VI + "r abcdef" + ESC + "~A; exit\n",
        stdout=None,
        stdout_contains=("R:abcdeF\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.command-case-toggle/test]
    # [spec:posix:req:edit.command-count/test]
    Case(
        id="edit-command-case-toggle-count",
        rules=("edit.command-case-toggle", "edit.command-count"),
        script=VI + "r abcdef" + ESC + "hh3~A; exit\n",
        stdout=None,
        stdout_contains=("R:abcDEF\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.command-repeat/test]
    Case(
        id="edit-command-repeat",
        rules=("edit.command-repeat",),
        script=VI + "r abcdef" + ESC + "x.A; exit\n",
        stdout=None,
        stdout_contains=("R:abcd\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # ------------------------------------------------------------------
    # Motion commands
    # ------------------------------------------------------------------
    # [spec:posix:req:edit.motion-char/test]
    Case(
        id="edit-motion-char-forward",
        rules=("edit.motion-char",),
        script=VI + "r abcdef" + ESC + "03lxA; exit\n",
        stdout=None,
        stdout_contains=("R:acdef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.motion-char/test]
    Case(
        id="edit-motion-char-space",
        rules=("edit.motion-char",),
        script=VI + "r abcdef" + ESC + "0   xA; exit\n",
        stdout=None,
        stdout_contains=("R:acdef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.motion-char/test]
    Case(
        id="edit-motion-char-backward",
        rules=("edit.motion-char",),
        script=VI + "r abcdef" + ESC + "hhxA; exit\n",
        stdout=None,
        stdout_contains=("R:abcef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.motion-char/test]
    # [spec:posix:req:edit.command-count/test]
    Case(
        id="edit-motion-char-count-clamps",
        rules=("edit.motion-char", "edit.command-count"),
        # 99h is not an error: it moves to the first character on the line.
        script=VI + "r abcdef" + ESC + "99h3lxA; exit\n",
        stdout=None,
        stdout_contains=("R:acdef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.motion-word-forward/test]
    Case(
        id="edit-motion-word-forward",
        rules=("edit.motion-word-forward",),
        script=VI + "r abc def" + ESC + "0wxA; exit\n",
        stdout=None,
        stdout_contains=("R:bc def\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.motion-word-forward/test]
    Case(
        id="edit-motion-bigword-forward",
        rules=("edit.motion-word-forward",),
        script=VI + "r ab.c def" + ESC + "0WWxA; exit\n",
        stdout=None,
        stdout_contains=("R:ab.c ef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.motion-word-end/test]
    Case(
        id="edit-motion-word-end",
        rules=("edit.motion-word-end",),
        script=VI + "r abc def" + ESC + "0exA; exit\n",
        stdout=None,
        stdout_contains=("R:ab def\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.motion-word-end/test]
    Case(
        id="edit-motion-bigword-end",
        rules=("edit.motion-word-end",),
        script=VI + "r ab.c def" + ESC + "0ExA; exit\n",
        stdout=None,
        stdout_contains=("R:ab. def\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.motion-word-backward/test]
    Case(
        id="edit-motion-word-backward",
        rules=("edit.motion-word-backward",),
        script=VI + "r abc def" + ESC + "bxA; exit\n",
        stdout=None,
        stdout_contains=("R:abc ef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.motion-word-backward/test]
    Case(
        id="edit-motion-bigword-backward",
        rules=("edit.motion-word-backward",),
        script=VI + "r ab.c def" + ESC + "BxA; exit\n",
        stdout=None,
        stdout_contains=("R:ab.c ef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.motion-line-position/test]
    Case(
        id="edit-motion-line-first-nonblank",
        rules=("edit.motion-line-position",),
        # '^' must land on the 'r', not on the leading <blank>s that '0' finds.
        script=VI + "  rX abcdef" + ESC + "^lxA; exit\n",
        stdout=None,
        stdout_contains=("R:abcdef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.motion-line-position/test]
    Case(
        id="edit-motion-line-end",
        rules=("edit.motion-line-position",),
        script=VI + "r abcdefZ" + ESC + "0$xA; exit\n",
        stdout=None,
        stdout_contains=("R:abcdef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.motion-line-position/test]
    Case(
        id="edit-motion-line-start",
        rules=("edit.motion-line-position",),
        script=VI + "Xr abcdef" + ESC + "0xA; exit\n",
        stdout=None,
        stdout_contains=("R:abcdef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.motion-line-position/test]
    Case(
        id="edit-motion-line-column",
        rules=("edit.motion-line-position",),
        # The first character position is numbered 1, so 3| is the 'X'.
        script=VI + "r Xabcdef" + ESC + "3|xA; exit\n",
        stdout=None,
        stdout_contains=("R:abcdef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.motion-char-search/test]
    Case(
        id="edit-motion-find-forward",
        rules=("edit.motion-char-search",),
        script=VI + "r abZcdef" + ESC + "0fZxA; exit\n",
        stdout=None,
        stdout_contains=("R:abcdef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.motion-char-search/test]
    Case(
        id="edit-motion-find-backward",
        rules=("edit.motion-char-search",),
        script=VI + "r abZcdef" + ESC + "FZxA; exit\n",
        stdout=None,
        stdout_contains=("R:abcdef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.motion-char-search/test]
    Case(
        id="edit-motion-till-forward",
        rules=("edit.motion-char-search",),
        script=VI + "r abZcdef" + ESC + "0tZxA; exit\n",
        stdout=None,
        stdout_contains=("R:aZcdef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.motion-char-search/test]
    Case(
        id="edit-motion-till-backward",
        rules=("edit.motion-char-search",),
        script=VI + "r abZcdef" + ESC + "TZxA; exit\n",
        stdout=None,
        stdout_contains=("R:abZdef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.motion-char-search-repeat/test]
    Case(
        id="edit-motion-find-repeat",
        rules=("edit.motion-char-search-repeat",),
        script=VI + "r aZbZc" + ESC + "0fZ;xA; exit\n",
        stdout=None,
        stdout_contains=("R:aZbc\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.motion-char-search-repeat/test]
    Case(
        id="edit-motion-find-repeat-reverse",
        rules=("edit.motion-char-search-repeat",),
        script=VI + "r aZbZc" + ESC + "0fZfZ,xA; exit\n",
        stdout=None,
        stdout_contains=("R:abZc\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # ------------------------------------------------------------------
    # Entering insert mode
    # ------------------------------------------------------------------
    # [spec:posix:req:edit.enter-insert-mode/test]
    Case(
        id="edit-enter-insert-append",
        rules=("edit.enter-insert-mode",),
        script=VI + "r abc" + ESC + "aZ" + ESC + "AY; exit\n",
        stdout=None,
        stdout_contains=("R:abcZY\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.enter-insert-mode/test]
    Case(
        id="edit-enter-insert-before-cursor",
        rules=("edit.enter-insert-mode",),
        script=VI + "r abd" + ESC + "iX" + ESC + "A; exit\n",
        stdout=None,
        stdout_contains=("R:abXd\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.enter-insert-mode/test]
    Case(
        id="edit-enter-insert-line-start",
        rules=("edit.enter-insert-mode",),
        script=VI + "abcd" + ESC + "Ir " + ESC + "A; exit\n",
        stdout=None,
        stdout_contains=("R:abcd\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.enter-insert-mode/test]
    Case(
        id="edit-enter-insert-replacing",
        rules=("edit.enter-insert-mode",),
        script=VI + "r abcdXY" + ESC + "hRzz" + ESC + "A; exit\n",
        stdout=None,
        stdout_contains=("R:abcdzz\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # ------------------------------------------------------------------
    # Change, delete, yank, put, undo
    # ------------------------------------------------------------------
    # [spec:posix:req:edit.change-motion/test]
    Case(
        id="edit-change-word",
        rules=("edit.change-motion",),
        script=VI + "r abc def" + ESC + "0wcwZZ" + ESC + "A; exit\n",
        stdout=None,
        stdout_contains=("R:ZZ def\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.change-motion/test]
    Case(
        id="edit-change-whole-line",
        rules=("edit.change-motion",),
        script=VI + "r abcdef" + ESC + "ccr zzz" + ESC + "A; exit\n",
        stdout=None,
        stdout_contains=("R:zzz\n",),
        stdout_excludes=("R:abcdef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:sem:edit.change-to-end-and-line/test]
    Case(
        id="edit-change-to-end-of-line",
        rules=("edit.change-to-end-and-line",),
        script=VI + "r abcdef" + ESC + "hhCZ" + ESC + "A; exit\n",
        stdout=None,
        stdout_contains=("R:abcZ\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:sem:edit.change-to-end-and-line/test]
    Case(
        id="edit-change-clear-edit-line",
        rules=("edit.change-to-end-and-line",),
        script=VI + "r abcdef" + ESC + "Sr zzz" + ESC + "A; exit\n",
        stdout=None,
        stdout_contains=("R:zzz\n",),
        stdout_excludes=("R:abcdef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.replace-char/test]
    Case(
        id="edit-replace-char",
        rules=("edit.replace-char",),
        script=VI + "r abcdef" + ESC + "rZA; exit\n",
        stdout=None,
        stdout_contains=("R:abcdeZ\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.replace-char/test]
    # [spec:posix:req:edit.command-count/test]
    Case(
        id="edit-replace-char-count",
        rules=("edit.replace-char", "edit.command-count"),
        script=VI + "r abcdef" + ESC + "hh3rZA; exit\n",
        stdout=None,
        stdout_contains=("R:abcZZZ\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:sem:edit.append-last-bigword/test]
    Case(
        id="edit-append-last-bigword",
        rules=("edit.append-last-bigword",),
        # '_' leaves sh in insert mode, so "; exit" is typed, not appended.
        script=VI + "r firstword\n" + "r" + ESC + "_; exit\n",
        stdout=None,
        stdout_contains=("R:firstword\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:sem:edit.append-last-bigword/test]
    Case(
        id="edit-append-counted-bigword",
        rules=("edit.append-last-bigword",),
        script=VI + "r aa bb cc\n" + "r" + ESC + "2_; exit\n",
        stdout=None,
        stdout_contains=("R:aa\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.delete-char/test]
    Case(
        id="edit-delete-char-at-cursor",
        rules=("edit.delete-char",),
        script=VI + "r abcdef" + ESC + "xA; exit\n",
        stdout=None,
        stdout_contains=("R:abcde\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.delete-char/test]
    Case(
        id="edit-delete-char-before-cursor",
        rules=("edit.delete-char",),
        script=VI + "r abcdef" + ESC + "XA; exit\n",
        stdout=None,
        stdout_contains=("R:abcdf\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.delete-char/test]
    # [spec:posix:req:edit.command-count/test]
    Case(
        id="edit-delete-char-before-count",
        rules=("edit.delete-char", "edit.command-count"),
        script=VI + "r abcdef" + ESC + "3XA; exit\n",
        stdout=None,
        stdout_contains=("R:abf\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.delete-motion/test]
    Case(
        id="edit-delete-word",
        rules=("edit.delete-motion",),
        script=VI + "r abc def" + ESC + "0wdwA; exit\n",
        stdout=None,
        stdout_contains=("R:def\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.delete-motion/test]
    Case(
        id="edit-delete-to-end-of-line",
        rules=("edit.delete-motion",),
        script=VI + "r abcdef" + ESC + "hhDA; exit\n",
        stdout=None,
        stdout_contains=("R:abc\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.delete-motion/test]
    Case(
        id="edit-delete-whole-line",
        rules=("edit.delete-motion",),
        script=VI + "r abcdef" + ESC + "ddAr zzz; exit\n",
        stdout=None,
        stdout_contains=("R:zzz\n",),
        stdout_excludes=("R:abcdef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.yank-motion/test]
    Case(
        id="edit-yank-whole-line",
        rules=("edit.yank-motion",),
        script=VI + "r abcdef" + ESC + "yypA; exit\n",
        stdout=None,
        stdout_contains=("R:abcdefr abcdef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.yank-motion/test]
    Case(
        id="edit-yank-to-end-of-line",
        rules=("edit.yank-motion",),
        script=VI + "r abcdef" + ESC + "0Y$pA; exit\n",
        stdout=None,
        stdout_contains=("R:abcdefr abcdef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.put-save-buffer/test]
    Case(
        id="edit-put-after-cursor",
        rules=("edit.put-save-buffer",),
        script=VI + "r abcdef" + ESC + "XpA; exit\n",
        stdout=None,
        stdout_contains=("R:abcdfe\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.put-save-buffer/test]
    Case(
        id="edit-put-before-cursor",
        rules=("edit.put-save-buffer",),
        script=VI + "r abcdef" + ESC + "xPA; exit\n",
        stdout=None,
        stdout_contains=("R:abcdfe\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.undo/test]
    Case(
        id="edit-undo-last-change",
        rules=("edit.undo",),
        script=VI + "r abcdef" + ESC + "xxuA; exit\n",
        stdout=None,
        stdout_contains=("R:abcde\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.undo/test]
    Case(
        id="edit-undo-all-changes",
        rules=("edit.undo",),
        script=VI + "r abcdef" + ESC + "xxUA ok; exit\n",
        stdout=None,
        stdout_contains=("R:abcdef ok\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.undo/test]
    Case(
        id="edit-undo-all-preserves-history-copy",
        rules=("edit.undo",),
        script=VI + "r original\n" + ESC + "kxUA ok; exit\n",
        stdout=None,
        stdout_contains=("R:original ok\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # ------------------------------------------------------------------
    # Command history navigation and search
    # ------------------------------------------------------------------
    # [spec:posix:req:edit.history-prev-next/test]
    Case(
        id="edit-history-previous",
        rules=("edit.history-prev-next",),
        script=VI + "r one\nr two\n" + ESC + "kkA ok; exit\n",
        stdout=None,
        stdout_contains=("R:one ok\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.history-prev-next/test]
    Case(
        id="edit-history-next",
        rules=("edit.history-prev-next",),
        script=VI + "r one\nr two\n" + ESC + "3k2jA ok; exit\n",
        stdout=None,
        stdout_contains=("R:two ok\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.history-prev-next/test]
    Case(
        id="edit-history-previous-past-limit",
        rules=("edit.history-prev-next",),
        # Retreating past the oldest command alerts and has no effect.
        script=VI + "r one\n" + "r zzz" + ESC + "9kA; exit\n",
        stdout=None,
        stdout_contains=(BEL, "R:zzz\n"),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.history-prev-next/test]
    Case(
        id="edit-history-minus-plus",
        rules=("edit.history-prev-next",),
        script=VI + "r one\nr two\n" + ESC + "--+A ok; exit\n",
        stdout=None,
        stdout_contains=("R:two ok\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.history-prev-next/test]
    Case(
        id="edit-history-next-past-edit-line",
        rules=("edit.history-prev-next",),
        script=VI + "r old\n" + "r live" + ESC + "k99jA; exit\n",
        stdout=None,
        stdout_contains=(BEL, "R:live\n"),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.history-goto/test]
    Case(
        id="edit-history-goto-oldest",
        rules=("edit.history-goto",),
        script="r one\n" + VI + "r two\n" + ESC + "GA ok; exit\n",
        stdout=None,
        stdout_contains=("R:one ok\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.history-goto/test]
    Case(
        id="edit-history-goto-number",
        rules=("edit.history-goto",),
        # History event 1 is "r one"; 1G must recall exactly that line.
        script="r one\n" + VI + "r two\n" + ESC + "1GA ok; exit\n",
        stdout=None,
        stdout_contains=("R:one ok\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.history-goto/test]
    Case(
        id="edit-history-goto-missing",
        rules=("edit.history-goto",),
        # A missing event alerts and leaves the current command unchanged.
        script=VI + "r keep" + ESC + "99GA; exit\n",
        stdout=None,
        stdout_contains=(BEL, "R:keep\n"),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.history-search-backward/test]
    Case(
        id="edit-history-search-backward",
        rules=("edit.history-search-backward",),
        script=VI + "r alpha\nr beta\n" + ESC + "/alpha\nA ok; exit\n",
        stdout=None,
        stdout_contains=("R:alpha ok\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.history-search-backward/test]
    Case(
        id="edit-history-search-backward-missing",
        rules=("edit.history-search-backward",),
        script=VI + "r alpha\n" + "r keep" + ESC + "/missing\nA; exit\n",
        stdout=None,
        stdout_contains=(BEL, "R:keep\n"),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.history-search-forward/test]
    Case(
        id="edit-history-search-forward",
        rules=("edit.history-search-forward",),
        script=VI + "r alpha\nr beta\n" + ESC + "kkk?beta\nA ok; exit\n",
        stdout=None,
        stdout_contains=("R:beta ok\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.history-search-forward/test]
    Case(
        id="edit-history-search-forward-missing",
        rules=("edit.history-search-forward",),
        script=VI + "r alpha\nr beta\n" + ESC + "kk?missing\nA; exit\n",
        stdout=None,
        stdout_contains=(BEL, "R:alpha\n"),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.history-search-repeat/test]
    Case(
        id="edit-history-search-repeat",
        rules=("edit.history-search-repeat",),
        script=VI + "r alpha1\nr other\nr alpha2\n" + ESC + "/alpha\nnA ok; exit\n",
        stdout=None,
        stdout_contains=("R:alpha1 ok\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.history-search-repeat/test]
    Case(
        id="edit-history-search-repeat-reverse",
        rules=("edit.history-search-repeat",),
        script=VI + "r alpha1\nr other\nr alpha2\n" + ESC + "/alpha\nnNA ok; exit\n",
        stdout=None,
        stdout_contains=("R:alpha2 ok\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.history-search-repeat/test]
    Case(
        id="edit-history-search-repeat-no-previous",
        rules=("edit.history-search-repeat",),
        script=VI + "r keep" + ESC + "nA; exit\n",
        stdout=None,
        stdout_contains=(BEL, "R:keep\n"),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:req:edit.history-search-repeat/test]
    Case(
        id="edit-history-search-reverse-no-previous",
        rules=("edit.history-search-repeat",),
        script=VI + "r keep" + ESC + "NA; exit\n",
        stdout=None,
        stdout_contains=(BEL, "R:keep\n"),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:syn:edit.history-search-pattern/test]
    Case(
        id="edit-history-search-pattern-glob",
        rules=("edit.history-search-pattern",),
        script=VI + "r alpha\nr beta\n" + ESC + "/a*ha\nA ok; exit\n",
        stdout=None,
        stdout_contains=("R:alpha ok\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:syn:edit.history-search-pattern/test]
    Case(
        id="edit-history-search-pattern-anchored",
        rules=("edit.history-search-pattern",),
        # A leading '^' is discarded and anchors the match to line start.
        script=VI + "r alpha\nr beta\n" + ESC + "/^r alp\nA ok; exit\n",
        stdout=None,
        stdout_contains=("R:alpha ok\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:syn:edit.history-search-pattern/test]
    Case(
        id="edit-history-search-pattern-empty-reuses",
        rules=("edit.history-search-pattern",),
        # An empty pattern repeats the last non-empty one.
        script=VI + "r alpha\nr beta\n" + ESC + "/alpha\n" + "jj/\nA ok; exit\n",
        stdout=None,
        stdout_contains=("R:alpha ok\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:syn:edit.history-search-pattern/test]
    Case(
        id="edit-history-search-pattern-no-previous",
        rules=("edit.history-search-pattern",),
        # No previous pattern: alert, and the command line stays as typed.
        script=VI + "r alpha\n" + "r keep" + ESC + "/\nA ok; exit\n",
        stdout=None,
        stdout_contains=(BEL, "R:keep ok\n"),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.edit-line/test]
    # [spec:posix:req:edit.edit-line-replacement/test]
    Case(
        id="edit-line-replacement",
        rules=("edit.edit-line", "edit.edit-line-replacement"),
        # k recalls "r one"; appending '9' modifies it, which must copy the
        # modified content into the edit line. j then advances past the edit
        # line, which must restore the modified content, not "r zzz".
        script=VI + "r one\n" + "r zzz" + ESC + "kA9" + ESC + "jA; exit\n",
        stdout=None,
        stdout_contains=("R:one9\n",),
        stdout_excludes=("R:zzz\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # ------------------------------------------------------------------
    # The motion command set
    #
    # `edit.motion-command-set` names the nineteen commands that stand for
    # "motion" in the c/d/y operators. Naming that set is an obligation, not
    # a gloss: each listed command has to be usable where a motion command is
    # required, so each case below applies exactly one of them as the operand
    # of `d` and asserts the command line that is then executed.
    #
    # Two deliberate choices keep these cases about the operand and nothing
    # else:
    #
    # * The cursor is always placed explicitly with `0` and a count, never
    #   left wherever <ESC> happened to leave it. The port leaves the cursor
    #   one position past the last character on leaving insert mode (that is
    #   what edit-motion-char-backward already fails on), which would
    #   otherwise turn every backward-motion case into a second report of the
    #   same defect.
    # * Where the extent of the deletion is at stake, the expectation follows
    #   vi, which `edit.word-bigword-terms` imports by reference: forward
    #   `e`, `E`, `f`, `t`, `;` and `$` take the character they land on,
    #   `w`, `W`, `l`, `<space>` and `|` stop before it, and
    #   `edit.delete-motion` states outright that a motion toward the
    #   beginning of the line leaves the character under the cursor alone.
    # ------------------------------------------------------------------
    # [spec:posix:def:edit.motion-command-set/test]
    # [spec:posix:req:edit.delete-motion/test]
    Case(
        id="blt2-edit-motion-operand-space",
        rules=("edit.motion-command-set", "edit.delete-motion"),
        script=VI + "r abcdef" + ESC + "02ld " + "A; exit\n",
        stdout=None,
        stdout_contains=("R:bcdef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.motion-command-set/test]
    # [spec:posix:req:edit.delete-motion/test]
    Case(
        id="blt2-edit-motion-operand-zero",
        rules=("edit.motion-command-set", "edit.delete-motion"),
        # Backward motion: the 'r' under the cursor survives, the 'Z' does not.
        script=VI + "Zr abcdef" + ESC + "0ld0" + "A; exit\n",
        stdout=None,
        stdout_contains=("R:abcdef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.motion-command-set/test]
    # [spec:posix:req:edit.delete-motion/test]
    Case(
        id="blt2-edit-motion-operand-word-back",
        rules=("edit.motion-command-set", "edit.delete-motion"),
        script=VI + "r abc def" + ESC + "08ldb" + "A; exit\n",
        stdout=None,
        stdout_contains=("R:abc f\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.motion-command-set/test]
    # [spec:posix:req:edit.delete-motion/test]
    Case(
        id="blt2-edit-motion-operand-find-back",
        rules=("edit.motion-command-set", "edit.delete-motion"),
        script=VI + "r abcdef" + ESC + "07ldFb" + "A; exit\n",
        stdout=None,
        stdout_contains=("R:af\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.motion-command-set/test]
    # [spec:posix:req:edit.delete-motion/test]
    Case(
        id="blt2-edit-motion-operand-char-right",
        rules=("edit.motion-command-set", "edit.delete-motion"),
        script=VI + "r abcdef" + ESC + "02ldl" + "A; exit\n",
        stdout=None,
        stdout_contains=("R:bcdef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.motion-command-set/test]
    # [spec:posix:req:edit.delete-motion/test]
    Case(
        id="blt2-edit-motion-operand-bigword-forward",
        rules=("edit.motion-command-set", "edit.delete-motion"),
        script=VI + "r ab.c def" + ESC + "02ldW" + "A; exit\n",
        stdout=None,
        stdout_contains=("R:def\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.motion-command-set/test]
    # [spec:posix:req:edit.delete-motion/test]
    Case(
        id="blt2-edit-motion-operand-first-nonblank",
        rules=("edit.motion-command-set", "edit.delete-motion"),
        script=VI + "Zr abcdef" + ESC + "0ld^" + "A; exit\n",
        stdout=None,
        stdout_contains=("R:abcdef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.motion-command-set/test]
    # [spec:posix:req:edit.delete-motion/test]
    Case(
        id="blt2-edit-motion-operand-line-end",
        rules=("edit.motion-command-set", "edit.delete-motion"),
        script=VI + "r abcdefzz" + ESC + "08ld$" + "A; exit\n",
        stdout=None,
        stdout_contains=("R:abcdef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.motion-command-set/test]
    # [spec:posix:req:edit.delete-motion/test]
    Case(
        id="blt2-edit-motion-operand-search-repeat",
        rules=("edit.motion-command-set", "edit.delete-motion"),
        # fX leaves the cursor on the first X; d; deletes through the second.
        script=VI + "r aXbXc" + ESC + "0fXd;" + "A; exit\n",
        stdout=None,
        stdout_contains=("R:ac\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.motion-command-set/test]
    # [spec:posix:req:edit.delete-motion/test]
    Case(
        id="blt2-edit-motion-operand-bigword-end",
        rules=("edit.motion-command-set", "edit.delete-motion"),
        script=VI + "r ab.c def" + ESC + "02ldE" + "A; exit\n",
        stdout=None,
        stdout_contains=("R:def\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.motion-command-set/test]
    # [spec:posix:req:edit.delete-motion/test]
    Case(
        id="blt2-edit-motion-operand-find-forward",
        rules=("edit.motion-command-set", "edit.delete-motion"),
        script=VI + "r abcXdef" + ESC + "02ldfX" + "A; exit\n",
        stdout=None,
        stdout_contains=("R:def\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.motion-command-set/test]
    # [spec:posix:req:edit.delete-motion/test]
    Case(
        id="blt2-edit-motion-operand-till-back",
        rules=("edit.motion-command-set", "edit.delete-motion"),
        script=VI + "r abcdef" + ESC + "07ldTb" + "A; exit\n",
        stdout=None,
        stdout_contains=("R:abf\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.motion-command-set/test]
    # [spec:posix:req:edit.delete-motion/test]
    Case(
        id="blt2-edit-motion-operand-word-forward",
        rules=("edit.motion-command-set", "edit.delete-motion"),
        script=VI + "r abc def" + ESC + "02ldw" + "A; exit\n",
        stdout=None,
        stdout_contains=("R:def\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.motion-command-set/test]
    # [spec:posix:req:edit.delete-motion/test]
    Case(
        id="blt2-edit-motion-operand-column",
        rules=("edit.motion-command-set", "edit.delete-motion"),
        # The first character position is 1, so 6| is the 'd' of abcdef.
        script=VI + "r abcdef" + ESC + "02ld6|" + "A; exit\n",
        stdout=None,
        stdout_contains=("R:def\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.motion-command-set/test]
    # [spec:posix:req:edit.delete-motion/test]
    Case(
        id="blt2-edit-motion-operand-search-reverse",
        rules=("edit.motion-command-set", "edit.delete-motion"),
        # fX; leaves the cursor on the second X; d, runs back to the first,
        # and a backward motion leaves the character under the cursor.
        script=VI + "r aXbXc" + ESC + "0fX;d," + "A; exit\n",
        stdout=None,
        stdout_contains=("R:aXc\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.motion-command-set/test]
    # [spec:posix:req:edit.delete-motion/test]
    Case(
        id="blt2-edit-motion-operand-bigword-back",
        rules=("edit.motion-command-set", "edit.delete-motion"),
        script=VI + "r ab.c def" + ESC + "09ldB" + "A; exit\n",
        stdout=None,
        stdout_contains=("R:ab.c f\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.motion-command-set/test]
    # [spec:posix:req:edit.delete-motion/test]
    Case(
        id="blt2-edit-motion-operand-word-end",
        rules=("edit.motion-command-set", "edit.delete-motion"),
        script=VI + "r abc def" + ESC + "02lde" + "A; exit\n",
        stdout=None,
        stdout_contains=("R:def\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.motion-command-set/test]
    # [spec:posix:req:edit.delete-motion/test]
    Case(
        id="blt2-edit-motion-operand-char-left",
        rules=("edit.motion-command-set", "edit.delete-motion"),
        script=VI + "r abcdef" + ESC + "03ldh" + "A; exit\n",
        stdout=None,
        stdout_contains=("R:bcdef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.motion-command-set/test]
    # [spec:posix:req:edit.delete-motion/test]
    Case(
        id="blt2-edit-motion-operand-till-forward",
        rules=("edit.motion-command-set", "edit.delete-motion"),
        script=VI + "r abcXdef" + ESC + "02ldtX" + "A; exit\n",
        stdout=None,
        stdout_contains=("R:Xdef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # ------------------------------------------------------------------
    # Terminology the vi-mode sections rely on
    #
    # `edit.cursor-terminology` and `edit.word-bigword-terms` are glossary
    # entries, but each meaning they fix is observable: which character a
    # motion lands on, where the command line begins, where a word ends and a
    # bigword does not, and that there is one save buffer rather than
    # separate yank and delete registers.
    # ------------------------------------------------------------------
    # [spec:posix:def:edit.cursor-terminology/test]
    # [spec:posix:req:edit.motion-word-backward/test]
    Case(
        id="blt2-edit-cursor-word-beginning",
        rules=("edit.cursor-terminology", "edit.motion-word-backward"),
        # "beginning of the word" is the FIRST character of the word: b from
        # inside "beta" lands on its 'b', so x leaves "eta".
        script=VI + "r alpha beta" + ESC + "bx" + "A; exit\n",
        stdout=None,
        stdout_contains=("R:alpha eta\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.cursor-terminology/test]
    # [spec:posix:req:edit.motion-word-end/test]
    Case(
        id="blt2-edit-cursor-word-end",
        rules=("edit.cursor-terminology", "edit.motion-word-end"),
        # "end of the word" is the LAST character of the word, not the
        # <blank> after it: e lands on the final 'a' of "alpha".
        script=VI + "r alpha beta" + ESC + "0ex" + "A; exit\n",
        stdout=None,
        stdout_contains=("R:alph beta\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.cursor-terminology/test]
    # [spec:posix:req:edit.enter-insert-mode/test]
    Case(
        id="blt2-edit-cursor-command-line-beginning",
        rules=("edit.cursor-terminology", "edit.enter-insert-mode"),
        # The "beginning of the command line" is between the prompt and the
        # first character of the command text, so I inserts "r " in front of
        # "alpha" and nothing lands in or after the prompt.
        script=VI + "alpha" + ESC + "Ir " + ESC + "A; exit\n",
        stdout=None,
        stdout_contains=("R:alpha\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.word-bigword-terms/test]
    Case(
        id="blt2-edit-word-boundary-punctuation",
        rules=("edit.word-bigword-terms",),
        # A word ends at punctuation, so w from "ab" stops on the '.'.
        script=VI + "r ab.cd ef" + ESC + "02lwx" + "A; exit\n",
        stdout=None,
        stdout_contains=("R:abcd ef\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.word-bigword-terms/test]
    Case(
        id="blt2-edit-bigword-spans-punctuation",
        rules=("edit.word-bigword-terms",),
        # A bigword ends only at a <blank>, so W skips "ab.cd" entirely.
        script=VI + "r ab.cd ef" + ESC + "02lWx" + "A; exit\n",
        stdout=None,
        stdout_contains=("R:ab.cd f\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
    # [spec:posix:def:edit.word-bigword-terms/test]
    # [spec:posix:req:edit.delete-char/test]
    Case(
        id="blt2-edit-save-buffer-single",
        rules=("edit.word-bigword-terms", "edit.delete-char"),
        # One save buffer, vi's unnamed buffer: yl stores "r", then x stores
        # the deleted 'b' over it, so p must put back 'b' and not 'r'.
        script=VI + "r abcdef" + ESC + "0yl03lx$p" + "A; exit\n",
        stdout=None,
        stdout_contains=("R:acdefb\n",),
        stdout_excludes=("R:acdefr\n",),
        mode="interactive",
        environment=TERMINAL,
        files=R,
        status="any",
        timeout=TIMEOUT,
        requires=("UP",),
    ),
)
