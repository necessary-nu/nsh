//! Literal port of `src/signames.c`.
//!
//! `signames.c` is *generated* at build time by `src/mksignames.c` (see
//! `crate::gen::mksignames`, `docs/spec/port/src/mksignames.md`).  It is not
//! checked-in C source, so nothing here carries `[spec:dash:…]` annotations;
//! only the generator does.
//!
//! The table below is the real generator's output on Linux/glibc, where
//! `NSIG` is 65, `SIGRTMIN` is 34 and `SIGRTMAX` is 64.  As in the C:
//!
//! * index 0 is `"EXIT"`, the pseudo-signal for the exit trap (which is why
//!   `decode_signal` takes a `minsig` argument);
//! * names carry no `SIG` prefix;
//! * every slot below `NSIG` that no `#if defined (SIG…)` claimed gets its
//!   decimal number as its name (here 16 — `SIGSTKFLT` — and 32/33, which
//!   glibc reserves), so `kill -l` never prints a NULL;
//! * aliases resolve the way the `#if` order in `mksignames.c` dictates:
//!   6 is `ABRT` (not `IOT`), 17 is `CHLD` (not `CLD`), 29 is `IO` (not
//!   `POLL`).
//!
//! Entries are `&CStr` because every consumer (`trap.c`'s `strcasecmp`,
//! `jobs.c`'s `outfmt`) wants a NUL-terminated string.  The C array has one
//! more element than there are names — a trailing NULL sentinel — which is
//! represented here by the empty string at index `NSIG`; nothing in the
//! shell reads it, because `decode_signal` and `showjobs` both bound their
//! loops with `signo < NSIG`.

use core::ffi::CStr;

/// `<signal.h>`: glibc's `NSIG`.  `mksignames.c` falls back to 64 where the
/// system does not define it.
pub const NSIG: usize = 65;

/// `#define LASTSIG NSIG-1`
pub const LASTSIG: usize = NSIG - 1;

/// A translation list so we can be polite to our users.
pub static signal_names: [&CStr; NSIG + 1] = [
    c"EXIT",
    c"HUP",
    c"INT",
    c"QUIT",
    c"ILL",
    c"TRAP",
    c"ABRT",
    c"BUS",
    c"FPE",
    c"KILL",
    c"USR1",
    c"SEGV",
    c"USR2",
    c"PIPE",
    c"ALRM",
    c"TERM",
    c"16",
    c"CHLD",
    c"CONT",
    c"STOP",
    c"TSTP",
    c"TTIN",
    c"TTOU",
    c"URG",
    c"XCPU",
    c"XFSZ",
    c"VTALRM",
    c"PROF",
    c"WINCH",
    c"IO",
    c"PWR",
    c"SYS",
    c"32",
    c"33",
    c"RTMIN",
    c"RTMIN+1",
    c"RTMIN+2",
    c"RTMIN+3",
    c"RTMIN+4",
    c"RTMIN+5",
    c"RTMIN+6",
    c"RTMIN+7",
    c"RTMIN+8",
    c"RTMIN+9",
    c"RTMIN+10",
    c"RTMIN+11",
    c"RTMIN+12",
    c"RTMIN+13",
    c"RTMIN+14",
    c"RTMIN+15",
    c"RTMAX-14",
    c"RTMAX-13",
    c"RTMAX-12",
    c"RTMAX-11",
    c"RTMAX-10",
    c"RTMAX-9",
    c"RTMAX-8",
    c"RTMAX-7",
    c"RTMAX-6",
    c"RTMAX-5",
    c"RTMAX-4",
    c"RTMAX-3",
    c"RTMAX-2",
    c"RTMAX-1",
    c"RTMAX",
    c"", // (char *)0x0
];
