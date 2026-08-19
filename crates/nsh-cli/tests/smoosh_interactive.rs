//! Exact witnesses for the Smoosh interactive prompt and job-ID profile.

use std::io::Write;
use std::process::{Command, Output, Stdio};

fn interactive(input: &[u8], ps1: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nsh"));
    command
        .arg("-i")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    match ps1 {
        Some(value) => {
            command.env("PS1", value);
        }
        None => {
            command.env_remove("PS1");
        }
    }

    let mut child = command.spawn().expect("start interactive shell");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(input)
        .expect("write shell input");
    child.wait_with_output().expect("wait for shell")
}

fn script(source: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nsh"))
        .args(["-c", source])
        .output()
        .expect("run shell script")
}

// [spec:nsh:req:compat.smoosh.interactive-job-prompt/test]
#[test]
fn non_tty_default_prompt_is_exact() {
    let output = interactive(b"exit\n", None);

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"$ ");
}

// [spec:nsh:req:compat.smoosh.interactive-job-prompt/test]
#[test]
fn non_tty_prompt_override_is_exact() {
    let default = interactive(b"echo hi\necho bye\n", None);
    let override_ = interactive(b"echo hi\necho bye\n", Some("PS1$ "));

    assert!(default.status.success());
    assert!(override_.status.success());
    assert_eq!(
        [default.stdout, override_.stdout].concat(),
        b"hi\nbye\nhi\nbye\n"
    );
    assert_eq!(
        [default.stderr, override_.stderr].concat(),
        b"$ $ $ PS1$ PS1$ PS1$ "
    );
}

// [spec:nsh:req:compat.smoosh.interactive-job-prompt/test]
#[test]
fn job_ids_follow_monitor_mode() {
    let source = "sleep 2 & p1=$!\n\
                  sleep 2 & p2=$!\n\
                  kill %1 %2 >/dev/null 2>&1 && exit 3\n\
                  kill $p1 $p2\n\
                  wait\n\
                  set -m\n\
                  sleep 2 &\n\
                  sleep 2 &\n\
                  kill %1 %2 || exit 4\n\
                  wait\n\
                  exit 0\n";
    let output = script(source);

    assert_eq!(output.status.code(), Some(0), "stderr: {:?}", output.stderr);
}
