//! How `alias` prints an entry, measured against the pinned Bash 5.3.
//!
//! There are three shapes for one line and this shell already had two of
//! them. dash prints `'name=value'`, quoting the whole assignment; this
//! shell prints `name='value'` because
//! `[spec:posix:req:builtin.alias.stdout-format]` requires the name and
//! the equals sign unquoted, and that is the POSIX dialect's answer,
//! delivered by `fix-builtin-alias-stdout-format` and registered as
//! `alias_stdout_format` in `tests/harness/divergences.sh`. The reference
//! prints `alias name='value'`, so its listing re-enters as commands
//! rather than as assignments, and it takes `-p`.
//!
//! THE PREFIX IS NOT "WHAT BASH DOES" UNCONDITIONALLY, which is the part
//! worth knowing before reading a row here. Measured 2026-09-05 at load
//! 58, `bash --posix` drops it from a bare `alias` and from a name query
//! and keeps it for `-p`:
//!
//!     alias zz=1; alias        bash: alias zz='1'   bash --posix: zz='1'
//!     alias zz=1; alias -p     bash: alias zz='1'   bash --posix: alias zz='1'
//!
//! So the two references disagree with each other, and Bash mode is
//! measured against plain `bash` — the same choice `c3936a2` made for
//! `exec`'s letters and `5c5ebd2` for `hash`'s columns.
//!
//! THE POSIX DIALECT MUST NOT MOVE, and that half cannot be a
//! differential here: dash is not wired into this crate's harness the way
//! `pinned_bash` wires the reference Bash. It is recorded, against
//! `tests/.build/ref/src/dash` 0.5.12-12 on 2026-09-05 at load 58 —
//! `alias -p` is a name dash does not hold, so it reports `-p not found`
//! and fails, `alias -- -z` reports both words, and a bare listing is
//! `zz='1'` with no prefix. `the_default_dialect_keeps_its_own_shape`
//! pins all three.
//!
//! Nothing in the differential rows is a recorded expectation. Every case
//! runs in both shells and the two answers are compared, so there is no
//! literal to go stale. Diagnostic wording is registered as differing in
//! `docs/divergences.md`, so only standard output and the exit status are
//! read.
//!
//! NO VALUE HERE CONTAINS A SINGLE QUOTE. The two shells escape one
//! differently inside the quoted value — the reference writes
//! `zz='a'\''b'` and this shell `zz='a'"'"'b'` — which is a property of
//! the quoting helper in both dialects and is nothing to do with the
//! prefix this file is about.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// The listing, and the single-name query that is the same line.
const PRINTS_THE_TABLE: &[&str] = &[
    "alias zz=1\nalias\n",
    /* Both shells list in name order, so more than one entry is a fair
     * comparison here in a way it is not for `hash`. */
    "alias a=1 b=2\nalias\n",
    "alias b=2 a=1\nalias\n",
    /* An empty table is silence in both, unlike `hash`'s. */
    "alias\necho st=$?\n",
    /* A name query prints the same line the listing would. */
    "alias ll='ls -l'\nalias ll\n",
    "alias a=1 b=2\nalias b\n",
    /* A value with a space is quoted the same way by both. */
    "alias g='grep -n'\nalias\n",
    "alias zz=1\nunalias zz\nalias\necho st=$?\n",
];

/// `-p` prints the whole table and ignores everything after it.
const DASH_P_IS_THE_WHOLE_TABLE: &[&str] = &[
    "alias zz=1\nalias -p\necho st=$?\n",
    "alias -p\necho st=$?\n",
    /* Not a filter: the operands are neither queried nor reported, so a
     * name the table does not hold still succeeds. */
    "alias a=1 b=2\nalias -p nosuchalias\necho st=$?\n",
    "alias a=1 b=2\nalias -p a\necho st=$?\n",
    /* And not a definition either. */
    "alias -p zz=1\necho st=$?\nalias\necho second=$?\n",
    "alias zz=1\nalias -p -p\necho st=$?\n",
];

/// What the option scan refuses, and what it hands through as a name.
const REFUSES: &[&str] = &[
    /* A letter that is not `p`, on its own and clustered behind one. */
    "alias -z\necho after=$?\n",
    "alias -pz\necho after=$?\n",
    /* `--` ends the options, and what follows is a name rather than a
     * letter — so this is a miss, not a refusal. */
    "alias -- -z\necho after=$?\n",
    /* An ordinary missing name, and a word that is not a name at all. */
    "alias nosuchalias\necho after=$?\n",
    "alias =1\necho after=$?\n",
    /* A miss does not stop the words after it being defined. */
    "alias nosuchalias after=1\necho after=$?\nalias\n",
];

/// Both shells on one script, as `(what nsh said, what the pinned Bash
/// said)`.
fn both(script: &str) -> ((Vec<u8>, i32), (Vec<u8>, i32)) {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let bash = pinned_bash::path();
    (
        pinned_bash::answer(nsh, &["-o", "bash"], script),
        pinned_bash::answer(&bash, &[], script),
    )
}

/// Every script in `cases` produces the reference's bytes and status.
fn agrees(cases: &[&str]) {
    for script in cases {
        let (ours, theirs) = both(script);
        assert_eq!(
            String::from_utf8_lossy(&ours.0),
            String::from_utf8_lossy(&theirs.0),
            "output differed for\n{script}"
        );
        assert_eq!(ours.1, theirs.1, "status differed for\n{script}");
    }
}

/// Each line carries the `alias ` the reference puts in front of it.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn the_listing_re_enters_as_commands() {
    agrees(PRINTS_THE_TABLE);
}

/// `alias -p` is the listing spelled explicitly, operands and all.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn dash_p_prints_everything_and_asks_nothing() {
    agrees(DASH_P_IS_THE_WHOLE_TABLE);
}

/// A letter that is not `p`, and the words that are names instead.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn the_option_scan_refuses_what_bash_refuses() {
    agrees(REFUSES);
}

/// The POSIX dialect keeps dash's shape and has no `-p`.
///
/// Recorded rather than differential for the reason the module comment
/// gives. `alias -p` failing is the whole assertion for the letter: dash
/// reads it as a name, reports it, and answers 1.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn the_default_dialect_keeps_its_own_shape() {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let (stdout, status) = pinned_bash::answer(nsh, &[], "alias zz=1\nalias\n");
    assert_eq!(status, 0);
    assert_eq!(String::from_utf8_lossy(&stdout), "zz='1'\n");

    /* `-p` is a name, so it is reported and the listing never happens;
     * the `echo` is what shows the status it took. */
    let (stdout, status) = pinned_bash::answer(nsh, &[], "alias zz=1\nalias -p\necho after=$?\n");
    assert_eq!(status, 0, "the shell did not carry on past `alias -p`");
    assert_eq!(String::from_utf8_lossy(&stdout), "after=1\n");

    /* dash has no `--` either, so both words are reported. */
    let (stdout, status) = pinned_bash::answer(nsh, &[], "alias -- -z\necho after=$?\n");
    assert_eq!(status, 0);
    assert_eq!(String::from_utf8_lossy(&stdout), "after=1\n");
}
