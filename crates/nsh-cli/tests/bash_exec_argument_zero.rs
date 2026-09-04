//! What `exec` calls the program it runs, measured against the pinned
//! Bash 5.3.
//!
//! `exec -a name prog` runs `prog` under an `argv[0]` of `name`. It is
//! the only way a script can hand a child a different `argv[0]`, and in
//! this repository `argv[0]` is what selects the dialect, so it is also
//! how a Bash-mode script re-enters itself.
//!
//! `-l` is the same mechanism and is measured with it: it prefixes a
//! hyphen to whichever name is in force, which is `-a`'s when there is
//! one and the program word otherwise. `-c`, the third letter of the
//! reference's `[-cl] [-a name]`, is an environment rather than a name,
//! and no row here asserts either side of it --
//! `bash.divergences.exec-empty-environment` holds its measurement.
//!
//! THE LETTERS ARE BASH'S ALONE, so the POSIX dialect must go on reading
//! `-a` as a program name. That half cannot be a differential here --
//! dash is not wired into this crate's harness the way `pinned_bash`
//! wires the reference Bash -- so it is recorded, against
//! `tests/.build/ref/src/dash` 0.5.12-12 on 2026-09-04, at load 60:
//! `exec -a MYNAME sh -c ...`, `exec -aMYNAME ...`, `exec -a` alone,
//! `exec -l ...`, `exec -al N ...` and `exec -z ...` all answer 127 with
//! the option read as the missing program, and all end the shell. The
//! default dialect answers the same six, which is what
//! `the_default_dialect_still_has_no_such_letter` pins.
//!
//! Nothing in the differential rows is a recorded expectation. Every case
//! runs in both shells and the two answers are compared, so there is no
//! literal to go stale: if Bash changes its mind, this reports it rather
//! than passing. Diagnostic wording is registered as differing in
//! `docs/divergences.md`, so only standard output and the exit status are
//! read -- but *whether* a case reported is still measured, because a
//! refusal shows up in the status and in the commands that no longer run.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// The name the image runs under.
const NAMES_THE_PROCESS: &[&str] = &[
    "exec -a MYNAME /bin/sh -c 'echo 0=$0'\n",
    /* The name may be attached to the letter, and may be empty. */
    "exec -aMYNAME /bin/sh -c 'echo 0=$0'\n",
    "exec -a '' /bin/sh -c 'echo [$0]'\n",
    /* The program is still found by the word after the name, whether
     * that word is a path or a PATH search. */
    "exec -a MYNAME sh -c 'echo 0=$0'\n",
    "exec -a MYNAME /bin/echo hi\n",
    /* `-l` prefixes a hyphen to whichever name is in force. */
    "exec -l /bin/sh -c 'echo 0=$0'\n",
    "exec -l -a N /bin/sh -c 'echo 0=$0'\n",
    /* `--` ends the options and does not become the name. */
    "exec -a N -- /bin/sh -c 'echo 0=$0'\n",
    /* The redirections `exec` makes permanent are unaffected, and a
     * command before it still runs. */
    "echo first\nexec -a N /bin/sh -c 'echo 0=$0'\n",
    "exec -a N /bin/sh -c 'echo 0=$0' 2>/dev/null\n",
];

/// What the letters refuse, and whether the refusal ends the shell.
const REFUSES: &[&str] = &[
    /* An option that is not one of them, and a `-a` with nothing to
     * take: both report, take a status, and let the next command run. */
    "exec -z /bin/true\necho after=$?\n",
    "exec -a\necho after=$?\n",
    /* `-a` clusters, so `-al` is `-a` with an argument of `l` and `N` is
     * then the program. */
    "exec -al N /bin/sh -c 'echo 0=$0'\necho after=$?\n",
    /* A name with no program at all is the redirection-only form and
     * spends the name on nothing. */
    "exec -a N\necho after=$?\n",
    "exec -a N 2>/dev/null\necho after=$?\n",
    /* Past `--` the letter is a program name and there is no such
     * program, which ends a non-interactive shell. */
    "exec -- -a\necho after=$?\n",
    /* The two ways the program itself cannot be run keep their own
     * statuses past the option scan: 127 for a name nothing answers to,
     * 126 for one that answers and cannot be executed. */
    "exec -a N /no/such/prog-for-exec-argument-zero\necho after=$?\n",
    "exec -a N /\necho after=$?\n",
];

/// Both shells on one script, as `(what nsh said, what the pinned Bash
/// said)`.
fn both(script: &str) -> ((Vec<u8>, i32), (Vec<u8>, i32)) {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let bash = pinned_bash::path();
    (
        pinned_bash::answer(nsh, &["-o", "bash"], script),
        pinned_bash::answer(&bash, &[], script),
    )
}

/// Every script in `cases` produces the reference's bytes and status.
fn agrees(cases: &[&str]) {
    for script in cases {
        let (ours, theirs) = both(script);
        assert_eq!(
            String::from_utf8_lossy(&ours.0),
            String::from_utf8_lossy(&theirs.0),
            "output differed for\n{script}"
        );
        assert_eq!(ours.1, theirs.1, "status differed for\n{script}");
    }
}

/// `exec -a name` and `exec -l` say what the image will call itself.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn the_letters_name_the_process() {
    agrees(NAMES_THE_PROCESS);
}

/// A letter that is not one of them, and a letter with nothing to take.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn the_option_scan_refuses_what_bash_refuses() {
    agrees(REFUSES);
}

/// The default dialect has no such letters and must keep dash's answer.
///
/// Recorded rather than differential for the reason the module comment
/// gives. The status is the whole assertion: the letter is read as the
/// program, no such program is found, and a non-interactive shell ends
/// there, so nothing after the `exec` runs in any of the six.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn the_default_dialect_still_has_no_such_letter() {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    for script in [
        "exec -a MYNAME sh -c 'echo 0=$0'\necho after\n",
        "exec -aMYNAME sh -c 'echo 0=$0'\necho after\n",
        "exec -a\necho after\n",
        "exec -l sh -c 'echo 0=$0'\necho after\n",
        "exec -al N sh -c 'echo 0=$0'\necho after\n",
        "exec -z /bin/true\necho after\n",
    ] {
        let (stdout, status) = pinned_bash::answer(nsh, &[], script);
        assert_eq!(status, 127, "status for\n{script}");
        assert!(stdout.is_empty(), "the shell ran on past\n{script}");
    }
}
