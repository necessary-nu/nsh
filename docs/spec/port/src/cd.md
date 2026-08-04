# src/cd.c

Directory tracking keeps two module-private strings, both initialised to
`nullstr`: `curdir`, the shell's logical current directory (the one
`PWD` reports, with symlinks preserved), and `physdir`, the physical one
as reported by `getcwd`. `physdir` is computed lazily and reset to
`nullstr` whenever `curdir` changes. Two flag bits control behaviour:
`CD_PHYSICAL` (1) selects `-P` semantics, `CD_PRINT` (2) requests that
the resulting directory be echoed.

> [spec:dash:def:cd.cdcmd-fn]
> int cdcmd(int argc, char **argv)

> [spec:dash:sem:cd.cdcmd-fn]
> The `cd` builtin. Parse `-L`/`-P` with `cdopt` to get `flags`, then
> pick the destination from the first operand `*argptr`. With no operand
> use `bltinlookup(homestr)` (`HOME`). With the exact operand `-` use
> `bltinlookup("OLDPWD")` and set `CD_PRINT`, so `cd -` echoes where it
> landed. If the chosen value is still NULL (the variable is unset),
> substitute `nullstr` — the empty destination is then handled below
> rather than being an error here.
>
> Decide whether `CDPATH` applies. It does not for a destination that is
> absolute (`*dest == '/'`) or that begins with a `.` component: examine
> `dest[1]`, and treat `\0` or `/` as "this is a `.` or `./…` path" and
> jump to step 6; for `.` re-examine `dest[2]` and loop once more so that
> `..` and `../…` are also step 6, while `...` and longer runs of dots
> fall through and are ordinary names. This mirrors POSIX `cd` step 5,
> which bypasses `CDPATH` only for paths starting with `.` or `..`
> components.
>
> Otherwise search `CDPATH`. An empty destination first becomes `dotdir`
> (`"."`). Loop with `p = path` saved before each
> `padvance_magic(&path, dest, 0)` call, which appends `dest` to the next
> `CDPATH` entry on the stack and returns the length, negative when the
> list is exhausted. Remember `c = *p`, the first character of the entry
> just consumed, then claim the built path with `stalloc(len)`. If
> `stat64` succeeds and `S_ISDIR`, this entry wins: if `c` is neither NUL
> nor `:` — i.e. the matching `CDPATH` entry was not the empty/current
> directory — set `CD_PRINT`, since POSIX requires echoing when the
> destination came from a non-trivial `CDPATH` element. Then attempt
> `docd(p, flags)`; on success go to the output step, and on failure
> raise the error rather than continuing the search — the first matching
> directory is the only one tried.
>
> Step 6 (reached directly, or after the `CDPATH` loop runs out) attempts
> `docd(dest, flags)` on the destination as given. On failure raise
> `sh_error("can't cd to %s", dest)`, which does not return. On success,
> if `CD_PRINT` is set write `curdir` followed by a newline to `out1`.
> Return 0.

> [spec:dash:def:cd.cdopt-fn]
> STATIC int cdopt()

> [spec:dash:sem:cd.cdopt-fn]
> Parse the `-L`/`-P` options shared by `cd` and `pwd`, returning
> `CD_PHYSICAL` if physical semantics win and 0 otherwise. Start with
> `flags = 0` and `j = 'L'`, the implied default. For each option letter
> `i` from `nextopt("LP")`, toggle `flags ^= CD_PHYSICAL` and record
> `j = i` only when `i` differs from the previous letter `j`. Repeating
> the same option is therefore idempotent while alternating them flips
> the mode each time, so the last distinct option given wins — `-L -P`
> yields physical, `-P -P` yields physical, `-P -L` yields logical.

> [spec:dash:def:cd.docd-fn]
> STATIC int docd(const char *dest, int flags)

> [spec:dash:sem:cd.docd-fn]
> Perform the actual directory change and keep the shell's bookkeeping in
> step. Trace the call under `DEBUG`. With interrupts suspended: unless
> `CD_PHYSICAL` is set, compute the logical path with `updatepwd(dest)`
> and, if that returned non-NULL, chdir to it instead of the raw
> destination — this is what makes `cd ..` out of a symlinked directory
> return to the logical parent. Call `chdir`; on error skip the rest and
> return its non-zero result unchanged, leaving `curdir` and `physdir`
> untouched. On success call `setpwd(dir, 1)` — passing the computed
> logical path, or NULL when `-P` was used, which makes `setpwd` derive
> the name from `getcwd` — with `setold` non-zero so `OLDPWD` is updated.
> Then `hashcd()` to invalidate the command hash table's relative-path
> entries, since a different directory may now shadow hashed commands.
> Restore interrupts and return the `chdir` result.

> [spec:dash:def:cd.getpwd-fn]
> inline STATIC char * getpwd()

> [spec:dash:sem:cd.getpwd-fn]
> Return a freshly allocated string holding the physical current
> directory. Under glibc call `getcwd(0, 0)` and return its malloc'd
> result directly if non-NULL. Otherwise call `getcwd` into a `PATH_MAX`
> stack buffer and return `savestr(buf)` on success. If the call fails,
> warn with `sh_warnx("getcwd() failed: %s", strerror(errno))` and return
> `nullstr` — the shared empty string, which callers detect by pointer
> identity, not by content, and must never free.

> [spec:dash:def:cd.pwdcmd-fn]
> int pwdcmd(int argc, char **argv)

> [spec:dash:sem:cd.pwdcmd-fn]
> The `pwd` builtin. Parse `-L`/`-P` with `cdopt`. Default to reporting
> `curdir`, the logical directory. If physical was selected, first ensure
> `physdir` is populated — when it is still `nullstr` call
> `setpwd(curdir, 0)`, which recomputes it via `getpwd` without touching
> `OLDPWD` — then report `physdir`. Write the chosen string followed by a
> newline to `out1` and return 0.

> [spec:dash:def:cd.setpwd-fn]
> void setpwd(const char *val, int setold)

> [spec:dash:sem:cd.setpwd-fn]
> Install a new current directory and publish it to the environment.
> Take `oldcur = dir = curdir`. If `setold` is non-zero, export the
> outgoing directory first: `setvar("OLDPWD", oldcur, VEXPORT)`.
>
> With interrupts suspended, discard any cached physical directory: if
> `physdir` is not `nullstr`, free it unless it aliases `oldcur` (in
> which case the single allocation is still owned by `curdir` and is
> freed below), and reset `physdir = nullstr`. Then decide the new value.
> If `val` is NULL, or `val` aliases the existing `curdir`, call
> `getpwd()` and store the result in `physdir`; when `val` was NULL also
> adopt that string as `dir`, which is the `-P` path where only the
> kernel knows where we landed. Otherwise `dir = savestr(val)`. Free the
> old string if it was replaced and was not the shared `nullstr`, assign
> `curdir = dir`, and restore interrupts.
>
> Finally `setvar("PWD", dir, VEXPORT)` outside the critical section.

> [spec:dash:def:cd.updatepwd-fn]
> STATIC const char * updatepwd(const char *dir)

> [spec:dash:sem:cd.updatepwd-fn]
> Compute the logical path that results from moving to `dir`, resolving
> `.` and `..` textually against `curdir` without consulting the
> filesystem, and return it on the stack (the caller must use it before
> the next stack reset). Returns NULL if `dir` is relative but `curdir`
> is unknown (`nullstr`), which tells `docd` to chdir to the raw
> destination instead. On Cygwin, `dir` is first normalised to POSIX form
> with `cygwin_conv_path`, raising `sh_error("can't normalize %s", dir)`
> on failure, because absolute Windows paths need not start with `/`.
>
> Copy `dir` to the stack as `cdcomppath` for destructive tokenising, and
> start a fresh stack string `new`. For a relative `dir`, seed `new` with
> `curdir` (returning NULL first if it is `nullstr`). Reserve
> `strlen(dir) + 2` bytes and set `lim = stackblock() + 1` — the floor
> that `..` may never erase past, which is why the leading `/` survives.
> For a relative `dir`, append `/` unless the seed already ends in one,
> and if the buffer holds more than the root and `*lim` is `/` advance
> `lim` so the root slash is protected. For an absolute `dir`, emit `/`
> and skip the corresponding character of `cdcomppath`; additionally, if
> `dir` begins with exactly two slashes (`dir[1] == '/' && dir[2] != '/'`)
> emit a second `/`, skip another character and advance `lim`, preserving
> the POSIX implementation-defined `//` prefix while collapsing three or
> more slashes to one.
>
> Then tokenise `cdcomppath` on `/` with `strtok` and fold each component
> into `new`: a component of exactly `..` pops the last component by
> unputting characters while `new > lim`, stopping once the character
> just below is `/`; a component of exactly `.` is dropped; anything else
> (including `...` and names merely starting with `.`) is appended
> followed by `/`. Empty components never appear because `strtok`
> coalesces runs of separators, so embedded `//` collapses.
>
> After the loop remove the trailing `/` if anything beyond `lim` was
> written, NUL-terminate, and return `stackblock()`.
