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

## The inbox, and why the handler needs one at all

The decision has two halves and they land in that order: **delivery**,
which is the inbox, and **disposition**, which is the `Host` trait.

The handler does not only write. `onsig` asks the shell two questions it
cannot ask through a receiver, because a handler has none: *is a trap set
for this signal?* — at `SIGCHLD` and `SIGINT`, and both are presence
tests rather than reads of the action — and *am I the vforked child?*.
Until those are answerable from a shared inbox, `trap.rs`'s table and
`jobs.rs`'s `vforked` cannot leave the process's statics, which is the
obligation `move-state` transferred here.

So the inbox carries an `[AtomicBool; NSIG]` mirror of
`trap[n].is_some()` and an `AtomicI32` for `vforked`, beside the arrival
flags `docs/api-design.md` §5.3 already names.

**The mirror's two stores are bracketed by a blocked signal mask, not by
`INTOFF`.** `INTOFF` defers *taking* an interrupt; it does not stop the
handler running, and since `errors-are-values` step F `INTON` is not a
delivery point at all. Nor is there a safe one-sided order to write the
slot and the bit in, because the two signals want opposite ones: a mirror
that reads "trapped" while the table says none swallows a `^C` and makes
`wait` answer `128 + SIGCHLD`, and a mirror that reads the other way
takes the interrupt instead of running the user's trap. Blocking closes
the window rather than choosing which end of it to lose, and restores the
property the C has for free — its `trap[signo]` is one pointer, so its
handler never reads an inconsistent pair.

**The inbox is process-wide, and the `Arc` does not change that.** A
disposition is installed per process and the handler is called with
`signo` and nothing else, so it cannot know which `Shell` a signal was
meant for. `Arc` buys shareability, not per-shell-ness. That is recorded
as a property of the crate in `docs/api-design.md` §6, beside the locale
and `getopt`, rather than designed away.

`vforked` in particular is *not* shell state and must not become a
`Shell` field: the parent sets it before `vfork` and the child reads it
out of the address space it shares. It describes an address space.

## What the frontend has not claimed yet

Nothing. `nsh-cli` today only *undoes* Rust runtime state — it restores
the inherited SIGPIPE disposition, resets SIGSEGV and SIGBUS, and
re-closes the fds `std` sanitised. Every disposition the shell installs
is still installed by the library, from `trap::setsignal` and
`trap::ignoresig`.

The seam is small and measured: seven `setsignal` call sites, two
`ignoresig`, the two `sigaction` calls inside `setsignal` itself, and the
five raw `libc::signal` calls in the here-document child. Moving them
needs `Shell` to own a `Box<dyn Host>`, which needs the builder — so the
disposition half belongs to `public-api`, the same way `io` and `streams`
do, and the deferred consequence recorded above stands until it lands.
