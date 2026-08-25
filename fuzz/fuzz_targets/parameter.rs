//! Parameter expansion operators checked against GNU Bash.
//!
//! Fuzzer bytes are carried in variables. The script text is selected from a
//! fixed set of parameter-expansion forms so the target explores expansion
//! behavior without letting input bytes become shell syntax.

#![no_main]

mod support;

use libfuzzer_sys::fuzz_target;

const SCRIPTS: [&[u8]; 13] = [
    b"unset X\nprintf '[%s]\\n' \"${X-fallback}\"\n",
    b"printf '[%s]\\n' \"${X:-fallback}\"\n",
    b"printf '[%s]\\n' \"${X:+alt}\"\n",
    b"printf '[%s]\\n' \"${#X}\"\n",
    b"printf '[%s]\\n' \"${X#$P}\"\n",
    b"printf '[%s]\\n' \"${X##$P}\"\n",
    b"printf '[%s]\\n' \"${X%$P}\"\n",
    b"printf '[%s]\\n' \"${X%%$P}\"\n",
    b"printf '[%s]\\n' \"${X/$P/$R}\"\n",
    b"printf '[%s]\\n' \"${X//$P/$R}\"\n",
    b"printf '[%s]\\n' \"${X:O:L}\"\n",
    b"printf '[%s]\\n' \"${X@Q}\"\n",
    b"printf '[%s]\\n' \"${X^^}\"\n",
];

fn split_three(data: &[u8]) -> (&[u8], &[u8], &[u8]) {
    let first = data.len() / 3;
    let second = first + ((data.len() - first) / 2);
    (&data[..first], &data[first..second], &data[second..])
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 3 {
        return;
    }
    let limit = data.len().min(195);
    let rest = &data[3..limit];
    if rest.contains(&0) {
        return;
    }

    let script = SCRIPTS[usize::from(data[0]) % SCRIPTS.len()];
    let offset = i8::from_ne_bytes([data[1]]) % 16;
    let length = data[2] % 16;
    let (x, p, r) = split_three(rest);
    let env = vec![
        (b"X".to_vec(), x.to_vec()),
        (b"P".to_vec(), p.to_vec()),
        (b"R".to_vec(), r.to_vec()),
        (b"O".to_vec(), offset.to_string().into_bytes()),
        (b"L".to_vec(), length.to_string().into_bytes()),
    ];

    support::assert_matches_bash("parameter", data, script, env);
});
