//! What a bare `set` and a bare `declare` list, measured against the
//! pinned Bash 5.3.
//!
//! `dea4a93` gave `export -p` and `readonly -p` the declaration renderer
//! and deliberately left this listing alone. It was wrong in two ways at
//! once. It quoted unconditionally -- `x='1'` where Bash prints `x=1` --
//! and it read an array through `Variable::scalar`, which is element
//! zero, so a two-element array listed as its first element and an
//! associative array with no `"0"` key listed as a bare name.
//!
//! THE DIALECT BOUNDARY IS THE POINT OF THIS FILE'S CARE. `set` is a
//! POSIX built-in and the POSIX form is dash's, which quotes every value
//! and has no arrays to spell. Only the Bash branch moved, and the POSIX
//! branch is pinned where it already was, in
//! `tests/corpus/aud_state_var.txt` against the C dash reference.
//!
//! Nothing here is a recorded expectation. Every case runs in both
//! shells and the two answers are compared, so there is no literal to go
//! stale: if Bash changes its mind, this reports it rather than passing.
//! Diagnostic wording is a registered difference, so only stdout and the
//! exit status are read.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// A value that needs no quoting is printed bare, and one that does is
/// printed the way `${x@Q}` prints it.
///
/// The `$'...'` rows are the ordering: `set` reaches for it even though
/// a tab is also a metacharacter, where `set -x` on the same bytes
/// reaches for single quotes. The two are opposite and both are Bash's.
const QUOTED_ONLY_WHEN_NEEDED: &[&str] = &[
    "v=1\nset | grep '^v='\n",
    "v=abc\nset | grep '^v='\n",
    "v=\nset | grep '^v='\n",
    "v=-x\nset | grep '^v='\n",
    "v=a=b\nset | grep '^v='\n",
    "v=a#b\nset | grep '^v='\n",
    "v=a%b\nset | grep '^v='\n",
    "v=a+b,c.d/e:f@g_h\nset | grep '^v='\n",
    "v=a~b\nset | grep '^v='\n",
    "v=ab~\nset | grep '^v='\n",
    // And the ones that do need it.
    "v='a b'\nset | grep '^v='\n",
    "v='a$b'\nset | grep '^v='\n",
    "v='a\"b'\nset | grep '^v='\n",
    "v=\"a'b\"\nset | grep '^v='\n",
    "v='a\\\\b'\nset | grep '^v='\n",
    "v='a*b'\nset | grep '^v='\n",
    "v='a;b'\nset | grep '^v='\n",
    "v='a|b'\nset | grep '^v='\n",
    "v='a&b'\nset | grep '^v='\n",
    "v='a!b'\nset | grep '^v='\n",
    "v='a^b'\nset | grep '^v='\n",
    "v='a[b]'\nset | grep '^v='\n",
    "v='a{b}'\nset | grep '^v='\n",
    "v='a(b)'\nset | grep '^v='\n",
    "v='a<b>c'\nset | grep '^v='\n",
    "v='a?b'\nset | grep '^v='\n",
    "v='a`b'\nset | grep '^v='\n",
    // `~` and `#` are metacharacters only where they could expand.
    "v='~x'\nset | grep '^v='\n",
    "v='#x'\nset | grep '^v='\n",
    "v='a=~b'\nset | grep '^v='\n",
    "v='a:~b'\nset | grep '^v='\n",
    // A byte with no printable glyph takes `$'...'`, metacharacter or
    // not, and takes it even beside one.
    "v=$'a\\tb'\nset | grep '^v='\n",
    "v=$'a\\nb'\nset | grep '^v='\n",
    "v=$'\\001'\nset | grep '^v='\n",
    "v=$'a\\001 b'\nset | grep '^v='\n",
    "v=$'\\xff'\nset | grep '^v='\n",
    "v=héllo\nset | grep '^v='\n",
    // The same value through `set -x`, whose order is the other way
    // round and must not have moved.
    "v=$'a\\tb'\nset -x\n: \"$v\"\n",
    "v=abc\nset -x\n: \"$v\"\n",
];

/// An array lists the compound assignment that would rebuild it.
const AN_ARRAY_LISTS_ITS_ELEMENTS: &[&str] = &[
    "declare -a z=(1 2)\nset | grep '^z='\n",
    "declare -a z=()\nset | grep '^z='\n",
    "declare -a z=([3]=x)\nset | grep '^z='\n",
    "declare -a z=(1)\nz[9]=2\nset | grep '^z='\n",
    "declare -A m=([k]=v)\nset | grep '^m='\n",
    "declare -A m=()\nset | grep '^m='\n",
    "declare -A m=([a b]='c d')\nset | grep '^m='\n",
    "declare -A m=([k]='a b')\nset | grep '^m='\n",
    "declare -a z=('a b' 'c$d')\nset | grep '^z='\n",
    "declare -a z=($'a\\tb')\nset | grep '^z='\n",
    "declare -i n=3\nset | grep '^n='\n",
    "declare -ai z=(1 2)\nset | grep '^z='\n",
    "declare -r z=q\nset | grep '^z='\n",
    // A declared array holds nothing, so the listing has nothing to
    // print: `set` lists what has a value.
    "declare -a z\nset | grep -c '^z'\n",
    "declare -A m\nset | grep -c '^m'\n",
    "declare -i n\nset | grep -c '^n'\n",
    "readonly x\nset | grep -c '^x'\n",
    // The listing reads back as itself.
    "declare -a z=(1 2)\nt=$(set | grep '^z=')\nunset z\neval \"z=${t#z=}\"\ndeclare -p z\n",
    "declare -A m=([k]=v)\nt=$(set | grep '^m=')\nunset m\ndeclare -A m\neval \"m=${t#m=}\"\ndeclare -p m\n",
];

/// A bare `declare` is the same listing, and `declare -p` is not.
const A_BARE_DECLARE_IS_THE_SAME_LISTING: &[&str] = &[
    "declare -a z=(1 2)\ndeclare | grep '^z='\n",
    "declare -A m=([k]=v)\ndeclare | grep '^m='\n",
    "v=1\ndeclare | grep '^v='\n",
    "v='a b'\ndeclare | grep '^v='\n",
    "v=1\ndeclare -p v\n",
    "declare -a z=(1 2)\ndeclare -p z\n",
    "v=$'a\\tb'\ndeclare -p v\n",
    "v=1\ndeclare | grep -c '^v=1$'\ndeclare -p | grep -c '^declare -- v=\"1\"$'\n",
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

/// Every single-byte value, in the middle of a word and at its start,
/// through the listing.
///
/// The predicate is a table of characters with two positional cases in
/// it, and a table is what finds those: `~` is a metacharacter at the
/// start of a value and after `=` or `:` and nowhere else, `#` only at
/// the start, and `,` is not one at all despite reading like brace
/// expansion.
fn every_byte_in_a_value() -> Vec<String> {
    (1..=127)
        .flat_map(|byte| {
            [
                format!("v=$'a\\{byte:03o}b'\nset | grep '^v='\n"),
                format!("v=$'\\{byte:03o}ab'\nset | grep '^v='\n"),
            ]
        })
        .collect()
}

/// A value is quoted where the reference quotes it and bare where the
/// reference leaves it bare.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn the_listing_quotes_only_what_needs_it() {
    agrees(QUOTED_ONLY_WHEN_NEEDED);
}

/// An array lists the elements the reference lists, not its first one.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn an_array_lists_what_would_rebuild_it() {
    agrees(AN_ARRAY_LISTS_ITS_ELEMENTS);
}

/// A bare `declare` lists what the reference's bare `declare` lists.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn a_bare_declare_lists_the_same_thing() {
    agrees(A_BARE_DECLARE_IS_THE_SAME_LISTING);
}

/// The character table is the reference's, byte for byte and position
/// for position.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn the_metacharacter_table_is_the_references() {
    let cases = every_byte_in_a_value();
    let borrowed: Vec<&str> = cases.iter().map(String::as_str).collect();
    agrees(&borrowed);
}
