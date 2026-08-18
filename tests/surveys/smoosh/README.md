# Smoosh POSIX shell survey

This directory is the offline, reviewable input boundary for Smoosh's shell
system tests. Smoosh is formal-POSIX-derived evidence, kept distinct from both
the normative POSIX.1-2024 rules under `posix/` and the differential Oils
shell-spec survey under `tests/surveys/oils/`.

`SOURCE.toml` pins the exact upstream commit, tree, license, corpus counts,
timeouts, and legacy known-hang classification. Ordinary verification is
offline: it consumes only the checked-in files below this directory and does
not require OPAM, Smoosh itself, or a checkout under the user's home directory.

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
```

The native harness creates an isolated directory per script, supplies
`TEST_SHELL`, `TEST_SHELL_FLAGS`, `TEST_UTIL`, `HOME`, and `LOGNAME`, compares
only the streams for which upstream supplies an oracle, and defaults a missing
`.ec` oracle to status zero exactly like `tests/shell_tests.sh`. Its helper
executables are safe Rust implementations; no upstream C utility or shell
driver is built or invoked.
