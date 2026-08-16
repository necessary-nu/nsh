---
id [dec:nsh:fork-child-is-a-terminus]
epitome "A forked child is a terminus, not a frame: it never returns, and under `vfork` it writes nothing the parent reads."
state @decided
category @property
scope {
    elements ([arch:nsh:shell-core])
}
author "brendan@necessary.nu"
alternatives (
    {
        option "Let the forked child hand its `Flow` back through the call it forked in, like any other frame."
        rejected_because "The frames between the child and `main` are the *parent's*, copied by `fork`. Returning through them unwinds a stack the child does not own, running the parent's destructors on the parent's objects in a process that is about to die. `errors-are-values` found this the hard way and recorded it at `exec.rs:264-270` and `eval.rs:748`; `evalbackcmd`'s child is the sharpest case, because it sits under the whole expansion chain, which has no business carrying control flow that exists only on the far side of a fork."
    }
    {
        option "Delete `vfork` and use `fork` everywhere, so the shared-address-space question stops existing."
        rejected_because "The constraint it imposes is thirty lines of prologue and it is now audited end to end -- one violation found, one line fixed. Giving up a fast path dash relies on, to avoid an audit that has been done, is paying a permanent cost for a one-off. The audit is in this file and at `jobs.rs`'s fork boundary so the next reader does not have to redo it."
    }
    {
        option "Replace the fork/exec pair with `posix_spawn`."
        rejected_because "It covers the external-command case and not the others. `posix_spawn` file actions do `open`/`dup2`/`close` and `POSIX_SPAWN_SETPGROUP` does the process group, but it cannot perform the `tcsetpgrp` handoff between the two, and -- decisively -- **a subshell is not an exec**. `( ... )`, a pipeline stage and `$( ... )` all fork into a child that keeps interpreting, and there is no spawn primitive for \"become a copy of me\"."
    }
    {
        option "Make the forked child async-signal-safe, so `fork` from a multithreaded host is sound."
        rejected_because "Unattainable by construction, not by effort. A subshell *is* a shell: it evaluates, so it allocates. `forkchild`'s `freejob` walk frees the job table's owned strings before the child runs a single command. What can be done is to state the precondition honestly and put it on the embedder, which is what `Command::pre_exec` does for the same reason."
    }
)
consequences {
    accepted (
        "**A forked child ends with `_exit` and the ending is the child's own.** Three named endings and nothing else: `shellmain::exit_from_child` for a child that ran script, `jobs::forkchild_fatal` for one whose prologue failed, and `redir.rs:483` for the here-document writer. None of them is `Host` business -- ending a process the library made is not ending the host's, so [dec:nsh:host-owns-the-process]'s ban does not reach here and the `Host` trait needs no method for it."
        "**Under `vfork` the rule is stronger, and it is now audited rather than asserted.** The prologue, in the order the child runs it: the `mypid = 0` / `shlvl += 1` / `forkreset` / `jobctl = 0` block is skipped on `lvforked`; `setpgid` and `tcsetpgrp` are syscalls on the child, not memory; `setsignal` and `ignoresig` carry their own `lvforked` guard on the `sigmode` store (`trap.rs:306`, `:326`) and `sigaction` is per-process, so the child's dispositions are the child's; the `freejob` walk is skipped on `lvforked`. **One violation, and it was dash's**: `forkchild` wrote `mypid = pgrp = getpid()` unguarded (`src/jobs.c:891`), which the parent reads again at the next `vforkexec`. Fixed."
        "**Two writes are permitted and named rather than removed.** `forkchild_fatal`'s and `shellexec`'s `sh.status = ...`, each immediately before its own `_exit`. The parent overwrites `sh.status` from the child's wait status in `waitforjob` before anything reads it; and `shellexec`'s failure path has to build and write dash's diagnostic where dash writes it, which allocates in the shared heap whatever is done about the field. A rule with a stated exception is worth more than a rule that is quietly false."
        "**The child-side signal work is the library's and must not go through `Host`.** Counted rather than estimated: **21 disposition call sites, of which 12 run in a child the library just forked** -- `forkchild`'s two `setsignal`s in the job-control arm (`jobs.rs:1020-1021`), its two `ignoresig`s in the `FORK_BG` arm (`:1023-1024`), its three `setsignal`s in the interactive arm (`:1045-1047`), and the here-document writer's five raw `libc::signal` calls (`redir.rs:476-480`). Those change nobody's dispositions but the child's. Routing them through a `dyn Host` would be both pointless and unsound: it is an indirect call into embedder code, made in a forked child, and in the here-document case in a child of a possibly-multithreaded host. **The nine host-side sites are what `public-api`'s `Host` must cover**: `setjobctl`'s three (`jobs.rs:478-480`), `setinteractive`'s three (`trap.rs:491-493`), `trapcmd`'s (`builtins/trap.rs:111`), `mkinit_init`'s `SIGCHLD` (`trap.rs:149`), and `onint`'s `signal(SIGINT, SIG_DFL)` before it re-raises (`error.rs:274`), which `docs/api-design.md` 3.4 already assigns to the frontend."
        "**`clear_traps` is on both sides and follows the path it is on.** Its `setsignal` loop (`trap.rs:188`) is reached from `forkreset`, and `forkreset` has two callers: `forkchild` (`jobs.rs:972`), where it is child-side, and `evalsubshell`'s no-fork arm (`eval.rs:718`), where it runs in the *shell's own* process because the shell is about to exit or exec anyway. The second is reachable only under `EV_EXIT`, so [dec:nsh:host-owns-the-process]'s rule that `Shell::run` passes no `EV_EXIT` retires it from the API surface as a side effect: from `run`, `forkreset` is always child-side."
        "**The fork precondition belongs to the embedder and has to be written down.** `fork` from a multithreaded process carries only the calling thread into the child, and the library's children allocate before they exec -- or never exec at all. So a `Shell` in a multithreaded host inherits the standard `fork` caveat: another thread holding the allocator lock at the moment of the fork deadlocks the child. This is not removable and it is not new; it is `Command::pre_exec`'s caveat with the same cause. It goes in the crate docs, next to 6's process-wide facts."
        "**The `vfork` window is narrower than the fork one and that is why it survives.** Between `vfork` and `execve` the child runs `forkchild`'s early return and `shellexec`'s `PATH` walk. The walk calls `execve` and reads `padvance`'s buffers; it allocates only on the failure path, where it is already building a diagnostic. So the shared-address-space exposure is bounded by \"the command was not found\", which is exactly where an embedder would tolerate it least and where dash has always been."
        "**What the syscall floor inherits from this.** The wrappers a forked child calls -- `execve`, `_exit`, `dup2`, `close`, `open`, `setpgid`, `tcsetpgrp`, `signal`, `sigaction` -- must be callable **without allocating** and must not return a boxed or formatted error. A floor crate whose error type is `Box<dyn Error>` or that formats a message on failure cannot be used between `vfork` and `execve`. Raw `errno` in the error, and formatting at the call site, is the shape [dec:nsh:printf-is-parsed-not-interpreted] and `output-is-a-writer` already established for other reasons."
    )
    deferred ("Whether the `vfork` audit stays true is a property of the prologue, and nothing enforces it. It is thirty lines and it is commented at the boundary; if `forkchild` grows, the audit is re-run rather than trusted. A `debug_assert` cannot express \"writes no location the parent reads\", so this is a reading obligation and is recorded as one.")
}
edges {
    requires ([dec:nsh:errors-are-values] [dec:nsh:shell-as-library])
    related_to ([dec:nsh:host-owns-the-process] [dec:nsh:host-owns-signals] [dec:nsh:minimal-unsafe])
}
establishes ([arch:nsh:shell-core])
---

## Rationale

`errors-are-values` established that a shell error is a value the library
returns. The forked child is the one place where that is false, and it is
false for a reason no amount of design removes: **the frames it would
return through belong to another process.**

`fork` copies the parent's stack. The child's `evaltree` sits below the
parent's `evalcommand`, below the parent's `evaltree`, below the parent's
`main`. A `Result` travelling out of the child walks that copy, running
`Drop` on objects the parent still owns in its own copy, and arrives at a
`main` that will then try to carry on being a shell. dash's answer was
`longjmp` to a handler that `_exit`s; the port's is a function that
`_exit`s. Both are the same admission: this is not a frame, it is an
ending.

So the child's contract is one sentence -- *it does not come back* -- and
the rest of this decision is about what it may touch on the way out.

## Two levels, because there are two calls

**After `fork`, the child owns its address space.** It may allocate,
because it must: `evalsubshell`, `evalpipe` and `evalbackcmd` all fork
into a child that goes on interpreting, and interpreting allocates.
What it may not do is *return*. That is the whole of the rule, and
`exit_from_child` is its implementation -- exact rather than approximate,
because `forkchild`'s `shlvl += 1` makes `main`'s handler take `goto exit`
for every outcome a child can produce.

**After `vfork`, the child owns nothing.** It writes the parent's memory
by definition, and the parent is suspended and will resume expecting its
own state. The rule that makes this survivable is not "touch nothing" --
the child has to `setpgid`, hand over the terminal and set its own
dispositions -- but:

> **The vforked child writes no location the parent reads again.**

That is checkable, and it has been checked. The audit is in the
consequences above and at `vforkexec`'s fork boundary in the source. It
found one violation, in `forkchild`'s job-control arm, and it was
inherited from `src/jobs.c:891` rather than introduced by the port. Every
other write on the path was already guarded -- which is the interesting
result, because it means dash's authors had the rule and applied it four
times out of five.

## The defect the audit found, since a one-line fix deserves its reasoning

`forkchild` wrote `mypid = pgrp = getpid()` with no `lvforked` guard. In a
vforked child that stores the *child's* pid into the *parent's* `mypid`.

The parent reads it again: `vforkexec`'s `if mypid == 0 { mypid =
getpid() }` never re-runs, so the next `vfork` publishes a stale pid as
`vforked`. `onsig`'s test is `vforked != 0 && getpid() != vforked` -- the
question "am I the vforked child?" -- and with a stale value the *parent*
answers yes. Any signal arriving between `vfork` returning and
`set_vforked(0)` is dropped.

The window is a few instructions and the trigger is a foreground external
command under `set -m`, so this is not a bug anybody was going to hit. It
is recorded at length anyway, because the value of the audit is the
method: five writes, five guards expected, four found. A rule that is
checked is worth having; a rule that is asserted is decoration.

## Why the child-side signal calls stay in the library

`[dec:nsh:host-owns-signals]` says the library never installs a handler on
its own authority, and that is right about the *host's* dispositions. It
is wrong about a forked child's, and the difference matters for how much
work `public-api` has in front of it.

Twelve of the twenty-one disposition sites in the crate are in a child the
library just made. `forkchild` sets SIGTSTP and SIGTTOU for a job it is
about to become, ignores SIGINT and SIGQUIT for a background job, and
restores the interactive three; the here-document writer ignores INT,
QUIT, HUP and TSTP and puts SIGPIPE back to `SIG_DFL` so that a reader
closing the pipe kills it. None of that is visible to the host, because
the host is not in that process.

And routing them through `Host` would be actively wrong. It is a virtual
call through a `Box<dyn Host>` -- an indirect call into code the embedder
wrote -- performed in a forked child, and in the here-document case in a
child of a possibly-multithreaded host. That is the async-signal-safety
hazard this decision exists to bound, arrived at from the other direction.

So the split is: **the host owns dispositions on the host's process; the
library owns dispositions on children it made.** `public-api`'s trait
covers the first set, and that set is nine call sites, not twenty-one.
