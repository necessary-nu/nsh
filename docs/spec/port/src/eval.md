# src/eval.c, src/eval.h

The evaluator. `evaltree` dispatches on node type to the `eval*`
routines; the result of every command is left in the global `exitstatus`
as well as returned.

Control-flow transfer out of nested constructs is done with the
`evalskip` flag rather than by unwinding: `SKIPBREAK` (1), `SKIPCONT`
(2), `SKIPFUNC` (4, `return`), `SKIPFUNCDEF` (8, `return` outside a
function, which abandons the rest of the file). `skipcount` holds how
many loop levels a `break`/`continue` should cross, and `loopnest` how
many are currently active. Every evaluation routine checks `evalskip` and
stops executing when it is set.

`flags` on `evaltree` are `EV_EXIT` (01, exit the shell after this tree —
which lets the last command of a subshell `exec` in place instead of
forking) and `EV_TESTED` (02, the status is being tested, so `set -e`
must not fire).

Status bookkeeping: `exitstatus` is the last command's status,
`back_exitstatus` the last command substitution's, and `savestatus` the
status from outside a trap, -1 when not in one. `funcline` is the line
number where the current function was defined, used to report line
numbers relative to the function.

`bltin` is a synthetic `struct builtincmd` for "no command name at all"
(a bare assignment or redirection), whose handler is `bltincmd`.

**Dash source shape (`eval.backcmd`):**

    struct backcmd {
      int fd;
      char *buf;
      int nleft;
      struct job *jp;
    }

**Dash source shape (`eval.bltincmd-fn`):**

    STATIC int bltincmd(int argc, char **argv)

> [spec:dash:sem:eval.bltincmd-fn]
> Handler for a command consisting only of assignments and/or
> redirections. Returns `back_exitstatus` — the status of the last
> command substitution performed while expanding it — which is what POSIX
> requires: `x=$(false)` must leave `$?` as 1.

**Dash source shape (`eval.breakcmd-fn`):**

    int breakcmd(int argc, char **argv)

> [spec:dash:sem:eval.breakcmd-fn]
> Implements both `break` and `continue`; which one is decided by
> `**argv == 'c'`. The count is `number(argv[1])` or 1. A count of zero
> or less raises `badnum`. A count larger than `loopnest` is silently
> clamped to `loopnest` — breaking out of more loops than exist is
> arguably an error but is not one in the standard shell. When the
> resulting count is positive, set `evalskip` to `SKIPCONT` or
> `SKIPBREAK` and `skipcount` to the count. Outside any loop
> (`loopnest == 0`) nothing happens at all. Always returns 0.

**Dash source shape (`eval.eprintlist-fn`):**

    STATIC int eprintlist(struct output *out, struct strlist *sp, int sep)

> [spec:dash:sem:eval.eprintlist-fn]
> Print a `strlist` space-separated for `set -x` tracing, returning the
> updated separator state. The format is `" %s"` advanced by `1 - sep`
> bytes, so the leading space is skipped while `sep` is 0 and included
> once it is 1; `sep |= 1` after the first item. Returning `sep` lets two
> consecutive calls (assignments then arguments) share one separator
> state.

**Dash source shape (`eval.evalbackcmd-fn`):**

    void evalbackcmd(union node *n, struct backcmd *result)

> [spec:dash:sem:eval.evalbackcmd-fn]
> Start a command substitution, returning a descriptor to read its output
> from. Should be called with interrupts off. Initialise `result` to
> empty (`fd` -1) and return immediately for a NULL command.
>
> Otherwise `sh_pipe`, publishing both ends in the global `tpip` so the
> `EXITRESET` handler can close them if an exception unwinds past here.
> Make a job and `forkshell(jp, n, FORK_NOJOB)`. Clear `tpip[0]` once the
> fork has returned, since the parent no longer needs the emergency
> close.
>
> The child force-enables interrupts, closes the read end, moves the
> write end onto descriptor 1 (skipping the move if it is already 1),
> calls `ifsfree()` to drop inherited expansion state, and
> `evaltreenr(n, EV_EXIT)`, which never returns.
>
> The parent closes the write end and records the read end and the job in
> `result`. The caller reads the output and then `waitforjob`s.

**Dash source shape (`eval.evalbltin-fn`):**

    STATIC int evalbltin(const struct builtincmd *cmd, int argc, char **argv, int flags)

> [spec:dash:sem:eval.evalbltin-fn]
> Run a builtin with its own exception handler, returning 0 normally or
> the non-zero `setjmp` value if the builtin raised. Save `commandname`
> and `handler` (both `volatile`, since they are live across the jump),
> install a local `jmploc`, set `commandname` to `argv[0]`, and set up
> `nextopt` by pointing `argptr` at `argv + 1` and clearing `optptr`.
>
> `eval` is special-cased — `evalcmd` needs the `flags` that the generic
> builtin signature has no room for — and everything else is called
> through `cmd->builtin`.
>
> Afterwards `flushall()`, then check `outerr(out1)`: an output error
> warns `"<name>: I/O error"` and is OR-ed into the status, so a builtin
> whose output could not be written reports failure. Store the result in
> `exitstatus`.
>
> The shared exit path (also reached by the jump) calls `freestdout()` to
> discard any buffered or errored output state, and restores
> `commandname` and `handler`.

**Dash source shape (`eval.evalcase-fn`):**

    STATIC int evalcase(union node *n, int flags)

> [spec:dash:sem:eval.evalcase-fn]
> Evaluate a `case`. Set `errlinno`/`lineno` from the node, adjusting by
> `funcline - 1` when inside a function so the reported line is relative
> to the definition. Expand the subject word with `expandarg` and
> `EXP_TILDE` — plus `EXP_MBCHAR` when the shell is not using libc
> `fnmatch`, so its own matcher receives multibyte markers.
>
> Walk the clauses while `evalskip` is clear, and within each walk the
> patterns, testing with `casematch`. On the first match, evaluate the
> clause body and stop — no fall-through. The body is evaluated only when
> it is non-empty; the guard exists because with `EV_EXIT` an empty body
> would leave the exit status unset. Return the body's status, or 0 if no
> clause matched.

**Dash source shape (`eval.evalcmd-fn`):**

    static int evalcmd(int argc, char **argv, int flags)

> [spec:dash:sem:eval.evalcmd-fn]
> The `eval` builtin. With no operands return 0. With one, evaluate it
> directly. With several, join them with single spaces onto the stack
> (NUL-terminated, then claimed with `grabstackstr`) and evaluate the
> result. Pass only `flags & EV_TESTED` through to `evalstring` — never
> `EV_EXIT`, since `eval` must return to its caller.

**Dash source shape (`eval.evalcommand-fn`):**

    STATIC int evalcommand(union node *, int, struct backcmd *)

> [spec:dash:sem:eval.evalcommand-fn]
> Evaluate a simple command: expand its words, apply assignments and
> redirections, resolve the name, and run it. (The three-argument
> signature above is the `#ifdef notyet` variant; the compiled one is
> `evalcommand(union node *cmd, int flags)`.)
>
> **Line number.** Set `errlinno`/`lineno` from the node, less
> `funcline - 1` when inside a function.
>
> **Word expansion and command-word interpretation.** Expand words
> lazily with `fill_arglist`, which stops as soon as at least one field
> exists, so the *first* word can be resolved before the rest are
> expanded. Loop on that first word: `find_command(..., cmd_flag |
> DO_REGBLTIN, pathval())` — restricted to regular builtins, since only
> those can affect how the remaining words are treated. Stop when it is
> not a builtin. Otherwise record whether the builtin takes assignments
> as arguments (`BUILTIN_ASSIGN` → `pseudovarflag`); on the first
> iteration only, record whether it is special (`BUILTIN_SPECIAL` →
> `spclbltin`) and set `vlocal` to the complement, because a special
> builtin's variable assignments persist while an ordinary one's are
> scoped to it. Note whether it is `exec`. Stop unless it is `command`,
> in which case parse `command`'s own options with `parse_command_args`
> and go round again on the word after them — this is what makes
> `command command exec …` behave.
>
> Then expand the remaining words: with `EXP_VARTILDE` for words that
> look like assignments when `pseudovarflag` is set (so `export
> x=~/bin` expands the tilde), and `EXP_FULL | EXP_TILDE` otherwise.
> Count the fields into `argc`. For `exec` with operands, set
> `vflags = VEXPORT` so assignments are exported into the exec'd image.
>
> **Setup.** `pushlocalvars(vlocal)` opens a scope for the assignments.
> Build `argv` in stack space of `argc + 2` pointers, reserving one slot
> *before* `argv[0]` for `tryexec`'s `#!`-less-script retry. Remember
> `lastarg`, the final argument, when interactive and not in a function,
> to be assigned to `_` at the end. Set `preverrout.fd` to 2, expand the
> redirection targets with `expredir`, push a redirection scope, and
> apply them with `redirectsafe(…, REDIR_PUSH|REDIR_SAVEFD2)`. A
> redirection error sets the status and skips to cleanup — but for a
> special builtin it raises `EXERROR` instead, since a redirection error
> on a special builtin is fatal to the shell.
>
> **Assignments.** Expand each with `EXP_VARTILDE`, then either
> `mklocal(..., VEXPORT)` when `vlocal` (scoped to this command) or
> `setvareq(..., vflags)` when not (persisting).
>
> **Tracing.** With `xflag` and not already inside a `PS4` expansion
> (`inps4` guards against recursion), write the expanded `PS4` followed
> by the assignments and then the arguments to `preverrout`, then a
> newline.
>
> **Resolution.** Unless the first word already resolved to a regular
> builtin, re-run `find_command(argv[0], &cmdentry, cmd_flag | DO_ERR,
> path)` with the real (or `command -p`) path — this is the lookup that
> may report "not found".
>
> **Execution**, by resolved kind:
> - `CMDUNKNOWN` — status 127, flush stderr, go to the failure path.
> - default (an external program) — `flush_input()` to give back
>   read-ahead the child would otherwise lose, then either `shellexec`
>   directly (when `EV_EXIT` is set and no traps are pending, so no fork
>   is needed) or `vforkexec` a child and record the job.
> - `CMDBUILTIN` — `evalbltin`; if it raised, re-raise by
>   `longjmp(handler->loc, 1)` unless the exception was `EXERROR` on a
>   non-special builtin, which is caught here so the shell survives.
> - `CMDFUNCTION` — `evalfun`; re-raise on any exception.
>
> Then `waitforjob(jp)` (a no-op returning the current status when `jp` is
> NULL) and `FORCEINTON`.
>
> **Cleanup**, on every path: `popredir(execcmd)` — dropping rather than
> restoring for `exec`, so its redirections persist — then unwind the
> redirection, input and local-variable scopes to where they were on
> entry, and finally set `_` to `lastarg` if one was recorded.

**Dash source shape (`eval.evalfor-fn`):**

    STATIC int evalfor(union node *n, int flags)

> [spec:dash:sem:eval.evalfor-fn]
> Evaluate a `for` loop. Set the line number as in `evalcase`. Expand
> every word of the list with `EXP_FULL | EXP_TILDE` into one arglist —
> all of them up front, so modifying the variables they reference inside
> the body does not change the iteration. Increment `loopnest`, mask
> `flags` to `EV_TESTED` (`EV_EXIT` must not leak into the body), and for
> each field assign it to the loop variable and evaluate the body. After
> each iteration consult `skiploop()`: any skip other than `SKIPCONT`
> ends the loop. Decrement `loopnest` and return the last body status —
> 0 if the list was empty.

**Dash source shape (`eval.evalfun-fn`):**

    STATIC int evalfun(struct funcnode *func, int argc, char **argv, int flags)

> [spec:dash:sem:eval.evalfun-fn]
> Call a shell function. Returns 0 normally, or the non-zero `setjmp`
> value if an exception propagated out. Save `shellparam`, `funcline`,
> `loopnest` and `handler` (the first and last `volatile`, being live
> across the jump) and install a local `jmploc`.
>
> With interrupts suspended: take a reference on the function
> (`func->count++`) so it survives being redefined or unset while
> running, set `funcline` to its definition line, and reset `loopnest` to
> 0 so a `break` inside cannot escape into an enclosing loop. Set
> `shellparam.malloc = 0` because `argv` is borrowed, then install the
> arguments as the positional parameters (`argc - 1` of them, starting at
> `argv + 1`) and reset the `getopts` cursor.
>
> Evaluate the body with `flags & EV_TESTED` — `EV_EXIT` is dropped so
> the function returns rather than exiting.
>
> On the shared exit path, with interrupts suspended, restore
> `loopnest` and `funcline`, drop the function reference with `freefunc`,
> release and restore the positional parameters, and restore `handler`.
> Finally clear `SKIPFUNC` and `SKIPFUNCDEF` from `evalskip` — a `return`
> stops here and does not propagate further.

**Dash source shape (`eval.evalloop-fn`):**

    STATIC int evalloop(union node *n, int flags)

> [spec:dash:sem:eval.evalloop-fn]
> Evaluate `while` or `until`. Increment `loopnest`, mask `flags` to
> `EV_TESTED`. Each iteration evaluates the condition with `EV_TESTED`
> (so `set -e` does not fire on a false condition), then `skiploop()`. A
> `SKIPFUNC` adopts the condition's status as the loop's result — a
> `return` inside the condition. Any skip at all restarts the loop
> control without running the body. Otherwise invert the condition for
> `until` (`n->type != NWHILE`), and a non-zero result ends the loop.
> Evaluate the body, then `skiploop()` again. The loop continues while
> the skip state is zero or exactly `SKIPCONT`. Decrement `loopnest` and
> return the last body status — 0 if the body never ran.

**Dash source shape (`eval.evalpipe-fn`):**

    STATIC int evalpipe(union node *n, int flags)

> [spec:dash:sem:eval.evalpipe-fn]
> Evaluate a pipeline. Every process is a child of the shell that created
> the pipeline — not of its predecessor, as in some other shells.
>
> Count the elements, add `EV_EXIT` to `flags` (each child is the last
> thing its process does), and with interrupts suspended make a job of
> that size. Track `prevfd`, the read end left over from the previous
> element, starting at -1.
>
> For each element: `prehash` it so the command lookup happens in the
> parent and is inherited; create a pipe unless this is the last element.
> The child enables interrupts, closes the new pipe's read end, and wires
> up its descriptors — `prevfd` onto 0 (preceded by `reset_input()`,
> since stdin is being replaced) and the new write end onto 1, each
> skipped when already correct — then `evaltreenr`, which never returns.
> The parent closes `prevfd` and the write end, and keeps the read end as
> the next `prevfd`.
>
> A pipe-creation failure closes `prevfd` first, then raises
> `sh_error("Pipe call failed")`.
>
> For a foreground pipeline, `waitforjob(jp)` gives the status — which
> for a plain pipeline is the last element's, and with `pipefail` the
> rightmost non-zero. Restore interrupts.

**Dash source shape (`eval.evalstring-fn`):**

    int evalstring(char *s, int flags)

> [spec:dash:sem:eval.evalstring-fn]
> Parse and execute the commands in a string. Copy it onto the stack with
> `sstrdup` — the caller's storage may be reclaimed by the very commands
> being run — and make it the input with `setinputstring`. Take a stack
> mark.
>
> Loop parsing one command at a time until `NEOF`, popping the stack mark
> each iteration so per-command allocation does not accumulate. Evaluate
> each, keeping the status of every non-NULL tree. `EV_EXIT` is cleared
> unless the parser has reached end of input (`parser_eof()`), so only
> the genuinely last command may exec in place. Stop early if `evalskip`
> becomes set.
>
> Then pop the mark, `popfile()` to restore the previous input, and
> `stunalloc(s)` to release the copy. Return the last status.

**Dash source shape (`eval.evalsubshell-fn`):**

    STATIC int evalsubshell(union node *n, int flags)

> [spec:dash:sem:eval.evalsubshell-fn]
> Evaluate a subshell (`NSUBSHELL`) or a backgrounded command
> (`NBACKGND`). Set the line number as in `evalcase` and expand the
> redirection targets in the *parent*, since they may reference state the
> child would not see.
>
> With interrupts suspended: if this is not a background command, the
> caller already intends to exit (`EV_EXIT`), and no traps are pending,
> the fork can be skipped entirely — call `forkreset(NULL)` to reset the
> state a child would reset and fall into the child path in this process.
> Otherwise make a job and `forkshell`.
>
> The child (or the no-fork path) enables interrupts, adds `EV_EXIT`,
> clears `EV_TESTED` for a background command, applies the redirections
> with `redirect(…, 0)` — flags 0, so nothing is saved for restoration —
> and `evaltreenr`, which never returns.
>
> The parent returns 0 for a background command, and otherwise
> `waitforjob(jp)`.

**Dash source shape (`eval.evaltree-fn`):**

    int evaltree(union node *n, int flags)

> [spec:dash:sem:eval.evaltree-fn]
> Evaluate a parse tree; the status is both returned and left in
> `exitstatus`. Take a stack mark. Do nothing under `noexec` (`nflag`) or
> for a NULL node. Run any pending traps first with `dotrap()`.
>
> Dispatch on `n->type`. Most cases set `evalfn` to the specialised
> routine and fall into a shared call; `checkexit` is set to `EV_TESTED`
> for the node types whose failure `set -e` should act on (`NCMD`,
> `NSUBSHELL`, `NBACKGND`, `NPIPE`).
>
> - `NNOT` — evaluate the operand with `EV_TESTED` and logically negate
>   the status, unless a skip is in progress.
> - `NREDIR` — set the line number, expand and push the redirections, and
>   apply them with `redirectsafe`. On failure the status is the
>   redirection error and `checkexit` is set; on success evaluate the
>   body with only `EV_TESTED` retained. Pop the redirections afterwards.
> - `NCMD`, `NFOR`, `NWHILE`/`NUNTIL`, `NSUBSHELL`/`NBACKGND`, `NPIPE`,
>   `NCASE` — delegate to `evalcommand`, `evalfor`, `evalloop`,
>   `evalsubshell`, `evalpipe`, `evalcase`.
> - `NAND`/`NOR`/`NSEMI` — `isor = n->type - NAND`, relying on those
>   three constants being consecutive (enforced by `#error` checks).
>   Evaluate the left side; `((isor >> 1) - 1)` is all-ones for `NAND`
>   and `NOR` and 0 for `NSEMI`, so `EV_TESTED` is forced on for the
>   short-circuit operators and left alone for `;`. Stop if a skip
>   started, or if `(!status) == isor` — i.e. success for `NOR` (1) or
>   failure for `NAND` (0) — otherwise continue into the right side.
>   For `NSEMI` (2) the test can never hold, so both sides always run.
> - `NIF` — evaluate the test with `EV_TESTED`; on zero take `ifpart`,
>   otherwise `elsepart` if there is one, else status 0.
> - `NDEFUN` — `defun(n)`.
> - default — under `DEBUG`, print the node type and `break`. **In a
>   release build the arm is empty and its `break` is compiled out with
>   it, so control falls through into `NNOT`**: an unrecognised node type
>   is evaluated as if it were a negation, dereferencing `n->nnot.com`
>   and inverting the result. Reproduce the fall-through.
>
> Store the result in `exitstatus`. Then `dotrap()` again, so a signal
> that arrived during the command is handled promptly. If `eflag` is set,
> the status is non-zero, and `checkexit` is not masked out by `flags`
> (i.e. the status is not being tested), raise `EXEND` — this is `set -e`.
> `EV_EXIT` raises `EXEND` too. Otherwise pop the stack mark and return
> `exitstatus`.

**Dash source shape (`eval.evaltreenr-fn`):**

    void evaltreenr(union node *n, int flags) #ifdef HAVE_ATTRIBUTE_ALIAS __attribute__ ((alias("evaltree"))); #else

> [spec:dash:sem:eval.evaltreenr-fn]
> `evaltree` declared `noreturn`, for the child paths that always pass
> `EV_EXIT` and therefore never return. Where the compiler supports
> `__attribute__((alias))` it is literally the same function under
> another name; otherwise it is a wrapper that calls `evaltree` and
> `abort()`s if it ever comes back. The point is to let the caller's code
> after the call be eliminated. The generated `def` signature above
> includes the surrounding preprocessor text; the real one is
> `void evaltreenr(union node *n, int flags)`.

**Dash source shape (`eval.execcmd-fn`):**

    int execcmd(int argc, char **argv)

> [spec:dash:sem:eval.execcmd-fn]
> The `exec` builtin. With no operands do nothing and return 0 — the
> redirections have already been applied and made permanent by
> `evalcommand`'s `popredir(execcmd)`. With operands, clear `iflag` and
> `mflag` so that a failed exec exits rather than returning to a prompt,
> `optschanged()` to propagate that, `flush_input()` to give back
> read-ahead, and `shellexec(argv + 1, pathval(), 0)`, which does not
> return.

**Dash source shape (`eval.expredir-fn`):**

    STATIC void expredir(union node *n)

> [spec:dash:sem:eval.expredir-fn]
> Expand the target of each redirection in a list, before any of them is
> applied. For the file forms (`NFROMTO`, `NFROM`, `NTO`, `NCLOBBER`,
> `NAPPEND`) expand the filename with `EXP_TILDE | EXP_REDIR` and store
> the single resulting field in `nfile.expfname`. `EXP_REDIR` suppresses
> field splitting, so an unquoted filename containing `IFS` characters
> still names one file. For the duplicating forms (`NFROMFD`, `NTOFD`)
> with a variable target, expand it and hand the text to `fixredir`,
> which re-interprets the node as a numeric duplication or a close.

**Dash source shape (`eval.falsecmd-fn`):**

    int falsecmd(int argc, char **argv)

> [spec:dash:sem:eval.falsecmd-fn]
> Return 1.

**Dash source shape (`eval.fill-arglist-fn`):**

    static struct strlist *fill_arglist(struct arglist *arglist, union node **argpp)

> [spec:dash:sem:eval.fill-arglist-fn]
> Expand words from `*argpp` into `arglist` until at least one new field
> has been produced, advancing `*argpp` past each word consumed. Remember
> the list tail on entry and stop as soon as something has been appended
> there. Returns the first new field, or NULL when the words ran out
> without producing any. The point is laziness: `evalcommand` needs the
> command name before it knows how to expand the rest, and a word can
> expand to zero fields (`$@` with no parameters), so "one word" is not
> the same as "one field".

**Dash source shape (`eval.parse-command-args-fn`):**

    static int parse_command_args(struct arglist *arglist, union node **argpp, const char **path)

> [spec:dash:sem:eval.parse-command-args-fn]
> Parse the options of the `command` builtin from the already-expanded
> fields, pulling more in with `fill_arglist` as needed. Returns
> `DO_NOFUNC` when a command word follows — telling `evalcommand` to
> resolve it while ignoring shell functions, which is `command`'s purpose
> — and 0 when it should instead fall through to `typecmd` (for `-v`/`-V`)
> or there is nothing left.
>
> Walk the fields: stop at the first that does not begin with `-`, or is
> exactly `-`. An exact `--` consumes itself and stops. Otherwise process
> the letters: `p` sets `*path = defpath`, and anything else returns 0 so
> the generic option handling deals with it. On success, advance
> `arglist->list` past the options so the caller sees the command word
> first.

**Dash source shape (`eval.prehash-fn`):**

    STATIC void prehash(union node *n)

> [spec:dash:sem:eval.prehash-fn]
> Resolve a simple command's name before forking, so the hash-table entry
> is created in the parent and inherited by every child of a pipeline
> rather than being recomputed in each. Only acts on `NCMD` nodes with
> arguments, and only when `goodname` says the first word is a plain name
> that expansion cannot change — a conservative test, since expanding it
> here would have side effects.

**Dash source shape (`eval.returncmd-fn`):**

    int returncmd(int argc, char **argv)

> [spec:dash:sem:eval.returncmd-fn]
> The `return` builtin. With an operand, set `evalskip = SKIPFUNC` and
> return `number(argv[1])`. With none, set `evalskip = SKIPFUNCDEF` and
> return the current `exitstatus`. The two skip codes differ in reach:
> `SKIPFUNC` is cleared by `evalfun`, so it returns from a function,
> while `SKIPFUNCDEF` also abandons the rest of the current file — which
> is ksh's behaviour for `return` outside a function.

**Dash source shape (`eval.skiploop-fn`):**

    static int skiploop(void)

**Retired evaluator flag plumbing (`eval.skiploop-fn`):**
> Consume one loop level's worth of a pending `break`/`continue` and
> report what remains. With no skip pending, return 0. For `SKIPBREAK` or
> `SKIPCONT`, decrement `skipcount`; when it reaches zero this is the
> level being broken out of, so clear `evalskip` — but **return the
> original `skip` unchanged**, not 0. The C clears `evalskip` and then
> `break`s out of the switch to the shared `return skip`. This is load
> bearing: `evalloop`'s trailing `while (!(skip & ~SKIPCONT))` uses the
> returned value to tell a finished `break` (leave the loop) from a
> finished `continue` (run it again). Returning 0 here would make
> `break` spin forever. Otherwise
> return `SKIPBREAK` regardless of which it was — an unfinished
> `continue` behaves as a break at intermediate levels, since the
> `continue` applies to an outer loop. Any other skip (`SKIPFUNC`,
> `SKIPFUNCDEF`) is returned unchanged for the caller to propagate.

**Dash source shape (`eval.truecmd-fn`):**

    int truecmd(int argc, char **argv)

> [spec:dash:sem:eval.truecmd-fn]
> Return 0.
