# src/bltin/times.c

> [spec:dash:def:times.timescmd-fn]
> int timescmd(int argc, char *argv[])

> [spec:dash:sem:times.timescmd-fn]
> The `times` builtin: report accumulated CPU time for the shell and for
> its children. Get the clock tick rate with `sysconf(_SC_CLK_TCK)` and
> the four counters with `times(&buf)`: `tms_utime`, `tms_stime`
> (this process) and `tms_cutime`, `tms_cstime` (waited-for children).
>
> Convert each to seconds by dividing by the tick rate, then split into
> whole minutes and a fractional-seconds remainder — the minutes come
> from truncating `seconds / 60` to `int`, and the remainder from
> subtracting `minutes * 60.0`.
>
> Print two lines with `printf("%dm%fs %dm%fs\n%dm%fs %dm%fs\n", …)`:
> the shell's user and system time on the first, the children's on the
> second. `%f` gives six decimal places. Return 0. Arguments are ignored
> — not even options are parsed.
>
> Note this file is built as part of the `bltin` sub-library, so under
> `USE_GLIBC_STDIO` it uses real stdio and otherwise the shell's own
> `printf` shim from `bltin.h`.
