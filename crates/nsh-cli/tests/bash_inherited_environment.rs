//! An inherited environment entry whose name is not a shell identifier,
//! measured against the pinned Bash 5.3 and against `/usr/bin/dash`.
//!
//! `A-B=1` cannot be named by any expansion, so no script can read it and
//! the only thing a shell can do with it is hand it to the children it
//! execs. The two references disagree and this shell follows each in the
//! dialect that has it: Bash mode passes such an entry on, the POSIX
//! dialect drops it as dash does. `CARGO_BIN_EXE_<name>` for a hyphenated
//! binary is the case that matters — a build system passes a per-target
//! value that way, and a shell in the middle that deletes it produces a
//! missing variable two processes later that nobody set.
//!
//! Being passed on is only half of it. The entry must stay *invisible*:
//! it is in no listing, no expansion names it, and `unset` cannot reach
//! it. Those are held here as their own rows because an implementation
//! that made it an ordinary exported variable would pass the first half
//! and fail every one of them.
//!
//! Both halves run through both shells and nothing here records an
//! expected value, so a change in either reference shows up as a
//! disagreement rather than as a stale literal.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;
use std::process::{Command, Stdio};

/// The entry every case inherits.
const ENTRY: (&str, &str) = ("A-B", "1");

/// A name a build system really produces, kept beside the short one so a
/// fix that special-cased a two-letter name would not pass.
const CARGO_ENTRY: (&str, &str) = ("CARGO_BIN_EXE_nsh-survey", "/x/y");

/// Scripts whose answer is whether the entry reached a child.
///
/// `env` is an external command in both shells, so each row is a real
/// `execve` and reads the environment the shell actually built.
const REACHES_A_CHILD: &[&str] = &[
    "env",
    "exec env",
    "( env )",
    "f() { env; }; f",
    "env | cat",
    /* `unset` cannot reach a name it cannot spell, so the entry is still
     * there afterwards. */
    "unset 'A-B'; env",
    "unset -v 'A-B' 2>/dev/null; env",
];

/// Scripts whose answer is that nothing inside the shell can see it.
const STAYS_INVISIBLE: &[&str] = &[
    "export -p",
    "set",
    "echo \"[${A-B}]\"",
    "echo \"[${A:-fallback}]\"",
    /* An assignment prefix cannot create one either: the word is not an
     * assignment, so it is a command name and the shell says so. */
    "'a-b'=1 true; echo \"status=$?\"",
];

/// One script through one shell, as `(stdout, status)`.
///
/// `env_clear` before the entry is what makes the row mean anything: the
/// name has to arrive from this call and not from whatever the test
/// runner happened to be holding.
fn answer(shell: &Path, dialect: &[&str], entry: (&str, &str), script: &str) -> (Vec<u8>, i32) {
    let output = Command::new(shell)
        .args(dialect)
        .arg("-c")
        .arg(script)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .env(entry.0, entry.1)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .unwrap_or_else(|error| panic!("start {}: {error}", shell.display()));
    (output.stdout, output.status.code().unwrap_or(-1))
}

/// How many lines of `stdout` mention the entry's name.
fn mentions(output: &[u8], name: &str) -> usize {
    String::from_utf8_lossy(output)
        .lines()
        .filter(|line| line.contains(name))
        .count()
}

/// Bash mode answers what the pinned Bash answers, for every script.
#[track_caller]
fn agrees_with_bash(scripts: &[&str], entry: (&str, &str)) {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let bash = pinned_bash::path();
    for script in scripts {
        let ours = answer(nsh, &["-o", "bash"], entry, script);
        let theirs = answer(&bash, &[], entry, script);
        assert_eq!(
            mentions(&ours.0, entry.0),
            mentions(&theirs.0, entry.0),
            "Bash mode differed on how often `{}` appears for\n{script}",
            entry.0
        );
        assert_eq!(ours.1, theirs.1, "status differed for\n{script}");
    }
}

/// Bash mode hands the entry to every child, as the reference does.
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn bash_mode_passes_the_entry_to_a_child() {
    agrees_with_bash(REACHES_A_CHILD, ENTRY);
    agrees_with_bash(REACHES_A_CHILD, CARGO_ENTRY);
}

/// Nothing inside the shell can name it, in either shell.
#[test]
fn the_entry_is_invisible_inside_the_shell() {
    agrees_with_bash(STAYS_INVISIBLE, ENTRY);
    agrees_with_bash(STAYS_INVISIBLE, CARGO_ENTRY);
}

/// The POSIX dialect drops it, which is what `/usr/bin/dash` does.
///
/// The reference here is dash rather than Bash: Bash has no mode in which
/// the entry is dropped, so there is nothing of Bash's to compare against.
#[test]
fn the_posix_dialect_drops_the_entry() {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let dash = Path::new("/usr/bin/dash");
    assert!(
        dash.exists(),
        "the POSIX reference /usr/bin/dash is missing"
    );
    for entry in [ENTRY, CARGO_ENTRY] {
        for script in ["env", "exec env", "( env )"] {
            let ours = answer(nsh, &[], entry, script);
            let theirs = answer(dash, &[], entry, script);
            assert_eq!(
                mentions(&ours.0, entry.0),
                mentions(&theirs.0, entry.0),
                "the POSIX dialect left dash for\n{script}"
            );
            assert_eq!(
                mentions(&ours.0, entry.0),
                0,
                "the POSIX dialect passed `{}` to a child",
                entry.0
            );
        }
    }
}

/// `set -o posix` stops passing it on and `set -o bash` starts again.
///
/// This is the one row where the reference is deliberately not followed:
/// Bash keeps passing the entry on under `set -o posix`, because its POSIX
/// mode is a set of behaviours rather than a different shell, while here
/// `set -o posix` leaves the dialect whose reference is Bash. Registered
/// in `docs/divergences.md`. The entry is held for the life of the shell
/// rather than discarded, which is what the third row proves.
#[test]
fn the_dialect_decides_and_nothing_is_destroyed() {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let bash = pinned_bash::path();
    let cases = [
        ("env", 1),
        ("set -o posix; env", 0),
        ("set -o posix; set -o bash; env", 1),
    ];
    for (script, expected) in cases {
        let ours = answer(nsh, &["-o", "bash"], ENTRY, script);
        assert_eq!(
            mentions(&ours.0, ENTRY.0),
            expected,
            "Bash mode answered the wrong way for\n{script}"
        );
    }
    /* The reference is asked for the divergent row rather than assumed,
     * so this fails if a later Bash ever drops it. */
    let theirs = answer(&bash, &[], ENTRY, "set -o posix; env");
    assert_eq!(
        mentions(&theirs.0, ENTRY.0),
        1,
        "the reference no longer keeps the entry under `set -o posix`; \
         the divergence registered in docs/divergences.md is stale"
    );
}
