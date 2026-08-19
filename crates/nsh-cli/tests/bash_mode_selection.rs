use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::process::CommandExt as _;
use std::process::{Command, Output};

fn run_shell(arg0: &[u8], args: &[&[u8]]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nsh"));
    command.arg0(OsStr::from_bytes(arg0));
    for arg in args {
        command.arg(OsStr::from_bytes(arg));
    }
    command.env("LC_ALL", "C").output().expect("run nsh")
}

fn bash_lines(output: &Output) -> Vec<&[u8]> {
    assert!(
        output.status.success(),
        "shell failed: stdout={:?}, stderr={:?}",
        output.stdout,
        output.stderr
    );
    output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| line.starts_with(b"bash "))
        .collect()
}

// [spec:nsh:req:compat.bash.selection/test]
#[test]
fn invocation_and_options_select_bash_mode() {
    let ordinary = run_shell(b"nsh", &[b"-c", b"set -o"]);
    assert_eq!(bash_lines(&ordinary), [b"bash            off"]);

    let explicit = run_shell(b"nsh", &[b"-o", b"bash", b"-c", b"set -o"]);
    assert_eq!(bash_lines(&explicit), [b"bash            on"]);

    let disabled = run_shell(b"nsh", &[b"+o", b"bash", b"-c", b"set -o"]);
    assert_eq!(bash_lines(&disabled), [b"bash            off"]);
}

// [spec:nsh:req:compat.bash.selection/test]
#[test]
fn raw_invocation_basename_selects_mode() {
    for arg0 in [
        b"bash".as_slice(),
        b"/opt/shells/bash",
        b"/opt/shells/-bash",
    ] {
        let output = run_shell(arg0, &[b"-c", b"set -o"]);
        assert_eq!(bash_lines(&output), [b"bash            on"], "{arg0:?}");
    }

    let overridden = run_shell(b"bash", &[b"+o", b"bash", b"-c", b"set -o"]);
    assert_eq!(bash_lines(&overridden), [b"bash            off"]);
}

// [spec:nsh:req:compat.bash.selection/test]
#[test]
fn command_operand_does_not_select_mode() {
    let output = run_shell(b"nsh", &[b"-c", b"printf '%s\\n' \"$0\"; set -o", b"bash"]);
    assert!(output.stdout.starts_with(b"bash\n"));
    assert_eq!(bash_lines(&output), [b"bash            off"]);
}

// [spec:nsh:req:compat.bash.state-isolation/test]
#[test]
fn subshell_option_changes_remain_local() {
    let output = run_shell(
        b"nsh",
        &[b"-o", b"bash", b"-c", b"(set +o bash; set -o); set -o"],
    );
    assert_eq!(
        bash_lines(&output),
        [
            b"bash            off".as_slice(),
            b"bash            on".as_slice(),
        ]
    );
}
