# POSIX.1-2024 rule harness

This harness judges a shell against executable expectations derived from the
wording in `posix/docs/spec/`. The checked-in rule corpus is the oracle. A
reference shell is useful for identifying port regressions, but its behavior
never changes an expected POSIX result.

The catalog parser reads all 1,130 rule markers and their complete contiguous
Markdown blockquotes. Every rule receives one current disposition:

- `automatic`: one or more executable cases cite the rule.
- `manual`: the rule currently needs source or human review. Uncovered `def`
  rules begin here because a definition is not itself a runtime assertion.
- `conditional`: `[UP]` or `[XSI]` wording applies only when the corresponding
  implementation option is advertised.
- `not-applicable`: a reviewed override establishes that the rule does not
  apply to this implementation.
- `pending`: no trustworthy verification method has been encoded yet.

Explicit reviewed dispositions live in `dispositions.json`. Unknown rule ids,
duplicate case ids, unsafe fixture paths, and malformed overrides stop the
harness before a shell is executed.

## Run it

Build the Rust port, then run:

```sh
cargo build
python3 posix/harness/run.py --shell target/debug/nsh
```

The default exit status is non-zero if any normative case fails or encounters
a harness error. Use `--report-only` to capture a known-nonconforming baseline
without converting the report into a successful compliance result.

Compare the port with another shell:

```sh
python3 posix/harness/run.py \
  --shell target/debug/nsh \
  --reference /path/to/c-dash
```

The reference column is differential evidence only. Text output is concise by
default; `--verbose` includes passes. Machine-readable output is available with
`--format json`.

Useful focused commands:

```sh
python3 posix/harness/run.py --list
python3 posix/harness/run.py --list --rule builtin.set.opt-u-nounset
python3 posix/harness/run.py --rule builtin.trap.action-executed-as-eval
python3 posix/harness/run.py --case nounset-arithmetic
```

## Add a case

Add a `Case` declaration to `cases.py`:

1. Translate the normative wording into observable setup and expectations.
2. Cite every rule id in the `rules` tuple.
3. Put the exact `[spec:posix:verb:id/test]` annotations immediately above the
   declaration.
4. Avoid asserting unspecified or undefined behavior.
5. Use `requires=("UP",)` or `requires=("XSI",)` for option-conditional
   cases.
6. Prefer exact status and standard output. Match diagnostics by required
   content because POSIX usually leaves their formatting open.

Cases use `mode="command"` by default (`sh -c`). `mode="stdin"` executes a
script through the shell's standard input, and `mode="interactive"` starts
`sh -i` with a real controlling terminal. In interactive mode, set prompts in
`environment` when the transcript needs deterministic prompt text and assert
the merged terminal transcript as stdout; a terminal cannot preserve separate
stdout and stderr streams. `{ROOT}`, `{HOME}`, and `{SHELL}` placeholders are
expanded in environment values and output expectations.

Every case runs in a new temporary directory with a new HOME, C locale, bounded
runtime, captured output, and a `sh` PATH entry pointing back to the shell under
test. Fixtures cannot escape the case directory. A timeout kills the complete
case process group.

Run the harness self-tests with:

```sh
python3 -m unittest discover -s posix/harness/tests -v
```

## A known-flaky differential

`locale-jobs-multibyte-command-text` shows up in `--reference` mode's
`differentials` list under load and not otherwise. It has now been seen
three times, always with **both** shells scoring PASS and the case
totals unchanged, and it disappears on a re-run of an idle machine.

The case backgrounds a job and reads its command text, so whether the
child has been reaped when `jobs` renders it is decided by the
scheduler. Two shells racing the same way is not a divergence.

So: `differentials` of `['edit-history-goto-number',
'edit-history-search-pattern-anchored']` is the expected result. A third
entry naming this case, with `summary.cases` still at FAIL=54 PASS=657,
is the race — re-run before investigating. A third entry that moves the
totals is not.
