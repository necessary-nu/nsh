# src/error.c, src/error.h

Exceptions are implemented with `setjmp`/`longjmp`. The global `handler`
points at the innermost active `struct jmploc`; nesting is done by
saving `handler`, pointing it at a local `jmploc`, and restoring it on
scope exit. `exception` carries the code across the jump: `EXINT` (0,
SIGINT received), `EXERROR` (1, generic error), `EXEND` (3, exit the
shell), `EXEXIT` (4, exit via the `exit` builtin).

Interrupts are deferred rather than blocked. `suppressint` is a nesting
counter incremented by `INTOFF`; `intpending` is set by the SIGINT
handler when `suppressint` is non-zero. `INTON` decrements the counter
and, if it reaches zero with `intpending` set, runs the deferred `onint`.
Both macros carry a compiler `barrier()` so the compiler cannot hoist
work across the critical section. `errlinno` holds the line number
reported in diagnostics.

`errmsg`'s `action` argument is a mask of `E_OPEN` (01), `E_CREAT` (02)
and `E_EXEC` (04).

**Dash source shape (`error.errmsg-fn`):**

    const char * errmsg(int e, int action)

> [spec:dash:sem:error.errmsg-fn]
> Map an `errno` value to a message, tailored to what the shell was
> trying to do. For any `e` other than `ENOENT` and `ENOTDIR`, return
> `strerror(e)`. For those two, ignore the system text and pick by
> `action`: `"No such file"` if `E_OPEN` is set, else
> `"Directory nonexistent"` if `E_CREAT` is set, else `"not found"` —
> the message a failed command lookup produces. The returned pointer may
> be a static buffer (`strerror`'s) that the next call overwrites, so
> callers must not retain it.

**Dash source shape (`error.exerror-fn`):**

    void exerror(int cond, const char *msg, ...)

**Retired C exception plumbing (`error.exerror-fn`):**
> Variadic front end that raises exception `cond` with a printf-style
> message: collect the arguments and hand them to `exverror(cond, msg, ap)`.
> Does not return. Unlike `sh_error` it leaves `exitstatus` alone, so the
> caller controls the resulting status — used where the exception is not
> a plain error, e.g. raising `EXEXIT`.

**Dash source shape (`error.exraise-fn`):**

    void exraise(int e)

**Retired C exception plumbing (`error.exraise-fn`):**
> Raise exception `e` by longjmp'ing to the innermost handler. Under
> `DEBUG`, `abort()` first if `handler` is NULL, since there is nowhere
> to jump. If `vforked` is set the process is a vfork child sharing the
> parent's address space, so unwinding is not possible — `_exit`
> immediately with the current `exitstatus` instead. Otherwise `INTOFF`
> (the handler re-enables interrupts once it has restored its state),
> store `e` into the global `exception`, and `longjmp(handler->loc, 1)`.
> Does not return.

**Dash source shape (`error.exverror-fn`):**

    static void exverror(int cond, const char *msg, va_list ap)

**Retired C exception plumbing (`error.exverror-fn`):**
> Report and raise. Under `DEBUG`, trace the call — the message is
> rendered through a `va_copy` so the original `ap` stays usable. If
> `msg` is non-NULL, print it via `exvwarning`. Then `flushall()` so the
> diagnostic and any pending output reach their descriptors before
> control leaves, and `exraise(cond)`. Does not return.
>
> The `exvwarning(a, b, c)` macro discards its first argument and calls
> `exvwarning2(b, c)`; `-1` is passed as a vestigial line-number slot.
> Note that in a non-`DEBUG` build the `if (msg)` guard is compiled out
> along with the tracing, so `exvwarning` is called unconditionally —
> `exverror` is only ever reached with a non-NULL `msg` in that
> configuration.

**Dash source shape (`error.exvwarning2-fn`):**

    static void exvwarning2(const char *msg, va_list ap)

> [spec:dash:sem:error.exvwarning2-fn]
> Write one diagnostic line to `out2`. The prefix is the shell name —
> `arg0`, or `"sh"` if that is NULL — then `errlinno`, then, when
> `commandname` is set, the current command name: format
> `"%s: %d: "` without it and `"%s: %d: %s: "` with it.
>
> Note the call is a single `outfmt(errs, fmt, name, errlinno, commandname)`
> in both cases: `commandname` is passed even when the chosen format has
> only two conversions, and C simply ignores the surplus argument. A port
> whose formatter validates argument count against the format must
> tolerate the extra argument rather than treat it as an error. Follow with the
> caller's message rendered by `doformat(errs, msg, ap)`, then a newline.
> The newline goes through `outc` when `FLUSHERR` is configured and
> `outcslow` otherwise, so that on an unbuffered stderr the line is
> pushed out immediately rather than waiting for a later flush.

**Dash source shape (`error.inton-fn`):**

    void __inton()

**Retired C exception plumbing (`error.inton-fn`):**
> Out-of-line body of the `INTON` macro, compiled only under
> `REALLY_SMALL` where the macro expands to a call instead of inline
> code: decrement `suppressint`, and if it reached zero while
> `intpending` is set, run `onint()` to deliver the deferred SIGINT.
> Behaviour is identical to the inline form; only code size differs.

**Dash source shape (`error.jmploc`):**

    struct jmploc {
      jmp_buf loc;
    }

**Dash source shape (`error.onint-fn`):**

    void onint(void)

> [spec:dash:sem:error.onint-fn]
> Deliver a SIGINT that was received (or deferred). Clear `intpending`
> and `sigclearmask()` to unblock signals. Unless this is the interactive
> root shell (`rootshell && iflag`), the shell must die by the signal so
> its parent sees a signal death rather than an exit code: restore
> `SIG_DFL` for SIGINT and `raise(SIGINT)`. In the interactive root shell
> that is skipped, so the shell survives and returns to the prompt. Set
> `exitstatus = SIGINT + 128` and `exraise(EXINT)`. Does not return.
>
> Called from `trap.c` on SIGINT, but only when the user has not trapped
> or ignored SIGINT with the `trap` builtin.

**Dash source shape (`error.sh-error-fn`):**

    void sh_error(const char *msg, ...)

**Retired C exception plumbing (`error.sh-error-fn`):**
> Report a shell error and unwind. Set `exitstatus = 2`, then collect the
> variadic arguments and call `exverror(EXERROR, msg, ap)`. Does not
> return. This is the ordinary error path for the shell proper; external
> builtins use `sh_warnx` instead when they intend to continue.

**Dash source shape (`error.sh-warnx-fn`):**

    void sh_warnx(const char *fmt, ...)

> [spec:dash:sem:error.sh-warnx-fn]
> Print a warning to `out2` in the same prefixed format as an error, then
> return normally — no exception, no change to `exitstatus`. Collect the
> variadic arguments and pass them to `exvwarning`. Used by external
> builtins and by code that must report a problem but keep going.
