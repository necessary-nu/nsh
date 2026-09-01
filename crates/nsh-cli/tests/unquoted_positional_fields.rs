//! Unquoted `$@` and `$*` are one expansion, measured against the pinned Bash.
//!
//! Quoted, the two differ and that is the whole point of `$@`. Unquoted,
//! POSIX makes them the same: the words are joined with `IFS` and the
//! result is split like any other expansion. This shell gave unquoted
//! `$@` a field per positional instead, so an empty positional survived
//! as an empty field where splitting would have dropped it, and
//! `set -- '' b; echo $@` printed a leading space that Bash, dash and
//! POSIX do not.
//!
//! Found by the `differential` fuzz target on 2026-09-01, inside a
//! twenty-line generated script, on the first campaign this repository
//! had run against the current tree. No survey case reached it: the
//! corpora set positionals to non-empty words.
//!
//! Nothing here is a recorded expectation. Every case runs in both
//! shells and the answers are compared, so there is no literal to go
//! stale.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

/// The shapes that separate "a field per word" from "join and split".
///
/// `printf '[%s]'` is the instrument rather than `echo`, because the
/// question is how many fields there were and `echo` answers with one
/// line either way.
const CASES: &[&str] = &[
    /* An empty positional: dropped unquoted, kept quoted. */
    "set -- '' b; printf '[%s]' $@; echo",
    "set -- '' b; printf '[%s]' \"$@\"; echo",
    "set -- a '' b; printf '[%s]' $@; echo",
    "set -- '' ''; printf '[%s]' $@; echo",
    "set -- '' ''; echo \"<$@>\"",
    "set -- ''; printf '[%s]' $@; echo",
    /* Adjacent literal text: the first and last words join to it, and an
     * empty word in the middle still contributes nothing. */
    "set -- '' b; printf '[%s]' x$@y; echo",
    "set -- a '' b; printf '[%s]' x$@y; echo",
    /* `IFS` decides, and a non-whitespace separator keeps the empty
     * field that a whitespace one drops. */
    "set -- '' b; IFS=:; printf '[%s]' $@; echo",
    "set -- '' b; IFS=; printf '[%s]' $@; echo",
    "set -- a b; IFS=:; printf '[%s]' $@; echo",
    "set -- a b; IFS=:; printf '[%s]' \"$@\"; echo",
    "set -- a b; IFS=:; v=\"$@\"; echo \"[$v]\"",
    /* `$*` is the control: it was already right, and must stay so. */
    "set -- '' b; printf '[%s]' $*; echo",
    "set -- a '' b; printf '[%s]' $*; echo",
    "set -- '' b; IFS=; printf '[%s]' $*; echo",
    "set -- a b; echo \"$*\"",
    /* Splitting inside a word, and the loop that made it visible. */
    "set -- 'a b' c; printf '[%s]' $@; echo",
    "set -- '' b; for w in $@; do printf '<%s>' \"$w\"; done; echo",
    "set -- a b c; for w in \"$@\"; do printf '<%s>' \"$w\"; done; echo",
];

/// One script's standard output and status from one shell.
fn answer(shell: &Path, dialect: &[&str], script: &str) -> (Vec<u8>, i32) {
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
    (output.stdout, output.status.code().unwrap_or(-1))
}

/// The dialect does not enter into it: the rule is POSIX's, so both of
/// this shell's dialects are held to the same reference.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn an_unquoted_at_splits_like_a_star() {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let bash = pinned_bash::path();
    for script in CASES {
        let theirs = answer(&bash, &[], script);
        for dialect in [&[][..], &["-o", "bash"][..]] {
            let ours = answer(nsh, dialect, script);
            assert_eq!(
                String::from_utf8_lossy(&ours.0),
                String::from_utf8_lossy(&theirs.0),
                "{script}  (dialect {dialect:?})",
            );
            assert_eq!(ours.1, theirs.1, "status differed for {script}");
        }
    }
}
