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
        "**Where the floor sits, measured (`process-model`, at `410e729`) -- this resolves the deferred consequence this decision carried.** `grep -rhoE 'libc::[a-z_0-9]+\\(' crates/nsh/src --include=*.rs` gives **255 call sites across 68 distinct symbols**, plus **13 symbols hand-declared in 7 `extern \"C\"` blocks** that `libc` 0.2 does not bind. One vendor: no `nix`, no direct `rustix`. `docs/api-design.md` 11.3 has the thirteen-group breakdown, and the estimate this decision made turns out to be wrong in one direction that matters. **The floor is not the syscalls.** \"fork/exec/wait/signals/termios/fd\" -- the shape the operator's directive names, and the shape the sentence above guessed -- is 170 of the 255. The other 85 are `stat`, identity, resource limits, `errno`, `getopt`, and the C library's string, ctype, collation and multibyte routines. Not syscalls, still FFI, and every one of them has to move before `nsh` can deny `unsafe_code`. The last group is the hard one and it is hard for a reason wrapping does not address: `strcoll`, `isalpha` and `mbrtowc` are locale-dependent, so replacing them is a behaviour decision rather than a safety one."
        "**The wrapper layer already covers about half the floor, and the uncovered half is not random.** Wrapped: `redir::{sh_open, sh_pipe, sh_dup2, savefd}`, `jobs::{forkshell, vforkexec, xtcsetpgrp, xxtcsetpgrp, waitproc}`, `trap::{setsignal, ignoresig, sigblockall}`, `system::{sigclearmask, errno}`, `output::{write_fd_once, write_fd, xwrite}`, `siginbox::SignalsBlocked`. Raw at 90-odd sites: `setpgid`, `getpgrp`, `tcgetpgrp`, `isatty`, `close`, `stat64`, `kill`, `killpg`, `raise`, `_exit`. That unwrapped list is almost exactly [dec:nsh:host-owns-the-process]'s list of grant-bearing operations, and the coincidence is not one: the calls nobody had to think about are the calls nobody wrapped."
        "**One constraint on the floor crate's API, from [dec:nsh:fork-child-is-a-terminus].** The wrappers a forked child calls between `fork`/`vfork` and `execve` -- `execve`, `_exit`, `dup2`, `close`, `open`, `setpgid`, `tcsetpgrp`, `signal`, `sigaction` -- must not allocate, so their error type is a bare `errno` and never a `Box<dyn Error>`, a `String`, or anything that renders a message on construction. Cheap to design in, expensive to retrofit."
    )
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
