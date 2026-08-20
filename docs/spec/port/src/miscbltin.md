# src/miscbltin.c

The `read`, `umask` and `ulimit` builtins. The `ulimit` half is compiled
only under `HAVE_GETRLIMIT` and is driven by a table of `struct limits`,
each entry naming a resource, its `RLIMIT_*` code, the unit multiplier
(`factor`) that converts between the user-visible number and the kernel
value, and the option letter that selects it. The table is
`#ifdef`-guarded per resource, so its contents and the accepted option
set vary by platform, and it is terminated by an entry with a NULL
`name`. `enum limtype` distinguishes the soft (`SOFT`, 0x1) and hard
(`HARD`, 0x2) limits; both bits set means "act on both", which is the
`ulimit` default.

Note the file `#undef`s `rflag` so `readcmd` can use that name for its
own local, rather than the global shell option of the same name.

**Dash source shape (`miscbltin.limits`):**

    struct limits {
      const char *name;
      int cmd;
      int factor;
      char option;
    }

**Dash source shape (`miscbltin.limtype`):**

    enum limtype {
      SOFT = 0x1;
      HARD = 0x2;
    }

**Dash source shape (`miscbltin.printlim-fn`):**

    static void printlim(enum limtype how, const struct rlimit *limit, const struct limits *l)

> [spec:dash:sem:miscbltin.printlim-fn]
> Print one limit value and a newline. Select `limit->rlim_cur` when
> `how` has `SOFT` set and `limit->rlim_max` otherwise — so with both
> bits set the soft limit wins. Print the literal `"unlimited"` for
> `RLIM_INFINITY`; otherwise divide by `l->factor` (integer division, so
> a value not a whole number of units is truncated) and print it as a
> decimal `intmax_t`.

**Dash source shape (`miscbltin.readcmd-fn`):**

    int readcmd(int argc, char **argv)

> [spec:dash:sem:miscbltin.readcmd-fn]
> The `read` builtin. Parse options with `nextopt("p:r")`: `-p` takes an
> argument used as a prompt, `-r` sets the local `rflag`, which disables
> backslash escape processing. If a prompt was given and stdin is a tty,
> write it to `out2` (flushing when `FLUSHERR` is configured). Require at
> least one variable name in `argptr`, raising `sh_error("arg count")`
> otherwise. Initialise `status` to 0, start a stack string `p`, and
> `pushstdin()` to read unbuffered directly from fd 0 — so the shell does
> not consume input beyond the line, which matters when the script itself
> is on stdin.
>
> The loop accumulates one logical line onto the stack while tracking two
> offsets: `startloc`, the start of the current unquoted region, and
> `newloc`, which marks where a pending backslash was seen. Entry jumps
> straight to the `start` label, which sets `startloc` to the current
> offset and `newloc = startloc - 1`, i.e. "no pending backslash".
>
> Each iteration reserves room for at least
> `max(MB_LEN_MAX, 16) + 4` bytes and reads a character with `pgetc()`.
> `PEOF` sets `status = 1` and ends the loop — this is how `read` reports
> end of input. A NUL byte is silently dropped. Otherwise `getmbc(c, p, 0)`
> tries to complete a multibyte character at `p`; a non-zero return means
> it consumed one, so advance `p` by that many bytes and go to the record
> step without further interpretation.
>
> For a single byte: if `newloc >= startloc` a backslash is pending, so
> the character is escaped — a `\n` there is a line continuation and
> reaches the record step (dropping both characters), anything else is
> emitted literally. Otherwise, when `rflag` is clear, a `\` sets
> `newloc` to the current offset and continues, deferring the decision;
> and a `\n` ends the line. Emitting a character prefixes it with
> `CTLESC` when it appears in `qchars`, protecting it from later field
> splitting and pathname expansion.
>
> The record step, reached after a completed character, calls
> `recordregion(startloc, newloc, 0)` when a backslash was pending —
> marking the text before it as a quoted region — then restarts the
> region bookkeeping at the current offset.
>
> After the loop: `popfile()` to undo `pushstdin`, record the final
> region up to the current offset, NUL-terminate the stack string, and
> hand off to `readcmd_handle_line(p + 1, argc - (ap - argv), ap)` —
> `p + 1` because `STACKSTRNUL` leaves `p` at the terminator, and the
> count is the number of remaining variable names. Return `status`: 0
> normally, 1 if end of input was reached before a newline.

**Dash source shape (`miscbltin.readcmd-handle-line-fn`):**

    static void readcmd_handle_line(char *s, int ac, char **ap)

> [spec:dash:sem:miscbltin.readcmd-handle-line-fn]
> Split one input line into `IFS` fields and assign them to the variable
> names in `ap`. Claim the stack string with `grabstackstr(s)`, initialise
> an empty `struct arglist`, and split with `ifsbreakup(s, ac, &arglist)`
> — passing `ac`, the number of variables, so that once that many fields
> have been produced the remainder of the line is left in the last one,
> as POSIX requires. Terminate the list and release the IFS working state
> with `ifsfree()`.
>
> Then walk variables and fields in lockstep. When the fields run out
> before the variables do, assign `nullstr` to every remaining variable
> and return — trailing variables are set to empty, not left unset.
> Otherwise `rmescapes` the field (removing the `CTLESC` bytes `readcmd`
> inserted) and `setvar(*ap, sl->text, 0)`. The loop is do/while on
> `*++ap`, so at least one assignment always happens.

**Dash source shape (`miscbltin.ulimitcmd-fn`):**

    int ulimitcmd(int argc, char **argv)

> [spec:dash:sem:miscbltin.ulimitcmd-fn]
> The `ulimit` builtin. Default `what` to `'f'` (file size) and `how` to
> `SOFT | HARD`. Parse options with a `nextopt` string built from `"HSa"`
> plus one letter per resource the platform supports: `-H` selects the
> hard limit, `-S` the soft one, `-a` requests every limit, and any other
> letter selects that resource into `what`. Later `-H`/`-S` overwrite
> earlier ones rather than combining.
>
> Find the table entry whose `option` equals `what`. The search is
> unbounded — it relies on `nextopt` having already rejected letters not
> in the option string, so a mismatch cannot occur.
>
> Determine whether this is a set or a query from whether `argptr` holds
> an operand. To set: reject `-a` together with an operand, or more than
> one operand, with `sh_error("too many arguments")`. The literal
> `unlimited` becomes `RLIM_INFINITY`; otherwise accumulate decimal
> digits into `val`, with a `val < (rlim_t)0` test that reads as a crude
> overflow guard but is **dead code**: `rlim_t` is unsigned, so the
> comparison is never true and the accumulator wraps silently. Reproduce
> the dead test. Raise `sh_error("bad number")` if any
> non-digit remains. Multiply the result by the entry's `factor` to
> convert from user units to kernel units.
>
> With `-a`, walk the whole table, `getrlimit` each entry, print its
> `name` left-justified in 20 columns followed by a space, and
> `printlim` its value; return 0.
>
> Otherwise `getrlimit` the selected resource. When setting, assign `val`
> to `rlim_max` if `how` has `HARD` and to `rlim_cur` if it has `SOFT` —
> so the default of both bits sets both — and `setrlimit`, raising
> `sh_error("error setting limit (%s)", strerror(errno))` on failure.
> When querying, `printlim`. Return 0.

**Dash source shape (`miscbltin.umaskcmd-fn`):**

    int umaskcmd(int argc, char **argv)

> [spec:dash:sem:miscbltin.umaskcmd-fn]
> The `umask` builtin. Parse `-S` with `nextopt("S")` to select symbolic
> output. Read the current mask by the usual dance — `umask(0)` then
> `umask(mask)` to restore it — with interrupts suspended so an interrupt
> cannot leave the process with a zero mask.
>
> With no operand, print it. In symbolic mode, work from `~mask` (the
> *permitted* bits) and build a string like `u=rwx,g=rx,o=rx` into an
> 18-byte buffer: for each of `u`, `g`, `o` emit the letter and `=`, then
> for each of `r`, `w`, `x` emit the letter if bit `8 - (3*i + j)` is set,
> then a `,`; finally overwrite the trailing comma with NUL. Otherwise
> print the mask as `%.4o` — zero-padded octal — followed by a newline.
>
> With an operand, parse it two ways. If it starts with a digit it is
> octal: accumulate `new_mask = (new_mask << 3) + digit`, raising
> `sh_error(illnum, *argptr)` on any character outside `0`–`7`.
>
> Otherwise it is a symbolic mode, and — as in `chmod` — it is applied to
> the *permission* bits, so the code inverts the mask (`mask = ~mask`),
> works in permission space, and inverts back at the end. Parse
> comma-separated clauses: a `who` part built from `a` (0111), `u`
> (0100), `g` (0010), `o` (0001), defaulting to 0111 when empty; an
> operator from `=`, `+`, `-`, with a missing operator being an error and
> any other character ending the parse; and a `perm` part from `r` (04),
> `w` (02), `x` (01), the copy-from letters `u`/`g`/`o` (which take the
> existing bits shifted by 6, 3 and 0), `X` (sets execute only if any
> execute bit is already set anywhere in `mask & 0111`), and `s` (parsed
> and ignored, since the shell's umask has no setuid bit). Replicate the
> three permission bits across the selected `who` positions with
> `(new_val & 07) * positions`, then apply: `-` clears those bits, `+`
> sets them, `=` replaces exactly the selected positions, preserving the
> others. A following `,` resets the `who` accumulator for the next
> clause; anything that is not another operator ends the parse.
>
> If unconsumed text remains, raise `sh_error("Illegal mode: %s",
> *argptr)`. Otherwise invert back to a mask and `umask(new_mask)`.
> Return 0.
