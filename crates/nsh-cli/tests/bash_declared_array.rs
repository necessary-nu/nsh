//! What a declared array spells back before anything is assigned to it,
//! measured against the pinned Bash 5.3.
//!
//! Bash keeps a name declared with `-a` or `-A` and never assigned
//! *invisible*: it has a kind and no value, so `declare -a z` spells
//! itself back as `declare -a z`, where `declare -a z=()` spells the
//! empty list it was handed. The two are one question asked in two
//! places, because `${name[@]@A}` answers out of the same renderer
//! `declare -p` does; a table here asks it both ways so the two cannot
//! be fixed apart.
//!
//! Nothing here is a recorded expectation. Every case runs in both
//! shells and the two answers are compared, so there is no literal to go
//! stale: if Bash changes its mind, this reports it rather than passing.
//! Diagnostic wording is a registered difference and only stdout and the
//! exit status are read, which is why a refused conversion appears here
//! for the declaration it leaves behind rather than for what it said.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// A declared array against an assigned empty one, through `declare -p`.
///
/// The scalar rows are the contrast: a name declared `-i` or `-r` and
/// never assigned already prints its declaration alone, and it is only
/// the array half of the distinction that was missing.
const DECLARED_AGAINST_EMPTY: &[&str] = &[
    "declare -a z\ndeclare -p z\n",
    "declare -A m\ndeclare -p m\n",
    "declare -a z=()\ndeclare -p z\n",
    "declare -A m=()\ndeclare -p m\n",
    "declare -i n\ndeclare -p n\n",
    "declare -x s\ndeclare -p s\n",
    /* The kind survives everything that adds an attribute beside it,
     * and none of those is an assignment. */
    "declare -a z\ndeclare -r z\ndeclare -p z\n",
    "declare -a z\ndeclare -x z\ndeclare -p z\n",
    "declare -a z\ndeclare -i z\ndeclare -p z\n",
    "declare -ar z\ndeclare -p z\n",
    "declare -a z\ndeclare -a z\ndeclare -p z\n",
    "declare -a z\ndeclare -p z\ndeclare -p z\n",
    /* A conversion Bash refuses leaves the declaration it refused to
     * change, invisible as it found it. */
    "declare -a z\ndeclare -A z\ndeclare -p z\n",
    "declare -A m\ndeclare -a m\ndeclare -p m\n",
    /* `local` and the declaring built-ins declare the same way. */
    "f() { local -a z; declare -p z; }\nf\n",
    "f() { declare -A m; declare -p m; }\nf\n",
    "readonly -a a\ndeclare -p a\n",
    "readonly -A m\ndeclare -p m\n",
    /* An invisible name is still a name: it is listed by `declare -p`
     * and not by `${!prefix@}`, which is about values. */
    "declare -a z\ndeclare -p | grep -c '^declare -a z$'\n",
    "declare -a z=()\ndeclare -p | grep -c '^declare -a z=()$'\n",
    "declare -A m\ndeclare -p | grep -c '^declare -A m$'\n",
    "declare -a z\ndeclare -pa | grep -c '^declare -a z$'\n",
    "declare -a z\necho \"[${!z@}]\"\n",
    "declare -a z=()\necho \"[${!z@}]\"\n",
];

/// The first assignment makes the name visible, and only an assignment
/// does.
///
/// `unset a[0]` on a declared array writes nothing and leaves it
/// declared; `read` and `mapfile` with nothing to read still assign, so
/// they leave the empty list behind.
const ASSIGNMENT_MAKES_IT_VISIBLE: &[&str] = &[
    "declare -a z\ndeclare -p z\nz=()\ndeclare -p z\n",
    "declare -A m\ndeclare -p m\nm=()\ndeclare -p m\n",
    "declare -a z\nz=q\ndeclare -p z\n",
    "declare -A m\nm=q\ndeclare -p m\n",
    "declare -a z\nz[0]=v\ndeclare -p z\n",
    "declare -a z\ndeclare -p z\nz+=(1)\ndeclare -p z\n",
    "declare -A m\ndeclare -p m\nm+=([k]=1)\ndeclare -p m\n",
    "f() { local -a z; declare -p z; z=(1); declare -p z; }\nf\n",
    "declare -a z\nread -a z </dev/null\ndeclare -p z\n",
    "declare -a z\nmapfile z </dev/null\ndeclare -p z\n",
    /* Taking an element away from a name that has none is not an
     * assignment, and the name stays declared. */
    "declare -a z\nunset z[0]\ndeclare -p z\nz[0]=v\ndeclare -p z\n",
    "declare -A m\nunset m[k]\ndeclare -p m\n",
    "declare -a z\nunset z[@]\ndeclare -p z\n",
    "declare -a z\nz[3]=x\nunset z[3]\ndeclare -p z\n",
    /* And an element that was never there does not become one. */
    "declare -a z\necho \"${#z[@]}\"\n[ -v z ] && echo v || echo nov\necho \"[${z[@]}]\"\n",
    "declare -a z\necho \"[${z-D}]\" \"[${z[0]-D}]\" \"[${z[@]+S}]\"\n",
];

/// `${name@A}` on a declared array, which has to say what `declare -p`
/// says.
///
/// The last three rows are the round trip: what the transform spells is
/// what recreates the name, so an invisible array has to come back
/// invisible and an assigned empty one has to come back assigned.
const SPELLED_BACK: &[&str] = &[
    "declare -a z\necho \"[${z@A}]\"\n",
    "declare -A m\necho \"[${m@A}]\"\n",
    "declare -a z\nprintf '[%s]' \"${z[@]@A}\"\necho\n",
    "declare -A m\nprintf '[%s]' \"${m[@]@A}\"\necho\n",
    "declare -a z=()\nprintf '[%s]' \"${z[@]@A}\"\necho\n",
    "declare -a z\necho \"[${z[*]@A}]\"\n",
    /* A subscript naming an element that is not there reads the same
     * way as an array with nothing in it: the declaration alone. */
    "declare -a z\necho \"[${z[0]@A}]\"\n",
    "declare -A m\necho \"[${m[k]@A}]\"\n",
    "declare -a z=(1 2)\necho \"[${z[5]@A}]\"\n",
    "declare -A m=([a]=1)\necho \"[${m[q]@A}]\"\n",
    "declare -a z\nt=\"${z[@]@A}\"\nunset z\neval \"$t\"\ndeclare -p z\n",
    "declare -A m\nt=\"${m[@]@A}\"\nunset m\neval \"$t\"\ndeclare -p m\n",
    "declare -a z=()\nt=\"${z[@]@A}\"\nunset z\neval \"$t\"\ndeclare -p z\n",
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

/// A declared array prints the declaration the reference prints, with
/// no empty list in it.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
// [spec:nsh:req:compat.bash.value-model/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn a_declared_array_prints_no_value() {
    agrees(DECLARED_AGAINST_EMPTY);
}

/// The name becomes visible where the reference makes it visible, and
/// stays declared where the reference leaves it declared.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
// [spec:nsh:req:compat.bash.value-model/test]
#[test]
fn only_an_assignment_gives_it_a_value() {
    agrees(ASSIGNMENT_MAKES_IT_VISIBLE);
}

/// `${name@A}` spells the declaration the reference spells, in the
/// fields the reference spells it in.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn the_transform_spells_the_declaration_too() {
    agrees(SPELLED_BACK);
}
