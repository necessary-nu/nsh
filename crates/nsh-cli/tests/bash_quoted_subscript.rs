//! A quoted subscript, measured against the pinned Bash 5.3.
//!
//! An indexed array's subscript is an arithmetic expression, and single
//! quotes mean nothing in arithmetic: Bash reports `${a['1']}` as a
//! syntax error where this shell read element one. A *double*-quoted
//! subscript is accepted by both, because Bash expands the subscript's
//! text as if it were inside double quotes before evaluating it -- which
//! is also why `${a['$n']}` reports the error token as `'1'` and not as
//! `'$n'`: the expansion ran inside the quotes and the quotes themselves
//! survived it.
//!
//! An *associative* subscript is a key and not an expression, and none
//! of this touches it: `m['a b']` is the key `a b` in both shells.
//!
//! THE NODE'S DIAGNOSIS WAS HALF OF IT. `variables::arrays::text_word`
//! did strip the quotes, but putting them back was not enough: the
//! arithmetic lexer skipped `'` as though it were a blank, so `$(( '1'
//! ))` was 1 and `(( i = '3' ))` assigned 3. Both halves are here.
//!
//! Diagnostic wording is a registered difference, so only stdout and the
//! exit status are read -- but *whether* a case reports is still
//! measured, in the status and in what the commands after it print. An
//! associative array's iteration order is its hash order and the two
//! shells do not share it, so no row prints more than one key.
//!
//! Nothing here is a recorded expectation. Every case runs in both
//! shells and the two answers are compared.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// A single-quoted index, through every construct that names one.
///
/// Each row runs a command after the failure, so a difference in what
/// the shell abandons shows up in stdout even though the diagnostic
/// itself is not compared.
const A_SINGLE_QUOTED_INDEX_REPORTS: &[&str] = &[
    "declare -a a=(10 11 12)\necho \"[${a['1']}]\"\necho after=$?\n",
    "declare -a a=(10 11 12)\na['1']=z\necho after=$?\ndeclare -p a\n",
    "declare -a a=(10 11 12)\na['1']+=z\necho after=$?\ndeclare -p a\n",
    "declare -a a=(10 11 12)\necho $(( a['1'] ))\necho after=$?\n",
    "declare -a a=(10 11 12)\nunset -v \"a['1']\"\necho after=$?\ndeclare -p a\n",
    "declare -a a=(10 11 12)\nunset \"a['1']\"\necho after=$?\ndeclare -p a\n",
    "declare -a a=(10 11 12)\nn=1\necho \"[${a['n']}]\"\necho after=$?\n",
    "declare -a a=(10 11 12)\nn=1\necho \"[${a['$n']}]\"\necho after=$?\n",
    "declare -a a=(10 11 12)\necho \"[${a[1+'1']}]\"\necho after=$?\n",
    "declare -a a=(10 11 12)\necho \"[${a['']}]\"\necho after=$?\n",
    "declare -a a=(10 11 12)\necho \"[${a[\"'1'\"]}]\"\necho after=$?\n",
    "declare -a a=(10 11 12)\ndeclare -n r=\"a['1']\"\necho \"[$r]\"\necho after=$?\n",
    /* A name that does not exist defaults to indexed, so it reports
     * too rather than reading nothing. */
    "echo \"[${zz['1']}]\"\necho after=$?\n",
    "echo \"[${zz[1]}]\"\necho after=$?\n",
    /* A backslash is not an escape in that text either: it survives
     * into the expression and is rejected with it. */
    "declare -a a=(10 11 12)\necho \"[${a[\\'1\\']}]\"\necho after=$?\n",
    "declare -a a=(10 11 12)\necho \"[${a[\\1]}]\"\necho after=$?\n",
    "declare -a a=(10 11 12)\nn=1\necho \"[${a[\\$n]}]\"\necho after=$?\n",
    "declare -a a=(10 11 12)\necho \"[${a['a'b]}]\"\necho after=$?\n",
    "declare -a a=(10 11 12)\necho \"[${a[a'b']}]\"\necho after=$?\n",
];

/// The shapes that must not move.
const THE_SHAPES_THAT_MUST_NOT_MOVE: &[&str] = &[
    // A double-quoted index is accepted, and so is an expansion.
    "declare -a a=(10 11 12)\necho \"[${a[\"1\"]}]\"\n",
    "declare -a a=(10 11 12)\necho \"[${a[\"1\"+1]}]\"\n",
    "declare -a a=(10 11 12)\nn=1\necho \"[${a[$n]}]\"\n",
    "declare -a a=(10 11 12)\nn=1\necho \"[${a[\"$n\"]}]\"\n",
    "declare -a a=(10 11 12)\necho \"[${a[\"\"]}]\"\n",
    "declare -a a=(10 11 12)\necho \"[${a[ 1 ]}]\"\n",
    "declare -a a=(10 11 12)\na[\"1\"]=z\ndeclare -p a\n",
    "declare -a a=(10 11 12)\nn=1\na[$n]=z\ndeclare -p a\n",
    "declare -a a=(10 11 12)\nunset -v 'a[1]'\ndeclare -p a\n",
    "declare -a a=(10 11 12)\ndeclare -n r='a[1]'\necho \"[$r]\"\n",
    "declare -a a=(10 11 12)\necho \"[${a[@]}]\" \"[${a[*]}]\"\n",
    "declare -a a=(10 11 12)\necho $(( a[1] ))\n",
    "declare -a a=(10 11)\na[1]='q'\ndeclare -p a\n",
    // An associative subscript is a key: its quotes come off.
    "declare -A m=([k]=v)\necho \"[${m['k']}]\"\n",
    "declare -A m=([k]=v)\necho \"[${m[\"k\"]}]\"\n",
    "declare -A m=([a b]=sp)\necho \"[${m['a b']}]\"\n",
    "declare -A m=([a=1]=eq)\necho \"[${m['a=1']}]\"\n",
    "declare -A m=([k]=v)\nm['k']=w\necho \"[${m[k]}]\"\n",
    "declare -A m\nm['a b']=w\necho \"[${m['a b']}]\"\n",
    "declare -A m\nk='a b'\nm[$k]=w\necho \"[${m['a b']}]\"\n",
    "declare -A m=([k]=v)\nunset -v \"m['k']\"\necho \"${#m[@]}\"\n",
    "declare -A m=([k]=v)\ndeclare -n r=\"m['k']\"\necho \"[$r]\"\n",
    "declare -A c=(['a b']=w)\necho \"[${c['a b']}]\"\n",
    // A compound element's subscript is a word in Bash too, so its
    // quotes come off on both sides of the `=`.
    "declare -a b=(['1']=x)\ndeclare -p b\n",
    "declare -a b=([\"1\"]=x)\ndeclare -p b\n",
    "b=(['1']=x)\ndeclare -p b\n",
    "declare -A c=(['k']=x)\necho \"[${c[k]}]\"\n",
];

/// The arithmetic evaluator itself, which is where the quotes were being
/// skipped.
const THE_EVALUATOR_REJECTS_A_QUOTE: &[&str] = &[
    "i=0\n(( i = '3' ))\necho after=$? i=$i\n",
    "echo $(( '1' ))\necho after=$?\n",
    "echo $(( 1 + '1' ))\necho after=$?\n",
    "echo $(( 'a' ))\necho after=$?\n",
    "i=0\n(( i = \"3\" ))\necho after=$? i=$i\n",
    "echo $(( \"1\" ))\necho after=$?\n",
    "echo $(( \"1\" + 1 ))\necho after=$?\n",
    "x=5\necho $(( \"x\" ))\necho after=$?\n",
    /* `let` is handed a word, so the shell removes the quotes before
     * the evaluator ever sees them and `let i='4'` is 4. */
    "i=0\nlet i='4'\necho after=$? i=$i\n",
    "i=0\nlet i=\"4\"\necho after=$? i=$i\n",
    "declare -a a=(10 11 12)\necho $(( a[\"1\"] ))\n",
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

/// A single-quoted index reports where the reference reports, through
/// every construct that names one.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
// [spec:nsh:req:compat.bash.error-boundary/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn a_single_quoted_index_is_an_arithmetic_error() {
    agrees(A_SINGLE_QUOTED_INDEX_REPORTS);
}

/// The subscripts that were already right are unmoved.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn a_key_and_a_quoted_index_are_unmoved() {
    agrees(THE_SHAPES_THAT_MUST_NOT_MOVE);
}

/// The arithmetic evaluator rejects a single quote and accepts a double
/// one, as the reference does.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn the_evaluator_takes_the_quotes_the_reference_takes() {
    agrees(THE_EVALUATOR_REJECTS_A_QUOTE);
}
