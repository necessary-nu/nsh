# src/mkinit.c

A build-time code generator, not part of the shell. It scans every `.c`
source file for specially marked fragments and emits `init.c`, which
defines the five lifecycle routines declared in `init.h`. The point is
encapsulation: a module can register startup or reset code without
anything outside it knowing.

Usage is `mkinit sourcefile...`; output goes to `init.c.new` and is then
renamed to `init.c`, so a failed run never leaves a truncated `init.c`.

**What it recognises**, each at the start of a line:

- an event keyword — `INIT`, `EXITRESET`, `FORKRESET`, `POSTEXITRESET`,
  `RESET` — followed by a brace-delimited block, whose body is appended
  to that event's routine.
- `INCLUDE "file"` or `INCLUDE <file>`, which adds a header to the
  generated file's include list.
- `MKINIT` alone, introducing a struct/union declaration to be copied
  into the generated file; or `MKINIT <declaration>`, which is emitted as
  an `extern` declaration with any initialiser stripped.
- a simple `#define`, which is copied out preceded by a matching
  `#undef`.

These markers sit inside `#ifdef mkinit` in the real sources, so the C
compiler never sees them.

**Porting note.** Wave 2 does not need to reproduce a code generator: the
contract is the *content and ordering* of the five routines, which
`init.md` records. A Rust port can call the corresponding per-module
functions directly. The rules here specify what the generator does so
that the generated output remains derivable.

The `event[]` table drives everything; each entry names the keyword, the
routine to emit, a comment, an optional argument list (only `FORKRESET`
has one, `union node *n`), and a `struct text` accumulating the code.

> [spec:dash:def:mkinit.addchar-fn]
> void addchar(int c, struct text *text)

> [spec:dash:sem:mkinit.addchar-fn]
> Append one character to a `struct text`. Decrement `nleft`; when it
> goes negative the current block is full, so allocate a new
> `struct block`, link it (or make it the first), and reset `nextc` and
> `nleft` to `BLOCKSIZE - 1`. Then store the character and advance.

> [spec:dash:def:mkinit.addstr-fn]
> void addstr(char *s, struct text *text)

> [spec:dash:sem:mkinit.addstr-fn]
> Append a NUL-terminated string. Per character, decrement `nleft` and
> either store directly in the current block or, when it goes negative,
> delegate that one character to `addchar` — which allocates the next
> block and re-does the decrement, so the two stay consistent.

> [spec:dash:def:mkinit.block]
> struct block {
>   struct block *next;
>   char text[BLOCKSIZE];
> }

> [spec:dash:def:mkinit.ckfopen-fn]
> FILE * ckfopen(char *file, char *mode)

> [spec:dash:sem:mkinit.ckfopen-fn]
> `fopen`, printing `"Can't open <file>"` to stderr and `exit(2)` on
> failure.

> [spec:dash:def:mkinit.ckmalloc-fn]
> void * ckmalloc(int nbytes)

> [spec:dash:sem:mkinit.ckmalloc-fn]
> `malloc`, calling `error("Out of space")` on failure. Distinct from the
> shell's own `ckmalloc` — this program does not link against the shell.

> [spec:dash:def:mkinit.dodecl-fn]
> void dodecl(char *line1, FILE *fp)

> [spec:dash:sem:mkinit.dodecl-fn]
> Handle a `MKINIT` declaration.
>
> `MKINIT` alone on the line starts a struct or union declaration: copy
> lines verbatim into `decls` until one begins with `}`, and clear
> `amiddecls`.
>
> Otherwise the rest of the line (from offset 6, past `MKINIT`) is a
> variable declaration. Scan for `=`, `/` or newline; on an `=`, find the
> `;` that ends the statement and cut the initialiser out, trimming
> trailing spaces before the `=` — so
> `MKINIT int tpip[2] = { -1 };` becomes `extern int tpip[2];`. Emit
> `extern` plus the declaration plus whatever followed the initialiser,
> and set `amiddecls` so consecutive declarations are not separated by
> blank lines.

> [spec:dash:def:mkinit.doevent-fn]
> void doevent(struct event *ep, FILE *fp, char *fname)

> [spec:dash:sem:mkinit.doevent-fn]
> Copy one event block's body into that event's accumulated code. Emit a
> `/* from <file>: */` comment and an opening brace, then read lines
> until one is exactly `}\n`, which ends the block and is not copied.
>
> Each line is re-indented: count the original leading tabs as 8 columns
> each and spaces as 1, add a base indent of 6, then re-emit that many
> columns as tabs and spaces before the line's content. Blank lines and
> preprocessor lines get no indent at all, so `#ifdef`s stay at column 0.
> Close with a matching brace. Unexpected end of file is
> `error("Unexpected EOF")`.

> [spec:dash:def:mkinit.doinclude-fn]
> void doinclude(char *line)

> [spec:dash:sem:mkinit.doinclude-fn]
> Record a header for the generated file. Find the opening `"` or `<` —
> its absence is `error("Expecting '\"' or '<'")` — then scan to
> whitespace and require the preceding character to be the matching `"`
> or `>`, else `error("Missing terminator")`. NUL-terminate the name and
> add it to `header_files` only if not already present, so duplicates
> across source files collapse.

> [spec:dash:def:mkinit.error-fn]
> static void error(char *msg)

> [spec:dash:sem:mkinit.error-fn]
> Print `<file>:<line>: ` when a file is being read, then the message,
> and `exit(2)`. Does not return.

> [spec:dash:def:mkinit.event]
> struct event {
>   char *name;
>   char *routine;
>   char *comment;
>   char *args;
>   struct text code;
> }

> [spec:dash:def:mkinit.gooddefine-fn]
> int gooddefine(char *line)

> [spec:dash:sem:mkinit.gooddefine-fn]
> Decide whether a `#define` may be copied into the generated file.
> Requires the line to start with `#define`. Rejects function-like macros
> — detected by a `(` before the first whitespace after the name — and
> multi-line definitions, detected by a trailing backslash. Only simple
> object-like macros qualify, since those are the only ones safe to
> re-emit out of context.

> [spec:dash:def:mkinit.main-fn]
> int main(int argc, char **argv)

> [spec:dash:sem:mkinit.main-fn]
> Seed `header_files` with `"shell.h"`, `"mystring.h"` and `"init.h"`,
> scan every file named on the command line with `readfile`, write the
> result with `output`, then `rename(OUTTEMP, OUTFILE)` so `init.c`
> appears atomically. Exit 0.

> [spec:dash:def:mkinit.match-fn]
> int match(char *name, char *line)

> [spec:dash:sem:mkinit.match-fn]
> Return whether `line` begins with `name` *as a whole token*: every
> character of `name` must match, and the character that follows in
> `line` must be `{`, space, tab or newline. That trailing check is what
> stops `INITIALIZE` from being taken for `INIT`.

> [spec:dash:def:mkinit.output-fn]
> void output(void)

> [spec:dash:sem:mkinit.output-fn]
> Write `init.c.new`: the "generated by mkinit" banner, one `#include`
> per collected header, the accumulated `#undef`/`#define` pairs, the
> accumulated declarations, and then one routine per event — its comment,
> `void <routine>(<args>) {`, its accumulated code, and a closing brace.
> Events with no registered code still produce an empty function, which
> is what lets callers call all five unconditionally.

> [spec:dash:def:mkinit.readfile-fn]
> void readfile(char *fname)

> [spec:dash:sem:mkinit.readfile-fn]
> Scan one source file line by line, tracking `curfile` and `linno` for
> error messages. For each line: check it against every event keyword
> (with a cheap first-character test before the full `match`) and, on a
> hit, hand the block to `doevent`; check for `INCLUDE` and `MKINIT`; and
> for a `#define` that `gooddefine` approves, emit a matching `#undef`
> followed by the definition itself.
>
> The `#undef` is synthesised by copying the line, overwriting the
> leading `#define ` with `#undef `, then skipping whitespace and the
> macro name and terminating there — so a definition that is already in
> scope is cleanly replaced rather than warned about.

> [spec:dash:def:mkinit.savestr-fn]
> char * savestr(char *s)

> [spec:dash:sem:mkinit.savestr-fn]
> `strdup` via this program's `ckmalloc`.

> [spec:dash:def:mkinit.text]
> struct text {
>   char *nextc;
>   int nleft;
>   struct block *start;
>   struct block *last;
> }

> [spec:dash:def:mkinit.writetext-fn]
> void writetext(struct text *text, FILE *fp)

> [spec:dash:sem:mkinit.writetext-fn]
> Write a `struct text` to a file: every block before the last in full,
> then `BLOCKSIZE - nleft` bytes of the last — the only partially filled
> one. An empty text writes nothing. A short `fwrite` is
> `error("Can't write data\n")`.
