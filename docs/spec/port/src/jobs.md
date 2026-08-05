# src/jobs.c, src/jobs.h

Job control and process management. `jobtab` is a dynamically grown array
of `njobs` `struct job`; a job's number is its index + 1. `curjob` heads
a `prev_job` chain that is *not* array order — it is the "current job"
ordering that `%+`, `%-` and the `+`/`-` markers in `jobs` output use,
maintained by `set_curjob`.

A job holds `nprocs` `struct procstat`, one per pipeline element, in the
inline `ps0` when there is exactly one and in a separate allocation
otherwise; `ps` points at whichever is in use. A `procstat`'s `status` is
-1 while the process is still running. Job `state` is `JOBRUNNING` (0),
`JOBSTOPPED` (1) or `JOBDONE` (2). The remaining fields are bitfields:
`sigint` (the job died of SIGINT), `jobctl` (it was created under job
control), `waited`, `used`, `changed`.

`forkshell` modes: `FORK_FG` (0), `FORK_BG` (1), `FORK_NOJOB` (2 — like
foreground but without its own process group). `FORK_FG` and `FORK_BG`
must keep those values, as `fgcmd` derives them arithmetically.

`showjob` modes: `SHOW_PGID` (0x01), `SHOW_PID` (0x04),
`SHOW_CHANGED` (0x08).

`dowait` modes: `DOWAIT_NONBLOCK` (0), `DOWAIT_BLOCK` (1),
`DOWAIT_WAITCMD` (2), `DOWAIT_WAITCMD_ALL` (4).

`set_curjob` modes: `CUR_STOPPED` (0), `CUR_RUNNING` (1),
`CUR_DELETE` (2).

Under job control the shell keeps `ttyfd`, a saved descriptor for the
control terminal, and `initialpgrp`, the process group it was started in,
so it can hand the terminal back on exit. `vforked` is non-zero inside a
`vfork` child — which shares the parent's address space, so almost every
routine that writes global state checks it.

Everything compiled under `#if JOBS` is absent from a build without job
control.

> [spec:dash:def:jobs.bgcmd-fn]
> int bgcmd(int argc, char **argv) #ifdef HAVE_ALIAS_ATTRIBUTE __attribute__((__alias__("fgcmd"))); #else

> [spec:dash:sem:jobs.bgcmd-fn]
> The `bg` builtin. It is the same function as `fgcmd` — an alias where
> the compiler supports `__attribute__((alias))`, and a forwarding
> wrapper otherwise. `fgcmd` distinguishes the two by inspecting
> `argv[0]`. The generated `def` above includes the surrounding
> preprocessor text; the real signature is
> `int bgcmd(int argc, char **argv)`.

> [spec:dash:def:jobs.cmdlist-fn]
> STATIC void cmdlist(union node *np, int sep)

> [spec:dash:sem:jobs.cmdlist-fn]
> Render a list of `narg`-linked nodes into the command-text buffer. With
> `sep` non-zero, emit a space *between* items (arguments). With `sep`
> zero, emit a space *before* every item including the first
> (redirections, which follow the arguments).

> [spec:dash:def:jobs.cmdputs-fn]
> STATIC void cmdputs(const char *s)

> [spec:dash:sem:jobs.cmdputs-fn]
> Append `s` to the command-text buffer at `cmdnextc`, translating the
> shell's internal control bytes back into source syntax. Reserve
> `(strlen(s) + 1) * 8` bytes up front, the worst-case expansion, so the
> loop can use the unchecked `USTPUTC`.
>
> Track `subtype`, the pending parameter-expansion subtype, and `quoted`,
> a *stack of bits* — shifted left on entering a nested expansion and
> right on leaving — recording whether each level was inside double
> quotes.
>
> Per byte: `CTLESC` emits the following byte literally. `CTLVAR` reads
> the subtype and emits `${#` for `VSLENGTH` or `${`. `CTLENDVAR` emits
> `"}` or `}` depending on the current quote bit, then pops it.
> `CTLBACKQ` emits `$(...)` — the substituted command's own text is not
> reproduced. `CTLARI`/`CTLENDARI` emit `$((` and `))`. `CTLQUOTEMARK`
> toggles the low quote bit and emits `"`.
>
> A literal `=` is only special while a subtype is pending: it emits the
> operator from the `vstype` table (`""`, `}`, `-`, `+`, `?`, `=`, `%`,
> `%%`, `#`, `##` indexed by `subtype & VSTYPE`), preceded by `:` when
> `VSNUL` is set, and pushes a quote level for any non-`VSNORMAL` type.
> The characters `'`, `\`, `"` and `$` can only occur here inside quotes,
> so they are emitted backslash-escaped.
>
> At the end, close an unbalanced quote and NUL-terminate, leaving
> `cmdnextc` at the terminator.

> [spec:dash:def:jobs.cmdtxt-fn]
> STATIC void cmdtxt(union node *n)

> [spec:dash:sem:jobs.cmdtxt-fn]
> Recursively render a parse tree as shell source into the command-text
> buffer, for `jobs` output. A NULL node emits nothing. The `default:`
> arm holds only `abort()` under `DEBUG`; in a release build it is empty
> and falls through into `NPIPE`, so an unrecognised node is rendered as
> a pipeline — dereferencing `npipe.cmdlist`. Reproduce the
> fall-through.
>
> Renderings: `NPIPE` joins its elements with `" | "`; `NSEMI`, `NAND`,
> `NOR` join their two children with `"; "`, `" && "`, `" || "`;
> `NREDIR` and `NBACKGND` render only their inner command (the `&` and
> the redirections are not shown); `NNOT` prefixes `!`; `NIF` renders
> `if <test>; then <part>[; else <part>]; fi`; `NSUBSHELL` wraps in
> parentheses; `NWHILE`/`NUNTIL` render `while|until <cond>; do <body>;
> done`; `NFOR` renders `for <var> in <args>; do <body>; done`; `NDEFUN`
> renders `<name>() { ... }`, eliding the body; `NCMD` renders its
> arguments then its redirections; `NARG` emits its text; here documents
> render as `<<...`; `NCASE` renders `case <word> in <pat>) <body>;; …
> esac`.
>
> Redirections render as the descriptor digit, the operator (`>`, `>|`,
> `>>`, `>&`, `<`, `<&`, `<>`), and then either the duplicated descriptor
> digit or the filename node. Note the descriptor is rendered as
> `fd + '0'`, a single character, so a descriptor above 9 prints as
> garbage.

> [spec:dash:def:jobs.commandtext-fn]
> STATIC char * commandtext(union node *n)

> [spec:dash:sem:jobs.commandtext-fn]
> Produce a heap-allocated string describing the command `n`, for display
> by `jobs`. Start a stack string in the module-global `cmdnextc`, render
> with `cmdtxt`, then `savestr` the result off the stack so it survives
> the next stack reset. The caller (a `procstat`) owns and frees it.

> [spec:dash:def:jobs.dowait-fn]
> static int dowait(int block, struct job *jp)

> [spec:dash:sem:jobs.dowait-fn]
> Reap children until there are none left to reap, updating job state.
> Returns 0 only when a `waitcmd` wait was interrupted by a signal, and
> 1 otherwise.
>
> Read `gotsigchld` through a `volatile` cast so the compiler cannot cache
> it. If the target job is already not running there is nothing to wait
> for, so force non-blocking. A non-blocking call with no SIGCHLD pending
> returns 1 immediately — the common no-op case.
>
> Otherwise loop `waitone`, accumulating `rpid &= !!pid` so a single 0
> return (interrupted) makes the result 0. Clear `DOWAIT_WAITCMD_ALL`
> after the first iteration, and drop to non-blocking once nothing was
> reaped or the target job has stopped running. Continue while `waitone`
> returns non-negative; a negative return means no more children.

> [spec:dash:def:jobs.fgcmd-fn]
> int fgcmd(int argc, char **argv)

> [spec:dash:sem:jobs.fgcmd-fn]
> Implements both `fg` and `bg`; `**argv == 'f'` selects `FORK_FG` over
> `FORK_BG`. Consume options with `nextopt(nullstr)`, then for each
> operand: resolve it with `getjob(*argv, 1)` — requiring a job-control
> job. For `bg`, move it to the front of the running jobs with
> `set_curjob(jp, CUR_RUNNING)` and print `[n] `. Print the job's command
> text and the rest of its pipeline, then `restartjob`.
>
> The loop condition `*argv && *++argv` means a missing operand still runs
> one iteration, resolving the current job. Returns the last `restartjob`
> result: the job's status for `fg`, 0 for `bg`.

> [spec:dash:def:jobs.forkchild-fn]
> static void forkchild(struct job *jp, union node *n, int mode)

> [spec:dash:sem:jobs.forkchild-fn]
> Set up the child side of a fork. Note it runs in a `vfork` child too
> (`vforked` non-zero), where it shares the parent's memory and so must
> not modify shell state — most of the body is skipped in that case.
>
> For a real fork: clear `mypid` (recomputed lazily), increment `shlvl`,
> run `forkreset` — passing `n` only for `FORK_NOJOB`, so the subshell's
> command is available for the `trap`-command special case — and turn
> `jobctl` off, since only the root shell does job control.
>
> Then set up process group and signals. Under job control, for a job
> that gets its own group and is not already a nested shell: the group is
> this process's pid for the first element of a pipeline and the first
> element's pid for the rest; `setpgid(0, pgrp)` (which may fail
> harmlessly because the parent does it too — whichever wins, the result
> is the same); for a foreground job, hand the terminal over with
> `xxtcsetpgrp`; and restore default handling for SIGTSTP and SIGTTOU so
> the child can actually be stopped. Otherwise, for a background job
> without job control, ignore SIGINT and SIGQUIT and — for the first
> element only — redirect stdin from `/dev/null`, so a background job
> does not steal terminal input. In an interactive root shell, re-derive
> SIGINT, SIGQUIT and SIGTERM handling for the child.
>
> Finally (real fork only) release job bookkeeping the child does not
> own: free its own job, then — unless the child is a plain `jobs`
> command, which needs to report them — free every remaining job, so the
> child does not later report or wait on the parent's jobs.

> [spec:dash:def:jobs.forkparent-fn]
> static void forkparent(struct job *jp, union node *n, int mode, pid_t pid)

> [spec:dash:sem:jobs.forkparent-fn]
> Record a newly forked child in the parent. A negative `pid` is a fork
> failure: free the job and raise `sh_error("Cannot fork")`.
>
> With no job structure there is nothing to record. Otherwise, under job
> control and when the job gets its own group, `setpgid(pid, pgrp)` with
> the same group rule as the child (its own pid for the first element,
> the first element's for the rest) — done in both processes so neither
> can race ahead. For a background job, set `backgndpid` (`$!`), move the
> job to the front of the running list, and — if the shell is interactive
> — announce it on stderr as `"[%d] %d\n"`, job number then pid.
>
> That announcement is a **fix, upstreamable**, not upstream behaviour.
> POSIX XCU 2.9.3.1 requires it of an interactive shell; dash wrote
> nothing, so `[%d] ` appeared only in the `jobs` listing and completion
> formats and there was no way to learn a job's number except by running
> `jobs`. Found by the POSIX case `jobctl-async-notification-format`.
> Gated on `iflag`, so no non-interactive behaviour changes.
>
> Then append a `procstat`: the pid, `status = -1` (running), and the
> command text — `nullstr` unless job control is on and a command node
> was supplied, in which case `commandtext(n)` renders it for `jobs`.

> [spec:dash:def:jobs.forkshell-fn]
> int forkshell(struct job *jp, union node *n, int mode)

> [spec:dash:sem:jobs.forkshell-fn]
> Fork, dispatching to `forkchild` or `forkparent`, and return the `fork`
> result. `flush_input()` first, so buffered read-ahead is given back
> before it would be duplicated into the child. Must be called with
> interrupts off. `jp` and `n` may both be NULL.

> [spec:dash:def:jobs.freejob-fn]
> STATIC void freejob(struct job *jp)

> [spec:dash:sem:jobs.freejob-fn]
> Mark a job slot unused. With interrupts suspended, free each
> `procstat`'s command text (skipping the shared `nullstr`), free the
> `ps` array unless it is the inline `ps0`, clear `used`, and unlink the
> job from the current-job chain with `set_curjob(jp, CUR_DELETE)`.

> [spec:dash:def:jobs.getjob-fn]
> job * getjob(const char *name, int getctl)

> [spec:dash:sem:jobs.getjob-fn]
> Resolve a job specification to a `struct job`, raising `sh_error` on
> failure — it never returns NULL. `getctl` additionally requires the job
> to have been created under job control.
>
> A NULL `name`, or `%` alone, or `%%`, or `%+` means the current job;
> `%-` means the previous one (`curjob->prev_job`). A name not starting
> with `%` is an error. `%<number>` selects by job number when it is in
> range and the slot is used. Otherwise it is a pattern: `%string`
> matches jobs whose command text *starts with* `string` (via `prefix`),
> and `%?string` matches those *containing* it (via `strstr`). A pattern
> matching more than one job is an error — `"%s: ambiguous"` — and the
> scan continues to the end rather than stopping at the first match, so
> ambiguity is always detected.
>
> The error message is selected as the scan proceeds, so the diagnostic
> names the specific failure: `"No such job: %s"`, `"No current job"`,
> `"No previous job"`, `"%s: ambiguous"`, or
> `"job %s not created under job control"`.

> [spec:dash:def:jobs.getstatus-fn]
> STATIC int getstatus(struct job *job)

> [spec:dash:sem:jobs.getstatus-fn]
> Convert a finished job's raw wait status into a shell exit status. Take
> the *last* process's status. Under `pipefail`, scan leftward while the
> status is 0, so the rightmost non-zero status wins.
>
> A normally exited process contributes `WEXITSTATUS`. Otherwise the
> status is a signal: `WSTOPSIG` when stopped, `WTERMSIG` when killed —
> and for a `SIGINT` kill, set the job's `sigint` flag so `waitforjob`
> knows to re-raise it. Signal statuses are reported as `signal + 128`.

> [spec:dash:def:jobs.growjobtab-fn]
> job * growjobtab(void)

> [spec:dash:sem:jobs.growjobtab-fn]
> Extend `jobtab` by four slots and return the first new one. `ckrealloc`
> the array; if it moved, every pointer *into* it must be adjusted by the
> byte offset. Walk the existing entries backwards, relocating each
> `prev_job` link and — only where `ps` pointed at that job's own inline
> `ps0`, not at a separate allocation — the `ps` pointer. Then relocate
> `curjob`. The `joff`/`jmove` macros exist to express "the same field of
> the entry at offset `l`, in the new array".
>
> Finally add 4 to `njobs`, mark the four new slots unused, and return
> the first.

> [spec:dash:def:jobs.job]
> struct job {
>   struct procstat ps0;
>   struct procstat *ps;
>   uint32_t nprocs: 16, /* number of processes */ state: 8, #define JOBRUNNING 0 /* at least one proc running */ #define JOBSTOPPED 1 /* all procs are stopped *...;
>   struct job *prev_job;
> }

> [spec:dash:def:jobs.jobno-fn]
> STATIC int jobno(const struct job *jp)

> [spec:dash:sem:jobs.jobno-fn]
> Return the job's user-visible number: its index in `jobtab` plus 1.

> [spec:dash:def:jobs.jobscmd-fn]
> int jobscmd(int argc, char **argv)

> [spec:dash:sem:jobs.jobscmd-fn]
> The `jobs` builtin. Parse `-l` (`SHOW_PID`, include process ids) and
> `-p` (`SHOW_PGID`, print only the process group id); the last one given
> wins, as `mode` is assigned rather than OR-ed. With operands, show each
> named job; with none, `showjobs(out, mode)` shows all. Return 0.

> [spec:dash:def:jobs.killcmd-fn]
> int killcmd(int argc, char **argv)

> [spec:dash:sem:jobs.killcmd-fn]
> The `kill` builtin. With no arguments, print the usage message via
> `sh_error`.
>
> If the first argument starts with `-`, first try to read the rest as a
> signal name or number with `decode_signal(…, 1)` — `minsig` 1 excludes
> `EXIT` — which handles `-9` and `-TERM`. If that fails it must be an
> option: parse `-l` (list) and `-s sigspec`, rejecting an unknown spec
> with `"invalid signal number or name: %s"`. Default the signal to
> `SIGTERM` when not listing.
>
> That option `switch` also has a `default:` arm holding only `abort()`
> under `DEBUG`, which in a release build is empty and falls through into
> `case 'l'`. Reproduce the fall-through.
>
> Validate the combination: `(signo < 0 || !*argv) ^ list` must be false,
> i.e. listing requires no signal and sending requires operands;
> otherwise print usage.
>
> Listing with no operand prints `0` followed by every signal name.
> Listing with one converts it to a number, subtracts 128 if above (so a
> `$?` value from a signal death can be passed straight in), and prints
> the name — or raises
> `"invalid signal number or exit status: %s"`.
>
> Otherwise send the signal to each operand: `%job` resolves through
> `getjob` and targets the negated first pid, i.e. the process group;
> `-n` is an explicit negative pid (also a group); anything else is a
> pid. A failed `kill` warns with `strerror` and makes the result 1.

> [spec:dash:def:jobs.makejob-fn]
> struct job *makejob(int nprocs)

> [spec:dash:sem:jobs.makejob-fn]
> Allocate a job for `nprocs` processes. Must be called with interrupts
> off. Scan `jobtab` for a slot: an unused one is taken immediately; a
> used one is reclaimed only if it is `JOBDONE`, has been waited for, and
> job control is *off* — under job control a finished job must survive
> until reported. Running out of slots calls `growjobtab`.
>
> Zero the slot, point `ps` at the inline `ps0` or at a fresh array for
> more than one process, set `jobctl` when job control is on, push it
> onto the front of the current-job chain, and mark it used. `nprocs` is
> left at 0 — it is incremented by `forkparent` as processes are actually
> created, so it tracks how many have been started rather than how many
> were requested.

> [spec:dash:def:jobs.onsigchild-fn]
> STATIC int onsigchild(void)

> [spec:dash:sem:jobs.onsigchild-fn]
> Declared under `#ifdef SYSV` but never defined anywhere in the tree — a
> vestige of System V SIGCHLD handling that was removed. There is nothing
> to port; the prototype is dead. Wave 2 should carry the annotation on
> an equivalent placeholder, or record the omission.

> [spec:dash:def:jobs.procstat]
> struct procstat {
>   pid_t pid;
>   int status;
>   char *cmd;
> }

> [spec:dash:def:jobs.restartjob-fn]
> STATIC int restartjob(struct job *jp, int mode)

> [spec:dash:sem:jobs.restartjob-fn]
> Resume a stopped job. With interrupts suspended, and skipping
> everything for an already-finished job: mark it `JOBRUNNING`, and for
> `FORK_FG` hand the terminal to its process group first. Send `SIGCONT`
> to the group, then reset every stopped process's status to -1 so it is
> treated as running again. Return `waitforjob(jp)` for a foreground
> resume and 0 for a background one.

> [spec:dash:def:jobs.set-curjob-fn]
> STATIC void set_curjob(struct job *jp, unsigned mode)

> [spec:dash:sem:jobs.set-curjob-fn]
> Maintain the current-job ordering that `%+`/`%-` and the `+`/`-`
> markers use. First unlink `jp` from the `prev_job` chain — the search
> is unbounded and assumes the job is present. Then re-insert according
> to `mode`: `CUR_DELETE` does not re-insert; `CUR_STOPPED` puts it at
> the very front, making it the current job; `CUR_RUNNING` skips past any
> leading stopped jobs first, so a newly started or backgrounded job
> ranks behind every stopped one — which is why `%+` prefers a stopped
> job. The `default:` arm holds only `abort()` under `DEBUG`; in a
> release build it is empty and falls through into `CUR_DELETE`, so an
> unrecognised mode silently unlinks without re-inserting. Reproduce the
> fall-through.

> [spec:dash:def:jobs.setjobctl-fn]
> void setjobctl(int on)

> [spec:dash:sem:jobs.setjobctl-fn]
> Turn job control on or off. Must be called with interrupts off. Does
> nothing if already in the requested state or if this is not the root
> shell.
>
> Turning **on**: obtain a descriptor for the control terminal — open
> `_PATH_TTY` read-write, and if that fails scan downward for a
> terminal among the low descriptors. Note what the fallback actually
> computes: `sh_open(..., mayfail=1)` returns `-errno`, so `fd += 3`
> yields `3 - errno` rather than a fixed starting point — for `ENOENT`
> (2) the scan starts at descriptor 1, and for any `errno >= 4` it is
> already `<= -1` and jumps straight to the failure exit. The intent is
> "try 2, then 1, then 0"; the code only does that for `errno == 1`.
> Reproduce the arithmetic, not the intent. Move it above 9 with
> `savefd`.
>
> Then wait until the shell is in the foreground: read the terminal's
> process group with `tcgetpgrp` and, while it differs from the shell's
> own, `killpg(0, SIGTTIN)` to stop the whole shell until someone
> foregrounds it. A `tcgetpgrp` failure, or a mismatch in a
> non-interactive shell, abandons the attempt. Abandoning in an
> interactive shell warns `"can't access tty; job control turned off"`
> and clears `mflag`; in a non-interactive one it proceeds silently
> without a terminal. Record the inherited group in `initialpgrp` and
> target `rootpid`.
>
> Turning **off**: reuse the saved `ttyfd` and target `initialpgrp`, so
> the terminal goes back to whoever had it.
>
> Either way, re-derive SIGTSTP, SIGTTOU and SIGTTIN handling, then
> `setpgid(0, pgrp)` and `xtcsetpgrp(fd, pgrp)` — putting the shell in
> its own group and giving it the terminal when enabling, and reversing
> that when disabling. Close the descriptor when disabling. Record
> `ttyfd` and `jobctl`.

> [spec:dash:def:jobs.showjob-fn]
> static void showjob(struct output *out, struct job *jp, int mode)

> [spec:dash:sem:jobs.showjob-fn]
> Print one job. With `SHOW_PGID`, print just the first process's pid and
> return.
>
> Otherwise build a status line into a buffer: `"[%d]   "` with the job
> number, then overwrite the character two before the current column with
> `+` if this is the current job or `-` if it is the previous one. With
> `SHOW_PID`, append the first pid. Then append the state: the literal
> `Running`, or `sprint_status` of the last process's status — or of
> `stopstatus` when the job is stopped, since that records which signal
> stopped it.
>
> Emit the status line padded to column 33 followed by the command text.
> Without `SHOW_PID` that is the whole job: `showpipe` appends the rest
> of the pipeline on the same line. With `SHOW_PID`, loop over the
> remaining processes, each on its own line prefixed `" |\n"` and
> indented to line up, showing its pid and command.
>
> Clear `changed`, and free the job if it is `JOBDONE` — reporting a
> finished job is what retires it.

> [spec:dash:def:jobs.showjobs-fn]
> void showjobs(struct output *out, int mode)

> [spec:dash:sem:jobs.showjobs-fn]
> Print the job list. First `dowait(DOWAIT_NONBLOCK, NULL)` to reap any
> finished children so their state is current. Then walk the current-job
> chain, printing each with `showjob`; with `SHOW_CHANGED`, print only
> jobs whose `changed` flag is set — which is how the interactive prompt
> reports newly finished jobs without listing everything.

> [spec:dash:def:jobs.showpipe-fn]
> STATIC void showpipe(struct job *jp, struct output *out)

> [spec:dash:sem:jobs.showpipe-fn]
> Append the *remaining* elements of a pipeline — from `ps[1]` on, since
> the caller has already printed `ps[0]` — each as `" | <cmd>"`, then a
> newline, then `flushall()`.

> [spec:dash:def:jobs.sprint-status-fn]
> STATIC int sprint_status(char *os, int status, int sigonly)

> [spec:dash:sem:jobs.sprint-status-fn]
> Format a wait status into `os`, returning the number of characters
> written. `sigonly` restricts output to the cases worth reporting
> spontaneously.
>
> For a process that did not exit normally, take `WSTOPSIG` if stopped
> and `WTERMSIG` otherwise, and write `strsignal` of it (bounded to 32
> bytes with `stpncpy`), followed by `" (core dumped)"` where
> `WCOREDUMP` says so. Under `sigonly`, write nothing for SIGINT or
> SIGPIPE — the user caused those and does not need telling — nor for a
> stopped process.
>
> For a normal exit, and only when not `sigonly`, write `"Done"` or
> `"Done(%d)"` with a non-zero status. Under `sigonly` a normal exit
> produces nothing, which is how the caller detects "nothing to report".

> [spec:dash:def:jobs.stoppedjobs-fn]
> int stoppedjobs(void)

> [spec:dash:sem:jobs.stoppedjobs-fn]
> Return 1 and print `"You have stopped jobs.\n"` if the current job is
> stopped, else 0. Returns 0 immediately when `job_warning` is already
> set, so the warning is given only once — `job_warning` is set to 2 here
> and decayed by `cmdloop`, giving the user two commands' grace in which
> a second `exit` succeeds. Always 0 in a build without job control.

> [spec:dash:def:jobs.vforkexec-fn]
> struct job *vforkexec(union node *n, char **argv, const char *path, int idx)

> [spec:dash:sem:jobs.vforkexec-fn]
> Run an external command with `vfork` instead of `fork`, avoiding the
> address-space copy. Make a one-process job, ensure `mypid` is
> populated, and set the global `vforked` to it — which is how every
> routine that would otherwise write shell state knows to skip. `vfork`,
> and in the child run `forkchild(jp, n, FORK_FG)` followed by
> `shellexec`, which never returns. In the parent clear `vforked` and run
> `forkparent`. Return the job.
>
> The child shares the parent's memory until the exec, so the code it
> runs must not modify anything the parent will read — which is what all
> the `vforked` guards enforce.

> [spec:dash:def:jobs.waitcmd-fn]
> int waitcmd(int argc, char **argv)

> [spec:dash:sem:jobs.waitcmd-fn]
> The `wait` builtin. With no operands, wait for every job: repeatedly
> scan the current-job chain for one still `JOBRUNNING`, marking each
> non-running job `waited` as it passes; when none is found, return 0.
> Otherwise `dowait(DOWAIT_WAITCMD_ALL, 0)`, and a 0 result means a
> signal interrupted the wait.
>
> With operands, start from 127 — the status for an operand that names
> nothing. For each: a non-`%` operand is a pid, matched against the
> *last* process of each job (the one whose status the job reports); an
> unmatched pid silently skips to the next operand, leaving the status at
> 127. A `%` operand goes through `getjob`. Then `dowait(DOWAIT_WAITCMD,
> job)` until it stops running, mark it `waited`, and take its status
> with `getstatus`.
>
> An interrupted wait returns `128 + pending_sig`.

> [spec:dash:def:jobs.waitforjob-fn]
> int waitforjob(struct job *jp)

> [spec:dash:sem:jobs.waitforjob-fn]
> Wait for a job to finish and return its status. Must be called with
> interrupts off. A NULL job just reaps non-blockingly and returns the
> current `exitstatus` — which is what makes `evalcommand` able to call
> it unconditionally.
>
> Otherwise block until the job is done, then `getstatus`. Under job
> control, hand the terminal back to `rootpid`; and because the shell was
> not in the foreground process group while the job ran, it never
> received the user's `^C` — so if `getstatus` observed a SIGINT death,
> re-raise SIGINT here so the shell reacts as if it had.
>
> Then `if (! JOBS || jp->state == JOBDONE) freejob(jp);`. `JOBS` is the
> compile-time constant from `shell.h` (1 in every shipped build), *not*
> the runtime `jobctl` flag — so in practice the job is freed **iff** its
> state is `JOBDONE`, whatever `set -m` says. A stopped job therefore
> persists so it can be resumed.
>
> The shell deliberately ignores interrupts while waiting on a foreground
> process and then re-raises them, so that an interrupt aimed at the
> child does not also abort an enclosing loop. Programs that catch SIGINT
> and then `exit` rather than re-raising it defeat this.

> [spec:dash:def:jobs.waitone-fn]
> static int waitone(int block, struct job *job)

> [spec:dash:sem:jobs.waitone-fn]
> Reap one child and update the job it belongs to. Returns the pid, 0 if
> interrupted, or negative when there are no children.
>
> With interrupts suspended, `waitproc(block, &status)`. On a positive
> pid, search the jobs (skipping finished ones) for the matching
> `procstat`, store the status there, and simultaneously re-derive the
> job's aggregate state: `JOBDONE` unless some process still has status
> -1 (`JOBRUNNING`), or — under job control, once no process is still
> running — `JOBSTOPPED` if any is stopped, recording that raw status in
> `stopstatus`.
>
> When the state is no longer running, mark the job `changed` so
> `showjobs` will report it, and on an actual state transition update it,
> moving a newly stopped job to the front of the current-job chain.
>
> Finally, when the reaped job is the one the caller asked about, format
> its status with `sprint_status(..., 1)` and, if that produced anything,
> write it to `out2` — this is the spontaneous "Terminated"/"Stopped"
> notice.

> [spec:dash:def:jobs.waitproc-fn]
> STATIC int waitproc(int block, int *status)

> [spec:dash:sem:jobs.waitproc-fn]
> Perform one wait system call. `WNOHANG` unless `block` is
> `DOWAIT_BLOCK`; `WUNTRACED` as well under job control, so stopped
> children are reported.
>
> Clear `gotsigchld`, then `wait3` (or `waitpid(-1, …)`) retrying on
> `EINTR`. Return immediately on any result, or — via `err = -!block` —
> return -1 for a non-blocking call that found nothing.
>
> For a `DOWAIT_WAITCMD` wait, the non-blocking wait is combined with
> `sigsuspend` so that any signal, not just SIGCHLD, ends the wait
> promptly: block all signals, `sigsuspend` until `gotsigchld` or
> `pending_sig` is set, then unblock. Loop back to the wait only if it
> was SIGCHLD that woke us; a different signal falls out with 0, which
> `dowait` reports as an interruption.
>
> Note the caller of a non-blocking `dowait` must keep calling until
> every dead child is reaped, or zombies accumulate.

> [spec:dash:def:jobs.xtcsetpgrp-fn]
> STATIC void xtcsetpgrp(int fd, pid_t pgrp)

> [spec:dash:sem:jobs.xtcsetpgrp-fn]
> `tcsetpgrp` with all signals blocked around it — otherwise the call
> would deliver SIGTTOU to the very process making it — raising
> `sh_error("Cannot set tty process group (%s)", strerror(errno))` on
> failure.

> [spec:dash:def:jobs.xxtcsetpgrp-fn]
> static void xxtcsetpgrp(pid_t pgrp)

> [spec:dash:sem:jobs.xxtcsetpgrp-fn]
> `xtcsetpgrp(ttyfd, pgrp)`, doing nothing when there is no saved
> terminal descriptor — the case where job control was requested but no
> control terminal could be opened.
