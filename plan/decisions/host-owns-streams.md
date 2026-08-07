---
id [dec:nsh:host-owns-streams]
epitome "The library reads and writes streams it is given, not descriptors 0, 1 and 2."
state @decided
category @property
scope {
    elements ([arch:nsh:shell-core] [arch:nsh:shell-bin])
}
author "brendan@necessary.nu"
consequences {
    accepted (
        "An embedder can drive a shell from a buffer and collect its output without a pipe or a subprocess -- which is most of the point of a shell library."
        "Redirection and `exec` still manipulate real descriptors: those are the shell language's semantics, not the library's I/O policy, and they stay."
        "There are two ways to be given streams, because embedders have opposite constraints. `install` lends the shell descriptors 0, 1 and 2 and restores the host's afterwards: full fidelity, since within the shell's execution environment the standard descriptors really are standard. `set` moves only the shell's own I/O, for a host that cannot have `dup2` called on its descriptor 1 at all."
    )
    deferred ("Under `set`, the shell's own writes follow but the language's descriptor numbers do not: `echo hi` reaches the supplied stream, `echo hi >file` and every external command still mean the process's descriptor 1. Making those agree needs a per-instance descriptor table, which cannot be built while the shell keeps its state in statics -- it lands with [dec:nsh:no-ambient-state]. `crates/dash/tests/streams_embed.rs` pins the limit as a test rather than leaving it as a claim.")
}
edges {
    requires ([dec:nsh:shell-as-library])
}
---

## Rationale

`output.rs` holds `output` and `errout` as statics on descriptors 1 and
2; `input.rs` reads descriptor 0. A library that hardcodes those is a
library that can only be used by a program willing to give up its own
standard streams.

The distinction worth keeping is between the shell's *I/O policy* and
the shell *language's* semantics. `>`, `<`, `exec 3>&1` and pipelines
manipulate real descriptors and must go on doing so -- that is what the
language means. What changes is where the shell's own three streams
come from: given to it, rather than assumed.

The line editor already works this way and shows the shape. `el_init_fd`
takes the three descriptors as parameters, precisely because the
alternative -- deriving them from `FILE *` inside the library -- was the
bug that made the editor read fd -1 and exit before a key was pressed.

## What this cost in the port

`crates/dash/src/streams.rs` is the whole of the mechanism, and the rest
was finding the places where the C's `0`, `1` and `2` were load bearing
in a way a rename would have missed. Three were not simple substitutions:

  * `input.c`'s `forkreset` closes `parsefile->fd` when it is `> 0`,
    meaning "an open file that is not stdin". With a supplied stdin the
    second half stops being implied by the first, and the literal
    translation would close the shell's own input.

  * `preadfd`'s `fd == 0` is not about descriptor 0 either: it is the
    test for "this parse file is the shell's standard input", and it
    gates both line editing and the stdin tee.

  * `forkparent` closes descriptor 0 for a background job and reopens
    `/dev/null`, relying on `open` returning the lowest free descriptor
    to land back on 0. That is only true when the shell's stdin is 0.

Each is invisible under the default streams, which is exactly why they
are worth naming: the differential harness cannot catch them, because it
only ever runs the identity case.
