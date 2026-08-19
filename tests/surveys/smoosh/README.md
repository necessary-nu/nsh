# Smoosh shell survey

This directory contains a pinned copy of Smoosh's shell system tests and a
native runner for `nsh`. It is supplemental compatibility data; the normative
POSIX.1-2024 corpus and harness are under `posix/`.

## Files

```text
SOURCE.toml    upstream commit, tree, license, counts, and timeouts
FILES.sha256   hashes for imported files
MANIFEST.toml  generated regular, known-hang, and full groups
LICENSE.txt    imported upstream license
shell/         imported tests and expected output
RESULTS.toml   recorded full-suite result for nsh
```

Verification is offline and does not require OPAM, Smoosh, or an external
checkout. `SOURCE.toml` also pins the Oils revision and `test/smoosh.sh` used
to classify the legacy known-hang cases.

## Running the survey

```sh
cargo build --release --bin nsh
cargo run -p nsh-survey -- run-smoosh
cargo run -p nsh-survey -- run-smoosh --group known-hang
cargo run -p nsh-survey -- run-smoosh --group full --format json
cargo run -p nsh-survey -- run-smoosh --group full --summary results.toml
```

The available groups are:

| Group | Timeout | Contents |
|---|---:|---|
| `regular` | 5 seconds | tests not classified as known hangs |
| `known-hang` | 1 second | retained legacy hang cases |
| `full` | per group | all imported tests |

The default group is `regular`.

The runner creates a working directory for each script and sets `TEST_SHELL`,
`TEST_SHELL_FLAGS`, `TEST_UTIL`, `HOME`, and `LOGNAME`. It compares only streams
with an upstream oracle. A missing `.ec` file means exit status zero, matching
Smoosh's `tests/shell_tests.sh`. Helper programs are native Rust
implementations; the upstream C utilities and shell driver are not run.

`--shell-flag` is repeatable. The `smoosh-shell` wrapper applies the flags to
the top-level script and nested `$TEST_SHELL` calls while preserving parent-PID
semantics. `nsh` needs no extra flag. To survey Bash in POSIX mode, use:

```sh
cargo run -p nsh-survey -- run-smoosh \
  --shell /bin/bash \
  --shell-flag --posix
```

## Containment

Run top-level Cargo and survey commands through `scripts/sandboxed`:

```sh
scripts/sandboxed -- cargo run -p nsh-survey -- run-smoosh --group full
```

The runner verifies its own fail-closed sandbox before executing any script.
Each test gets fresh PID and network namespaces, a read-only root, a writable
scratch directory, a private `/tmp`, a process limit, closed inherited
descriptors, reset signals, and no controlling terminal. Leaked descendants
are killed when the case ends. The survey aborts if containment is unavailable.

## Updating the pin

1. Check out the proposed Smoosh commit in detached state.
2. Review the upstream license, `tests/shell_tests.sh`, `tests/Makefile`,
   `tests/shell/`, and `tests/util/`.
3. Update the identity and reviewed counts in `SOURCE.toml`.
4. Run `cargo run -p nsh-survey -- import-smoosh CHECKOUT`.
5. Review the generated diff.
6. Run `cargo run -p nsh-survey -- verify-smoosh`.

## Recorded result

`RESULTS.toml` contains the deterministic result for the complete `full`
group, including the release binary SHA-256, both timeout classes, totals, and
all non-passing scripts. The current baseline is 155 passes and 31 failures
across 186 scripts, with no timeouts or harness errors.
