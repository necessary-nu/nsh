---
id [dec:nsh:differential-is-the-oracle]
epitome "With per-function tests skipped, the differential harness is what makes idiomatization safe -- so its coverage has to be known."
state @decided
category @executive
scope {
    elements ([arch:nsh:conformance] [arch:nsh:shell-core])
}
author "brendan@necessary.nu"
alternatives (
    {
        option "Complete wave 3 first: a `/test` facet for all 534 symbols."
        rejected_because "410 remain, and a unit test asserts what its author believed the C does. Eight of this project's assertions about the C were wrong on the day they were written, every one caught by comparing against the real thing. For guarding a refactor the differential harness is the better oracle, not merely the cheaper one."
    }
)
consequences {
    accepted (
        "Coverage measurement stops being a nice-to-have. A refactor of code the corpus never executes is unguarded, and looks identical to one that is guarded."
        "Measurement moves to the Rust side. gcov loses the parent's counters on fork+exec, which is most cases; LLVM instrumentation writes one profraw per process and merges them."
        "**The absence of per-function attribution sets the step size.** When a differential case fails after a refactor, the harness says the shell changed, not which function changed it -- so the unit of work has to be small enough that \"which change broke this\" is answerable by inspection. One property per step. Paying to rewrite the same signatures twice is the price of a bisectable failure, and it is worth paying."
        "**The harness tests exactly one configuration**, and therefore has zero coverage of every axis idiomatization adds. It runs `Streams::INHERIT`, one shell per process, signals claimed, exit by `_exit`. Each library property opens a configuration axis and the harness samples precisely the point where nsh must equal dash. [dec:nsh:host-owns-streams] already recorded this locally -- three of its subtle fixes are invisible to the harness \"because it only ever runs the identity case\" -- and it generalises: coverage of a new surface is zero from the day the axis exists, not from some later step. Every step must therefore bring its own test for the thing it just made possible."
    )
    deferred ("What guards the 92 functions the corpus never enters -- `system.rs` at 15.38%, `linedit.rs` at 26.09%, `show.rs` at 0%. There the rejected per-function suite is not the worse oracle, it is the only one available.")
}
edges {
    requires ([dec:nsh:shell-as-library])
}
---

## Rationale

Wave 4's own guidance names the wave-3 suite as its oracle, so skipping
wave 3 removes the stated safety net. The replacement is stronger for
this particular job: 61,498 differential cases compare the port against
the actual C dash, and they care about observable behaviour rather than
about which function moved -- which is exactly the property a refactor
needs.

What that does not give is reach. `tests/README.md` records that
`covrun.sh`'s numbers are untrustworthy, because gcov loses the parent's
counters whenever a case forks and execs: `main` reads 54.17% with no
fork and 39.58% with `/bin/true`. So today nobody can say which code the
corpus executes.

That gap is tolerable while the code is a transliteration and intolerable
while it is being restructured. Refactoring `expand.rs` is guarded by
thousands of cases; refactoring an error path, an OOM branch or an
`#ifndef HAVE_…` fallback is guarded by none -- and the two feel
identical from the inside.

The fix is cheaper than the abandoned one because it asks a different
question. Wave 4 does not need to know which C lines ran; it needs to
know which *Rust* functions the corpus reaches, and LLVM's
instrumentation handles fork and exec by writing a profile per process.

## The oracle's contract, and where it runs out

Two things narrow this oracle, and neither is a reason to distrust it for
the job it does.

**It only ever runs one configuration.** dash's. One shell per process,
descriptors 0/1/2, signals claimed, terminating by `_exit`. That is the
right thing to compare, because the claim under test is "nsh is dash".
But every property under [dec:nsh:shell-as-library] adds an axis the
comparison cannot reach: a second `Shell` in one process, a stream that
is not descriptor 1, an error that returns instead of unwinding, a
signal the host owns. There is no version of the differential harness
that covers those, because dash cannot do them and there is nothing to
compare against. So each step owes its own test, and
`crates/nsh/tests/streams_embed.rs` is the pattern -- four cases that
exist precisely because the 61,498 cannot see what they check.

**Its verdict is whole-shell.** A failure names the case, not the
function. That is why step size is a constraint of this decision rather
than a matter of taste, and it is why the tempting economy -- fold two
properties into one commit to avoid rewriting signatures twice -- is a
false one. The signatures are cheap; a red harness with two candidate
causes is not.

What the oracle does keep, all the way to the end, is authority over the
shell *language*: `parser.rs` at 100% of functions, `eval.rs` at 100%,
`expand.rs` at 89.83%. Its contract becomes "the frontend, in dash's
configuration, is dash" -- which is a statement about the frontend, and
one more reason the frontend is worth being a separate crate.
