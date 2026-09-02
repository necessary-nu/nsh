//! Where `ERR` stops at a fork, measured against the pinned Bash 5.3.
//!
//! Bash raises `ERR` where `errexit` would act -- but a command it forked
//! a child *for* is not such a place, and the reason is mechanical rather
//! than a rule anyone wrote down. `execute_simple_command` forks an
//! asynchronous simple command from inside itself and the child becomes
//! the command, so `execute_command_internal` never reaches the foot
//! where the status is read back; that same foot is guarded by `pipe_in
//! == NO_PIPE && pipe_out == NO_PIPE`, which says it again for a pipeline
//! member; and entering a subshell, Bash runs the subshell's *body*, so
//! the subshell node is never a command anything read a status back from
//! either. Every other compound command is a control structure Bash runs
//! inside the child it forked first, and its failure is noticed there as
//! usual -- which is why `(( 0 )) &` raises and `false &` does not.
//!
//! This shell forked above the check in all three cases and raised where
//! Bash is silent. The corpus saw one of the shapes
//! (`builtin-trap-err.test.sh:12`); the rest were found by sweeping the
//! shapes around it against the reference, which is why the table below
//! is that sweep rather than the one case.
//!
//! What is asserted is the trap's own output, because the divergence was
//! never in the status: every script here ends the same way under both
//! shells, and only the trap says how it got there. Both shells are run
//! as processes for the same reason -- the cases turn on what a forked
//! child does, and an in-process shell sharing this test binary cannot
//! `wait` for one.

/// Shared with `nsh`'s own differential tests rather than copied: one
/// answer to "which Bash", not a second one that can drift from it.
#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

/// Two lines the shell reads before any case's own first line, so that a
/// `line=` below names a line of the body plus two.
const PRELUDE: &str = "set -o errtrace\ntrap 'echo line=$LINENO' ERR\n";

/// One script body, the trap output the pinned Bash gives it, and this
/// shell's where it still differs.
///
/// A `Some` is a divergence recorded rather than described: it fails when
/// the divergence is closed, so the entry cannot outlive its reason. None
/// is left: the last two, `$LINENO` for a subshell, went with the line a
/// compound command records, whose own table is
/// `bash_compound_command_line`.
const CASES: &[(&str, &str, Option<&str>)] = &[
    /* Forked as the command itself: Bash is silent. */
    ("false &\nwait\n", "", None),
    ("false | false &\nwait\n", "", None),
    ("false >/dev/null &\nwait\n", "", None),
    ("false | cat &\nwait\n", "", None),
    ("false | cat\n", "", None),
    ("! true &\nwait\n", "", None),
    ("while false; do :; done &\nwait\n", "", None),
    /* A pipeline member is silent, and the pipeline itself is not. */
    ("false | false\n", "line=3\n", None),
    ("{ false; } | cat\n", "line=3\n", None),
    /* An asynchronous control structure keeps its own check, and the
     * body of an asynchronous subshell or function keeps its. */
    ("(( 0 )) &\nwait\n", "line=3\n", None),
    ("[[ 1 = 2 ]] &\nwait\n", "line=3\n", None),
    ("{\nfalse\n} &\nwait\n", "line=4\n", None),
    ("for i in 1; do\nfalse\ndone &\nwait\n", "line=4\n", None),
    ("case x in x)\nfalse;; esac &\nwait\n", "line=4\n", None),
    ("(\nfalse\n) &\nwait\n", "line=4\n", None),
    ("(\n(\nfalse\n)\n) &\nwait\n", "line=5\n", None),
    ("f() {\nfalse\n}\nf &\nwait\n", "line=4\n", None),
    /* Nothing was forked for these, so every one of them is noticed. */
    ("false\n", "line=3\n", None),
    ("{ false; }\n", "line=3\n", None),
    ("x=$(false)\n", "line=3\n", None),
    ("if false; then :; fi\n", "", None),
    ("f() {\nfalse\n}\nf\n", "line=4\nline=6\n", None),
    /* A subshell's own failure, which is the shape the sweep found the
     * count right and the line wrong on. The line it records is
     * `bash_compound_command_line`'s subject; both are asserted here so
     * the count cannot be closed by moving the line. */
    ("(\nfalse\n)\n", "line=4\nline=5\n", None),
    ("(\n(\nfalse\n)\n)\n", "line=5\nline=7\n", None),
];

/// Feed one case to a shell on standard input and return what it printed.
fn trap_output(shell: &Path, dialect: &[&str], body: &str) -> String {
    let mut child = Command::new(shell)
        .args(dialect)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|error| panic!("start {}: {error}", shell.display()));
    let mut source = PRELUDE.as_bytes().to_vec();
    source.extend_from_slice(body.as_bytes());
    child
        .stdin
        .take()
        .expect("the child's standard input")
        .write_all(&source)
        .expect("write the script");
    let output = child.wait_with_output().expect("wait for the shell");
    String::from_utf8(output.stdout).expect("the trap prints ASCII")
}

// [spec:nsh:req:compat.bash.traps-introspection/test]
#[test]
fn a_fork_stops_the_error_trap() {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    for (body, reference, divergence) in CASES {
        let want = divergence.unwrap_or(reference);
        assert_eq!(
            trap_output(nsh, &["-o", "bash"], body),
            *want,
            "for the script\n{PRELUDE}{body}"
        );
    }
}

/// The table is the reference's answer, not this repository's opinion --
/// including the two rows that record where this shell still differs.
// [spec:nsh:req:compat.bash.traps-introspection/test]
#[test]
fn the_recorded_output_is_the_references_own() {
    let bash = pinned_bash::path();
    for (body, reference, divergence) in CASES {
        assert_eq!(
            trap_output(&bash, &[], body),
            *reference,
            "the reference disagrees with the recorded output for\n{PRELUDE}{body}"
        );
        assert!(
            divergence.is_none_or(|recorded| recorded != *reference),
            "a divergence that matches the reference is not one; drop the row for\n{PRELUDE}{body}"
        );
    }
}
