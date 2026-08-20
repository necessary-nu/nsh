# src/input.c, src/input.h

Input is a stack of `struct parsefile`, one per active source (the script,
a `.`-sourced file, a `-c` string, an `eval` string). `parsefile` points
at the current one; `basepf` is the statically allocated bottom of the
stack, using the static `basebuf` of `IBUFSIZ` = `BUFSIZ + PUNGETC_MAX + 1`
bytes; `toppf` records how far `popallfiles` should unwind.

Within a level, `buf` is the buffer, `nextc` the read cursor, `nleft` the
characters left *in the current line*, and (outside `SMALL`) `lleft` the
characters left in the buffer beyond that line — the split exists so
`preadbuffer` can hand out one line at a time while still reading in
large blocks. `unget` counts outstanding `pungetc` pushbacks, satisfied
by reading backwards from `nextc`, which is why the buffer reserves
`PUNGETC_MAX` bytes of history. `eof` is a two-bit field: bit 1 records
that a `PEOF` was returned and may be ungotten, bit 2 marks a string
input that can never be refilled.

Layered on top, `struct strpush` implements pushing text back at the same
level — how alias expansion works. Each level has one inline
`basestrpush` so the common single push needs no allocation. A pushed
string linked to an alias holds `ALIASINUSE` on it; freeing is deferred
through `spfree` until the *next* `pgetc`, so that `PEOA` (the
end-of-alias marker, `PEOF - 1`) is generated before the alias becomes
eligible for reuse — this is what stops a self-referential alias from
recursing.

`struct stdin_state` tracks what can be done with fd 0 when it is shared
with other processes: `seekable` is `lseek(0,0,SEEK_CUR) + 1` (so 0 means
not seekable), `bufferable` says whether reading ahead is safe at all,
and `pip`/`pending` hold a pipe and a byte count used to "un-read" data
via `tee` when it is not.

**Dash source shape (`input.stdin-state`):**

    MKINIT struct stdin_state {
      off_t seekable;
      int pip[2];
      int pending;
      tcflag_t bufferable;
    }

    The historical extractor missed this type because the `MKINIT` marker on
    the preceding line defeated it. The shape remains provenance prose only.

**Dash source shape (`input.flush-input-fn`):**

    void __attribute__((noinline)) flush_input(void)

> [spec:dash:sem:input.flush-input-fn]
> Give back input that was read ahead from fd 0 but not consumed, so
> another reader of the same descriptor sees it. Compute
> `left = basepf.nleft + lleft`. With interrupts suspended: if stdin is
> seekable **and `left` is non-zero**, `lseek(0, -left, SEEK_CUR)` to
> rewind by exactly that much. The `&& left` conjunct matters — without
> it the `else if` below could never be reached when `left` is 0.
> Otherwise, if more bytes are outstanding in the `tee` pipe than we
> still hold (`stdin_state.pending > left`), drain the difference with
> `flush_tee` and clear `pending` — the bytes we did not use were never
> actually consumed from fd 0, so only the surplus needs discarding.
> Then zero both `nleft` and `lleft`. Restore interrupts.

**Dash source shape (`input.flush-tee-fn`):**

    static void flush_tee(void *buf, int nr, int pending)

> [spec:dash:sem:input.flush-tee-fn]
> Consume `pending` bytes from fd 0 into the scratch buffer `buf` of size
> `nr`, discarding them. Loop reading `min(nr, pending)` at a time,
> subtracting only what a successful read returned. A read error or
> end-of-file returns a non-positive value, which is not subtracted, so
> the loop spins in that case — acceptable because it is only reached
> when the bytes are known to be present in the pipe.

**Dash source shape (`input.freestrings-fn`):**

    static void freestrings(struct strpush *sp)

> [spec:dash:sem:input.freestrings-fn]
> Release the deferred `strpush` chain starting at `sp`. With interrupts
> suspended, for each entry: if it carried an alias, clear `ALIASINUSE`
> and, if `ALIASDEAD` is set — the alias was deleted while it was being
> expanded — complete the deletion now with `unalias(sp->ap->name)`.
> Then follow `spfree` to the next entry and free the current one, unless
> it is the level's inline `basestrpush`, which is not separately
> allocated. Finally clear `parsefile->spfree`.

**Dash source shape (`input.input-get-lleft-fn`):**

    static inline int input_get_lleft(struct parsefile *pf)

> [spec:dash:sem:input.input-get-lleft-fn]
> Return `pf->lleft`, or the constant 0 under `SMALL`, where the field
> does not exist because the small build reads one character at a time.

**Dash source shape (`input.input-init-fn`):**

    void input_init(void)

> [spec:dash:sem:input.input-init-fn]
> Determine what fd 0 supports. `tcgetattr(0, &tios) + 1` is non-zero
> exactly when stdin is a terminal; record that in `stdin_istty`. For a
> terminal, `bufferable` is the `ICANON` bit — reading ahead is safe only
> in canonical mode, where the driver already hands over whole lines. For
> a non-terminal, compute `seekable` as `lseek(0,0,SEEK_CUR) + 1` (0 when
> the descriptor is not seekable) and set `bufferable` to whether it is —
> if we can seek backwards afterwards, reading ahead is safe.

**Dash source shape (`input.input-set-lleft-fn`):**

    static inline void input_set_lleft(struct parsefile *pf, int len)

> [spec:dash:sem:input.input-set-lleft-fn]
> Store `len` into `pf->lleft`; a no-op under `SMALL`.

**Dash source shape (`input.parsefile`):**

    struct parsefile {
      struct parsefile *prev;
      int linno;
      int fd;
      int nleft;
      int eof;
      char *nextc;
      char *buf;
      struct strpush *strpush;
      struct strpush basestrpush;
      struct strpush *spfree;
      int unget;
    }

**Dash source shape (`input.pgetc-eoa-fn`):**

    int pgetc_eoa(void)

> [spec:dash:sem:input.pgetc-eoa-fn]
> Like `pgetc`, but return `PEOA` at the end of an alias instead of
> transparently continuing. Yields `PEOA` when a string is pushed, its
> characters are exhausted (`nleft == -1`), and the push came from an
> alias; otherwise defers to `pgetc`. The parser uses this where the end
> of an alias is syntactically significant.

**Dash source shape (`input.pgetc-fn`):**

    int __attribute__((noinline)) pgetc(void)

> [spec:dash:sem:input.pgetc-fn]
> Read the next input character, or `PEOF` at end of input. Values are
> returned through `(signed char)`, so bytes above 0x7F come back
> negative — the syntax tables are indexed with that convention.
>
> First, if a deferred `strpush` free is pending, run `freestrings` now;
> delaying it to here is what gives `PEOA` its chance to be generated.
>
> If `unget` is non-zero, satisfy the read from the pushback history:
> decrement it and return `nextc[-(unget_before)]`, i.e. step backwards
> through characters already passed. No buffer state changes, so a
> pushback followed by a read is exactly reversible.
>
> Otherwise, if `nleft > 0`, take `*nextc++` and decrement. If not, and a
> string is pushed, `popstring()` and start over — which is where `nleft`
> becomes -1 and `pgetc_eoa` can see the alias boundary. Otherwise refill
> via `preadbuffer()`.
>
> Under `SMALL` only, a NUL result is deleted from the buffer by moving
> the remaining `nleft` bytes down over it and retrying; the normal build
> strips NULs in `preadbuffer` instead.

**Dash source shape (`input.popallfiles-fn`):**

    void popallfiles(void)

> [spec:dash:sem:input.popallfiles-fn]
> `unwindfiles(toppf)` — pop back to the top-level input, which is
> `basepf` unless a non-pushing `setinputfd` moved `toppf`.

**Dash source shape (`input.popfile-fn`):**

    void popfile(void)

> [spec:dash:sem:input.popfile-fn]
> Pop one input level. With interrupts suspended, make `prev` current and
> clear the popped level's `prev`. `basepf` is statically allocated and
> its buffer is static, so stop there without freeing anything.
> Otherwise close `fd` if it is open, free `buf`, run any deferred
> `freestrings` for the now-current level, and free the level itself.
>
> Two things about the popped level's own `strpush` state are not what
> the source reads as, and both are observable.
>
> The `while (pf->strpush) { popstring(); freestrings(…); }` loop between
> the two does not do what it says. `popstring` operates on
> `parsefile->strpush`, and `parsefile` became the *outer* level two
> lines earlier, so the loop pops the outer level's stack while testing
> the popped one's — the condition never changes, and the second
> iteration dereferences a NULL `strpush` on any outer level with an
> empty stack. It was correct in 0.5.10, where `parsefile = pf->prev`
> came *after* the loop. It cannot execute in any run that survives, so
> reproducing it is reproducing a crash; discard the popped level's
> `strpush` entries instead.
>
> `pf->spfree` is never drained. `freestrings` above runs on the level
> being returned to, not the one being popped, and `ckfree(pf)` takes the
> popped level's chain with it — with `ALIASINUSE` still set on every
> entry. Because `popstring` defers by one `pgetc`, a level can end with
> an entry still there: `alias q='echo hi '; echo \`q\`` leaves `q`
> marked in use for the rest of the shell's life, so it never expands
> again and the next `q` reports "not found". Do not release the chain;
> that is a behaviour change, not a leak fix.

**Dash source shape (`input.popstring-fn`):**

    static void popstring(void)

> [spec:dash:sem:input.popstring-fn]
> Undo one `pushstring`. With interrupts suspended: when the push came
> from an alias and at least one character was consumed, check the last
> character read — if it was a space or tab, set `CHKALIAS` in `checkkwd`
> so the *next* word is also alias-expanded, which is how
> `alias foo='bar '` makes the following word eligible. Also free the
> saved string if it is not the alias's own `name` buffer.
>
> Restore `nextc`, `nleft` and `unget` from the save, unlink the entry,
> and put it on `spfree` rather than freeing it — the deferred free is
> what keeps the alias marked in-use until the next `pgetc`.

**Dash source shape (`input.preadbuffer-fn`):**

    static int preadbuffer(void)

> [spec:dash:sem:input.preadbuffer-fn]
> Refill the buffer and return the next character. Returns `PEOF` at end
> of input, latching `eof` to 3 so subsequent calls return it
> immediately without another read; a level with `eof & 2` (a string) can
> never be refilled and takes that path at once.
>
> `flushall()` first, so any prompt or prior output appears before the
> read blocks. Track `something`, whether the line contains anything
> other than blanks — initialised from `whichprompt != 1`, so a
> continuation line always counts — since only non-blank lines are worth
> adding to the history.
>
> With interrupts suspended: when no buffered data remains, save how much
> of the current line has already been scanned, call `preadfd()`, and
> re-derive the scan pointer, since `preadfd` moves the buffer contents.
> A non-positive result is end of input: zero the counters, and either
> fall through to hand out the partial line that was already scanned, or
> return `PEOF`.
>
> Then find the end of one line. Under `SMALL` take everything read as-is.
> Otherwise walk forward, deleting NUL bytes in place by moving the
> remainder down, stopping at a `\n`, and setting `something` for any
> character other than tab or space. Running out of buffered data before
> a newline loops back for another read, so a line may span reads.
>
> Publish the line: `nleft` becomes the count up to but excluding the
> terminator, and the character at the line end is temporarily replaced
> with NUL so the buffer can be passed as a C string. If input is coming
> from fd 0, history is active, and the line had content, add it —
> `H_ENTER` for a first line (`whichprompt == 1`), `H_APPEND` for a
> continuation, so a multi-line command becomes one history entry.
> Restore interrupts, echo the line to `out2` when `verbose` is set,
> restore the saved character, and return the first character, advancing
> `nextc`.

**Dash source shape (`input.preadfd-fn`):**

    static int preadfd(void)

> [spec:dash:sem:input.preadfd-fn]
> Fill the buffer from the current descriptor, returning the number of
> bytes read (0 at end of file, negative on error). Before reading,
> compact: preserve up to `PUNGETC_MAX` bytes of already-consumed history
> plus the unscanned remainder by moving them to the front of the buffer,
> and repoint `nextc` past the preserved history. Read into the space
> after them, at most `BUFSIZ` minus what is already buffered; outside
> `SMALL`, a full buffer returns 0 immediately.
>
> With line editing active on fd 0, take the line from `el_gets` instead
> of `read`: fetch one line if none is held, copy up to `nr` bytes out of
> it, and keep any remainder for the next call. The `el_gets` call is
> bracketed by a stack mark because the editor may allocate.
>
> Otherwise decide whether to tee. Teeing applies when reading fd 0, line
> editing is not in use, and `stdin_bufferable()` is false — i.e. we must
> not consume more than we use, because another process shares the
> descriptor. In that case `stdin_tee` duplicates the pending data into a
> pipe and the actual `read` is redirected to the pipe's read end, so fd
> 0's offset is not disturbed. If `tee` is unavailable (`EINVAL`), fall
> back to unbuffered reading by retrying with a one-byte request.
>
> Perform the `read`. On `EINTR`, retry — unless this is a nested input
> level with a signal pending, where the caller needs to see the
> interruption. On `EWOULDBLOCK` on fd 0, clear `O_NONBLOCK`, report
> `"sh: turning off NDELAY mode"` and retry, recovering from a caller
> that left stdin non-blocking.

**Dash source shape (`input.pungetc-fn`):**

    void pungetc(void)

> [spec:dash:sem:input.pungetc-fn]
> Undo one `pgetc`. Push back `1 - (eof & 1)` characters — so a `PEOF`
> that was synthesised rather than read consumes no buffer position — and
> clear the low `eof` bit, making the end-of-file readable again.

**Dash source shape (`input.pungetn-fn`):**

    void pungetn(int n)

> [spec:dash:sem:input.pungetn-fn]
> Add `n` to the level's `unget` counter, pushing back that many
> characters. The next `n` `pgetc` calls re-read backwards from `nextc`.
> The buffer reserves `PUNGETC_MAX` bytes of history, which bounds how
> far back this is valid.

**Dash source shape (`input.pushfile-fn`):**

    STATIC void pushfile(void)

> [spec:dash:sem:input.pushfile-fn]
> Push a new, zeroed input level: allocate a `struct parsefile`,
> `memset` it to 0, link `prev` to the current one, set `linno = 1` and
> `fd = -1` (the "string input" marker), and make it current. Zeroing is
> what leaves `nleft`, `eof`, `strpush` and `unget` in their initial
> states.

**Dash source shape (`input.pushstdin-fn`):**

    void pushstdin(void)

> [spec:dash:sem:input.pushstdin-fn]
> Temporarily switch to reading from the base level: link `basepf.prev`
> to the current level and make `basepf` current. Used by `read`, which
> must consume from fd 0 rather than from the script being executed.
> Undone by `popfile`.

**Dash source shape (`input.pushstring-fn`):**

    void pushstring(char *s, void *ap)

> [spec:dash:sem:input.pushstring-fn]
> Push `s` to be read before the rest of the current input — the
> mechanism behind alias expansion. With interrupts suspended, take the
> level's inline `basestrpush` when neither `strpush` nor `spfree` is in
> use, and otherwise allocate one and link it. Save `nextc`, `nleft`,
> `unget` and the current `spfree` into it. When `ap` names an alias,
> mark it `ALIASINUSE` and record its `name` in `sp->string` so the
> deferred free can tell whether the string is the alias's own storage.
> Then point `nextc` at `s` with `nleft = strlen(s)`, and reset `unget`
> and `spfree` to 0 — pushbacks do not cross a string boundary.

**Dash source shape (`input.reset-input-fn`):**

    void reset_input(void)

> [spec:dash:sem:input.reset-input-fn]
> Forget everything cached about fd 0: set `stdin_istty` to -1 so
> `stdin_bufferable` re-probes, clear `basepf.eof` so end-of-file is
> re-tested, and `flush_input()` to give back any read-ahead.

**Dash source shape (`input.setinputfd-fn`):**

    static void setinputfd(int fd, int push)

> [spec:dash:sem:input.setinputfd-fn]
> Make `fd` the input source. Must be called with interrupts off.
> `pushfile()` for a new level; when `push` is zero the caller does not
> want the old input restorable, so move `toppf` to the new level, which
> makes `popallfiles` stop here. Store the descriptor and allocate an
> `IBUFSIZ` buffer.

**Dash source shape (`input.setinputfile-fn`):**

    int setinputfile(const char *fname, int flags)

> [spec:dash:sem:input.setinputfile-fn]
> Open `fname` and read from it. `flags` may carry `INPUT_PUSH_FILE`
> (keep the previous input restorable) and `INPUT_NOFILE_OK` (a failure
> to open returns negative rather than raising). With interrupts
> suspended, `sh_open` the file read-only; move any descriptor below 10
> out of the way with `savefd`, so ordinary redirections cannot collide
> with it; then `setinputfd`. Return the descriptor, or the negative
> `sh_open` result.

**Dash source shape (`input.setinputstring-fn`):**

    void setinputstring(char *string)

> [spec:dash:sem:input.setinputstring-fn]
> Read from a string rather than a descriptor — how `eval` and command
> substitution feed the parser. With interrupts suspended, push a level,
> point `nextc` at `string` with `nleft = strlen(string)`, and set
> `eof = 2` marking it unrefillable, so `preadbuffer` returns `PEOF`
> immediately once it is exhausted. `fd` stays -1 from `pushfile` and no
> buffer is allocated, so in the C the string must outlive the level —
> which is why `evalstring` holds its `sstrdup` across the `popfile` and
> `parsebackq` cannot release the block it grabbed. Nothing writes
> through it, so a level that copies the string into its own buffer is
> behaviour for behaviour.

**Dash source shape (`input.stdin-bufferable-fn`):**

    static bool stdin_bufferable(void)

> [spec:dash:sem:input.stdin-bufferable-fn]
> Return whether reading ahead on fd 0 is safe, calling `input_init()`
> first if the state has never been probed (`stdin_istty < 0`).

**Dash source shape (`input.stdin-clear-nonblock-fn`):**

    static int stdin_clear_nonblock(void)

> [spec:dash:sem:input.stdin-clear-nonblock-fn]
> Clear `O_NONBLOCK` on fd 0: `F_GETFL`, mask the bit off, `F_SETFL`.
> Returns the final `fcntl` result, negative on failure — including when
> the initial `F_GETFL` failed, in which case nothing is attempted.

**Dash source shape (`input.stdin-tee-fn`):**

    static int stdin_tee(void *buf, int nr)

> [spec:dash:sem:input.stdin-tee-fn]
> Copy up to `nr` bytes of fd 0's pending input into a private pipe
> without consuming them from fd 0, so the shell can read ahead on a
> descriptor it shares with another process. Create the pipe on first use
> and move both ends above fd 9 with `savefd` so redirections cannot
> collide. Discard whatever was left in the pipe from the previous call
> with `flush_tee`, then `tee(0, pip[1], nr, 0)` — which duplicates
> without consuming. Record the byte count in `stdin_state.pending` and
> return it. Where `tee` is unavailable the function reports `-1` with
> `errno = EINVAL`, which is the caller's signal to fall back to
> one-byte-at-a-time reads.

**Dash source shape (`input.strpush`):**

    struct strpush {
      struct strpush *prev;
      char *prevstring;
      int prevnleft;
      struct alias *ap;
      char *string;
      struct strpush *spfree;
      int unget;
    }

**Dash source shape (`input.unwindfiles-fn`):**

    void __attribute__((noinline)) unwindfiles(struct parsefile *stop)

> [spec:dash:sem:input.unwindfiles-fn]
> `popfile()` until `parsefile` is `stop` *and* `basepf.prev` is NULL.
> The second condition matters because `pushstdin` can make `basepf`
> current while levels remain beneath it, and those must be unwound too.
