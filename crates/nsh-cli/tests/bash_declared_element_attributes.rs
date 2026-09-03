//! What `-i`, `-u` and `-l` do to an array's elements, measured against
//! the pinned Bash 5.3.
//!
//! The attribute belongs to the variable and not to one scalar slot, so
//! every element takes it however it is written: a compound assignment,
//! a subscripted statement, an appending element. The key of an
//! associative element does not -- `declare -Ai v=(1)` is Bash 5.1's
//! key/value form, so `1` is a key holding nothing and `-i` turns the
//! *value* into `0` while the key stays `1`.
//!
//! The reshape runs after the append and never before it:
//! `declare -al v=(AB); v[0]+=CD` is `abcd` in the reference, which is
//! the whole value lowercased once; lowercasing only what `+=`
//! contributed would give `ABcd`.
//!
//! Nothing here is a recorded expectation. Every case runs in both
//! shells and the two answers are compared, so there is no literal to go
//! stale: if Bash changes its mind, this reports it rather than passing.
//! Diagnostic wording is registered as differing, so only stdout and the
//! exit status are read.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// A compound assignment, whose elements the attribute reshapes one by
/// one.
const A_COMPOUND_ASSIGNMENT: &[&str] = &[
    "declare -ai v=(Q)\ndeclare -p v\n",
    "declare -ai v=(a)\ndeclare -p v\n",
    "declare -ai v=(1+1 2*3)\ndeclare -p v\n",
    "declare -ai v=()\ndeclare -p v\n",
    "declare -ai v=([2]=3+4)\ndeclare -p v\n",
    "declare -au v=(a b)\ndeclare -p v\n",
    "declare -al v=(AB)\ndeclare -p v\n",
    "declare -Ai m=([k]=2+2)\ndeclare -p m\n",
    "declare -Au m=([k]=ab)\ndeclare -p m\n",
    "declare -i a\na=(1+1 2+2)\ndeclare -p a\n",
    /* The attribute reaches the value of a key/value pair and not its
     * key. */
    "declare -Ai v=(1)\ndeclare -p v\n",
    "declare -Ai m=(1 2+2)\ndeclare -p m\n",
    "declare -Au m=(k v)\ndeclare -p m\n",
    /* An arithmetic value the reference will not read abandons the
     * command list rather than storing its text. */
    "declare -ai v=(1+)\ndeclare -p v\necho st=$?\n",
    /* Without the attribute the text is stored as written. */
    "declare -a v=(1+1)\ndeclare -p v\n",
    "declare -A m=([k]=1+1)\ndeclare -p m\n",
];

/// A written element, which takes the attribute the same way.
const A_WRITTEN_ELEMENT: &[&str] = &[
    "declare -ai v=(1 2)\nv[5]=3+4\ndeclare -p v\n",
    "declare -ai v\nv[0]=1+1\ndeclare -p v\n",
    "declare -Ai m\nm[k]=2*3\ndeclare -p m\n",
    "declare -A m\ndeclare -Au m\nm[abc]=xy\ndeclare -p m\n",
    "declare -ai v=(5)\nv=(x)\ndeclare -p v\n",
    "declare -ai v=(1)\nv+=(2+2 3+3)\ndeclare -p v\n",
    "declare -ai a=(1)\na+=(x)\ndeclare -p a\n",
    "declare -ai v=()\nv+=(2+2)\ndeclare -p v\n",
    /* An attribute given after the value is not retroactive. */
    "declare -a v=(x)\ndeclare -u v\ndeclare -p v\n",
    "declare -ai v=(1 2)\ndeclare -i v\ndeclare -p v\n",
];

/// `+=`, whose result the attribute reshapes once the append has been
/// made.
const AN_APPENDED_ELEMENT: &[&str] = &[
    "declare -ai v=(1)\nv[0]+=2\ndeclare -p v\n",
    "declare -au v=(ab)\nv[0]+=cd\ndeclare -p v\n",
    "declare -al v=(AB)\nv[0]+=CD\ndeclare -p v\n",
    "declare -Au m=([k]=ab)\nm[k]+=cd\ndeclare -p m\n",
    "declare -ai v=(3)\nv+=([0]+=2)\ndeclare -p v\n",
    "declare -Ai m=([k]=1)\nm+=([k]+=2)\ndeclare -p m\n",
    "declare -au v=(ab)\nv+=([0]+=cd)\ndeclare -p v\n",
    "declare -ai v=(3)\ndeclare -ai v=([0]+=2)\ndeclare -p v\n",
    /* An unsubscripted `+=` on a scalar reads the same rule. */
    "x=ab\ndeclare -u x\nx+=cd\ndeclare -p x\n",
    "declare -u x=ab\nx+=cd\ndeclare -p x\n",
    "n=1\ndeclare -i n\nn+=' 2 '\ndeclare -p n\n",
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

/// A compound assignment stores what the attribute makes of each
/// element's text.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn a_compound_assignment_reshapes_its_elements() {
    agrees(A_COMPOUND_ASSIGNMENT);
}

/// A written element takes the attribute the compound one takes.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn a_written_element_reshapes_too() {
    agrees(A_WRITTEN_ELEMENT);
}

/// `+=` reshapes the value it built rather than the bytes it added.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn an_append_reshapes_what_it_built() {
    agrees(AN_APPENDED_ELEMENT);
}
