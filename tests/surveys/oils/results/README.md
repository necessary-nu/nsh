# Oils results for nsh

These files record release `nsh` results for the three Bash-selected Oils
groups. Each TOML summary includes the pinned Oils revision, shell binary hash,
`bash` expectation namespace, containment mode, totals, and non-passing cases.
Timing data is omitted so reruns produce stable diffs.

## Current baseline

Shell SHA-256:
`b8289d9e6f0ec3ecbab6f568ede78ecc36bdeeb25adedab19281be31fed910c2`

| Group | Selected | Pass | Fail | Unsupported | Known bug | Timeout | Error |
|---|---:|---:|---:|---:|---:|---:|---:|
| `bash-comparison` | 2,735 | 1,100 | 1,596 | 26 | 13 | 0 | 0 |
| `bash-extension` | 1,121 | 74 | 1,039 | 6 | 2 | 0 | 0 |
| `bash-named-diagnostic` | 112 | 4 | 107 | 0 | 1 | 0 | 0 |

## Regenerating

Build the release shell and `nsh-survey`, then run each group through the
top-level sandbox:

```sh
scripts/sandboxed --writable tests/surveys/oils/results -- \
  target/release/nsh-survey run-oils \
  --group bash-comparison --expect-shell bash \
  --summary tests/surveys/oils/results/bash-comparison.toml

scripts/sandboxed --writable tests/surveys/oils/results -- \
  target/release/nsh-survey run-oils \
  --group bash-extension --expect-shell bash \
  --summary tests/surveys/oils/results/bash-extension.toml

scripts/sandboxed --writable tests/surveys/oils/results -- \
  target/release/nsh-survey run-oils \
  --group bash-named-diagnostic --expect-shell bash \
  --summary tests/surveys/oils/results/bash-named-diagnostic.toml
```

The outer sandbox protects the caller; the survey runner separately contains
each test case.
