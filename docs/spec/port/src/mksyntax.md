# src/mksyntax.c

A build-time code generator, not part of the shell. It emits `syntax.h`
and `syntax.c`, which define the byte-classification tables the parser
indexes on every input character.

**The tables.** Four syntax tables — `basesyntax`, `dqsyntax`,
`sqsyntax`, `arisyntax` — plus one classification table, `is_type`. Each
is 257 entries covering byte values -129..127, indexed through the
`SYNBASE` offset (129): `BASESYNTAX` is `basesyntax + SYNBASE`, so the
parser can index with a sign-extended `char` directly, and `PEOF` is
-129, which lands at index 0. That is why `pgetc` returns characters
through `(signed char)` and why `PEOF` is exactly -129.

**Syntax classes**, numbered by position in `synclass[]`: `CWORD` (0,
nothing special), `CNL`, `CBACK`, `CSQUOTE`, `CDQUOTE`, `CENDQUOTE`,
`CBQUOTE`, `CVAR`, `CENDVAR`, `CLP`, `CRP`, `CEOF`, `CCTL` (like `CWORD`
but must be escaped, because the byte collides with an internal marker),
`CSPCL` (terminates a word).

**`is_` classes** are bit flags: `ISDIGIT` (01), `ISUPPER` (02),
`ISLOWER` (04), `ISUNDER` (010), `ISSPECL` (020) — the note in the source
warns that adding classes may require changing `is_in_name`.

**Porting note.** What must survive is the *content* of the five tables
and the semantics of the `is_*` macros; a Rust port can build them as
`const` arrays or `match` arms rather than generating C.

> [spec:dash:def:mksyntax.add-fn]
> static void add(char *p, char *type)

> [spec:dash:sem:mksyntax.add-fn]
> Set every character of the string `p` to syntax class `type`, indexing
> as `(signed char)*p + 129` so the table's offset convention is applied.

> [spec:dash:def:mksyntax.filltable-fn]
> static void filltable(char *dftval)

> [spec:dash:sem:mksyntax.filltable-fn]
> Set all 257 entries of the working table to `dftval`.

> [spec:dash:def:mksyntax.init-fn]
> static void init(void)

> [spec:dash:sem:mksyntax.init-fn]
> Reset the working table for a new syntax: fill it with `CWORD`, set
> index 0 (i.e. `PEOF`) to `CEOF`, and mark every byte from `CTL_FIRST`
> to `CTL_LAST` as `CCTL` — the range reserved for the shell's internal
> markers, which must always be escaped when they appear as data.

> [spec:dash:def:mksyntax.main-fn]
> int main(int argc, char **argv)

> [spec:dash:sem:mksyntax.main-fn]
> Create `syntax.c` and `syntax.h`, each starting with the generated-file
> banner.
>
> **Header**: include `<ctype.h>`; `#undef CEOF` if the system defines it
> (stdio does); emit one `#define` per syntax class numbered by position
> and one per `is_` class as an octal bit value, each padded to column 32
> with its comment; then `SYNBASE` 129 and `PEOF` -129, the four
> `*SYNTAX` macros that apply the offset, and the `is_*` macros.
>
> **Tables**, each built by `init()` then a series of `add()` calls:
>
> - `basesyntax` — `\n` `CNL`; `\` `CBACK`; `'` `CSQUOTE`; `"` `CDQUOTE`;
>   `` ` `` `CBQUOTE`; `$` `CVAR`; `}` `CENDVAR`; and
>   `<>();&| ` plus tab as `CSPCL`.
> - `dqsyntax` — as base but `"` is `CENDQUOTE` rather than an opener,
>   no `CSQUOTE` and no `CSPCL` (nothing splits inside quotes); plus
>   `^!*?[=~:/-]` as `CCTL` — note the set ends with a literal `]`, so
>   both `[` and `]` are `CCTL` — the characters that must be protected
>   from later pattern matching and tilde expansion.
> - `sqsyntax` — only `\n` `CNL` and `'` `CENDQUOTE`; the same `CCTL` set
>   (again including `]`) plus `\`, since inside single quotes a
>   backslash is literal.
> - `arisyntax` — `\n`, `\`, `` ` ``, `$`, `}` as in base, plus `(` `CLP`
>   and `)` `CRP` for arithmetic nesting.
> - `is_type` — filled with `0` rather than `CWORD`, then digits
>   `ISDIGIT`, letters `ISLOWER`/`ISUPPER`, `_` `ISUNDER`, and
>   `#?$!-*@` `ISSPECL` (the special parameter names).
>
> Note the letter strings passed for `ISLOWER`/`ISUPPER` are
> `"abcdefghijklmnopqrstucvwxyz"` and its uppercase counterpart — which
> contain `c` twice and omit `v`'s correct position, an upstream typo
> that leaves `v`/`V` marked but reached via the duplicate. The resulting
> table still marks every letter, since `c` and `v` both appear; a port
> should reproduce the *table contents*, not the typo.

> [spec:dash:def:mksyntax.output-type-macros-fn]
> static void output_type_macros(void)

> [spec:dash:sem:mksyntax.output-type-macros-fn]
> Emit the character-classification macros into the header:
> `is_digit(c)` as the unsigned range test `((unsigned)((c) - '0') <= 9)`
> — locale-independent, unlike `isdigit`; `is_alpha`, `is_name` (letter or
> underscore) and `is_in_name` (alphanumeric or underscore) via `ctype`
> on the unsigned char value; `is_special(c)` as a lookup in `is_type`
> for `ISSPECL|ISDIGIT`, which is what recognises `$1`, `$@`, `$?` and
> friends; and `digit_val(c)` as `(c) - '0'`.

> [spec:dash:def:mksyntax.print-fn]
> static void print(char *name)

> [spec:dash:sem:mksyntax.print-fn]
> Emit the working table as `const char <name>[]` in `syntax.c`, with a
> matching `extern` declaration in `syntax.h`. Entries are written four
> per line, each padded to a nine-column field so the result lines up.

> [spec:dash:def:mksyntax.synclass]
> struct synclass {
>   char *name;
>   char *comment;
> }
