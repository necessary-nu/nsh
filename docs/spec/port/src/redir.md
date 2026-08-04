# src/redir.c

Redirections are applied by saving the descriptors they displace onto a
stack of `struct redirtab`, so that leaving the construct can undo them.
Each entry holds `renamed[10]`, one slot per descriptor 0–9 — descriptors
above 9 are never saved, which is why the shell moves its own
descriptors above that range with `savefd`. Slot values:

- `EMPTY` (-2) — this descriptor was not touched at this level.
- `CLOSED` (-1) — the descriptor was not open before, so undoing means
  closing it.
- `>= 0` — the descriptor was saved to this (high) number, so undoing
  means `dup2` back and close.

`closed_redirs` is a bitmap of descriptors currently closed by a
redirection, distinguishing "never open" from "closed by an enclosing
`n>&-`" so nested redirections restore correctly.

`PIPESIZE` is `PIPE_BUF` where available and 4096 otherwise; here
documents at or below that size can be stuffed into a pipe without a
writer process.

> [spec:dash:def:redir.redirtab]
> MKINIT struct redirtab {
>   struct redirtab *next;
>   int renamed[10];
> }
>
> Note: this type is absent from `plan/.port-manifest.styx` — the `MKINIT`
> marker on the line before the declaration defeated the extractor. The
> rule and its sidecar annotation are hand-added.

> [spec:dash:def:redir.dupredirect-fn]
> static void dupredirect(union node *redir, int f) #endif

> [spec:dash:sem:redir.dupredirect-fn]
> Put the opened result `f` in place at the descriptor the redirection
> names (`redir->nfile.fd`). For `NTOFD`/`NFROMFD` (`>&n`, `<&n`): a
> non-negative `f` is a duplication, `sh_dup2(f, fd, -1)` — copy without
> closing the source, since it belongs to the user. A negative `f` means
> the form was `>&-`, so just close `fd`. For every other type, `f` is a
> descriptor this module opened, so `sh_dup2(f, fd, f)` moves it into
> place and closes the temporary.
>
> The generated `def` signature above includes a stray `#endif` — the
> declaration is split across an `#ifdef notyet`, and the extractor
> captured the non-`notyet` arm plus the following preprocessor line. The
> real signature is `static void dupredirect(union node *redir, int f)`.
> Under `notyet` it takes an extra `char memory[10]` used to redirect
> into the in-memory output sink.

> [spec:dash:def:redir.openhere-fn]
> STATIC int openhere(union node *redir)

> [spec:dash:sem:redir.openhere-fn]
> Make a here document readable, returning a descriptor positioned at its
> start. The text is `redir->nhere.doc->narg.text`; for `NXHERE` (a
> document whose delimiter was unquoted, so it is subject to expansion)
> first `expandarg(..., EXP_QUOTED)` and take the result from
> `stackblock()`.
>
> Ask `sh_pipe` for a memfd when the document exceeds `PIPESIZE`. If a
> memfd was obtained, or the document fits in a pipe's buffer, write the
> whole text and — for the memfd case — `lseek` back to 0, then close the
> write end and return the read end. No process is needed because the
> write cannot block.
>
> Otherwise fork a writer with `forkshell(NULL, NULL, FORK_NOJOB)`. The
> child closes the read end, ignores SIGINT, SIGQUIT, SIGHUP and SIGTSTP
> so it is not killed before the reader has finished, restores SIGPIPE to
> default so it dies quietly if the reader goes away, writes the text and
> `_exit(0)`. The parent closes the write end and returns the read end.

> [spec:dash:def:redir.openredirect-fn]
> STATIC int openredirect(union node *redir)

> [spec:dash:sem:redir.openredirect-fn]
> Produce the descriptor a single redirection should install, without
> installing it. Returns a descriptor, or -2 to mean "nothing to do".
> Dispatch on `redir->nfile.type`:
>
> - `NFROM` (`<`): open read-only.
> - `NFROMTO` (`<>`): open `O_RDWR|O_CREAT`.
> - `NTO` (`>`): under `noclobber` (`Cflag`), refuse to truncate an
>   existing regular file. If `stat64` fails the file does not exist, so
>   open `O_WRONLY|O_CREAT|O_EXCL` — which also closes the race. If it
>   exists and is a regular file, fail with `EEXIST`. Otherwise it is a
>   device, fifo or similar: open it `O_WRONLY` and re-check with
>   `fstat64`, failing if what was actually opened turned out to be a
>   regular file. Without `noclobber`, fall through to `NCLOBBER`.
> - `NCLOBBER` (`>|`): open `O_WRONLY|O_CREAT|O_TRUNC`.
> - `NAPPEND` (`>>`): open `O_WRONLY|O_CREAT|O_APPEND`.
> - `NTOFD`/`NFROMFD` (`>&n`, `<&n`): the result is `redir->ndup.dupfd`
>   itself, except that duplicating a descriptor onto itself is a no-op,
>   reported as -2.
> - `NHERE`/`NXHERE`: `openhere(redir)`.
> - anything else: `abort()` under `DEBUG`, otherwise falls into the here
>   document case.
>
> Failures raise through `sh_open`/`sh_open_fail` and do not return.

> [spec:dash:def:redir.popredir-fn]
> void popredir(int drop)

> [spec:dash:sem:redir.popredir-fn]
> Undo one level of redirections and pop it. With `drop` non-zero the
> saved descriptors are discarded rather than restored — used when the
> shell is about to exec or exit and the redirections should persist.
>
> With interrupts suspended, walk slots 0–9 of the top `redirtab`,
> skipping `EMPTY`. Update `closed_redirs` for each (or treat it as
> already closed when dropping). For `CLOSED`, close the descriptor
> unless it was already closed by an enclosing redirection. Otherwise, if
> not dropping, `dup2` the saved descriptor back into place — calling
> `reset_input()` first for descriptor 0, since restoring stdin
> invalidates any read-ahead — and in all cases close the saved copy.
> Then unlink and free the entry.

> [spec:dash:def:redir.pushredir-fn]
> struct redirtab *pushredir(union node *redir)

> [spec:dash:sem:redir.pushredir-fn]
> Begin a redirection scope and return the previous top of `redirlist`,
> which the caller passes to `unwindredir` to undo exactly this scope.
> When `redir` is NULL there is nothing to redirect, so return without
> pushing — an empty scope costs no allocation. Otherwise allocate a
> `struct redirtab`, link it in, and initialise all ten slots to `EMPTY`.

> [spec:dash:def:redir.redirect-fn]
> void redirect(union node *redir, int flags)

> [spec:dash:sem:redir.redirect-fn]
> Apply a list of redirections. `flags` may carry `REDIR_PUSH` (save the
> displaced descriptors so `popredir` can restore them), `REDIR_BACKQ`
> (used with the `notyet` in-memory output support) and `REDIR_SAVEFD2`.
> Returns immediately for an empty list.
>
> With interrupts suspended, and with `sv` set to the current `redirlist`
> when pushing, walk the list. For each node, `openredirect` gives the
> new descriptor; -2 or below means nothing to do. Take the target `fd`
> from the node, and call `reset_input()` when it is 0 because
> redirecting stdin invalidates read-ahead.
>
> When saving, look at `sv->renamed[fd]`. Only the *first* redirection of
> a given descriptor at this level is saved — a later one finds the slot
> already non-`EMPTY` and leaves it alone, so the restore returns to the
> state on entry rather than to an intermediate one. For a first touch,
> the default save value is `CLOSED`; but if the descriptor is genuinely
> being replaced (`fd != newfd`) and was not already closed by an
> enclosing redirection, move the old descriptor out of the way with
> `savefd(fd, fd)` and record where it went. In that case `fd` is set to
> -1 so the `fd == newfd` test below cannot short-circuit.
>
> Skip the duplication when source and target coincide; otherwise
> `dupredirect`. After the loop, if `REDIR_SAVEFD2` is set and descriptor
> 2 was saved, point `preverrout.fd` at the saved copy, so error output
> can still reach the original stderr.
>
> Note `REDIR_SAVEFD2` is **03**, i.e. `REDIR_PUSH | REDIR_BACKQ`, not a
> distinct bit. So `flags & REDIR_SAVEFD2` is also true for a plain
> `REDIR_PUSH` call, and the test then dereferences `sv` — which is NULL
> whenever `REDIR_PUSH` was not set. Reproduce the overlap; do not "fix"
> it to a distinct bit.

> [spec:dash:def:redir.redirectsafe-fn]
> int redirectsafe(union node *redir, int flags)

> [spec:dash:sem:redir.redirectsafe-fn]
> `redirect` with errors caught instead of propagated. Save the interrupt
> count and the current `handler`, install a local `jmploc`, and run
> `redirect`. Returns 0 on success and, on an exception, `2 *` the
> `setjmp` value — so a caught error yields 2, the shell's usual
> redirection-failure status. Restore the previous handler with
> `restore_handler_expandarg(savehandler, err)`, which also re-raises if
> the exception was not one that may be caught here, and restore the
> interrupt count.

> [spec:dash:def:redir.savefd-fn]
> int savefd(int from, int ofd)

> [spec:dash:sem:redir.savefd-fn]
> Duplicate `from` to some descriptor ≥ 10 and close `ofd`, returning the
> new descriptor. Use `F_DUPFD_CLOEXEC` where available, otherwise
> `F_DUPFD` followed by an explicit `FD_CLOEXEC` — the shell's own
> descriptors must not leak into executed commands.
>
> `EBADF` is treated specially: it means `from` was not open, which is
> not an error, so `ofd` is left alone and the negative result is
> returned for the caller to interpret. Any other failure raises
> `sh_error("%d: %s", from, strerror(err))`. Keeping the shell's
> descriptors above 9 is what makes the ten-slot `renamed` array
> sufficient.

> [spec:dash:def:redir.sh-dup2-fn]
> static int sh_dup2(int ofd, int nfd, int cfd)

> [spec:dash:sem:redir.sh-dup2-fn]
> Duplicate `ofd` onto `nfd`, or to the lowest free descriptor when `nfd`
> is negative, then close `cfd` if it is non-negative. When `nfd` was
> negative and the `dup` succeeded, `cfd` is suppressed — the caller
> passes the same descriptor for both, and closing it would undo the
> duplication it just made. A failure raises
> `sh_error("%d: %s", ofd, strerror(errno))`. Returns the new descriptor.

> [spec:dash:def:redir.sh-open-fail-fn]
> static int sh_open_fail(const char *pathname, int flags, int e)

> [spec:dash:sem:redir.sh-open-fail-fn]
> Raise the diagnostic for a failed open and do not return. The verb and
> the `errmsg` action are `"create"`/`E_CREAT` when `flags` has `O_CREAT`
> and `"open"`/`E_OPEN` otherwise, giving
> `"cannot open <path>: No such file"` versus
> `"cannot create <path>: Directory nonexistent"`.

> [spec:dash:def:redir.sh-open-fn]
> int sh_open(const char *pathname, int flags, int mayfail)

> [spec:dash:sem:redir.sh-open-fn]
> `open64(pathname, flags, 0666)`, retrying while it fails with `EINTR`
> and no signal is pending — a pending signal must be allowed to reach
> the shell rather than being swallowed by the retry. Mode 0666 is used
> for creation; the process umask reduces it. On failure, return the
> negative result when `mayfail` is set, otherwise raise via
> `sh_open_fail`.

> [spec:dash:def:redir.sh-pipe-fn]
> int sh_pipe(int pip[2], int memfd)

> [spec:dash:sem:redir.sh-pipe-fn]
> Fill `pip` with a read and a write descriptor and report whether it is
> a memfd (1) or a real pipe (0). When `memfd` is requested and
> `memfd_create` is available, create an anonymous file and duplicate it
> for the second end — both descriptors then refer to the same seekable
> object, which is what lets `openhere` write a large document and rewind
> rather than forking a writer. Otherwise `pipe(pip)`, raising
> `sh_error("Pipe call failed")` on failure.

> [spec:dash:def:redir.unwindredir-fn]
> void unwindredir(struct redirtab *stop)

> [spec:dash:sem:redir.unwindredir-fn]
> `popredir(0)` — restoring, not dropping — until `redirlist` reaches
> `stop`. Passing the value from `pushredir` unwinds exactly that scope;
> passing 0 unwinds everything, which is what the `EXITRESET` event does.

> [spec:dash:def:redir.update-closed-redirs-fn]
> static unsigned update_closed_redirs(int fd, int nfd)

> [spec:dash:sem:redir.update-closed-redirs-fn]
> Record whether `fd` is now closed by a redirection and report whether
> it already was. Take a snapshot of `closed_redirs`, then clear `fd`'s
> bit when `nfd >= 0` (the descriptor is being opened) and set it
> otherwise (it is being closed). Return the snapshot masked to `fd`'s
> bit — non-zero if it was already closed. That distinction is what stops
> an inner `n>&-` from closing a descriptor an outer one had already
> closed, and stops the restore from double-closing.
