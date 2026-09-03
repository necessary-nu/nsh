//! The two entries a declaration leaves that carry nothing at all,
//! measured against the pinned Bash 5.3.
//!
//! Bash's *invisible* variable is a name that has a declaration and no
//! value. `92f97f9` built it for `declare -a z`; these are the two other
//! ways to reach one, and both are invisible to everything except a
//! declaration printer -- the values were never in question.
//!
//! A bare `declare NAME` on a name this shell reserved for a callback is
//! one. `initialize_variables` enters `MAIL`, `MAILPATH`, `HISTSIZE`,
//! `TERM` and the five locale names as entries holding nothing, so that
//! a later assignment has a callback to run, and Bash has no variable
//! there at all. Every other declaration of one leaves a mark that tells
//! the two apart; a bare `declare` leaves none, so the state carries it.
//!
//! `unset` on a name the running body made local is the other.
//! `unset` there leaves an invisible local rather than taking the entry
//! away, and the declaration it leaves carries nothing:
//! `local -i pv=1; unset pv` is `declare -- pv` and not `declare -i pv`.
//! The scope that owns the local is the one that gets it -- a nested
//! function unsetting its caller's local removes the entry outright.
//!
//! Nothing here is a recorded expectation. Every case runs in both
//! shells and the two answers are compared, so there is no literal to go
//! stale: if Bash changes its mind, this reports it rather than passing.
//! Diagnostic wording is registered as differing, so only stdout and the
//! exit status are read.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// A declaration of a name the shell had already reserved a slot for.
///
/// `TERM` is deliberately absent: Bash publishes it with a value, so the
/// row measures `publish-the-names-bash-publishes` rather than this.
const A_RESERVED_NAME: &[&str] = &[
    "declare MAIL\ndeclare -p MAIL\n",
    "declare LANG\ndeclare -p LANG\n",
    "declare HISTSIZE\ndeclare -p HISTSIZE\n",
    "declare LC_CTYPE\ndeclare -p LC_CTYPE\n",
    "declare MAILPATH\ndeclare -p MAILPATH\n",
    "declare MAIL\ndeclare -p | grep -c '^declare -- MAIL$'\n",
    /* The slot itself is still nobody's declaration. */
    "declare -p LANG\necho st=$?\n",
    "declare -p MAIL\necho st=$?\n",
    "declare -p | grep -c '^declare -- MAIL$'\n",
    /* And `unset` gives the slot back. */
    "declare MAIL\nunset MAIL\ndeclare -p MAIL\n",
    /* A declaration that leaves any other mark is told apart by the
     * mark, and these rows are what says the state has not replaced
     * that test. */
    "declare -i MAILPATH\ndeclare -p MAILPATH\n",
    "readonly MAIL\ndeclare -p MAIL\n",
    "declare -x HISTSIZE\ndeclare -p HISTSIZE\n",
    "declare -a LC_CTYPE\ndeclare -p LC_CTYPE\n",
    "declare -l MAIL\ndeclare -p MAIL\n",
    /* The name still holds nothing, and an assignment still lands. */
    "declare MAIL\necho \"[${MAIL-unset}]\"\n",
    "declare MAIL\necho \"[${MAIL@A}]\"\n",
    "declare MAIL\nMAIL=x\ndeclare -p MAIL\n",
    /* An ordinary name reaches the same state by the same route. */
    "declare pv\ndeclare -p pv\n",
    "declare pv\necho \"[${pv@A}]\"\n",
    "declare zz\ndeclare -p | grep -c '^declare -- zz$'\n",
];

/// `unset` on a name the running function body made local.
const AN_UNSET_LOCAL: &[&str] = &[
    "f(){ local pv=1; unset pv; declare -p pv; echo \"[${pv@A}]\"; }\nf\n",
    "f(){ local pv; declare -p pv; }\nf\n",
    "f(){ declare pv=1; unset pv; declare -p pv; }\nf\n",
    "f(){ local pv=1; unset pv; unset pv; declare -p pv; }\nf\n",
    "f(){ local pv=1; unset pv; declare -p | grep -c '^declare -- pv$'; }\nf\n",
    /* Whatever the declaration carried goes with the value. */
    "f(){ local -i pv=1; unset pv; declare -p pv; }\nf\n",
    "f(){ local -x pv=1; unset pv; declare -p pv; }\nf\n",
    "f(){ local -a pv=(1); unset pv; declare -p pv; }\nf\n",
    /* The caller's value is untouched and comes back on return. */
    "f(){ local pv=1; unset pv; declare -p pv; }\npv=outer\nf\ndeclare -p pv\n",
    "f(){ local pv=1; unset pv; pv=2; declare -p pv; }\npv=outer\nf\ndeclare -p pv\n",
    "f(){ local pv=1; unset pv; echo \"[${pv-gone}]\"; }\npv=outer\nf\necho \"$pv\"\n",
    "f(){ local pv=1; unset pv; declare -p pv; }\nf\ndeclare -p pv\n",
    /* Only the body that owns the local gets the invisible entry. */
    "f(){ local pv=1; g(){ unset pv; declare -p pv; }; g; declare -p pv; }\nf\n",
    "g(){ local pv=1; }\nf(){ g; unset pv; declare -p pv; }\nf\n",
    /* A global `unset` takes the entry away, declaration and all. */
    "declare pv=1\nunset pv\ndeclare -p pv\n",
    "declare -i n=1\nunset n\ndeclare -p n\n",
    "declare -a z\nunset z\ndeclare -p z\n",
    "x=1\nunset x\ndeclare -p x\n",
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

/// A bare declaration of a reserved name is a declaration, and the
/// reserved slot beside it is still not one.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn a_declaration_of_a_reserved_name_is_listed() {
    agrees(A_RESERVED_NAME);
}

/// `unset` leaves the local it was given, carrying nothing.
// [spec:nsh:req:compat.bash.functions-scoping/test]
#[test]
fn unset_leaves_an_invisible_local() {
    agrees(AN_UNSET_LOCAL);
}
