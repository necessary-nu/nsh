//! Whose children a `Shell` may reap.
//!
//! A shell *binary* owns every child in its process, so `waitpid(-1)` is
//! the right call for it and the only one dash ever needed. An embedded
//! shell does not own them: [dec:nsh:shell-as-library] says a `Shell` is a
//! value in somebody else's process, and there `waitpid(-1)` is the
//! library reaching into its host's process table. Reaping is destructive
//! -- the status is gone once taken -- so the host's own `wait()` answers
//! `ECHILD` for a child it is still holding, or, if it was already blocked
//! in one, is woken with the same error and no status.
//!
//! The test suite was the first host to notice, and it looked like flake
//! rather than like this. `an_unpinned_bash_is_refused` runs
//! `Command::output()` in a process shared with hundreds of other tests,
//! several of which drive a `Shell`; its `ECHILD` and the
//! `errors_are_values` hang are one theft seen from two sides. Neither
//! reproduces when its own test is run alone, because the theft needs
//! another shell to be running to do the stealing.

use nsh::{Shell, Streams};
use std::process::{Command, Stdio};

/// The host forks a child, then drives a shell that forks one of its own
/// and waits for it. The shell's wait must not take the host's child.
// [spec:nsh:req:embedding-safety.host-children-are-not-reaped/test]
#[test]
fn a_shell_leaves_the_host_its_children() {
    let mut theirs = Command::new("/bin/sleep")
        .arg("0.2")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the host forks a child of its own");

    let mut shell = Shell::builder()
        .streams(Streams::capture().expect("create capture streams"))
        .build()
        .expect("build shell");
    /* Outlives the host's child, so the shell is blocked in its own wait
     * at the moment the host's child becomes reapable. */
    let status = shell.run(b"/bin/sleep 1\n".as_slice());
    drop(shell);

    assert_eq!(
        status.map(|status| status.code()).unwrap_or(255),
        0,
        "the shell did not run a child of its own"
    );
    let theirs = theirs
        .wait()
        .expect("the host's child is still the host's to wait for");
    assert!(
        theirs.success(),
        "the host's child reported {theirs} rather than its own exit"
    );
}
