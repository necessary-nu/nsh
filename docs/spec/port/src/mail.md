# src/mail.c

Mail checking keeps module-private state: `mailtime[MAXMBOXES]` (10)
holding the last-seen `st_mtime` of each mailbox in `MAILPATH` order, and
`changed`, a counter set when `MAIL` or `MAILPATH` is assigned. The
`changed` flag suppresses notification for one check cycle so that
merely repointing the variable does not announce mail that was already
there.

**Dash source shape (`mail.changemail-fn`):**

    void changemail(const char *val)

> [spec:dash:sem:mail.changemail-fn]
> Increment the module-private `changed` counter. Installed as the
> assignment hook on the `MAIL` and `MAILPATH` variables, so any
> assignment to either marks the mailbox list dirty; the next `chkmail`
> then re-baselines the timestamps instead of reporting new mail. The
> `val` argument is the new variable value and is deliberately ignored —
> the hook signature is shared with other variable change handlers.

**Dash source shape (`mail.chkmail-fn`):**

    void chkmail(void)

> [spec:dash:sem:mail.chkmail-fn]
> Report newly arrived mail. Save the stack with `setstackmark`. Choose
> the path list: `mpathval()` (the value of `MAILPATH`) if `mpathset()`
> says `MAILPATH` is set, otherwise `mailval()` (the value of `MAIL`).
> Iterate `mtp` over `mailtime[0 .. MAXMBOXES)`, so at most 10 mailboxes
> are ever examined and any beyond that are silently ignored.
>
> Each iteration takes the next colon-separated entry with
> `padvance_magic(&mpath, nullstr, 2)`, which builds the candidate onto
> the stack and returns its length, or a negative value when the list is
> exhausted — break out then. The magic value 2 makes `%` rather than
> `/` introduce the per-entry option text, so a `MAILPATH` entry may be
> written `path%message`; the option text is left in the global
> `pathopt`. Take `p = stackblock()`; if it is empty, skip this entry
> while still consuming its `mailtime` slot. Advance `q` to the string's
> NUL and overwrite `q[-1]` with NUL to strip the trailing `/` that
> `padvance_magic` appends (under `DEBUG` this is preceded by an
> `abort()` if that last character is not in fact `/`).
>
> `stat64` the resulting path. On failure record `*mtp = 0` and continue,
> so a mailbox that later appears is reported as new. On success, if
> `changed` is zero and `statb.st_mtime` differs from the stored `*mtp`,
> write the notification to `&errout` as `"%s\n"` — `pathopt` if the
> entry supplied a custom message, otherwise the literal
> `"you have mail"`. Note the test is inequality, not "newer", so a
> mailbox whose mtime moves backwards also reports. Store
> `*mtp = statb.st_mtime` either way.
>
> After the loop reset `changed` to 0 — the suppression lasts exactly one
> pass — and restore the stack with `popstackmark`.
