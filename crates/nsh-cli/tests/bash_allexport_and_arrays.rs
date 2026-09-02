//! Which names `set -a` exports, measured against the pinned Bash 5.3.
//!
//! The option was applied in one place -- the write that lands a value
//! -- and Bash applies it in two, on opposite sides of the same
//! question. It marks an assignment that stores a *scalar*, so a
//! compound one is never marked; and `declare` decides for itself,
//! marking a declaration that does not name an array whether or not it
//! has a value to store. This shell marked every write and no bare
//! declaration, so it was wrong in both directions at once: `set -a`
//! exported an array here and Bash never exports one, and it left
//! `declare -i n` unexported where Bash exports it.
//!
//! Nothing escapes into the environment either way -- an array has no
//! environment spelling, and `environment()` passes only a scalar -- so
//! it is the letter on the declaration that differs, which `declare -p`,
//! `${name[@]@A}` and `export -p` all print. That is why no survey saw
//! it.
//!
//! Nothing here is a recorded expectation. Every case runs in both
//! shells and the two answers are compared, so there is no literal to go
//! stale: if Bash changes its mind, this reports it rather than passing.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// An array is never exported by the option, however it is written.
const AN_ARRAY_IS_NOT_EXPORTED: &[&str] = &[
    "set -a; z=(1); declare -p z\n",
    "set -a; declare -a z=(); declare -p z\n",
    "set -a; declare -a z; declare -p z\n",
    "set -a; declare -A m=([k]=v); declare -p m\n",
    "set -a; declare -A m; declare -p m\n",
    "set -a; m=([k]=v); declare -p m\n",
    "set -a; z[0]=5; declare -p z\n",
    "set -a; declare -a z; z+=(1); declare -p z\n",
    "declare -a z=(9); set -a; z[1]=1; declare -p z\n",
    "declare -a z; set -a; z=(1); declare -p z\n",
    "set -a; declare -al z; declare -p z\n",
    "set -a; declare -A m; m[k]=v; declare -p m\n",
    "set -a; mapfile mv <<< hi; declare -p mv\n",
    /* Nor by a `declare` that names one Bash already knows about. */
    "set -a; declare -a z=(1); declare -i z; declare -p z\n",
    "set -a; declare -a z=(1); declare z; declare -p z\n",
    "set -a; declare -a z; declare z=q; declare -p z\n",
    "set -a; declare -a z; declare z=(9); declare -p z\n",
    "set -a; declare -a z; declare -r z=(1); declare -p z\n",
    "set -a; declare -A m; declare -r m=([k]=v); declare -p m\n",
    "set -a; declare -A m=([k]=v); declare -r m; declare -p m\n",
    /* A reference reaches the array, and the array is still not one the
     * option marks. */
    "set -a; declare -a z; declare -n r=z; declare -i r; declare -p z\n",
    /* An array holds nothing an environment could carry, either way. */
    "set -a; z=(1); env | grep -c '^z'\n",
    "set -a; declare -a z=(1 2); export -p | grep -c 'z='\n",
    "set -a; declare -a z=(1); echo \"${z[@]@A}\"\n",
    /* And the letter asked for outright is still applied. */
    "set -a; declare -x -a z=(1); declare -p z\n",
    "set -a; declare -a z; declare -x z; declare -p z\n",
    "set -a; export -a z=(1); declare -p z\n",
];

/// A declaration that does not name an array is exported by the option,
/// with or without a value to store.
const A_DECLARATION_IS_EXPORTED: &[&str] = &[
    "set -a; declare -i n; declare -p n\n",
    "set -a; declare s; declare -p s\n",
    "set -a; declare s=v; declare -p s\n",
    "set -a; declare -i n=1; declare -p n\n",
    "set -a; typeset -i n; declare -p n\n",
    "set -a; declare -r r=1; declare -p r\n",
    "set -a; declare -n rr=t; declare -p rr\n",
    "set -a; readonly -a z; declare -p z\n",
    "set -a; readonly q=1; declare -p q\n",
    "set -a; readonly zz; declare -p zz\n",
    "set -a; declare -r z=(1) w=2; declare -p z; declare -p w\n",
    /* `+x` still takes the option's letter straight back off. */
    "set -a; declare +x -i n; declare -p n\n",
    /* `-g` sends the declaration to the shell's own table, where the
     * option reaches it; a local one it does not reach. */
    "f() { set -a; declare -gi q; declare -p q; }; f; declare -p q\n",
    "f() { set -a; local -i q; declare -p q; }; f\n",
    "f() { set -a; declare -i q; declare -p q; }; f\n",
    "f() { set -a; local -a y=(1); declare -p y; }; f\n",
    /* A value goes through the ordinary assignment path, which is where
     * a *local* one picks the option up. */
    "f() { set -a; local x=1; declare -p x; }; f\n",
];

/// The plain assignment the option has always marked, which has to keep
/// being marked -- including onto a name that already holds an array.
const A_SCALAR_ASSIGNMENT_IS_EXPORTED: &[&str] = &[
    "set -a; n=1; declare -p n\n",
    "set -a; x=1; export -p | grep ' x='\n",
    "set -a; declare -a z; z=q; declare -p z\n",
    "set -a; declare -a y=(1); y=w; declare -p y\n",
    "set -a; ((n=5)); declare -p n\n",
    "set -a; let m=5; declare -p m\n",
    "set -a; read x <<< hi; declare -p x\n",
    "set -a; for i in 1; do :; done; declare -p i\n",
    "set -a; printf -v pv hi; declare -p pv\n",
    "set -a; getopts 'a' og; declare -p og\n",
    "set -a; z=(1); set +a; z2=(2); declare -p z; declare -p z2\n",
    "set -a; x=1; set +a; y=2; declare -p x; declare -p y\n",
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

/// `set -a` leaves an array unexported, as the reference leaves it.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn the_option_does_not_reach_an_array() {
    agrees(AN_ARRAY_IS_NOT_EXPORTED);
}

/// `set -a` exports a declaration that is not an array, as the reference
/// exports it.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
// [spec:nsh:req:compat.bash.functions-scoping/test]
#[test]
fn the_option_reaches_a_declaration() {
    agrees(A_DECLARATION_IS_EXPORTED);
}

/// The plain scalar assignment `set -a` has always exported still is.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn the_option_still_reaches_an_assignment() {
    agrees(A_SCALAR_ASSIGNMENT_IS_EXPORTED);
}
