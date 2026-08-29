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

A `return None` inside a closure is not an exit that reports success, and
is not reported.

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

The one real defect in that count was `fuzz/fuzz_targets/differential.rs`,
which the commit that moved the differential targets onto the pinned Bash
did not reach: it kept a private `Command::new("bash")` -- the ambient
5.2, not the 5.3 the repository pins -- and turned a spawn failure into
`None` that the comparison skipped. It now goes through `support.rs` with
the others. It was found by hand while calibrating the lint, which is the
argument for the lint rather than against it, and also the reason this
section exists rather than a claim of full coverage.
