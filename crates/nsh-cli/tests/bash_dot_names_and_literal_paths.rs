//! What a pattern matches when it names `.` or `..`, and what the word it
//! generates keeps of the pattern's own text, measured against the pinned
//! Bash 5.3.
//!
//! Four divergences were found while fixing `**` and left for this file
//! to close. Three of them are the same two questions:
//!
//! * `globskipdots` — on unless a script turns it off, and then `.` and
//!   `..` are names no pattern matches, however it spells them;
//! * a run of slashes is part of the word, not a separator between its
//!   parts: Bash copies the run through while it is still reading
//!   literal text and writes one slash after that, so `a//*` generates
//!   `a//b` and `*//*` generates `a/b`;
//! * a non-empty `GLOBIGNORE` hides `.` and `..` outright, so `*/.`
//!   matches nothing under any ignore list at all while `*/./f` still
//!   matches.
//!
//! The fourth is where a `**` that carries the word on leaves it: Bash
//! matches what follows inside a link to a directory for every `**`
//! except one the pattern opens with, which is why `**/f` stops at a
//! link and `d/**/f` goes through it.
//!
//! Nothing here is a recorded expectation. Every case runs in both
//! shells in the same fixture and the two answers are compared, so there
//! is no literal to go stale.
//!
//! TWO SHAPES ARE LEFT DIVERGING, both of them a pattern with two `**`
//! in it, and they are recorded on `match-bash-dot-names-and-literal-paths`
//! rather than asserted here. Bash cuts a pattern at its last slash and
//! recurses, and the seam shows: `**//**/f` generates less than the same
//! walk written any other way, and `**/*/**/f` generates one word twice
//! because two routes reach it. This shell walks left to right and
//! answers each once.
//!
//! Fields are printed one bracketed word at a time because an empty
//! field and a missing one are the same thing to `echo`.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// `globskipdots` is on in a fresh shell, and what it hides is the name
/// rather than the pattern: a bracket expression that can only match
/// `..` matches nothing while it is on.
const DOT_NAMES: &[(&str, &str)] = &[
    ("", ".*"),
    ("", ".[.]*"),
    ("", "./.*"),
    ("", ".??*"),
    ("shopt -s dotglob", ".*"),
    ("shopt -s dotglob", "*"),
    ("shopt -u globskipdots", ".*"),
    ("shopt -u globskipdots", ".[.]*"),
    ("shopt -u globskipdots", "./.*"),
    ("shopt -u globskipdots; shopt -s dotglob", "*"),
    ("shopt -u globskipdots; shopt -s dotglob", ".*"),
    ("shopt -s globstar", "**/.*"),
    ("shopt -s globstar; shopt -u globskipdots", "**/.*"),
    /* A `.` the pattern spells out is a component and not an entry, so
     * neither the option nor its absence has anything to say about it. */
    ("", "*/."),
    ("", "*/.."),
    ("shopt -u globskipdots", "*/."),
    ("shopt -u globskipdots", "*/.."),
    /* The option is a name `shopt` reports and `$BASHOPTS` carries. */
    ("shopt globskipdots", "-"),
    ("shopt -u globskipdots; shopt globskipdots", "-"),
    ("case $BASHOPTS in *globskipdots*) echo listed;; esac", "-"),
];

/// A run of slashes is text the word keeps, until something has had to
/// be matched.
const REPEATED_SLASHES: &[(&str, &str)] = &[
    ("", "a//*"),
    ("", "a///*"),
    ("", ".//*"),
    ("", "*//*"),
    ("", "a//b//*"),
    ("", "*//"),
    ("", "a//*//f"),
    ("", "a//*//*"),
    ("", "*/b//*"),
    ("", "?//*"),
    ("", "a//?//f"),
    ("", "a//[b]//f"),
    ("", "a//*/"),
    ("", "a//*//"),
    ("", "./a//*"),
    ("", "a//b/*//"),
    ("", "a//"),
    ("", "*//*//*"),
    /* The run reaches the pattern through quoting and through a variable
     * the same way it reaches it as literal text. */
    ("", "a/\"/\"*"),
    ("", "\"a//\"*"),
    ("p=a//", "$p*"),
    /* A `**` written with a repeated slash generates what the same `**`
     * with one slash and no name generates, and then looks inside each. */
    ("shopt -s globstar", "a//**"),
    ("shopt -s globstar", ".//**"),
    ("shopt -s globstar", "a//**//f"),
    ("shopt -s globstar", "**//f"),
    ("shopt -s globstar", "**//"),
    ("shopt -s globstar", "**//*"),
    ("shopt -s globstar", "a//**/"),
    ("shopt -s globstar", "a//**//"),
    ("shopt -s globstar", "*/**//f"),
    ("shopt -s globstar", "./**//f"),
    ("shopt -s globstar", ".//**//f"),
];

/// The same, where the run is the pattern's root.
const REPEATED_SLASHES_AT_THE_ROOT: &[(&str, &str)] =
    &[("", "//t*"), ("", "///t*"), ("", "//tmp/")];

/// A non-empty `GLOBIGNORE` hides `.` and `..` however the pattern
/// spelled them, and hides nothing else about a `.` in mid-path.
const IGNORED_DOT_NAMES: &[(&str, &str)] = &[
    ("GLOBIGNORE=a", "*/."),
    ("GLOBIGNORE=a", "*/.."),
    ("GLOBIGNORE=zz", "*/."),
    ("GLOBIGNORE=zz", "*/.."),
    ("GLOBIGNORE=zz", "a/*/."),
    ("GLOBIGNORE=zz", "*/./f"),
    ("GLOBIGNORE=zz", "*/../f"),
    ("GLOBIGNORE=zz", "*/./"),
    ("GLOBIGNORE=zz", "*/../"),
    ("GLOBIGNORE=zz", "*/"),
    ("GLOBIGNORE=zz", "./*"),
    ("GLOBIGNORE=zz", "./a/*"),
    ("GLOBIGNORE=zz", "*"),
    ("GLOBIGNORE=zz", ".*"),
    ("GLOBIGNORE=a", ".*"),
    ("GLOBIGNORE=a", "*"),
    ("GLOBIGNORE=zz", "*/.d"),
    /* Turning `globskipdots` off does not put them back: the ignore list
     * hides them on its own. */
    ("shopt -u globskipdots; GLOBIGNORE=a", ".*"),
    ("shopt -u globskipdots; GLOBIGNORE=a", ".[.]*"),
    ("shopt -u globskipdots; GLOBIGNORE=zz", ".*"),
    ("shopt -u globskipdots; GLOBIGNORE=a", "*/."),
    /* An empty value is no ignore list, so the dot names come back to
     * whatever `globskipdots` says about them. */
    ("GLOBIGNORE=", "*/."),
    ("shopt -u globskipdots; GLOBIGNORE=", ".*"),
    /* The list is matched against the word the pattern generated, so it
     * has to be spelled with the same slash run the word carries. */
    ("GLOBIGNORE=b", "a//*"),
    ("GLOBIGNORE=a/b", "a//*"),
    ("GLOBIGNORE=a//b", "a//*"),
    ("GLOBIGNORE=*/b", "a//*"),
    ("GLOBIGNORE=t*", "//t*"),
];

/// Where a `**` that carries the word on leaves it, in a tree with a link
/// to a directory above it and a link to the directory it is in.
#[cfg(unix)]
const LINKS_UNDER_A_GLOBSTAR: &[(&str, &str)] = &[
    ("shopt -s globstar", "**"),
    ("shopt -s globstar", "**/"),
    ("shopt -s globstar", "**/f"),
    ("shopt -s globstar", "**/*"),
    ("shopt -s globstar", "**/*/f"),
    ("shopt -s globstar", "**/**/f"),
    ("shopt -s globstar", "**/**/**/f"),
    ("shopt -s globstar", "**/d/**/f"),
    ("shopt -s globstar", "./**/f"),
    ("shopt -s globstar", "d/**/f"),
    ("shopt -s globstar", "d/**/"),
    ("shopt -s globstar", "d/**"),
    ("shopt -s globstar", "*/**"),
    ("shopt -s globstar", "*/**/"),
    ("shopt -s globstar", "*/**/*"),
    ("shopt -s globstar", "*/**/f"),
    ("shopt -s globstar", "**//f"),
    ("shopt -s globstar; shopt -s dotglob", "*/**/*"),
];

#[cfg(not(unix))]
const LINKS_UNDER_A_GLOBSTAR: &[(&str, &str)] = &[];

/// The same, in the fixture the globstar work already measures links in.
#[cfg(unix)]
const LINKS_BESIDE_A_GLOBSTAR: &[(&str, &str)] = &[
    ("shopt -s globstar", "**/f"),
    ("shopt -s globstar", "*/**/f"),
    ("shopt -s globstar", "./**/f"),
    ("shopt -s globstar", "d/**/f"),
    ("shopt -s globstar", "ldir/**/f"),
    ("shopt -s globstar", "*/**"),
    ("shopt -s globstar", "**/"),
    ("shopt -s globstar", "lf/**"),
];

#[cfg(not(unix))]
const LINKS_BESIDE_A_GLOBSTAR: &[(&str, &str)] = &[];

/// The three trees the tables above are measured in.
fn fixture() -> PathBuf {
    /* One root per call, because the tables run as concurrent threads of
     * one process and a shared root is one test deleting another's tree. */
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let ordinal = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut root = std::env::temp_dir();
    root.push(format!("nsh-dotnames-{}-{ordinal}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for directory in ["tree/a/b", "tree/e", "tree/.d", "link/d", "sym/d"] {
        std::fs::create_dir_all(root.join(directory)).expect("create a fixture directory");
    }
    for file in [
        "tree/f",
        "tree/a/f",
        "tree/a/b/f",
        "tree/.hidden",
        "tree/.d/f",
        "link/d/f",
        "sym/d/f",
    ] {
        std::fs::File::create(root.join(file)).expect("create a fixture file");
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("d", root.join("link/ldir")).expect("link to a directory");
        std::os::unix::fs::symlink("d/f", root.join("link/lf")).expect("link to a file");
        std::os::unix::fs::symlink("../d", root.join("sym/d/inner")).expect("link back up");
        std::os::unix::fs::symlink(".", root.join("sym/loop")).expect("link to this directory");
    }
    root
}

/// A script that prints each generated field bracketed and on one line.
///
/// A pattern of `-` marks a prelude that speaks for itself — an option
/// report or a variable — and generates no fields of its own.
fn script(prelude: &str, pattern: &str) -> String {
    if pattern == "-" {
        return format!("{prelude}\n");
    }
    format!("{prelude}\nset -- {pattern}\nfor x; do printf '<%s>' \"$x\"; done\nprintf '\\n'\n")
}

/// Feed one script to a shell running in `directory` and return its
/// standard output and status.
fn answer(shell: &Path, dialect: &[&str], directory: &Path, script: &str) -> (Vec<u8>, i32) {
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
    (output.stdout, output.status.code().unwrap_or(-1))
}

/// Every case in `cases` generates the reference's fields, in the tree
/// the caller names.
fn agrees(tree: &str, cases: &[(&str, &str)]) {
    let root = fixture();
    let directory = root.join(tree);
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let bash = pinned_bash::path();
    for (prelude, pattern) in cases {
        let script = script(prelude, pattern);
        let ours = answer(nsh, &["-o", "bash"], &directory, &script);
        let theirs = answer(&bash, &[], &directory, &script);
        assert_eq!(
            String::from_utf8_lossy(&ours.0),
            String::from_utf8_lossy(&theirs.0),
            "fields differed for `{pattern}` under `{prelude}` in {tree}"
        );
        assert_eq!(
            ours.1, theirs.1,
            "status differed for `{pattern}` under `{prelude}` in {tree}"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// `.` and `..` are names `globskipdots` hides, and it is on to start.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn the_dot_names_are_skipped_unless_asked_for() {
    agrees("tree", DOT_NAMES);
}

/// A run of slashes is the word's text until something has been matched.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn a_repeated_slash_survives_into_the_generated_word() {
    agrees("tree", REPEATED_SLASHES);
}

/// The root is a run like any other, and `//` is the one POSIX reserves.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn a_repeated_slash_at_the_root_survives_too() {
    agrees("tree", REPEATED_SLASHES_AT_THE_ROOT);
}

/// `GLOBIGNORE` hides `.` and `..` however the pattern spelled them.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn an_ignore_list_hides_the_dot_names() {
    agrees("tree", IGNORED_DOT_NAMES);
}

/// A `**` the word carries on past matches the rest inside a link to a
/// directory, unless it is the `**` the pattern opens with.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn a_later_globstar_carries_the_word_through_links() {
    agrees("sym", LINKS_UNDER_A_GLOBSTAR);
    agrees("link", LINKS_BESIDE_A_GLOBSTAR);
}
