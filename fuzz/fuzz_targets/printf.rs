//! Bash-mode `printf` format behavior checked against GNU Bash.
//!
//! Format strings are fixed. The fuzzer supplies string bytes through `V`
//! and bounded numeric text through `N`.

#![no_main]

mod support;

use libfuzzer_sys::fuzz_target;

const SCRIPTS: [&[u8]; 9] = [
    b"printf '[%s]\\n' \"$V\"\n",
    b"printf '[%q]\\n' \"$V\"\n",
    b"printf '[%b]\\n' \"$V\"\n",
    b"printf '[%5.3s]\\n' \"$V\"\n",
    b"printf '[%-6s]\\n' \"$V\"\n",
    b"printf '[%c]\\n' \"$V\"\n",
    b"printf '[%d]\\n' \"$N\"\n",
    b"printf '[%#x]\\n' \"$N\"\n",
    b"printf '[%08u]\\n' \"$N\"\n",
];

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }
    let limit = data.len().min(257);
    let value = &data[1..limit];
    if value.contains(&0) {
        return;
    }

    let script = SCRIPTS[usize::from(data[0]) % SCRIPTS.len()];
    let number = i16::from_ne_bytes([data[0], data.get(1).copied().unwrap_or(0)]);
    let env = vec![
        (b"V".to_vec(), value.to_vec()),
        (b"N".to_vec(), number.to_string().into_bytes()),
    ];

    support::assert_matches_bash("printf", data, script, env);
});
