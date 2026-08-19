# Oils results for nsh

These files record release `nsh` results for the three Bash-selected Oils
groups. Each TOML summary includes the pinned Oils revision, shell binary hash,
`bash` expectation namespace, containment mode, totals, and non-passing cases.
Timing data is omitted so reruns produce stable diffs.

## Current baseline

Shell SHA-256:
`f72ebfa60ed098d4a0849ae8448f09e4a95e415a99f84a2d14cf8353d2acdf4a`

| Group | Selected | Pass | Fail | Unsupported | Known bug | Timeout | Error |
|---|---:|---:|---:|---:|---:|---:|---:|
| `bash-comparison` | 2,735 | 1,086 | 1,613 | 26 | 10 | 0 | 0 |
| `bash-extension` | 1,121 | 70 | 1,043 | 6 | 2 | 0 | 0 |
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
