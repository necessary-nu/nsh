//! How `readonly` and `export` list the names they carry, measured
//! against the pinned Bash 5.3.
//!
//! Bash prints declarations there -- the same `declare -p` line, out of
//! the same renderer -- where this shell printed the POSIX
//! `readonly NAME='value'` in both dialects. The POSIX dialect's form is
//! dash's and is right; it is Bash mode that had the wrong one, and an
//! array showed it worst: `declare -x xa=(1 2)` listed as `export xa='1'`,
//! which spells the first element and calls it the variable.
//!
//! `-a` and `-A` are not attributes in these two built-ins. Bash consults
//! them for a compound operand, and with no operand at all it spends them
//! on the listing instead: `readonly -a` names the read-only indexed
//! arrays and nothing else.
//!
//! Nothing here is a recorded expectation. Every case runs in both shells
//! and the two answers are compared. No row prints a whole listing: the
//! two shells do not carry the same set of read-only and exported names
//! -- Bash makes `EUID`, `UID`, `PPID` and `BASH_VERSINFO` read-only and
//! this shell does not -- so each row selects the names it declared.
//!
//! The POSIX dialect's half of this is pinned where it was already
//! pinned, in `tests/corpus/aud_state_var.txt`, which runs `export -p`
//! and `readonly -p` against the C dash reference.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// The form: a declaration, with every flag the name carries.
const LISTED_AS_DECLARATIONS: &[&str] = &[
    "readonly r=1\nreadonly -p | grep ' r'\n",
    "readonly r=1\nreadonly | grep ' r'\n",
    "export e=1\nexport -p | grep ' e'\n",
    "export e=1\nexport | grep ' e'\n",
    /* The line carries the flags the name has, not the one the listing
     * was selected by. */
    "declare -rx q=1\nreadonly -p | grep ' q'\nexport -p | grep ' q'\n",
    "declare -ri ri=5\nreadonly -p | grep ' ri'\n",
    /* An array is spelled as an array, where its first element used to
     * stand in for it. */
    "readonly -a ra=(1 2)\nreadonly -p | grep ' ra'\n",
    "declare -rA rm=([k]=v)\nreadonly -p | grep ' rm'\n",
    "declare -x xa=(1 2)\nexport -p | grep ' xa'\n",
    "declare -ax z\nexport -p | grep ' z'\n",
    "declare -ax z=()\nexport -p | grep ' z'\n",
    "declare -rA rm\nreadonly -p | grep ' rm'\n",
    /* A name with the attribute and no value prints the declaration
     * alone, as it does through `declare -p`. */
    "readonly ru\nreadonly -p | grep ' ru'\n",
    "export eu\nexport -p | grep ' eu'\n",
    /* The value is quoted so it reads back. */
    "export e='a b'\nexport -p | grep ' e'\n",
    "export e=\"a'b\"\nexport -p | grep ' e'\n",
    "export e=$'a\\tb'\nexport -p | grep ' e'\n",
    "export -a xa=(1 2)\nt=$(export -p | grep ' xa')\nunset xa\neval \"$t\"\ndeclare -p xa\n",
    /* A name without the attribute is not listed, and an empty listing
     * is still a success. */
    "x=1\nexport -p | grep -c ' x'\n",
    "readonly -p > /dev/null\necho rc=$?\nexport -p > /dev/null\necho rc=$?\n",
];

/// The letters, which with no operand select rather than declare.
const THE_ARRAY_LETTERS_SELECT: &[&str] = &[
    "readonly -a ra=(1)\nreadonly -a | grep ' ra'\n",
    "readonly -a ra=(1)\nreadonly r2=x\nreadonly -a | grep -c 'r2'\n",
    "readonly -a ra=(1)\nreadonly -A | grep -c 'ra'\n",
    "readonly -a ra=(1)\nreadonly -pa | grep ' ra'\n",
    "declare -rA rm=([k]=v)\nreadonly -A | grep ' rm'\n",
    "declare -rA rm=([k]=v)\nreadonly -a | grep -c 'rm'\n",
    "export -a xa=(1)\nexport -a | grep ' xa'\n",
    "declare -xA xm=([k]=v)\nexport -A | grep ' xm'\n",
    "declare -xA xm=([k]=v)\nexport -a | grep -c 'xm'\n",
    /* A scalar is neither kind, so both letters leave it out. */
    "readonly r=1\nreadonly -a | grep -c ' r'\n",
    "export e=1\nexport -A | grep -c ' e'\n",
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

/// `readonly` and `export` list what the reference lists, in the lines
/// the reference writes.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn the_listing_is_a_declaration_in_bash_mode() {
    agrees(LISTED_AS_DECLARATIONS);
}

/// `-a` and `-A` with no operand narrow the listing as the reference
/// narrows it.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn the_array_letters_narrow_the_listing() {
    agrees(THE_ARRAY_LETTERS_SELECT);
}
