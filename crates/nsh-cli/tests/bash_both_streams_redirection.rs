//! Bash's `&>` and `&>>`, measured against the pinned Bash 5.3 and
//! against `/usr/bin/dash`.
//!
//! The form had no reading here at all: the `&` reached the operator
//! reader first, so `echo x &> f` ran `echo x` in the *background* and
//! opened `f` for a command with no words. The output stayed on the
//! terminal, the file was created empty, and the backgrounded command
//! raced whatever the shell did next -- which is what made
//! `sh-options.test.sh:23` a coin toss whose bias depended on what else
//! was running in the same survey run.
//!
//! That reading is correct in the POSIX dialect and `/usr/bin/dash` does
//! it, so both dialects are measured here: the Bash half against the
//! pinned reference, the POSIX half against dash. A form that is one
//! operator in one dialect and two tokens in the other is exactly the
//! shape a change in shared code moves by accident.
//!
//! Nothing here is a recorded expectation. Every case runs in both
//! shells and the two answers are compared. Diagnostic wording is
//! registered as differing and `answer` discards standard error, so only
//! standard output and the status are read -- but *whether* a case
//! reports is still measured, in the status and in what stops running.
//!
//! Every case makes files, so every case is given its own directory and
//! removes it on the way out. Two shells sharing one directory would
//! have the second measure the first one's leavings.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// Both streams reach the file, and the file is the only place they go.
const BOTH_STREAMS: &[&str] = &[
    "echo baz &> f; echo s=$?; cat f\n",
    "echo baz &>> f; echo s=$?; cat f\n",
    "{ echo out; echo err >&2; } &> f; echo s=$?; cat f\n",
    "{ echo out; echo err >&2; } &>> f; echo s=$?; cat f\n",
    "echo one &> f; echo two &>> f; echo s=$?; cat f\n",
    "echo one &> f; echo two &> f; echo s=$?; cat f\n",
    /* An external command, whose streams are the descriptors and not a
     * built-in's writer. */
    "ls /nonexistent-4a1c &> f; echo s=$?; wc -l < f\n",
    "f() { echo out; echo err >&2; }; f &> f; cat f\n",
    /* No blank is needed before the operand, and the last operator wins
     * for the standard output while every file named is still made. */
    "echo a &>f; echo s=$?; cat f\n",
    "echo a &>>f; echo s=$?; cat f\n",
    "echo a &> f &> g; echo s=$?; echo \"f=[$(cat f)] g=[$(cat g)]\"\n",
    /* The redirection ends with the command that carried it. */
    "echo a &> f; echo plain; echo s=$?; cat f\n",
    /* `&>` names no descriptor, so a digit in front of it is a word. */
    "echo a 2&> f; echo s=$?; cat f\n",
];

/// `set -C` judges the ampersand forms as it judges the plain ones: it
/// refuses the truncating open on a file that exists and allows the
/// append.
const UNDER_NOCLOBBER: &[&str] = &[
    "echo one > f; set -C; echo two &> f; echo s=$?; cat f\n",
    "echo one > f; set -C; echo two &>> f; echo s=$?; cat f\n",
    "set -C; echo two &> f; echo s=$?; cat f\n",
    "set -C; echo two &>> f; echo s=$?; cat f\n",
    /* The refusal is the redirection's, so the command does not run and
     * the file keeps what it had. */
    "echo one > f; set -C; { echo two; echo three >&2; } &> f; echo s=$?; cat f\n",
];

/// The `&` is still the background operator everywhere the form is not
/// exactly `&>`.
const STILL_BACKGROUND: &[&str] = &[
    "echo a & > f\nwait; echo \"file=[$(cat f)]\"\n",
    "echo a &\nwait; echo done\n",
    "true && echo and-if\n",
    "echo a > f & wait; cat f\n",
];

/// A script the two shells run in a directory of its own, cleaned up
/// however the script ends.
fn contained(script: &str) -> String {
    format!(
        "d=$(mktemp -d) || exit 9\n\
         trap 'cd /; rm -rf \"$d\"' EXIT\n\
         cd \"$d\" || exit 9\n\
         {script}"
    )
}

/// Both shells on one script, as `(what nsh said, what the reference
/// said)`, in the dialect the reference speaks.
fn both(dialect: &[&str], reference: &Path, script: &str) -> ((Vec<u8>, i32), (Vec<u8>, i32)) {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let script = contained(script);
    (
        pinned_bash::answer(nsh, dialect, &script),
        pinned_bash::answer(reference, &[], &script),
    )
}

/// Every script produces the pinned Bash's bytes and status in the Bash
/// dialect.
fn agrees_with_bash(cases: &[&str]) {
    let bash = pinned_bash::path();
    for script in cases {
        let (ours, theirs) = both(&["-o", "bash"], &bash, script);
        assert_eq!(
            String::from_utf8_lossy(&ours.0),
            String::from_utf8_lossy(&theirs.0),
            "output differed for\n{script}"
        );
        assert_eq!(ours.1, theirs.1, "status differed for\n{script}");
    }
}

/// The system dash, which is what the POSIX reading of `&>` is judged
/// against.
///
/// Not optional. A dialect boundary measured against nothing is the
/// boundary this change was most likely to break silently
/// ([`spec:nsh:req:oracle.cannot-measure-is-a-failure`]).
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
fn system_dash() -> &'static Path {
    let dash = Path::new("/usr/bin/dash");
    assert!(
        dash.exists(),
        "no /usr/bin/dash, so the POSIX reading of `&>` has nothing to be \
         checked against and this file would pass without measuring it"
    );
    dash
}

/// `&>` and `&>>` put both streams in the file, as the reference does.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn both_streams_reach_the_file() {
    agrees_with_bash(BOTH_STREAMS);
}

/// `set -C` judges the ampersand forms as the reference judges them.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn noclobber_judges_the_ampersand_forms() {
    agrees_with_bash(UNDER_NOCLOBBER);
}

/// A separated `&` still backgrounds, as it does in the reference.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn a_separated_ampersand_still_backgrounds() {
    agrees_with_bash(STILL_BACKGROUND);
}

/// The POSIX reading, in a shape whose output order is settled.
///
/// `echo x &> f` is two commands there, and the first is a background
/// job: its bytes and the shell's own race unless the script waits
/// before it looks. Each case therefore takes the status into a variable,
/// waits, and only then prints -- so what is compared is the *reading*
/// and not the scheduler that made the survey case a coin toss.
const POSIX_READING: &[&str] = &[
    "echo baz &> f; s=$?; wait; echo s=$s; echo \"file=[$(cat f)]\"\n",
    "echo baz &>> f; s=$?; wait; echo s=$s; echo \"file=[$(cat f)]\"\n",
    "echo a &>f; s=$?; wait; echo s=$s; echo \"file=[$(cat f)]\"\n",
    "echo a &>>f; s=$?; wait; echo s=$s; echo \"file=[$(cat f)]\"\n",
    "{ echo out; echo err >&2; } &> f; s=$?; wait; echo s=$s; echo \"file=[$(cat f)]\"\n",
    "echo a 2&> f; s=$?; wait; echo s=$s; echo \"file=[$(cat f)]\"\n",
    "ls /nonexistent-4a1c &> f; s=$?; wait; echo s=$s; echo \"file=[$(cat f)]\"\n",
    /* `&>>` and not `&>`: the truncating form makes the second command a
     * *null* command whose redirection fails, and a failed redirection on
     * a wordless command leaves 2 in dash and 1 here -- a pre-existing
     * divergence with nothing to do with this form, filed as
     * `give-a-null-commands-failed-redirection-dashs-status`. */
    "echo one > f; set -C; echo two &>> f; s=$?; wait; echo s=$s; echo \"file=[$(cat f)]\"\n",
    "echo a & > f\nwait; echo \"file=[$(cat f)]\"\n",
    "echo a &\nwait; echo done\n",
    "true && echo and-if\n",
    "echo a > f & wait; cat f\n",
];

/// The POSIX dialect reads `&>` as dash reads it: a background operator
/// and a redirection of the next command, not one operator.
///
/// This is the half a change in shared code moves by accident. `&>` is
/// the Bash *dialect's* form and not the `posix` option's, so `bash
/// --posix` still reads it as one operator -- which is why the gate here
/// is `bash::active` and why the check is against dash rather than
/// against the reference with a flag.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn the_posix_dialect_reads_two_tokens() {
    let dash = system_dash();
    for script in POSIX_READING {
        let (ours, theirs) = both(&[], dash, script);
        assert_eq!(
            String::from_utf8_lossy(&ours.0),
            String::from_utf8_lossy(&theirs.0),
            "output differed from dash for\n{script}"
        );
        assert_eq!(ours.1, theirs.1, "status differed from dash for\n{script}");
    }
}
