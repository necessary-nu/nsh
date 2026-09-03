//! Four letters a declaring built-in reads, measured against the pinned
//! Bash 5.3.
//!
//! `local -` names no variable: it saves the option set for the duration
//! of the function and restores it on return, out of a nested call as
//! readily as out of the declaring one. Only `local` spells it --
//! `typeset -` is `not a valid identifier` in both shells -- so the
//! rows below pin the boundary as well as the behaviour.
//!
//! `export -n` takes the export attribute back, and is the mirror of
//! `declare +x`. The letter reaches `readonly` as well, where it can do
//! nothing: a read-only attribute cannot be removed by any means in
//! either shell. Neither spelling brings a name into being.
//!
//! `declare +r` is refused for a name that already carries the
//! attribute, which is the same refusal `+a` makes and for the same
//! reason. `declare +r` with no operand is a listing and not a
//! declaration, so the letter must not make the invocation one.
//!
//! `declare -n` is refused when what it would refer to is not an
//! identifier, and what it would refer to is the value the name already
//! holds when none is written. A reference over a value the reference
//! shell cannot hold is a state no differential row can ever pin --
//! `${ref@A}` is `ref='1'` there and would be `declare -n ref='1'` here
//! -- so the refusal is what makes the rest of the table meaningful.
//!
//! Nothing here is a recorded expectation. Every case runs in both
//! shells and the two answers are compared, so there is no literal to go
//! stale: if Bash changes its mind, this reports it rather than passing.
//! Diagnostic wording is registered as differing, so only stdout and the
//! exit status are read -- and `$-` is never printed whole, because the
//! two shells publish different default flags.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// `local -`, which saves the option set rather than naming a variable.
const LOCAL_DASH: &[&str] = &[
    "h(){ local -; set +e; echo ok; }\nset -e\nh\ncase $- in *e*) echo kept;; *) echo lost;; esac\n",
    "f(){ local -; set -f; case $- in *f*) echo in;; esac; }\nf\ncase $- in *f*) echo yes;; *) echo no;; esac\n",
    "f(){ local -; echo st=$?; }\nf\n",
    /* The restore reaches a `return` and a nested call alike. */
    "g(){ local -; set -f; return; }\nf(){ g; case $- in *f*) echo yes;; *) echo no;; esac; }\nf\n",
    "f(){ local -; set -f; g; }\ng(){ case $- in *f*) echo yes;; *) echo no;; esac; }\nf\ncase $- in *f*) echo after-yes;; *) echo after-no;; esac\n",
    /* It sits beside ordinary operands and beside the letters. */
    "f(){ local - x; echo \"x=[${x-unset}]\"; }\nx=out\nf\necho \"$x\"\n",
    "f(){ local -x -; echo ok; }\nf\necho st=$?\n",
    /* And only `local` spells it. */
    "typeset -\necho st=$?\n",
    "declare -\necho st=$?\n",
];

/// `export -n` and `readonly -n`, which take an attribute back.
const THE_N_LETTER: &[&str] = &[
    "x=1\nexport x\nexport -n x\ndeclare -p x\n",
    "x=1\nexport x\nexport -n x=2\ndeclare -p x\n",
    "export -n x=1\ndeclare -p x\necho st=$?\n",
    "set -a\nexport -n x=1\ndeclare -p x\n",
    "x=1\nexport x\nexport -na x\ndeclare -p x\n",
    "export -n -- x\necho st=$?\n",
    /* It reads through a reference, as every other attribute does. */
    "declare -n rr=t\nt=1\nexport rr\nexport -n rr\ndeclare -p t\ndeclare -p rr\n",
    /* Nothing is brought into being by taking something away. */
    "export -n zz\ndeclare -p zz\necho st=$?\n",
    "declare -n r=t\nexport -n r\ndeclare -p r\ndeclare -p t\necho st=$?\n",
    "x=1\nexport x\nexport -n x y\ndeclare -p x\necho st=$?\n",
    /* `readonly` takes the letter and can do nothing with it. */
    "readonly -n x\ndeclare -p x\necho st=$?\n",
    "readonly x=1\nreadonly -n x\ndeclare -p x\n",
    "readonly -n x=1\ndeclare -p x\necho st=$?\n",
    "x=1\nexport x\nreadonly -n x\ndeclare -p x\n",
    /* With no operand the letter spends itself on the listing. */
    "x=1\nexport x\nexport -n | grep -c '^declare -x x'\n",
];

/// `+r`, which a name already carrying the attribute refuses.
const TAKING_READ_ONLY_BACK: &[&str] = &[
    "declare -r q=1\ndeclare +r q\necho st=$?\ndeclare -p q\n",
    "declare -r q=1\ntypeset +r q\necho st=$?\ndeclare -p q\n",
    "f(){ declare -r q=1; local +r q; echo st=$?; }\nf\n",
    "declare -r q=1\ndeclare +rx q\necho st=$?\ndeclare -p q\n",
    "declare -r q=1\ndeclare +r q=2\necho st=$?\ndeclare -p q\n",
    /* Ordinary on a name that is not read-only, and `+x` is ordinary on
     * one that is. */
    "q=1\ndeclare +r q\necho st=$?\ndeclare -p q\n",
    "declare -rx q=1\ndeclare +x q\necho st=$?\ndeclare -p q\n",
    /* A bare `declare +r` is a listing, and the letter must not turn it
     * into a declaration. */
    "q=1\ndeclare +r | grep -c '^q=1$'\n",
    "declare -r q=1\ndeclare -r | grep -c ' q=\"1\"$'\n",
];

/// `-n` over a value that names nothing.
const A_REFERENCE_TO_WHAT_IS_NOT_A_NAME: &[&str] = &[
    "ref=1\ntypeset -n ref\necho st=$?\ndeclare -p ref\n",
    "ref=\ntypeset -n ref\necho st=$?\ndeclare -p ref\n",
    "ref='a b'\ntypeset -n ref\necho st=$?\ndeclare -p ref\n",
    "declare -a arr=(1 2)\ntypeset -n arr\necho st=$?\ndeclare -p arr\n",
    "declare -A m=([k]=v)\ntypeset -n m\necho st=$?\ndeclare -p m\n",
    "declare -a arr\ntypeset -n arr\necho st=$?\ndeclare -p arr\n",
    /* What a reference may hold: a name, an element, or nothing yet. */
    "ref=abc\ntypeset -n ref\necho st=$?\ndeclare -p ref\n",
    "ref='a[0]'\ntypeset -n ref\necho st=$?\ndeclare -p ref\n",
    "unset ref\ntypeset -n ref\necho st=$?\ndeclare -p ref\n",
    "f(){ local -n r; echo st=$?; declare -p r; }\nf\n",
    "declare -n r=x\ndeclare -n r\necho st=$?\ndeclare -p r\n",
    /* `-i` turns the value into a number, and a number is never an
     * identifier -- reported when the written value was already not one
     * and silent when the conversion is what spoiled it. */
    "declare -in r5=5\necho st=$?\ndeclare -p r5\n",
    "declare -in r5=x5\necho st=$?\ndeclare -p r5\n",
    "declare -in r=1+\necho st=$?\ndeclare -p r\n",
    "declare -ni r=abc\necho st=$?\ndeclare -p r\n",
    /* The case letters reshape the target and leave it a name. */
    "declare -un r=x\necho st=$?\ndeclare -p r\n",
    "declare -ln r=X\necho st=$?\ndeclare -p r\n",
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

/// `local -` saves and restores the option set.
// [spec:nsh:req:compat.bash.functions-scoping/test]
#[test]
fn local_dash_saves_the_option_set() {
    agrees(LOCAL_DASH);
}

/// `-n` takes the export attribute back, and takes nothing else.
// [spec:nsh:req:compat.bash.functions-scoping/test]
#[test]
fn the_n_letter_takes_an_export_back() {
    agrees(THE_N_LETTER);
}

/// `+r` is refused for a name that already carries the attribute.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn taking_the_read_only_attribute_back_is_refused() {
    agrees(TAKING_READ_ONLY_BACK);
}

/// `-n` is refused over a value that is not a valid name.
// [spec:nsh:req:compat.bash.functions-scoping/test]
#[test]
fn a_reference_needs_a_name_to_refer_to() {
    agrees(A_REFERENCE_TO_WHAT_IS_NOT_A_NAME);
}
