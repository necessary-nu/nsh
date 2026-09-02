# Fuzzing plan

## Why this is a plan and not a list of targets

The first two targets were written one at a time, and both assert only
that the shell *returns*. That is the weakest thing a fuzzer can check,
and it is why the two crashes found so far were both panics rather than
wrong answers. A shell that answers `0` where Bash answers `1` will never
trip a crash oracle, and most of what a shell gets wrong is an answer.

So the plan is organised by **oracle strength**, not by component. Each
target is a (surface, oracle) pair, and the interesting axis is the
oracle.

## The oracle ladder

**1. It returns.** No panic, no stack overflow, no hang. Cheap, applies
to every surface, catches the class that killed us twice. Necessary and
nowhere near sufficient.

**2. It holds a property.** An invariant the shell must satisfy against
*itself*, with no second implementation needed:

* `eval "y=${x@Q}"` must reproduce `x` exactly, for any byte string.
* `printf %q` then re-read, likewise.
* `parse → print → parse` must reach the same tree.
* Field splitting: with `IFS` unset, joining the fields of `$*` with a
  space must reproduce the input's significant text.

Properties are strictly stronger than crash oracles and cost nothing per
run. They are the biggest gap in what exists today.

**3. It agrees with the reference.** Run the same script under this shell
and under GNU Bash 5.2 / dash, compare stdout, stderr and status.
Strongest available, and the only oracle that catches a *wrong answer* in
a construct nobody thought to write a case for.

`[dec:nsh:differential-is-the-oracle]` already says this is the
authority. What exists is 119 *static* corpora against dash covering
POSIX mode. Nothing differential exists for the Bash dialect at all --
arrays, `[[ ]]`, process substitution, `select`, `time`, the sparse
descriptor table -- which is the single largest hole in the shell's
testing.

## Input generation

Random bytes reach the lexer well and the evaluator almost never: nearly
every mutation is a syntax error, so a byte-level fuzzer spends its life
in `command()` and never expands a parameter. Three sources, in
increasing structure:

1. **Bytes**, seeded from the Smoosh and Oils corpora. What the `parse`
   target uses. Right for the lexer and parser.
2. **Dictionary.** libFuzzer `-dict=` with shell tokens -- operators,
   reserved words, `${`, `$((`, `<<`, `@Q`, the extglob prefixes. Cheap,
   and it is the difference between the mutator discovering `${x:-` by
   chance and having it in hand.
3. **Structure-aware.** An `arbitrary`-derived AST, printed to a script.
   Guarantees a syntactically valid, arbitrarily deep input, which is the
   only practical way to fuzz the *evaluator* rather than the parser.
   Required for the differential targets, where a syntax error tells you
   nothing because both shells reject it.

## Targets

Status: **done**, **next**, **blocked**.

| # | surface | oracle | forks | status |
|---|---|---|---|---|
| 1 | Parser: bytes → AST, under `noexec` | returns | no | done |
| 2 | Glob, extglob and ERE matching | returns + hang | no | done |
| 3 | Quoting round-trip: `${x@Q}`, `printf %q` | **property** | no | done |
| 4 | Field splitting and `IFS` | **property** | no | done |
| 5 | Arithmetic `$(( ))` | returns + differential | no | done |
| 6 | Parameter expansion `${...}` operators | returns + differential | no | done |
| 7 | `printf` format interpreter | returns + differential | no | done |
| 8 | Here-documents, redirection, descriptor table | returns + differential | some | done |
| 9 | `parse → print → parse` | **property** | no | done |
| 10 | Whole shell vs GNU Bash 5.2, Bash dialect | **differential** | yes | done |
| 11 | Whole shell vs dash, POSIX mode | **differential** | yes | partial |
| 12 | Locale and multibyte decoding | returns | no | later |

Ordering rationale: 3 and 4 are properties reachable through the public
API today, so they were the cheapest strength upgrade available. 10 was
the largest hole; its generator is now the thing to extend, since every
construct it cannot emit is a construct nothing differential covers -- and
it is where 9's budget belongs too, because byte mutation reaches the
degenerate corner of the grammar and stays there while the defects that
matter live in programs people write.

## What is blocked, and on what

* **Target 10 needs a divergence policy.** A differential fuzzer against
  Bash will find divergences that are deliberate --
  `[dec:nsh:safety-trumps-compatibility]` has three registered already --
  and it needs somewhere to record them or it will report the same known
  differences forever. The Oils `BASH_DISPOSITIONS.toml` register is the
  right shape and should be reused rather than a second one invented.

## Operating it

**Corpus.** Gitignored; seeded from the repository's own shell text by
`fuzz/run.sh`. It grows without bound and nothing is deleted from it: `cargo
fuzz cmin` would minimise it in place and discard the seeded scripts, which
are the provenance the archive exists to keep. `fuzz/run.sh --derive TARGET`
instead derives a coverage-equivalent campaign corpus beside it, and running
every archived input is how it does that -- so the reduction and the
regression check are one pass. See the README.

**Crashes.** Minimise, fix the cause, then sweep. `fuzz/sweep.sh TARGET`
replays every stored artifact against the current build and reports which
still reproduce; `--prune` removes the rest, so the artifact directory
stays a list of open findings rather than a history of closed ones. A
`slow-unit-` or `timeout-` artifact reproduces by *costing* rather than by
failing, so the sweep measures those against what an ordinary input of the
same target costs instead of reading their exit status; see the README. One
fix usually kills a family: the round-trip corpus went from 284 artifacts
to 3 across four fixes, so triaging artifact by artifact is triaging the
same defect thirty times.

What gets pinned is the *mechanism*, named, with the artifact hash in a
comment. A bug found is a test written -- not an artifact found. The cost
of the other rule was measured rather than guessed: the round-trip table
reached 111 tests over 101 distinct inputs, ten of them byte-for-byte
duplicates, with `<<t\n${f-'}` pinned six times under six hashes, for
something closer to eight real defects. The named cases are the ones
anyone reads afterwards.

**Sanitizers.** ASan by default through `cargo-fuzz`. A debug-profile run
is worth doing periodically as well: `debug_assert!` fires there and not
in release, and this tree has several that encode real invariants.

**Containment.** Use one outer sandbox. `fuzz/run.sh` is the normal entry
point on both a host and in a managed Codex workspace: it detects the latter
and runs `cargo fuzz` directly under the existing boundary; on a host it
wraps Cargo with `scripts/sandboxed`. `--containment outer` overrides the
detector for another managed environment, `--containment new` forces the
host wrapper, and `--dry-run` exposes the selected command without running
the fuzzer.

**Cadence.** There is no CI in this repository, so this is a manual
discipline, and the discipline is to stop when you have a finding. Run a
byte target until it produces a root cause you have not seen; then stop,
fix it, sweep the corpus, and resume. Running past that point buys copies
rather than information -- an hour against a printer with three open
losses is an hour of rediscovering them, which is how 284 artifacts came
to be waiting at once.

The number to watch is distinct root causes per hour, not artifacts per
hour. Below roughly one an hour a target has said what it knows and the
budget belongs on the next rung. Run the differential targets before
closing a compatibility node.

## Findings so far

Recorded because the point of the ladder is that each rung finds a class
the one below cannot, and that is now measured rather than argued.

| rung | target | finding |
|---|---|---|
| returns | `parse` | `<<${e` -- five bytes -- panicked; four sites read a byte out of an end-of-input item |
| property | `quoting` | the shell emitted `$'\E'` from `@Q` and could not read it back; one byte, found in seconds |
| differential | `differential` | Bash accepts `! ! cmd`, this shell refused it |
| differential | `differential` | unquoted `$@` with non-whitespace `IFS` lost an empty field at a positional boundary |
| property | `roundtrip` | `declare -f` printed a function body that runs differently -- and the fixed-point oracle passed it |

`parse` and `matcher` executed over 800,000 inputs between them without
finding either of the two differential findings, because both only assert
that the shell *returns*, and in both cases it returned happily with the
wrong answer.

The last row is the same lesson one rung higher, and it is the reason
`[spec:nsh:req:idiom.printable-ast]` exists. `roundtrip` asked for a fixed
point -- print twice, get the same text -- which any output the printer can
spell consistently satisfies, including output that means something else.
`echo "${a+"a}b"}"` prints as `echo "${a+a}b}"`, runs differently, and is a
fixed point, so 107 artifacts of triage never surfaced it. A property is
only as strong as the thing it refuses.

## What "properly" means here

A fuzz target that only checks for crashes is a smoke test with extra
steps. The measure of this plan is how much of it sits on rungs 2 and 3
of the ladder -- two of eleven targets, so far, and both of them found
something on their first run.
