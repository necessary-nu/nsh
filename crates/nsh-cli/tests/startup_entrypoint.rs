use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

const NSH: &str = env!("CARGO_BIN_EXE_nsh");
static NEXT_SCRIPT: AtomicUsize = AtomicUsize::new(0);

struct Script(PathBuf);

impl Script {
    fn new(contents: &[u8]) -> Self {
        let sequence = NEXT_SCRIPT.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("nsh-startup-{}-{sequence}.sh", std::process::id()));
        fs::write(&path, contents).expect("write script");
        Self(path)
    }
}

impl Drop for Script {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

// [spec:nsh:req:idiom.shell-entrypoint/test]
#[test]
fn command_sets_arguments() {
    let output = Command::new(NSH)
        .args([
            "-c",
            "printf '<%s>\\n' \"$0\" \"$#\" \"$1\"",
            "command-name",
            "one",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    assert_eq!(output.stdout, b"<command-name>\n<1>\n<one>\n");
}

// [spec:nsh:req:idiom.shell-entrypoint/test]
#[test]
fn script_sets_arguments() {
    let script = Script::new(b"printf '<%s>\\n' \"$0\" \"$#\" \"$1\"\n");
    let output = Command::new(NSH)
        .arg(&script.0)
        .arg("one")
        .output()
        .expect("run script");

    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    let mut expected = b"<".to_vec();
    expected.extend_from_slice(script.0.to_string_lossy().as_bytes());
    expected.extend_from_slice(b">\n<1>\n<one>\n");
    assert_eq!(output.stdout, expected);
}

// [spec:nsh:req:idiom.shell-entrypoint/test]
#[test]
fn script_preserves_interpreter_name() {
    let script = Script::new(b"printf output >&-\n");
    let output = Command::new(NSH)
        .arg(&script.0)
        .output()
        .expect("run script");

    assert!(!output.status.success());
    assert!(
        output.stderr.starts_with(NSH.as_bytes()),
        "stderr: {:?}",
        output.stderr
    );
    assert!(
        !output
            .stderr
            .starts_with(script.0.to_string_lossy().as_bytes()),
        "stderr: {:?}",
        output.stderr
    );
}

// [spec:nsh:req:idiom.shell-entrypoint/test]
#[test]
fn stdin_runs_command_loop() {
    let mut child = Command::new(NSH)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start shell");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(b"printf stdin; exit 7\n")
        .expect("write input");
    let output = child.wait_with_output().expect("wait for shell");

    assert_eq!(output.status.code(), Some(7));
    assert_eq!(output.stdout, b"stdin");
    assert!(output.stderr.is_empty(), "stderr: {:?}", output.stderr);
}

// [spec:nsh:req:idiom.shell-entrypoint/test]
#[test]
fn command_can_precede_stdin() {
    let mut child = Command::new(NSH)
        .args(["-sc", "printf 'command\\n'"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start shell");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(b"printf 'stdin\\n'\n")
        .expect("write input");
    let output = child.wait_with_output().expect("wait for shell");

    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    assert_eq!(output.stdout, b"command\nstdin\n");
    assert!(output.stderr.is_empty(), "stderr: {:?}", output.stderr);
}

// [spec:nsh:req:idiom.shell-entrypoint/test]
#[test]
fn invocation_errors_are_presented() {
    for (arguments, message) in [
        (&["-c"][..], b"-c requires an argument".as_slice()),
        (&["-Q"][..], b"Illegal option -Q".as_slice()),
    ] {
        let output = Command::new(NSH)
            .args(arguments)
            .output()
            .expect("run invalid invocation");
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        let mut expected = NSH.as_bytes().to_vec();
        expected.extend_from_slice(b": 0: ");
        expected.extend_from_slice(message);
        expected.push(b'\n');
        assert_eq!(output.stderr, expected);
    }
}
