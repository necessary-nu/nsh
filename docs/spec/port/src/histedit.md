# src/histedit.c

Command-line editing and history, on top of libedit. The whole file is
inside `#ifndef SMALL`. `hist` is the history cookie, `el` the editline
cookie, and `el_in`/`el_out` the stdio streams bound to descriptors 0 and
2. `displayhist` is set by `evaltree` before each command and causes an
`fc -s` re-execution to echo what it is running. `MAXHISTLOOPS` (4)
bounds recursion through `fc`; `DEFEDITOR` is `"ed"`.

For a Rust port this is the natural boundary to substitute an idiomatic
line-editing crate: the contract is the behaviour of `fc`/`histcmd` and
the history-recording calls, not the libedit API.

> [spec:dash:def:histedit.fc-replace-fn]
> STATIC const char * fc_replace(const char *s, char *p, char *r)

> [spec:dash:sem:histedit.fc-replace-fn]
> Implement `fc`'s `old=new` substitution: copy `s` to the stack,
> replacing occurrences of `p` with `r`. After the *first* replacement it
> writes a NUL over `p[0]`, which makes every later comparison fail — so
> only the first occurrence is replaced, and the caller's pattern buffer
> is destroyed as a side effect. Returns the result claimed off the
> stack.

> [spec:dash:def:histedit.histcmd-fn]
> int histcmd(int argc, char **argv)

> [spec:dash:sem:histedit.histcmd-fn]
> The `fc` builtin: list, edit or re-execute history events. Raises
> `sh_error("history not active")` when history is off. The many
> `(void) &var` statements exist only to stop GCC from putting those
> variables in registers, where `longjmp` could clobber them.
>
> **Options** are parsed with the library `getopt` (reset first, which
> differs between glibc and BSD), and only while the next argument is not
> a history number — so `fc -1` selects an event rather than failing on an
> unknown option. `-e` names an editor, `-l` lists, `-n` suppresses event
> numbers when listing, `-r` reverses the order, `-s` re-executes without
> editing. A missing option argument or unknown option raises.
>
> **Fixed here, and upstreamable.** The `-e` arm used to read `optionarg`
> — the shell's *own* option variable, set by `nextopt` in `options.c` —
> rather than `optarg`, which is what the library `getopt` used on the
> line above actually sets. The two are unrelated, so `fc -e ed` silently
> discarded its argument, `editor` kept whatever NULL or stale value
> `optionarg` held, and the DEFEDITOR path ran `ed` instead. Present since
> the original NetBSD import; found by the POSIX case
> `fc-opt-e-names-an-editor`, which had been excused as `manual` until the
> harness grew a writable /tmp. Both languages now read `optarg`.
>
> **Mode.** Unless purely listing, an exception handler is installed that
> resets the recursion counter, unlinks the temporary file and re-raises —
> so an interrupt cannot leave temporary files behind. `active` is
> incremented and, past `MAXHISTLOOPS`, raises
> `"called recursively too many times"`. Unless `-s`, the editor is
> `-e`'s argument, else `FCEDIT`, else `EDITOR`, else `ed`; an editor of
> exactly `-` means "no edit", equivalent to `-s`.
>
> **`-s` substitution.** A first operand containing `=` is split into
> pattern and replacement and consumed. More than one remaining operand
> is `"too many args"`.
>
> **Range.** With no operands, list mode defaults to `-16`..`-1` and
> execute mode to `-1`..`-1`; with one, it is both ends in execute mode
> and that event to `-1` in list mode; with two, they are the ends. More
> is an error. Both are converted by `str_to_event`, and `-r` swaps them.
> The traversal direction is `H_PREV` when the first event number is less
> than the last, `H_NEXT` otherwise — which assumes event numbers
> increase monotonically.
>
> **Editing** allocates a temporary file with `mkstemp` under
> `_PATH_TMP`, raising if it cannot be created or wrapped in a stream.
>
> **The loop** walks from `first` in `direction`, stopping when the event
> number reaches `last` or the history is exhausted. Listing prints
> `"%5d "` and the text (the number suppressed by `-n`). `-s` applies the
> substitution, echoes the command when `displayhist` is set, runs it with
> `evalstring`, re-enters it into the history, and stops after one event.
> Otherwise the text is written to the temporary file.
>
> **After editing**, close the file, build `"<editor> <file>"` and run it
> with `evalstring`, then `readcmdfile` the result to execute it and
> unlink the file.
>
> Finally decrement `active`, clear `displayhist`, and return 0.

> [spec:dash:def:histedit.histedit-fn]
> void histedit(void)

> [spec:dash:sem:histedit.histedit-fn]
> Bring history and line editing into line with the current options.
> Called from `optschanged`, so it must be idempotent. Editing is wanted
> when `Eflag` or `Vflag` is set.
>
> **Interactive**: initialise history if not already on, sizing it from
> `HISTSIZE`, and report `"sh: can't initialize history\n"` on failure.
> If editing is wanted, editing is not already on, and stdin is a tty,
> initialise editline — opening `el_in` on descriptor 0 and `el_out` on
> descriptor 2, directing errors to the trace file under `DEBUG` — then
> bind the history and set `getprompt` as the prompt callback with `\1`
> as the non-printing-sequence delimiter. A failure reports
> `"sh: can't initialize editing\n"`. If editing is *not* wanted but is
> on, shut it down. Finally, when editing is active, select the `vi` or
> `emacs` keymap and re-read the user's `.editrc` with `el_source`.
>
> **Non-interactive**: shut down both editing and history.

> [spec:dash:def:histedit.not-fcnumber-fn]
> int not_fcnumber(char *s)

> [spec:dash:sem:histedit.not-fcnumber-fn]
> Return whether `s` is *not* a history event number: 0 for NULL (so a
> missing argument stops option parsing), and otherwise the negation of
> `is_number` applied after skipping one leading `-`. Used to stop
> `getopt` before it misreads `-1` as an option.

> [spec:dash:def:histedit.sethistsize-fn]
> void sethistsize(const char *hs)

> [spec:dash:sem:histedit.sethistsize-fn]
> `HISTSIZE` change callback. When history is active, set its size to
> `atoi(hs)`, substituting 100 when the value is absent, empty or
> negative. Does nothing when history is off.
>
> Two consequences of `atoi` and of libedit's trimming rule that a port
> must reproduce. `atoi` yields 0 for a non-numeric value, so
> `HISTSIZE=abc` sets the size to 0 — and libedit trims with
> `while (h->cur > h->max && h->cur > 0)`, which has no "0 means
> unbounded" case, so a size of 0 discards the whole list after every
> insert and any later `fc` fails with
> `"history number %s not found (internal error)"`. Separately,
> `history_setsize` only stores the new maximum; the trim happens in
> `history_def_enter`, so shrinking `HISTSIZE` has no effect until the
> next entry is added.

> [spec:dash:def:histedit.setterm-fn]
> void setterm(const char *term)

> [spec:dash:sem:histedit.setterm-fn]
> Tell editline the terminal type. When both editing is active and
> `term` is non-NULL, `el_set(el, EL_TERMINAL, term)`; on failure report
> `"sh: Can't set terminal type %s\n"` and
> `"sh: Using dumb terminal settings.\n"` and carry on.

> [spec:dash:def:histedit.str-to-event-fn]
> int str_to_event(const char *str, int last)

> [spec:dash:sem:histedit.str-to-event-fn]
> Convert an `fc` operand into a history event number. `last` says
> whether this is the range's end, which changes how an out-of-range
> number is clamped.
>
> A leading `-` marks the value relative (counting back from the most
> recent); a leading `+` is skipped and the value is absolute.
>
> For a relative number, step back that many events, clamping to the
> oldest if the history is shorter. For an absolute one, seek that event
> number; if it does not exist, clamp to the newest or oldest end —
> noting that the history package's notions of "first" and "last" are the
> reverse of `fc`'s, hence the inverted `H_FIRST`/`H_LAST` choice and the
> extra `H_NEXT` step for the range end. A number that still cannot be
> resolved raises `"history number %s not found (internal error)"`.
>
> A non-numeric operand is a pattern: search with `H_PREV_STR`, raising
> `"history pattern not found: %s"` if nothing matches. Note the search
> **starts on the entry the cursor is already on**, not the one before
> it — libedit's `history_prev_string` is
> `for (retval = HCURR(h, ev); retval != -1; retval = HNEXT(h, ev))`.
> Since this function seeks `H_FIRST` first, and `input.c` has already
> recorded the `fc` line itself, the pattern can and does match that
> line: `fc -l fc` finds itself. Returns the resulting event number.
