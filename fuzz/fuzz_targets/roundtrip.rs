//! Parse and print, and require the bytes back, without executing anything.
//!
//! The property this replaced compared two trees: printing a program and
//! parsing the result had to give back the same tree. That is satisfiable
//! by output which is not what was written, and it was satisfied by output
//! that ran differently -- `declare -f` handed back function bodies whose
//! meaning had changed, and a fixed point never noticed.
//!
//! `[spec:nsh:req:idiom.printable-ast+2]` is a byte equality instead. A
//! node carries the run it was read from, so printing is emission and the
//! only way to fail is to emit bytes that were not there. Nothing derived
//! from the tree takes part in the comparison: the right-hand side is the
//! fuzzer's own input.
//!
//! An artifact reduces to a first differing offset rather than to a pair
//! of fingerprints, which is the reason this reports one.
//!
//! Rejecting the input is an ordinary answer to arbitrary bytes. So is an
//! alias expansion, which replaces text before the parser sees it and is
//! carved out of the rule for that reason.

#![no_main]

use bstr::BStr;
use libfuzzer_sys::fuzz_target;
use nsh::fuzzing::{RoundTrip, round_trips_byte_exactly};
use nsh::{Shell, Streams};

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

    match round_trips_byte_exactly(&mut shell, BStr::new(data)) {
        RoundTrip::NotParsed | RoundTrip::Aliased | RoundTrip::Exact => {}
        RoundTrip::Differed { at, printed } => panic!(
            "printed program is not the source it came from at byte {at}:\n\
             read    {:?}\n\
             printed {:?}",
            BStr::new(&data[at.saturating_sub(24).min(data.len())..data.len().min(at + 24)]),
            BStr::new(
                &printed[at.saturating_sub(24).min(printed.len())..printed.len().min(at + 24)]
            ),
        ),
        RoundTrip::Misplaced { outer, inner } => panic!(
            "a node's run is not inside the run above it:\n\
             outer {:?}\n\
             inner {:?}",
            BStr::new(&outer[..outer.len().min(96)]),
            BStr::new(&inner[..inner.len().min(96)]),
        ),
    }
    drop(shell.take_captured_stdout());
    drop(shell.take_captured_stderr());
});
