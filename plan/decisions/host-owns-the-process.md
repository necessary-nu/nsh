---
id [dec:nsh:host-owns-the-process]
epitome "The library may create a process. Replacing, regrouping or ending the host's is the host's to grant."
state @decided
category @ban
scope {
    elements ([arch:nsh:shell-core] [arch:nsh:shell-bin])
}
author "brendan@necessary.nu"
alternatives (
    {
        option "Let the library keep doing what dash does: `execve` in place, take the terminal, `_exit` when the script says `exit`."
        rejected_because "It is not a library, and the sharpest case is not `exit`. `sh.run(b\"exec ls\")` replaces the embedding program's image: no unwind, no `Drop`, no return, and not even the atexit handlers that terminating would run. An embedder cannot defend against it, because nothing in the call's type says it might happen."
    }
    {
        option "Refuse the operations outright -- make `exec`, `set -m` and `exit` errors when the shell is a library."
        rejected_because "It forks the language. `nsh-cli` has to be dash exactly, and a shell whose `exec` is a diagnostic is a different shell -- the differential harness would be measuring two dialects. A grant keeps one implementation and moves the authority, which is the whole shape of [dec:nsh:host-owns-signals] and [dec:nsh:host-owns-streams]."
    }
    {
        option "Ask the host per occurrence, with a callback carrying the command line."
        rejected_because "The question is about authority, not about which command. A host that would say yes to `exec ls` has already given up its image, and one that would say no does not become safer by being asked with an argument. One boolean per capability, answered once at build time, is both the honest granularity and the one that keeps `Host` a leaf (`docs/api-design.md` 4.3)."
    }
    {
        option "Divide reaping per shell, so a `Shell` reaps only the children it forked."
        rejected_because "OVERTURNED 2026-09-02, and the refutation is kept verbatim because the way it was wrong is the useful part. It read: \"`wait3(-1)`/`waitpid(-1)` is how the shell learns about the children it does *not* track -- the here-document writer forks with `FORK_NOJOB` and no job entry, and would become a zombie under per-pid waits. `waitid(WNOWAIT)` peeks without reaping but busy-loops when the only exited child is somebody else's. It is the same shape as the locale: a process-wide fact, recorded rather than designed away.\" Neither half holds. The zombie does not follow, because the set of forked pids sits *beside* the job table rather than inside it: a fork with no job entry is still a fork, and it is recorded and reaped like every other. The spin does not arise, because nothing peeks -- a shell that names each of its own pids to `waitpid` never asks a question about a child it does not own, and so needs neither `WNOWAIT` nor a claim on the process's `SIGCHLD`. See \"Correction\" below."
    }
)
consequences {
    accepted (
        "**Creating a process needs no grant; the three things done *to* the host's own process do.** `fork` and `vfork` make a new process and touch nothing of the host's, so they stay the library's. What moves behind a grant is the trio that operates on the process the embedder is running in: replacing its image, moving its process group and taking its controlling terminal, and ending it."
        "**`exec cmd` is not the only site that replaces the image, and `docs/api-design.md` 5.4 names only that one.** There are three `shellexec` callers. `jobs.rs:1160` is inside a vforked child and is the point of the exercise. `builtins/exec.rs:48` is the `exec` builtin. The third is `eval.rs:1389` -- `evalcommand`'s `EV_EXIT` fast path, which `execve`s the *last command of the script* in place with no `exec` written anywhere, and it is reached from `main`'s `-c` at `shellmain.rs:199`. So a naive `Shell::run` built on the `-c` path would replace the embedder's image on `sh.run(b\"ls\")`. **`run` must not pass `EV_EXIT`; `nsh-cli` still may.** That is the same grant as `exec`, and it is `public-api`'s to wire."
        "**Job control is off unless the host grants it, because `setjobctl(1)` operates on the host three times.** `libc::setpgid(0, rootpid)` (`jobs.rs:482`) moves the embedding process into its own process group; `xtcsetpgrp` (`:483`) takes the controlling terminal from whoever had it; `libc::killpg(0, SIGTTIN)` (`:452`) stops the whole process group -- the host and every sibling -- until someone foregrounds it. None of the three is undone by anything but `setjobctl(0)`, which is reached from `exitshell`."
        "**Terminal handoff is `setjobctl`'s alone, and everything downstream of it is safe by construction.** `xxtcsetpgrp` short-circuits on `ttyfd < 0` (`jobs.rs:351-357`), and `ttyfd` is set only by `setjobctl`. So `forkchild`'s handoff to a new job (`:994`) and `waitforjob`'s hand-back to `rootpid` (`:1211`) and `fg`'s (`builtins/fg.rs:85`) cannot reach `tcsetpgrp` at all unless job control was turned on first. Gating `setjobctl` gates the terminal entirely; no second gate is needed."
        "**The library ends a process in four places and three of them are correct.** `shellmain::exit_from_child`, `jobs::forkchild_fatal` and `redir.rs:483`'s here-document writer all `_exit` a child the library itself forked, which is [dec:nsh:fork-child-is-a-terminus] and is not the host's business. The fourth is `trap::exitshell` (`trap.rs:562`), which ends the *host's* process, and turning it into a return is `public-api`'s -- `docs/api-design.md` 5.4 already says terminating is not on the `Host` trait because after the builder the library never needs it."
        "**Reaping is per shell, and the pids are what divide it.** CORRECTED 2026-09-02. This entry used to read \"reaping is process-wide and the API has to say so\", and it described `waitproc`'s `wait3(status, flags, NULL)` -- `waitpid(-1)` -- reaping *any* child of the process, an embedder's `std::process::Child` included, and the embedder's own `wait` then answering `ECHILD`. That was true of the code and false as a necessity. Each `Shell` now keeps the pids it forked and asks after those by name, so it is no longer an entry in `docs/api-design.md` 6 beside the locale, `strtok`, `getopt`, the environment, the working directory and the signal inbox. What remains process-wide about waiting is only the `SIGCHLD` disposition, which is [dec:nsh:host-owns-signals]'s and was always listed separately."
        "**The guarantees the library does give about waiting, stated positively.** Every child it forks is recorded before it is waited for -- in the job table when it has a job, and in the shell's own set of forked pids either way, which is what covers the deliberately jobless here-document writer and the jobless process-substitution child. A foreground job is waited to completion before `run` returns. A background job is **not**: `sh -c 'sleep 10 &'` returns with the child alive, exactly as dash does, and dropping the `Shell` neither kills nor reaps it. A `Drop` that waited would block the host and a `Drop` that killed would exceed the grant, so `Shell::drop` does neither -- and that is a promise, not an omission."
        "**Prompt reaping needs a SIGCHLD disposition, which the library does not own.** `trap::mkinit_init` installs one unconditionally at startup (`trap.rs:148-149`), and `waitproc`'s blocking arm is `sigsuspend` on it (`jobs.rs:1437`). So the wait design and [dec:nsh:host-owns-signals] are one design: a host that grants no `SIGCHLD` handler gets a shell that can still block in `wait3` but cannot notice a background job finishing between commands."
        "**What the syscall floor inherits from this.** The floor crate's job is mechanism, not authority: it exposes `execve`, `setpgid`, `tcsetpgrp`, `killpg` and `_exit` as safe wrappers and knows nothing about grants. The gate lives in `nsh`, at the three call sites named above, because only the shell knows whether the process it is about to operate on is the host's or a child it just made."
    )
    deferred ("There is no `Host` to ask yet, so none of the three grants is enforced today: `sh.run(b\"exec ls\")` still replaces the embedder's image, `set -m` in a shell with a terminal still takes it, and `exitshell` still `_exit`s the host. All three need `Builder` and `Box<dyn Host>`, which is `public-api`'s, and the obligations are recorded on its log with the call sites.")
}
edges {
    requires ([dec:nsh:shell-as-library] [dec:nsh:host-owns-signals])
    related_to ([dec:nsh:host-owns-streams] [dec:nsh:no-ambient-state])
}
establishes ([arch:nsh:shell-core] [arch:nsh:shell-bin])
---

## Rationale

`[dec:nsh:shell-as-library]` names four things a library may not do to the
process -- exit, claim a signal, assume a descriptor, hold ambient state --
and three of them now have a decision each. This is the fourth, and it is
the one the original list got wrong by naming only `exit`.

The list should have said *process*, not *exit*. dash reaches for the
process in more ways than terminating it, and the two it reaches for
without any user-visible syntax are the dangerous ones:

* **It replaces its own image.** `dash -c 'exec echo REPLACED; echo
  NOT-REACHED'` prints the first and never runs the second, which is
  `exec`'s documented job. But `dash -c 'ls'` *also* `execve`s in place --
  `evalcommand`'s `EV_EXIT` arm at `eval.rs:1389`, an optimisation with no
  syntax attached to it at all. An embedder reading the shell's manual
  would defend against the first and never see the second.
* **It takes the terminal and moves its own process group.** `set -m`
  reaches `setjobctl`, which calls `setpgid(0, rootpid)` and `tcsetpgrp`
  on the process the embedder is running in, and on the way there may
  `killpg(0, SIGTTIN)` the entire group into a stop.

Forking is not on that list and must not be. A child process is a thing
the library *makes*; it is not a thing it does to the host. Every
redirection, pipeline, subshell and external command is a fork, and a
shell that had to ask permission to fork would be asking permission to be
a shell.

So the line is not "syscalls the library may call". It is **whose process
is the object of the call**. `setpgid` on a child the shell just forked is
ordinary work; `setpgid` on the shell's own process is the host's
business. The same syscall, on the same line of the same function, twice
(`jobs.rs:992` in the child, `:1079` in the parent) -- and only one of them
is a grant.

## The three grants, and why they are three

**Replace the image.** `Host::may_replace_process`, which
`docs/api-design.md` 5.4 already proposes. This decision adds the second
call site and the constraint that follows from it: `Shell::run` passes no
`EV_EXIT`, so the optimisation is available to `nsh-cli` and unreachable
from the API. A frontend replacing its own image is what `exec` means; a
library doing it silently is the sharpest available example of the ban.

**Own the terminal.** One grant covers process groups and `tcsetpgrp`
together, because `ttyfd` is the interlock: `xxtcsetpgrp` returns `Ok(())`
when `ttyfd < 0`, and only `setjobctl` ever sets it. Gate `setjobctl` and
every terminal operation downstream is gated with it, including the ones
in forked children -- which is the right answer even though a child's
`tcsetpgrp` is not an operation on the host, because a child stealing the
terminal from the host's foreground group is the same theft performed one
process away.

**End the process.** This one is already answered and it is answered by
*deletion* rather than by a grant. `docs/api-design.md` 5.4 is right that
after the builder the library never needs to terminate: `run` returns and
`nsh-cli` calls `std::process::exit`. The grant that does not exist is the
strongest form of the ban.

## What the library still does freely, stated so it is not re-litigated

`fork`, `vfork`, `execve` **in a child it forked**, `setpgid` on such a
child, `_exit` in such a child, `wait3`, `sigsuspend`, and `kill` on a pid
a script named. The last is worth a sentence: the `kill` builtin will
signal anything the user's script asks it to, including the host. That is
not the library exceeding its authority -- it is the script exercising
the embedder's, and a shell that could not `kill` would not be one. An
embedder running untrusted script has a sandbox problem, not an API
problem, and `docs/api-design.md` 10.5 already says the same thing about
`expand_word` running `$(...)`.

## Reaping, which is the entry `no-ambient-state` did not have

`[dec:nsh:no-ambient-state]` records the process-wide facts two shells
share, and `docs/api-design.md` 6 lists them: the locale, `strtok`,
`getopt`, the environment, the working directory, the signal inbox. There
is a seventh and it is not a C library global -- it is the kernel's.

**The children of a process are one pool, and `wait3(-1)` drains it.**
Two `Shell`s in one process reap each other's children. A `Shell` and an
embedder that spawned its own `Command` do the same, and the embedder is
the one that loses: it gets `ECHILD` from a `wait` on a child the shell
already reaped and whose status is now in a job table it cannot see.

This cannot be fixed by tracking pids, and the reason is worth recording
because it is not obvious. Reaping is destructive, so a "is this one mine?"
test has to happen *before* the reap: `waitid(P_ALL, ..., WNOWAIT)` does
exactly that. But when the only exited child belongs to the embedder,
`WNOWAIT` returns it again immediately and forever, and the blocking wait
becomes a spin. The shell would need to own `SIGCHLD` for the whole
process and dispatch by pid -- which is precisely the disposition
`[dec:nsh:host-owns-signals]` says it may not claim.

So it is documented, in the same sentence as the locale, with the same
honesty: an embedder that spawns its own children and runs a `Shell` in
the same process is sharing something it cannot divide.

## Correction, 2026-09-02

**The three paragraphs above are wrong, and the shell now divides the
pool.** They are kept because the shape of the mistake is instructive: an
argument about the *filter* was mistaken for an argument about the
*question*.

Every step reasons about `waitpid(-1)` and what to do with the answer.
Given that call, all of it is correct -- the answer may be a foreign
child, the reap has already happened by the time you can look, and
`WNOWAIT` is the only way to look first, at the price of a spin. What was
never asked is whether `waitpid(-1)` had to be the call. It did not.
`waitpid(pid)` names the process, and a status this shell is not entitled
to is one it never asks for. No filter, no peek, no dispatch, and no
`SIGCHLD` claim.

The one real obstacle was the one the refutation named and then drew the
wrong conclusion from. The shell does fork children with no job entry --
`record_forked_child` returns early when it is given no job, so the
here-document writer and the process-substitution child are in no job
table -- and a set derived from the job table would have left them
unreaped. The answer is that the set is not derived from the job table.
It is written by `fork_shell` and `fork_and_execute`, which is where a
fork actually happens, and it holds every pid whether or not a job was
made for it.

What this changes about the decision: the decision itself stands
unaltered -- creating a process is the library's, and replacing,
regrouping or ending the host's is the host's to grant. What changes is
that reaping is no longer one of the process-wide facts two shells share.
`docs/api-design.md` 6 loses the entry; the locale, `strtok`, `getopt`,
the environment, the working directory and the signal inbox keep theirs.

What it does not change: a forked child inherits its parent's pid set by
copy and must drop it, because those processes are its siblings and not
its children. `jobs::fork::initialize_child_process` clears it, in the
same place it turns off job control.
