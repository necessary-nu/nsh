//! Whether a word whose `[` closes nothing is a pathname-expansion
//! candidate, put to both shells.
//!
//! A plain expansion cannot tell the two answers apart: a word that is
//! not a candidate is left alone, and so is a candidate that matched
//! nothing. The glob options can. `nullglob` drops what matched nothing,
//! `failglob` refuses it, and `nocaseglob` lets a file whose name differs
//! from the word only in case stand in for it -- which is why the fixture
//! holds both `[abc` and `[ABC`, and why a shell that opens the directory
//! answers differently here from one that does not.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The words that name themselves, whatever the directory holds.
const ORDINARY: &[&str] = &[
    "[", "[abc", "a[b", "[a-", "[[", "]", "a]b",
    /* Slashes are identified before bracket expressions, so the `[` in
     * each of these is left with nothing to close it. */
    "a[b/c]d", "sub/[", "[/x", "x/[",
];

/// The words that really are patterns, kept beside the others so that a
/// change which stops expanding brackets altogether fails here.
const PATTERNS: &[&str] = &["[a]", "[ab]", "a*[", "*/[", "[[:alpha:]]", "[!a]", "*"];

/// The empty first entry is the plain expansion, where the decision
/// shows only as a directory read and no assertion can reach it.
const OPTIONS: &[&str] = &["", "nullglob", "failglob", "nocaseglob", "dotglob"];

/// `[abc` and `[ABC` are both here, and differ only in case, which is
/// what makes `nocaseglob` able to see whether the directory was read.
fn fixture(name: &str) -> PathBuf {
    let mut root = std::env::temp_dir();
    root.push(format!(
        "nsh-unclosed-bracket-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("sub")).expect("create the fixture");
    std::fs::create_dir_all(root.join("x")).expect("create the fixture");
    for file in [
        "a", "ab", "[abc", "[ABC", "a[b", "a]b", "[a]", "sub/f", "x/q",
    ] {
        std::fs::File::create(root.join(file)).expect("create a fixture file");
    }
    root
}

fn script(option: &str, word: &str) -> String {
    let prelude = if option.is_empty() {
        String::new()
    } else {
        format!("shopt -s {option}\n")
    };
    format!("{prelude}set -- {word}\nfor x; do printf '<%s>' \"$x\"; done\nprintf '\\n'\n")
}

/// Standard error is dropped because the two shells spell a `failglob`
/// diagnostic differently, and it is the field list being compared.
fn output(shell: &Path, dialect: &[&str], directory: &Path, script: &str) -> String {
    let mut child = Command::new(shell)
        .args(dialect)
        .current_dir(directory)
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
    String::from_utf8(output.stdout).expect("these fixtures are named in ASCII")
}

// [spec:posix:req:pattern.filename-expansion-trigger/test]
// [spec:posix:req:pattern.no-special-chars-unchanged/test]
// [spec:posix:sem:pattern.left-bracket-literal/test]
// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn an_unclosed_bracket_answers_what_bash_answers() {
    /* Each shell gets a directory of its own: the scripts create no
     * files, but a shared one would still make the second run measure
     * whatever the first left behind. */
    let ours = fixture("shell");
    let theirs = fixture("reference");
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let bash = pinned_bash::path();

    for word in ORDINARY.iter().chain(PATTERNS) {
        for option in OPTIONS {
            let script = script(option, word);
            assert_eq!(
                output(nsh, &["-o", "bash"], &ours, &script),
                output(&bash, &[], &theirs, &script),
                "for `{word}` under `shopt -s {option}`"
            );
        }
    }

    let _ = std::fs::remove_dir_all(&ours);
    let _ = std::fs::remove_dir_all(&theirs);
}

/// The half no reference can be asked, because it is about the words and
/// not about one directory: an unclosed bracket comes back unchanged from
/// every directory, and `nullglob` is where that becomes visible.
// [spec:posix:req:pattern.no-special-chars-unchanged/test]
// [spec:posix:syn:pattern.slash-terminates-bracket/test]
#[test]
fn an_unclosed_bracket_is_never_dropped_by_nullglob() {
    let root = fixture("nullglob");
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    for word in ORDINARY {
        assert_eq!(
            output(nsh, &["-o", "bash"], &root, &script("nullglob", word)),
            format!("<{word}>\n"),
            "`{word}` was expanded as a pattern"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}
