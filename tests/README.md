# Dash differential test harness

This harness compares `nsh` with a pinned build of upstream Dash. The corpus
and runner are kept in this repository; the Dash source is downloaded and
built under `tests/.build/`.

`DASH_REFERENCE.env` pins the release archive, upstream tag and commit,
SHA-256, and two local oracle patches. The resulting reference is Dash
0.5.13.5 built with libedit and `$LINENO` enabled.

## Layout

```text
tests/build-reference.sh   download and build the Dash reference
tests/DASH_REFERENCE.env   source and patch lock
tests/reference/patches/   reference-only fixes
tests/harness/dsdiff.sh    compare one corpus against both shells
tests/harness/runall.sh    run every corpus
tests/harness/ptydiff.py   interactive PTY comparisons
tests/harness/covrun.sh    C reference coverage
tests/corpus/              differential corpora
```

## Running tests

```sh
./tests/build-reference.sh
scripts/sandboxed -- cargo build
scripts/sandboxed --writable tests/.build -- tests/harness/runall.sh 12
tests/harness/dsdiff.sh tests/corpus/everything.txt 12
python3 tests/harness/ptydiff.py
```

`runall.sh` writes details for failed corpora to `tests/.build/fail/`. The
reference archive is cached under `tests/.build/downloads/`, and a matching
build is reused. Downloads and patches are verified before any source is
extracted or run. `build-reference.sh` runs downloaded build code through
`scripts/sandboxed`. Its two patches retain the `fc -e` argument fix and POSIX
interactive background-job announcement.

The harness recognizes these overrides:

- `PORT`: shell under test; defaults to `target/debug/nsh`
- `REF`: reference shell; defaults to `tests/.build/ref/src/dash`
- `DASH_ROOT`: repository root
- `CLASSIFY_ROUNDS`: reruns used to classify output races; defaults to 10

## Containment

Do not run corpus cases directly. Cases may contain commands such as
`kill -- -1` and must be treated as hostile input.

Run top-level test commands through `scripts/sandboxed`. It requires PID and
network namespaces, a read-only root, a process limit, a detached terminal
session, and a startup canary. Only `target/` is writable by default; add
other output directories with `--writable`.

The differential harness adds its own checks:

- Each case runs in a PID namespace and cannot signal host processes.
- `corpus-lint.sh` drops cases containing `kill`, `killall`, or `pkill` unless
  a curated case starts with `#!allow-kill`.
- The root filesystem is read-only outside the case's scratch directory.
- `ds_assert_contained` aborts if containment is unavailable. There is no
  unsandboxed fallback.

`timeout` only limits runtime; it is not a containment boundary.

## Comparison controls

Several checks prevent false agreement or false differences:

- `ds_assert_harness_live` verifies that each binary runs and emits expected
  output before any cases are counted.
- Each shell gets a private `.bin/sh`, so nested `sh` commands use the shell
  being tested rather than the system shell.
- Both shells run from equivalent directories through the same `.bin/sh`
  path, keeping `argv[0]`, `$0`, and `$PWD` comparable.
- Shells start through `env --default-signal` so the caller's signal
  dispositions do not leak into either run.
- `build-reference.sh` enforces the pinned Dash configuration.

After changing the runner, use Bash as a negative control and confirm that it
does not compare cleanly:

```sh
PORT=/bin/bash tests/harness/dsdiff.sh tests/corpus/aud_bltin_test.txt 12
```

## Corpus format

Cases are separated by a line containing exactly `%%%`. A file without a
separator contains one case per line. A case may begin with these directives:

```text
#!name label             label in failure output
#!mode=c | file | stdin  how the script is passed to the shell
#!shargs -i -E           shell flags before -c
#!args a b c             positional arguments
#!norm=pid               normalize runs of three or more digits
#!allow-kill             allow curated self-signalling commands
```

The scratch directories are named `w` and `w2`. Some corpora depend on those
names when normalizing the working directory.

## Flaky output

Concurrent writes can arrive in different orders without indicating a shell
difference. On a mismatch, the harness reruns both shells up to
`CLASSIFY_ROUNDS` times. It reports `FLAKY` when the port produces an output
also observed from the reference; otherwise it reports `FAIL` and includes the
number of distinct outputs from each side.

`ptydiff.py` uses the same approach for prompt-drain timing. PID-based output
is not a useful flakiness probe because `$$` is deterministic inside the PID
namespace.

## Coverage and expected noise

`covrun.sh` measures C function coverage, but its totals are only a lower
bound: gcov loses the parent's counters when a case forks and execs. Correct
aggregation will require per-process `GCOV_PREFIX` data and `gcov-tool merge`.

Some fuzz corpora crash the Dash reference. Contained Dash segfaults in
`dmesg` are expected during these runs and do not indicate host instability.
