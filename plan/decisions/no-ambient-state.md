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
        "Nearly every function in the crate changes shape. This is the largest single piece of wave 4."
        "The unit tests' `testutil::lock()` becomes unnecessary: tests serialise today only because the state is shared."
    )
    deferred ("Until it lands, `cargo test` must keep serialising anything that touches shell state, and an embedder gets one shell per process.")
}
edges {
    requires ([dec:nsh:shell-as-library])
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
