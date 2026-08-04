# src/mksignames.c

A build-time code generator, not part of the shell — imported from GNU
bash (and so GPL-2+, unlike the rest of the tree, which is BSD-licensed;
worth noting before a port copies from it). It emits `signames.c`,
defining `signal_names[]`, the table `trap` and `kill` use to convert
between signal numbers and names.

The names are stored *without* the `SIG` prefix (`"INT"`, `"TERM"`), and
index 0 is `"EXIT"` — the pseudo-signal for the exit trap, which is why
`decode_signal` takes a `minsig` argument to exclude it where a real
signal is required.

`NSIG` defaults to 64 where the system does not define it. Real-time
signals are handled specially and are disabled entirely under
`UNUSABLE_RT_SIGNALS`, which `configure` sets on systems (AIX 4.3) whose
`SIGRTMAX` is far beyond the table's bounds.

**Porting note.** What must survive is the *mapping* between numbers and
names, including the `EXIT` alias at 0 and the numeric fallback; the
generator itself need not be reproduced.

> [spec:dash:def:mksignames.initialize-signames-fn]
> void initialize_signames ()

> [spec:dash:sem:mksignames.initialize-signames-fn]
> Populate `signal_names[]`. Clear every entry from 1 upward, then set
> index 0 to `"EXIT"`.
>
> Assign the real-time signals first, so that ordinary names assigned
> later overwrite any collision — the source notes this deliberately
> lets `SIGABRT` overwrite `SIGLOST`. `SIGRTMIN` and `SIGRTMAX` get those
> names, and the signals between them are named `RTMIN+1`, `RTMIN+2`, …
> from the bottom and `RTMAX-1`, `RTMAX-2`, … from the top, meeting in
> the middle with an extra `RTMIN+x` when the count is odd — the naming
> ksh and Solaris `/usr/xpg4/bin/sh` use.
>
> Then assign every conventional signal that the platform defines, each
> under its own `#if defined`, so the table matches the build host:
> `HUP`, `INT`, `QUIT`, `ILL`, `TRAP`, `ABRT`, `BUS`, `FPE`, `KILL`,
> `USR1`, `SEGV`, `USR2`, `PIPE`, `ALRM`, `TERM`, `CHLD`, `CONT`, `STOP`,
> `TSTP`, `TTIN`, `TTOU`, `URG`, `XCPU`, `XFSZ`, `VTALRM`, `PROF`,
> `WINCH`, `INFO`, and various platform-specific ones.
>
> Finally, any index below `NSIG` still unnamed is given its decimal
> number as its name, allocated with `malloc(18)` — so every slot has a
> usable string and `kill -l` never prints a NULL.

> [spec:dash:def:mksignames.main-fn]
> int main(int argc, char **argv)

> [spec:dash:sem:mksignames.main-fn]
> Record `argv[0]` as `progname` for the generated file's header. With no
> argument the output is `signames.c`; with one it is that name; with
> more, print `"Usage: %s [output-file]"` and `exit(1)`. Open the file,
> reporting `"%s: %s: cannot open for writing"` and exiting on failure,
> then `initialize_signames()`, `write_signames(stream)`, and exit 0.

> [spec:dash:def:mksignames.write-signames-fn]
> void write_signames(FILE *stream)

> [spec:dash:sem:mksignames.write-signames-fn]
> Write the generated file: a "created automatically, do not edit"
> header naming the program, `#include <signal.h>`, then
> `const char *const signal_names[NSIG + 1] = { … };` with one quoted
> name per line for indices 0 through `LASTSIG`, terminated by a NULL
> entry. Note that nothing actually reads that NULL: both consumers
> bound their loops with `signo < NSIG` (`decode_signal` in `trap.c` and
> the `-l` listing in `killcmd`). It is vestigial, and a port may
> represent the table without it.
