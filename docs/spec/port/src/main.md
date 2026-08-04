# src/main.c

Entry point and the top-level read-eval loop. `rootpid` is the pid of the
shell that started everything and `mypid` the current process's pid;
`shlvl` records nesting depth. `main_handler` is the outermost
`struct jmploc`, declared `MKINIT` so the generated init code can reset
it: the `FORKRESET` block at the end of the file re-points `handler` at
it after a fork, so a child that raises an exception unwinds to its own
top level rather than into inherited state.

> [spec:dash:def:main.cmdloop-fn]
> static int cmdloop(int top)

> [spec:dash:sem:main.cmdloop-fn]
> Read and execute commands until end of input. `top` is non-zero for the
> interactive top-level loop and zero for sourced files. Track `status`
> (last command's exit status, initially 0) and `numeof` (consecutive
> end-of-file reads, initially 0).
>
> Each iteration: `setstackmark` to reclaim per-command stack allocation;
> if job control is on (`jobctl`) report finished/changed jobs with
> `showjobs(out2, SHOW_CHANGED)`; set `inter` non-zero — and check for
> mail with `chkmail()` — only when both `iflag` and `top` hold. Parse one
> command with `parsecmd(inter)`, where `inter` requests prompting.
>
> If the parse returned `NEOF`: break out immediately when not at top
> level, or once `numeof` has reached 50 — a backstop against spinning on
> a terminal that reports EOF forever. Otherwise consult `stoppedjobs()`;
> when there are none, and `Iflag` (`ignoreeof`) is not set, emit a
> newline to `out2` if interactive and break. When `ignoreeof` is set, or
> there were stopped jobs, print `"\nUse \"exit\" to leave shell.\n"` and
> keep going. Increment `numeof` in the non-breaking cases.
>
> Otherwise a command was parsed: decay `job_warning` from 2 to 1 and
> from anything else to 0, which gives the "you have stopped jobs"
> warning a two-command lifetime; reset `numeof` to 0; evaluate with
> `evaltree(n, 0)` and adopt its result as `status` when `n` is
> non-NULL. Then `popstackmark`.
>
> Finally, if `evalskip` is set — a `break`, `continue`, `return` or
> `exit` propagating out — clear the `SKIPFUNC` and `SKIPFUNCDEF` bits
> and break out of the loop, leaving any remaining skip bits for the
> caller to act on. Return `status`.

> [spec:dash:def:main.dotcmd-fn]
> int dotcmd(int argc, char **argv)

> [spec:dash:sem:main.dotcmd-fn]
> The `.` (dot / source) builtin. Consume options with `nextopt(nullstr)`
> — an empty option string, so any leading `-` is rejected — and take the
> operands from `argptr`. With no operand do nothing and return 0. With
> one, resolve it via `find_dot_file`, push it as the input source with
> `setinputfile(fullname, INPUT_PUSH_FILE)`, set `commandname` to the
> resolved name so diagnostics point at the sourced file, run
> `cmdloop(0)`, then `popfile()` to restore the previous input. Return
> the status `cmdloop` reported. Extra operands beyond the first are
> ignored — dash does not support passing positional parameters to `.`.

> [spec:dash:def:main.etext-fn]
> extern int etext()

> [spec:dash:sem:main.etext-fn]
> Not a function of this program: `etext` is the linker-provided symbol
> marking the end of the text segment. It is declared only under
> `#if PROFILE` and its address is passed to `monitor()` to bound the
> profiling range. There is nothing to port — a Rust build gets profiling
> from its own tooling — so Wave 2 should carry the annotation on an
> equivalent no-op or profiling-setup site rather than reimplement it.

> [spec:dash:def:main.exitcmd-fn]
> int exitcmd(int argc, char **argv)

> [spec:dash:sem:main.exitcmd-fn]
> The `exit` builtin. If `stoppedjobs()` reports stopped jobs, refuse:
> return 0 without exiting (the check itself prints the warning, and the
> `job_warning` decay in `cmdloop` makes a second `exit` succeed). With
> an operand, set `savestatus = number(argv[1])`, which parses it as a
> number and errors on garbage; with none, leave `savestatus` so the
> status of the last command is used. Then `exraise(EXEXIT)`, which
> unwinds to `main` and does not return.

> [spec:dash:def:main.find-dot-file-fn]
> STATIC char * find_dot_file(char *basename)

> [spec:dash:sem:main.find-dot-file-fn]
> Resolve the operand of `.` against `PATH`. If `basename` contains a
> `/` it is already an absolute or relative path — return it unchanged.
> Otherwise walk `PATH` with `padvance(&path, basename)`, which builds
> each candidate on the stack and returns its length, negative when
> exhausted. Accept the first candidate whose `pathopt` is absent or
> begins with `f` (the `%func` directory marker — a directory flagged for
> anything else is skipped) and which `stat64`s successfully as a regular
> file (`S_ISREG`); claim it with `stalloc(len)` and return it, leaving
> the caller to release the stack space. If nothing matches, raise
> `sh_error("%s: not found", basename)`, which does not return.

> [spec:dash:def:main.main-fn]
> int main(int argc, char **argv)

> [spec:dash:sem:main.main-fn]
> Shell entry point. Under glibc cache `__errno_location()` in
> `dash_errno`; under `PROFILE` start `monitor()`. The body is structured
> as a `setjmp` on `main_handler.loc` guarding a linear startup sequence,
> with a `state` variable (declared `volatile` so it survives the jump)
> recording how far startup has progressed, and labels `state1`..`state4`
> allowing re-entry at that point.
>
> On the initial (non-jump) pass: point `handler` at `main_handler`;
> under `DEBUG` open the trace file and log the arguments; set
> `mypid = rootpid = getpid()`; `init()`; `setstackmark(&smark)`; and
> `procargs(argv)` to parse the command line, which reports whether this
> is a login shell. If it is, `state = 1` and read `/etc/profile`, then
> `state = 2` and read `$HOME/.profile`. Then `state = 3` and, if `iflag`
> — and, on non-Linux, only when real and effective uid and gid all match,
> so a set-id shell does not source user files — read the file named by
> `ENV` when it is set and non-empty. `popstackmark`, `state = 4`. If
> `minusc` (a `-c` command string) is set, `evalstring` it, passing
> `EV_EXIT` unless `sflag` so the shell exits after it. Then, if `sflag`
> or there was no `-c`, enter `cmdloop(1)`. Fall through to `exitshell()`,
> which does not return.
>
> On a jump back: `exitreset()`, then read `exception` into `e` and
> `state` into `s`. Go straight to exit for `EXEND` or `EXEXIT`, or when
> `s == 0` (the exception arrived before startup got anywhere), or when
> not interactive, or when `shlvl` is non-zero — a nested shell does not
> try to recover. Otherwise `reset()` the parser and input state; for
> `EXINT` (and, where `ATTY` is configured, only when the terminal is not
> an ATTY-style one or `TERM` is `emacs`) emit a newline to `out2` so the
> prompt starts on a fresh line; `popstackmark(&smark)`; `FORCEINTON` to
> unconditionally re-enable interrupts, since the counter's nesting is
> lost across the jump; and resume at the label matching `s` — `state1`
> for 1, `state2` for 2, `state3` for 3, `state4` otherwise. Startup thus
> continues from where it failed instead of restarting, and a broken
> `/etc/profile` cannot wedge the shell.

> [spec:dash:def:main.read-profile-fn]
> STATIC void read_profile(const char *name)

> [spec:dash:sem:main.read-profile-fn]
> Source a startup file, tolerating its absence. Expand `name` first with
> `expandstr` — which is why `"$HOME/.profile"` can be passed as a
> literal. Push it with
> `setinputfile(name, INPUT_PUSH_FILE | INPUT_NOFILE_OK)`; the
> `INPUT_NOFILE_OK` flag turns "cannot open" into a negative return
> instead of an error, in which case return silently. Otherwise run
> `cmdloop(0)` and `popfile()`.

> [spec:dash:def:main.readcmdfile-fn]
> void readcmdfile(char *name)

> [spec:dash:sem:main.readcmdfile-fn]
> Read a file of shell function definitions: `setinputfile(name,
> INPUT_PUSH_FILE)` — without `INPUT_NOFILE_OK`, so a missing file is an
> error — then `cmdloop(0)`, then `popfile()`. The status is discarded.
