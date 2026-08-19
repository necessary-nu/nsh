# Smoosh POSIX shell survey

This directory is the offline, reviewable input boundary for Smoosh's shell
system tests. Smoosh is formal-POSIX-derived evidence, kept distinct from both
the normative POSIX.1-2024 rules under `posix/` and the differential Oils
shell-spec survey under `tests/surveys/oils/`.

`SOURCE.toml` pins the exact upstream commit, tree, license, corpus counts,
timeouts, and legacy known-hang classification. Ordinary verification is
offline: it consumes only the checked-in files below this directory and does
not require OPAM, Smoosh itself, or a checkout under the user's home directory.
The origin of the known-hang classification is pinned separately to the exact
Oils revision and `test/smoosh.sh` path that defined it.

The generated `MANIFEST.toml` exposes three selections: `regular`,
`known-hang`, and `full`. The known-hang cases are never discarded; they run
with the separately locked one-second deadline. All other cases retain
Smoosh's five-second default.

## Updating the pin

1. Fetch the proposed commit into a disposable Smoosh checkout and detach at
   that exact commit.
2. Review the upstream license, `tests/shell_tests.sh`, `tests/Makefile`,
   `tests/shell/`, and `tests/util/`.
3. Update the source identity and reviewed counts in `SOURCE.toml`.
4. Run `cargo run -p nsh-survey -- import-smoosh CHECKOUT`.
5. Review the complete generated diff and run
   `cargo run -p nsh-survey -- verify-smoosh`.

## Running the survey

Build the release shell, then run the native harness:

```text
cargo build --release --bin nsh
cargo run -p nsh-survey -- run-smoosh
cargo run -p nsh-survey -- run-smoosh --group known-hang
cargo run -p nsh-survey -- run-smoosh --group full --format json
cargo run -p nsh-survey -- run-smoosh --group full --summary results.toml
```

The native harness creates an isolated directory per script, supplies
`TEST_SHELL`, `TEST_SHELL_FLAGS`, `TEST_UTIL`, `HOME`, and `LOGNAME`, compares
only the streams for which upstream supplies an oracle, and defaults a missing
`.ec` oracle to status zero exactly like `tests/shell_tests.sh`. Its helper
executables are safe Rust implementations; no upstream C utility or shell
driver is built or invoked.

Before any script runs, the harness proves its fail-closed sandbox canary.
Every script runs with fresh PID and network namespaces, a read-only root with
only its scratch tree writable, a private `/tmp`, a bounded process limit,
closed inherited descriptors, reset signal dispositions, and no controlling
terminal. The namespace reaper kills leaked descendants. Missing or defective
containment aborts the survey with no unsandboxed fallback, and result files
record the active containment mode. Use `scripts/sandboxed -- COMMAND` around
the top-level Cargo or regression-test command too, so harness regressions are
contained before the per-script boundary is established.

`--shell-flag` is repeatable. The native `smoosh-shell` wrapper process-replaces
itself with the selected shell, ensuring those flags apply to the top-level
script and every nested `$TEST_SHELL` invocation without changing parent-PID
semantics. `nsh` needs no extra flag because its baseline language is POSIX;
for example, use `--shell /bin/bash --shell-flag --posix` when surveying Bash.

## Recorded result

`RESULTS.toml` is a deterministic summary from the complete `full` group. It
records the tested release binary's SHA-256, both timeout classes, totals, and
every non-passing script without conflating those observations with normative
POSIX.1-2024 conformance. The initial nsh result is 155 pass and 31 fail, with
zero timeouts and zero harness errors across all 186 scripts.
