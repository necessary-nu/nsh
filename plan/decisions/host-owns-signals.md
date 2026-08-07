---
id [dec:nsh:host-owns-signals]
epitome "The frontend claims signal dispositions; the library never installs a handler on its own authority."
state @decided
category @ban
scope {
    elements ([arch:nsh:shell-core] [arch:nsh:shell-bin])
}
author "brendan@necessary.nu"
consequences {
    accepted ("Job control and interrupt handling need an explicit hand-off, because they genuinely require a handler -- the library asks, the frontend installs.")
    deferred ("Until then, constructing a shell silently displaces the host's SIGINT, SIGQUIT and SIGTERM handlers and does not put them back.")
}
edges {
    requires ([dec:nsh:shell-as-library])
}
---

## Rationale

`setsignal` installs `onsig` for SIGINT, SIGQUIT and SIGTERM whenever
the shell becomes interactive, and job control takes SIGTSTP, SIGTTIN
and SIGTTOU. For a program that is the whole point. For a library it
means constructing a shell steals the host's interrupt handling.

The port has already been bitten by the inverse of this and it is the
sharpest available illustration: Rust's runtime sets SIGPIPE to SIG_IGN
before `main`, dash never touches SIGPIPE, and the disposition is
inherited across fork *and* exec -- so a decision made by one component
reached every child of the shell and printed ~99,930 spurious `I/O
error` lines. A library that takes signals does that to its host on
purpose.

Signal dispositions are process-global whether or not the code
acknowledges it. Only the frontend knows what the process is for.
