---
id [dec:nsh:we-own-the-defects]
epitome "The port phase is over. A defect in the Rust is fixed in the Rust, whether or not dash has it too."
state @decided
category @executive
scope {
    elements ([arch:nsh:shell-core] [arch:nsh:shell-bin] [arch:nsh:conformance])
}
author "brendan@necessary.nu"
alternatives (
    {
        option "Keep bug-for-bug fidelity: reproduce every dash defect, and fix only in the C on an upstreamable branch."
        rejected_because "That was the right rule while the port was being *established*, because the differential harness was the only evidence the translation was faithful. It stops being right once the code is being restructured for its own sake: it makes nsh's correctness hostage to whether a patch is accepted upstream, and it means shipping known-wrong behaviour we have already diagnosed."
    }
    {
        option "Fix freely and let the differential harness report the differences as failures."
        rejected_because "It destroys the oracle. `runall.sh` reports FAIL=0 today, and that single number is what makes a regression visible. Every deliberate fix that a corpus case observes would become a permanent FAIL indistinguishable from a real one, and within a handful of fixes nobody would read the number."
    }
)
consequences {
    accepted (
        "dash becomes a reference, not an authority. Where the two disagree the question is which is *right*, not which is the C."
        "Upstreaming stays worthwhile where a fix applies to the C as well -- the two branches already cut for `fc -e` and the background job announcement were worth cutting -- but it is no longer a precondition for fixing the Rust."
        "[dec:nsh:differential-is-the-oracle] changes meaning, not force. The harness stops asserting \"any difference is a port bug\" and asserts \"any *unsanctioned* difference is a port bug\". It is still the gate; what it gates is now a judgement rather than an identity."
    )
    deferred ("The harness has no sanctioned-divergence mechanism. `docs/divergences.md` is a register humans read; `dsdiff.sh` knows nothing about it. Until that gap is closed, a fix that a corpus case observes cannot be landed without turning FAIL=0 into FAIL=n permanently -- so the mechanism is a *prerequisite* for the first such fix, not a follow-up.")
}
edges {
    requires ([dec:nsh:shell-as-library])
    constrains ([dec:nsh:differential-is-the-oracle])
}
---

## Rationale

While the port was being established, bug-for-bug fidelity was the whole
method. There was no other evidence the translation was faithful: 61,498
differential cases at FAIL=0 is a strong claim precisely because it
admits no exceptions, and any tolerance for "well, that difference is
fine" would have hidden the defects the harness existed to find.

That phase ended. The code is now being restructured for its own sake
under [dec:nsh:shell-as-library] and [dec:nsh:owned-data], and the rule
that served the translation actively harms the product:

  * It makes nsh's correctness contingent on an upstream maintainer
    accepting a patch.
  * It means knowingly shipping behaviour already diagnosed as wrong.
  * It gives no answer at all for defects that *cannot* be reproduced.
    `owned-nodes` hit exactly this: `list()` never writes `linno` on the
    NBACKGND wrapper it synthesises, so the C reads whatever `stalloc`
    returned. An owned node has to name a value. There is no bug-for-bug
    option available -- reading uninitialised memory is not a behaviour a
    safe language can reproduce -- so the only question left is which
    correct value to write.

## What this costs, and why the cost is the interesting part

The second rejected alternative is the one that matters. "Just fix
things" is not free, because the harness's authority comes entirely from
being absolute. FAIL=0 is legible; FAIL=7-but-six-are-fine is not, and
the seventh will be missed.

So the register in `docs/divergences.md` has to stop being prose that
humans read and start being data the harness reads. Until it is, the rule
is narrow but real:

> A Rust-side fix may land only if no corpus case observes it. The first
> fix that a corpus case *does* observe must be preceded by teaching
> `dsdiff.sh` the sanctioned-divergence list.

The `linno` fix satisfies that condition -- nothing in `tests/corpus`
reaches it -- which is why it can go in now and why the mechanism did not
have to come first. The next one may not be so convenient.

## What does not change

The gates. Three suites, before and after, on every change: the
differential sweep, the POSIX suite with `--reference`, and the
interactive pty run. What changes is how a difference is *read*, not
whether it has to be explained. An unexplained difference is still a
defect, and the burden of showing which side is wrong sits with whoever
changed something.
