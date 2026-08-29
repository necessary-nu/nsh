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
