//! Literal port of `src/signames.c`.
//!
//! `signames.c` is *generated* at build time by `src/mksignames.c`
//! (`docs/spec/port/src/mksignames.md`).  It is not checked-in C source, so
//! nothing here carries `[spec:dash:…]` annotations.
//!
//! The table below is the real generator's output on Linux/glibc, where
//! `NSIG` is 65, `SIGRTMIN` is 34 and `SIGRTMAX` is 64; the test at the foot
//! of this file asserts it against the `signames.c` the reference build
//! generated.  As in the C:
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

// ---------------------------------------------------------------------
// Provenance: the table against the C generator's own output.
//
// Same shape as `crate::syntax`: the reference build runs the real
// `mksignames` and leaves `signames.c` beside the binary the differential
// harness compares against, so the module's claim is checked on the table
// the shell actually indexes.
// ---------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    /// The C generator's output from the reference build, if built.
    fn reference(name: &str) -> Option<String> {
        let root = std::env::var("DASH_ROOT")
            .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../..").to_string());
        std::fs::read_to_string(format!("{root}/tests/.build/ref/src/{name}")).ok()
    }

    #[test]
    fn the_table_is_the_c_generators_output() {
        let text = match reference("signames.c") {
            Some(t) => t,
            None => {
                eprintln!(
                    "note: tests/.build/ref absent, skipped the signames.c comparison \
                     (run tests/build-reference.sh for the stronger assertion)"
                );
                return;
            }
        };
        let head = "const char *const signal_names[NSIG + 1] = {";
        let start = text.find(head).expect("declaration not found") + head.len();
        let len = text[start..].find("(char *)0x0").expect("NULL sentinel not found");
        let theirs: Vec<&str> = text[start..start + len]
            .lines()
            .filter_map(|l| l.trim().strip_prefix('"'))
            .filter_map(|l| l.strip_suffix("\","))
            .collect();

        // The C writes one name per signal up to LASTSIG and then the NULL
        // sentinel that this port spells as the empty string at NSIG.
        assert_eq!(theirs.len(), LASTSIG + 1, "entry count");
        for (i, name) in theirs.iter().enumerate() {
            assert_eq!(signal_names[i].to_bytes(), name.as_bytes(), "signal_names[{i}]");
        }
        assert_eq!(signal_names[NSIG].to_bytes(), b"");
    }
}
