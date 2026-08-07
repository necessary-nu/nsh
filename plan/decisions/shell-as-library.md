---
id [dec:nsh:shell-as-library]
epitome "nsh is a shell library; the binary is a thin frontend over it."
state @decided
category @existence
scope {
    elements ([arch:nsh:shell-core] [arch:nsh:shell-bin])
}
author "brendan@necessary.nu"
alternatives (
    {
        option "Ship a shell binary and expose nothing."
        rejected_because "It is the shape dash already has, and it is the reason there is no way to embed a POSIX shell in a Rust program today without spawning one and talking to it over a pipe."
    }
    {
        option "Expose the library but keep the process-global state, so an embedder gets exactly one shell per process."
        rejected_because "It reads as a library and behaves as a program. An embedder discovers the constraint the second time it constructs a shell, at which point the two share variables, a stack allocator and an exception handler."
    }
)
consequences {
    accepted (
        "Idiomatization is not cosmetic. Four properties of the literal port have to change before the library is honest, and they are the four decisions this one requires."
        "The binary keeps doing exactly what dash does. Everything a library may not do -- exit, signals, argv, the standard descriptors -- moves INTO the frontend rather than away."
    )
    deferred ("Until those four land, `dash` the crate is a library only in the Cargo sense: it has a lib target, and embedding it twice in one process is undefined.")
}
establishes ([arch:nsh:shell-core] [arch:nsh:shell-bin])
---

## Rationale

A POSIX shell is a parser, an expander, an executor and a set of
built-ins. None of that is inherently a program. What makes dash a
program is the handful of places it assumes it *is* the process: it
exits, it takes the signal dispositions, it reads descriptor 0, and it
keeps every piece of state in a static.

Separating those is worth doing on its own terms -- a shell you can
embed is a different thing from a shell you can spawn: no pipe, no
quoting round-trip, no second process, and errors that arrive as values
rather than as an exit status and some text on stderr.

It also gives idiomatization a definition it otherwise lacks. "Make it
idiomatic" is a matter of taste and stops nowhere in particular.
"Make it embeddable" is four specific, checkable properties, and each
one is a refactor whose success is decided by the differential harness
rather than by opinion.

The boundary is the point. `shell-core` may not terminate the process,
claim a signal, assume a descriptor, or hold state that a second
instance would share. `shell-bin` does all four, because that is what a
shell frontend is for, and it is where dash's observable behaviour has
to be preserved exactly.
