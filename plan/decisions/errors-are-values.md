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
    )
    deferred ("The unwind stays until then, and with it the constraint on any binary that links this crate.")
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
`panic = "unwind"` on both profiles with a comment saying the mechanism
requires it -- so linking this crate constrains the host's panic
strategy, and a host that chose `abort` would see every ordinary shell
error take the process down.

Two bugs on this mechanism are worth remembering, because both were
about a jump crossing a boundary it should not have: a subshell in an
EXIT trap unwound past `main` and exited 101, and `onint` aborted the
process because an unwind cannot leave an `extern "C"` frame. Both are
fixed. Both would have been ordinary control flow in a design where an
error is a returned value.
