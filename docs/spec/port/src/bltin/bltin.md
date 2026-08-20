# src/bltin/bltin.h

The compatibility header for the `bltin` sub-library — the builtins
(`printf`, `test`, `times`) that were imported from standalone BSD
utilities and can in principle still be built as such. It redefines the
stdio-ish names those sources use (`printf`, `putchar`, `warnx`,
`error`, …) onto the shell's own output layer, and defines `INITARGS` to
capture `argv[0]` into `commandname`, aborting with `"Argc is zero\n"`
when there is none.

**Dash source shape (`bltin.echocmd-fn`):**

    int echocmd(int, char **)

**Retired duplicate declaration (`bltin.echocmd-fn`):**
> The `echo` builtin, declared here and defined in `printf.c` (see
> `printf.echocmd-fn` for the full semantics). In outline: an initial
> `-n` suppresses the trailing newline; each remaining argument is
> printed through `print_escape_str`, so SysV-style backslash escapes are
> interpreted; arguments are separated by a single space and the last is
> followed by a newline unless `-n` was given or a `\c` escape was
> encountered.
