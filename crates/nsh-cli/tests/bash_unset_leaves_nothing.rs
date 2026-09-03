//! What `unset` leaves behind, measured against the pinned Bash 5.3.
//!
//! Two questions, both about a name after `unset` rather than about the
//! value, which is why neither showed up in a survey: the value is
//! genuinely gone either way and only a declaration printer can see the
//! difference.
//!
//! `unset` is not an assignment, so `set -a` does not reach it. The
//! option's mark makes a write's attributes non-empty, and non-empty
//! attributes are enough to bring an entry into being; the entry is then
//! inherited by the next declaration of the same name, which is how it
//! reaches past a listing.
//!
//! A refused operand does not end the command either: `unset a x b` with
//! a read-only `x` reports `x`, unsets `b`, and answers 1. A caller that
//! stops at the refusal leaves every name after it silently set, and the
//! exit status cannot say how much of the list ran.
//!
//! Both are Bash-dialect answers. dash keeps the entry -- `set -a; unset
//! zz; export -p` names `zz` there -- and a special built-in's failure is
//! fatal in the POSIX dialect, so there is no operand after the refusal
//! to reach; `tests/harness` covers that side.
//!
//! Nothing here is a recorded expectation. Every case runs in both
//! shells and the two answers are compared, so there is no literal to go
//! stale: if Bash changes its mind, this reports it rather than passing.
//! Diagnostic wording is registered as differing, so only stdout and the
//! exit status are read.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// `set -a` and `unset`, which have nothing to do with one another.
const ALLEXPORT_DOES_NOT_REACH_UNSET: &[&str] = &[
    "set -a\nx=1\nunset x\ndeclare -p x\necho st=$?\n",
    "set -a\nunset zz\ndeclare -p zz\n",
    "set -a\nunset PATHX\ndeclare -p PATHX\n",
    "set -a\ndeclare -a z=(1)\nunset z\ndeclare -p z\n",
    "set -a\nx=1\nunset x\nexport -p | grep -c '^declare -x x'\n",
    /* The entry left behind was inherited by the next declaration of the
     * same name, which is the reach beyond the listing. */
    "set -a\nunset z\ndeclare -a z\ndeclare -p z\n",
    "set -a\nx=1\nunset x\ndeclare -a x\ndeclare -p x\n",
    "set -a\nunset z\ndeclare -i z\ndeclare -p z\n",
    /* The value was never in doubt. */
    "set -a\nx=1\nunset x\necho \"[${x-gone}]\"\n",
    /* And a name the option marked is still exported until it goes. */
    "set -a\nx=1\ndeclare -p x\n",
];

/// A refused operand, after which the reference keeps unsetting and
/// answers 1 for the whole command.
const A_REFUSED_OPERAND_STOPS_NOTHING: &[&str] = &[
    "readonly x=1\na=1\nb=2\nunset a x b\necho \"status=$? a=[${a-gone}] b=[${b-gone}]\"\n",
    "readonly x=1\ny=2\nunset y x\necho \"st=$? y=[${y-gone}]\"\n",
    "readonly x=1\nunset x y\necho st=$?\n",
    "readonly x=1\nunset x\necho after\n",
    "readonly x=1\nunset x\necho st=$?\n",
    "readonly a=1 b=2\nc=3\nunset a b c\necho \"st=$? c=[${c-gone}]\"\n",
    /* A function body is no different, and the caller sees the status. */
    "f(){ readonly r=1; unset r; echo \"st=$?\"; }\nf\necho done\n",
    /* Nothing refused, nothing reported. */
    "a=1\nunset a\necho st=$?\n",
    "unset nope\necho st=$?\n",
    "a=1\nb=2\nunset a b\necho \"st=$? [${a-gone}] [${b-gone}]\"\n",
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

/// `unset` leaves no entry for `set -a` to have marked.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn allexport_does_not_reach_unset() {
    agrees(ALLEXPORT_DOES_NOT_REACH_UNSET);
}

/// A refused operand is reported and the operands after it still run.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn a_refused_operand_does_not_end_the_list() {
    agrees(A_REFUSED_OPERAND_STOPS_NOTHING);
}
