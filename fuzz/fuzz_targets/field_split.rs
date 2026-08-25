//! Field splitting with the default `IFS`, checked as a self-property.
//!
//! The fuzzer's bytes arrive through `X`, then unquoted `$X` is split with
//! globbing disabled. Joining the resulting positional parameters with the
//! first byte of default `IFS` must equal the same normalization performed
//! directly over the input bytes.

#![no_main]

mod support;

use libfuzzer_sys::fuzz_target;
use support::{fingerprint, under_nsh};

const SCRIPT: &[u8] = b"set -f\nset -- $X\nprintf '%s' \"$*\"\n";

fn is_default_ifs(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n')
}

fn expected_joined_fields(value: &[u8]) -> Vec<u8> {
    let mut fields = Vec::new();
    let mut offset = 0;

    while offset < value.len() {
        while offset < value.len() && is_default_ifs(value[offset]) {
            offset += 1;
        }
        if offset == value.len() {
            break;
        }
        let start = offset;
        while offset < value.len() && !is_default_ifs(value[offset]) {
            offset += 1;
        }
        fields.push(&value[start..offset]);
    }

    let mut expected = Vec::new();
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            expected.push(b' ');
        }
        expected.extend_from_slice(field);
    }
    expected
}

fuzz_target!(|data: &[u8]| {
    if data.contains(&0) {
        return;
    }

    let expected = expected_joined_fields(data);
    let Some(outcome) = under_nsh(SCRIPT, vec![(b"X".to_vec(), data.to_vec())]) else {
        return;
    };

    assert!(
        !outcome.refused && outcome.status == 0 && outcome.stdout == expected,
        "field splitting property failed: input={:016x} status={} stdout={:016x} expected={:016x}",
        fingerprint(data),
        outcome.status,
        fingerprint(&outcome.stdout),
        fingerprint(&expected),
    );
});
