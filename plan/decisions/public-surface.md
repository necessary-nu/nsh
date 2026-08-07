---
id [dec:nsh:public-surface]
epitome "`Shell` is the API; everything else is `pub(crate)`, and the frontend is a separate crate so the compiler says so."
state @decided
category @existence
scope {
    elements ([arch:nsh:shell-core] [arch:nsh:shell-bin])
}
author "brendan@necessary.nu"
alternatives (
    {
        option "Keep one crate with a `[[bin]]` target, and mark internals `pub(crate)` as they are found."
        rejected_because "A binary inside the library crate can reach `crate::` internals, so nothing ever reports that the frontend used something an embedder cannot. The boundary would be enforced by inspection, which means it would be enforced until someone is in a hurry. The split makes it a compile error, and there is no other mechanism that does."
    }
    {
        option "Expose the modules as they are and let embedders take what they need."
        rejected_because "That is the present state, and it is why [dec:nsh:shell-as-library] is unfalsifiable. `crate::eval::exitstatus` is public. So is `memalloc::stalloc`. With ~1,000 public items every internal detail is already API, so no restructuring can be described as compatible or incompatible with anything."
    }
)
consequences {
    accepted (
        "The workspace becomes `crates/nsh` (lib) and `crates/nsh-cli` (bin), and the crate is renamed from `dash` to `nsh` in the same move."
        "The surface is roughly twenty documented items under `#![deny(missing_docs)]`: `Shell` and its builder, `run` / `run_command`, the variable accessors, `status`, `expand_word`, `Streams`, and the value types `Error` / `ExitStatus` / `Source` / `Signal`. Everything else is `pub(crate)`."
        "`expand_word` is on the list deliberately. Word expansion without execution is the thing embedders actually want and no `Command`-style API can offer, and it is the clearest single argument for a shell library over spawning one."
        "A `Host` trait carries what a library may not do on its own authority -- install a signal disposition, terminate the process -- and the frontend implements it. That is where [dec:nsh:host-owns-signals] lands structurally."
        "The design has to be settled EARLY and implemented LATE. It determines what [dec:nsh:no-ambient-state] builds -- which fields live on `Shell`, what its borrow shape is -- so designing it afterwards means moving the state twice."
    )
    deferred ("What `run` does about the parse-file stack when called twice. dash's input stack is global. Two `run` calls on one `Shell` should compose like two lines of one script; a `run` from inside a `Host` callback should not. This is a semantics question, not a naming one, and it is unanswered -- see `docs/idiomatization.md` §7.6.")
}
edges {
    requires ([dec:nsh:shell-as-library])
    enables ([dec:nsh:no-ambient-state])
}
---

## Rationale

`lib.rs` declares 35 `pub mod`, and the crate exposes something on the
order of a thousand public items. That is not an API with too much in it;
it is the absence of one. There is no line anywhere between what an
embedder may touch and what is internal, which means
[dec:nsh:shell-as-library] cannot be checked -- every internal detail is
already part of the surface, so no change to any of them is a change to
the API or not.

What an embedder should be writing:

```rust
let mut sh = Shell::builder()
    .arg0("myapp")
    .streams(Streams::capture())
    .build()?;

let status = sh.run(b"for f in *.txt; do wc -l \"$f\"; done")?;
let out: &BStr = sh.captured_stdout();
```

Three things separate that from spawning `/bin/sh`: no second process,
no quoting round-trip in or out ([dec:nsh:bytes-not-text]), and errors
that arrive as values rather than as a status and some text on a pipe
([dec:nsh:errors-are-values]). A fourth is easy to promise and hard to
deliver -- two `Shell` values sharing nothing -- and it is the whole
content of [dec:nsh:no-ambient-state].

## Why the crate split is the load-bearing part

The surface could in principle be closed by marking things `pub(crate)`
one at a time. It would not stay closed. While the frontend is a
`[[bin]]` inside the library crate it can reach `crate::` internals
freely, so the only thing standing between the boundary and its erosion
is whoever is reading the diff.

Splitting `crates/nsh-cli` out changes the kind of guarantee. The
frontend then links the library as an external crate and can use nothing
but its public API, so **anything it needs that is not public stops
compiling**. That converts [dec:nsh:shell-as-library] from an intention
into a build failure, and it costs a workspace member and one path
dependency.

The rename travels with it because the rename is the only piece of this
work whose cost strictly grows with delay: every commit message, module
path and decision written before it has to be written again after.

## Why the design comes early and the implementation late

These are separable and the ordering matters in opposite directions.

The *design* has to precede [dec:nsh:no-ambient-state], because moving
the statics onto an instance is exactly the act of deciding what `Shell`
is. Doing it without knowing the intended surface means choosing the
fields, the borrow shape and the mutability by accident, and then moving
them again once the API is written.

The *implementation* has to follow [dec:nsh:host-owns-signals], because
the `Host` trait is the last thing to take shape and closing the surface
before it exists would only mean reopening it.
