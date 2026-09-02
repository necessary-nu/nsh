# Oils shell-spec survey

This directory contains a pinned, offline copy of the Oils shell-spec corpus
and a native runner for `nsh`. Oils provides compatibility data; normative
POSIX conformance is tested under `posix/`.

The imported corpus is pinned to Oils commit
`15de8fd779569e6e3a9f5fcbfc00e7df0ebe0380`: 222 spec files and 3,964 cases in
total, including 134 active OSH files and 2,755 cases.

## Files

```text
SOURCE.toml                 upstream commit, tree, counts, and import settings
FILES.sha256                hashes for imported files
FIXTURES.txt                non-case files required by active specs
MANIFEST.toml               generated survey groups and case counts
spec/                       imported specs and their fixture data
BASH_REFERENCE.toml         pinned GNU Bash 5.3 reference profile
BASH_REFERENCE_CASES.json   Bash case eligibility and exclusions
BASH_DISPOSITIONS.toml      why each eligible case nsh does not pass is expected
BASH_COMPARISON_FAILURES.toml  which bash-comparison cases nsh fails, by id
results/                    recorded nsh results for Bash-selected groups
```

Normal builds and tests use only these checked-in files. They do not fetch
Oils or depend on a checkout elsewhere on the machine.

## Verify the import

```sh
scripts/sandboxed --writable tests/surveys/oils -- \
  target/release/nsh-survey generate-oils-manifests
scripts/sandboxed -- target/release/nsh-survey verify-oils
```

`MANIFEST.toml` is generated from the imported metadata and `FILES.sha256`;
do not edit it by hand.

## Survey groups

| Group | Files | Cases | Selection |
|---|---:|---:|---|
| `full` | 134 | 2,755 | all active OSH specs |
| `posix-candidate` | 76 | 1,614 | `compare_shells` includes Dash |
| `bash-comparison` | 130 | 2,735 | `compare_shells` includes a Bash token |
| `bash-extension` | 54 | 1,121 | Bash-selected, but not Dash-selected |
| `bash-named-diagnostic` | 7 | 112 | `*-bash` spec files |

The Dash-selected group is useful for differential POSIX work, but individual
cases are not necessarily normative POSIX requirements.

## Running the survey

```sh
scripts/sandboxed -- cargo build --release -p nsh-cli --bin nsh
scripts/sandboxed -- cargo build --release -p nsh-survey
scripts/sandboxed -- target/release/nsh-survey run-oils --group posix-candidate
scripts/sandboxed -- target/release/nsh-survey run-oils \
  --group bash-extension --format json
scripts/sandboxed --writable tests/.build -- target/release/nsh-survey run-oils \
  --group bash-extension \
  --expect-shell bash \
  --summary tests/.build/bash-extension.toml
```

The default shell is `target/release/nsh`, the default group is `full`, and
the default expectation namespace is `osh`. `--expect-shell` selects another
per-shell expectation namespace.

Qualified `OK` results count as passes. Matching `N-I` results are unsupported,
and matching `BUG` results are reported as known bugs. A byte or status
mismatch is a failure. The runner exits non-zero for failures, timeouts, and
runner errors.

Use `--spec NAME`, `--case TEXT`, and `--max-cases N` for focused runs.
`--timeout-ms` sets the per-case timeout. Excluded cases are reported as
skipped.

The runner supplies native Rust replacements for imported Python 2 helpers.
The small `python2 -c` subset used by the corpus is handled directly; compatible
larger snippets use the repository's Python 3 test dependency.

## Containment

The native runner also isolates every case. Before starting, it verifies a
fail-closed sandbox canary. Each case gets fresh PID and network namespaces, a
read-only root, a private `/tmp`, a writable scratch directory, a process
limit, closed inherited descriptors, reset signals, bounded output, and no
controlling terminal. Leaked descendants are killed on exit or timeout. The
survey aborts if containment cannot be established.

## GNU Bash 5.3 reference

`BASH_REFERENCE.toml` pins GNU Bash 5.3 patch level 15, source and patch
digests, compiler and libc details, build flags, runtime settings, the Oils
revision, the reference binary hash, and disposition totals.
`BASH_REFERENCE_CASES.json` records the eligible and explicitly excluded
`bash-comparison` cases. The extension and named-diagnostic groups are checked
against the same observations.

Put `bash-5.3.tar.gz` and `bash53-001` through `bash53-015` in a local source
cache, then run:

```sh
scripts/sandboxed --writable /path/to/source-cache -- \
  cargo run -p nsh-survey -- \
  build-bash-reference /path/to/source-cache target/bash-reference

scripts/sandboxed \
  --writable /path/to/source-cache \
  --writable tests/surveys/oils -- \
  cargo run -p nsh-survey -- calibrate-bash-reference \
  --shell target/bash-reference/bash-5.3/bash \
  --sources /path/to/source-cache

scripts/sandboxed -- cargo run -p nsh-survey -- verify-bash-reference
```

The builder verifies all digests, applies patches in order, requires an empty
output directory, and writes a build receipt. Ordinary verification is
offline; `--sources` and `--shell` additionally verify local source and binary
artifacts. Calibration verifies the imported corpus and does not modify its
expectations.

The 2026-08-19 calibration used GNU Bash `5.3.15(1)-release`. Of 2,735
comparison cases, 2,447 are eligible and 288 are excluded: 108 reference
failures, 52 unsupported expectations, 94 known upstream bugs, and 34
expectations for another Bash version. There were no timeouts or runner errors.
Two clean builds produced SHA-256
`5aca12bd46aaef0d8183df3d9ba1de80cd36d2d52f179ec448d3b007a297d173`.

## The Bash closure gate

`[spec:nsh:req:compat.bash.survey-closure]` asks for zero *unexpected*
failures. `BASH_DISPOSITIONS.toml` is what makes "unexpected" decidable: one
entry per eligible `bash-extension` case that `nsh` does not pass, with a
category and a reason, plus the whole files that
`[dec:nsh:bash-compatibility-is-scripts]` places outside the contract.

```sh
scripts/sandboxed -- cargo build --release --bin nsh --bin nsh-survey
mkdir -p target/bash-mode && cp target/release/nsh target/bash-mode/bash
scripts/sandboxed -- target/release/nsh-survey gate-bash \
  --shell target/bash-mode/bash
```

The basename must be exactly `bash`: `argv[0]` selects the dialect, so a name
like `bash-pre` silently measures the profile with the profile turned off, and
the gate refuses to run under one.

Categories:

| Category | Meaning |
|---|---|
| `not-implemented` | Nobody has built it. The honest backlog. |
| `defect` | A bug here, with a known reproduction. |
| `sanctioned-divergence` | Bash's behaviour is refused on purpose; see `docs/divergences.md`. |
| `harness-artifact` | Cannot pass for any shell under this runner. |
| `out-of-contract` | Outside the profile per the scope decision. |

Bash's own defects are not in this file: the corpus qualifies them with
`## BUG bash` and the runner reports them as `known-bug`, and every case the
pinned Bash 5.3 build does not itself pass already carries a disposition in
`BASH_REFERENCE_CASES.json`.

The gate is symmetric. A registered case that starts passing fails it, exactly
as an unregistered case that stops passing does: a stale excuse is how a real
regression eventually gets waved through. When a fix lands, delete the entry.

## Comparing a run against the failing-case list

The summary's `fail=` count is not a check. It cannot tell a case that was
fixed from one that broke, and two `process-sub` cases move between runs under
load -- one of them toward a *pass* -- so the number drifts for reasons no
register describes. A commit once credited two fixes that were not real
because it read the count.

`BASH_COMPARISON_FAILURES.toml` is the list the count cannot be: every
`bash-comparison` case `nsh` fails, by `spec:index` id. Compare a run against
it in one command:

```sh
scripts/sandboxed -- target/release/nsh-survey run-oils \
  --shell target/gate/bash --expect-shell bash --group bash-comparison \
  --baseline tests/surveys/oils/BASH_COMPARISON_FAILURES.toml
```

It names every id on either side of a difference and exits non-zero when there
is one; with `--baseline` the run's verdict is the comparison rather than the
score, since a group with hundreds of expected failures is never otherwise a
check. The verdict goes to stderr so it cannot corrupt `--format json` or
`--format ids` on stdout.

Add `--update-baseline` to re-record the file. It prints the difference it is
about to enshrine before writing, so a refresh is never silent.

A refresh also has to be able to say *what* it measured. `target/release/nsh`
is built from the working tree, so in a checkout several sessions share it is
everybody's work at once, and a refresh taken over it records changes no commit
explains. `ee98cec` removed two ids from this list and attributed them to a
commit that did not remove them, for exactly that reason. So `--update-baseline`
refuses a checkout carrying uncommitted work and names every path that made it
so, before the run rather than after it:

```text
nsh-survey: refusing to re-record tests/surveys/oils/BASH_COMPARISON_FAILURES.toml
from a checkout with 18 uncommitted path(s):
  crates/nsh/src/pattern.rs
  ...
```

Commit them and run again, or -- when the change being measured is necessarily
still uncommitted, which is the ordinary case -- pass
`--update-baseline-from-dirty-tree`. That records the list and writes
`nsh_commit`, `shell_sha256` and every uncommitted path into the file, so a
later reader can see which files' effects are in it. `plan/` is the one
exception: nplan rewrites it on every `nplan start` and nothing under it reaches
a shell or a survey run.

Those three keys are the opposite of a pin. They are never compared against
anything, and every comparison prints the sentence they make:

```text
recorded at 848e0ab over 19 uncommitted path(s), whose effects are in this
list and in no commit: crates/nsh/src/pattern.rs, ...
```

`--format ids` prints just the failing ids, one per line, sorted. Do not
recover them from the text report with a pattern: the one used before this
existed was `[a-z0-9-]`, which has no underscore, so `case_.test.sh:1`,
`command_.test.sh:3` and `command_.test.sh:12` sat outside every comparison
made with it and a regression in either spec would have passed in silence.

A baseline records the group, the expectation namespace, POSIX mode, the
per-case timeout and the Oils commit, and a run that differs in any of them is
refused rather than compared. `--spec`, `--case` and `--max-cases` are refused
with `--baseline` outright: a filtered run would report every unselected case
as fixed.

## Updating the Oils pin

1. Check out the proposed commit from the repository named in `SOURCE.toml`.
   Use a detached commit, not a moving branch.
2. Verify `git rev-parse HEAD` and `git rev-parse HEAD^{tree}` against the new
   lock values.
3. Review `LICENSE.txt`, `spec/README.md`, `test/sh_spec.py`,
   `test/spec-runner.sh`, and `test/spec-compat.sh` upstream.
4. Run `scripts/sandboxed --writable CHECKOUT --writable tests/surveys/oils -- target/release/nsh-survey import-oils CHECKOUT`. The importer must
   reject a commit or tree that does not match the lock.
5. Review the generated corpus, fixtures, hashes, manifests, and count changes.
6. Run the importer tests, negative controls, and every manifest group.

Review any change to the commit, tree, license, selected files, or counts as a
source update.

## Recorded results

`results/` contains deterministic summaries for the three Bash-selected
groups, run against the `bash` expectation namespace. The summaries retain the
source identity, release binary hash, totals, and non-passing cases while
omitting timings. See `results/README.md` for regeneration commands and the
current baseline.
