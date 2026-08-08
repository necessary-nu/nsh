# src/mknodes.c

A build-time code generator, not part of the shell. It reads the
`nodetypes` description file and the `nodes.c.pat` template and emits
`nodes.h` (the parse-tree types) and `nodes.c` (the deep-copy routines).
Invoked as `mknodes nodetypes nodes.c.pat`.

**Input format.** A line starting in column 0 declares a node type:
`<NODENAME> <structtag>`. Several node types may share a struct tag, in
which case they share its layout — `NSEMI`, `NAND` and `NOR` all use
`nbinary`. Indented lines declare that struct's fields:
`<name> <type> [decl]`, where the type is one of `nodeptr` (a
`union node *`), `nodelist` (a `struct nodelist *`), `string` (a
`char *`), `int`, `other` (the declaration text follows verbatim) or
`temp` (like `other`, but the field is not copied). Comments run from
`#` to end of line.

The field type is what drives the generated copy code, which is the real
purpose of the tool: a `nodeptr` is copied recursively, a `string` is
duplicated into the function block, an `int`/`other` is copied by value,
and a `temp` is skipped.

**Porting note.** Wave 2 need not reproduce the generator: what must
survive is the *set of node types and their field layouts*, and the
deep-copy semantics that `copyfunc`/`freefunc` provide. In Rust an enum
with per-variant data and a derived clone covers both.

**Rules retired.** `delete-gen` removed `crates/nsh/src/gen/mknodes.rs`.
The workspace has no `build.rs`, so nothing in the Rust build ran it, and
`crates/nsh/src/nodes.rs` stopped being its output when
[dec:nsh:owned-data] made the parse tree an owned enum — `%SIZES`,
`%CALCSIZE` and `%COPY`, the three things this program emits, have no
counterpart there. `src/Makefile.am` still builds and runs it for the C
reference, which is untouched. The blocks below therefore carry no
`[spec:dash:…]` ids — each is a C signature followed by its semantics —
and are kept because they still describe `src/mknodes.c`.

> static void error(const char *msg, ...)

> Print `line <n>: ` followed by the printf-style message and a newline
> to stderr, then `exit(2)`. Does not return.

> struct field {
>   char *name;
>   int type;
>   char *decl;
> }

> static void indent(int amount, FILE *fp)

> Emit `amount` columns of indentation as tabs while at least 8 remain,
> then one space per remaining column. The space loop is
> `while (--amount >= 0)`, which pre-decrements and so emits exactly the
> remaining count — `indent(12)` produces one tab and four spaces,
> landing on column 12.

> int main(int argc, char **argv)

> Require exactly two arguments, else `error("usage: mknodes file")`.
> Open the node description file, then read it line by line: a line
> starting with a space or tab is a field (`parsefield`), a non-empty
> line starting in column 0 is a node type (`parsenode`), and an empty
> line is skipped. Then `output(argv[2])` with the template file name,
> and exit 0.

> static int nextfield(char *buf)

> Read the next whitespace-delimited word from the current line into
> `buf`, advancing `linep`. Returns whether a non-empty word was found.

> static void outfunc(FILE *cfile, int calcsize)

> Emit the body of either `calcsize` (`calcsize` non-zero) or `copynode`
> (zero) — the two halves of the deep copy, which must walk the tree
> identically so that the size computed by the first is exactly what the
> second consumes.
>
> Both start by returning early for a NULL node (`return;` versus
> `return NULL;`). The size pass then adds `nodesize[n->type]` to
> `funcblocksize`; the copy pass carves that many bytes off `funcblock`.
>
> Then a `switch` on `n->type` with one arm per struct — every node type
> mapping to that struct becomes a `case` label on the same arm, which is
> how shared layouts collapse. Within an arm, fields are emitted from
> last to *index 1*, deliberately skipping field 0, which is always the
> `type` field and is handled separately.
>
> Per field type: `nodeptr` emits `calcsize(...)` or
> `new->… = copynode(...)`; `nodelist` emits `sizenodelist(...)` or
> `copynodelist(...)`; `string` emits `funcstringsize += strlen(...) + 1`
> or `nodesavestr(...)`; `int` and `other` emit a plain assignment in the
> copy pass and nothing in the size pass; `temp` emits nothing at all.
>
> The copy pass finishes with `new->type = n->type;`.

> static void output(char *file)

> Write `nodes.h` and `nodes.c`.
>
> `nodes.h` gets the banner, one `#define <NODENAME> <n>` per node type
> numbered in declaration order, one `struct` per distinct tag with its
> fields, then `union node` — an `int type` followed by one member per
> struct, so any node can be accessed through the right view — then the
> fixed `struct nodelist` and `struct funcnode` definitions and the
> `copyfunc`/`freefunc` prototypes.
>
> `nodes.c` is produced by copying the template file, substituting three
> markers: `%SIZES` becomes the `nodesize[]` table, `%CALCSIZE` the body
> of the size pass and `%COPY` the body of the copy pass. Markers are
> recognised after leading whitespace, and every other line is copied
> verbatim.
>
> Note the node numbering is positional, so reordering `nodetypes`
> silently renumbers every node — and `eval.c` has `#error` checks that
> depend on `NAND`, `NOR` and `NSEMI` staying consecutive.

> static void outsizes(FILE *cfile)

> Emit `static const short nodesize[N]`, one entry per node type giving
> `SHELL_ALIGN(sizeof (struct <tag>))` — the aligned size of whichever
> struct that node type uses. Indexed by node type at run time to size
> each copy.

> static void parsefield(void)

> Parse one field line into the current struct. Errors if there is no
> current struct or it is already complete, or if the name or type word
> is missing.
>
> Map the type word to a `T_*` code and, for the four built-in kinds,
> synthesise the C declaration: `nodeptr` → `union node *name`,
> `nodelist` → `struct nodelist *name`, `string` → `char *name`, `int` →
> `int name`. An unknown type word is an error.
>
> For `other` and `temp` the rest of the line is the declaration text and
> is taken verbatim; for the others, anything left on the line is
> `error("Garbage at end of line")`.

> static void parsenode(void)

> Parse a node type declaration: a node name and a struct tag, with
> anything further being an error. First mark the previous struct
> complete if it had any fields — which is what allows several node names
> to precede one field list.
>
> Record the node name, then look for an existing struct with that tag:
> on a hit the node simply reuses it, and on a miss a new one is created
> and becomes current. Either way the node's struct pointer is recorded
> and the node count advances.

> static int readline(void)

> Read one line into the buffer, returning 0 at end of file. Truncate at
> the first `#` or newline, then strip trailing spaces and tabs, so a
> comment-only or blank line becomes empty. Reset the field cursor,
> increment `linno`, and error if the result exceeds `BUFLEN`.

> static char * savestr(const char *s)

> `malloc` a copy of `s`, calling `error("Out of space")` on failure.

> static void skipbl(void)

> Advance `linep` past spaces and tabs.

> struct str {
>   char *tag;
>   int nfields;
>   struct field field[MAXFIELDS];
>   int done;
> }
