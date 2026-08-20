# src/trap.c, src/trap.h

Signal handling state. `trap[NSIG]` holds the trap action string per
signal, indexed by signal number with slot 0 being the `EXIT` trap: NULL
means no trap (default disposition), the empty string means the signal is
ignored (`trap '' SIG`), and any other string is the command to run.
`trapcnt` counts the non-empty entries so `have_traps` is a cheap test.
`ptrap` records that traps were only *partially* cleared — see
`clear_traps`.

`sigmode[NSIG - 1]` caches what disposition is currently installed, so
redundant `sigaction` calls are skipped. Its values:

- `S_DFL` (1) — default handling.
- `S_CATCH` (2) — the shell's `onsig` handler is installed.
- `S_IGN` (3) — ignored.
- `S_HARD_IGN` (4) — ignored on entry to the shell, and must stay
  ignored: POSIX requires a signal inherited as ignored to remain so.
- `S_RESET` (5) — the current setting is known not to match; force a set.

A value of 0 means "not yet probed". Note `sigmode` and `gotsig` are
indexed by `signo - 1` while `trap` is indexed by `signo`.

Delivery is deferred: `onsig` only records into `gotsig[]` and
`pending_sig`, and the actual trap commands run later from `dotrap` at a
safe point. `gotsigchld` is a separate flag because SIGCHLD is tracked
even when untrapped.

**Dash source shape (`trap.clear-traps-fn`):**

    void clear_traps(union node *n)

> [spec:dash:sem:trap.clear-traps-fn]
> Reset traps for a child process. POSIX says a subshell resets trapped
> signals to their default, but a subshell whose body is a single `trap`
> command must still be able to *report* the inherited traps — so `n` is
> the command about to run, and `issimplecmd(n, TRAPCMD->name)` decides
> which behaviour applies.
>
> With interrupts suspended, walk every entry that is neither NULL nor
> the empty string (ignored signals stay ignored across a fork, so they
> are skipped). Clear the slot, then `setsignal` for that signal to
> install the default disposition — skipping index 0, which is `EXIT` and
> has no signal. Then either restore the string (when the child is a
> plain `trap` command, so it can print what was inherited) or "free" it.
> Set `trapcnt` to 0 and record `ptrap = simplecmd`, which flags that the
> table still holds strings that are no longer in effect.
>
> **That free is a no-op and the string leaks.** The slot is set to NULL
> *before* the release, and the release is written `ckfree(*tp)` — so it
> frees the NULL it just stored rather than the saved `otp`. Reproduce
> both the no-op free and the leak.

**Dash source shape (`trap.decode-signal-fn`):**

    int decode_signal(const char *string, int minsig)

> [spec:dash:sem:trap.decode-signal-fn]
> Turn a signal name or number into a signal number, or -1 if it is
> neither. Try `decode_signum` first, so a numeric string wins. Otherwise
> compare `string` case-insensitively against `signal_names[]` from
> `minsig` upward and return the index that matches. `minsig` lets a
> caller exclude signal 0 (`EXIT`), which is a valid `trap` target but
> not a valid `kill` target.

**Dash source shape (`trap.decode-signum-fn`):**

    static int decode_signum(const char *string)

> [spec:dash:sem:trap.decode-signum-fn]
> Return the signal number if `string` is a valid decimal number below
> `NSIG`, else -1. `is_number` accepts only digits, so a leading sign or
> space fails; the upper bound rejects out-of-range numbers. Note 0 is
> accepted, since it names the `EXIT` trap.

**Dash source shape (`trap.dotrap-fn`):**

    void dotrap(void)

> [spec:dash:sem:trap.dotrap-fn]
> Run any pending trap commands. Returns at once if `pending_sig` is
> clear.
>
> Preserve the exit status across the traps: remember `savestatus`, and
> if it is negative (no status is being saved yet) set it from
> `exitstatus` so a trap that inspects `$?` sees the status of the
> command that was interrupted. Clear `pending_sig` and issue a
> `barrier()` so the compiler cannot move the reads of `gotsig` above it —
> a signal arriving during the loop must re-set the flag rather than be
> lost.
>
> Then scan `gotsig[0 .. NSIG-1)`. Skip flags that are clear. If
> `evalskip` is set — a `break`, `continue`, `return` or `exit` is
> propagating — stop and put the signal back into `pending_sig` so it is
> handled after the unwind. Otherwise clear the flag, and if a trap
> string exists run it with `evalstring(p, 0)`. Afterwards restore
> `exitstatus` to the saved value unless the trap performed a `return`
> (`evalskip == SKIPFUNC`), which is allowed to set the status.
>
> Finally restore `savestatus` to what it was on entry.

**Dash source shape (`trap.exitshell-fn`):**

    void exitshell(void)

> [spec:dash:sem:trap.exitshell-fn]
> Terminate the shell, running the `EXIT` trap first. Save `exitstatus`
> into `savestatus` so `$?` inside the trap reports the status that
> caused the exit. Install a local `jmploc` so an error inside the trap
> jumps forward to the cleanup rather than recursing.
>
> If an `EXIT` trap is set, clear the slot first — so a trap that itself
> exits does not re-enter — and skip running it if `ptrap` says traps
> were only partially cleared, which is the case in a `trap`-only
> subshell where the string is present for reporting but not in effect.
> Otherwise clear `evalskip`, run the trap, and set
> `evalskip = SKIPFUNCDEF` so nothing further executes.
>
> Then `exitreset()` and `postexitreset()`. Disable job control with
> `setjobctl(0)` — guarded by a second `setjmp` because it touches the
> terminal and may fail — so the process that had the foreground before
> the shell started gets it back. `flushall()` and `_exit(exitstatus)`.
> Note the status used is `exitstatus`, which the trap may have changed.

**Dash source shape (`trap.have-traps-fn`):**

    static inline int have_traps(void)

> [spec:dash:sem:trap.have-traps-fn]
> Return `trapcnt`, the number of signals with a non-empty trap command.
> Used as a boolean to decide whether the shell must stay in a position
> to run traps — e.g. whether it can `exec` directly instead of forking.

**Dash source shape (`trap.ignoresig-fn`):**

    void ignoresig(int signo)

> [spec:dash:sem:trap.ignoresig-fn]
> Set a signal to `SIG_IGN`, skipping the work when `sigmode` already
> says it is ignored or hard-ignored. Update the cache only when not in a
> vfork child, since such a child shares the parent's memory and must not
> corrupt its view of the parent's dispositions.

**Dash source shape (`trap.onsig-fn`):**

    void onsig(int signo)

> [spec:dash:sem:trap.onsig-fn]
> The installed signal handler. It records rather than acts, so that the
> trap command runs at a controlled point.
>
> In a vfork child that is not the process the handler belongs to
> (`getpid() != vforked`), return immediately — the child shares memory
> with the parent and must not touch this state. For SIGCHLD set
> `gotsigchld` and return unless SIGCHLD is actually trapped, since the
> flag alone is what job control needs.
>
> Otherwise set `gotsig[signo - 1]` and `pending_sig = signo`. For an
> untrapped SIGINT, additionally deliver it now: call `onint()` directly
> when interrupts are not suppressed, and otherwise set `intpending` so
> the next `INTON` runs it.

**Dash source shape (`trap.setinteractive-fn`):**

    void setinteractive(int on)

> [spec:dash:sem:trap.setinteractive-fn]
> Re-derive the dispositions that depend on interactivity. `on` is
> incremented before comparison so that the static `is_interactive`,
> which starts at 0, differs from both possible arguments on the first
> call and the work is never skipped at startup. Returns early when the
> state is unchanged; otherwise records it and re-runs `setsignal` for
> SIGINT, SIGQUIT and SIGTERM, which are the signals whose default
> handling differs between interactive and non-interactive shells.

**Dash source shape (`trap.setsignal-fn`):**

    void setsignal(int signo)

> [spec:dash:sem:trap.setsignal-fn]
> Install the correct disposition for `signo`, derived from the trap
> table and the shell's mode.
>
> Start from the trap entry: NULL gives `S_DFL`, a non-empty string
> `S_CATCH`, and the empty string `S_IGN`. Then, only in the root shell,
> only when the derived action is `S_DFL`, and only when not inside a
> vfork, apply the interactive overrides: SIGINT becomes `S_CATCH` when
> interactive, or running `-c`, or not reading from stdin — so the shell
> can clean up rather than die; SIGQUIT and SIGTERM become `S_IGN` when
> interactive so a stray signal cannot kill the session (SIGQUIT is
> exempted under `DEBUG` with `debug` set, to allow core dumps); and with
> job control, SIGTSTP and SIGTTOU become `S_IGN` so the shell itself is
> not stopped by its own job control. SIGCHLD is always `S_CATCH`,
> overriding everything, because job status tracking depends on it.
>
> Then reconcile with `sigmode[signo - 1]`. When it is 0 the current
> setting is unknown, so query with `sigaction`; a failure returns
> without recording anything, so the probe is retried next time. If the
> inherited handler is `SIG_IGN`, treat it as `S_HARD_IGN` — permanently
> ignored, as POSIX requires — except that with job control the
> stop-related signals (SIGTSTP, SIGTTIN, SIGTTOU) are recorded as plain
> `S_IGN`, since the shell must be able to change those. Any other
> inherited handler becomes `S_RESET`, forcing a set.
>
> Return without acting when the cached state is `S_HARD_IGN`, or already
> equals the wanted action. Otherwise install `onsig` for `S_CATCH`,
> `SIG_IGN` for `S_IGN`, `SIG_DFL` otherwise, with `sa_flags = 0` (so
> system calls are *not* restarted — the shell wants `EINTR`) and a full
> `sa_mask` so no other signal interrupts the handler. Update the cache
> unless in a vfork child.

**Dash source shape (`trap.sigblockall-fn`):**

    void sigblockall(sigset_t *oldmask)

> [spec:dash:sem:trap.sigblockall-fn]
> Block every signal, storing the previous mask through `oldmask` (which
> may be NULL): `sigfillset` then
> `sigprocmask(SIG_SETMASK, &mask, oldmask)`.

**Dash source shape (`trap.trapcmd-fn`):**

    int trapcmd(int argc, char **argv)

> [spec:dash:sem:trap.trapcmd-fn]
> The `trap` builtin. Consume options with `nextopt(nullstr)`. With no
> operands, print every set trap as
> `trap -- <single-quoted action> <name>` — re-executable input — and
> return 0.
>
> Otherwise, if `ptrap` is set the table still holds strings that a fork
> retained only for reporting, so complete the clearing now with
> `clear_traps(NULL)`.
>
> Decide whether the first operand is an action or a signal. It is a
> signal — meaning the traps are being reset to default — when it is the
> only operand, or when it decodes as a signal *number*. Otherwise it is
> the action and is consumed. Note the test uses `decode_signum`, not
> `decode_signal`, so `trap TERM INT` treats `TERM` as an action string
> rather than a signal.
>
> For each remaining operand, decode it with `decode_signal(*ap, 0)`,
> which accepts `EXIT`/0; on failure print `"trap: <name>: bad trap"` to
> `out2` and return 1, leaving earlier operands applied. With interrupts
> suspended: normalise an action of exactly `-` to NULL (reset to
> default), count a non-empty action into `trapcnt`, and copy it with
> `savestr`. Drop the previous entry, decrementing `trapcnt` if it was
> non-empty, and install the new one. Call `setsignal(signo)` for every
> signal except 0, which is `EXIT` and has no disposition to install.
> Return 0.
>
> Note the action is `savestr`'d inside the loop, so with several signals
> each gets its own copy — but `action` is reassigned to that copy, so
> after the first iteration the subsequent copies are made from the
> previous copy rather than the original. The observable behaviour is the
> same.
