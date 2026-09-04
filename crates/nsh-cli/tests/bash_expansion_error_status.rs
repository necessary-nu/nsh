//! What a terminal expansion refusal leaves as a Bash-mode shell's exit
//! status, measured against the pinned Bash 5.3 in all three invocation
//! shapes.
//!
//! The status turned out to belong to the *frame the shell leaves
//! through* rather than to the failure, which is why every case below is
//! run three ways. `sh -c 'unset x; echo ${x?z}'` ends the shell with
//! 127; the same script as a file operand or on standard input ends it
//! with 1. Bash evaluates a `-c` string through `parse_and_execute`,
//! whose jump handler answers `EX_NOTFOUND`, and reads a file or
//! standard input through a loop that answers the failure's own status.
//! Both of its modes agree, so it is not a `--posix` question.
//!
//! Nothing here is a recorded expectation. Every case runs in both
//! shells, in all three shapes, and the answers are compared, so a
//! reference that changes its mind reports rather than passes. Only
//! stdout and the exit status are read: diagnostic wording is registered
//! as differing in `docs/divergences.md`, while *whether* a case reported
//! still shows in the status and in the commands that no longer ran.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

/// Every case gets a directory of its own, because these tests are
/// threads of one process and a shared working directory is exactly the
/// shape `[spec:nsh:req:oracle.checks-do-not-share-state]` refuses.
static NEXT_CASE: AtomicU64 = AtomicU64::new(0);

/// How the shell was started, which is what this file is about.
#[derive(Clone, Copy)]
enum Invocation {
    /// `sh -c SCRIPT`, the one shape that answers 127.
    CommandString,
    /// `sh ./case.sh`.
    FileOperand,
    /// `sh` with the script on standard input.
    StandardInput,
}

const SHAPES: &[Invocation] = &[
    Invocation::CommandString,
    Invocation::FileOperand,
    Invocation::StandardInput,
];

/// A refusal the reference does not recover from: the shell ends, and
/// only the number is in question.
const TERMINAL: &[&str] = &[
    "unset x; echo ${x?z}; echo R\n",
    "unset x; echo ${x:?boom}; echo R\n",
    "unset x; : ${x?z}; echo R\n",
    "unset x; eval \"echo \\${x?z}\"; echo R\n",
    "unset x; f() { echo ${x?z}; }; f; echo R\n",
    "set -u; unset x; echo $x; echo R\n",
    "set -u; unset x; echo \"pre ${x}\"; echo R\n",
];

/// A refusal the reference *does* recover from, which answers 1 in every
/// shape. Here so that the change cannot widen 127 across the class:
/// these leave through the same frame under `-c` and must not move.
const RECOVERED: &[&str] = &[
    "echo ${x!bad}; echo R\n",
    "echo $((1+)); echo R\n",
    "declare -A m; echo ${m[$(exit 3)]}; echo R\n",
];

/// A refusal contained by a child, which the enclosing shell only ever
/// sees as a status.
const CONTAINED: &[&str] = &[
    "unset x; ( echo ${x?z} ); echo \"s=$?\"\n",
    "unset x; echo $(echo ${x?z}); echo \"s=$?\"\n",
    "unset x; ( set -u; echo $x ); echo \"s=$?\"\n",
];

/// One script through one shell in one invocation shape, as
/// `(stdout, status)`.
fn answer(shell: &Path, dialect: &[&str], shape: Invocation, script: &str) -> (Vec<u8>, i32) {
    let directory = std::env::temp_dir().join(format!(
        "nsh-expansion-status-{}-{}",
        std::process::id(),
        NEXT_CASE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).expect("create the case directory");
    let mut command = Command::new(shell);
    command
        .args(dialect)
        .current_dir(&directory)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    match shape {
        Invocation::CommandString => {
            command.arg("-c").arg(script).stdin(Stdio::null());
        }
        Invocation::FileOperand => {
            std::fs::write(directory.join("case.sh"), script).expect("write the case script");
            command.arg("./case.sh").stdin(Stdio::null());
        }
        Invocation::StandardInput => {
            command.stdin(Stdio::piped());
        }
    }
    let mut child = command
        .spawn()
        .unwrap_or_else(|error| panic!("start {}: {error}", shell.display()));
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(script.as_bytes())
            .expect("write the script to the shell");
    }
    let output = child.wait_with_output().expect("wait for the shell");
    (output.stdout, output.status.code().unwrap_or(-1))
}

/// Every script, in every invocation shape, answers what the reference
/// answers.
#[track_caller]
fn agrees(cases: &[&str]) {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let bash = pinned_bash::path();
    for script in cases {
        for shape in SHAPES {
            let ours = answer(nsh, &["-o", "bash"], *shape, script);
            let theirs = answer(&bash, &[], *shape, script);
            assert_eq!(
                (String::from_utf8_lossy(&ours.0).into_owned(), ours.1),
                (String::from_utf8_lossy(&theirs.0).into_owned(), theirs.1),
                "disagreed with the reference for\n{script}"
            );
        }
    }
}

/// A refusal the reference ends the shell over answers the reference's
/// number, which is 127 from `-c` and 1 from a file or standard input.
// [spec:nsh:req:compat.bash.error-boundary/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn a_terminal_refusal_answers_the_references_number() {
    agrees(TERMINAL);
}

/// A refusal the reference recovers from still answers 1, in every
/// invocation shape.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn a_recovered_refusal_still_answers_one() {
    agrees(RECOVERED);
}

/// A child contains the refusal, so the enclosing shell reads a status
/// and carries on -- and the status it reads is the reference's.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn a_child_contains_the_refusal() {
    agrees(CONTAINED);
}

/// The default dialect is not in any of this: it answers dash's 2 in
/// every shape, and this is the check that the Bash-mode number did not
/// leak into it.
///
/// dash's own answer is recorded rather than run -- `/usr/bin/dash`
/// 0.5.12-12 answers 2 for every row of `TERMINAL` in all three shapes,
/// measured 2026-09-04 -- because dash is not wired into this crate's
/// harness the way `pinned_bash` wires the reference Bash. The
/// whole-corpus differential sweep is what runs it.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn the_default_dialect_answers_dashs_two() {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    for script in TERMINAL {
        for shape in SHAPES {
            let (stdout, status) = answer(nsh, &[], *shape, script);
            assert_eq!(
                (String::from_utf8_lossy(&stdout).into_owned(), status),
                (String::new(), 2),
                "the default dialect moved for\n{script}"
            );
        }
    }
}
