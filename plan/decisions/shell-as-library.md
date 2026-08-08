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
        "**Those four are necessary and not sufficient.** They describe what the library stops doing to the process and say nothing about what an embedder can hold, call or read -- the surface, which is the part an embedder actually sees. Four more properties, checkable the same way, in `docs/idiomatization.md` §1.7: the library does not end the process (4 sites today); it is re-entrant, so two `Shell`s coexist and `testutil::lock()` can be deleted; it is publishable, meaning no out-of-repo path dependency and no blanket `allow`; and the API is a surface rather than the source."
        "**The surface property is the one that separates \"has a lib target\" from \"is a library\", and today it is unmet by two orders of magnitude.** `lib.rs` declares 35 `pub mod` and the crate exposes roughly a thousand public items -- a public transliteration, not an API. `main_fn(argc, argv, streams) -> !` is the whole of what an embedder could call, and the `-> !` is why none can."
        "**`exec cmd` replaces the host's process image, and that is a sharper 'a library may not do this' than terminating.** `eval.rs:1341` reaches `exec.rs:118`, which `execve`s in the *current* process: `dash -c 'exec echo REPLACED; echo NOT-REACHED'` prints the first and never runs the second. In an embedded shell that means `sh.run(b\"exec ls\")` ends the host -- no unwind, no `Drop`, no return, and nothing the caller can catch. Terminating at least runs atexit handlers; this does not. The API answer is a `Host` method the frontend grants and an ordinary embedder does not, which turns it into a value the library returns rather than an event the host cannot survive."
        "**The frontend becomes a separate crate, and that is a mechanism rather than tidiness.** While `main.rs` is a `[[bin]]` inside the library it can reach `crate::` internals and nothing would ever report it. With the split, anything the frontend needs that is not public is a compile error. There is no other way to make this decision checkable."
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
