# src/show.c, src/show.h

Debug tracing and parse-tree dumping. The whole file is inside
`#ifdef DEBUG`, so none of it exists in a release build; the `TRACE`,
`TRACEV` and `trputs` call sites elsewhere compile away with it. Output
goes to `tracefile`, a stdio `FILE *` — deliberately not the shell's own
`struct output` layer, so tracing keeps working when that layer is the
thing being debugged.

Every routine here re-tests `debug != 1` and returns silently otherwise.
The comparison is against exactly 1, not merely non-zero, because
`debug` is a `set -o debug` option whose other values are reserved.

For the port, this is the natural place to substitute idiomatic
target-language tracing: the contract is "when tracing is enabled, emit
this information in this form", not the specific stdio calls.

> [spec:dash:def:show.indent-fn]
> static void indent(int amount, char *pfx, FILE *fp)

> [spec:dash:sem:show.indent-fn]
> Write `amount` tab characters, and when `pfx` is non-NULL emit it
> immediately before the *last* tab — so the prefix marks the deepest
> level rather than the start of the line.

> [spec:dash:def:show.opentrace-fn]
> void opentrace(void)

> [spec:dash:sem:show.opentrace-fn]
> Open or reopen the trace file. When tracing is off, flush an already
> open `tracefile` but deliberately leave it open — libedit may be using
> the descriptor — and return.
>
> The path is the literal `./trace`. (An `#ifdef not_this_way` block
> preserves an alternative that would place it under `$HOME`, or `/` for
> root and `/tmp` otherwise; it is not compiled.)
>
> If `tracefile` is already open, `freopen` it — on klibc, which lacks a
> working `freopen`, close and reopen instead. Otherwise `fopen` for
> append. On failure print `"Can't (re-)open ./trace"` to stderr, set
> `debug = 0` so nothing else tries, and return.
>
> Then set `O_APPEND` explicitly via `fcntl` where the macro exists, so
> concurrent shells appending to the same file do not overwrite each
> other, and `setlinebuf` so a crash does not lose buffered trace. Emit
> `"\nTracing started.\n"`.

> [spec:dash:def:show.sharg-fn]
> static void sharg(union node *arg, FILE *fp)

> [spec:dash:sem:show.sharg-fn]
> Print one `NARG` node, decoding the shell's internal control-byte
> encoding back into readable syntax. A node of any other type prints
> `<node type N>` and `abort()`s, since it indicates a corrupt tree.
>
> Walk `arg->narg.text` byte by byte, comparing as `signed char` because
> the control bytes are negative:
>
> - `CTLESC` — the next byte is literal; print it and skip the marker.
> - `CTLVAR` — a parameter expansion. Print `${`, take the subtype byte,
>   print `#` first for `VSLENGTH`, then copy the variable name up to the
>   `=` terminator, then `:` if `VSNUL` is set, then the operator implied
>   by `subtype & VSTYPE`: `}` for `VSNORMAL`, `-`, `+`, `?`, `=` for the
>   substitution forms, `#`/`##` for `VSTRIMLEFT`/`VSTRIMLEFTMAX`,
>   `%`/`%%` for `VSTRIMRIGHT`/`VSTRIMRIGHTMAX`, nothing for `VSLENGTH`,
>   and `<subtype N>` for anything unrecognised.
> - `CTLENDVAR` — print `}`.
> - `CTLBACKQ` — a command substitution. Print `$(`, recurse into
>   `shtree(bqlist->n, -1, NULL, fp)` with `ind` -1 so it renders inline
>   without newlines, then `)`. Note `bqlist` is not advanced, so a node
>   containing several substitutions prints the first one each time.
> - anything else — print it as-is.

> [spec:dash:def:show.shcmd-fn]
> static void shcmd(union node *cmd, FILE *fp)

> [spec:dash:sem:show.shcmd-fn]
> Print a simple command: its arguments, then its redirections,
> space-separated (the `first` flag suppresses the leading space).
>
> Each redirection prints as an optional descriptor number, the operator,
> and the target. The operator and its default descriptor are: `>`/1 for
> `NTO`, `>|`/1 for `NCLOBBER`, `>>`/1 for `NAPPEND`, `>&`/1 for `NTOFD`,
> `<`/0 for `NFROM`, `<&`/0 for `NFROMFD`, `<>`/0 for `NFROMTO`, and
> `*error*`/0 for anything else. The descriptor number is printed only
> when it differs from the operator's default, reproducing the source
> form. For the two duplicating forms the target is `ndup.dupfd` as a
> number; otherwise it is the filename node, printed with `sharg`.
>
> Note the separating spaces go to `putchar` (stdout) while everything
> else goes to `fp`; with `fp` set to something other than stdout the
> spaces are misdirected.

> [spec:dash:def:show.showtree-fn]
> void showtree(union node *n)

> [spec:dash:sem:show.showtree-fn]
> Dump a parse tree: trace `"showtree called\n"`, then
> `shtree(n, 1, NULL, stdout)` — indent level 1, no prefix, to standard
> output.

> [spec:dash:def:show.shtree-fn]
> static void shtree(union node *n, int ind, char *pfx, FILE *fp)

> [spec:dash:sem:show.shtree-fn]
> Print a command tree. `ind` is the indent level, and a negative `ind`
> additionally means "inline": no trailing newlines, used when rendering
> a command substitution inside an argument. A NULL node prints nothing.
>
> Indent, then dispatch on `n->type`:
>
> - `NSEMI`, `NAND`, `NOR` — print the left child, the operator
>   (`"; "`, `" && "`, `" || "`), then the right child, all at the same
>   indent level.
> - `NCMD` — `shcmd`, then a newline unless inline.
> - `NPIPE` — each element of `npipe.cmdlist` via `shcmd`, joined by
>   `" | "`, then `" &"` if the pipeline is backgrounded, then a newline
>   unless inline.
> - anything else — `<node type N>` and a newline unless inline. Loops,
>   conditionals, function definitions and subshells are therefore not
>   rendered; this is a debugging aid, not a decompiler.

> [spec:dash:def:show.trace-fn]
> void trace(const char *fmt, ...)

> [spec:dash:sem:show.trace-fn]
> `vfprintf` the formatted message to `tracefile`, when `debug == 1`.
> This is what the `TRACE(( … ))` macro expands to — the doubled
> parentheses let the whole argument list be passed as one macro
> argument.

> [spec:dash:def:show.tracev-fn]
> void tracev(const char *fmt, va_list va)

> [spec:dash:sem:show.tracev-fn]
> As `trace` but taking an already-collected `va_list`, for callers that
> are themselves variadic. Backs the `TRACEV` macro.

> [spec:dash:def:show.trargs-fn]
> void trargs(char **ap)

> [spec:dash:sem:show.trargs-fn]
> Trace a NULL-terminated argument vector: `trstring` each element,
> separated by spaces, with a newline after the last. An empty vector
> emits nothing at all — not even the newline.

> [spec:dash:def:show.trputc-fn]
> void trputc(int c)

> [spec:dash:sem:show.trputc-fn]
> Write one character to `tracefile`, when `debug == 1`.

> [spec:dash:def:show.trputs-fn]
> void trputs(const char *s)

> [spec:dash:sem:show.trputs-fn]
> Write a NUL-terminated string to `tracefile`, when `debug == 1`.

> [spec:dash:def:show.trstring-fn]
> static void trstring(char *s)

> [spec:dash:sem:show.trstring-fn]
> Write `s` to the trace file in double quotes with non-printables
> escaped, so control bytes in the shell's internal encoding are
> readable. Escape `\n`, `\t`, `\r`, `"` and `\` conventionally, and give
> the internal markers their own letters: `CTLESC` as `\e`, `CTLVAR` as
> `\v`, `CTLBACKQ` as `\q`. Any other byte outside the printable ASCII
> range `' '`..`'~'` is written with a backslash followed by
> `putc(*p >> 6 & 03)`, `putc(*p >> 3 & 07)` and `putc(*p & 07)`.
>
> **Those are raw values, not digit characters** — the code omits the
> `'0' +` that would make them printable — so the escape renders as three
> control bytes rather than as `\101`. It is a real bug in the debug
> tracing. Wave 2 must reproduce it exactly: emitting `'0' + digit`
> instead would be a behaviour change, and this rule previously (and
> wrongly) instructed that change.
