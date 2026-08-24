//! Exact witnesses for the signal and EXIT trap status profile.
//!
//! Five of these pinned the Smoosh corpus's answer, which contradicts
//! POSIX: "The value of `"$?"` after the trap action completes shall be
//! the value it had before the trap action was executed"
//! (`[spec:posix:req:builtin.trap.action-overrides-and-exit-status]`).
//! They now pin the rule, and every expectation below was taken from GNU
//! bash 5.2.37 run against the same script. dash agrees on three of the
//! five and has its own bugs in the other two -- the Smoosh cases they
//! came from cite the dash threads. See docs/divergences.md.

use nsh::streams::Streams;

fn run(script: &str) -> (Vec<u8>, Vec<u8>, i32) {
    let command = script.as_bytes().to_vec();
    let (stdout_read, stdout_write) = nsh_platform::pipe().expect("create stdout pipe");
    let (stderr_read, stderr_write) = nsh_platform::pipe().expect("create stderr pipe");

    let status = nsh_platform::run_in_child(move || {
        let supplied = Streams::from_fds(std::io::stdin(), &stdout_write, &stderr_write)
            .expect("duplicate test streams");
        let mut shell = nsh::Shell::builder()
            .argument_zero(bstr::BStr::new(b"smoosh"))
            .inherit_env()
            .streams(supplied)
            .host(nsh::ProcessHost)
            .build()
            .expect("build process shell");
        let status = shell.run_to_completion(nsh::Startup::command(command));
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

/// The action's own failure is not the shell's: `$?` was 0 when the
/// action started, so 0 is what the shell leaves with.
// [spec:posix:req:builtin.trap.action-overrides-and-exit-status/test]
#[test]
fn an_action_failure_is_not_the_status() {
    let (stdout, _, status) = run(r#"trap "(false) && echo BUG" EXIT"#);

    assert!(stdout.is_empty());
    assert_eq!(status, 0);
}

/// And the converse: `false` set 1 before the action ran, so 1 survives
/// an action that succeeds.
// [spec:posix:req:builtin.trap.action-overrides-and-exit-status/test]
#[test]
fn an_action_keeps_the_pre_trap_status() {
    let (stdout, _, status) = run("trap '(:; exit) && echo WEIRD' EXIT; false");

    assert_eq!(stdout, b"WEIRD\n");
    assert_eq!(status, 1);
}

/// A signal action and an EXIT action in one script, each restoring the
/// status that was current before it ran.
// [spec:nsh:req:compat.smoosh.trap-status/test]
// [spec:posix:req:builtin.trap.action-overrides-and-exit-status/test]
#[test]
fn nested_signal_and_exit_statuses() {
    let script = "trap 'set -o bad@option' INT; kill -s INT $$ && echo HUH\n\
                  trap '(:; exit) && echo WEIRD' EXIT; false";
    let (stdout, _, status) = run(script);

    assert_eq!(stdout, b"HUH\nWEIRD\n");
    assert_eq!(status, 1);
}

/// The one that matters in practice: `set -e` with a `cleanup` EXIT
/// action must still report the failure that ended the script.
// [spec:posix:req:builtin.trap.action-overrides-and-exit-status/test]
#[test]
fn a_successful_action_keeps_the_failure() {
    let (stdout, _, status) = run("trap '(true) || echo bug' EXIT; false");
    assert!(stdout.is_empty());
    assert_eq!(status, 1);

    let (printed, _, errexit) = run("set -e; trap 'echo cleanup' EXIT; false; echo NOTREACHED");
    assert_eq!(printed, b"cleanup\n");
    assert_eq!(errexit, 1, "a defensive script must not report success");
}

/// `return` from a subshell function sets the status the EXIT action
/// then runs under, and does not lose it.
// [spec:nsh:req:compat.smoosh.trap-status/test]
// [spec:posix:req:builtin.trap.action-overrides-and-exit-status/test]
#[test]
fn subshell_return_runs_exit_action() {
    let script = "f() ( trap 'echo FOO' EXIT; return 5; echo BAR )\nf";
    let (stdout, _, status) = run(script);

    assert_eq!(stdout, b"FOO\n");
    assert_eq!(status, 5);
}
