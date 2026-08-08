---
id [dec:nsh:minimal-unsafe]
epitome "`unsafe` marks what is genuinely unsafe, not the whole shell."
state @decided
category @property
scope {
    elements ([arch:nsh:shell-core])
}
author "brendan@necessary.nu"
alternatives (
    {
        option "Leave the port as it is: unsafe on nearly every function."
        rejected_because "It is correct for a transliteration and useless as a signal. When everything is unsafe, the keyword tells a reader nothing about where the real hazards are, and it tells a compiler nothing at all."
    }
)
consequences {
    accepted (
        "The raw pointers go with the ambient state: most of the unsafety is reaching into statics and walking C strings, and both stop existing rather than get wrapped."
        "What remains is a real, small surface -- the syscall wrappers, the signal handler, the fd manipulation redirection performs -- and being small is what makes it reviewable."
        "**The baseline is 598 of 794 functions carrying `unsafe fn`, 75%**, counted at `4463fc6` by matching function definitions across `crates/nsh/src`. Taken now rather than deferred, because a property with no number attached is not checkable, and \"most of the unsafety goes with the state\" is a claim that needs a before and an after to mean anything. The target is under 5% -- a few dozen functions, each of which a reader can be pointed at."
    )
    deferred ("Where the floor actually sits. The target below is an estimate, not a measurement: the syscall wrappers, the signal handler, the descriptor manipulation and the pointer handoffs to libc have never been counted separately from the incidental unsafety around them. Resolved by taking the count again once `delete-memalloc` lands, when what remains is the design rather than the transliteration.")
}
edges {
    requires ([dec:nsh:shell-as-library] [dec:nsh:no-ambient-state])
}
---

## Rationale

Every function in the port is `unsafe`, and that is the honest
translation: they dereference raw pointers into process-global statics
and walk NUL-terminated C strings. The keyword is accurate and carries
no information -- a reader cannot use it to find the parts that need
care, because it is on everything.

Most of it is not intrinsic. It is the cost of two other decisions:
state in statics, and C strings as `*mut c_char`. Making the state an
instance and the strings owned removes the reason for the `unsafe`
rather than hiding it behind a wrapper, which is why this is a
consequence of `no-ambient-state` and not a separate cleanup.

What is genuinely unsafe stays and should be conspicuous: the syscall
wrappers, the signal handler, the descriptor manipulation redirection
performs, and the places the shell hands a pointer to libc. A small
`unsafe` surface is reviewable; the present one is not.
