# src/init.h

These five functions have no hand-written body. `mkinit` scans every
`.c` file for blocks introduced by the keywords `INIT`, `EXITRESET`,
`FORKRESET`, `POSTEXITRESET` and `RESET` (each written at column 0 inside
`#ifdef mkinit`), concatenates the bodies it finds in file order, and
emits `init.c` containing one function per event. `init.h` declares them;
the bodies exist only in generated code. See `mkinit.md` for how the
generation works.

For nsh the behavioral contract is "run these fragments, in this order, at
this lifecycle point". Explicit Rust subsystem calls replace the code
generator while preserving the observable order and effects. The historical
block locations are listed per event
below, in the order `mkinit` visits them — the order of `dash_CFILES` in
`src/Makefile.am`, which is *not* alphabetical.

**Dash source shape (`init.exitreset-fn`):**

    void exitreset(void)

> [spec:dash:sem:init.exitreset-fn]
> Called when an error or interrupt in an interactive shell returns
> control to the main command loop, *before* `exitshell` — and also on
> the way out of the shell. Runs the `EXITRESET` fragments in
> `dash_CFILES` order — **`eval.c`, `expand.c`, `redir.c`**: unwind the
> `evaltree` bookkeeping (`savestatus`, `evalskip`, `loopnest`, `inps4`
> and the command-substitution pipe); release the IFS/expansion working
> state with `ifsfree()`; and discard every saved redirection with
> `unwindredir(0)`. The distinction from `reset` is that
> `exitreset` undoes things that must be undone even when the shell is
> about to exit, whereas `reset` restores state only needed for
> continuing.

**Dash source shape (`init.forkreset-fn`):**

    void forkreset(union node *)

> [spec:dash:sem:init.forkreset-fn]
> Called immediately after entering a subshell, in the child. Runs the
> `FORKRESET` fragments in `dash_CFILES` order — **`input.c`, `main.c`,
> `redir.c`, `trap.c`**: reset the input stack so the child does not
> share the parent's pushed-back input, closing its descriptor and the
> stdin tee pipe; re-point `handler` at `main_handler` so the child
> unwinds to its own top level; discard the saved redirection state the
> child must not restore; and clear the trap handlers a subshell does
> not inherit. It is the only
> event whose routine takes a parameter — the `union node *` for the
> command being run in the subshell.

**Dash source shape (`init.init-fn`):**

    void init(void)

> [spec:dash:sem:init.init-fn]
> One-time startup, called from `main` before anything else. Runs the
> `INIT` fragments in `dash_CFILES` order — **`input.c`, `trap.c`,
> `output.c`, `var.c`**: establish the base input file; record the
> initial signal dispositions so `trap` can report and restore them, and
> set up SIGCHLD; bind the output structures to their streams (under
> `USE_GLIBC_STDIO`); and build the variable table from the environment,
> set the shell's own defaults and establish `PWD`.

**Dash source shape (`init.postexitreset-fn`):**

    void postexitreset(void)

> [spec:dash:sem:init.postexitreset-fn]
> Called from `exitshell`, after the exit trap has run. Runs the single
> `POSTEXITRESET` fragment, in `input.c`, which tears down the input
> stack. Separated from `exitreset` so it happens after any code that
> could still need to read input.

**Dash source shape (`init.reset-fn`):**

    void reset(void)

> [spec:dash:sem:init.reset-fn]
> Called when an error or interrupt in an interactive shell returns
> control to the main command loop and the shell intends to keep running.
> Runs the `RESET` fragments in `dash_CFILES` order — **`input.c`,
> `output.c`, `var.c`**: discard buffered and pushed-back input so the
> next read starts at a clean line; reset the output structures (under
> `notyet`, restoring `out1`/`out2` and releasing the memory sink); and
> unwind every local-variable scope with `unwindlocalvars(0)`.
