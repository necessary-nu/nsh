//! `{name}<word`: the shell picks the descriptor, measured against the
//! pinned Bash 5.3.
//!
//! `2>file` names a slot in the source. `{name}>file` names none: the
//! shell allocates a free one and stores its number in `name`, so a script
//! can hold a descriptor open without picking a number that might collide
//! with one it wrote by hand. Four rules are not guessable from that
//! sentence and every one of them is a row below.
//!
//! * Allocation starts at ten -- above every number an ordinary IO_NUMBER
//!   is likely to be -- and takes the lowest free slot, so closing one
//!   hands it back.
//! * `{name}<&-` is the exception that reads `name` rather than assigning
//!   it: the script is closing a descriptor it already has. A name holding
//!   no number is `ambiguous redirect`.
//! * The name is published only once the open has succeeded, so a failed
//!   redirection leaves it exactly as it was.
//! * The slot is *not* undone when the command finishes, where a numeric
//!   one is. `{ echo; } {fd}<<< walrus` leaves `$fd` readable afterwards
//!   and `{ echo; } 7<<< walrus` does not.
//!
//! `{name}` must also be a place a number can go, which is what keeps the
//! form from swallowing ordinary words: `{1a}`, `{}` and `{fd}x` are not
//! prefixes, and Bash runs `exec {1a}` as a command.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

/// One script, what the pinned Bash prints for it, and this shell's where
/// it still differs.
///
/// Stdout only: the diagnostics this shell writes are registered as
/// differing in wording, and the subject here is which descriptor was
/// chosen and where its number went.
///
/// A `Some` is a divergence recorded rather than described, and it fails
/// when the divergence is closed. The one left is where the redirection is
/// *applied*: Bash forks an external command before applying, so its
/// parent never has the slot and never sets the name, while this shell
/// applies in the parent and forks after -- which is the same place a
/// built-in's redirection is applied, and why `echo x {v}>f` agrees.
const CASES: &[(&str, &str, Option<&str>)] = &[
    /* Allocation: from ten, lowest free, and a closed slot comes back. */
    (
        "exec {a}</dev/null\nexec {b}</dev/null\necho \"$a $b\"\n",
        "10 11\n",
        None,
    ),
    (
        "exec {a}</dev/null\nexec {a}</dev/null\necho \"$a\"\n",
        "11\n",
        None,
    ),
    (
        "exec {fd}</dev/null\nexec {fd}<&-\nexec {g}</dev/null\necho \"$fd $g\"\n",
        "10 10\n",
        None,
    ),
    (
        "exec 3</dev/null\nexec {n}</dev/null\necho \"$n\"\n",
        "10\n",
        None,
    ),
    /* An existing value does not steer the choice, and is overwritten. */
    ("fd=7\nexec {fd}</dev/null\necho \"$fd\"\n", "10\n", None),
    /* `<&-` reads the name instead, and leaves it alone. */
    (
        "exec {fd}</dev/null\nexec {fd}<&-\necho \"[$fd]\"\n",
        "[10]\n",
        None,
    ),
    (
        "fd=9\nexec 9</dev/null\nexec {fd}<&-\necho \"st=$? [$fd]\"\n",
        "st=0 [9]\n",
        None,
    ),
    /* A failed open publishes nothing. Written on `true` rather than on
     * `exec`, because a special built-in's redirection failure still ends
     * the record here, which is `bash-mode-error-boundary`'s and not this
     * form's. */
    (
        "true {fd}</nonesuch\necho \"[${fd-unset}]\"\n",
        "[unset]\n",
        None,
    ),
    /* The slot outlives the command it was written on, where a numeric one
     * does not. Both halves, because it is the surprising rule. */
    (
        "{ echo x; } {fd}>/dev/null\necho hi >&$fd\necho \"st=$?\"\n",
        "x\nst=0\n",
        None,
    ),
    (
        "{ echo x; } {fd}<<< walrus\ncat <&$fd\n",
        "x\nwalrus\n",
        None,
    ),
    ("echo x {v}>/dev/null\necho \"v=$v\"\n", "x\nv=10\n", None),
    (
        "f() { :; }\nf {fd}>/dev/null\necho hi >&$fd\necho \"st=$?\"\n",
        "st=0\n",
        None,
    ),
    (
        "while false; do :; done {fd}>/dev/null\necho \"$fd\"\n",
        "10\n",
        None,
    ),
    /* Where the redirection is applied, and the one divergence left. Bash
     * forks an external command before applying, so its parent never has
     * the slot and never sets the name; this shell applies in the parent
     * and forks after, which is the same place a built-in's redirection is
     * applied and why the `echo` row above agrees. A subshell forks first
     * in both. */
    (
        "cat /dev/null {fd}</dev/null\necho \"[${fd-unset}]\"\n",
        "[unset]\n",
        Some("[10]\n"),
    ),
    (
        "( : ) {fd}>/dev/null\necho \"[${fd-unset}]\"\n",
        "[unset]\n",
        None,
    ),
    /* Duplication and the other operators reach the same allocator. */
    (
        "exec {fd}>&1\necho hello >&$fd\necho \"fd=$fd\"\n",
        "hello\nfd=10\n",
        None,
    ),
    (
        "exec {fd}</dev/null\nexec {g}<&$fd\necho \"$fd $g\"\n",
        "10 11\n",
        None,
    ),
    ("exec {fd}<> /dev/null\necho \"$fd\"\n", "10\n", None),
    /* A nameref is followed, and a subscript is a place a number can go. */
    (
        "declare -n fd=x\nexec {fd}</dev/null\necho \"$fd $x\"\n",
        "10 10\n",
        None,
    ),
    (
        "a=(1)\nexec {a[0]}</dev/null\necho \"${a[0]}\"\n",
        "10\n",
        None,
    ),
    /* Not a prefix: the braces must hold a name and be the whole word. */
    (
        "echo {fd}x>/dev/null\necho \"[${fd-unset}]\"\n",
        "[unset]\n",
        None,
    ),
    (
        "echo '{fd}' >/dev/null\necho \"[${fd-unset}]\"\n",
        "[unset]\n",
        None,
    ),
];

/// Feed one case to a shell on standard input and return its stdout.
fn output(shell: &Path, dialect: &[&str], script: &str) -> String {
    let mut child = Command::new(shell)
        .args(dialect)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|error| panic!("start {}: {error}", shell.display()));
    child
        .stdin
        .take()
        .expect("the child's standard input")
        .write_all(script.as_bytes())
        .expect("write the script");
    let output = child.wait_with_output().expect("wait for the shell");
    String::from_utf8(output.stdout).expect("these scripts print ASCII")
}

// [spec:nsh:req:compat.bash.parser-ast/test]
#[test]
fn the_shell_picks_the_descriptor_bash_picks() {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    for (script, reference, divergence) in CASES {
        let want = divergence.unwrap_or(reference);
        assert_eq!(output(nsh, &["-o", "bash"], script), *want, "for\n{script}");
    }
}

/// The table is the reference's answer, not this repository's opinion.
// [spec:nsh:req:compat.bash.parser-ast/test]
#[test]
fn the_recorded_output_is_the_references_own() {
    let bash = pinned_bash::path();
    for (script, reference, divergence) in CASES {
        assert_eq!(
            output(&bash, &[], script),
            *reference,
            "the reference disagrees with the recorded output for\n{script}"
        );
        assert!(
            divergence.is_none_or(|recorded| recorded != *reference),
            "a divergence that matches the reference is not one; drop the row for\n{script}"
        );
    }
}

/// The form belongs to the dialect: with it off, `{fd}` is a word, and
/// `exec {fd}</dev/null` is Dash's "not found" rather than a redirection.
// [spec:nsh:req:compat.bash.default-isolation/test]
#[test]
fn the_posix_dialect_reads_it_as_a_word() {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let script = "exec {fd}</dev/null\necho \"[${fd-unset}]\"\n";
    assert_eq!(output(nsh, &[], script), "");
    assert_eq!(
        output(nsh, &[], "echo {fd}>/dev/null\necho \"[${fd-unset}]\"\n"),
        "[unset]\n"
    );
}
