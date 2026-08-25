//! Quoting must round-trip, for any byte string.
//!
//! A property target, not a crash target -- rung 2 of the ladder in
//! `PLAN.md`. `${x@Q}` and `printf %q` both claim to produce text that the
//! shell reads back as the original value, and that claim is checkable
//! against the shell itself with no second implementation:
//!
//!     eval "y=${X@Q}"   =>  y == X
//!     printf -v q %q X  ;  eval "z=$q"   =>  z == X
//!
//! It is also a *security* property rather than a cosmetic one. `@Q` and
//! `%q` are what a script reaches for when it has to put untrusted data
//! back into shell syntax -- building a command line, writing a file another
//! shell will source. A value that escapes its quoting there is command
//! injection, and it is the same shape as CVE-1999-0491: data becoming
//! syntax. `[dec:nsh:safety-trumps-compatibility]` names "an ambient
//! data-to-syntax path" as one of its four unsafeties.
//!
//! Which is why the check runs the round-trip through `eval` rather than
//! comparing strings in Rust: if the quoting leaks, `eval` is where it
//! would execute, and a corpus entry that manages it is exactly the
//! finding. The target does not print fuzzer bytes in panic messages; it
//! reports stable fingerprints instead.
//!
//! The verdict comes back as an exit status the shell computes, so the
//! comparison happens in the shell's own `[` and cannot be fooled by a
//! difference in how Rust would compare the bytes.

#![no_main]

use libfuzzer_sys::fuzz_target;
use nsh::{Shell, Streams};

/// 8 and 9 distinguish the two failures; 0 is agreement.
const SCRIPT: &[u8] = b"\
eval \"y=${X@Q}\"\n\
[ \"$y\" = \"$X\" ] || exit 9\n\
printf -v q '%q' \"$X\"\n\
eval \"z=$q\"\n\
[ \"$z\" = \"$X\" ] || exit 8\n\
exit 0\n";

/// A stable artifact identifier that does not render fuzzer bytes as text.
fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fuzz_target!(|data: &[u8]| {
    /* A shell variable cannot hold a NUL, so a value containing one is
     * not a value this property is about. */
    if data.contains(&0) {
        return;
    }

    let Ok(streams) = Streams::capture() else {
        return;
    };
    let Ok(mut shell) = Shell::builder()
        .streams(streams)
        // `@Q` and `printf -v` are both Bash's.
        .option(bstr::BStr::new(b"bash"), true)
        .env([(b"X".to_vec(), data.to_vec())])
        .build()
    else {
        return;
    };

    let Ok(status) = shell.run(SCRIPT) else {
        /* An `Err` is the shell refusing the script outright, which is a
         * different finding and one the `parse` target owns. */
        return;
    };
    let code: i32 = status.code().into();
    drop(shell.take_captured_stdout());
    drop(shell.take_captured_stderr());

    assert!(
        code == 0,
        "quoting did not round-trip via {}: input={:016x}",
        match code {
            9 => "${X@Q}",
            8 => "printf %q",
            _ => "script did not complete",
        },
        fingerprint(data),
    );
});
