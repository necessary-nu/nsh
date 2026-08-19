# POSIX conformance harness

This harness runs executable cases derived from the rules in `../docs/spec/`.
The checked-in rule corpus supplies the expected POSIX behaviour. An optional
reference shell adds differential data, but does not change the expected
result.

## Running the harness

Build `nsh`, then run the full suite:

```sh
cargo build
python3 posix/harness/run.py --shell target/debug/nsh
```

The command exits non-zero when a normative case fails or the harness reports
an error. Use `--report-only` when recording a known-nonconforming baseline.

To compare against another shell:

```sh
python3 posix/harness/run.py \
  --shell target/debug/nsh \
  --reference /path/to/c-dash
```

Reference differences are reported separately. Pass `--verbose` to include
successful cases or `--format json` for machine-readable output.

Useful filters:

```sh
python3 posix/harness/run.py --list
python3 posix/harness/run.py --list --rule builtin.set.opt-u-nounset
python3 posix/harness/run.py --rule builtin.trap.action-executed-as-eval
python3 posix/harness/run.py --case nounset-arithmetic
```

## Rule catalog

The catalog loads all 1,130 rule markers and their complete Markdown
blockquotes. Every rule has one disposition:

- `automatic`: covered by one or more executable cases
- `manual`: requires source or human review; uncovered `def` rules start here
- `conditional`: applies only when the implementation advertises `[UP]` or
  `[XSI]`
- `not-applicable`: excluded by a reviewed implementation-specific override
- `pending`: does not yet have a reliable verification method

Reviewed dispositions are stored in `dispositions.json` and
`dispositions.d/*.json`. Unknown rule ids, duplicate case ids, unsafe fixture
paths, and malformed overrides are fatal errors.

## Adding a case

Add the case to `cases.py` or the appropriate `cases_*.py` module.

1. Turn the normative wording into observable setup and expectations.
2. List every covered rule in the case's `rules` tuple.
3. Put the corresponding `[spec:posix:verb:id/test]` annotations immediately
   above the declaration.
4. Do not assert unspecified or undefined behaviour.
5. Set `requires=("UP",)` or `requires=("XSI",)` for conditional cases.
6. Prefer exact exit status and stdout. Match diagnostics only where POSIX
   specifies their content.

Cases default to `mode="command"`, which invokes the shell with `-c`.
`mode="stdin"` feeds a script on standard input. `mode="interactive"` runs
`sh -i` on a controlling terminal; set deterministic prompts in `environment`
and assert the combined terminal transcript as stdout. A terminal does not
preserve separate stdout and stderr streams.

`{ROOT}`, `{HOME}`, and `{SHELL}` placeholders are expanded in environment
values and expected output. Each case receives a temporary working directory,
a new `HOME`, the C locale, a bounded runtime, captured output, and a `sh` entry
in `PATH` that points to the shell under test. Fixture paths may not escape the
case directory. On timeout, the harness kills the case's process group.

Run the harness tests after changing cases or runner code:

```sh
python3 -m unittest discover -s posix/harness/tests -v
```

## Known flaky reference comparison

The expected `--reference` differentials are:

```text
edit-history-goto-number
edit-history-search-pattern-anchored
```

Under load, `locale-jobs-multibyte-command-text` can also appear even though
both shells pass it. The case observes a background job while it may be reaped,
so the rendered command text is scheduler-dependent. If that case is the only
extra differential and neither side failed, rerun on an idle machine. Treat
changed pass/fail totals as a real result.
