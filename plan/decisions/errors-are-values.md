---
id [dec:nsh:errors-are-values]
epitome "A shell error is a value the library returns, not an unwind through the host."
state @decided
category @property
scope {
    elements ([arch:nsh:shell-core])
}
author "brendan@necessary.nu"
alternatives (
    {
        option "Keep catch_unwind and document that callers must not let it escape."
        rejected_because "It makes every entry point a panic hazard for the host, and it forces `panic = \"unwind\"` on anything that links the crate."
    }
)
consequences {
    accepted (
        "The Cargo profile constraint goes away. Today both profiles pin `panic = \"unwind\"` because the exception mechanism REQUIRES it; an embedder building with `panic = \"abort\"` would find every shell error aborting the process."
        "The frontend keeps the current behaviour: dash's exit statuses and diagnostics are observable and must not change."
        "**A diagnostic is written where dash writes it, AND the error is returned as a value.** `tests/harness/dscase.sh:64-71` runs every case with `2>&1` and compares the merged stream, so the interleaving of the shell's diagnostics with command output is under test in all 61,498 cases. A design in which the library returns an error and the frontend prints it emits every diagnostic at the end of the run instead of at the point of failure, and fails thousands of cases at once. The value is for the embedder and for control flow; it is not the delivery mechanism for the text. An embedder wanting structure rather than bytes gets a diagnostic hook."
        "**Control flow is not an error.** The mechanism conflates three unrelated things behind four integers (`EXINT`, `EXERROR`, `EXEND`, `EXEXIT`, `error.rs:77-81`), and only one of them is an error. Diagnostics -- \"not found\", \"Bad substitution\", a syntax error -- are `Error`. `exit`, `return`, `break`, `continue`, the `set -e` abort, `EXEND` and `EXEXIT` are control flow and must not sit in the `Err` position: a `Result` whose `Err` includes \"the script called `exit 0`\" makes every caller wrong by default. `EXINT` is a third thing again, asynchronous and signal-delivered, and a host has to be able to tell \"your script failed\" from \"the user hit ^C\"."
    )
    deferred (
        "The unwind stays until then, and with it the constraint on any binary that links this crate."
        "Whether `set -e` survives this taxonomy is not settled. It decides whether an error terminates from syntactic context -- a command in a `while` condition, a `!`, a `&&` left operand -- and dash carries that as `EV_TESTED` flags through `evaltree`. Whether it stays a flag on the call or becomes a property of the error is an open question; see `docs/idiomatization.md` §7.4."
    )
}
edges {
    requires ([dec:nsh:shell-as-library])
}
---

## Rationale

C's `longjmp` became `catch_unwind` over a typed payload, because that
is the honest translation: every shell error, interrupt, `exit` and
`set -e` raise is one non-local jump, and Rust's only non-local jump is
an unwind.

It is also a decision the host has to live with. `Cargo.toml` pins
`panic = "unwind"` on both profiles because the mechanism requires it --
so linking this crate constrains the host's panic strategy, and a host
that chose `abort` would see every ordinary shell error take the process
down.

That pin carries no comment explaining itself, deliberately: manifests
say what, not why. This decision is the why, and it is the thing to read
before changing either profile. `panic = "abort"` does not fail to
build; it silently breaks every shell exception path -- errors,
interrupts, `exit`, and the `set -e` unwind -- so the pins must not come
out until this decision is implemented.

Two bugs on this mechanism are worth remembering, because both were
about a jump crossing a boundary it should not have: a subshell in an
EXIT trap unwound past `main` and exited 101, and `onint` aborted the
process because an unwind cannot leave an `extern "C"` frame. Both are
fixed. Both would have been ordinary control flow in a design where an
error is a returned value.
