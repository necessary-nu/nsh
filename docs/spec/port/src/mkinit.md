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

**Historical note.** nsh does not reproduce this code generator. The
observable lifecycle ordering is recorded in `init.md` and implemented by
explicit Rust subsystem calls. The text here records what the generator did so
that the generated output remains derivable.

The `event[]` table drives everything; each entry names the keyword, the
routine to emit, a comment, an optional argument list (only `FORKRESET`
has one, `union node *n`), and a `struct text` accumulating the code.

**Rules retired.** `delete-gen` removed `crates/nsh/src/gen/mkinit.rs`.
The workspace has no `build.rs`, so nothing in the Rust build ran it, and
the port hand-writes the five lifecycle routines rather than generating
them. `src/Makefile.am` still builds and runs this program for the C
reference, which is untouched. The blocks below therefore carry no
`[spec:dash:…]` ids — each is a C signature followed by its semantics —
and are kept because they still describe `src/mkinit.c`.

> void addchar(int c, struct text *text)

> Append one character to a `struct text`. Decrement `nleft`; when it
> goes negative the current block is full, so allocate a new
> `struct block`, link it (or make it the first), and reset `nextc` and
> `nleft` to `BLOCKSIZE - 1`. Then store the character and advance.

> void addstr(char *s, struct text *text)

> Append a NUL-terminated string. Per character, decrement `nleft` and
> either store directly in the current block or, when it goes negative,
> delegate that one character to `addchar` — which allocates the next
> block and re-does the decrement, so the two stay consistent.

> struct block {
>   struct block *next;
>   char text[BLOCKSIZE];
> }

> FILE * ckfopen(char *file, char *mode)

> `fopen`, printing `"Can't open <file>"` to stderr and `exit(2)` on
> failure.

> void * ckmalloc(int nbytes)

> `malloc`, calling `error("Out of space")` on failure. Distinct from the
> shell's own `ckmalloc` — this program does not link against the shell.

> void dodecl(char *line1, FILE *fp)

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

> void doevent(struct event *ep, FILE *fp, char *fname)

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

> void doinclude(char *line)

> Record a header for the generated file. Find the opening `"` or `<` —
> its absence is `error("Expecting '\"' or '<'")` — then scan to
> whitespace and require the preceding character to be the matching `"`
> or `>`, else `error("Missing terminator")`. NUL-terminate the name and
> add it to `header_files` only if not already present, so duplicates
> across source files collapse.

> static void error(char *msg)

> Print `<file>:<line>: ` when a file is being read, then the message,
> and `exit(2)`. Does not return.

> struct event {
>   char *name;
>   char *routine;
>   char *comment;
>   char *args;
>   struct text code;
> }

> int gooddefine(char *line)

> Decide whether a `#define` may be copied into the generated file.
> Requires the line to start with `#define`. Rejects function-like macros
> — detected by a `(` before the first whitespace after the name — and
> multi-line definitions, detected by a trailing backslash. Only simple
> object-like macros qualify, since those are the only ones safe to
> re-emit out of context.

> int main(int argc, char **argv)

> Seed `header_files` with `"shell.h"`, `"mystring.h"` and `"init.h"`,
> scan every file named on the command line with `readfile`, write the
> result with `output`, then `rename(OUTTEMP, OUTFILE)` so `init.c`
> appears atomically. Exit 0.

> int match(char *name, char *line)

> Return whether `line` begins with `name` *as a whole token*: every
> character of `name` must match, and the character that follows in
> `line` must be `{`, space, tab or newline. That trailing check is what
> stops `INITIALIZE` from being taken for `INIT`.

> void output(void)

> Write `init.c.new`: the "generated by mkinit" banner, one `#include`
> per collected header, the accumulated `#undef`/`#define` pairs, the
> accumulated declarations, and then one routine per event — its comment,
> `void <routine>(<args>) {`, its accumulated code, and a closing brace.
> Events with no registered code still produce an empty function, which
> is what lets callers call all five unconditionally.

> void readfile(char *fname)

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

> char * savestr(char *s)

> `strdup` via this program's `ckmalloc`.

> struct text {
>   char *nextc;
>   int nleft;
>   struct block *start;
>   struct block *last;
> }

> void writetext(struct text *text, FILE *fp)

> Write a `struct text` to a file: every block before the last in full,
> then `BLOCKSIZE - nleft` bytes of the last — the only partially filled
> one. An empty text writes nothing. A short `fwrite` is
> `error("Can't write data\n")`.
