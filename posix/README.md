# POSIX shell specification → nspec corpus

This turns the shell portion of the POSIX.1-2024 standard (IEEE Std 1003.1-2024,
The Open Group Base Specifications Issue 8) into an [nspec](https://github.com/)
rule corpus that an implementation can be tracked against with `nplan`.

The vendored HTML of the standard is the input; `docs/spec/` is the output.

**705 rules across 15 files**, covering XCU chapter 2 (Shell Command Language,
including the fifteen special built-ins) and the `sh` reference page — 497 `req`,
84 `syn`, 68 `def`, 56 `sem`.

## What's here

```
utilities/, basedefs/, functions/, …   the vendored Open Group HTML (untouched)
tools/                                 the conversion + validation pipeline
build/md/                              HTML converted to clean GFM
build/units/                           that markdown sliced into authoring units
build/AUTHORING.md                     the contract the corpus was authored against
docs/spec/                             the nspec rule corpus
.config/nspec/config.styx              nspec configuration
```

## Scope

The **shell** portion of XCU:

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

Each file owns an exclusive id prefix. That matters more than it looks: nspec
keys coverage on the **bare rule id**, dropping namespace and verb, so two files
using the same id would silently merge into one rule.

Not covered: the regular built-ins that live in their own utility pages (`cd`,
`read`, `command`, `getopts`, …) and the rest of the XCU utilities. The pipeline
handles them unchanged — `tools/convert.sh utilities/cd.html` — they just
haven't been authored into rules.

## Regenerating the markdown

```sh
tools/convert.sh utilities/V3_chap02.html utilities/sh.html
```

`tools/strip-boilerplate.py` removes Open Group page furniture (navigation
tables, publisher banners, the option-code popup script, font hacks) and fixes
two places where the source encodes structure in something pandoc discards:

- The special built-ins in XCU 2.15 have **no heading of their own** — each is
  introduced only by an anchor pair and an HTML comment naming it. Without
  promoting that to a real heading, all fifteen man pages run together as an
  undifferentiated wall of `NAME` / `SYNOPSIS` / `DESCRIPTION` sections.
- `<blockquote class="synopsis">` is a command synopsis, not a quotation.
  pandoc's `BlockQuote` carries no attributes, so it becomes `<pre>` first.

`tools/posix.lua` then handles what needs the document tree: hoisting each
`tag_…` section anchor onto its heading (emitted as a raw anchor, since GFM has
no heading attributes), converting the option-shading images that bracket
option-conditional text into `[Option Start]` / `[Option End]` markers, turning
the informative-section boxes into `<!-- INFORMATIVE-START -->` markers,
rewriting definition lists into bullet lists (GFM has none, and pandoc's
fallback loses the term/definition association), and unwrapping the blockquotes
the man sections use purely for indentation.

Content is preserved: the converted markdown carries the same normative
sentences as the source, verified by diffing normalised `shall`-statements
between the two.

## Validating the corpus

```sh
tools/check-nspec.py docs/spec     # format + corpus invariants
tools/coverage-report.py           # how much normative source text landed
nplan spec status                  # what nspec itself sees
```

`check-nspec.py` exists because **nspec reports nothing when a rule marker is
malformed** — the line simply isn't a rule, silently. It mirrors the def-site
grammar and flags every line that looks like it was meant to be a rule but
wouldn't be seen, plus the invariants nspec doesn't enforce: duplicate ids
(coverage is keyed on the bare id, so a duplicate merges two rules into one),
empty bodies, missing citations, and markers inside fenced code blocks.

`coverage-report.py` measures the other direction: it pulls every normative
sentence out of the source slice (skipping informative regions) and checks that
a distinctive run of its words survives somewhere in the corpus. It currently
reports 99%; the residue is section lead-ins whose content is covered by the
individual rules beneath them ("The following operands shall be supported:") and
"a future version of this standard may…" notes, which impose no obligation on a
conforming implementation and were deliberately not converted.

Two parser behaviours worth knowing if you edit the corpus by hand:

- The def-site scanner **does not track fenced code blocks**. A `> [spec:…]`
  line inside a fence in a spec file registers as a real rule.
- The scanner **strips leading whitespace from body lines**, so indentation-
  significant material (the yacc grammar's aligned `|` continuations, a GFM
  table's leading pipes) reads differently in the parsed body than in the file.

## Corpus conventions

Rules follow the authoring contract in `build/AUTHORING.md`:

- Namespace `posix`; ids are `CONCERN.NAME`, and each source file owns an
  exclusive prefix so ids can't collide across the corpus.
- Verbs are chosen deliberately — `def` for vocabulary, `syn` for grammar and
  well-formedness, `sem` for behaviour stated without a normative keyword, `req`
  for obligations.
- **POSIX's wording is kept verbatim.** "shall" is not rewritten to "MUST":
  POSIX "shall" already carries RFC 2119 MUST force and nspec treats them as the
  same modality, so rewording would only risk changing the standard.
- Option-conditional text keeps the standard's own margin code inline, as a code
  span, at the point the standard shades it — `` `[UP]` `` (User Portability
  Utilities), `` `[XSI]` `` (X/Open System Interfaces), `` `[OB]` ``
  (obsolescent). Each code is defined once per file in a definition list after
  the RFC 2119 boilerplate. Where the shaded span is a few words mid-sentence,
  the `` `[Option Start]` `` / `` `[Option End]` `` pair is kept too, since the
  rule boundary no longer supplies the extent. An unconditional rule where the
  standard has a conditional one is a real defect.
- Informative text (APPLICATION USAGE, EXAMPLES, RATIONALE, FUTURE DIRECTIONS,
  SEE ALSO, CHANGE HISTORY) is excluded — it carries no requirements.
- Every rule cites its source section and anchor.

## Implementing against it

`.config/nspec/config.styx` declares an impl at `src/**/*.rs`. Annotate the
implementation with the rule it satisfies:

```rust
// [spec:posix:req:quote.backslash]
fn unescape(...) { ... }
```

and the test that verifies it:

```rust
// [spec:posix:req:quote.backslash/test]
#[test]
fn backslash_preserves_literal_value() { ... }
```

Then `nplan spec status` reports coverage, `nplan spec uncovered` lists what's
left, and `nplan unplanned` lists rules no plan node claims.

## Licensing

`docs/spec/` reproduces normative text from IEEE Std 1003.1-2024, Copyright ©
2001-2024 The IEEE and The Open Group. The vendored HTML carries the same
notice. The pipeline in `tools/` is the only original work here.
