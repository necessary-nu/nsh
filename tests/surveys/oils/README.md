# Oils shell-spec survey

This directory is the offline, reviewable input boundary for the Oils
shell-spec survey. Oils is a differential compatibility survey; it is not the
normative POSIX oracle. The normative rules and their harness remain under
`posix/`.

`SOURCE.toml` identifies the exact upstream commit and tree. Ordinary builds
and test runs MUST consume files committed below this directory and MUST NOT
fetch, clone, or otherwise depend on the network. A changed commit, tree,
license, file selection, or case count is a reviewed source update rather than
an incidental test result.

## Updating the pin

1. Fetch the proposed commit into a disposable checkout of the repository in
   `SOURCE.toml` and detach at that commit. Never update from a moving branch.
2. Verify that `git rev-parse HEAD` and `git rev-parse HEAD^{tree}` equal the
   proposed lock values.
3. Review `LICENSE.txt`, `spec/README.md`, `test/sh_spec.py`,
   `test/spec-runner.sh`, and `test/spec-compat.sh` for license, format,
   selection, and execution changes.
4. Run the repository importer against that local checkout. The importer must
   regenerate the corpus, fixtures, hashes, and manifests without network
   access and must reject a checkout whose commit or tree differs from the
   lock.
5. Review the complete generated diff. Count changes are expected to fail the
   verification command until the values in `SOURCE.toml` and the generated
   manifests are consciously updated together.
6. Run the survey runner's parser tests, negative controls, and every imported
   manifest before committing the new pin.

The initial pin is Oils commit
`15de8fd779569e6e3a9f5fcbfc00e7df0ebe0380`, whose metadata reports 222 total
spec files with 3,964 cases and 134 active OSH files with 2,755 cases.

## Generated survey manifest

`MANIFEST.toml` is generated from the imported file metadata and
`FILES.sha256`; do not edit it by hand. Regenerate and verify it with:

```text
cargo run -p nsh-survey -- generate-oils-manifests
cargo run -p nsh-survey -- verify-oils
```

The manifest exposes five stable selections:

- `full`: all 134 active OSH files and 2,755 cases;
- `posix-candidate`: the 76 files and 1,614 cases whose `compare_shells`
  metadata contains Dash;
- `bash-comparison`: the 130 files and 2,735 cases whose metadata contains a
  Bash token;
- `bash-extension`: the 54 Bash-selected files and 1,121 cases not selected
  for Dash; and
- `bash-named-diagnostic`: the seven `*-bash` files and 112 cases.

The Dash-selected set is a differential POSIX candidate survey, not a claim
that each case is normative POSIX. Normative conformance remains under
`posix/`. Each manifest entry records its complete source hash and qualified
assertion count; the native runner reads the exact per-case shell qualifiers
from those hashed source files.

## Native runner

Build the release shell, then run any manifest selection without Python 2 or
the upstream Bash harness:

```text
cargo build --release --bin nsh
cargo run -p nsh-survey -- run-oils --group posix-candidate
cargo run -p nsh-survey -- run-oils --group bash-extension --format json
cargo run -p nsh-survey -- run-oils --group bash-extension --expect-shell bash --summary results.toml
```

The default target is `target/release/nsh`, the default group is `full`, and
the default expectation namespace is `osh`. Use `--expect-shell` to select a
different per-shell qualifier namespace. Qualified `OK` results count as
passes, matching `N-I` results count as unsupported, matching `BUG` results
are reported separately as known bugs, and any byte or status mismatch is a
failure. The command exits nonzero for failures, timeouts, or runner errors.

`--spec NAME`, `--case TEXT`, and `--max-cases N` provide reproducible focused
runs; excluded cases are counted as skipped. `--timeout-ms` bounds every case.
Before any case runs, the native runner proves a fail-closed sandbox canary.
Every execution then receives a fresh writable directory inside new PID and
network namespaces, a read-only root, a private `/tmp`, a bounded process
limit, closed inherited descriptors, a cleared deterministic environment,
reset signal dispositions, bounded stdout and stderr capture, and no
controlling terminal. The namespace reaper kills leaked descendants on normal
exit and timeout. The runner refuses to execute cases if the sandbox is absent
or any containment check fails; reports record the active containment mode.

Use `scripts/sandboxed -- COMMAND` for the top-level Cargo or regression-test
command as well. That outer boundary protects the caller if harness code
regresses before it reaches the runner's per-case sandbox.

## Pinned GNU Bash 5.3 reference

`BASH_REFERENCE.toml` is the machine-readable reference profile. It pins GNU
Bash 5.3 patch level 15, the base archive and all 15 patch digests, the exact
compiler and libc environment, build flags, the contained runtime environment,
the Oils revision, the reference binary digest, and disposition totals.
`BASH_REFERENCE_CASES.json` is the immutable case-level evidence: every case
in `bash-comparison` is either named in the eligible list because the reference
passed it or has one explicit exclusion disposition. The extension and named
diagnostic selections are independently executed and checked against the same
case observations.

Place the official `bash-5.3.tar.gz` and `bash53-001` through `bash53-015`
files in a local source cache. The native builder verifies every digest before
extracting or executing anything, refuses a non-empty output directory, applies
the patches in order, and emits a build receipt. Run the complete workflow
through the fail-closed outer sandbox:

```text
scripts/sandboxed --writable /path/to/source-cache -- cargo run -p nsh-survey -- build-bash-reference /path/to/source-cache target/bash-reference
scripts/sandboxed --writable /path/to/source-cache --writable tests/surveys/oils -- cargo run -p nsh-survey -- calibrate-bash-reference --shell target/bash-reference/bash-5.3/bash --sources /path/to/source-cache
scripts/sandboxed -- cargo run -p nsh-survey -- verify-bash-reference
```

Ordinary verification is offline and does not require the GNU source cache or
binary. Supplying `--sources` and `--shell` additionally proves that local
artifacts match the profile. Calibration verifies the imported corpus before
running it and never edits imported expectations.

The 2026-08-19 calibration executed all three Bash selections against
`5.3.15(1)-release`. Of the 2,735 comparison cases, 2,447 are eligible and 288
are explicitly excluded: 108 reference failures, 52 unsupported expectations,
94 known upstream bugs, and 34 expectations scoped to another Bash version.
No case timed out or produced a harness error. Two clean builds in distinct
output directories produced the recorded binary SHA-256
`5aca12bd46aaef0d8183df3d9ba1de80cd36d2d52f179ec448d3b007a297d173`.

## Recorded nsh baseline against Bash expectations

`results/` contains deterministic release-shell summaries for
`bash-comparison`, `bash-extension`, and `bash-named-diagnostic`, each run with
`--expect-shell bash`. These are Bash compatibility observations, not claims
of normative POSIX conformance. The summaries omit timings while retaining the
pinned source identity, release binary hash, complete totals, and every
non-passing case.

The runner mounts a disposable fixture view over the byte-pinned corpus.
Native Rust entry points replace the imported Python 2 helpers, including the
small `python2 -c` surface used by the corpus; compatible complex snippets are
forwarded to the repository's existing Python 3 test dependency. No user-home
checkout, network access, Python 2 interpreter, or Bash orchestration is used.
