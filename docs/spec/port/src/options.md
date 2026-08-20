# src/options.c, src/options.h

Shell options live in one `char optlist[NOPTS]` array (`NOPTS` = 18),
indexed by position. Three parallel tables give the three views of the
same option, and **all three must stay in the same order**:

| # | `optlist` macro | `optnames[]` | `optletters[]` |
|---|---|---|---|
| 0 | `eflag` | errexit | `e` |
| 1 | `fflag` | noglob | `f` |
| 2 | `Iflag` | ignoreeof | `I` |
| 3 | `iflag` | interactive | `i` |
| 4 | `mflag` | monitor | `m` |
| 5 | `nflag` | noexec | `n` |
| 6 | `sflag` | stdin | `s` |
| 7 | `xflag` | xtrace | `x` |
| 8 | `vflag` | verbose | `v` |
| 9 | `Vflag` | vi | `V` |
| 10 | `Eflag` | emacs | `E` |
| 11 | `Cflag` | noclobber | `C` |
| 12 | `aflag` | allexport | `a` |
| 13 | `bflag` | notify | `b` |
| 14 | `uflag` | nounset | `u` |
| 15 | `nolog` | nolog | (none, 0) |
| 16 | `pipefail` | pipefail | (none, 0) |
| 17 | `debug` | debug | (none, 0) |

The last three have no letter, so they are reachable only through
`-o name`. A value of 2 in `optlist` is a third state meaning "not yet
decided", used during startup so `procargs` can tell an option the user
set from one still at its default; everything still at 2 is zeroed at the
end of `procargs`.

`struct shparam` holds the positional parameters: `nparam` (count,
excluding `$0`), `p` (the vector), `malloc` (whether `p` and its strings
are owned), and `optind`/`optoff`, the `getopts` cursor — `optind` is the
1-based word index and `optoff` the byte offset within that word, or -1
for "start of word".

`argptr` is the shared cursor that `nextopt` walks for builtins;
`optionarg` receives an option's argument and `optptr` the position
within a clustered option word.

**Dash source shape (`options.freeparam-fn`):**

    void freeparam(volatile struct shparam *param)

**Retired C ownership helper (`options.freeparam-fn`):**
> Release a positional-parameter list, but only when `param->malloc` says
> it is owned: free each string, then the vector itself. When `malloc` is
> 0 the vector points into `argv` and must not be freed. The parameter is
> `volatile` because callers hold these across `setjmp`.

**Dash source shape (`options.getopts-fn`):**

    STATIC int getopts(char *optstr, char *optvar, char **optfirst)

> [spec:dash:sem:options.getopts-fn]
> One step of `getopts`. Returns 1 when the options are exhausted and 0
> otherwise; the option letter itself is delivered by assigning it to the
> variable named by `optvar`, and its argument to `OPTARG`. `optfirst`
> points at the first word to scan. The cursor is kept in
> `shellparam.optind`/`optoff` across calls, and is loaded into locals
> `ind`/`off` on entry; `shellparam.optind` is set to -1 for the duration
> so that a `setvar("OPTIND", …)` triggered indirectly cannot be mistaken
> for a user reset.
>
> Resolve the scan position. `optnext = optfirst + ind - 1` is the
> current word. If `ind <= 1`, or `off < 0`, or the previous word is
> shorter than `off`, there is no partially consumed word, so `p` is
> NULL; otherwise `p = optnext[-1] + off` resumes inside it. When `p` is
> NULL or exhausted, advance to `*optnext`: if it is absent, does not
> start with `-`, or is just `-`, the options are over — return with
> `done = 1`. Otherwise step `optnext` past it, and treat an exact `--`
> as the end too.
>
> Take the letter `c = *p++` and look it up in `optstr`, skipping the
> leading `:` if the string starts with one (silent-error mode). Walk `q`
> forward, stepping over a `:` that follows a letter. On reaching the end
> without a match the option is invalid: in silent mode set `OPTARG` to
> the offending letter, otherwise print `"Illegal option -%c"` to
> `errout` and unset `OPTARG`; either way the reported letter becomes `?`.
>
> If the matched letter is followed by `:` it takes an argument. Use the
> rest of the current word if any; otherwise consume the next word. If
> there is none, then in silent mode set `OPTARG` to the letter and
> report `:` — the "missing argument" signal — and otherwise print
> `"No arg for -%c option"`, unset `OPTARG`, and report `?`. When the
> argument came from a following word, step `optnext` past it. Set
> `OPTARG` and clear `p`, since the word is fully consumed. An option
> without an argument sets `OPTARG` to the empty string.
>
> Finally compute `ind = optnext - optfirst + 1` and publish it with
> `setvarint("OPTIND", ind, VNOFUNC)` — `VNOFUNC` suppresses the
> `getoptsreset` callback that would otherwise undo the cursor. Assign
> the one-character result to `optvar`. Store `optoff` as the offset
> within the current word when one is partially consumed, else -1, and
> store `optind`. Return `done`.

**Dash source shape (`options.getoptscmd-fn`):**

    int getoptscmd(int argc, char **argv)

> [spec:dash:sem:options.getoptscmd-fn]
> The `getopts` builtin. Consume options with `nextopt(nullstr)`, then
> re-derive `argc`/`argv` relative to `argptr` so indices below refer to
> the operands: `argv[1]` is the option string and `argv[2]` the variable
> name. Fewer than three requires
> `sh_error("Usage: getopts optstring var [arg...]")`.
>
> With exactly three, scan the positional parameters: `optbase` is
> `shellparam.p`, and if `optind` has run past `nparam + 1` the cursor is
> stale, so reset it to 1/-1. With more, scan the explicit operands from
> `argv[3]` onward, resetting the cursor when `optind` exceeds
> `argc - 2`. The unsigned casts make a negative `optind` compare high
> and also trigger the reset. Then delegate to `getopts` and return its
> value, which is the builtin's exit status: 0 while options remain, 1 at
> the end.

**Dash source shape (`options.getoptsreset-fn`):**

    void getoptsreset(const char *value)

> [spec:dash:sem:options.getoptsreset-fn]
> Change callback on `OPTIND`: reset the cursor to `optind = 1`,
> `optoff = -1`. Any assignment to `OPTIND` therefore restarts option
> parsing, regardless of the value assigned — the `value` argument is
> ignored. `getopts` suppresses this with `VNOFUNC` when it updates
> `OPTIND` itself.

**Dash source shape (`options.minus-o-fn`):**

    STATIC void minus_o(char *name, int val)

> [spec:dash:sem:options.minus-o-fn]
> Handle `-o`/`+o`. With a `name`, find it in `optnames` and set that
> slot to `val`, or raise `sh_error("Illegal option -o %s", name)`.
>
> With no name, print the settings — in a form determined by `val`, which
> is 1 for `-o` and 0 for `+o`. `-o` prints a header
> `"Current option settings"` then one `"%-16s%s"` line per option with
> `on`/`off`. `+o` prints one `set -o name` / `set +o name` line per
> option, which is re-executable input that restores the current state.

**Dash source shape (`options.nextopt-fn`):**

    int nextopt(const char *optstring)

> [spec:dash:sem:options.nextopt-fn]
> `getopt`-style option scanning for builtins, over the global `argptr`.
> Returns the option letter, or `'\0'` when the options end. The
> library `getopt` is avoided because it needs the BSD `optreset`
> extension to be safely reusable inside a long-lived shell.
>
> Resume inside a clustered word when `optptr` is non-NULL and not
> exhausted. Otherwise take `*argptr`: return `'\0'` if it is absent,
> does not begin with `-`, or is exactly `-`. Step `argptr` past it, and
> also return `'\0'` for an exact `--` — which consumes the `--`, so the
> caller's operands start after it.
>
> Take `c = *p++` and search `optstring`, stepping over the `:` that
> marks an argument-taking option; a letter not found raises
> `sh_error("Illegal option -%c", c)`. If the matched letter is followed
> by `:`, take its argument from the rest of the word, or from the next
> word if the current one is exhausted, raising
> `sh_error("No arg for -%c option", c)` if neither exists; store it in
> `optionarg` and clear `p` so the word is not rescanned. Save `p` into
> `optptr` and return `c`.

**Dash source shape (`options.options-fn`):**

    STATIC int options(int cmdline)

> [spec:dash:sem:options.options-fn]
> Parse option words from `argptr`, advancing it past them. `cmdline` is
> non-zero when parsing the shell's own command line and zero for the
> `set` builtin; it enables `-c` and `-l` and changes how `-`/`--` are
> treated. Returns whether `-l` was seen.
>
> For each word: a leading `-` means turn options on (`val = 1`), a
> leading `+` means off (`val = 0`), anything else ends option parsing —
> and steps `argptr` back so the word is seen as an operand.
>
> A bare `-` or `--` terminates options. Off the command line these also
> have effects: `-` alone turns off `xtrace` and `verbose` (the historic
> behaviour), and `--` with no following words resets the positional
> parameters to empty via `setparam(argptr)`. On the command line they
> just terminate.
>
> Otherwise process each letter of the cluster. On the command line, `c`
> takes the rest of the word as the start of the command string into
> `minusc` — note it is not consumed here, the real command text is
> picked up later by `procargs` — and `l` sets the login flag. `o`
> consumes the next word as the long option name and passes it to
> `minus_o`, advancing `argptr` only if a word was actually there. Any
> other letter goes to `setoption(c, val)`.

**Dash source shape (`options.optschanged-fn`):**

    void optschanged(void)

> [spec:dash:sem:options.optschanged-fn]
> Propagate option changes into the subsystems that cache them: reopen
> the trace file under `DEBUG`, `setinteractive(iflag)`, re-configure
> line editing with `histedit()` (unless `SMALL`), and
> `setjobctl(mflag)`. Must be called after anything that writes
> `optlist` wholesale — `set`, `procargs`, and the `$-` restore in
> `poplocalvars`.

**Dash source shape (`options.procargs-fn`):**

    int procargs(char **xargv)

> [spec:dash:sem:options.procargs-fn]
> Process the shell's command line. Returns non-zero for a login shell.
>
> A login shell is one whose `argv[0]` begins with `-`. Record `argv[0]`
> as `arg0` and skip it. Set every `optlist` entry to the sentinel 2, then
> parse options with `options(1)`, OR-ing in its `-l` result. If no words
> remain: `-c` without its command string is
> `sh_error("-c requires an argument")`, and otherwise `sflag` is forced
> on, since the shell will read from standard input.
>
> Resolve the two options still at their sentinel. `iflag` becomes 1 when
> reading from stdin and both stdin and stderr are terminals — checked
> after `input_init()`, which is what determines `stdin_istty`. `mflag`
> (job control) defaults to whatever `iflag` ended up as. Then flatten
> every remaining 2 to 0. Under `DEBUG == 2`, force `debug` on.
>
> Establish `$0` and the positional parameters. With `-c`, POSIX says the
> first word after the command string is `$0` and the rest are `$1…`, so
> take the command string from the next word and, if any words remain,
> take `$0` from the following one. Without `-c` and without `-s`, the
> first word names a script: open it with `setinputfile(*xargv, 0)` and
> use it as `$0`. Point `shellparam.p` at what is left, set `optind = 1`
> and `optoff = -1`, and count the words into `nparam`. The vector is not
> copied, so `malloc` stays 0. Finish with `optschanged()`.

**Dash source shape (`options.setcmd-fn`):**

    int setcmd(int argc, char **argv)

> [spec:dash:sem:options.setcmd-fn]
> The `set` builtin. With no arguments, print all set variables with
> `showvars(nullstr, 0, VUNSET)` and return that. Otherwise, with
> interrupts suspended: parse options with `options(0)`, apply them with
> `optschanged()`, and if any operands remain replace the positional
> parameters with `setparam(argptr)`. Return 0. Note that `set --` with
> no operands is handled inside `options`, which is why it clears the
> parameters rather than leaving them alone here.

**Dash source shape (`options.setoption-fn`):**

    STATIC void setoption(int flag, int val)

> [spec:dash:sem:options.setoption-fn]
> Set the option whose letter is `flag` to `val`, raising
> `sh_error("Illegal option -%c", flag)` if no letter matches. When
> turning an option *on*, enforce that the two line-editing modes are
> mutually exclusive, ksh-style: `-V` (vi) clears `Eflag` and `-E`
> (emacs) clears `Vflag`. Turning one off does not enable the other.

**Dash source shape (`options.setparam-fn`):**

    void setparam(char **argv)

> [spec:dash:sem:options.setparam-fn]
> Replace the positional parameters with copies of `argv`. Count the
> entries, allocate a vector of `nparam + 1`, `savestr` each string into
> it, and NULL-terminate. Release the previous list with `freeparam`,
> then install the new one with `malloc = 1` (it is owned),
> `nparam`, `p`, and the `getopts` cursor reset to 1/-1. Copying is
> required because `argv` frequently points into storage that is about to
> be reclaimed.

**Dash source shape (`options.shiftcmd-fn`):**

    int shiftcmd(int argc, char **argv)

> [spec:dash:sem:options.shiftcmd-fn]
> The `shift` builtin. The count is 1 by default, or `number(argv[1])`
> when given. Shifting more than `nparam` raises
> `sh_error("can't shift that many")`; note `number` rejects negatives,
> so `n` is always in range afterwards. With interrupts suspended, reduce
> `nparam` by `n`, free the first `n` strings if the list is owned, then
> move the remaining pointers — including the NULL terminator — down to
> the front. Reset the `getopts` cursor to 1/-1. Return 0.

**Dash source shape (`options.shparam`):**

    struct shparam {
      int nparam;
      unsigned char malloc;
      char **p;
      int optind;
      int optoff;
    }
