//! Here-document and descriptor-table behavior checked against GNU Bash.
//!
//! The generated source contains only fixed shell forms and a bounded decimal
//! descriptor number. No fuzzer byte string is embedded into the script.
//!
//! # Why this target claims no rule
//!
//! Here-documents and the descriptor table are POSIX ground, and the
//! three `nsh` rules that touch descriptors --
//! `idiom.descriptor-materialization`, `idiom.resource-scopes` and
//! `idiom.immutable-ast`, which is where a finalized here-document body
//! is required -- are each about how the implementation is shaped rather
//! than about what a shell prints. A target that reads a byte stream and
//! an exit status cannot see any of the three, and would go on passing if
//! all three were violated by an implementation that behaved.
//!
//! What it does establish is that this shell answers a write to a closed
//! descriptor, and a here-document body, the way the pinned Bash does.
//! No rule in a scope that reaches this directory claims that.

#![no_main]

mod support;

use libfuzzer_sys::fuzz_target;

fn script(data: &[u8]) -> String {
    let selector = data.first().copied().unwrap_or(0) % 5;
    let fd = 3 + (data.get(1).copied().unwrap_or(0) % 48);

    match selector {
        0 => {
            format!("exec {fd}>&1\nprintf '[x]\\n' >&{fd}\nexec {fd}>&-\nprintf '[y]\\n'\n")
        }
        1 => String::from("read line <<'EOF'\nalpha beta\nEOF\nprintf '[%s]\\n' \"$line\"\n"),
        2 => String::from("read line <<-EOF\n\talpha\nEOF\nprintf '[%s]\\n' \"$line\"\n"),
        3 => format!("exec {fd}>&1\n: <<EOF >&{fd}\nignored\nEOF\nprintf '[done]\\n'\n"),
        _ => format!(
            "exec {fd}>&1\nexec {fd}>&-\nprintf '[closed]\\n' >&{fd}\nprintf '[after]\\n'\n"
        ),
    }
}

fuzz_target!(|data: &[u8]| {
    let script = script(data);
    support::assert_matches_bash("redirection", data, script.as_bytes(), Vec::new());
});
