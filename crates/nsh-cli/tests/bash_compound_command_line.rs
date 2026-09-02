//! Which line a compound command records, measured against the pinned
//! Bash 5.3.
//!
//! A command records a line when it starts, and everything that reports a
//! position later reads it back: `$LINENO`, the `DEBUG` and `ERR`
//! actions, and the shell's own diagnostics. For a simple command the
//! answer is obvious and both shells agree on it. For a compound command
//! Bash does not have one answer, and the two it has are artefacts of
//! where its parser had got to when the node was built:
//!
//! * a subshell records the line its `)` is on, and `(( ))` the line its
//!   `))` is on, because both nodes are built after their closing token
//!   has been read;
//! * `for`, `case`, `select`, `for ((;;))` and `[[ ]]` record a line from
//!   inside themselves, because those nodes are handed one explicitly;
//! * a group, a `while`, an `until` and an `if` record nothing at all,
//!   and what is read back is whatever the last command left.
//!
//! This shell recorded the opening line for every one of them, which is
//! right for the second group, wrong for the first, and a third answer
//! for the last. The first group is what the table below fixes.
//!
//! THE LAST GROUP IS A DIVERGENCE THIS TABLE RECORDS RATHER THAN CLOSES.
//! It is only reachable through a compound command whose *redirection*
//! fails, because that is the one way such a command's own status is read
//! back without its body having run and recorded a line of its own. Bash
//! answers with the previous command's line -- the Oils corpus registers
//! that as `## BUG bash/mksh` on `builtin-trap-err.test.sh:19` and OSH
//! declines to reproduce it -- and this shell answers with the line the
//! construct opens on, which is a line the construct actually occupies.
//! A stale read is not worth reproducing, so those rows carry both
//! answers and fail if either shell moves.
//!
//! Both shells are run as processes because a subshell's own failure is
//! observed in the parent of a fork, and the trap is the channel because
//! the divergence was never in the status.

/// Shared with `nsh`'s own differential tests rather than copied: one
/// answer to "which Bash", not a second one that can drift from it.
#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

/// Three lines the shell reads before any case's own first line, so that
/// a `line=` below names a line of the body plus three. The third is a
/// command of its own, so a line left over from it is told apart from
/// every line the body occupies.
const PRELUDE: &str = "set -o errtrace\ntrap 'echo line=$LINENO' ERR\n:\n";

/// One script body, the output the pinned Bash gives it, and this shell's
/// where it deliberately differs.
///
/// A `Some` is a divergence recorded rather than described: it fails when
/// the divergence is closed, so the entry cannot outlive its reason.
const CASES: &[(&str, &str, Option<&str>)] = &[
    /* A subshell records its `)`, wherever the `)` was written. */
    ("(\nfalse\n)\n", "line=5\nline=6\n", None),
    ("( false )\n", "line=4\nline=4\n", None),
    ("( false\n)\n", "line=4\nline=5\n", None),
    ("(\nfalse )\n", "line=5\nline=5\n", None),
    ("(\nfalse\n\n\n)\n", "line=5\nline=8\n", None),
    ("(\nfalse\n# nothing here\n)\n", "line=5\nline=7\n", None),
    ("(\n(\nfalse\n)\n)\n", "line=6\nline=8\n", None),
    (
        "f() {\n(\nfalse\n)\n}\nf\n",
        "line=6\nline=7\nline=9\n",
        None,
    ),
    /* `(( ))` records its `))`, which a line continuation moves. */
    ("((\n0\n))\n", "line=6\n", None),
    ("(( 0 ))\n", "line=4\n", None),
    ("(( 0 \\\n))\n", "line=5\n", None),
    /* The forms Bash hands a line of their own keep it, which is why
     * "the line it ends on" is a rule about the two above and not about
     * every compound command. */
    ("for i in $LINENO\ndo\necho $i\ndone\n", "4\n", None),
    (
        "for ((\ni = 0; i < 1; i++\n)); do\nfalse\ndone\n",
        "line=7\n",
        None,
    ),
    ("[[ 1 = 2\n]]\n", "line=4\n", None),
    /* A subshell has recorded its line before its redirection is tried,
     * so the one form Bash records early agrees here too. */
    ("(\n:\n) > /dev/null/x\n", "line=6\n", None),
    /* And the forms Bash records nothing for do not: it answers with the
     * prelude's third line, this shell with the line the construct opens
     * on. Four shapes rather than one, because the rule is the family's
     * and not the group's. */
    ("{\n:\n} > /dev/null/x\n", "line=3\n", Some("line=4\n")),
    (
        "for i in 1; do\n:\ndone > /dev/null/x\n",
        "line=3\n",
        Some("line=4\n"),
    ),
    (
        "while false; do\n:\ndone > /dev/null/x\n",
        "line=3\n",
        Some("line=4\n"),
    ),
    ("((\n1\n)) > /dev/null/x\n", "line=3\n", Some("line=4\n")),
];

/// Feed one case to a shell on standard input and return what it printed.
fn shell_output(shell: &Path, dialect: &[&str], body: &str) -> String {
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
    let mut source = PRELUDE.as_bytes().to_vec();
    source.extend_from_slice(body.as_bytes());
    child
        .stdin
        .take()
        .expect("the child's standard input")
        .write_all(&source)
        .expect("write the script");
    let output = child.wait_with_output().expect("wait for the shell");
    String::from_utf8(output.stdout).expect("the trap prints ASCII")
}

// [spec:nsh:req:compat.bash.traps-introspection/test]
#[test]
fn a_compound_command_records_the_line_bash_records() {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    for (body, reference, divergence) in CASES {
        let want = divergence.unwrap_or(reference);
        assert_eq!(
            shell_output(nsh, &["-o", "bash"], body),
            *want,
            "for the script\n{PRELUDE}{body}"
        );
    }
}

/// The table is the reference's answer, not this repository's opinion --
/// including the rows that record where this shell deliberately differs.
// [spec:nsh:req:compat.bash.traps-introspection/test]
#[test]
fn the_recorded_lines_are_the_references_own() {
    let bash = pinned_bash::path();
    for (body, reference, divergence) in CASES {
        assert_eq!(
            shell_output(&bash, &[], body),
            *reference,
            "the reference disagrees with the recorded output for\n{PRELUDE}{body}"
        );
        assert!(
            divergence.is_none_or(|recorded| recorded != *reference),
            "a divergence that matches the reference is not one; drop the row for\n{PRELUDE}{body}"
        );
    }
}
