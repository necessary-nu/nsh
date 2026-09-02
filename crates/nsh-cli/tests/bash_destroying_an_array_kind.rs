//! What `declare +a` and `declare +A` do to a name that holds that
//! kind, measured against the pinned Bash 5.3.
//!
//! An array's kind is how its value is *held* rather than a flag over
//! it, so Bash will not take it back: `declare +a` on an indexed array
//! reports and answers 1 with the variable exactly as it was. This shell
//! accepted the letter silently, answered 0, and left the kind alone --
//! the value ended up right and the status did not, which is why nothing
//! in the surveys ever saw it.
//!
//! The refusal is narrow, and the rows below are mostly about how narrow.
//! `+a` on an *associative* array is ordinary, and so is either letter on
//! a name that holds no array at all -- `declare +a nosucharray` even
//! brings the name into being. `+n` is not one of the letters Bash
//! refuses, despite naming the same kind of representation.
//!
//! Nothing here is a recorded expectation. Every case runs in both
//! shells and the two answers are compared, so there is no literal to go
//! stale: if Bash changes its mind, this reports it rather than passing.
//! Diagnostic wording is registered as differing and is not read, so what
//! carries the refusal here is the exit status and the declaration the
//! command did not change.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// The kind a declaration may not take away.
const A_KIND_CANNOT_BE_DESTROYED: &[&str] = &[
    "declare -a z; declare +a z; declare -p z; echo rc=$?\n",
    "declare -a z; declare +a z; echo st=$?\n",
    "declare -a z=(1); declare +a z; echo st=$?; declare -p z\n",
    "declare -A m; declare +A m; echo st=$?; declare -p m\n",
    "declare -A m=([k]=v); declare +A m; echo st=$?; declare -p m\n",
    /* Nothing else the same command asked for lands either: the operand
     * is refused before any letter and before any value it carries. */
    "declare -a z; declare +ai z; echo st=$?; declare -p z\n",
    "declare -a z; declare +a z=1; echo st=$?; declare -p z\n",
    "declare -A m; declare +A m=x; echo st=$?; declare -p m\n",
    "declare -a z; declare +a -r z; echo st=$?; declare -p z\n",
    /* The operands beside it are unaffected, in either order. */
    "declare -a z; declare +a z ok; echo st=$?; declare -p ok\n",
    "declare -a z; declare +a ok z; echo st=$?; declare -p ok\n",
    /* A subscripted operand names its array, and a reference reaches
     * one. */
    "declare -a z; declare +a 'z[0]'; echo st=$?; declare -p z\n",
    "declare -a z; declare -n r=z; declare +a r; echo st=$?; declare -p z; declare -p r\n",
    "declare -A m; declare -n r=m; declare +A r; echo st=$?; declare -p m\n",
    /* In a function, and under the built-in's other two names. */
    "f() { declare -a z; declare +a z; echo st=$?; }; f\n",
    "f() { declare -a z; local +a z; echo st=$?; }; f\n",
    "declare -a z; typeset +a z; echo st=$?; declare -p z\n",
    /* A second refusal is still a refusal, and the array still works. */
    "declare -a z; declare +a z; declare +a z; echo st=$?; declare -p z\n",
    "declare -a z; declare +a z 2>/dev/null; z[0]=5; declare -p z\n",
];

/// The shapes the refusal deliberately does not cover.
const THE_LETTER_IS_OTHERWISE_ORDINARY: &[&str] = &[
    /* The *other* letter is not the kind the name holds. */
    "declare -A m; declare +a m; echo st=$?; declare -p m\n",
    "declare -a z; declare +A z; echo st=$?; declare -p z\n",
    /* A name that holds no array takes the letter, and is created. */
    "declare +a nosucharray; echo st=$?; declare -p nosucharray\n",
    "declare +A nosuchmap; echo st=$?; declare -p nosuchmap\n",
    "x=1; declare +a x; echo st=$?; declare -p x\n",
    "x=1; declare +A x; echo st=$?; declare -p x\n",
    /* `-a` after a refused `+a` still declares. */
    "declare +a z; declare -a z; declare -p z; echo st=$?\n",
    /* `+n` names a representation too and is not refused. */
    "declare -n r=t; declare +n r; echo st=$?; declare -p r\n",
    /* With no operand the letter is spent on the listing, which is not
     * an error. The listing's own form is `quote-the-set-listing-as-
     * bash-quotes-it`'s question and is deliberately not read here. */
    "declare -a zq=(1); declare +a > /dev/null; echo st=$?\n",
    "declare -a zq=(1); declare -p +a | grep ' zq='; echo st=$?\n",
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

/// `+a` and `+A` on a name that holds that kind are refused as the
/// reference refuses them, with the variable left as it was.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
// [spec:nsh:req:compat.bash.error-boundary/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn a_kind_cannot_be_taken_away() {
    agrees(A_KIND_CANNOT_BE_DESTROYED);
}

/// Everywhere else the letters are ordinary, as they are in the
/// reference.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn the_letter_is_otherwise_ordinary() {
    agrees(THE_LETTER_IS_OTHERWISE_ORDINARY);
}
