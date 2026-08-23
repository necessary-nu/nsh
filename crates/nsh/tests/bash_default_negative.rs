//! Every construct the Bash dialect adds must be inert or an error while
//! the `bash` option is off.
//!
//! This is the strongest guarantee the compatibility profile offers, and
//! it is the one that decays quietest: a feature written without a
//! dialect gate keeps working, so nothing fails and the leak is invisible
//! until a POSIX script means something new. The matrix below is written
//! from the other side -- it names each addition and asserts that the
//! default shell refuses it -- so a missing gate fails a test instead.

use bstr::BStr;
use nsh::{Shell, Streams};

fn shell(bash: bool) -> Shell {
    Shell::builder()
        .streams(Streams::capture().expect("create capture streams"))
        .option(BStr::new(b"bash"), bash)
        .build()
        .expect("build shell")
}

struct Observation {
    status: i32,
    stdout: Vec<u8>,
}

fn run(shell: &mut Shell, script: &[u8]) -> Observation {
    /* A refused construct can arrive either way -- a parse failure is an
     * error value, a missing built-in is a status -- and the matrix cares
     * about the refusal, not about which side it came from. */
    let status = match shell.run(script) {
        Ok(status) => status.code().into(),
        Err(error) => error.status().code().into(),
    };
    let stdout = shell
        .take_captured_stdout()
        .expect("capture stdout")
        .to_vec();
    let _ = shell.take_captured_stderr().expect("capture stderr");
    Observation { status, stdout }
}

/// What the default dialect must do with one Bash-only construct.
#[derive(Clone, Copy)]
enum Refusal {
    /// The construct is not a command the default shell knows.
    NotFound,
    /// The construct does not parse, or the expansion is rejected.
    Rejected,
    /// The construct parses and runs, but means what it meant before the
    /// dialect existed -- the text stays literal.
    Literal(&'static str),
    /// The name the dialect publishes is not set.
    Unset,
}

const NOT_FOUND: i32 = 127;

/// One row per addition the Bash dialect makes. `bash` is what the
/// dialect does with it, so a row that stops being a Bash feature at all
/// fails just as loudly as one that leaks.
const MATRIX: &[(&str, &[u8], Refusal)] = &[
    ("conditional command", b"[[ a == a ]]", Refusal::NotFound),
    ("regex operand", b"[[ x =~ x ]]", Refusal::NotFound),
    ("arithmetic command", b"(( 1 + 1 ))", Refusal::NotFound),
    (
        "arithmetic for",
        b"for ((i = 0; i < 1; i++)); do :; done",
        Refusal::Rejected,
    ),
    ("array assignment", b"a=(1 2)", Refusal::Rejected),
    ("array element read", b"echo \"${a[0]}\"", Refusal::Rejected),
    ("whole-array read", b"echo \"${a[@]}\"", Refusal::Rejected),
    ("array key read", b"echo \"${!a[@]}\"", Refusal::Rejected),
    (
        "input process substitution",
        b"cat <(echo x)",
        Refusal::Rejected,
    ),
    (
        "output process substitution",
        b"echo hi > >(cat)",
        Refusal::Rejected,
    ),
    ("brace list", b"echo {a,b}", Refusal::Literal("{a,b}\n")),
    (
        "brace range",
        b"echo a{1..3}",
        Refusal::Literal("a{1..3}\n"),
    ),
    (
        "parameter transform",
        b"x=ab; echo \"${x@Q}\"",
        Refusal::Rejected,
    ),
    (
        "case transform",
        b"x=ab; echo \"${x^^}\"",
        Refusal::Rejected,
    ),
    (
        "pattern substitution",
        b"x=abc; echo \"${x/b/X}\"",
        Refusal::Rejected,
    ),
    ("substring", b"x=abc; echo \"${x:1:1}\"", Refusal::Rejected),
    (
        "indirect expansion",
        b"y=x; x=1; echo \"${!y}\"",
        Refusal::Rejected,
    ),
    (
        "function keyword",
        b"function f { echo hi; }",
        Refusal::Rejected,
    ),
    (
        "non-POSIX function name",
        b"a-b() { :; }",
        Refusal::Rejected,
    ),
    (
        "eval option terminator",
        b"eval -- \"echo hi\"",
        Refusal::NotFound,
    ),
    ("declare", b"declare -a v", Refusal::NotFound),
    (
        "associative declaration",
        b"declare -A m",
        Refusal::NotFound,
    ),
    ("nameref declaration", b"declare -n r=x", Refusal::NotFound),
    ("shopt", b"shopt -s extglob", Refusal::NotFound),
    ("mapfile", b"mapfile x < /dev/null", Refusal::NotFound),
    ("readarray", b"readarray x < /dev/null", Refusal::NotFound),
    ("let", b"let 1+1", Refusal::NotFound),
    ("caller", b"f() { caller; }; f", Refusal::NotFound),
    ("enable", b"enable -n echo", Refusal::NotFound),
    ("directory stack", b"pushd .", Refusal::NotFound),
    (
        "BASH_VERSION",
        b"printf %s \"$BASH_VERSION\"",
        Refusal::Unset,
    ),
    (
        "BASH_VERSINFO",
        b"printf %s \"${BASH_VERSINFO[0]}\"",
        Refusal::Unset,
    ),
    ("BASHOPTS", b"printf %s \"$BASHOPTS\"", Refusal::Unset),
    /* An option has to be on for this one to have anything to say:
     * `SHELLOPTS` lists the `set -o` names that are, and none of them
     * is in a fresh shell. `set -x` writes its trace to stderr, so the
     * POSIX half of the row still sees an empty stdout. */
    (
        "SHELLOPTS",
        b"set -x; printf %s \"$SHELLOPTS\"",
        Refusal::Unset,
    ),
    (
        "EPOCHSECONDS",
        b"printf %s \"$EPOCHSECONDS\"",
        Refusal::Unset,
    ),
    (
        "EPOCHREALTIME",
        b"printf %s \"$EPOCHREALTIME\"",
        Refusal::Unset,
    ),
    ("RANDOM", b"printf %s \"$RANDOM\"", Refusal::Unset),
    (
        "BASH_SUBSHELL",
        b"printf %s \"$BASH_SUBSHELL\"",
        Refusal::Unset,
    ),
    (
        "FUNCNAME",
        b"f() { printf %s \"$FUNCNAME\"; }; f",
        Refusal::Unset,
    ),
    (
        "BASH_SOURCE",
        b"f() { printf %s \"$BASH_SOURCE\"; }; f",
        Refusal::Unset,
    ),
    (
        "BASH_LINENO",
        b"f() { printf %s \"$BASH_LINENO\"; }; f",
        Refusal::Unset,
    ),
    (
        "BASH_REMATCH",
        b"[[ x =~ x ]] 2>/dev/null; printf %s \"$BASH_REMATCH\"",
        Refusal::Unset,
    ),
];

// [spec:nsh:req:compat.bash.default-isolation/test]
#[test]
fn default_mode_refuses_every_bash_addition() {
    for (label, script, refusal) in MATRIX {
        let mut posix = shell(false);
        let observed = run(&mut posix, script);
        match refusal {
            Refusal::NotFound => assert_eq!(
                observed.status, NOT_FOUND,
                "{label}: default mode ran a Bash built-in"
            ),
            Refusal::Rejected => assert_ne!(
                observed.status, 0,
                "{label}: default mode accepted Bash-only syntax"
            ),
            Refusal::Literal(text) => {
                assert_eq!(observed.status, 0, "{label}");
                assert_eq!(
                    observed.stdout,
                    text.as_bytes(),
                    "{label}: default mode expanded a Bash-only construct"
                );
            }
            Refusal::Unset => assert!(
                observed.stdout.is_empty(),
                "{label}: default mode publishes a Bash-only variable"
            ),
        }
    }
}

/// The other half of the same claim: each row is a Bash feature, so the
/// matrix cannot quietly become a list of things no dialect does.
// [spec:nsh:req:compat.bash.default-isolation/test]
#[test]
fn matrix_rows_are_bash_features() {
    for (label, script, refusal) in MATRIX {
        let mut bash = shell(true);
        let observed = run(&mut bash, script);
        match refusal {
            Refusal::Unset => assert!(
                !observed.stdout.is_empty(),
                "{label}: Bash mode does not publish this variable"
            ),
            Refusal::Literal(text) => assert_ne!(
                observed.stdout,
                text.as_bytes(),
                "{label}: Bash mode leaves this construct literal too"
            ),
            _ => assert_eq!(observed.status, 0, "{label}: Bash mode refuses it too"),
        }
    }
}

/// Turning the dialect off again restores the baseline for input parsed
/// after the change, in the same process and the same `Shell`.
// [spec:nsh:req:compat.bash.default-isolation/test]
#[test]
fn disabling_the_dialect_restores_baseline() {
    let mut shell = shell(true);
    assert_eq!(run(&mut shell, b"[[ a == a ]]").status, 0);
    assert_eq!(run(&mut shell, b"set +o bash").status, 0);
    assert_eq!(run(&mut shell, b"[[ a == a ]]").status, NOT_FOUND);
    assert_eq!(run(&mut shell, b"echo {a,b}").stdout, b"{a,b}\n");
    assert_eq!(run(&mut shell, b"set -o bash").status, 0);
    assert_eq!(run(&mut shell, b"[[ a == a ]]").status, 0);
    assert_eq!(run(&mut shell, b"echo {a,b}").stdout, b"a b\n");
}

/// Two shells driven in one process keep separate dialects.
// [spec:nsh:req:compat.bash.state-isolation/test]
#[test]
fn two_shells_keep_separate_dialects() {
    let mut bash = shell(true);
    let mut posix = shell(false);
    assert_eq!(run(&mut bash, b"echo {a,b}").stdout, b"a b\n");
    assert_eq!(run(&mut posix, b"echo {a,b}").stdout, b"{a,b}\n");
    assert_eq!(run(&mut bash, b"echo {a,b}").stdout, b"a b\n");
}
