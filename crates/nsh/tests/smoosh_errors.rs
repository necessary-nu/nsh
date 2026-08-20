//! Exact witnesses for the Smoosh error, status, and diagnostic profile.

use nsh::streams::Streams;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_CASE: AtomicU64 = AtomicU64::new(0);

fn run(script: &str, interactive: bool, file_operand: bool) -> (Vec<u8>, Vec<u8>, i32) {
    let directory = std::env::temp_dir().join(format!(
        "nsh-smoosh-error-{}-{}",
        std::process::id(),
        NEXT_CASE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).expect("create isolated case directory");

    let startup = if file_operand {
        let script_path = directory.join("case.sh");
        std::fs::write(&script_path, script).expect("write case script");
        nsh::Startup::script(script_path.as_os_str().as_encoded_bytes().to_vec())
    } else {
        nsh::Startup::command(script.as_bytes().to_vec())
    };

    let (stdout_read, stdout_write) = nsh_platform::pipe().expect("create stdout pipe");
    let (stderr_read, stderr_write) = nsh_platform::pipe().expect("create stderr pipe");

    let status = nsh_platform::run_in_child(move || {
        std::env::set_current_dir(directory).expect("enter isolated case directory");
        let supplied = Streams::from_fds(std::io::stdin(), &stdout_write, &stderr_write)
            .expect("duplicate test streams");
        let mut builder = nsh::Shell::builder()
            .argument_zero(bstr::BStr::new(b"smoosh"))
            .inherit_env()
            .streams(supplied)
            .host(nsh::ProcessHost);
        if interactive {
            builder = builder
                .shell_option(nsh::ShellOption::Interactive, true)
                .shell_option(nsh::ShellOption::Monitor, true);
        }
        let mut shell = builder.build().expect("build process shell");
        let status = shell.run_to_completion(startup);
        nsh_platform::exit_immediately(status.code().into());
    })
    .expect("run shell child");

    let stdout = nsh_platform::read_to_end(&stdout_read).expect("read stdout");
    let stderr = nsh_platform::read_to_end(&stderr_read).expect("read stderr");
    (stdout, stderr, status)
}

// [spec:nsh:req:compat.smoosh.error-contracts/test]
#[test]
fn command_demotes_readonly() {
    let (stdout, stderr, status) = run(
        "command readonly x=foo\ncommand readonly x=bar\necho ?=$?",
        false,
        false,
    );

    assert_eq!(stdout, b"?=1\n");
    assert_eq!(stderr, b"readonly: x: is read only\n");
    assert_eq!(status, 0);
}

// [spec:nsh:req:compat.smoosh.error-contracts/test]
#[test]
fn missing_dot_is_fatal() {
    let (stdout, stderr, status) = run(". ./nonesuch", false, false);

    assert!(stdout.is_empty());
    assert_eq!(stderr, b".: ./nonesuch: not found\n");
    assert_eq!(status, 1);
}

// [spec:nsh:req:compat.smoosh.error-contracts/test]
#[test]
fn special_redirections_use_one() {
    let (_, _, special) = run(": 2>&9\necho unreachable", false, false);
    let (_, _, no_command) = run("exec 9&<-", false, false);

    assert_eq!(special, 1);
    assert_eq!(no_command, 1);
}

// [spec:nsh:req:compat.smoosh.error-contracts/test]
#[test]
fn readonly_assignment_is_fatal() {
    let (stdout, _, status) = run("readonly a=b\nexport a=c\necho unreachable", false, false);

    assert!(stdout.is_empty());
    assert_eq!(status, 1);
}

// [spec:nsh:req:compat.smoosh.error-contracts/test]
#[test]
fn unset_readonly_is_one() {
    let (stdout, stderr, status) = run(
        "readonly x=foo\ny=bar\nunset y\necho ${y-unset}\necho ${x-error}\nunset y\necho ${y-unset}\nunset x",
        false,
        false,
    );

    assert_eq!(stdout, b"unset\nfoo\nunset\n");
    assert_eq!(stderr, b"unset: x is read-only\n");
    assert_eq!(status, 1);
}

// [spec:nsh:req:compat.smoosh.error-contracts/test]
#[test]
fn expansion_modes_diverge() {
    let (stdout, stderr, status) = run("unset x; echo ${x?z}; echo unreachable", false, false);
    assert!(stdout.is_empty());
    assert_eq!(stderr, b"x: z\n");
    assert_eq!(status, 1);

    let (stdout, _, status) = run("echo ${x?alas, poor yorick}; echo hello; exit", true, false);
    assert_eq!(stdout, b"hello\n");
    assert_eq!(status, 0);
}

// [spec:nsh:req:compat.smoosh.error-contracts/test]
#[test]
fn times_write_failure_is_two() {
    let script = "exec 3>&1\n(\ntrap \"\" PIPE\nsleep 1\ncommand times\necho ?=$? >&3\n) | true";
    let (stdout, stderr, status) = run(script, false, true);

    assert_eq!(stdout, b"?=2\n");
    assert_eq!(stderr, b"smoosh: times: I/O error\n");
    assert_eq!(status, 0);
}

// [spec:nsh:req:compat.smoosh.error-contracts/test]
#[test]
fn closed_descriptor_stays_closed() {
    let script = "{ exec 8</dev/null; } 8<&-; : <&8 && echo 'oops, still open'";
    let (stdout, _, status) = run(script, false, false);

    assert!(stdout.is_empty());
    assert_eq!(status, 1);
}
