# Differential test harness

Compares the Rust port under `crates/` against a C dash built from this
same tree. This is the only evidence that the port behaves like dash, so
it belongs in the repo — it lived in a scratch directory for far too long,
which meant a tmp sweep would have erased every case backing the port.

    tests/build-reference.sh     build the C oracle into tests/.build/ref
    tests/harness/dsdiff.sh      differential run: port vs reference
    tests/harness/ptydiff.py     interactive (pty) differential run
    tests/harness/covrun.sh      coverage of the C under a corpus
    tests/corpus/                111 corpora, ~183k lines

## Usage

    ./tests/build-reference.sh                      # once
    cargo build
    tests/harness/dsdiff.sh tests/corpus/everything.txt 12
    python3 tests/harness/ptydiff.py

`PORT`, `REF` and `DASH_ROOT` override the defaults, which are
`target/debug/dash` and `tests/.build/ref/src/dash` relative to the repo.

## Containment — read this before adding cases

**Test cases are hostile input. Never run one outside the harness.**

On 2026-08-02 a generated case ran `kill -- -1` — POSIX for "signal every
process this uid may signal". Running as the login uid with no isolation,
it killed tmux, the login shell, the agent daemon, every pty host and the
harness itself. `auditd` recorded `syscall=kill a0=0xffffffff`.

Three layers now stand between a case and the machine:

1. **PID namespace** (`sandboxed.sh`, via `sandbox --unshare all`). A
   process cannot signal what it cannot see. Verified: `kill -- -1` and
   `kill -9 -1` inside return ESRCH while sentinels outside — including
   one in a separate session — survive. The namespace also reaps on exit,
   so background jobs cannot leak.
2. **`corpus-lint.sh`** drops `kill`/`killall`/`pkill` before they run. A
   case may opt back in with `#!allow-kill`, for curated self-signalling
   tests only.
3. **Read-only root.** A case writes to its own scratch directory, or
   nowhere.

`timeout` is **not** containment — it bounds how long a case runs, not
what it can reach. `ds_assert_contained` aborts the run if the namespace
is not active; there is deliberately no fallback, because a harness that
silently degrades to "no sandbox" is how the incident happened.

## Three ways this harness has lied, and the guards against each

**It ran nothing.** `timeout 10 ds_sandboxed …` cannot work — `timeout`
is a binary, `ds_sandboxed` a shell function — so every invocation exited
127, both shells "failed identically", and 6,964 cases passed without a
single one executing. `ds_assert_harness_live` now requires each binary to
independently emit expected bytes before any count is reported.

**It compared the wrong binary.** Cases invoking `sh` got the *system*
shell, so both sides ran the same third binary. Each case now gets a
private `.bin/sh` symlinked to its own shell under test.

**It compared against the wrong configuration.** See
`build-reference.sh`.

The lesson each time: equality between two runs proves nothing until you
know both ran, and ran the thing you meant. When adding a corpus, check it
can *fail* — point `PORT` at `/bin/bash` and confirm failures appear.

## Corpus format

Cases are separated by lines that are exactly `%%%`; a file with no `%%%`
is one case per line. Directives on a case's leading lines:

    #!name label            label in the failure report
    #!mode=c | file | stdin how the case is fed to the shell
    #!shargs -i -E          flags before -c (needed for history: dash only
                            creates the editor with -E/-V)
    #!args a b c            argv after
    #!norm=pid              normalise runs of 3+ digits
    #!allow-kill            opt out of the kill filter (curated only)

Working directories are named `w` and `w2`, and that is load bearing —
much of the corpus normalises the shell's cwd with `sed 's|/w2$|/W|'`.

## Flaky classification

dash prints `I/O error` when a pipeline stage writes to a peer that has
already exited, which depends on scheduling: measured 19/200 for the
reference and 7/200 for the port on one input. On a mismatch the harness
re-runs both sides and records `FLAKY` rather than `FAIL` when the port
produced an output the reference also produces. `ptydiff.py` has **no**
such classifier yet and can report a spurious failure; re-run before
believing it.

Note `$$` is deterministic inside a PID namespace, so pid-based
nondeterminism does not work as a flakiness probe.

## Coverage

`covrun.sh` measures which C functions a corpus reaches. **Its numbers are
not currently trustworthy**: gcov loses the parent's counters whenever a
case forks and execs, which is most cases. Isolated to fork+exec —
`main` reads 54.17% with no fork, 54.17% with `( : )`, and 39.58% with
`/bin/true`. A likely fix is per-process `GCOV_PREFIX` plus
`gcov-tool merge`; until then treat any figure as a floor of unknown
tightness.

## Expected noise

Fuzzing a shell produces crashing shells. **Segfaults from `dash` in
`dmesg` are expected output of this harness, not machine instability.**
They are contained and do not affect the host.
