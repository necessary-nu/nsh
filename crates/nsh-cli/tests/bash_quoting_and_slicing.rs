//! Two tables this repository calls the pinned Bash's, run against it.
//!
//! `crates/nsh/src/escape/bash.rs` and
//! `crates/nsh/src/expand/typed/bash.rs` each hold a table of expected
//! bytes whose doc comment says the pinned Bash 5.3 produced them. Both
//! were written by hand from a session with that build and neither had
//! ever been run against it again -- which is a check that cannot fail
//! for the reason its comment gives, and the third shape
//! `[spec:nsh:req:oracle.cannot-measure-is-a-failure]` reaches.
//!
//! Nothing here is a recorded expectation. Each case runs in both shells
//! and the two answers are compared, so there is no literal to go stale:
//! if Bash changes its mind, this reports it rather than passing.
//!
//! The two subjects are `%q`, which decides whether a byte is syntax
//! where it sits, and `${x:offset:length}`, where an offset past either
//! end is settled before the length is looked at.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

/// Every input the `%q` table names, in the order it names them.
///
/// A tilde is an expansion at the front of a word or straight after the
/// `:` or `=` of an assignment and nowhere else, a `#` starts a comment
/// only at the front, and a comma is escaped everywhere.
const QUOTED: &[&str] = &[
    "~", "~a", ":~", "a=~", "a=b=~", "a=x:~/y", "a~b", "a~", "P~2T", "~~", "#", "a#b", ":#", ",",
    "a,b",
];

/// Every case the substring table names, as the value and the expansion.
///
/// The negative offsets carry the space Bash's grammar needs to tell
/// `${x:-word}` from a subscript that counts back from the end.
const SLICED: &[(&str, &str)] = &[
    ("'abcdef'", "${x: -6:3}"),
    ("'abcdef'", "${x: -7:3}"),
    ("'ab'", "${x: -8:14}"),
    ("'ab'", "${x: -8}"),
    ("'abcdef'", "${x:2:2}"),
    ("'ab'", "${x:3:1}"),
    /* The ordering: past the end wins over a backwards length, and only
     * the third of these is refused. */
    ("'ab'", "${x:3:-1}"),
    ("'ab'", "${x:1:-1}"),
    ("'ab'", "${x:2:-1}"),
    ("'abcdef'", "${x:0:-1}"),
    /* Bytes that are not valid UTF-8 are still bytes to count. */
    ("$'\\x8b\\xab'", "${x: -8:14}"),
    ("$'\\x8b\\xab'", "${x:0:14}"),
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

/// Both shells on one script, as `(what nsh said, what the pinned Bash said)`.
fn both(script: &str) -> ((Vec<u8>, i32), (Vec<u8>, i32)) {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let bash = pinned_bash::path();
    (
        answer(nsh, &["-o", "bash"], script),
        answer(&bash, &[], script),
    )
}

/// `%q` escapes a byte exactly where the reference escapes it.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn the_quoting_table_is_the_references_own() {
    for input in QUOTED {
        let script = format!("printf '%q' '{input}'\n");
        let (ours, theirs) = both(&script);
        assert_eq!(
            String::from_utf8_lossy(&ours.0),
            String::from_utf8_lossy(&theirs.0),
            "`printf %q` disagreed on {input:?}",
        );
        assert_eq!(ours.1, theirs.1, "status differed on {input:?}");
    }
}

/// A substring offset past either end selects what the reference selects.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn the_substring_table_is_the_references_own() {
    for (value, expansion) in SLICED {
        let script = format!("x={value}\nprintf '[%s]' \"{expansion}\"\n");
        let (ours, theirs) = both(&script);
        assert_eq!(
            ours.0, theirs.0,
            "`{expansion}` on {value} printed {:?} here and {:?} there",
            ours.0, theirs.0
        );
        assert_eq!(
            ours.1, theirs.1,
            "status differed for `{expansion}` on {value}"
        );
    }
}
