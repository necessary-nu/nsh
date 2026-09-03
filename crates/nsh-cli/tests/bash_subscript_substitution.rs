//! A command substitution written inside an array subscript, measured
//! against the pinned Bash 5.3.
//!
//! A subscript's bytes are re-parsed at expansion time -- the parser
//! scans to the matching `]` and `variables::arrays` reads what it found
//! as a word -- and that re-parse runs on top of whatever parser state
//! the caller was in. A pushed-back token is part of that state and
//! belongs to the source the caller was reading, so a string-fed shell,
//! whose outer parse has already pushed `Eof` back, replayed it into the
//! `$( )` inside the subscript: `${a[$(echo 1)]}` reported
//! `end of file unexpected` and `${a[`echo 1`]}` expanded to nothing and
//! silently read element zero.
//!
//! WHICH IS WHY EVERY CASE HERE RUNS THROUGH `-c` AND `eval`. Fed the
//! same script on standard input both spellings always worked, because
//! the outer parse had not reached its end: a table built on the shared
//! stdin helper would have been green against the defect. `-c` is the
//! shape a script actually meets it in.
//!
//! EVERY CASE IS BASH MODE, and that is a boundary rather than a
//! convenience. dash has the same defect -- `dash -c 'PS4="$(echo P)+ ";
//! set -x; echo hi'` traces with the unexpanded text -- so the POSIX
//! dialect keeps it, and `errors_are_values.rs` pins dash's answer
//! there. `state-a-re-entered-parse-starts-clean` holds that half.
//!
//! Nothing here is a recorded expectation. Every case runs in both
//! shells and the two answers are compared, so there is no literal to go
//! stale: if Bash changes its mind, this reports it rather than passing.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// Both spellings, on both sides of an assignment, on both array kinds.
const A_SUBSTITUTION_IN_A_SUBSCRIPT: &[&str] = &[
    "declare -a a=(10 11 12); echo \"${a[$(echo 1)]}\"",
    "declare -a a=(10 11 12); echo \"${a[`echo 1`]}\"",
    "declare -a a=(10 11 12); echo \"${a[$(echo 1) + 1]}\"",
    "declare -A m=([k]=v [j]=w); echo \"${m[$(echo k)]}\"",
    "declare -A m=([k]=v [j]=w); echo \"${m[`echo j`]}\"",
    "declare -a a=(1 2 3); a[$(echo 1)]=v; declare -p a",
    "declare -a a=(1 2 3); a[`echo 1`]=v; declare -p a",
    "declare -A m; m[$(echo k)]=v; declare -p m",
    "declare -A m; m[`echo k`]=v; declare -p m",
    "declare -a b=([$(echo 2)]=x); declare -p b",
    "declare -A m=([$(echo k)]=v); declare -p m",
    /* The other readers of a subscript's bytes. */
    "declare -a a=(1 2 3); echo ${#a[$(echo 1)]}",
    "declare -a a=(1 2 3); unset \"a[$(echo 0)]\"; declare -p a",
    "declare -a a=(1 2 3); echo \"${a[$(echo 1)]:-none}\"",
    "declare -a a=(1 2 3); (( a[$(echo 1)] = 9 )); declare -p a",
    "declare -a a=(5 6); echo $(( a[$(echo 1)] ))",
    /* Nested expansions, blanks and two substitutions in one word. */
    "n=1; declare -a a=(1 2 3); echo \"${a[$(echo $n)]}\"",
    "declare -a a=(1 2 3); echo \"${a[$( echo 2 )]}\"",
    "declare -a a=(1 2 3); echo \"${a[$(echo 1)]}${a[`echo 2`]}\"",
];

/// The same subscripts reached through `eval`, which re-enters the
/// parser the way `-c` does from inside a running script.
const A_SUBSTITUTION_UNDER_EVAL: &[&str] = &[
    "a=(9 8 7)\neval 'echo \"${a[$(echo 1)]}\"'\n",
    "a=(9 8 7)\neval 'echo \"${a[`echo 2`]}\"'\n",
    "declare -A m=([k]=v)\neval 'echo \"${m[$(echo k)]}\"'\n",
    "a=(9 8 7)\neval 'a[$(echo 0)]=z'\ndeclare -p a\n",
    /* The prompt expansion runs through the same re-entry, and a `$( )`
     * in `PS4` under `set -x` is where a script meets it. */
    "PS4=\"$(echo P)\"\nset -x\necho hi\n",
];

/// What one shell said: its standard output and its exit status.
type Answer = (Vec<u8>, i32);

/// One script through both shells as a `-c` argument, as `(what nsh
/// said, what the pinned Bash said)`.
fn both_as_argument(script: &str) -> (Answer, Answer) {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let bash = pinned_bash::path();
    (
        pinned_bash::answer(nsh, &["-o", "bash", "-c", script], ""),
        pinned_bash::answer(&bash, &["-c", script], ""),
    )
}

/// One script through both shells on standard input.
fn both_on_input(script: &str) -> (Answer, Answer) {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let bash = pinned_bash::path();
    (
        pinned_bash::answer(nsh, &["-o", "bash"], script),
        pinned_bash::answer(&bash, &[], script),
    )
}

/// Every script in `cases` produces the reference's bytes and status.
fn agrees(cases: &[&str], run: fn(&str) -> (Answer, Answer)) {
    for script in cases {
        let (ours, theirs) = run(script);
        assert_eq!(
            String::from_utf8_lossy(&ours.0),
            String::from_utf8_lossy(&theirs.0),
            "output differed for\n{script}"
        );
        assert_eq!(ours.1, theirs.1, "status differed for\n{script}");
    }
}

/// A substitution inside a subscript reads the element the reference
/// reads, whichever way the script was handed to the shell.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
// [spec:nsh:def:idiom.token-stream/test]
#[test]
fn a_substitution_in_a_subscript_is_expanded() {
    agrees(A_SUBSTITUTION_IN_A_SUBSCRIPT, both_as_argument);
    agrees(A_SUBSTITUTION_IN_A_SUBSCRIPT, both_on_input);
}

/// A re-entered parse does not inherit the outer one's pushed-back
/// token.
// [spec:nsh:def:idiom.token-stream/test]
#[test]
fn a_re_entered_parse_reads_its_own_source() {
    agrees(A_SUBSTITUTION_UNDER_EVAL, both_on_input);
    agrees(A_SUBSTITUTION_UNDER_EVAL, both_as_argument);
}
