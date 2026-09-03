# What this project's own checks must do

The rules elsewhere say what the shell must do. These say what the things
that *check* the shell must do, because a check that cannot fail is worse
than no check: it occupies the place where evidence would go, and it
reports success while doing it.

This file exists because five of them were found in one sitting. Each had
been green for as long as anyone had looked, and each produced a
plausible result rather than an error.

## Oracles

> [spec:nsh:req:oracle.cannot-measure-is-a-failure]
> A check that compares behaviour against a reference MUST report a failure
> when it cannot obtain that reference. The absence of a fixture, a reference
> implementation, a comparison target, or the environment one of them needs is
> itself a result, and that result is "could not measure" -- never "passed". A
> check MUST NOT return early, skip, or narrow its assertions because its
> reference is unavailable.
>
> A check that does not apply to the host or configuration it is running on is
> a different thing and remains permitted, but MUST say so statically --
> through `cfg`, `#[ignore]`, or an equivalent the runner can report -- rather
> than by returning at run time. The distinction the rule turns on is whether
> the check knows in advance that it has nothing to measure, or discovers at
> run time that it cannot measure what it came for.

## Why it is worth a rule

The five instances took two shapes between them, which is what makes the
requirement mechanically checkable rather than merely stated:

* a check whose first statement is a guarded `return`, and
* an assertion nested inside `if let Ok(reference)` or `if let Some(fixture)`,
  so that the unavailable case falls through with nothing asserted.

They were:

| where | how it passed |
|---|---|
| the round-trip printer property | a fixed point any self-consistent output satisfied, including output that ran differently from its source |
| `nsh-survey gate-bash` | scored GNU Bash, nsh, and a stub shell that only calls `exit 7` identically, because a shell kept under `/tmp` is invisible inside the survey's own tmpfs containment |
| the differential fuzz targets | the reference shell's spawn failure returned `None` and the assertion was skipped, so every target could pass with no reference at all |
| seven locale tests across three files | `if !has_single_byte_fixture() { return; }` |
| `tests/harness/locale-sweep.sh` | setting `LOCPATH` makes glibc bypass the system locale archive, so the sweep's UTF-8 axis silently re-measured its C axis |

The last two spread by citation rather than by copying: a helper's doc
comment recorded skipping as "the established shape for a locale fixture
here", and named the file to imitate.

## How the rule is enforced

`crates/nsh/tests/oracle_measurement.rs`, which runs with the rest of the
suite under `cargo test --workspace`, beside the four other structural
lints this repository already keeps as tests. It reads every `#[test]`
function under `crates/` and reports two shapes:

| code | shape |
|---|---|
| `early-return` | a `return;` or `return Ok(())` -- an exit that reports success -- reached at run time |
| `unmeasured-branch` | every assertion in the check sits inside one `if let Ok(…)` or `if let Some(…)` that has no `else` |
| `unmeasured-provenance` | a comment says the pinned reference produced the values below it, in a file that never runs that reference |

A `return None` inside a closure is not an exit that reports success, and
is not reported.

The third code was added later, for a shape the first two cannot see.
`crates/nsh/src/escape/bash.rs` said its `%q` table was "derived byte by
byte from the pinned Bash 5.3 build" and
`crates/nsh/src/expand/typed/bash.rs` said the same of its substring
table. Neither returned early and neither nested an assertion, because
neither file held a reference at all: both tables were a transcript of
one session with that build, written down once and never run again. The
values turned out to be right, which is the part worth saying -- the
defect is not a wrong table, it is a table that could not have told
anyone it had gone wrong.

The claim is recognised by the words `from the pinned` or `against the
pinned`, `pinned` being this repository's name for its one calibrated
oracle; a file counts as running it when it reaches `pinned_bash`,
`fuzzing::reference` or `NSH_FUZZ_BASH`. A claim that names a `.rs` file
is a cross-reference rather than a claim, and is not reported -- which is
also the resolution the code asks for. Both instances were closed that
way: `crates/nsh-cli/tests/bash_quoting_and_slicing.rs` runs every case
from both tables through both shells and compares the answers, holding no
expected values of its own, and each table's comment now names it.

### The permitted static skip

A check that does not apply to the host is permitted, and the rule asks
it to say so statically. In this workspace the static spelling is a
`pub const fn … -> bool` in `nsh-platform`: `[dec:nsh:platform-boundary]`
keeps `cfg(target_os …)` out of the shell, so a host fact reaches a test
as a call the compiler has already folded to a constant. The lint reads
those declarations out of `crates/nsh-platform/src/` and treats a guard
made of nothing but one as static. Five checks are silent for this
reason, across four predicates: `can_unlink_current_directory`,
`reports_pipe_short_writes`,
`supports_glob_metacharacters_in_filenames` and
`supports_bidirectional_pseudoterminal_pair`. Turn one of those
predicates into a runtime probe and every guard using it starts being
reported, which is the distinction the rule turns on.

### The opt-out

An early return unrelated to a reference is ordinary code. A report is
suppressed by a line comment inside the check it belongs to:

```rust
// oracle-violation: early-return=the loop below asserts on every case
```

The grammar is `<code>=<reason>`, deliberately the same shape as
`nplan commit --violation`, for the same purpose: a bypass is a recorded
decision rather than a silent one. A reason shorter than a dozen
characters is refused, an unknown code is refused, and a suppression that
suppresses nothing is reported, so they cannot outlive their reason.
There are none in the tree today.

## Three other ways a check goes wrong

The rule above is about a check that cannot reach its reference. Three
neighbouring shapes were found afterwards, and none of them is that: each
obtains everything it needs and still cannot say what it claims to.

> [spec:nsh:req:oracle.refuses-a-mismeasurement]
> A check MUST refuse to run when its own configuration means it would measure
> something other than what it names, and MUST say which configuration it
> refused on. Scoring the Bash-comparison group with the dialect disabled,
> running a survey against a shell the containment boundary cannot see, judging
> a build under a `target` that is a symbolic link, and binding a work
> directory reached through one are each such a configuration.
>
> `[spec:nsh:req:oracle.cannot-measure-is-a-failure]` covers a check that
> cannot obtain its reference. This covers a check that obtains one and
> compares it against the wrong thing, which produces a number rather than an
> absence and is therefore the harder of the two to notice: the run is green,
> the count is plausible, and nothing says the shell under test was a stub, or
> the same shell twice, or the dialect the profile is not about.

> [spec:nsh:req:oracle.recording-carries-its-age]
> A checked-in recording of a reference's answers MUST carry which build of
> this shell it was made against and when. A run that reads one MUST report
> that provenance alongside its verdict, so a recording older than the code it
> is scoring is visible without anyone going to look.
>
> A regression set nothing has run against the current tree is not a regression
> set, and it does not announce itself: the count it prints is the same either
> way. Reporting the age is a fact and is required; what threshold, if any,
> should turn an old recording into a failure is a separate argument and this
> rule does not settle it.

> [spec:nsh:req:oracle.checks-do-not-share-state]
> A check MUST NOT depend on, and MUST NOT leave behind, process state it
> shares with the checks running beside it. Descriptor numbers, the
> file-creation mask, the controlling terminal, the working directory, and a
> pipe inherited by a forked child are shared by every test in one binary,
> which the harness runs in threads of one process.
>
> A check that reads a shared setting MUST read it without changing it; where
> the only way to observe something is to change it, the check MUST take the
> process to itself -- a child, or a pseudo-terminal of its own -- rather than
> restore it afterwards and hope no sibling looked in between. A check needing
> a specific descriptor number MUST NOT assume a number it closed stays free.
>
> These fail as flakes attributed to the shell, which is why they are worth a
> rule: the failure appears in a sibling that did nothing wrong, at a rate that
> depends on scheduling, and the natural reading of a rare failure in a
> concurrency test is that the concurrency is at fault.

## What the enforcement does not reach

Two of the five instances sit outside the lint's corpus, so the rule's
coverage reads complete while they are checked by nobody:

* `tests/harness/locale-sweep.sh` and `tests/build-locales.sh` are shell
  scripts, and the `nsh` spec's impl scope is `crates/**/*.rs`. Their fix
  is real and committed, but cannot carry an annotation or be swept.
* `fuzz/` is a cargo workspace of its own and is not read by the lint. A
  fuzz target returns early on every input it finds uninteresting -- an
  `Arbitrary` that did not parse, a byte string with a NUL in it -- which
  is the fuzzing loop rather than a missing reference; sweeping the eleven
  targets reports twenty-five of those against one real defect, and a lint
  needing twenty-five suppressions on its first day does not survive to
  catch the twenty-sixth. What the fuzz side has instead is
  `fuzz/fuzz_targets/support.rs`, where obtaining the oracle panics, so no
  target can reach a comparison without one.

### What no lint here reaches

A check can also fail to measure because its *input* is not what its name
says, and that is not findable in the source text. Three were found by
hand and none of the three codes above would have reported any of them:

* `a_long_flat_list_is_not_deep` built its list with `":\n".repeat(20_000)`,
  and a newline ends a top-level list, so it never built a deep tree. It
  checked that a shallow thing was shallow, under a name saying otherwise,
  for as long as it existed. The 20,000-element `&&` chain it was believed
  to cover was a reachable SIGSEGV the whole time.
* `a_reply_outside_the_menu_empties_the_name` asserted this shell's own
  `select` output as the reference's, on the one stream where the two
  differ.
* the round-trip printer property was a fixed point that any
  self-consistent output satisfies, including output that runs differently
  from its source.

The honest statement is that these were caught by re-measuring the
behaviour for an unrelated reason, and that nothing in the tree looks for
them. A name is prose and an input is code; no lint can compare the two.
What the repository has instead is the standing bar that a check be seen
to fail before it is trusted -- which each of these would have failed on
the day it was written, had anyone asked.

The one real defect in that count was `fuzz/fuzz_targets/differential.rs`,
which the commit that moved the differential targets onto the pinned Bash
did not reach: it kept a private `Command::new("bash")` -- the ambient
5.2, not the 5.3 the repository pins -- and turned a spawn failure into
`None` that the comparison skipped. It now goes through `support.rs` with
the others. It was found by hand while calibrating the lint, which is the
argument for the lint rather than against it, and also the reason this
section exists rather than a claim of full coverage.
