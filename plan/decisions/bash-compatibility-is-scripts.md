---
id [dec:nsh:bash-compatibility-is-scripts]
epitome "Bash mode is a contract about what a Bash script means, not about what the shell looks like to a person."
state @decided
category @executive
scope {
    elements ([arch:nsh:shell-core] [arch:nsh:line-editor] [arch:nsh:conformance])
}
author "brendan@necessary.nu"
alternatives (
    {
        option "Keep the interactive surface in the profile: prompt expansion, history controls, `bind` and programmable completion, as `[spec:nsh:req:compat.bash.interactive]` required."
        rejected_because "It is a contract to reimplement GNU Readline, which is a different project from Bash and one this shell does not use -- the editor is `nshedit`. The work produced exactly that: 665 lines reproducing `bind -l`, `bind -v` and `bind -p` observed from a host Bash, describing the interface of a library that is not here, advertising 173 functions the editor cannot run and 401 key bindings that do nothing. The shell was reporting capabilities it did not have, and had acquired an unresolved question about embedding a GPL-3 program's interface data in a BSD-3-Clause work. Both followed from the scope, not from the implementation."
    }
    {
        option "Keep the interactive code but stop developing it, and exclude those cases from the closure manifest."
        rejected_because "The false capability report is the defect, and it survives being excluded from a score. `bind -l` would still answer with Readline's inventory, the licence question would still be open, and roughly 2,500 lines of a surface nobody intends to maintain would still be in the core carrying the maintenance cost of the whole compatibility effort."
    }
    {
        option "Keep the parts a person interacts with, but reimplement them over `nshedit`'s own command set rather than Readline's."
        rejected_because "That is a defensible product, and it is not this profile. A Bash-compatibility contract that promises Bash-shaped interactivity implemented over a different editor promises something neither Bash nor nsh delivers. If nsh's own interactive surface should grow, it grows as an `nsh` feature with its own rules -- not as an obligation inherited from Bash."
    }
)
consequences {
    accepted (
        "`[spec:nsh:req:compat.bash.interactive]` is retired and `implement-bash-interactive-profile` is removed from the plan. The commit that implemented them is removed from history rather than reverted, because nothing downstream depended on it and a revert would have left the surface in the tree's past for someone to restore."
        "The closure gate for [dec:nsh:survey-evidence-has-tiers] is judged on script and syntax cases. The Oils survey's prompt, completion, bind, history, fc and interactive files -- 142 of 1,121 -- are outside the contract, so the number that means anything is the remaining 979."
        "Syntax stays in scope even when it names an interactive concept. `${name@P}` is a parameter transform, so it is recognised and yields its value; it does not decode prompt escapes, because there is no prompt rendering to decode them with."
        "Baseline up/down history navigation stays, as an `nsh` property with its own documentation. It predates this profile and does not depend on it."
    )
    deferred ("Whether nsh should have a designed interactive surface of its own is left open. This decision says only that Bash is not the specification for one. If that work happens it needs its own rules, its own oracle, and no obligation to report another project's command inventory.")
}
edges {
    requires ([dec:nsh:shell-as-library])
    constrains ([dec:nsh:survey-evidence-has-tiers] [dec:nsh:safety-trumps-compatibility])
}
---

## Rationale

"Compatible with Bash" was read, while the profile was being written, as
"behaves like Bash". For a shell that is also a library
([dec:nsh:shell-as-library]) the useful reading is narrower and sharper:

> A Bash script that runs under Bash runs the same way under nsh.

Everything a script can observe is in scope — arrays, `[[ ]]`, `(( ))`,
brace and extended-glob expansion, parameter transforms, functions and
scoping, namerefs, traps, process substitution, the builtins a script
calls, the variables a script reads. None of that is affected by this
decision; all of it stays.

What a *person* observes at a terminal is not in scope. Prompt rendering,
history recall, key bindings and completion are the shell's user
interface. They are also, specifically, GNU Readline's user interface —
and Readline is a separate GPL-3 project that Bash links, not part of
Bash. Writing them into a Bash-compatibility profile quietly committed
this project to reimplementing a library it does not use.

## How the scope error showed itself

Not as an argument. As artefacts:

  * `bind -l` reported 173 Readline function names. The editor is
    `nshedit`, whose command set is not Readline's, so `bind` recorded
    bindings and never installed them. The shell answered a capability
    question with another program's answer.
  * Those tables were obtained by running `bind -l`, `bind -v` and
    `bind -p` against a host Bash and transcribing the output — 665
    lines of a GPL-3 program's interface data inside a BSD-3-Clause
    repository. Whether that is clean is a question nobody involved was
    qualified to answer, and it only arose because the data was there.
  * 142 of the 1,121 cases in the compatibility survey measured this
    surface. A closure gate reading 812/1,121 would have been 12%
    composed of evidence about something outside the contract.

Each is a consequence of the rule, not a mistake in carrying it out. The
implementation was careful — the prompt work in particular closed the
CVE-1999-0491 / CVE-2016-0634 injection class properly, by splicing host
data as opaque fragments instead of quoting it into a string that gets
re-expanded. That care is why the scope error is worth recording rather
than quietly dropping: good work in the wrong place still has to come
out, and the reason has to survive so it does not go back in.

## What this does not say

It does not say interactivity is unimportant, or that nsh should be
unpleasant to use. It says Bash is not the specification for nsh's
interactive surface. The deferred consequence is genuine: if that surface
is designed later it starts from what nsh's editor can actually do.
