# Where the port deliberately differs from dash

This is a bug-for-bug port. When the Rust and the C disagree, the default
assumption is that the Rust is wrong — that is what
`tests/harness/runall.sh` and the POSIX suite's `--reference` mode are
for, and both currently report zero unexplained differences.

This file is the exception list. A difference recorded here is
**sanctioned**: the harness will show it, and it must not be read as a
regression. A difference *not* recorded here is a defect, whichever side
it is on.

Three things can happen when the port and dash disagree about something
POSIX requires, and only the third belongs in this file:

1. **The port is wrong.** Fix the port. This is the overwhelmingly common
   case — see the errata commits.
2. **dash has a bug.** Fix it in the C on a branch cut from the 0.5.13.5
   release commit so it can go upstream, merge it, then bring the spec
   and the port into line. Both languages end up agreeing, so nothing is
   recorded here. Done twice: `fc -e` reading `optionarg` instead of
   `optarg` (`fix/fc-e-optarg`), and the missing `"[%d] %d\n"` background
   job announcement (`fix/async-job-notification`).
3. **dash is deliberately not doing something.** Not a bug — a decision,
   usually to stay small. If the port decides otherwise, the two stop
   agreeing on purpose, and *that* is what gets written down below.

The distinction between 2 and 3 is a judgement about intent, not about
conformance. `fc -e` reads the wrong variable; nobody meant that. Not
managing the terminal is a design position dash has held since the
NetBSD import.

## Register

### `jobctl.save-terminal-settings` / `jobctl.fg-terminal-settings-restore`

**Status:** agreed, not yet implemented.

POSIX XCU 2.11 requires an interactive shell to save the terminal
settings when a foreground job is stopped, move the terminal to the
settings it needs to read commands, and have `fg` restore the saved ones
before sending SIGCONT.

dash does none of it. `tcsetattr` appears nowhere in the tree; the single
`tcgetattr`, at `input.c:138`, only asks whether fd 0 is a tty. A shell
that never drives the terminal has no state to save, which is a coherent
position for a shell this size.

The observable consequence is not theoretical: `^Z` out of a program that
put the terminal in raw mode and the *shell* is left in raw mode, so the
prompt misbehaves until `stty sane`. The port will implement the POSIX
behaviour and dash will not.

Scope, so nobody mistakes this for a small change: it needs terminal
state on the job structure, save points on the stop path, restore points
in `fg` and `bg`, and correct ordering against `tcsetpgrp` and SIGCONT.
The two rules are a pair — `fg` must restore "the ones that the shell
saved" — so neither can be done alone.

Expect `suspend-shell-restores-its-own-terminal-settings` to pass on the
port and fail on the reference once this lands. That mismatch is this
entry.

**Caveat on the requirement itself.** The corpus records these rules as
unconditional, but that is what the extractor saw: it detects `[UP]`,
`[XSI]` and `[OB]` only when the marker appears in the rule body, so a
marker sitting on the XCU 2.11 section heading would have been lost.
Worth checking the chapter front matter before treating "POSIX requires
it" as settled.

## Candidates not yet decided

### The port is *more* conformant than dash in two line-editing cases

`edit-history-goto-number` (`edit.history-goto`, the `[number]G` command)
and `edit-history-search-pattern-anchored`
(`edit.history-search-pattern`) pass on the port and fail on the C. The
port's line editor is nshedit; dash's is libedit; nshedit satisfies these
two and libedit does not.

Nobody chose this. It is what fell out of attaching the history, and it
is the same shape as the `edit.undo` entry that used to sit here —
which resolved itself, since nshedit reproduces dash's behaviour there
and both now fail together.

Under the bug-for-bug contract a port that is *better* than its original
is still a divergence, so the choice is the usual three:

  1. Reproduce libedit's failure, keeping the port faithful.
  2. Register these as sanctioned improvements and let the port lead.
  3. Fix libedit, so both conform and nothing is registered — the route
     `fc -e` and the background job announcement took.

Option 3 is not available in the same way here: the defect is in
libedit, not in dash, and nshedit is already the fixed implementation.
So it reduces to 1 or 2, and 2 costs nothing to hold — no maintenance,
no divergence in the shell language, only in the editor.

Left undecided deliberately. Recorded so that a future parity run
showing "2 mismatches, port ahead" is read as this entry and not as a
regression.
