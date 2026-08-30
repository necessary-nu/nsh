//! What a `**` matches when nothing follows it, measured against the
//! pinned Bash 5.3.
//!
//! Bash matches the last component of a pattern against the directory
//! everything before it names, and a `**` there matches that directory
//! itself as well as everything under it. The word it writes for that
//! match is the prefix Bash already had in hand, which is why the three
//! spellings of the same directory come out differently: `a/**` yields
//! `a/` because the pattern spelled the prefix out, `?/**` yields `a`
//! because the shell had to find it, and a bare `**` yields nothing at
//! all for it -- its prefix is nothing at all. This shell generated an
//! empty field there instead, so `echo **` printed a leading space and
//! `for x in **` ran an extra iteration on the empty string.
//!
//! The rest of the rule is in the walk. `**` descends the directory tree
//! and not the link graph, so a symbolic link to a directory is reached
//! but never entered; a `**` that ends the word still asks what a name
//! resolves to, so `**/` names that link while `**/f` does not look
//! through it. Dot names are the walk's to hide or reveal like any other
//! component's, so `dotglob` reaches them and `GLOBIGNORE` does too.
//!
//! Fields are printed one bracketed word at a time because the field this
//! was filed for is empty, and `echo` shows an empty field only as a
//! space between two others.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// One prelude, one pattern, and the fields the pinned Bash generates for
/// it in the `tree` fixture.
const CASES: &[(&str, &str, &str)] = &[
    /* The filed defect and its neighbours: a leading `**` has no prefix,
     * so it generates no field for the directory it starts from. */
    ("shopt -s globstar", "**", "<a><a/b><a/b/f><a/f><e><f>\n"),
    ("shopt -s globstar", "**/", "<a/><a/b/><e/>\n"),
    ("shopt -s globstar", "**/**", "<a><a/b><a/b/f><a/f><e><f>\n"),
    ("shopt -s globstar", "**/**/", "<a/><a/b/><e/>\n"),
    (
        "shopt -s globstar",
        "**/**/**",
        "<a><a/b><a/b/f><a/f><e><f>\n",
    ),
    (
        "shopt -s globstar",
        "-- **",
        "<--><a><a/b><a/b/f><a/f><e><f>\n",
    ),
    /* A prefix the pattern spells out comes back with its slash. */
    ("shopt -s globstar", "a/**", "<a/><a/b><a/b/f><a/f>\n"),
    ("shopt -s globstar", "a/**/", "<a/><a/b/>\n"),
    (
        "shopt -s globstar",
        "./**",
        "<./><./a><./a/b><./a/b/f><./a/f><./e><./f>\n",
    ),
    ("shopt -s globstar", "e/**", "<e/>\n"),
    ("shopt -s globstar", "e/**/", "<e/>\n"),
    /* A prefix the shell had to match does not, even where the pattern
     * that matched it could only ever match one name. */
    ("shopt -s globstar", "*/**", "<a><a/b><a/b/f><a/f><e>\n"),
    ("shopt -s globstar", "?/**", "<a><a/b><a/b/f><a/f><e>\n"),
    ("shopt -s globstar", "[a]/**", "<a><a/b><a/b/f><a/f>\n"),
    ("shopt -s globstar", "a**/**", "<a><a/b><a/b/f><a/f>\n"),
    ("shopt -s globstar", "a/*/**", "<a/b><a/b/f>\n"),
    ("shopt -s globstar", "**/b/**", "<a/b><a/b/f>\n"),
    ("shopt -s globstar", "**/e/**", "<e>\n"),
    /* A `**` with a component after it is a prefix to carry on from, and
     * the directory it starts from contributes no separate field. */
    ("shopt -s globstar", "**/f", "<a/b/f><a/f><f>\n"),
    ("shopt -s globstar", "a/**/f", "<a/b/f><a/f>\n"),
    ("shopt -s globstar", "**/*", "<a><a/b><a/b/f><a/f><e><f>\n"),
    /* `**` matches a directory, so a file prefix matches nothing and the
     * word stays as it was written. Quoting takes the meaning away. */
    ("shopt -s globstar", "f/**", "<f/**>\n"),
    ("shopt -s globstar", "nope/**", "<nope/**>\n"),
    ("shopt -s globstar", "\"**\"", "<**>\n"),
    ("shopt -s globstar; shopt -s nullglob", "nope/**", "\n"),
    /* An empty directory is where the difference between a `**` that
     * matches its own directory and one that matches nothing shows. */
    ("cd e; shopt -s globstar", "**", "<**>\n"),
    ("cd e; shopt -s globstar", "**/", "<**/>\n"),
    ("cd e; shopt -s globstar; shopt -s nullglob", "**", "\n"),
    /* The walk hides and reveals dot names by the same rules as any other
     * component, so it descends into a revealed dot directory too. */
    (
        "shopt -s globstar; shopt -s dotglob",
        "**",
        "<.d><.d/f><.hidden><a><a/b><a/b/f><a/f><e><f>\n",
    ),
    (
        "shopt -s globstar; shopt -s dotglob",
        "**/",
        "<.d/><a/><a/b/><e/>\n",
    ),
    (
        "shopt -s globstar; shopt -s dotglob",
        "*/**",
        "<.d><.d/f><a><a/b><a/b/f><a/f><e>\n",
    ),
    (
        "shopt -s globstar; shopt -s dotglob",
        "**/*",
        "<.d><.d/f><.hidden><a><a/b><a/b/f><a/f><e><f>\n",
    ),
    (
        "shopt -s globstar; GLOBIGNORE=a",
        "**",
        "<.d><.d/f><.hidden><a/b><a/b/f><a/f><e><f>\n",
    ),
    /* Without `globstar` the two stars are one, and nothing recurses. */
    ("shopt -u globstar", "**", "<a><e><f>\n"),
    ("shopt -u globstar", "**/", "<a/><e/>\n"),
];

/// The same, in the `link` fixture. Symbolic links are a Unix fixture, so
/// on a host without them there is nothing here to measure.
#[cfg(unix)]
const LINK_CASES: &[(&str, &str, &str)] = &[
    /* The walk reaches a link to a directory and stops there: `d/f` is
     * under the walk, `ldir/f` is behind a link. */
    ("shopt -s globstar", "**", "<d><d/f><ldir><lf>\n"),
    ("shopt -s globstar", "**/f", "<d/f>\n"),
    ("shopt -s globstar", "*/**", "<d><d/f><ldir><ldir/f>\n"),
    /* A `**` the word ends at asks what the name resolves to, so the
     * link is a directory match even though it was not descended. */
    ("shopt -s globstar", "**/", "<d/><ldir/>\n"),
    ("shopt -s globstar", "ldir/**", "<ldir/><ldir/f>\n"),
    ("shopt -s globstar", "ldir/**/", "<ldir/>\n"),
    ("shopt -s globstar", "d/**", "<d/><d/f>\n"),
    /* A link to a file is not a directory, in either direction. */
    ("shopt -s globstar", "lf/**", "<lf/**>\n"),
];

#[cfg(not(unix))]
const LINK_CASES: &[(&str, &str, &str)] = &[];

/// The two trees the rows above are measured in, under one root the
/// caller names so that the two tests do not share a directory.
fn fixture(name: &str) -> PathBuf {
    let mut root = std::env::temp_dir();
    root.push(format!("nsh-globstar-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for directory in ["tree/a/b", "tree/e", "tree/.d", "link/d"] {
        std::fs::create_dir_all(root.join(directory)).expect("create a fixture directory");
    }
    for file in [
        "tree/f",
        "tree/a/f",
        "tree/a/b/f",
        "tree/.hidden",
        "tree/.d/f",
        "link/d/f",
    ] {
        std::fs::File::create(root.join(file)).expect("create a fixture file");
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("d", root.join("link/ldir")).expect("link to a directory");
        std::os::unix::fs::symlink("d/f", root.join("link/lf")).expect("link to a file");
    }
    root
}

/// A script that prints each generated field bracketed and on one line.
fn script(prelude: &str, pattern: &str) -> String {
    format!("{prelude}\nset -- {pattern}\nfor x; do printf '<%s>' \"$x\"; done\nprintf '\\n'\n")
}

/// Feed one script to a shell running in `directory` and return its stdout.
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

// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn globstar_generates_the_fields_bash_generates() {
    let root = fixture("shell");
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    for (tree, cases) in [("tree", CASES), ("link", LINK_CASES)] {
        for (prelude, pattern, fields) in cases {
            let directory = root.join(tree);
            assert_eq!(
                output(nsh, &["-o", "bash"], &directory, &script(prelude, pattern)),
                *fields,
                "for `{pattern}` under `{prelude}` in {tree}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// The table is the reference's answer, not this repository's opinion.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn the_recorded_fields_are_the_references_own() {
    let root = fixture("reference");
    let bash = pinned_bash::path();
    for (tree, cases) in [("tree", CASES), ("link", LINK_CASES)] {
        for (prelude, pattern, fields) in cases {
            let directory = root.join(tree);
            assert_eq!(
                output(&bash, &[], &directory, &script(prelude, pattern)),
                *fields,
                "the reference disagrees with the recorded fields for \
                 `{pattern}` under `{prelude}` in {tree}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}
