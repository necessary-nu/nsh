---
id [dec:nsh:safety-trumps-compatibility]
epitome "Where matching Bash would import an unsafety, we do not match Bash. The divergence is recorded, not silent."
state @decided
category @executive
scope {
    elements ([arch:nsh:shell-core] [arch:nsh:shell-bin] [arch:nsh:conformance])
}
author "brendan@necessary.nu"
alternatives (
    {
        option "Bit-exact Bash compatibility, defects included: whatever Bash 5.3 does, nsh does, and the survey number is the only thing that decides."
        rejected_because "It hands the threat model to a different project. Bash's exported-function import is the Shellshock class; its regex engine backtracks without bound; its restricted mode has been bypassed repeatedly. A compatibility rule with no exception clause obliges us to reproduce all three, and two of them are currently closed here only by absence. It also fails on its own terms: parts of the corpus annotate Bash's behaviour as a bug (`## BUG bash`), so bit-exactness is not even well-defined -- it asks us to match a thing the oracle itself marks wrong."
    }
    {
        option "Deviate for safety whenever it is cheap, and match Bash whenever deviating is expensive."
        rejected_because "Cost is not a property anyone can review. It makes every case an argument, and the argument is settled by whoever is tired -- which is exactly how a `set -u` failure stops exiting because swallowing one expansion error inside `[[ ]]` was locally convenient. The rule has to be legible before the work starts, or it is not a rule."
    }
    {
        option "Ship the safe behaviour by default and offer a `--bash-bugs` mode that reproduces the unsafe one for scripts that depend on it."
        rejected_because "Every such mode is a second implementation with the first one's threat model, and it is reachable from the environment the moment anything sets it. If a specific defect turns out to have real dependents, that is an argument for one narrowly-scoped, separately-reasoned option, not for a standing bug-compatibility switch."
    }
)
consequences {
    accepted (
        "Compatibility surveys stay diagnostic, per [dec:nsh:survey-evidence-has-tiers]. A survey case that encodes a Bash defect is expected to stay red; the number is read, not maximised."
        "A deliberate deviation must carry its reason at the point of divergence, not in a commit message. Whoever next sees the red case has to be able to tell refusal from omission without archaeology."
        "This extends [dec:nsh:we-own-the-defects] from dash to Bash. Both are references, neither is an authority, and the question at a disagreement is which behaviour is right."
        "Bounded matching is a semantic commitment, not an optimisation. The regex engine answers `no match` at its step budget rather than running unbounded, and that answer is the specified behaviour -- so the budget cannot be raised or removed as a performance tweak without revisiting this decision."
    )
    deferred ("The survey can express Bash's defects but not ours. Oils spec files carry `## BUG bash` and the runner reports `known_bug` separately, so a case where *Bash* is wrong is already legible. There is no counterpart for a case where nsh deliberately refuses Bash's behaviour: it reports as a plain FAIL, indistinguishable from a feature nobody has written yet. Until that exists, every deviation taken under this decision has to be recorded on its plan node by hand, and the register is only as good as that discipline. This is the same shape of gap [dec:nsh:we-own-the-defects] deferred for the differential harness.")
}
edges {
    requires ([dec:nsh:we-own-the-defects] [dec:nsh:minimal-unsafe])
    constrains ([dec:nsh:survey-evidence-has-tiers])
}
---

## Rationale

The Bash compatibility work has a built-in pressure: progress is measured
by a survey number, and the fastest way to move that number is always to
do whatever Bash does. That is the right instinct almost everywhere. It
is wrong in a specific and recurring case, and the case is common enough
that leaving it to judgement produced four separate instances inside a
single day's work.

This decision names the exception so it does not have to be re-argued:

> Where reproducing Bash's observable behaviour would require importing
> an unsafety -- unbounded resource consumption, an ambient
> data-to-syntax path, a weakened error or privilege boundary, or memory
> unsafety -- nsh implements the safe behaviour and records the
> divergence.

Everything else still follows Bash. This is a narrow carve-out, not a
licence to improve on Bash wherever it seems improvable. Cosmetic
disagreements are not covered: `declare -p` printing associative keys in
sorted order because the storage is a `BTreeMap` is a divergence to fix
or accept on ordinary compatibility grounds, and this decision has
nothing to say about it.

## The precedents this ratifies

The rule is being written down after the fact, which is the honest order
-- each of these was decided on its own and they turned out to agree.

  * **`[[ a =~ $(( 1/0 )) ]]`.** Bash answers 1 and continues. Matching
    it means swallowing expansion errors inside `[[ ]]`, and the same
    path carries `set -u` failures. One survey case, traded against
    silently weakening `nounset` for every script.
  * **The regex step budget.** A hand-written ERE engine that backtracks
    without bound is a denial-of-service surface reachable from any
    string a script matches. The budget makes the answer `no match`
    instead of a hang. Verified against the classic backtracking bombs,
    and against 5,000-character subjects to confirm the budget does not
    manufacture false negatives.
  * **`trap ERR with redirect`.** Bash does not advance `$LINENO` for a
    redirect. Matching that means suppressing the command-line record on
    redirect nodes, which degrades every diagnostic that reports a line.
  * **The glob memo key.** `Matcher::matches_from` memoizes on
    `(pattern node, offset)`, which is sound only while nothing varies
    mid-match. Extended globs introduce negation and flags that do vary;
    a shared key across differing contexts returns another context's
    answer. Correctness of the memo is not negotiable against the speed
    it buys.

## What this does not license

It is not a general warrant to deviate. Three tests, all of which must
hold:

1. The Bash behaviour would import a *named* unsafety -- resource,
   syntax boundary, error boundary, privilege boundary, or memory.
2. The safe behaviour is specified, not merely different. "We do
   something else" is not a decision; "we answer `no match` at the
   budget" is.
3. The divergence is recorded where the next person will look: on the
   plan node that owns the surface, with the reasoning, not just the
   symptom.

A deviation failing any of the three is a bug, not an exercise of this
decision.

## The gap this leaves open

The deferred consequence is the load-bearing one. The survey already
distinguishes *Bash's* defects: the corpus annotates them and the runner
counts `known_bug` apart from `fail`. Nothing expresses the converse. A
case we refuse on purpose looks exactly like a case we have not built,
so the evidence that a divergence was reasoned lives entirely in the
plan and nowhere in the gate.

That is tolerable while the divergences are few and recent. It stops
being tolerable at the point where someone reads a red case, cannot tell
which kind it is, and "fixes" it back to Bash's behaviour -- which is
precisely the failure this decision exists to prevent. Teaching the
survey a sanctioned-divergence register is therefore a prerequisite for
the compatibility profile being closable, not a nicety after it.
