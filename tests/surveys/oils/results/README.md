# Oils results for nsh

These files record release `nsh` results for the three Bash-selected Oils
groups. Each TOML summary includes the pinned Oils revision, shell binary hash,
`bash` expectation namespace, containment mode, totals, and non-passing cases.
Timing data is omitted so reruns produce stable diffs.

## Current baseline

Recorded with the release shell installed as `target/bash-mode/bash`; the
basename selects the dialect, so a shell under any other name measures the
profile with the profile turned off.

| Group | Selected | Pass | Fail | Unsupported | Known bug | Timeout | Error |
|---|---:|---:|---:|---:|---:|---:|---:|
| `bash-comparison` | 2,735 | 2,004 | 651 | 40 | 40 | 0 | 0 |
| `bash-extension` | 1,121 | 784 | 298 | 17 | 22 | 0 | 0 |
| `bash-named-diagnostic` | 112 | 93 | 16 | 0 | 3 | 0 | 0 |

These are whole-group numbers. The closure gate judges the `bash-extension`
group after the scope decision and the reference calibration have been
applied; see `../README.md` and `nsh-survey gate-bash`.

## Regenerating

Build the release shell and `nsh-survey`, then run each group through the
top-level sandbox:

```sh
mkdir -p target/bash-mode && cp target/release/nsh target/bash-mode/bash

scripts/sandboxed --writable tests/surveys/oils/results -- \
  target/release/nsh-survey run-oils \
  --group bash-comparison --expect-shell bash \
  --shell target/bash-mode/bash \
  --summary tests/surveys/oils/results/bash-comparison.toml

scripts/sandboxed --writable tests/surveys/oils/results -- \
  target/release/nsh-survey run-oils \
  --group bash-extension --expect-shell bash \
  --shell target/bash-mode/bash \
  --summary tests/surveys/oils/results/bash-extension.toml

scripts/sandboxed --writable tests/surveys/oils/results -- \
  target/release/nsh-survey run-oils \
  --group bash-named-diagnostic --expect-shell bash \
  --shell target/bash-mode/bash \
  --summary tests/surveys/oils/results/bash-named-diagnostic.toml
```

The outer sandbox protects the caller; the survey runner separately contains
each test case.
