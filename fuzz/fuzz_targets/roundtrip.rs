//! Parse, print, then parse again without executing the input.
//!
//! The printer is an AST renderer, not a text-preserving formatter, so the
//! naive contract is a fixed point: canonicalising twice gives the same text.
//! That is too weak to be a property. Any output the printer can spell
//! consistently satisfies it, including output that means something else --
//! `echo "${a+"a}b"}"` printed as `echo "${a+a}b}"` for 107 artifacts without
//! the fixed point ever noticing, and `declare -f` was handing back function
//! bodies that ran differently.
//!
//! `[spec:nsh:req:idiom.printable-ast]` is the contract instead: printing a
//! tree and parsing the result has to give back the same tree, apart from the
//! source positions the render relocates. A parse error on the fuzzer's own
//! bytes is an ordinary answer; a printed program that will not parse, or
//! that parses as something else, is the finding.

#![no_main]

use bstr::BStr;
use libfuzzer_sys::fuzz_target;
use nsh::fuzzing::{Reversibility, printing_is_reversible};
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

    match printing_is_reversible(&mut shell, BStr::new(data)) {
        Reversibility::NotParsed | Reversibility::Reversible { .. } => {}
        Reversibility::NotReparsed { printed } => panic!(
            "printed program did not parse: input={:016x} printed={:016x}",
            fingerprint(data),
            fingerprint(&printed),
        ),
        Reversibility::Changed { printed } => panic!(
            "printed program parsed as a different program: input={:016x} printed={:016x}",
            fingerprint(data),
            fingerprint(&printed),
        ),
    }
    drop(shell.take_captured_stdout());
    drop(shell.take_captured_stderr());
});
