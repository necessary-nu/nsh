//! What `readonly -a` and `export -A` do with an operand that carries a
//! value, measured against the pinned Bash 5.3.
//!
//! The letter is consulted for any operand that carries a **value**,
//! compound or not, and for no operand that carries none. That boundary
//! is a data difference and not a spelling one: `readonly -a q=1` is the
//! indexed array `([0]="1")`, so `q[1]=x`, `${q[@]}` and `${#q[@]}` all
//! answer differently from the scalar `"1"` afterwards.
//!
//! The export letter rides on the same boundary. An array is a
//! *declaration* and `set -a` marks assignments, so
//! `set -a; readonly -a z=(1)` is `declare -ar z` where the same value
//! without the letter -- `set -a; readonly z=(1)` -- is `declare -arx z`.
//!
//! Nothing here is a recorded expectation. Every case runs in both
//! shells and the two answers are compared, so there is no literal to go
//! stale: if Bash changes its mind, this reports it rather than passing.
//! Diagnostic wording is registered as differing, so only stdout and the
//! exit status are read.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// A `name=value` operand, whose value lands as element zero of the
/// array the letter declares.
const A_PLAIN_VALUE_TAKES_THE_LETTER: &[&str] = &[
    "readonly -a q=1\ndeclare -p q\n",
    "export -a e=1\ndeclare -p e\n",
    "readonly -A k=v\ndeclare -p k\n",
    "export -A k=v\ndeclare -p k\n",
    "readonly -a q=1 r=2\ndeclare -p q\ndeclare -p r\n",
    "readonly -a q=\ndeclare -p q\n",
    /* Element zero and not the whole value: an array already there keeps
     * every other element it holds. */
    "declare -a x=(7 8)\nreadonly -a x=1\ndeclare -p x\n",
    "x=5\nreadonly -a x=1\ndeclare -p x\n",
    /* Read back as an element rather than as a scalar, which is what a
     * `declare -p` row alone would not settle. */
    "readonly -a q=1\necho \"${#q[@]} [${q[0]}]\"\n",
    "export -a e=1\ne[1]=x\ndeclare -p e\n",
    "readonly -A k=v\necho \"${#k[@]} [${k[0]}]\"\n",
];

/// An operand with no value, which the letter does not reach: the
/// name stays a scalar and, with no operand at all, the letter can only
/// narrow the listing.
const A_BARE_NAME_DOES_NOT: &[&str] = &[
    "readonly -A m\ndeclare -p m\necho status=$?\n",
    "export -A m\ndeclare -p m\n",
    "readonly -a q\ndeclare -p q\n",
    "export -a e\ndeclare -p e\n",
    "set -a\nreadonly -a z\ndeclare -p z\n",
    "set -a\nexport -A m\ndeclare -p m\n",
    "declare -a z=(1)\nreadonly -a z\nreadonly -a\n",
    "readonly -A\necho status=$?\n",
];

/// `+=`, which reaches element zero of the array the letter declared
/// where without the letter it concatenates onto a scalar.
const AN_APPENDING_OPERAND: &[&str] = &[
    "readonly -a q+=1\ndeclare -p q\n",
    "export -a e+=1\ndeclare -p e\n",
    "declare -a q=(1 2)\nexport -a q+=3\ndeclare -p q\n",
    "x=ab\nreadonly -a x+=cd\ndeclare -p x\n",
    "readonly -A m+=v\ndeclare -p m\n",
    /* Without the letter an unsubscripted `+=` stays a scalar; a name
     * already declared an array appends to its zero element even so. */
    "readonly q+=1\ndeclare -p q\n",
    "declare -a z\nz+=v\ndeclare -p z\n",
    "declare -A m\nm+=v\ndeclare -p m\n",
    "declare -a z=(1 2)\nz+=v\ndeclare -p z\n",
];

/// A kind the name may not be given: the reference reports it, drops
/// the value, and still applies the attribute the command asked for.
const A_REFUSED_CONVERSION: &[&str] = &[
    "declare -a a=(1)\nreadonly -A a=v\necho status=$?\ndeclare -p a\n",
    "declare -A m=([k]=1)\nreadonly -a m=v\necho status=$?\ndeclare -p m\n",
    "declare -A m=([k]=1)\nexport -a m=v\necho status=$?\ndeclare -p m\n",
    "declare -a a=(1)\nreadonly -A a=v b=2\necho status=$?\ndeclare -p b\n",
];

/// `set -a`, which marks an assignment and not the declaration the
/// letter makes of an operand carrying a value.
const ALLEXPORT_LEAVES_A_DECLARATION_ALONE: &[&str] = &[
    "set -a\nreadonly -a z=(1)\ndeclare -p z\n",
    "set -a\nreadonly -A m=([k]=v)\ndeclare -p m\n",
    "set -a\nreadonly -a z=1\ndeclare -p z\n",
    "set -a\nreadonly -A z=v\ndeclare -p z\n",
    "set -a\nexport -a z=1\ndeclare -p z\n",
    "set -a\nexport -a z=(1)\ndeclare -p z\n",
    "set -a\nreadonly -a z=(1) y=(2)\ndeclare -p z\ndeclare -p y\n",
    /* The same value without the letter is an assignment, which the
     * option does mark. */
    "set -a\nreadonly z=(1)\ndeclare -p z\n",
    "set -a\nreadonly z=1\ndeclare -p z\n",
    "set -a\ndeclare -r z=(1)\ndeclare -p z\n",
    /* An export the name already carried is the script's and survives. */
    "set -a\nz=1\nreadonly -a z=(9)\ndeclare -p z\n",
    "set -a\nz=1\nreadonly -a z=9\ndeclare -p z\n",
    /* An array has no environment spelling, so nothing escapes either
     * way and no survey could have seen this. */
    "set -a\nreadonly -a z=1\nenv | grep -c '^z'\n",
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

/// A `name=value` operand becomes the array the letter names, and its
/// value becomes element zero.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn a_plain_value_takes_the_array_letter() {
    agrees(A_PLAIN_VALUE_TAKES_THE_LETTER);
}

/// An operand with no value stays the scalar it would be without the
/// letter.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn a_bare_operand_takes_neither_letter() {
    agrees(A_BARE_NAME_DOES_NOT);
}

/// `+=` reaches element zero of the array the letter declared.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn an_appending_operand_reaches_element_zero() {
    agrees(AN_APPENDING_OPERAND);
}

/// A refused conversion drops the value and keeps the attribute.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn a_refused_conversion_keeps_the_attribute() {
    agrees(A_REFUSED_CONVERSION);
}

/// `set -a` marks an assignment and leaves an array declaration bare.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn allexport_does_not_mark_an_array_declaration() {
    agrees(ALLEXPORT_LEAVES_A_DECLARATION_ALONE);
}
