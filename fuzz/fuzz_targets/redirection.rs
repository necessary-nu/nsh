//! Here-document and descriptor-table behavior checked against GNU Bash.
//!
//! The generated source contains only fixed shell forms and a bounded decimal
//! descriptor number. No fuzzer byte string is embedded into the script.

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
