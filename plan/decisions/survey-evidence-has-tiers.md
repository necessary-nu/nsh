---
id [dec:nsh:survey-evidence-has-tiers]
epitome "Conformance evidence keeps its source authority: POSIX rules are normative, while compatibility surveys remain explicitly diagnostic."
state @decided
category @executive
scope {
    elements ([arch:nsh:conformance])
}
author "brendan@necessary.nu"
alternatives (
    {
        option "Treat every Dash-selected Oils case as a normative POSIX requirement."
        rejected_because "Oils selects useful comparisons through compare_shells metadata, not through a traceable POSIX rule citation; the set contains extensions and implementation probes alongside portable behavior."
    }
    {
        option "Leave external suites in disposable checkouts and run their original harnesses."
        rejected_because "Moving branches, Python 2 orchestration, user-home fixtures, and network-time discovery make the evidence neither reproducible nor reviewable with the shell change it judges."
    }
)
consequences {
    accepted (
        "The rule-indexed POSIX.1-2024 harness under posix/ remains the normative conformance oracle."
        "The Dash differential harness remains the regression oracle defined by [dec:nsh:differential-is-the-oracle]; its sanctioned divergences remain explicit."
        "Oils is pinned and reported as a differential compatibility survey. A Dash-selected Oils case is a POSIX candidate, never a normative rule by selection alone."
        "Smoosh is pinned, executed, and reported separately. Its formal POSIX evidence is not merged into Oils totals or used to weaken a normative failure."
        "Ordinary survey runs are offline and repository-native; source identity, fixture closure, selection counts, and machine-readable results are reviewable inputs."
    )
    deferred ("Promoting an external case into the normative POSIX harness requires a rule citation and a repository-owned expectation; survey membership alone is insufficient.")
}
edges {
    requires ([dec:nsh:differential-is-the-oracle])
}
---

## Rationale

External suites answer valuable but different questions. Oils asks how shells
behave across a wide language-and-extension survey, and its `compare_shells`
field is a comparison roster. Smoosh supplies tests derived from a formal
POSIX model. The repository's own POSIX harness starts from published
POSIX.1-2024 rule wording and attaches each executable expectation to that
rule. Calling all three "POSIX tests" would erase the provenance needed to
interpret a failure.

Pinning and native execution solve reproducibility, not authority. A pinned
diagnostic case is stable enough to gate regressions and expose a backlog, but
it does not acquire a standards citation. Reports therefore retain their
suite identity and explicit pass, fail, skip, unsupported, timeout, and error
accounting. A maintainer can promote a useful survey case into `posix/`, but
that act includes the missing rule citation and repository-owned judgment.

This separation also protects the existing oracles. A large compatibility
backlog cannot normalize a normative POSIX failure, and intentional POSIX
improvements do not have to pretend that every historical shell behavior was
correct. The evidence composes because its boundaries stay visible.
