---
id [dec:nsh:printf-is-parsed-not-interpreted]
epitome "`printf` is a builtin. It parses its conversions into typed descriptors and renders them with Rust's own formatting; the shell still speaks no format string of its own."
state @decided
category @existence
scope {
    elements ([arch:nsh:shell-core] [arch:nsh:conformance])
}
author "brendan@necessary.nu"
alternatives (
    {
        option "Leave `printf` external, as [dec:nsh:no-format-interpreters] had it."
        rejected_because "The two costs it accepted are real and neither is recoverable from outside the shell. With `PATH` empty or short, dash still has `printf` and nsh did not, so a script that clears `PATH` worked on one shell and failed on the other; and a loop calling `printf` forked once per iteration. The third cost was to the harness: six corpora and 1236 further cases had to be retired or edited because they compared nsh-plus-coreutils against dash's builtin, which measures coreutils against dash rather than this shell against that one. The ban was aimed at a shape the crate had, not at this utility, and the shape is gone."
    }
    {
        option "Bring the builtin back but render `%a`/`%A` as Rust's shortest round-tripping decimal, and register the difference."
        rejected_because "That is the 576-case gap the earlier attempt left, and it is the one conversion that needs no conversion at all: a hexadecimal float is the double's IEEE-754 fields written out, so `f64::to_bits` and a tie-to-even rounding of the mantissa render it exactly. Excusing 576 cases to avoid thirty lines of bit manipulation would have bought a permanent divergence with nothing."
    }
    {
        option "Render each conversion with libc, as the C does -- `snprintf` per conversion behind the `PF`/`ASPF` arity switch."
        rejected_because "Still banned, and for the reasons that have not changed. It needs a C format string built at runtime, so it needs varargs, so it needs the arity switch -- and the port's version of that shipped two defects straight out of the shape: no INTOFF/INTON around the call, and `%d`/`%i` handing a `uintmax_t` to a `%ld` conversion, which is varargs undefined behaviour that happened to work on LP64."
    }
)
consequences {
    accepted (
        "`printf` is a builtin again, `type printf` says so, and `crates/nsh/src/builtins/mod.rs` is `mkbuiltins`' output for the default Linux build byte for byte once more."
        "The two behavioural costs are gone: with `PATH` empty the utility is still there, and a loop that calls `printf` no longer forks per iteration."
        "The six retired corpora and the cases edited out of the others come back, and measure the shell against the shell. The divergence register carries no printf entry."
        "Parsing a `%` conversion at runtime is sanctioned *for this utility*, because it is the utility's contract -- POSIX defines `printf` as a program that reads a pattern at runtime and renders arguments by it. The sanction is scoped to `builtins::printf` and travels no further."
    )
    deferred ("What stays banned is what the shape cost: no libc formatting anywhere in the crate, no transplanted C conversion engine, and no runtime format-string API in the output layer -- `Output` is an `io::Write` and gains no `doformat`, `fmtstr`, `out1fmt`, `outfmt` or `xasprintf`. Nothing outside `builtins::printf` may format a value by a pattern chosen at runtime. The nearest live temptations are a future `read -t`-style option and a prompt escape expander; both should reach for `write!` at the site that knows the types.")
}
edges {
    supersedes ([dec:nsh:no-format-interpreters])
    requires ([dec:nsh:we-own-the-defects])
    constrains ([dec:nsh:differential-is-the-oracle])
}
---

## Rationale

The objection that removed `printf` was to a shape, not to a utility. When
the output layer was still C, the builtin could only work by building a
format string at runtime and handing it to libc — and everything that made
that ugly (varargs, the three-arity `PF`/`ASPF` switch, `mklong` rewriting
`%92.3u` into `%92.3`PRIuMAX so the call would pull a whole `intmax_t`)
lived in the crate because the crate had a runtime format-string API for it
to live in. Banning the API and banning the builtin looked like one act.

They were two. `Output` became an `io::Write`, `doformat` and its family
went, and every builtin moved onto typed arguments in a module of its own.
What is left for `printf` to do in that world is not to interpret a format
string on the shell's behalf: it is to parse the user's `%` conversion into
a `Spec` — flags, width, precision as fields — and render a *typed*
argument against it. The digits come from `format!`. There is no varargs,
no arity switch, no length modifier to widen, and no libc.

That distinction is the decision. A format-string interpreter is machinery
the shell reaches for when *it* wants to print something; `printf`'s
conversion parser is the utility reading its own operand. POSIX requires
`printf` to exist and defines it as exactly that, so a shell that refuses
to parse a `%` conversion is not avoiding an interpreter, it is declining
to implement a standard utility. The ban stands where it was aimed — at the
output layer, at libc formatting, and at every call site that could reach
for a runtime pattern instead of `write!` — and stops at this one module.

`%a` is what made the earlier attempt look unfinishable, and it turned out
to be the easiest conversion in the set. Rust has no hexadecimal float
formatting, but `%a` needs no formatting: four mantissa bits are one
hexadecimal digit, so `f64::to_bits` and a half-to-even rounding of the
tail render it exactly, carry and all. Bit manipulation is not a C
transplant, and the 576 cases close.

## What this does not permit

Formatting by a pattern the program did not write. `write!(output,
"{name}={value}")` was never the objection: the pattern is a literal, the
arguments are typed at compile time, and `std::fmt` resolves it before the
program runs. `builtins::printf` is the single place a pattern arrives as
data, and it is there because a POSIX utility's contract put it there.
