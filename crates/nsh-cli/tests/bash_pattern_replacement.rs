//! `${v/pattern/replacement}` and its three other spellings, run against
//! the pinned Bash rather than against a recorded answer.
//!
//! The `parameter` fuzz target found `${X/$P/$R}` writing different bytes
//! from the reference, and reducing it produced a rule this shell did not
//! have: Bash 5.2 gave `&` in a replacement the meaning `sed` gives it,
//! the text the pattern matched. Reducing it further produced three more
//! -- which backslash and which quoting make `&` a literal one, that a
//! replacement expands with the surrounding double quotes taken off, and
//! that the `#` or `%` anchor is read off the *expanded* pattern and only
//! where the global `/` has not already taken its place. A fourth came
//! out of writing the table down: `nocasematch` reaches this operator,
//! which is the only parameter operator it reaches.
//!
//! Nothing here is a recorded expectation. Every case runs in both shells
//! and the two answers are compared, so there is no literal to go stale:
//! if Bash changes its mind, this reports it rather than passing.
//!
//! Two neighbouring differences are measured and deliberately not in the
//! table, because they belong to the matcher rather than to the
//! replacement and are open work:
//!
//!   * an empty value with a pattern whose first byte is `*` --
//!     `e=''; ${e/*/X}` is `X` in Bash and empty here, while
//!     `${e/?(z)/X}` is empty in both even though `?(z)` matches the
//!     empty string. Bash's `match_pattern_char` answers `*pat == '*'`
//!     for an empty subject, so the reference's own answer turns on the
//!     first byte of the pattern rather than on what it matches;
//!   * a pattern that matches the empty string mid-value --
//!     `v=abc; ${v//?(z)/X}` is `XaXbXc` in Bash and `abc` here, because
//!     the loop below skips a zero-width match instead of writing the
//!     replacement and stepping one character.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// The values every case in the table reads.
///
/// `HOME` is written rather than inherited because the runs clear the
/// environment, and a tilde with no `HOME` sends the two shells to
/// different places to look for one.
const PRELUDE: &str = r##"shopt -s extglob
HOME=/hh
v=abcabc
e=''
h='#a#'
y='%a%'
s='a/b'
n=ABC
a=(one two)
P=b
S='*b'
Q='#'
Z='%a'
U=''
R='<&>'
B='<\&>'
T='x\\y'
set -- p q
"##;

/// Every case, as the line of shell that prints it.
///
/// The reduction of the artifact is the first of them: a value, a pattern
/// made of metacharacters and a replacement carrying an `&`.
const CASES: &[&str] = &[
    /* The artifact, reduced. Bash writes `[<abcabc>]` and this shell
     * wrote `[<&>]` until the rule below was implemented. */
    r##"X=abcabc P='*' W='<&>'; printf '[%s]' "${X/$P/$W}""##,
    /* `&` names the matched span in each of the four spellings, under a
     * literal pattern and under one made of metacharacters. */
    r##"printf '[%s]' "${v/b/<&>}""##,
    r##"printf '[%s]' "${v//b/<&>}""##,
    r##"printf '[%s]' "${v/#a/<&>}""##,
    r##"printf '[%s]' "${v/%c/<&>}""##,
    r##"printf '[%s]' "${v/$S/<&>}""##,
    r##"printf '[%s]' "${v//$S/<&>}""##,
    r##"printf '[%s]' "${v/#$S/<&>}""##,
    r##"printf '[%s]' "${v/%b*/<&>}""##,
    r##"printf '[%s]' "${v/$P/$R}""##,
    r##"printf '[%s]' "${v//$P/$R}""##,
    r##"printf '[%s]' "${v/#$P/$R}""##,
    r##"printf '[%s]' "${v/%$P/$R}""##,
    r##"printf '[%s]' ${v/b/<&>}"##,
    r##"printf '[%s]' "${v//?/<&>}""##,
    r##"printf '[%s]' "${v//[abc]/<&>}""##,
    r##"printf '[%s]' "${v//*/<&>}""##,
    r##"printf '[%s]' "${v/*/<&>}""##,
    r##"printf '[%s]' "${v//+(b)/<&>}""##,
    r##"printf '[%s]' "${v//@(b|c)/<&>}""##,
    r##"printf '[%s]' "${a[@]/o/<&>}""##,
    r##"printf '[%s]' "${a[*]/o/<&>}""##,
    r##"printf '[%s]' "${a[@]//o/<&>}""##,
    /* `nocasematch` reaches this operator and no other one: Bash names
     * the pattern substitution expansions beside `case` and `[[ ]]`. */
    r##"shopt -s nocasematch; printf '[%s]' "${n/b/<&>}""##,
    r##"shopt -s nocasematch; printf '[%s]' "${n//b/<&>}""##,
    r##"shopt -s nocasematch; printf '[%s]' "${n/#a/<&>}""##,
    r##"shopt -s nocasematch; printf '[%s]' "${n/%c/<&>}""##,
    r##"shopt -s nocasematch; printf '[%s]' "${n#a}${n##a}${n%c}${n%%c}""##,
    r##"shopt -s nocasematch; printf '[%s]' "${n^b}${n,b}""##,
    /* Which `&` is a literal one. The replacement's own quoting makes it
     * so and the expansion's does not, and a backslash a value carries
     * quotes it the way `sed` reads a backslash. */
    r##"printf '[%s]' "${v/b/\&}""##,
    r##"printf '[%s]' "${v/b/"&"}""##,
    r##"printf '[%s]' "${v/b/'&'}""##,
    r##"printf '[%s]' "${v/b/"$R"}""##,
    r##"printf '[%s]' "${v/b/$B}""##,
    r##"printf '[%s]' "${v/b/$T}""##,
    r##"printf '[%s]' "${v/b/&&}""##,
    r##"printf '[%s]' "${v//b/&}""##,
    r##"printf '[%s]' "${v/b/$(printf '<&>')}""##,
    /* A replacement expands with the surrounding double quotes off, so
     * the tilde is a home directory and `$@` joins rather than making
     * fields of its own. */
    r##"printf '[%s]' "${v/b/~}""##,
    r##"printf '[%s]' "${v/b/$@}""##,
    r##"printf '[%s]' "${v/b/$*}""##,
    r##"IFS=x; printf '[%s]' "${v/b/$@}""##,
    r##"IFS=x; printf '[%s]' "${v/b/$*}""##,
    /* The anchor is the first byte of the expanded pattern, it has to be
     * unquoted, and the global spelling has no room for one. */
    r##"printf '[%s]' "${h/$Q/X}""##,
    r##"printf '[%s]' "${h//$Q/X}""##,
    r##"printf '[%s]' "${h/#$Q/X}""##,
    r##"printf '[%s]' "${h/%$Q/X}""##,
    r##"printf '[%s]' "${h/"#"/X}""##,
    r##"printf '[%s]' "${h/\#/X}""##,
    r##"printf '[%s]' "${h/'#'/X}""##,
    r##"printf '[%s]' "${h/##/X}""##,
    r##"printf '[%s]' "${h//#a/X}""##,
    r##"printf '[%s]' "${h/#a/X}""##,
    r##"printf '[%s]' "${v//#a/X}""##,
    r##"printf '[%s]' "${v//%c/X}""##,
    r##"printf '[%s]' "${y/$Z/X}""##,
    r##"printf '[%s]' "${y//$Z/X}""##,
    /* A pattern with no bytes: the anchored spellings put the
     * replacement at their end of the value with an empty span behind
     * the `&`, and the rest leave the value alone. An unset parameter
     * has no value to put one beside. */
    r##"printf '[%s]' "${v/#/<&>}""##,
    r##"printf '[%s]' "${v/%/<&>}""##,
    r##"printf '[%s]' "${v//#/<&>}""##,
    r##"printf '[%s]' "${v///<&>}""##,
    r##"printf '[%s]' "${v/#/}""##,
    r##"printf '[%s]' "${v/#}""##,
    r##"printf '[%s]' "${v/%}""##,
    r##"printf '[%s]' "${v/$U/X}""##,
    r##"printf '[%s]' "${v/#$U/X}""##,
    r##"printf '[%s]' "${v/%$U/X}""##,
    r##"printf '[%s]' "${e/#/X}""##,
    r##"printf '[%s]' "${e/%/X}""##,
    r##"printf '[%s]' "${z/#/X}""##,
    r##"printf '[%s]' "${z/%/X}""##,
    /* A pattern that matches nothing, and the slash the first byte of a
     * pattern is allowed to be. */
    r##"printf '[%s]' "${v/x/<&>}""##,
    r##"printf '[%s]' "${v//x/<&>}""##,
    r##"printf '[%s]' "${v/#x/<&>}""##,
    r##"printf '[%s]' "${v/%x/<&>}""##,
    r##"printf '[%s]' "${s///}""##,
    r##"printf '[%s]' "${s///-}""##,
    r##"printf '[%s]' "${s//\//-}""##,
    r##"printf '[%s]' "${s/\//-}""##,
];

/// Both shells on one case, as `(what nsh said, what the pinned Bash said)`.
fn both(case: &str) -> ((Vec<u8>, i32), (Vec<u8>, i32)) {
    let script = format!("{PRELUDE}{case}\n");
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let bash = pinned_bash::path();
    (
        pinned_bash::answer(nsh, &["-o", "bash"], &script),
        pinned_bash::answer(&bash, &[], &script),
    )
}

/// Each spelling of pattern replacement writes the reference's bytes.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn the_replacement_table_is_the_references_own() {
    for case in CASES {
        let (ours, theirs) = both(case);
        assert_eq!(
            String::from_utf8_lossy(&ours.0),
            String::from_utf8_lossy(&theirs.0),
            "`{case}` printed different bytes"
        );
        assert_eq!(ours.1, theirs.1, "status differed on `{case}`");
    }
}
