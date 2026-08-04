# src/exec.c, src/exec.h

Command resolution and the command hash table. `cmdtable` is
`CMDTABLESIZE` = 31 hash chains of `struct tblentry`, each caching what a
name resolved to. `cmdtype` is `CMDUNKNOWN` (-1), `CMDNORMAL` (0, an
executable found on `PATH`, with `param.index` recording *which* `PATH`
element it came from), `CMDFUNCTION` (1, `param.func`) or `CMDBUILTIN`
(2, `param.cmd`). `rehash` marks an entry as needing revalidation after a
`cd`. The trailing `cmdname[ARB]` is a flexible array — `ARB` is 1 and
the real size is chosen at allocation, so the name lives in the same
block as the entry.

`builtinloc` is the index of the `%builtin` element within `PATH`, or -1
if absent; it determines whether a non-special builtin can be shadowed by
an executable earlier on the path.

`find_command`'s `act` argument is a mask: `DO_ERR` (0x01, report
failures), `DO_ABS` (0x02, stat absolute paths rather than assuming),
`DO_NOFUNC` (0x04, ignore shell functions — used by `command`),
`DO_ALTPATH` (0x08, `path` is not `PATH`), `DO_REGBLTIN` (0x10, accept
only regular builtins).

`PATH` elements may carry a `%option` suffix: `%builtin` marks the point
in the search at which builtins are considered, and `%func` marks a
directory of files defining shell functions.

> [spec:dash:def:exec.addcmdentry-fn]
> STATIC void addcmdentry(char *name, struct cmdentry *entry)

> [spec:dash:sem:exec.addcmdentry-fn]
> Install `entry` under `name`, creating the table entry if needed. If the
> existing entry was a function, release it with `freefunc` first — this
> is what makes redefining a function free the old body. Copy `cmdtype`
> and the `param` union, and clear `rehash` since the entry is fresh.

> [spec:dash:def:exec.changepath-fn]
> void changepath(const char *newval)

> [spec:dash:sem:exec.changepath-fn]
> `PATH` change callback, invoked *before* the variable is updated, so
> `pathval()` still returns the old value. Must be called with interrupts
> off. Scan `newval` for a colon-separated element beginning `%builtin`,
> recording its index in `builtinloc` (-1 when absent), then
> `clearcmdentry()` to drop every cached resolution that the new path
> could change.

> [spec:dash:def:exec.clearcmdentry-fn]
> STATIC void clearcmdentry(void)

> [spec:dash:sem:exec.clearcmdentry-fn]
> Drop cached entries that a `PATH` change may invalidate. With
> interrupts suspended, walk every chain and unlink and free entries that
> are `CMDNORMAL`, or that are `CMDBUILTIN` for a non-regular builtin
> while `builtinloc > 0` — such a builtin sits behind at least one `PATH`
> element and could now be shadowed by an executable. Functions and
> regular builtins are never affected by `PATH` and are kept.

> [spec:dash:def:exec.cmdentry]
> struct cmdentry {
>   int cmdtype;
>   union param { int index; const struct builtincmd *cmd; struct funcnode *func; } u;
> }

> [spec:dash:def:exec.cmdentry.param]
> union param {
>   int index;
>   const struct builtincmd *cmd;
>   struct funcnode *func;
> }

> [spec:dash:def:exec.cmdlookup-fn]
> tblentry * cmdlookup(const char *name, int add)

> [spec:dash:sem:exec.cmdlookup-fn]
> Find `name` in the command table, optionally creating it. Hash by
> seeding with the first byte shifted left 4 and adding every byte
> (including, unlike `hashval` for variables, all of them — there is no
> `=` to stop at), then mask with `0x7FFF` to keep it positive and reduce
> modulo `CMDTABLESIZE`. Walk the chain comparing with `equal`.
>
> When `add` is non-zero and nothing matched, allocate an entry sized
> `sizeof(struct tblentry) - ARB + strlen(name) + 1` so the name fits in
> the flexible tail, link it at the end of the chain, mark it
> `CMDUNKNOWN`, and copy the name in. Interrupts must be off in that
> case.
>
> Either way, set the module-global `lastcmdentry` to the address of the
> link that points at the result, so `delete_cmd_entry` can unlink it
> without searching again. Returns NULL when not found and `add` is 0.

> [spec:dash:def:exec.commandcmd-fn]
> int commandcmd(int argc, char **argv)

> [spec:dash:sem:exec.commandcmd-fn]
> The `command` builtin, in its describing role. Parse `-p`, `-v`, `-V`:
> `-V` sets `VERIFY_VERBOSE` (2), `-v` sets `VERIFY_BRIEF` (1), and `-p`
> replaces the search path with `defpath`, the standard PATH. Under
> `DEBUG` an unexpected letter `abort()`s, since `nextopt` should have
> rejected it.
>
> If a verify option and an operand are both present, describe it with
> `describe_command(out1, cmd, path, verify - VERIFY_BRIEF)` — so `-v`
> passes verbose 0 and `-V` passes 1 — and return that result. Otherwise
> return 0.
>
> Note that `command` *executing* a command (its main purpose) is not
> handled here: `eval.c` recognises it earlier and applies `DO_NOFUNC`
> and the alternate path directly.

> [spec:dash:def:exec.defun-fn]
> void defun(union node *func)

> [spec:dash:sem:exec.defun-fn]
> Define a shell function. With interrupts suspended, build a
> `struct cmdentry` of type `CMDFUNCTION` whose `func` is
> `copyfunc(func)` — a reference-counted deep copy, since the parse tree
> it came from will be reclaimed — and install it under
> `func->ndefun.text`.

> [spec:dash:def:exec.delete-cmd-entry-fn]
> STATIC void delete_cmd_entry(void)

> [spec:dash:sem:exec.delete-cmd-entry-fn]
> Remove the entry found by the most recent `cmdlookup`, using the saved
> `lastcmdentry` link address. With interrupts suspended, unlink it,
> `freefunc` its body if it was a function, and free the entry. Calling
> this without a preceding successful `cmdlookup` is undefined.

> [spec:dash:def:exec.describe-command-fn]
> static int describe_command(struct output *out, char *command, const char *path, int verbose)

> [spec:dash:sem:exec.describe-command-fn]
> Back end of `type` and `command -v`/`-V`. `verbose` selects `type`-style
> prose over bare-path output. Returns 0, or 127 if the name is unknown.
> In verbose mode the name is echoed first, so each branch appends a
> description to it.
>
> Check in order: shell keywords (`findkwd`) print `" is a shell keyword"`
> or just the name; aliases (`lookupalias(command, 0)`) print
> `" is an alias for <value>"`, or in brief mode `alias ` followed by
> `printalias` and an immediate return with no trailing newline of its
> own.
>
> Then resolve. When `path` is NULL the standard search is in use, so
> consult the hash table first — a hit there is a *tracked alias* and is
> reported as such. Otherwise force a fresh `find_command(command, &entry,
> DO_ABS, path)`; `DO_ABS` makes it verify that an absolute name actually
> exists and is executable rather than assuming.
>
> Report by type. `CMDNORMAL` re-derives the full path by replaying
> `padvance` `index + 1` times over `path` — the loop is
> `do { ... } while (--j >= 0)`, as `[spec:dash:sem:exec.printentry-fn]`
> states correctly for the identical code (index -1 means the name already
> contained a slash and is used as-is), printing `" is[ a tracked alias
> for] <path>"` or the bare path. `CMDFUNCTION` prints
> `" is a shell function"` or the name. `CMDBUILTIN` prints
> `" is a [special ]shell builtin"` or the name. Anything else prints
> `": not found\n"` in verbose mode and returns 127 — note that branch
> emits its own newline and skips the shared one.
>
> All other paths finish with a newline and return 0.

> [spec:dash:def:exec.find-builtin-fn]
> struct builtincmd * find_builtin(const char *name)

> [spec:dash:sem:exec.find-builtin-fn]
> Binary-search the generated `builtincmd[]` table (`NUMBUILTINS` entries,
> sorted by name) for `name`, using `pstrcmp`. Returns the entry or NULL.
> The table is produced at build time by `mkbuiltins` from
> `builtins.def.in`.

> [spec:dash:def:exec.find-command-fn]
> void find_command(char *name, struct cmdentry *entry, int act, const char *path)

> [spec:dash:sem:exec.find-command-fn]
> Resolve `name` to a function, builtin or executable, filling `entry`.
> Must be kept behaviourally in step with `shellexec`, which repeats the
> search when actually executing.
>
> **Names containing `/`** bypass both `PATH` and the hash table: set
> `index` to -1 and report `CMDNORMAL`. With `DO_ABS`, first `stat64` the
> name (retrying on `EINTR` under SYSV) and require `test_exec`; failure
> gives `CMDUNKNOWN`.
>
> **Table caching** applies only when `path` is the real `pathval()`;
> otherwise `updatetbl` is 0 and `DO_ALTPATH` is added, so nothing is
> cached under a temporary path. On a table hit, check the cached kind
> against `act`: a `CMDNORMAL` entry is unusable under `DO_ALTPATH` or
> `DO_REGBLTIN`, a `CMDFUNCTION` under `DO_NOFUNC`, and a non-regular
> `CMDBUILTIN` under `DO_REGBLTIN`. A conflict on `DO_REGBLTIN`
> specifically fails outright; other conflicts just disable caching and
> force a fresh search. A usable entry with `rehash == 0` is returned
> immediately.
>
> That `switch` on `cmdp->cmdtype` has a `default:` arm whose only
> statement is `abort()` under `DEBUG`. In a release build the arm is
> empty and falls through into `CMDNORMAL`, so an entry with an
> unrecognised type is treated as an executable. Reproduce the
> fall-through.
>
> **Builtins** come next, but only if they are regular, or an alternate
> path is in use, or `%builtin` is not present later in `PATH`
> (`builtinloc <= 0`) — that is what lets a `PATH` entry before
> `%builtin` shadow a non-special builtin. `DO_REGBLTIN` with no builtin
> match fails here.
>
> **The path search** walks `padvance`, tracking `idx`, the element
> number. When rehashing, `prev` is the previously recorded element so
> absolute entries at or before it can be skipped — reaching exactly
> `prev` means nothing changed and the old answer stands.
>
> Within the loop, a `%builtin` element (`*lpathopt == 'b'`) accepts the
> builtin if one was found and is otherwise skipped; a `%func` element is
> honoured unless `DO_NOFUNC`; any other option is ignored, skipping the
> element. `stat64` failures record the errno unless it is `ENOENT` or
> `ENOTDIR`, which are the uninteresting "not here" cases, and move on. A
> `%func` hit claims the stack space, `readcmdfile`s the file, and
> requires that the name now be defined as a function — raising
> `sh_error("%s not defined in %s", …)` otherwise. An ordinary hit must
> pass `test_exec`, with the default error becoming `EACCES` from that
> point so an unexecutable match reports permission denied rather than
> not-found.
>
> On success the result is either written straight into `entry` (when not
> caching) or stored into the table with `cmdlookup(name, 1)` under
> interrupts-off. On failure, delete any stale table entry, and with
> `DO_ERR` warn `"<name>: <errmsg>"`. The shared success path clears
> `rehash` and copies `cmdtype` and `param` into `entry`.

> [spec:dash:def:exec.getcmdentry-fn]
> void getcmdentry(char *name, struct cmdentry *entry)

> [spec:dash:sem:exec.getcmdentry-fn]
> Read the table entry for `name` into `entry` without searching `PATH`:
> copy `param` and `cmdtype` on a hit, or set `CMDUNKNOWN` with
> `index = 0` on a miss.
>
> The whole function is inside `#ifdef notdef` and is not compiled; Wave 2
> need not port it.

> [spec:dash:def:exec.hashcd-fn]
> void hashcd(void)

> [spec:dash:sem:exec.hashcd-fn]
> Called after a successful `cd`. Set `rehash` on every entry whose
> resolution could depend on the current directory — the same predicate
> `clearcmdentry` uses (`CMDNORMAL`, or a non-regular `CMDBUILTIN` with
> `builtinloc > 0`). Marking rather than deleting lets `find_command`
> revalidate lazily and skip re-searching absolute path elements, which
> cannot have changed.

> [spec:dash:def:exec.hashcmd-fn]
> int hashcmd(int argc, char **argv)

> [spec:dash:sem:exec.hashcmd-fn]
> The `hash` builtin. With `-r`, `clearcmdentry()` and return 0. With no
> operands, print every `CMDNORMAL` entry with `printentry` and return 0.
>
> Otherwise, for each operand: if a cached entry exists and is of a kind
> `PATH` can invalidate (the same predicate as `clearcmdentry`), delete it
> so the lookup is genuinely fresh; then `find_command(name, &entry,
> DO_ERR, pathval())`, which repopulates the table and reports failures.
> Return 1 if any operand was unresolvable, else 0.

> [spec:dash:def:exec.legal-pathopt-fn]
> static const char *legal_pathopt(const char *opt, const char *term, int magic)

> [spec:dash:sem:exec.legal-pathopt-fn]
> Decide whether the text at `opt` (just past a `%`) is a recognised path
> option, and return the position where the option text ends — or NULL if
> it is not one. `magic` selects the dialect: 0 disables option
> recognition entirely (returns NULL); 1 accepts only `builtin` and
> `func`, returning the position after the matched word via `prefix`;
> anything else accepts any text, ending at the first character in `term`.
> Finally, a `%` at the resulting position is stepped over, so
> back-to-back options are handled.

> [spec:dash:def:exec.padvance-fn]
> static inline int padvance(const char **path, const char *name)

> [spec:dash:sem:exec.padvance-fn]
> `padvance_magic(path, name, 1)` — the ordinary path walk, recognising
> only `%builtin` and `%func`.

> [spec:dash:def:exec.padvance-magic-fn]
> int padvance_magic(const char **path, const char *name, int magic)

> [spec:dash:sem:exec.padvance-magic-fn]
> Produce the next candidate path for `name` by consuming one element of
> `*path`, advancing `*path` past it. Returns the length of the buffer
> reserved (including the NUL), or -1 when the list is exhausted —
> signalled by `*path` being NULL, which is also how a list ending
> *without* a trailing colon terminates. The result is built at
> `stackblock()`; the caller must consume or `stalloc` it before the next
> call. The global `pathopt` is set to the element's `%option` text, or
> NULL.
>
> Parsing: with `magic` non-zero, an element starting with `%` may be
> entirely an option, in which case the option text is recorded and the
> directory part starts after it, and the terminator set narrows from
> `"%:"` to `":"` — so a second `%` in the same element is then literal.
> Otherwise take the directory up to the first character in `term`. If
> that stopped at a `%`, the remainder up to the next `:` is examined:
> when `legal_pathopt` accepts it, it becomes `pathopt`; when it does not,
> it is folded back into the directory name, so a directory whose name
> genuinely contains `%` still works.
>
> Then build the candidate: reserve `len + strlen(name) + 2` bytes (the
> 2 covers the `/` and the NUL), and when the directory part is non-empty
> copy it followed by `/`. An *empty* element therefore produces `name`
> alone, which is the POSIX meaning of an empty `PATH` element — the
> current directory. Return the reserved length.
>
> Note the returned length is the size *reserved*, not `strlen` of the
> result, and `mail.c` relies on the trailing `/` when it passes an empty
> `name`.

> [spec:dash:def:exec.printentry-fn]
> STATIC void printentry(struct tblentry *cmdp)

> [spec:dash:sem:exec.printentry-fn]
> Print one hashed command as its full path. Replay `padvance` over
> `pathval()` `index + 1` times to rebuild the path element the entry came
> from, take the result from `stackblock()`, and print it followed by
> `*` if the entry is marked for rehashing, else nothing, and a newline.

> [spec:dash:def:exec.shellexec-fn]
> void shellexec(char **argv, const char *path, int idx)

> [spec:dash:sem:exec.shellexec-fn]
> Replace this process with the command in `argv`. Never returns. `idx`
> is the `PATH` element index `find_command` recorded, so the search can
> skip straight to it.
>
> Build the environment with `environment()`. A name containing `/` is
> exec'd directly. Otherwise walk `padvance`, and for each element
> decrement `idx`; only once it goes negative, and only for elements
> without a `%option`, attempt the exec. Record the errno of each failed
> attempt unless it is `ENOENT` or `ENOTDIR`, so a genuine permission
> error is reported in preference to a not-found from a later element.
>
> When everything fails, map the errno to a POSIX exit status: 127 for
> `ELOOP`, `ENAMETOOLONG`, `ENOENT` and `ENOTDIR` ("not found"), 126 for
> everything else ("found but not executable"). Set `exitstatus` and
> raise `exerror(EXEND, "%s: %s", argv[0], errmsg(e, E_EXEC))`.

> [spec:dash:def:exec.tblentry]
> struct tblentry {
>   struct tblentry *next;
>   union param param;
>   short cmdtype;
>   char rehash;
>   char cmdname[ARB];
> }

> [spec:dash:def:exec.test-access-fn]
> int test_access(const struct stat64 *sp, int stmode)

> [spec:dash:sem:exec.test-access-fn]
> Decide whether the current process may access a file with the given
> `struct stat64`, testing the permission bits directly rather than
> calling `access()`. Defined in `src/bltin/test.c`, and compiled only
> where `faccessat` is unavailable. `stmode` is `R_OK` (4), `W_OK` (2) or
> `X_OK` (1) — the same bit values as the `other` permission triplet, so
> the test is a shift-and-mask.
>
> For an effective uid of 0, read and write are always granted; execute is
> granted only if *any* of the three execute bits is set, expressed by
> replicating `stmode` across all three triplets
> (`(stmode << 6) | (stmode << 3) | stmode`) — this is POSIX's rule for
> the superuser. Otherwise pick the triplet: shift left 6 when the file's
> uid matches the effective uid, 3 when its gid matches the effective gid,
> and 3 also when its gid appears in the supplementary group list
> (fetched with a `getgroups(0, NULL)` sizing call, then a stack-allocated
> second call). Failing all of those, `stmode` stays in the `other`
> triplet.
>
> Return `sp->st_mode & stmode` — non-zero for granted. Testing the bits
> directly rather than using `access()` follows POSIX 1003.2-1992, under
> which `test -w` on a read-only filesystem still reports the write bit,
> and avoids `access()`'s uselessly permissive answers for root.

> [spec:dash:def:exec.test-exec-fn]
> static int test_exec(const char *fullname, struct stat64 *statb)

> [spec:dash:sem:exec.test-exec-fn]
> Return whether `fullname` is something the shell may execute. It must
> be a regular file (`S_ISREG`). Then, as a fast path, all three execute
> bits being set (`(st_mode & 0111) == 0111`) is accepted without further
> checks; otherwise fall back to `test_file_access(fullname, X_OK)` where
> `faccessat` exists, or `test_access(statb, X_OK)` where it does not.

> [spec:dash:def:exec.test-file-access-fn]
> int test_file_access(const char *path, int mode)

> [spec:dash:sem:exec.test-file-access-fn]
> `faccessat(AT_FDCWD, path, mode, AT_EACCESS)` inverted to a
> true-means-permitted result. Defined in `src/bltin/test.c` and compiled
> only where `faccessat` exists. `AT_EACCESS` makes the check use the
> effective rather than real ids, which is what the shell wants.
>
> One correction is applied: on kernels where `faccessat` wrongly grants
> execute permission to the superuser for a file with no execute bit set
> (`faccessat_confused_about_superuser()`), an `X_OK` query by euid 0 is
> answered 0 unless `has_exec_bit_set(path)` confirms at least one execute
> bit — matching the POSIX superuser rule that `test_access` implements
> directly.

> [spec:dash:def:exec.tryexec-fn]
> STATIC void tryexec(char *cmd, char **argv, char **envp)

> [spec:dash:sem:exec.tryexec-fn]
> Attempt one `execve(cmd, argv, envp)`, retrying on `EINTR` under SYSV.
> Returns only on failure, leaving `errno` set.
>
> If the failure is `ENOEXEC` and `cmd` is not already the shell itself,
> the file is a script without a `#!` line, so re-exec it through
> `_PATH_BSHELL`. The two statements are `*argv-- = cmd;` then
> `*argv = cmd = path_bshell;`, which is the shuffle in this order:
> store the command name into the *current* `argv[0]` (where it already
> is), step `argv` back one slot, then write the shell path into that
> new `argv[0]`. The net effect is `argv[-1] == _PATH_BSHELL` and
> `argv[0] == <command name>` relative to the original vector; retry. Writing below `argv[0]` is safe because
> callers construct the vector with a spare leading slot.

> [spec:dash:def:exec.typecmd-fn]
> int typecmd(int argc, char **argv)

> [spec:dash:sem:exec.typecmd-fn]
> The `type` builtin: consume options with `nextopt(nullstr)`, then
> `describe_command(out1, name, NULL, 1)` — verbose, standard path — for
> each operand, OR-ing the results so the status is non-zero if any name
> was unknown.

> [spec:dash:def:exec.unsetfunc-fn]
> void unsetfunc(const char *name)

> [spec:dash:sem:exec.unsetfunc-fn]
> Delete `name`'s function definition if it has one: look it up and, only
> when the entry is `CMDFUNCTION`, `delete_cmd_entry()`. A cached
> executable or builtin of the same name is left alone.
