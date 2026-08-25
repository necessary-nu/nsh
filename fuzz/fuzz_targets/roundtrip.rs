//! Parse, print, then parse again without executing the input.
//!
//! The printer is an AST renderer, not a text-preserving formatter. Its useful
//! round-trip contract is therefore a fixed point: canonicalising a parsed
//! program twice must produce the same program text. A parse error is an
//! ordinary answer to fuzzer bytes; a panic, hang, or changing canonical form
//! is the finding.

#![no_main]

use bstr::BStr;
use libfuzzer_sys::fuzz_target;
use nsh::{Shell, Streams};

fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fuzz_target!(|data: &[u8]| {
    let Ok(streams) = Streams::capture() else {
        return;
    };
    let Ok(mut shell) = Shell::builder()
        .streams(streams)
        .option(BStr::new(b"bash"), true)
        .build()
    else {
        return;
    };

    let Ok(once) = nsh::fuzzing::canonical_source(&mut shell, BStr::new(data)) else {
        return;
    };
    let Ok(twice) = nsh::fuzzing::canonical_source(&mut shell, BStr::new(&once)) else {
        panic!(
            "canonical source did not reparse: input={:016x} once={:016x}",
            fingerprint(data),
            fingerprint(&once),
        );
    };
    assert!(
        once == twice,
        "canonical source changed after reparse: input={:016x} once={:016x} twice={:016x}",
        fingerprint(data),
        fingerprint(&once),
        fingerprint(&twice),
    );
    drop(shell.take_captured_stdout());
    drop(shell.take_captured_stderr());
});
