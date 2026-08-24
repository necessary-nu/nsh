# Fuzzing

Coverage-guided fuzzing of the shell, with libFuzzer through `cargo-fuzz`.

    fuzz/run.sh parse            # until interrupted
    fuzz/run.sh parse 600        # for ten minutes
    fuzz/run.sh parse 600 -jobs=4

The first run seeds `fuzz/corpus/<target>` from the shell text this
repository already vendors — the Smoosh scripts and the Oils spec files.
Real scripts are far better starting points than random bytes: they reach
constructs a generator takes a very long time to stumble into. Neither the
corpus nor the artifacts are committed.

## Containment is not optional

`fuzz/run.sh` goes through `scripts/sandboxed`, and nothing here should be
run any other way. The rule is the same one the rest of the test tree
follows, and the reason is sharper for a fuzzer than for a test: the whole
point of one is to reach states nobody predicted, so "this target cannot
execute anything" is precisely a claim it is trying to falsify.

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

* **Expansion and execution.** `Shell::expand_word` runs command
  substitution by design, so a target over it executes whatever the fuzzer
  writes. That is survivable under the sandbox and is how the differential
  corpora already work, but it forks per iteration and wants its own
  budget rather than being bolted onto this one.
* **A parse/print round-trip.** `parse -> print -> parse` should reach the
  same tree, and the printer in `crates/nsh/src/nodes/source.rs` is the
  natural oracle. It is `pub(crate)`, so this needs an internals surface
  the fuzz crate can see — a `fuzzing` feature — which is a decision about
  the public surface rather than a line of code.
* **Differential fuzzing against Bash.** The 119 corpora under
  `tests/corpus/` are differential against *dash* and cover POSIX mode
  only; everything the Bash dialect adds has no differential oracle at
  all.

## Maintenance

`fuzz/` is its own Cargo workspace, because libFuzzer needs nightly and a
`-Z` sanitizer flag while the shell must keep building on stable. That
also means the workspace `fmt` and `clippy` gates do not see it, so a
target can rot without anything reporting it. Build it after touching the
public API:

    cd fuzz && cargo +nightly fuzz build
