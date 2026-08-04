# nspec authoring contract — POSIX shell corpus

You are converting a slice of the POSIX.1-2024 shell specification (IEEE Std
1003.1-2024 / The Open Group Base Specifications Issue 8) into an **nspec** rule
corpus. Read this whole file before writing anything.

## The format, exactly

A rule is a **markdown blockquote** whose **first line is a bare rule marker**:

```markdown
> [spec:posix:req:quote.backslash]
> A <backslash> that is not quoted shall preserve the literal value of the
> following character, with the exception of a <newline>.
>
> Source: XCU 2.2.1 Escape Character (Backslash) — utilities/V3_chap02.html#tag_19_02_01
```

The parser is a **line scanner**, not a markdown parser. These are hard
requirements — violating any of them makes the rule silently vanish, with no
error reported anywhere:

1. The marker line's first non-whitespace character is `>`.
2. After `> ` the very next thing is a complete, well-formed marker.
3. **Nothing but whitespace follows the closing `]`.** A trailing title kills
   the rule.
4. Use exactly one space after `>`. Two spaces breaks rendering.
5. Body lines each start with `> `. A bare `>` continues the rule (use it for
   paragraph breaks). A **truly blank line ends the rule.**
6. **Never start a body line with a bare `[spec:...]` marker** — it would start
   a new rule. Wrap in-body rule references in backticks: `` `[spec:posix:req:x]` ``.
7. **Never put a `>`-leading marker inside a fenced code block.** The scanner
   does not track fences and would read it as a real rule.

### Marker grammar

```
[spec:posix:<verb>:<id>]
```

- Namespace is always `posix`.
- `<verb>` is one of **`def` `syn` `sem` `req` `thm`** — nothing else parses.
- `<id>` is `[a-z0-9._-]+`. **Lowercase only.** Uppercase fails to parse.
- **Do not write a `+N` version pin.** Every rule stays at v0.
- Do not write a `/facet` suffix on a definition.

### Choosing the verb — do not default everything to `req`

| Verb | Use when the passage… | POSIX examples |
|---|---|---|
| `def` | introduces a name, term, or artifact | what a *reserved word* is; the list of special parameters; the set of shell variables |
| `syn` | states well-formedness or grammar | token recognition; the yacc grammar; pattern matching notation; operator forms |
| `sem` | describes runtime behaviour with no normative keyword | what an expansion evaluates to; how a construct is processed |
| `req` | states an obligation — POSIX "shall", "shall not", "should", "may" | "the shell shall …", "the application shall ensure …" |
| `thm` | states a provable derived result | rare here; probably none in your slice |

Most POSIX prose is `req`, but a slice that is *all* `req` means you did not
look. Sections that define vocabulary or grammar genuinely are `def`/`syn`.

## Rule ids

Convention is `CONCERN.NAME`, dot-separated, kebab-case within a segment.

**You have been assigned an exclusive id prefix. Every rule you write must use
it.** Coverage is keyed on the bare id across the whole corpus, so an id
colliding with another file's silently merges two rules into one.

Make the `NAME` part descriptive of the obligation, not the section number:
`quote.backslash`, `redir.here-doc-delimiter`, `expand.tilde-unquoted` — not
`quote.2-2-1`.

## What to convert, and what to skip

**Skip entirely** anything between `<!-- INFORMATIVE-START -->` and
`<!-- INFORMATIVE-END -->` markers. That is non-normative text — APPLICATION
USAGE, EXAMPLES, RATIONALE, FUTURE DIRECTIONS, SEE ALSO, CHANGE HISTORY. It is
not part of the standard's requirements and must not become rules.

Also skip: the `NAME` one-liner of a built-in, and pure cross-reference lists.

**Convert** all normative prose: DESCRIPTION, OPTIONS, OPERANDS, STDIN, INPUT
FILES, ENVIRONMENT VARIABLES, ASYNCHRONOUS EVENTS, STDOUT, STDERR, OUTPUT FILES,
EXTENDED DESCRIPTION, EXIT STATUS, CONSEQUENCES OF ERRORS, and all of the
chapter-2 prose sections.

## Fidelity — this is a standard, not a blog post

- **Keep POSIX's wording verbatim** wherever you can. Do not paraphrase a
  normative sentence, and do not "translate" *shall* into *MUST*. POSIX "shall"
  already carries RFC 2119 MUST force, and nspec treats `shall` and `must` as
  the same modality. Rewording a standard silently changes it.
- You may drop pure cross-reference parentheticals ("(see 2.6.2 Parameter
  Expansion)") when they add nothing, but keep them when the reference is
  load-bearing.
- Character names like `<backslash>`, `<newline>`, `<space>` appear escaped in
  the source markdown as `\<backslash\>`. Write them unescaped in rule bodies.
- **Do not correct the standard.** POSIX.1-2024 contains typos. Reproduce them
  verbatim and append `[sic]` — e.g. "the fist [sic] character". Silently fixing
  one makes the corpus diverge from the text it claims to reproduce, and the
  next reader can't tell your correction from the standard's wording.
- **Preserve option markers, using the standard's own notation.** POSIX shades
  option-conditional text in the margin: a code (`UP`, `XSI`, `OB`) names the
  option, then `[Option Start]` … `[Option End]` bracket the extent. Mirror that
  inline — do not paraphrase it into prose, and never describe the *typography*
  ("bracketed by the Option Start/Option End markers in the standard" tells an
  implementer nothing).

  Write the code as a code span so it can't be read as a link reference:

  - **Whole rule is conditional** — lead the body with the code. The rule
    boundary is the extent, so no end marker is needed:

    ```
    > [spec:posix:req:param.env]
    > `[UP]` The processing of the ENV shell variable shall be supported if the
    > system supports the User Portability Utilities option.
    ```

  - **A span mid-sentence is conditional** — keep all three of the standard's
    tokens, exactly where it puts them:

    ```
    > If an invalid signal name `[XSI]` `[Option Start]` or number `[Option End]`
    > is specified, the trap utility shall write a warning message to standard
    > error.
    ```

  Prefer splitting so a rule is wholly conditional; fall back to the inline
  bracket only when the shaded span is a few words inside a sentence.

  Define each code you use once, in a definition list after the RFC 2119
  boilerplate (markdawn supports these; nspec ignores them because they are not
  blockquoted):

  ```
  Option-conditional text carries the standard's own margin code inline, at the point the standard shades it:

  `[UP]`
  : User Portability Utilities. The functionality described is optional.
  ```

  The `: ` must be at the start of the line — markdawn's recogniser tolerates no
  leading whitespace.

  Getting the **extent** right matters as much as the code. In the `ENV` entry
  the shading covers one sentence and the paragraph after it is unconditional;
  sweeping both under the condition is a real defect. Check the extent against
  the `[Option Start]`/`[Option End]` pair in `build/units/`, not against your
  sense of what sounds conditional.

## Granularity

One rule per **coherent normative requirement** — usually a paragraph, sometimes
a tight group of sentences that only make sense together, occasionally a single
dense sentence carrying several obligations.

- Do not emit one rule per sentence. That produces unusable confetti.
- Do not emit one rule per section heading. That buries ten obligations in one
  body that no implementation can annotate against precisely.
- A rule should be something an implementer can point a single function or test
  at and say "this satisfies it".

Tables (the parameter-expansion truth table, the shell-errors table, the
pipefail table) are normative. Reproduce a table inside a rule body as a GFM
table with each line prefixed `> `.

## Source citation

End every rule body with a bare `>` line and then a citation line:

```
>
> Source: XCU 2.7.4 Here-Document — utilities/V3_chap02.html#tag_19_07_04
```

Take the section number and title from the nearest enclosing heading, and the
anchor from the `<a id="tag_..."></a>` line that precedes that heading in the
source markdown. For `sh` pages the path is `utilities/sh.html`.

## File shape

Your output file starts with a title heading and the RFC 2119 boilerplate, then
groups rules under `##` headings that mirror the source's section structure.
Headings are decorative — the parser ignores them — but they are how a human
navigates the corpus, so keep them faithful to the standard's own structure.

```markdown
# Quoting

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
"SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in RFC 2119. This document
reproduces normative text from IEEE Std 1003.1-2024 (POSIX.1-2024),
Copyright © 2001-2024 The IEEE and The Open Group.

## 2.2.1 Escape Character (Backslash)

> [spec:posix:req:quote.backslash]
> …
```

Separate consecutive rules with one blank line.

## Before you finish

Re-read your own output against requirements 1–7 above, checking specifically:

- every marker line ends with `]` and nothing else;
- no body line begins with a bare `[spec:` marker;
- no marker sits inside a fenced code block;
- every id starts with your assigned prefix and is lowercase;
- ids are unique within your file.

Report back: the output path, the rule count, the verb breakdown, and anything
in your slice you deliberately did not convert (and why).
