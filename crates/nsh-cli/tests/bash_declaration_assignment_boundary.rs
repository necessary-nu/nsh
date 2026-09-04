//! Which failure of a declaration utility abandons the record, measured
//! against the pinned Bash 5.3.
//!
//! Bash reaches two outcomes here by two mechanisms, and they are easy to
//! confuse because both report and both leave status 1. A utility that
//! cannot *evaluate* the value it was handed -- `declare -i x=1+` --
//! unwinds out of itself, so the rest of the record goes with it. A
//! utility that merely *refuses* an operand -- a read-only name, a bad
//! identifier, an option it does not have -- returns a failing status, and
//! the next command of the same list runs. Only the first is an
//! assignment error.
//!
//! The two spellings of one command reach the failure by different
//! routes and must still answer alike: `declare -ai v=(1+)` raises before
//! the built-in is entered, because a compound value is read by the
//! assignment-word path, while `declare -i x=1+` raises inside it and
//! crosses the frame that catches a built-in's failure. `local`,
//! `typeset` and `export` on a name already carrying `-i` are that second
//! route under other names, so each is held here rather than left to be
//! inferred from `declare`.
//!
//! Nothing here is a recorded expectation. Every case runs in both shells
//! and the two answers are compared, so there is no literal to go stale.
//! Diagnostic wording is a registered divergence, so only stdout and the
//! status are read -- but *whether* a case reported still shows up in
//! both, through the status and through the commands that no longer run.
//!
//! Every script is fed on standard input rather than as a `-c` string,
//! and that is load-bearing. Bash reads a `-c` string through
//! `parse_and_execute`, which abandons the whole string rather than the
//! record inside it, so `declare -i x=1+` there takes a following *line*
//! with it as well; a file and standard input both resume at the next
//! line. That frame difference is the subject of
//! `bash.divergences.assignment-error-in-a-c-string`, not of this file,
//! and feeding stdin keeps the two questions apart.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// An arithmetic value that will not evaluate, which is an assignment
/// error in every utility that can be handed one.
///
/// The first row is the whole divergence: the reference runs nothing
/// after it, and this shell ran the rest of the list. The rows after it
/// are the same failure reached through the other spellings, and
/// `1/0` is there so the case does not rest on a parse failure alone.
const ASSIGNMENT_ERROR_ABANDONS: &[&str] = &[
    "declare -i x=1+; echo NEXT\n",
    "declare -i x=1+; echo NEXT\necho AFTER-RECORD\n",
    "declare -i x=1/0; echo NEXT\n",
    "typeset -i x=1+; echo NEXT\n",
    "f() { local -i x=1+; echo INNER; }; f; echo NEXT\n",
    "declare -i e; export e=1+; echo NEXT\n",
    "declare -i r; readonly r=1+; echo NEXT\n",
    "declare -i x=1+; echo NEXT\ndeclare -p x\n",
    /* The value must not land, and the name must still exist: the
     * declaration ran before the value was read. */
    "declare -i x=1+\ndeclare -p x\n",
    "declare -i x=9\ndeclare -i x=1+\ndeclare -p x\n",
    /* A second operand after the failing one is never reached. */
    "declare -i x=1+ y=2; echo NEXT\ndeclare -p y\n",
    /* The compound spelling already agreed and must stay where it is. */
    "declare -ai v=(1+); echo NEXT\n",
    "declare -ai v=(1+); echo NEXT\necho AFTER-RECORD\n",
    /* A plain assignment to a name already carrying `-i` is the same
     * error reached without a built-in at all. */
    "declare -i x; x=1+; echo NEXT\n",
];

/// A refusal, which reports and leaves the rest of the list running.
///
/// These are the control: they travel as the same error value as the
/// rows above and must not be escalated by the same change. `let` and
/// `(( ))` are here because they fail *arithmetically* and still do not
/// abandon, which is what makes the boundary an assignment's and not
/// arithmetic's.
const REFUSAL_KEEPS_THE_LIST: &[&str] = &[
    "declare -i 9bad=1; echo NEXT\n",
    "declare -r r=1; declare r=2; echo NEXT\n",
    "declare -Z x; echo NEXT\n",
    "local x=1; echo NEXT\n",
    "readonly r=1; unset r; echo NEXT\n",
    "unset -v 'a['; echo NEXT\n",
    "let 'q=1+'; echo NEXT\n",
    "(( 1+ )); echo NEXT\n",
    "declare -i x; declare -p nosuchname; echo NEXT\n",
    /* A read-only name refusing a *value* is still a refusal and not an
     * assignment error, which is the pair that shows the mark is on the
     * arithmetic and not on the built-in. */
    "declare -r r=1; declare -i r=2; echo NEXT\n",
];

/// Where the abandonment stops.
///
/// A subshell and a command substitution contain it, so the enclosing
/// shell sees a status and reads on.
///
/// Only the single-record `errexit` case is here. What `set -e` does to
/// this class across a *record* boundary is a divergence of its own and
/// older than this file: the reference reports an arithmetic assignment
/// error and reads the next record with `set -e` live, where this shell
/// stops, and it does so for the compound spelling and the plain
/// assignment as much as for `declare -i` -- so it is not this boundary's
/// question. `bash.divergences.errexit-over-an-assignment-error` holds it,
/// with the measurement.
const THE_ABANDONMENT_IS_CONTAINED: &[&str] = &[
    "( declare -i x=1+; echo INNER ); echo NEXT\n",
    "v=$( declare -i x=1+; echo INNER ); echo \"NEXT [$v]\"\n",
    "( declare -i x=1+; echo INNER )\necho AFTER-RECORD\n",
    "f() { declare -i x=1+; echo INNER; }; f; echo NEXT\necho AFTER-RECORD\n",
    "set -e; declare -i x=1+; echo NEXT\n",
    /* A failure inside a condition is still a failure of the record it
     * was written in. */
    "if declare -i x=1+; then echo THEN; fi; echo NEXT\n",
    "declare -i x=1+ && echo AND; echo NEXT\n",
    "declare -i x=1+ || echo OR; echo NEXT\n",
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

/// A value that will not evaluate abandons the record, as it does in the
/// reference.
// [spec:nsh:req:compat.bash.error-boundary/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn an_unevaluable_value_abandons_the_record() {
    agrees(ASSIGNMENT_ERROR_ABANDONS);
}

/// An operand the reference merely refuses leaves its siblings running.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn a_refused_operand_leaves_the_list_running() {
    agrees(REFUSAL_KEEPS_THE_LIST);
}

/// A subshell contains the abandonment and `errexit` overrides it, as in
/// the reference.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn the_abandonment_stops_where_the_reference_stops_it() {
    agrees(THE_ABANDONMENT_IS_CONTAINED);
}

/// The POSIX dialect is not moved by any of it: `declare` is not a
/// utility there, and the fatal boundary the dialect keeps is the one
/// `/usr/bin/dash` has.
///
/// [`crate::pinned_bash::answer`]'s dialect argument is what selects it,
/// so this runs the same scripts with Bash mode off. The reference is
/// this shell's own POSIX dialect rather than Bash, because Bash has no
/// mode in which `declare` is absent -- `--posix` keeps it -- and the
/// question is whether the mark added for Bash mode leaked into the
/// dialect that must not have it.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn the_posix_dialect_keeps_its_own_boundary() {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    for script in ["declare -i x=1+; echo NEXT\n", "x=1+; echo NEXT\n"] {
        let (out, status) = pinned_bash::answer(nsh, &[], script);
        assert_eq!(
            String::from_utf8_lossy(&out),
            "NEXT\n",
            "the POSIX dialect ran something else for\n{script}"
        );
        assert_eq!(status, 0, "status differed for\n{script}");
    }
}
