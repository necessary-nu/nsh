# Oils results for nsh

These files record release `nsh` results for the three Bash-selected Oils
groups. Each TOML summary includes the pinned Oils revision, shell binary hash,
`bash` expectation namespace, containment mode, totals, and non-passing cases.
Timing data is omitted so reruns produce stable diffs.

## Current baseline

Recorded with the release shell run under the name `bash`; the basename
selects the dialect, so a shell under any other name measures the profile with
the profile turned off. `--expect-shell bash` now makes the runner install its
own copy under that name, so the `shell` field in the older summaries below
names `target/bash-mode/bash`, the fixed path the recipe used to ask for.

| Group | Selected | Pass | Fail | Unsupported | Known bug | Timeout | Error |
|---|---:|---:|---:|---:|---:|---:|---:|
| `bash-comparison` | 2,735 | 2,001 | 654 | 40 | 40 | 0 | 0 |
| `bash-extension` | 1,121 | 784 | 299 | 17 | 21 | 0 | 0 |
| `bash-named-diagnostic` | 112 | 95 | 15 | 0 | 2 | 0 | 0 |

These are whole-group numbers. The closure gate judges the `bash-extension`
group after the scope decision and the reference calibration have been
applied; see `../README.md` and `nsh-survey gate-bash`.

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

There is no `cp` any more, and there must not be one: a fixed path under
`target/` is a shared mutable file in a checkout several sessions use, and
another session's build replaced one between two runs a minute apart on
2026-09-02. `--shell` defaults to `target/release/nsh`, and the runner
installs its own copy of it, named `bash` because `--expect-shell bash` is
what needs that name.

The outer sandbox protects the caller; the survey runner separately contains
each test case.
