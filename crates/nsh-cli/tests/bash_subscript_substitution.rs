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
//! THE SUBSCRIPT CASES ARE BASH MODE BECAUSE A SUBSCRIPT IS, not because
//! the repair is. `bash.divergences.re-entered-parse` measured the other
//! four `expand_string` callers and ungated it: a pushed-back token belongs
//! to the caller's source rather than to either grammar, so both dialects
//! start the re-entered parse clean. `the_two_dialects_re_enter_alike`
//! below is that half, and `errors_are_values.rs` holds what the POSIX
//! dialect answers on its own. dash still replays the token -- registered
//! as `re_entered_prompt_substitution` in `docs/divergences.md`.
//!
//! Nothing here is a recorded expectation. Every case runs through two
//! shells and the two answers are compared, so there is no literal to go
//! stale: if Bash changes its mind, this reports it rather than passing.
//! The last test's pair is this shell's own two dialects rather than
//! nsh against Bash, because what it asserts is that the dialect does not
//! decide -- and dash, which is the other reference, still answers
//! differently on purpose.

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

/// A prompt written in the language both dialects share, so the dialect
/// cannot be what decides the answer. Single-quoted throughout: a
/// double-quoted `PS4="$(echo P)"` is expanded by the *assignment* and
/// never re-enters the parser, which is a case that cannot fail.
const A_SUBSTITUTION_IN_A_PROMPT: &[&str] = &[
    "PS4='$(echo P)+ '; set -x; echo hi",
    "PS4='[`echo P`] '; set -x; echo hi",
    "PS4='$(exit 7)+ '; set -x; echo hi",
    "PS4='[$(echo a)$(echo b)] '; set -x; echo hi",
    /* The trace of the *last* command is where a `-c` body reaches end of
     * input, so a multi-line body only shows it on `set +x`. Written as
     * one line above for the same reason: the whole body is then the last
     * command. */
    "PS4='$(echo P) '\nset -x\necho hi\nset +x",
    /* Genuinely unterminated, which must still be reported by both. */
    "PS4='$(exit 7+ '; set -x; echo hi",
];

/// One script through one shell with **stderr merged into stdout**.
///
/// `pinned_bash::answer` discards stderr, and the whole of what a prompt
/// case produces is the `set -x` trace, which is written there. Comparing
/// two shells' stdout for these scripts would compare `hi` against `hi`
/// and pass whatever either did with the prompt.
fn merged(shell: &Path, dialect: &[&str], script: &str) -> Answer {
    let output = std::process::Command::new(shell)
        .args(dialect)
        .arg(script)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .stderr(std::process::Stdio::piped())
        .output()
        .unwrap_or_else(|error| panic!("start {}: {error}", shell.display()));
    let mut bytes = output.stdout;
    bytes.extend_from_slice(&output.stderr);
    (bytes, output.status.code().unwrap_or(-1))
}

/// The same shell in its two dialects, as `(POSIX, Bash mode)`.
fn both_dialects_as_argument(script: &str) -> (Answer, Answer) {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    (
        merged(nsh, &["-c"], script),
        merged(nsh, &["-o", "bash", "-c"], script),
    )
}

/// A re-entered parse reads its own source in *both* dialects: the state
/// it used to inherit belongs to the caller's input rather than to either
/// grammar, so gating the repair on the dialect would have left five
/// callers answering differently for a reason neither dialect names.
// [spec:nsh:def:idiom.token-stream/test]
#[test]
fn the_two_dialects_re_enter_alike() {
    agrees(A_SUBSTITUTION_IN_A_PROMPT, both_dialects_as_argument);
}
