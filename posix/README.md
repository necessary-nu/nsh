# POSIX specification corpus

This directory contains the shell-related parts of POSIX.1-2024 (IEEE Std
1003.1-2024, The Open Group Base Specifications Issue 8), the conversion
tools, and the resulting nspec rule corpus.

The current corpus has 1,130 rules in 23 files: 821 `req`, 113 `def`, 106
`syn`, 89 `sem`, and 1 `thm`.

## Layout

```text
utilities/, basedefs/, functions/, ...  vendored Open Group HTML
tools/                                   conversion and validation tools
build/md/                                converted Markdown
build/units/                             Markdown split into authoring units
build/AUTHORING.md                       corpus authoring rules
docs/spec/                               nspec corpus
harness/                                 executable conformance cases
.config/nspec/config.styx                nspec configuration
```

## Scope

The corpus covers the shell portions of XCU:

| Source | Corpus | Rules | Prefix |
|---|---|--:|---|
| XCU 2.1–2.2 Shell Introduction, Quoting | `quoting.md` | 38 | `shell.` `quote.` |
| XCU 2.3–2.4 Token Recognition, Reserved Words | `tokens.md` | 26 | `token.` |
| XCU 2.5 Parameters and Variables | `parameters.md` | 45 | `param.` |
| XCU 2.6 Word Expansions | `expansion.md` | 67 | `expand.` |
| XCU 2.7 Redirection | `redirection.md` | 34 | `redir.` |
| XCU 2.8 Exit Status and Errors | `exit-status.md` | 10 | `exit.` |
| XCU 2.9 Shell Commands | `commands.md` | 111 | `cmd.` |
| XCU 2.10 Shell Grammar | `grammar.md` | 35 | `grammar.` |
| XCU 2.11–2.13 Job Control, Signals, Execution Environment | `execution.md` | 31 | `jobctl.` `signal.` `shenv.` |
| XCU 2.14 Pattern Matching Notation | `pattern-matching.md` | 30 | `pattern.` |
| XCU 2.15 Special Built-Ins (`break` … `exit`) | `builtins-control.md` | 55 | `builtin.` |
| XCU 2.15 Special Built-Ins (`export` … `unset`) | `builtins-variables.md` | 50 | `builtin.` |
| XCU 2.15 `set` and `trap` | `builtins-set-trap.md` | 69 | `builtin.` |
| `sh` reference page | `invocation.md` | 49 | `sh.` |
| `sh` command history and line editing | `line-editing.md` | 55 | `edit.` |
| XCU 1.1 Relationship to Other Documents | `relationship.md` | 28 | `xcurel.` |
| XCU 1.2–1.7 limits, defaults, built-ins | `utility-defaults.md` | 88 | `xcu.` |
| Intrinsic `command`, `type`, `hash` | `builtins-command.md` | 44 | `builtin.` |
| Intrinsic `cd`, `umask`, `ulimit` | `builtins-process.md` | 88 | `builtin.` |
| Intrinsic `bg`, `fg`, `jobs` | `builtins-jobs.md` | 41 | `builtin.` |
| Intrinsic `kill`, `wait` | `builtins-signals.md` | 34 | `builtin.` |
| Intrinsic `alias`, `unalias`, `fc` | `builtins-alias.md` | 49 | `builtin.` |
| Intrinsic `read`, `getopts` | `builtins-input.md` | 53 | `builtin.` |

Each file owns its rule-id prefixes. nspec keys coverage by bare rule id, so
reusing an id in another file would merge two rules.

### Shell boundary

XCU 1.6 and 1.7 define which utilities a conforming shell must provide
internally: the XCU 2.15 special built-ins and the intrinsic utilities
`alias`, `bg`, `cd`, `command`, `fc`, `fg`, `getopts`, `hash`, `jobs`, `kill`,
`read`, `type`, `ulimit`, `umask`, `unalias`, and `wait`.

Other XCU utilities may be separate executables and are not part of this
corpus. This includes `awk`, `sed`, `find`, `pwd`, `test`, `echo`, and
`printf`, even when a shell provides built-in versions. The conversion tools
can still process their source pages.

## Regenerating Markdown

Run from `posix/`:

```sh
tools/convert.sh utilities/V3_chap02.html utilities/sh.html
```

`tools/strip-boilerplate.py` removes site navigation and presentation markup.
It also restores headings for the XCU 2.15 built-ins and preserves command
synopses before pandoc converts the document.

`tools/posix.lua` handles document-level changes: section anchors,
option-conditional markers, informative-section markers, definition lists,
and blockquotes used only for indentation. The generated Markdown is checked
against the source's normalized `shall` statements.

## Validation

```sh
tools/check-nspec.py docs/spec
tools/coverage-report.py
nplan spec status
```

`check-nspec.py` validates marker syntax, duplicate ids, rule bodies,
citations, and markers inside fenced code blocks. This is necessary because a
malformed marker is otherwise ignored by nspec.

`coverage-report.py` checks that normative source sentences survive in the
corpus. It currently reports 99%; the remainder consists of section lead-ins
covered by their child rules and non-binding future-version notes.

## Editing rules

The full authoring contract is in `build/AUTHORING.md`. In summary:

- Use namespace `posix` and an id owned by the file's prefix.
- Use `def` for definitions, `syn` for grammar, `sem` for behaviour described
  without a normative keyword, and `req` for obligations.
- Preserve the standard's wording, including `shall`.
- Preserve `[UP]`, `[XSI]`, and `[OB]` option markers at their original scope.
- Exclude informative sections such as APPLICATION USAGE, EXAMPLES, RATIONALE,
  FUTURE DIRECTIONS, SEE ALSO, and CHANGE HISTORY.
- Include the source section and anchor for every rule.

Two parser details affect hand edits:

- A `> [spec:...]` marker inside a fenced block is still parsed as a rule.
- Leading whitespace is removed from rule body lines, so indentation-sensitive
  text does not have the same parsed form as the Markdown source.

## Implementation annotations

Annotate implementation and test sites with the corresponding rule:

```rust
// [spec:posix:req:quote.backslash]
fn unescape(...) { ... }

// [spec:posix:req:quote.backslash/test]
#[test]
fn backslash_preserves_literal_value() { ... }
```

Use `nplan spec status` for coverage, `nplan spec uncovered` for rules without
an implementation annotation, and `nplan unplanned` for rules not claimed by
a plan node.

## License

`docs/spec/` reproduces normative text from IEEE Std 1003.1-2024, Copyright ©
2001–2024 The IEEE and The Open Group. The vendored HTML carries the same
notice. The tools under `tools/` are original to this repository.
