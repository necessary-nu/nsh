# Fuzzing

Coverage-guided fuzzing of the shell, with libFuzzer through `cargo-fuzz`.

    fuzz/run.sh parse            # until interrupted
    fuzz/run.sh parse 600        # for ten minutes
    fuzz/run.sh parse 600 -jobs=4

A target that does not exist is refused, against `cargo fuzz list`, before
anything is seeded. `fuzz/run.sh parse 240` typed as `fuzz/run.sh 240` used
to pass the name check, seed a corpus for `240`, create an artifact
directory for it and hand it to cargo -- a session that measures nothing
and says nothing, which is what
`[spec:nsh:req:oracle.cannot-measure-is-a-failure]` exists to refuse.

The first run seeds `fuzz/corpus/<target>` from the shell text this
repository already vendors — the Smoosh scripts and the Oils spec files.
Real scripts are far better starting points than random bytes: they reach
constructs a generator takes a very long time to stumble into. Neither the
corpus nor the artifacts are committed.

## Containment

Use exactly one process containment layer. `fuzz/run.sh` is the one command
on both a normal host and in a managed Codex workspace: it detects the
workspace boundary and runs Cargo directly there; otherwise it wraps Cargo
with `scripts/sandboxed`.

Use `--containment outer` when another managed environment already provides
the boundary and is not auto-detected. Use `--containment new` to force the
normal-host wrapper. `--dry-run` prints the selected command without seeding
a corpus or executing a target.

The reason is sharper for a fuzzer than for a test: the whole point of one
is to reach states nobody predicted, so "this target cannot execute
anything" is precisely a claim it is trying to falsify. The managed
workspace sandbox already supplies that outer boundary in Codex sessions.

## Clocks

`SECONDS` is mutation and nothing else. A campaign builds, then replays its
corpus, then spends its budget, and each of the three is on a clock of its
own and says what it cost -- because the three were once one number, and one
number cannot tell a campaign that measured nothing from a campaign that
measured and found nothing.

**The build.** `cargo fuzz run` compiles first and only then hands the
binary its `-max_total_time`, so a build inside the campaign's wall clock is
paid for out of the fuzzing budget: a cold-cache `differential 60` spent 54
of its 180 seconds compiling and was killed part-way through the campaign at
exit 124 -- which is exactly what the fuzzer stopping at its own timeout
looks like, so a run that had barely started reported as a short campaign
that found nothing. The build goes first under a clock of its own
(`NSH_FUZZ_BUILD_TIMEOUT`, 1800 seconds by default). The campaign then runs
the built binary directly, the way `fuzz/sweep.sh` already replays
artifacts, rather than through `cargo fuzz run` -- which would compile a
second time inside the budget's clock, or, with another campaign already
running, spend that clock waiting on its Cargo build lock. A `differential
20` measured 2026-09-02 spent all 140 seconds of its wall clock doing
exactly that, and never started.

**The replay.** libFuzzer runs every input in the seed corpus before it
first consults `-max_total_time`, and that clock is measured from process
start-up, so the replay is charged to the budget -- and the budget can be
gone before a single input has been mutated. Measured 2026-09-02 on a warm
build, `fuzz/run.sh parse 10` reached the fuzzing loop at run #21259 and
stopped at run #21259: the corpus exactly, and not one mutation past it. Ten
seconds bought nothing at all, and added nothing to the corpus.

The replay is not waste. It is every stored input run against the build in
front of it, which is the regression check the corpus exists to be, and a
campaign that skipped it would be fuzzing against evidence it had not looked
at. So it is kept in full, and given a clock of its own
(`NSH_FUZZ_REPLAY_ALLOWANCE`, 900 seconds by default), rather than minimised
away. `fuzz/budget.sh` reads the `#N INITED` line libFuzzer prints at the
moment the replay ends and starts the budget there; at the end of it the
campaign is sent `SIGTERM`, which libFuzzer answers by printing its final
statistics and exiting zero. The same `parse 10`, after:

    fuzz/budget.sh: the corpus replay ended after 219s; the 10s budget starts here
    ...
    ==4== libFuzzer: run interrupted; exiting
    stat::number_of_executed_units: 23325
    stat::new_units_added:          8
    fuzz/budget.sh: replay 219s, then 11s of mutation for a 10s budget
    fuzz/run.sh: build 91s before the clock; replay and campaign 230s for a 10s budget

2067 mutations where there had been none, and eight of them kept. The start
of the budget is read off the fuzzer rather than estimated from the corpus,
so it stays right however large the corpus grows.

What that left costing wall time is the replay itself, which grew with the
corpus -- and the corpus grows every campaign. That is what the two corpora
below are for.

When the containment wall clock rather than the fuzzer's own budget is what
stopped a run, the runner says so, so a truncated campaign cannot pass for
one that measured and found nothing.

## Two corpora

`fuzz/corpus/TARGET` is the **archive**: every input any campaign has kept,
seeded from the Smoosh and Oils suites, and it is the regression set. It
grows without bound and nothing is ever deleted from it -- the seeded inputs
are real scripts that reach constructs a generator takes a very long time to
stumble into, which is exactly why `cargo fuzz cmin` is not the answer. It
minimises in place and discards inputs; a minimised archive keeps the
coverage and loses the provenance.

`fuzz/campaign/TARGET` is the **campaign corpus**, derived from the archive
and discarding nothing:

    fuzz/run.sh --derive parse

runs libFuzzer's own `-merge=1`, which executes every archived input against
the current build and writes out a set reaching the same features -- so the
expensive pass yields the regression evidence and the reduced set at once. A
campaign then seeds from that set, whose size is bounded by how many distinct
feature sets there are rather than by how many inputs have accumulated.
Measured 2026-09-02, with the load beside every number because these are wall
clocks:

                     archive          campaign      replay before   after
    parse         21,280  121 MB    2,639  15 MB    259s (load 93)   15s (load 107)
    differential   2,949   12 MB    1,553   6 MB    146s (load 22)   88s (load 9)

`fuzz/run.sh parse 10` went from 4m34 to 2m02 wall, and 95s of what is left
is a build that was not on any clock either way; the campaign itself is 26s.
`differential` gains less because its archive reduces by less -- the ratio of
the two counts is the ratio of the two replays, which is what you would
expect of a replay.

Four things make it more than a one-liner, and all four are in
`fuzz/corpus.sh`:

* **Staleness.** The derivation records the archive listing it was derived
  from. A campaign seeds from the campaign corpus *and* from whatever has
  reached the archive since, so a derivation going stale costs start-up and
  never costs coverage.
* **Age.** That listing says which inputs and not when or against what, so
  `seeding from 2659 campaign inputs standing for 21305 archived` reads the
  same whether the archive was last run against this build or against the
  tree of a fortnight ago -- and the archive is the regression set. The
  derivation writes `fuzz/campaign/TARGET.provenance` beside the listing,
  naming the commit it ran against, whether that tree was dirty, and the
  time; a campaign prints it, how many commits the tree has moved since,
  and how far the campaign corpus has grown past what the derivation
  bounded. Measured 2026-09-04 at `d27cf47`, before any of this existed:
  `parse` had last been derived on 2026-09-02 against `04582ce`, 73 commits
  and 51 `crates/` commits back, and nothing anywhere said so. That is a
  fact and not a threshold -- no campaign is refused for an old derivation
  (`[spec:nsh:req:oracle.recording-carries-its-age]`).
* **Copy-back.** libFuzzer writes what it finds into the first corpus
  directory it is given, which is now the campaign corpus. Every new input
  goes back into the archive when the campaign ends, whatever the campaign's
  status was.
* **The merge is crash-resistant by design.** It skips an input that kills
  the target, files the artifact and carries on -- so it writes findings and
  exits zero. The derivation counts the artifact directory before and after
  and reports on that rather than on the status, which is
  `[spec:nsh:req:oracle.cannot-measure-is-a-failure]`. The crashing input
  stays in the archive and stays out of the campaign corpus, so no campaign
  opens by re-running it, and `fuzz/sweep.sh` is what re-asks about it.

With no campaign corpus the runner replays the whole archive, exactly as it
always did, and says so.

## What a campaign has found

The first campaign against the current tree ran on 2026-09-01: fifteen
minutes on `differential` and five on each of the other nine. It produced
six artifacts across three targets, and none of the three had been seen by
the Oils survey, the POSIX harness or Smoosh.

* `differential` -- unquoted `$@` kept an empty positional as an empty
  field, so `set -- '' b; echo $@` printed a leading space that Bash, dash
  and POSIX do not. Fixed; `crates/nsh-cli/tests/unquoted_positional_fields.rs`
  pins it. The corpora set positionals to non-empty words, which is why
  none of them reached it.
* `parameter` -- `${X/$P/$R}` substitutes a different span from Bash when
  the pattern is made of metacharacters and the value is long. Filed as
  `close-the-pattern-replacement-divergence`.
* `matcher` -- four inputs of about four hundred bytes take between eleven
  and ninety-two seconds to match. libFuzzer files these as `slow-unit`
  rather than `crash`, so a replay of one exits zero. Fixed by `2c40a0e`
  and `e6551ef`; the sweep below is what stopped that exit status meaning
  "closed".

A divergence used to be reported as two fingerprints, which identifies it
across runs and says nothing about what it was, so every triage began by
writing a program to recover the script from the artifact. The report now
carries the script and both outputs.

## Sweeping the artifacts

`fuzz/sweep.sh` replays every stored artifact against the current build and
says which still reproduce; `--prune` deletes the rest, so the artifact
directory stays a list of open findings rather than a history of closed
ones. One fix usually kills a family -- the round-trip corpus went from 284
artifacts to 3 across four fixes -- which is what makes sweeping cheaper
than triaging the same defect thirty times.

**Two classes, and only one of them fails its replay.** A `crash-`, `leak-`
or `oom-` artifact is an input the target could not survive, so its replay
exits non-zero and the status settles it. A `slow-unit-` or a `timeout-` is
an input the target *survived* and took too long over, so its replay exits
zero -- and a sweep that reads only the status calls every performance
finding closed. It does that most eagerly right after a campaign has found
one, because that is when the sweep gets run. On the tree that still had
the alternation defect, the old sweep reported all five `matcher` slow units
"no longer reproduce" while each of them still took between seven and eleven
seconds, and `--prune` would have deleted all five.

**What a cost artifact is measured against.** Not a wall clock: the same
artifact read 1.16s on a quiet machine and 25.27s an hour later under load,
and a threshold in seconds turns that into a different verdict. It is
measured against what an *ordinary* input of the same target costs, taken
from the target's own corpus in the same replay, seconds apart, on the same
machine -- so whatever the machine is doing to one number it is doing to the
other. `NSH_SWEEP_COST_ALLOWANCE` is the multiple, 256 by default; the four
closed `matcher` artifacts read 7x to 93x across nine sweeps at loads from 8
to 95, and the same four read 2,282x to 3,439x on the tree that still has
the defect. Between load 8 and load 95 the sweep's own wall clock doubled
and no verdict moved. `d771ff817c9f` reads 480x to 587x on both trees and stays live
on purpose: `e6551ef` brought it to 1.16s rather than to tenths, and
`stop-selecting-the-locale-per-character` is the open node that says why.

**A corpus that cannot answer leaves the artifact undecided**, which is
neither live nor closed: `--prune` keeps it and the sweep's status says the
sweep was not clean. `[spec:nsh:req:oracle.cannot-measure-is-a-failure]`.

## Why this exists

Two reachable `SIGSEGV`s were found by hand before any of this was here,
in about ten minutes, and neither was found by the 119 differential
corpora, the POSIX harness, the Oils survey or the Smoosh suite. Those all
check what the shell *answers*; nothing checked that it answers at all.

* Nesting past roughly 1,700 levels overflowed the stack — under `-n`, so
  a syntax check on a hostile file was enough to kill the process.
* `f() { f; }; f`, mutual recursion and a script that sources itself all
  overflowed it too.

Both are bounded now (`bound-recursive-evaluation`). Fuzzing is what looks
for the next one, and the bounds had to land first or every run would find
only these.

Under `[dec:nsh:shell-as-library]` a crash is worse than a wrong answer: an
embedder gets a dead process where an `Err` was promised, with no unwind to
catch and no way to contain it.

## Targets

### `parse`

Arbitrary bytes through the parser, with `noexec` set. The parser must
answer every byte sequence with a tree or an `Err` — never a panic, a
blown stack or a hang.

`noexec` is what keeps the target pure and therefore fast: nothing parsed
is run, so no corpus entry forks, writes or spawns. A fresh `Shell` is
built per input, because option state, aliases, functions and variables
all outlive one `run`, and sharing one would make every finding depend on
the inputs before it.

Roughly 260 executions a second, dominated by that construction. Worth
improving if the target ever needs to run for days rather than hours.

### `matcher`

Arbitrary patterns against arbitrary subjects, through `case`
(glob), `[[ =~ ]]` (ERE) and `shopt -s extglob` (extended glob).

Both engines carry *budgets* rather than answers -- the glob matcher
memoizes on `(pattern node, offset)`, the ERE engine answers `no match` at
a step and depth budget -- and
`[dec:nsh:safety-trumps-compatibility]` makes those a semantic
commitment: "the budget cannot be raised or removed as a performance
tweak". A budget nobody attacks is a budget nobody has checked, and the
regex engine segfaulted on `[[ $s =~ (a)+ ]]` over a long subject once
already.

Neither byte string is spliced into script text. The scripts are fixed and
the fuzzer's bytes arrive through the *environment*, so a pattern cannot
quote its way out into a command -- which also means this target never
forks and runs at parser speed. A NUL separates pattern from subject,
unambiguous because a shell variable cannot hold one.

A hang counts as a finding here: libFuzzer's `-timeout` reports it, and an
unbounded backtrack is what the budgets exist to prevent.

## What is not covered yet

* **Long-running campaigns.** The target set now covers parser,
  matcher, quoting, field splitting, arithmetic, parameter expansion,
  `printf`, redirection and Bash-mode differential runs. The remaining
  work is operational: scheduled campaigns, corpus reduction, and turning
  each finding into a regression before the fix lands.
* **Differential dispositions.** Bash-mode differential fuzzing will keep
  finding deliberate divergences. Those need a register, in the shape of
  Oils' `BASH_DISPOSITIONS.toml`, so known policy differences do not keep
  reporting as fresh failures.

## Maintenance

`fuzz/` is its own Cargo workspace, because libFuzzer needs nightly and a
`-Z` sanitizer flag while the shell must keep building on stable. That
also means the workspace `fmt` and `clippy` gates do not see it, so a
target can rot without anything reporting it. Build it after touching the
public API:

    cargo +nightly fuzz build
