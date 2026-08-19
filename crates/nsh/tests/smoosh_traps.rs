//! Exact witnesses for the Smoosh signal and EXIT trap status profile.

use nsh::streams::Streams;

fn run(script: &str) -> (Vec<u8>, Vec<u8>, i32) {
    let argv = vec![
        b"smoosh".to_vec(),
        b"-c".to_vec(),
        script.as_bytes().to_vec(),
    ];
    let (stdout_read, stdout_write) = nsh_platform::pipe().expect("create stdout pipe");
    let (stderr_read, stderr_write) = nsh_platform::pipe().expect("create stderr pipe");

    let status = nsh_platform::run_in_child(move || {
        let supplied = Streams::from_fds(std::io::stdin(), &stdout_write, &stderr_write)
            .expect("duplicate test streams");
        let status = nsh::shellmain::main_fn(argv, supplied);
        nsh_platform::exit_immediately(status.code().into());
    })
    .expect("run shell child");

    let stdout = nsh_platform::read_to_end(&stdout_read).expect("read stdout");
    let stderr = nsh_platform::read_to_end(&stderr_read).expect("read stderr");
    (stdout, stderr, status)
}

// [spec:nsh:req:compat.smoosh.trap-status/test]
#[test]
fn chained_trap_uses_current_status() {
    let (stdout, _, status) = run("trap exit INT\ntrap 'true; kill -s INT $$' EXIT\nfalse");

    assert!(stdout.is_empty());
    assert_eq!(status, 0);
}

// [spec:nsh:req:compat.smoosh.trap-status/test]
#[test]
fn signal_failure_restores_status() {
    let (stdout, _, status) = run("trap 'set -o bad@option' INT\nkill -s INT $$");

    assert!(stdout.is_empty());
    assert_eq!(status, 0);
}

// [spec:nsh:req:compat.smoosh.trap-status/test]
#[test]
fn exit_action_failure_becomes_status() {
    let (stdout, _, status) = run(r#"trap "(false) && echo BUG" EXIT"#);

    assert!(stdout.is_empty());
    assert_eq!(status, 1);
}

// [spec:nsh:req:compat.smoosh.trap-status/test]
#[test]
fn action_subshell_exit_uses_current() {
    let (stdout, _, status) = run("trap '(:; exit) && echo WEIRD' EXIT; false");

    assert_eq!(stdout, b"WEIRD\n");
    assert_eq!(status, 0);
}

// [spec:nsh:req:compat.smoosh.trap-status/test]
#[test]
fn nested_signal_and_exit_statuses() {
    let script = "trap 'set -o bad@option' INT; kill -s INT $$ && echo HUH\n\
                  trap '(:; exit) && echo WEIRD' EXIT; false";
    let (stdout, _, status) = run(script);

    assert_eq!(stdout, b"HUH\nWEIRD\n");
    assert_eq!(status, 0);
}

// [spec:nsh:req:compat.smoosh.trap-status/test]
#[test]
fn successful_action_replaces_failure() {
    let (stdout, _, status) = run("trap '(true) || echo bug' EXIT; false");

    assert!(stdout.is_empty());
    assert_eq!(status, 0);
}

// [spec:nsh:req:compat.smoosh.trap-status/test]
#[test]
fn subshell_return_runs_exit_action() {
    let script = "f() ( trap 'echo FOO' EXIT; return 5; echo BAR )\nf";
    let (stdout, _, status) = run(script);

    assert_eq!(stdout, b"FOO\n");
    assert_eq!(status, 0);
}
