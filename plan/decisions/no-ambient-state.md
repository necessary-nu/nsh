---
id [dec:nsh:no-ambient-state]
epitome "Shell state belongs to a shell instance: no globals, and no thread-locals either."
state @decided
category @property
scope {
    elements ([arch:nsh:shell-core])
}
author "brendan@necessary.nu"
alternatives (
    {
        option "Keep the statics and document 'one shell per process'."
        rejected_because "The compiler cannot enforce it, so the failure is two live shells quietly sharing a variable table. Nothing about the API says so."
    }
    {
        option "Move the statics into `thread_local!`."
        rejected_because "It buys soundness, not an instance. The compiler stops complaining and two shells on one thread still share everything, which is the case that matters -- an embedder driving several shells does not get a thread each. It would also make the state invisible to the type system a second time, having just been made visible."
    }
    {
        option "Thread a context parameter through every function."
        rejected_because "Not rejected -- this is the likely mechanism. Recorded as an alternative only because the cost is real: it touches nearly every function in the crate, which is why it belongs in idiomatization and not before."
    }
)
consequences {
    accepted (
        "Independence includes the C library's locale. [dec:nsh:per-shell-locale] gives each Shell an owned locale object and confines any thread-locale selection to a restoring platform guard; no durable shell state lives in a Rust or libc thread-local. Other process-wide facilities remain separate questions governed by their own decisions."
        "Nearly every function in the crate changes shape. This is the largest single piece of wave 4."
        "The unit tests' `testutil::lock()` becomes unnecessary: tests serialise today only because the state is shared."
    )
    deferred (
        "The current directory, process group, controlling terminal, child-reaping pool and signal inbox remain process-wide for reasons that are not storage placement and are recorded by their owning decisions."
        "Corrected 2026-09-03; the sentence above is kept as written. Two of its terms have moved. The child-reaping pool is not process-wide any more: `750758b` gives each Shell the pids it forked and asks `waitpid(pid)` after those and no others, and `docs/api-design.md` s6 struck its entry on 2026-09-02. The file-creation mask belonged in the list from the start and was never in it: `umask(2)` is per-process, the `umask` builtin writes it, every child inherits it, and it does not divide -- `open` and `mkdir` take a mode but the kernel applies `mode & ~umask` to it, so a caller asking for 0o700 under a 0o777 mask gets 0o000. Reading it is a write as well, POSIX offering no read: `creation_mask()` is `umask(0)` followed by `umask(saved)`, and whatever another thread creates in that window is created unmasked."
    )
}
edges {
    requires ([dec:nsh:shell-as-library])
    related_to ([dec:nsh:per-shell-locale])
}
---

## Rationale

The literal port keeps the shell's variables, its stack allocator, its
open files, its trap table and its exception handler in `static mut`.
That is faithful: the C does the same, and for a program it is
unremarkable.

For a library it is the difference between an API and a global.
Two shells in one process would share a variable table and a stack
allocator; one shell in a process that also does other things can have
its state mutated by anything else linked in.

The scaffolding this already forces is visible in the test suite.
`testutil::lock()` exists because cargo runs tests on several threads in
one process and the state is shared -- every test that touches the shell
has to hold a mutex. That is the embedder's problem in miniature, and it
arrived before any embedder did.

Selecting a POSIX locale for one bounded operation does not weaken the ban on
thread-local shell state. The selected handle is borrowed from a `Shell`, the
previous selection is restored before the operation returns, and no later call
can discover which Shell ran last. [dec:nsh:per-shell-locale] owns that ABI
mechanism and its lifetime constraints.
