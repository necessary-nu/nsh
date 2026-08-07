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
    )
    deferred ("Per-function attribution. When a differential case fails after a refactor it says the shell changed, not which function did.")
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
