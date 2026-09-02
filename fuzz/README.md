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

What this costs is wall time: a campaign is its budget plus a replay that
grows with the corpus, and `NSH_FUZZ_REPLAY_ALLOWANCE` has to be raised as
that happens. It fails loudly and names itself when it is too low, which is
the trade the obvious alternative does not offer: `cargo fuzz cmin` would
keep short campaigns short by *discarding* inputs, and this corpus was
seeded from the Smoosh and Oils suites precisely because real scripts reach
constructs a generator takes a very long time to stumble into.

When the containment wall clock rather than the fuzzer's own budget is what
stopped a run, the runner says so, so a truncated campaign cannot pass for
one that measured and found nothing.

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
  rather than `crash`, so a replay exits zero and a sweep does not see
  them; they have to be read. Filed as
  `bound-the-cost-of-matching-a-pattern`.

A divergence used to be reported as two fingerprints, which identifies it
across runs and says nothing about what it was, so every triage began by
writing a program to recover the script from the artifact. The report now
carries the script and both outputs.

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
