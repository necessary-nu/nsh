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
    )
    deferred ("Until then, embedding means the shell reads the host's stdin and writes the host's stdout.")
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
