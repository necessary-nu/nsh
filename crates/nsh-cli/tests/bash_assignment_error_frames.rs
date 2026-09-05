//! Which frame an assignment error leaves through, measured against the
//! pinned Bash 5.3.
//!
//! Two failures that look identical inside one record part company at the
//! frame above it. The arithmetic the variable machinery evaluates itself
//! -- a declaration's integer value, an indexed subscript -- is recovered
//! only by the loop reading the shell's own input, so a `-c` string, an
//! `eval` operand and a `.` script all go with it. The same fault reached
//! through an expansion, `$(( ))` or a slice bound, abandons its record
//! and is read past in every frame.
//!
//! `declare -i x=$((1+))` and `declare -i x=1+` are the pair that shows it
//! is the raise and not the utility: same builtin, same attribute, same
//! unevaluable text, and the reference keeps reading after the first and
//! abandons the whole `-c` string for the second.
//!
//! A subshell and a command substitution still contain everything, and an
//! associative subscript is a key rather than arithmetic -- `m[1+]=v` is
//! not a failure at all -- so both are here as controls against a change
//! that widens the class.
//!
//! Nothing here is a recorded expectation. Every case runs in both shells
//! in every frame and the answers are compared, so there is no literal to
//! go stale. Diagnostic wording is a registered divergence, so only stdout
//! and the status are read; *whether* a case reported still shows through
//! the commands that no longer run.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Distinguishes each case's working directory, so no two cases share one.
// [spec:nsh:req:oracle.checks-do-not-share-state]
static NEXT_CASE: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy)]
enum Invocation {
    CommandString,
    FileOperand,
    StandardInput,
}

const FRAMES: &[Invocation] = &[
    Invocation::CommandString,
    Invocation::FileOperand,
    Invocation::StandardInput,
];

/// Two records. The first carries the failure and a second command that
/// only runs if the record survived; the second record only runs if the
/// frame above it survived.
fn two_records(failure: &str) -> String {
    format!("{failure}; echo SAME\necho AFTER\n")
}

/// The arithmetic the variable machinery evaluates itself, which leaves a
/// `-c` string, an `eval` and a `.` script entirely.
const UNWINDS_PAST_A_NESTED_FRAME: &[&str] = &[
    "declare -i x=1+",
    "declare -ai v=(1+)",
    "typeset -i x=1+",
    "declare -i x; x=1+",
    "declare -i x=1/0",
    "declare -a w; w[1+]=2",
    "declare -a w; echo ${w[1+]}",
    "f() { local -i x=1+; }; f",
    /* Reached through a nested frame, which must not be a recovery point
     * for this class. */
    "eval \"declare -i x=1+\"",
    "f() { declare -i x=1+; }; f",
    "declare -i x=1+ || echo OR",
    "if declare -i x=1+; then echo THEN; fi",
];

/// The same fault reached through an expansion, which abandons its record
/// and no more.
///
/// `declare -i x=$((1+))` is the discriminator: the declaration utility is
/// the same one as above and only the route to the arithmetic differs.
const ABANDONS_ONLY_ITS_RECORD: &[&str] = &[
    "x=$((1+))",
    "echo $((1+))",
    "declare -i x=$((1+))",
    "declare -a w; w[$((1+))]=2",
    "declare -a w; echo ${w[$((1+))]}",
    "declare -a w=(a b); echo ${w[@]:1+:1}",
    /* A refusal rather than an arithmetic failure, which has never left a
     * record and must not start. */
    "readonly r=1; r=2",
    "declare -r r=1; r=2",
];

/// What contains the abandonment however it was raised.
const CONTAINED_BY_A_CHILD: &[&str] = &[
    "( declare -i x=1+ )",
    "v=$( declare -i x=1+ ); echo \"v=[$v]\"",
];

/// An associative subscript is a key rather than arithmetic, so nothing
/// fails and the whole record runs. A change that read every subscript as
/// arithmetic would be caught here.
const AN_ASSOCIATIVE_KEY: &[&str] = &["declare -A m; m[1+]=v", "declare -A m; m[a b]=v; echo ok"];

/// One script through one shell in one frame, as `(stdout, status)`.
///
/// `beside` is a file written next to the case before it runs, which is
/// how the dot-script rows get a `lib.sh` to source.
fn answer(
    shell: &Path,
    dialect: &[&str],
    frame: Invocation,
    script: &str,
    beside: Option<(&str, &str)>,
) -> (Vec<u8>, i32) {
    let directory = std::env::temp_dir().join(format!(
        "nsh-assignment-frames-{}-{}",
        std::process::id(),
        NEXT_CASE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).expect("create the case directory");
    if let Some((name, text)) = beside {
        std::fs::write(directory.join(name), text).expect("write the neighbouring script");
    }
    let mut command = Command::new(shell);
    command
        .args(dialect)
        .current_dir(&directory)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    match frame {
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

/// Every failure, in every frame, answers what the reference answers.
#[track_caller]
fn agrees(failures: &[&str]) {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let bash = pinned_bash::path();
    for failure in failures {
        let script = two_records(failure);
        for frame in FRAMES {
            let ours = answer(nsh, &["-o", "bash"], *frame, &script, None);
            let theirs = answer(&bash, &[], *frame, &script, None);
            assert_eq!(
                (String::from_utf8_lossy(&ours.0).into_owned(), ours.1),
                (String::from_utf8_lossy(&theirs.0).into_owned(), theirs.1),
                "disagreed with the reference for\n{script}"
            );
        }
    }
}

/// The variable machinery's own arithmetic leaves every nested frame.
// [spec:nsh:req:compat.bash.error-boundary/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn a_declarations_arithmetic_leaves_a_nested_frame() {
    agrees(UNWINDS_PAST_A_NESTED_FRAME);
}

/// The same fault through an expansion stops at its own record.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn an_expansions_arithmetic_stops_at_its_record() {
    agrees(ABANDONS_ONLY_ITS_RECORD);
}

/// A subshell and a command substitution contain it.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn a_child_contains_the_abandonment() {
    agrees(CONTAINED_BY_A_CHILD);
}

/// An associative subscript is not arithmetic and cannot fail this way.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn an_associative_key_is_not_arithmetic() {
    agrees(AN_ASSOCIATIVE_KEY);
}

/// A `.` script is not a recovery point either, and the frame above it
/// decides what survives.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn a_dot_script_is_not_a_recovery_point() {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let bash = pinned_bash::path();
    let script = ". ./lib.sh; echo SAME\necho AFTER\n";
    let library = (
        "lib.sh",
        "declare -i x=1+; echo INNER-SAME\necho INNER-AFTER\n",
    );
    for frame in FRAMES {
        let ours = answer(nsh, &["-o", "bash"], *frame, script, Some(library));
        let theirs = answer(&bash, &[], *frame, script, Some(library));
        assert_eq!(
            (String::from_utf8_lossy(&ours.0).into_owned(), ours.1),
            (String::from_utf8_lossy(&theirs.0).into_owned(), theirs.1),
            "disagreed with the reference for a dot script"
        );
    }
}

/// The POSIX dialect keeps its own fatal boundary in every frame.
///
/// `declare` is not a utility there, so the reference is `/usr/bin/dash`
/// and the question is whether the mark added for Bash mode leaked.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn the_posix_dialect_is_not_moved() {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let dash = Path::new("/usr/bin/dash");
    assert!(
        dash.exists(),
        "the POSIX reference /usr/bin/dash is missing"
    );
    for failure in ["x=$((1+))", "readonly r=1; r=2", "unset q; echo ${q?b}"] {
        let script = two_records(failure);
        for frame in FRAMES {
            let ours = answer(nsh, &[], *frame, &script, None);
            let theirs = answer(dash, &[], *frame, &script, None);
            assert_eq!(
                (String::from_utf8_lossy(&ours.0).into_owned(), ours.1),
                (String::from_utf8_lossy(&theirs.0).into_owned(), theirs.1),
                "the POSIX dialect left dash for\n{script}"
            );
        }
    }
}
