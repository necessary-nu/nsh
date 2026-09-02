//! Which name a declaring built-in's attribute lands on when the
//! operand is a `declare -n` reference, measured against the pinned
//! Bash 5.3.
//!
//! `readonly rr` through a reference has to protect the variable `rr`
//! names. Until this file it protected `rr` itself, so a script that
//! made a name read-only through a reference could still write it --
//! the one failure the attribute exists to prevent. `export` had the
//! same shape, and so did every letter of `declare` but `-n`.
//!
//! The three built-ins do not read through in quite the same way, which
//! is why the tables below are separate:
//!
//! * `readonly` and `export` follow the chain to a variable and refuse
//!   anything else -- a reference at an element, a reference holding
//!   nothing, a cycle -- reporting it and leaving the status at zero;
//! * `declare` follows the same chain but takes what those refuse: an
//!   element gives the attribute to its array, and a reference holding
//!   nothing takes the attribute itself;
//! * `-n` is the one letter that never reads through, because
//!   `declare -n rr=y` re-points `rr` rather than writing through it.
//!
//! Nothing here is a recorded expectation. Every case runs in both
//! shells and the two answers are compared, so there is no literal to go
//! stale: if Bash changes its mind, this reports it rather than passing.
//! Diagnostic wording is registered as differing and is not read, but
//! *whether* a case reports still is, through its status and through the
//! declarations it did or did not leave behind.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// `readonly` and `export` reach the variable the reference names.
///
/// The second row is the guarantee: a name made read-only through a
/// reference refuses the write that follows.
const THROUGH_THE_REFERENCE: &[&str] = &[
    "declare -n rr=t; t=1; readonly rr; declare -p rr t\n",
    "declare -n rr=t; t=1; readonly rr; t=2; echo \"t=$t\"\n",
    "declare -n rr=t; t=1; readonly rr; echo st=$?; t=2; echo \"after=$? t=$t\"\n",
    "declare -n rr=t; t=1; readonly rr; rr=9; echo \"st=$? t=$t\"\n",
    "declare -n rr=t; readonly rr; declare -p rr; declare -p t\n",
    "declare -n rr=t; t=1; readonly rr; unset t; echo st=$?; declare -p t\n",
    /* The reference itself keeps neither the attribute nor its
     * immutability: it can still be pointed at something else. */
    "declare -n rr=t; readonly rr; declare -n rr=u; declare -p rr\n",
    "declare -n rr=t; t=1; export rr; declare -p rr t\n",
    "declare -n rr=t; t=1; export rr; export -p | grep -E ' (rr|t)='\n",
    "declare -n rr=t; t=1; export rr; env | grep -E '^(rr|t)='\n",
    /* A chain is followed to its end. */
    "declare -n a=b; declare -n b=c; readonly a; declare -p a; declare -p b; declare -p c\n",
    "declare -n a=b; declare -n b=c; export a; declare -p a; declare -p b; declare -p c\n",
    /* A name at the end of the chain that does not exist is created
     * there, invisible, as a bare `readonly zz` creates one. */
    "declare -n rr=nothere; readonly rr; echo st=$?; declare -p rr; declare -p nothere; echo p=$?\n",
    "declare -n rr=nothere; export rr; echo st=$?; declare -p rr; declare -p nothere\n",
    "declare -n rr=nothere; readonly rr; nothere=1; echo \"st=$? v=$nothere\"\n",
    /* An operand carrying a value assigns through the reference and
     * gives the attribute to the same name. */
    "declare -n rr=t; readonly rr=5; echo st=$?; declare -p rr; declare -p t\n",
    "declare -n rr=t; export rr=5; echo st=$?; declare -p rr; declare -p t\n",
    "declare -n rr=t; t=a; export rr+=b; echo st=$?; declare -p rr; declare -p t\n",
    "declare -n rr=t; t=a; readonly rr+=b; echo st=$?; declare -p rr; declare -p t\n",
    "declare -n rr=t; readonly t; readonly rr=5; echo st=$?; declare -p t\n",
    /* A reference to a whole array marks the array. */
    "declare -a z=(1 2); declare -n r=z; readonly r; declare -p r; declare -p z\n",
    "declare -a z=(1 2); declare -n r=z; readonly r; z[0]=9; echo \"st=$? v=${z[0]}\"\n",
    /* And the name that is not a reference is untouched by any of it. */
    "x=1; readonly x; declare -p x; x=2; echo \"st=$? x=$x\"\n",
    "x=1; export x; declare -p x; env | grep '^x='\n",
    "readonly zz; declare -p zz; echo p=$?\n",
    /* A reference declared inside a function still reaches the caller's
     * name, which is what the `declare -n ref=$1` idiom is for. */
    "f() { declare -n rr=$1; readonly rr; }; t=1; f t; declare -p t\n",
    "f() { declare -n rr=t; readonly rr; declare -p t; }; t=1; f; declare -p t\n",
    "f() { declare -n rr=t; export rr; declare -p t; }; t=1; f; declare -p t\n",
];

/// The references `readonly` and `export` will not take a name from.
///
/// All three report and leave the exit status alone, so the operands
/// beside them still take their attribute.
const NO_NAME_TO_REACH: &[&str] = &[
    "declare -n rr; readonly rr; echo st=$?; declare -p rr\n",
    "declare -n rr; export rr; echo st=$?; declare -p rr\n",
    "declare -n a=b; declare -n b; readonly a; echo st=$?; declare -p a; declare -p b\n",
    "declare -n a=b; declare -n b; export a; echo st=$?; declare -p a; declare -p b\n",
    "declare -A A=([k]=v); declare -n r='A[k]'; readonly r; echo st=$?; declare -p r; declare -p A\n",
    "declare -A A=([k]=v); declare -n r='A[k]'; readonly r; A[k]=z; echo \"st=$? v=${A[k]}\"\n",
    "declare -a q=(1 2); declare -n r='q[1]'; export r; echo st=$?; declare -p r; declare -p q\n",
    "declare -A A=([k]=v); declare -n r='A[k]'; readonly r=5; echo st=$?; declare -p A\n",
    "declare -n one=two; declare -n two=one; readonly one; echo st=$?; declare -p one; declare -p two\n",
    "declare -n one=two; declare -n two=one; export one; echo st=$?; declare -p one; declare -p two\n",
    /* The operand beside a refused one is unaffected. */
    "declare -n rr; readonly rr zz; echo st=$?; declare -p zz\n",
];

/// Every letter of `declare` but `-n` reaches the same variable.
const DECLARE_READS_THROUGH: &[&str] = &[
    "declare -n rr=t; t=1; declare -r rr; declare -p rr; declare -p t\n",
    "declare -n rr=t; t=1; declare -r rr; t=2; echo \"st=$? t=$t\"\n",
    "declare -n rr=t; t=1; declare -x rr; declare -p rr; declare -p t\n",
    "declare -n rr=t; t=1; declare -x rr; env | grep -E '^(rr|t)='\n",
    "declare -n rr=t; t=1; declare -i rr; declare -p rr; declare -p t\n",
    "declare -n rr=t; t=1; declare -i rr; rr=2+3; declare -p t\n",
    "declare -n rr=t; t=1; declare -u rr; t=abc; declare -p t\n",
    "declare -n rr=t; t=1; declare -l rr; t=ABC; declare -p t\n",
    "declare -n rr=t; t=1; declare -t rr; declare -p t\n",
    "declare -n rr=t; t=1; declare -a rr; declare -p rr; declare -p t\n",
    "declare -n rr=t; t=1; declare -A rr; declare -p rr; declare -p t\n",
    "declare -n rr=t; t=1; declare rr=9; declare -p rr; declare -p t\n",
    "declare -n rr=t; t=1; declare -i rr=3+4; declare -p rr; declare -p t\n",
    "declare -n rr=t; t=1; declare -x rr; declare +x rr; declare -p rr; declare -p t\n",
    "declare -n a=b; declare -n b=c; c=1; declare -i a; declare -p a; declare -p b; declare -p c\n",
    "declare -n a=b; declare -i a; declare -p a; declare -p b\n",
    "declare -n rr=nothere; declare -i rr; declare -p rr; declare -p nothere\n",
    /* Where `readonly` refuses, `declare` takes what it can: the array an
     * element belongs to, or the reference itself. */
    "declare -n rr; declare -i rr; echo st=$?; declare -p rr\n",
    "declare -A A=([k]=v); declare -n r='A[k]'; declare -i r; echo st=$?; declare -p r; declare -p A\n",
    /* A cycle, and a chain ending on another reference holding nothing,
     * declare nothing at all. */
    "declare -n one=two; declare -n two=one; declare -i one; echo st=$?; declare -p one; declare -p two\n",
    "declare -n a=b; declare -n b; declare -i a; echo st=$?; declare -p a; declare -p b\n",
    /* In a function it is the target that goes local. */
    "f() { declare -n rr=t; declare -i rr; declare -p t; }; t=1; f; declare -p t\n",
    "f() { declare -n rr=$1; local -i rr; declare -p t; }; t=1; f t; declare -p t\n",
    "f() { declare -n rr=t; typeset -r rr; }; t=1; f; declare -p t\n",
    /* A name that is not a reference is where it always was. */
    "declare -i n; n=3+4; declare -p n\n",
    "x=1; declare -r x; x=2; echo \"st=$? x=$x\"\n",
];

/// `-n` is the letter that writes the reference rather than its target.
const THE_NAMEREF_LETTER_STAYS_PUT: &[&str] = &[
    "declare -n rr=t; t=1; declare -rn rr; declare -p rr; declare -p t\n",
    "declare -n rr=t; t=1; declare -n rr=u; declare -p rr; declare -p t\n",
    "declare -n rr=t; t=1; declare +n rr; declare -p rr; declare -p t\n",
    "declare -n rr=t; t=1; declare -nr rr=q; declare -p rr; declare -p t\n",
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

/// `readonly` and `export` land on the name the reference holds, and the
/// refusal that follows is that name's.
// [spec:nsh:req:compat.bash.functions-scoping/test]
// [spec:nsh:req:compat.bash.arrays-declarations/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn an_attribute_reaches_the_reference_s_target() {
    agrees(THROUGH_THE_REFERENCE);
}

/// A reference that names no variable is reported and takes no
/// attribute, as it is in the reference.
// [spec:nsh:req:compat.bash.functions-scoping/test]
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn a_reference_naming_nothing_is_refused() {
    agrees(NO_NAME_TO_REACH);
}

/// `declare`'s letters reach the same variable `readonly`'s do.
// [spec:nsh:req:compat.bash.functions-scoping/test]
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn a_declaration_reads_through_the_reference() {
    agrees(DECLARE_READS_THROUGH);
}

/// `-n` writes the reference itself, as it does in the reference.
// [spec:nsh:req:compat.bash.functions-scoping/test]
#[test]
fn the_nameref_letter_writes_the_reference() {
    agrees(THE_NAMEREF_LETTER_STAYS_PUT);
}
