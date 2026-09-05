//! What `set -e` does to a failure that abandons its input record,
//! measured against the pinned Bash 5.3.
//!
//! `errexit` does not reach a failed arithmetic evaluation in the
//! reference: it reports, abandons the record, and reads the next one
//! with `set -e` live. It does reach a refusal, which ends the shell.
//! That split is the scope
//! `[spec:nsh:req:compat.bash.error-boundary]` states, and this file is
//! what observes it.
//!
//! The two classes are indistinguishable by the thing that is easiest to
//! reach for. Both report, both leave status 1, and both abandon the rest
//! of their own record -- `FAIL; echo SAME` prints nothing for either. So
//! `SAME_RECORD_IS_ABANDONED_EITHER_WAY` holds that shared half, and the
//! two `set -e` tables hold the half where they part.
//!
//! The control that makes the correction a scope error rather than a
//! preference is `WITHOUT_ERREXIT`: the arithmetic rows answer the same
//! with `set -e` removed, so `errexit` is doing no work for that class in
//! either shell.
//!
//! `(( 1+ ))` and `let x=1+` are in neither table. The arithmetic is the
//! command there, so its failure is that command's status rather than an
//! abandonment, and ordinary `errexit` ends the shell; they are held in
//! `ARITHMETIC_AS_A_COMMAND` precisely because they fail arithmetically
//! and must *not* follow the arithmetic rows.
//!
//! Nothing here is a recorded expectation. Every case runs in both shells
//! in both frames and the answers are compared, so there is no literal to
//! go stale. Diagnostic wording is a registered divergence, so only
//! stdout and the status are read.
//!
//! Two frames, not three. A `-c` string abandons *itself* in the
//! reference where a file and standard input resume at the next record,
//! which is a different question with its own node --
//! `bash.divergences.assignment-error-in-a-c-string` -- and mixing it in
//! here would let either divergence hide the other.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Distinguishes each case's working directory, so no two cases share
/// one.
// [spec:nsh:req:oracle.checks-do-not-share-state]
static NEXT_CASE: AtomicUsize = AtomicUsize::new(0);

/// The frames this file measures. A `-c` string is deliberately absent.
#[derive(Clone, Copy)]
enum Invocation {
    FileOperand,
    StandardInput,
}

const FRAMES: &[Invocation] = &[Invocation::FileOperand, Invocation::StandardInput];

/// A failed arithmetic evaluation, which the reference reads past even
/// with `errexit` on.
///
/// Every spelling that can carry one is here rather than left to be
/// inferred from `declare -i`: the value of a declaration, a subscript
/// being read, a subscript being assigned through, a slice bound, an
/// arithmetic expansion in a plain assignment and one in a command word.
/// Each reaches the arithmetic by a different route and all must answer
/// alike.
const ARITHMETIC_SURVIVES_ERREXIT: &[&str] = &[
    "set -e\ndeclare -i x=1+\necho NEXT\n",
    "set -e\ndeclare -ai v=(1+)\necho NEXT\n",
    "set -e\ntypeset -i x=1+\necho NEXT\n",
    "set -e\ndeclare -i x; x=1+\necho NEXT\n",
    "set -e\nf() { local -i x=1+; }; f\necho NEXT\n",
    "set -e\ndeclare -a w; w[1+]=2\necho NEXT\n",
    "set -e\ndeclare -a w; echo ${w[1+]}\necho NEXT\n",
    /* The array must have elements. The reference never evaluates a
     * slice bound on an array holding none, so an empty `w` would test
     * the arithmetic in this shell and nothing at all in the reference;
     * `bash.divergences.slice-bound-on-an-empty-array` holds that. */
    "set -e\ndeclare -a w=(a b c); echo ${w[@]:1+:2}\necho NEXT\n",
    "set -e\ns=abc; echo ${s:1+:2}\necho NEXT\n",
    "set -e\ndeclare -A m; m[$((1+))]=v\necho NEXT\n",
    "set -e\nx=$((1+))\necho NEXT\n",
    "set -e\necho $((1+))\necho NEXT\n",
    /* Division by zero, so the class does not rest on a parse failure
     * alone. */
    "set -e\ndeclare -i x=1/0\necho NEXT\n",
    /* More than one record after the failure, so a shell that merely
     * survives one is not mistaken for one that carried on. */
    "set -e\ndeclare -i x=1+\necho NEXT\necho LAST\n",
];

/// A refusal, which `errexit` still ends the shell for.
///
/// These travel as the same error value as the rows above and must not
/// be recovered by the same change.
///
/// `export 1bad=1`, `readonly 1bad=1` and `unset -v 1bad` belong to this
/// table by behaviour and are not in it: in Bash mode this shell answers
/// status 2 where the reference answers 1, which is a divergence of the
/// number and not of the boundary -- both shells stop. It is older than
/// this file and `bash.divergences.bash-mode-bad-identifier-status`
/// holds it with the measurement.
const A_REFUSAL_IS_STILL_FATAL: &[&str] = &[
    "set -e\nreadonly r=1; r=2\necho NEXT\n",
    "set -e\ndeclare -r r=1; r=2\necho NEXT\n",
    "set -e\nreadonly r=1; unset r\necho NEXT\n",
    "set -e\ndeclare -i 1bad=1\necho NEXT\n",
    "set -e\ndeclare -A a; declare -a a\necho NEXT\n",
    "set -e\necho ${!x@bad}\necho NEXT\n",
];

/// Arithmetic that *is* the command, which abandons nothing and which
/// ordinary `errexit` therefore ends the shell for.
const ARITHMETIC_AS_A_COMMAND: &[&str] = &[
    "set -e\n(( 1+ ))\necho NEXT\n",
    "set -e\nlet \"x=1+\"\necho NEXT\n",
    /* The same two failing arithmetically without `errexit`, where the
     * status is all that is left of the failure. */
    "(( 1+ )); echo \"s=$?\"\n",
    "let \"x=1+\"; echo \"s=$?\"\n",
];

/// The rest of the failing record is abandoned in both classes, which is
/// why abandonment cannot be what separates them.
const SAME_RECORD_IS_ABANDONED_EITHER_WAY: &[&str] = &[
    "declare -i x=1+; echo SAME\necho NEXT\n",
    "x=$((1+)); echo SAME\necho NEXT\n",
    "declare -a w; w[1+]=2; echo SAME\necho NEXT\n",
    "readonly r=1; r=2; echo SAME\necho NEXT\n",
    "echo ${!x@bad}; echo SAME\necho NEXT\n",
];

/// The same failures with `errexit` never set.
///
/// The control for the whole file: if these and the `set -e` tables
/// answer alike for the arithmetic rows, `errexit` is not the thing
/// deciding them.
const WITHOUT_ERREXIT: &[&str] = &[
    "declare -i x=1+\necho NEXT\n",
    "x=$((1+))\necho NEXT\n",
    "declare -a w; w[1+]=2\necho NEXT\n",
    "readonly r=1; r=2\necho NEXT\n",
    "echo ${!x@bad}\necho NEXT\n",
];

/// One script through one shell in one frame, as `(stdout, status)`.
fn answer(shell: &Path, dialect: &[&str], frame: Invocation, script: &str) -> (Vec<u8>, i32) {
    let directory = std::env::temp_dir().join(format!(
        "nsh-errexit-assignment-{}-{}",
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
    match frame {
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

/// Every script, in both frames, answers what the reference answers.
#[track_caller]
fn agrees(cases: &[&str]) {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let bash = pinned_bash::path();
    for script in cases {
        for frame in FRAMES {
            let ours = answer(nsh, &["-o", "bash"], *frame, script);
            let theirs = answer(&bash, &[], *frame, script);
            assert_eq!(
                (String::from_utf8_lossy(&ours.0).into_owned(), ours.1),
                (String::from_utf8_lossy(&theirs.0).into_owned(), theirs.1),
                "disagreed with the reference for\n{script}"
            );
        }
    }
}

/// `errexit` does not reach a failed arithmetic evaluation.
// [spec:nsh:req:compat.bash.error-boundary/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn an_arithmetic_failure_is_read_past_under_errexit() {
    agrees(ARITHMETIC_SURVIVES_ERREXIT);
}

/// `errexit` still ends the shell for a refusal.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn a_refusal_still_ends_the_shell_under_errexit() {
    agrees(A_REFUSAL_IS_STILL_FATAL);
}

/// Arithmetic as a command is an ordinary failing command.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn arithmetic_as_a_command_is_an_ordinary_failure() {
    agrees(ARITHMETIC_AS_A_COMMAND);
}

/// Both classes abandon the rest of the record they were raised in.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn the_failing_record_is_abandoned_in_both_classes() {
    agrees(SAME_RECORD_IS_ABANDONED_EITHER_WAY);
}

/// Without `errexit` the classes answer alike, which is what makes the
/// difference above `errexit`'s and not the failure's.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn the_classes_agree_when_errexit_is_never_set() {
    agrees(WITHOUT_ERREXIT);
}

/// The POSIX dialect is not moved by any of it.
///
/// `declare` is not a utility there and an arithmetic failure keeps the
/// fatal boundary, so the mark added for Bash mode must not have leaked.
/// The reference is this shell's own POSIX dialect against
/// `/usr/bin/dash`, because Bash has no mode in which `declare` is
/// absent.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn the_posix_dialect_keeps_its_fatal_boundary() {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let dash = Path::new("/usr/bin/dash");
    if !dash.exists() {
        panic!("the POSIX reference /usr/bin/dash is not installed");
    }
    for script in [
        "set -e\nx=$((1+))\necho NEXT\n",
        "x=$((1+))\necho NEXT\n",
        "set -e\nreadonly r=1; r=2\necho NEXT\n",
    ] {
        for frame in FRAMES {
            let ours = answer(nsh, &[], *frame, script);
            let theirs = answer(dash, &[], *frame, script);
            assert_eq!(
                (String::from_utf8_lossy(&ours.0).into_owned(), ours.1),
                (String::from_utf8_lossy(&theirs.0).into_owned(), theirs.1),
                "the POSIX dialect left dash for\n{script}"
            );
        }
    }
}
