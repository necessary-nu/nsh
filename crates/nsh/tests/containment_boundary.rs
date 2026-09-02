//! The boundary `cargo test` runs this workspace's tests behind.
//!
//! Almost every integration test here executes shell code: 51 of the 54 test
//! files in `crates/nsh`, `crates/nsh-cli` and `crates/nsh-survey` build a
//! shell, fork one, or exec one. Two of them write a `case.sh` and run it.
//! `enforce-survey-test-containment` named "workspace-test" among the commands
//! that must do that inside a fail-closed PID namespace, and until 2026-09-02
//! nothing enforced it: the survey runner enforced containment for itself and
//! `scripts/sandboxed` enforced it for whatever was typed behind it, while
//! `cargo test --workspace` ran the same shell programs with the session's own
//! uid, PID namespace and controlling terminal.
//!
//! `.cargo/config.toml` now names `scripts/sandboxed --cargo-runner` as the
//! target runner, so cargo puts every test binary behind the boundary itself.
//! This file is what notices when that stops being true — a deleted config
//! entry, a runner cargo silently ignored, a sandbox that came up without the
//! namespace it promised.
//!
//! What it cannot check is the one thing the wrapper checks and this cannot:
//! whether the PID namespace is *the session's*. A process has no way to name
//! a namespace it is not in, so `scripts/sandboxed` records the host's before
//! it crosses and compares from inside. From here the evidence is what the
//! boundary looks like rather than what it is not — the host's process table
//! is gone, the session is private, the terminal is gone, the root is
//! read-only, the process count is bounded — which is the same signature that
//! wrapper's canary asserts, minus the comparison only it can make.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The wrapper's own canary calls more than sixteen visible processes a
/// failed containment, and this is the same question asked from inside.
const VISIBLE_PROCESS_LIMIT: usize = 16;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The fields of `/proc/self/stat` from `state` onward.
///
/// Field 2 is the command in parentheses and may itself contain spaces and
/// `)`, so the fields after it begin at the last `") "` rather than at the
/// third whitespace-separated word.
fn own_stat_fields() -> Vec<String> {
    let stat = fs::read_to_string("/proc/self/stat").expect("read /proc/self/stat");
    let rest = stat
        .rsplit_once(") ")
        .expect("/proc/self/stat has a comm field")
        .1;
    rest.split_whitespace().map(str::to_owned).collect()
}

#[test]
fn the_host_process_table_is_hidden() {
    let visible = fs::read_dir("/proc")
        .expect("read /proc")
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(|c: char| c.is_ascii_digit())
        })
        .count();
    assert!(
        visible <= VISIBLE_PROCESS_LIMIT,
        "{visible} processes are visible to this test, so it is running in the \
         session's own PID namespace rather than behind the boundary. Run it \
         through cargo, which carries the boundary via .cargo/config.toml."
    );
}

#[test]
fn a_test_has_no_controlling_terminal() {
    let fields = own_stat_fields();
    // `state` is index 0 here, so `session` is field 6 and `tty_nr` field 7.
    let session: i64 = fields[3].parse().expect("session id is a number");
    let terminal: i64 = fields[4].parse().expect("tty_nr is a number");
    assert!(session > 0, "the test did not enter a session of its own");
    assert_eq!(
        terminal, 0,
        "the test kept a controlling terminal, so a case that signals its \
         terminal group can reach the session that started it"
    );
}

#[test]
fn only_target_accepts_a_write() {
    let root = workspace_root();
    let refused = root.join(format!(".containment-probe-{}", std::process::id()));
    let allowed = root.join(format!("target/.containment-probe-{}", std::process::id()));

    assert!(
        fs::create_dir(&refused).is_err(),
        "the repository root accepted a write from a test, so the root is not \
         read-only and a case can edit the sources it is being run against"
    );
    fs::create_dir(&allowed).expect("target/ is the explicit writable bind");
    fs::remove_dir(&allowed).expect("remove the write probe");
}

/// An orphan a test leaves behind is adopted by the boundary, not by init.
///
/// That a descendant then actually dies is not observable from in here — a
/// process cannot watch its own boundary be torn down — so the consequence is
/// measured from the host in `tests/harness/containment-selftest.sh`, where a
/// shell `cargo run` starts backgrounds a sleeper and the check is whether the
/// host still carries it afterwards. It carried one before this was wired up.
///
/// What is observable here is who adopts it, and that has to be asked in a way
/// that can fail: "the orphan is in my PID namespace" is trivially true of a
/// test running in the session's own namespace, where the adopter is the
/// machine's init and nothing bounds the orphan at all. So the adopter is
/// named as well. An unprivileged process cannot resolve the machine init's
/// `exe`; it can resolve the one the sandbox put at PID 1, which exists only
/// for as long as this run does.
#[test]
fn an_orphan_is_adopted_inside_the_boundary() {
    let own = fs::read_link("/proc/self/ns/pid").expect("read this test's PID namespace");
    let adopter = fs::read_link("/proc/1/exe");
    let record =
        workspace_root().join(format!("target/.containment-orphan-{}", std::process::id()));
    // The `sh` exits immediately and the sleep it backgrounded is reparented,
    // which is exactly the shape that ran for forty-seven hours on 2026-08-31.
    // Reparenting inside a PID namespace hands it to that namespace's init,
    // so it dies when the boundary does instead of outliving everything.
    //
    // The pid arrives through a file and the descendant's own descriptors go
    // to /dev/null, because a backgrounded process inherits the descriptors of
    // the shell that started it: read this through a pipe and the read does
    // not end until the descendant does, which is the thirty seconds this test
    // is trying not to spend.
    let script = format!(
        "sleep 30 </dev/null >/dev/null 2>&1 & printf %s \"$!\" >{}",
        record.display()
    );
    let started = Command::new("/bin/sh")
        .arg("-c")
        .arg(&script)
        .status()
        .expect("background a descendant");
    assert!(
        started.success(),
        "the shell did not background a descendant"
    );
    let orphan = fs::read_to_string(&record).expect("the shell recorded the descendant's pid");
    fs::remove_file(&record).expect("remove the pid record");
    let namespace = fs::read_link(format!("/proc/{orphan}/ns/pid"))
        .expect("the backgrounded descendant is still visible");
    // Cleared before the assertion rather than after it, because the run this
    // is asserting about is the one where the assertion fails: there the
    // boundary is not there to tear the descendant down.
    Command::new("/bin/kill")
        .args(["-KILL", &orphan])
        .status()
        .expect("clear the probe descendant");

    assert_eq!(
        namespace, own,
        "a descendant a test orphaned is outside this test's PID namespace, so \
         nothing tears it down when the test run ends"
    );
    assert!(
        adopter.is_ok(),
        "the process adopting this test's orphans is the machine's init \
         ({adopter:?} for /proc/1/exe), so an orphan outlives the run and \
         nothing on this machine is left waiting for it"
    );
}

/// The command's budget is spent by a process standing inside the boundary.
///
/// A budget spent from outside is spent by signalling the sandbox, and that
/// only works once the sandbox has finished setting up: a signal landing in
/// that window reaps the process the caller holds and leaves the tree inside
/// still running. Measured against the shape this replaced, a five-millisecond
/// budget left a descendant running 17 times in 20.
///
/// Whether a descendant then survives is only answerable from the host, and
/// `tests/harness/budget-selftest.sh` is where that is asked. What a test can
/// see from in here is who would enforce its budget, and the two shapes are
/// distinguishable: with the budget inside, the chain is `sandbox` at PID 1,
/// then `timeout`, then this process; with it outside, this process is the
/// child of PID 1 and there is nothing between them that a signal to the
/// sandbox would not take with it.
#[test]
fn the_budget_is_spent_inside_the_boundary() {
    let parent = own_stat_fields()[1]
        .parse::<u32>()
        .expect("the parent pid is a number");
    let enforcer =
        fs::read_to_string(format!("/proc/{parent}/comm")).expect("read the parent's command name");

    assert_eq!(
        enforcer.trim(),
        "timeout",
        "this test's parent is {} rather than a budget, so whatever bounds \
         this run is outside the boundary and a signal to the sandbox can \
         land before the sandbox is ready to pass it on",
        enforcer.trim()
    );
}
