# Recorded nsh results against Bash expectations

These TOML files are deterministic summaries of release `nsh` against the
three Bash-selected Oils groups. Each records the pinned Oils commit, release
binary hash, explicit `bash` expectation namespace, containment mode, totals,
and every non-passing case. Wall-clock and per-case timings are intentionally
excluded.

Regenerate all three with the native runner after building release binaries:

```text
target/release/nsh-survey run-oils --group bash-comparison --expect-shell bash --summary tests/surveys/oils/results/bash-comparison.toml
target/release/nsh-survey run-oils --group bash-extension --expect-shell bash --summary tests/surveys/oils/results/bash-extension.toml
target/release/nsh-survey run-oils --group bash-named-diagnostic --expect-shell bash --summary tests/surveys/oils/results/bash-named-diagnostic.toml
```

Run those commands through `scripts/sandboxed --writable
tests/surveys/oils/results -- COMMAND` so both the top-level process and every
individual case remain contained.

The baseline at release-shell SHA-256
`f72ebfa60ed098d4a0849ae8448f09e4a95e415a99f84a2d14cf8353d2acdf4a`
is:

| Group | Selected | Pass | Fail | Unsupported | Known bug | Timeout | Error |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `bash-comparison` | 2,735 | 1,086 | 1,613 | 26 | 10 | 0 | 0 |
| `bash-extension` | 1,121 | 70 | 1,043 | 6 | 2 | 0 | 0 |
| `bash-named-diagnostic` | 112 | 4 | 107 | 0 | 1 | 0 | 0 |
