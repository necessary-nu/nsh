# src/bltin/printf.c

The `printf` and `echo` builtins, derived from the BSD standalone
`printf(1)`. Arguments are consumed through the module-global cursor
`gargv` by the `get*` helpers, each of which returns a benign default
when the arguments run out — that is how `printf` reuses its format
string against however many arguments it was given. `rval` accumulates
the exit status, set to 1 by `check_conversion` on a malformed numeric
argument while output still proceeds.

`PF` and `ASPF` are macros that call `printf`/`xasprintf` with zero, one
or two extra leading `int` arguments, chosen by how many `*` width or
precision values were collected into `array` — C has no way to build a
variadic call at runtime, so the three cases are written out.

`conv_escape` is declared in `system.h` and shared with the parser; its
semantics are specified in `system.md` under `system.conv-escape-fn`.

> [spec:dash:def:printf.check-conversion-fn]
> static void check_conversion(const char *s, const char *ep)

> [spec:dash:sem:printf.check-conversion-fn]
> Validate the result of a `strto*` conversion of `s` that stopped at
> `ep`. Leftover text warns `"%s: expected numeric value"` when nothing
> at all was consumed and `"%s: not completely converted"` otherwise; a
> full conversion that set `errno` to `ERANGE` warns with `strerror`.
> Any of these sets `rval` to 1, so the builtin reports failure while
> still printing the value it managed to derive.

> [spec:dash:def:printf.conv-escape-fn]
> unsigned conv_escape(char *str0, char *out0, bool mbchar)

> [spec:dash:sem:printf.conv-escape-fn]
> Decode one backslash escape. Defined here but declared in `system.h`
> and shared with `parser.c`; the full specification is
> `system.conv-escape-fn`. Called from this file with `mbchar` false, the
> `printf` dialect, in which `"` and `'` after a backslash are not
> special and no `CTLESC`/`CTLMBCHAR` framing is emitted.

> [spec:dash:def:printf.conv-escape-str-fn]
> static int conv_escape_str(char *str, char **sp)

> [spec:dash:sem:printf.conv-escape-str-fn]
> Expand SysV `echo`-style escapes in `str` onto the stack, storing the
> end position through `sp`. Returns 0 normally, or 0x100 if a `\c`
> escape was found — the caller treats that as "stop all further output",
> and the value is chosen so its low byte is 0, which is also what ends
> the loop.
>
> Copy characters until a `\`. A `\c` sets the 0x100 result and is
> emitted as a NUL, ending the loop. A `\0` followed by an octal digit
> has the `0` skipped first, because `%b` and `echo` octal escapes are
> written `\0nnn` rather than C's `\nnn`. Everything else goes to
> `conv_escape`, which reports how much input it consumed and how much
> output it produced.

> [spec:dash:def:printf.echocmd-fn]
> int echocmd(int argc, char **argv)

> [spec:dash:sem:printf.echocmd-fn]
> The `echo` builtin. An initial `-n` is consumed and changes the final
> format from `"%s\n"` to `"%s"`, suppressing the trailing newline.
>
> Then print each argument with `print_escape_str`, using the format
> `"%s "` for every argument except the last, which uses the final
> format — so arguments are space-separated and the line ends once. With
> no arguments at all, the empty string is printed with the final format,
> so a bare `echo` still emits a newline.
>
> Stop early if `print_escape_str` reports that a `\c` escape was
> encountered. Always returns 0.

> [spec:dash:def:printf.getchr-fn]
> static int getchr(void)

> [spec:dash:sem:printf.getchr-fn]
> Consume one argument and return its first character, or 0 when the
> arguments are exhausted. Note `**gargv++` reads the first byte and then
> advances the cursor.

> [spec:dash:def:printf.getdouble-fn]
> static double getdouble(void)

> [spec:dash:sem:printf.getdouble-fn]
> Consume one argument as a floating-point value, or 0 when exhausted.
> An argument beginning with `"` or `'` yields the numeric value of the
> character that follows it — the POSIX rule that lets `printf %d "'A"`
> print 65. Otherwise `strtod`, validated by `check_conversion`.

> [spec:dash:def:printf.getstr-fn]
> static char * getstr(void)

> [spec:dash:sem:printf.getstr-fn]
> Consume one argument and return it, or `nullstr` when exhausted.

> [spec:dash:def:printf.getuintmax-fn]
> static uintmax_t getuintmax(int sign)

> [spec:dash:sem:printf.getuintmax-fn]
> Consume one argument as an integer, or 0 when exhausted. `sign`
> selects `strtoimax` over `strtoumax`. An argument beginning with `"` or
> `'` yields the value of the following character. Otherwise the string
> is converted with base 0, so `0x` hex and leading-`0` octal forms are
> honoured — and, on glibc >= 2.38, `0b`/`0B` binary too. `<inttypes.h>`
> redirects `strtoimax`/`strtoumax` to `__isoc23_*` whenever C23 strtol
> semantics are enabled, which they are throughout the dash build, and
> those variants accept binary constants at base 0. `nm -D` on the
> reference binary shows `__isoc23_strtoimax@GLIBC_2.38`, so
> `printf '%d' 0b11` prints 3. A port that binds the plain symbol loses
> binary literals. Validated by `check_conversion`.

> [spec:dash:def:printf.mklong-fn]
> static char * mklong(const char *str, const char *ch)

> [spec:dash:sem:printf.mklong-fn]
> Rewrite an integer conversion specification to use the `intmax_t`
> length modifier: `"%92.3u"` becomes `"%92.3" PRIuMAX`. Copy everything
> up to but excluding the conversion character, append `PRIdMAX`, then
> overwrite its final character with the original conversion character.
> This assumes `PRIiMAX`, `PRIoMAX`, `PRIuMAX`, `PRIxMAX` and `PRIXMAX`
> are all `PRIdMAX` with the last character substituted — not guaranteed
> by C99, but true everywhere dash is built. Returns the copy on the
> stack.

> [spec:dash:def:printf.print-escape-str-fn]
> static int print_escape_str(const char *f, int *param, int *array, char *s)

> [spec:dash:sem:printf.print-escape-str-fn]
> Print `s` with its escapes expanded, honouring the field width and
> precision of format `f`. Returns non-zero if a `\c` was encountered, so
> the caller stops.
>
> Inside a stack mark, expand the escapes with `conv_escape_str`. The
> expanded text may contain embedded NULs, so it cannot simply be handed
> to `printf`; the length is tracked explicitly.
>
> The byte at `q[-1]` is then set to `f[2]` — the character after `%s` in
> the caller's format, i.e. `echo`'s separating space or trailing
> newline — exactly when the conversion **is** `%s` **and** no `\c` was
> seen. The expression `(!!((f[1] - 's') | done) - 1) & f[2]` computes
> that branchlessly: when `f[1] == 's'` and `done == 0` the inner value
> is 0, `!!0 - 1` is -1 (all bits set), and the mask yields `f[2]`; in
> every other case the inner value is non-zero, `1 - 1` is 0, and the
> mask yields 0. `total` is then adjusted by whether the result was
> non-zero.
>
> This is what appends the separator in `echo`: `echocmd` passes `"%s "`
> for every argument but the last and `"%s\n"` (or `"%s"` under `-n`) for
> the last, and a `\c` escape suppresses it.
>
> For a plain `%s` format that is the whole job. Otherwise the width and
> precision must be applied to text `printf` cannot be given directly:
> build a placeholder string of `total` `X` characters, format *that*
> with `ASPF` so `printf` computes the padding, then find the run of `X`
> in the result and overwrite it with the real bytes. Emit the result
> with `out1mem`, which takes an explicit length and so tolerates
> embedded NULs.

> [spec:dash:def:printf.printfcmd-fn]
> int printfcmd(int argc, char *argv[])

> [spec:dash:sem:printf.printfcmd-fn]
> The `printf` builtin. Reset `rval`, consume options with
> `nextopt(nullstr)`, take the first operand as the format — a missing
> one is `error("usage: printf format [arg ...]")` — and point `gargv` at
> the rest.
>
> Then scan the format repeatedly until the arguments are exhausted: the
> outer `do … while (gargv != argv && *gargv)` re-runs the whole format
> while arguments remain *and* at least one was consumed, which is what
> makes `printf '%s\n' a b c` print three lines without looping forever
> on a format that consumes nothing.
>
> Within one pass, walk the format: a `\` introduces an escape, decoded
> by `conv_escape` and written with `out1mem`. A `%%` prints one `%`. Any
> other character prints as is. A `%` starts a conversion specification:
> skip the flags (`#-+ 0`), then the field width — a `*` consumes an
> argument into `array` — then optionally `.` and the precision, likewise.
> A specification with no conversion character is
> `error("missing format character")`.
>
> The specification is then temporarily NUL-terminated after the
> conversion character so it can be passed to the real `printf`, and
> restored afterwards. Dispatch by conversion:
>
> - `b` — the string is printed with escapes expanded; the `b` is
>   temporarily rewritten to `s` so the C `printf` accepts it, and a
>   `\c` in the argument ends the whole builtin.
> - `c` — first character of the next argument.
> - `s` — the next argument as a string.
> - `d`, `i` — a signed integer, with the specification widened by
>   `mklong`.
> - `o`, `u`, `x`, `X` — likewise unsigned.
> - `a`, `A`, `e`, `E`, `f`, `F`, `g`, `G` — a double.
> - anything else — `error("%s: invalid directive", start)`.
>
> Return `rval`: 0 unless a numeric argument was malformed.
