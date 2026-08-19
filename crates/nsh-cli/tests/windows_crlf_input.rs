#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn interactive_input_accepts_crlf_line_endings() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_nsh"))
        .arg("-i")
        .env("PS1", "")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start interactive shell");

    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(b"cd\r\ncd /\r\ncd C:/\r\nprintf ok\r\nexit\r\n")
        .expect("write CRLF shell input");

    let output = child.wait_with_output().expect("wait for shell");
    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    assert_eq!(output.stdout, b"ok");
    assert!(output.stderr.is_empty(), "stderr: {:?}", output.stderr);
}
