//! What `hash` takes and what it prints, measured against the pinned
//! Bash 5.3.
//!
//! POSIX gives `hash` only `-r`, and dash takes only `-r`. The reference
//! takes `[-lr] [-p pathname] [-dt] [name ...]`, and those four extra
//! letters are the only script-visible handle a shell has on its command
//! table: `-p` pins a name to a path without searching for it, `-t` reads
//! one back, `-d` forgets a single name, and `-l` prints the table as
//! `builtin hash -p` lines that re-enter it. The bare listing differs
//! too -- a `hits`/`command` header and a per-entry consultation count
//! where dash prints the path alone.
//!
//! THE LETTERS AND THE COLUMNS ARE BASH'S ALONE, so the POSIX dialect
//! must go on reading all four as illegal options and printing dash's
//! listing. That half cannot be a differential here -- dash is not wired
//! into this crate's harness the way `pinned_bash` wires the reference
//! Bash -- so it is recorded, against `tests/.build/ref/src/dash`
//! 0.5.12-12 on 2026-09-05 at load 25: `hash -p /bin/true tt`,
//! `hash -t ls`, `hash -d ls` and `hash -l` each report `Illegal option`
//! with status 2 and let the next command run, and a bare `hash` prints
//! the path with no header and no count.
//!
//! Nothing in the differential rows is a recorded expectation. Every case
//! runs in both shells and the two answers are compared, so there is no
//! literal to go stale: if Bash changes its mind, this reports it rather
//! than passing. Diagnostic wording is registered as differing in
//! `docs/divergences.md`, so only standard output and the exit status are
//! read -- but *whether* a case reported is still measured, because a
//! refusal shows up in the status and in the commands that no longer run.
//!
//! NO CASE HERE LISTS MORE THAN ONE ENTRY, and the reason is not
//! insertion order. Measured 2026-09-05 at load 58: hashing `ls` then
//! `cat` and hashing `cat` then `ls` both list `ls` before `cat` in the
//! reference, so the order is neither the order they were hashed in nor
//! the byte order of the names -- it is the order of Bash's own hash
//! buckets, which nothing outside Bash can reproduce. This shell's table
//! is a `BTreeMap` and lists `cat` before `ls` either way. That is the
//! divergence `docs/divergences.md` registers for this listing, widened
//! to the Bash dialect's columns rather than a new one, and a two-entry
//! row here would be asserting the divergence rather than the agreement.
//! `hash -t a b` is not affected and does have a two-name row: it prints
//! in the order the names were *asked for*, not the order they are held.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// The listing, its header, and what the consultation count counts.
const LISTS_THE_TABLE: &[&str] = &[
    /* An empty table is a sentence, not silence. */
    "hash\n",
    "ls >/dev/null 2>&1\nhash\n",
    /* Every use of an entry raises the count. */
    "ls >/dev/null 2>&1\nls >/dev/null 2>&1\nls >/dev/null 2>&1\nhash\n",
    /* Writing an entry restarts it, so a name hashed by hand reports 0
     * however many times the command has already run. */
    "hash ls\nhash\n",
    "ls >/dev/null 2>&1\nls >/dev/null 2>&1\nhash ls\nhash\n",
    /* `-r` empties the table; with operands it empties and then hashes
     * them, which dash does not do. */
    "ls >/dev/null 2>&1\nhash -r\nhash\n",
    "ls >/dev/null 2>&1\nhash -r ls\nhash\n",
    /* `-l` re-enters the table as commands, and says nothing at all
     * about an empty one. */
    "ls >/dev/null 2>&1\nhash -l\n",
    "hash -l\necho st=$?\n",
];

/// `-p` pins a path, and the pin is what runs.
const PINS_A_PATH: &[&str] = &[
    "hash -p /bin/true tt\necho st=$?\nhash\n",
    "hash -p /bin/true tt uu\nhash -l\n",
    /* Reading the pin back is itself a consultation, so the count is 2
     * by the time the program has also run once. */
    "hash -p /bin/true tt\nhash -t tt\ntt\necho ran=$?\nhash\n",
    /* The pin beats a PATH search for a name PATH would have answered. */
    "ls >/dev/null 2>&1\nhash -p /bin/true ls\nls\necho st=$?\n",
    /* Nothing checks the path when it is written, so a pin to a path
     * that is not there is only discovered by running it. */
    "hash -p /nonexistent/zzz zzz\nzzz\necho st=$?\n",
];

/// `-t` reads an entry back and `-d` forgets one, neither searching PATH.
const READS_AND_FORGETS: &[&str] = &[
    "ls >/dev/null 2>&1\nhash -t ls\necho st=$?\n",
    /* One name prints the path alone; two or more label each line. */
    "cat </dev/null\nls >/dev/null 2>&1\nhash -t cat ls\necho st=$?\n",
    /* `-t` does not fall back to a PATH search: a name that is not in
     * the table is not found, even when the command exists. */
    "hash -t ls\necho st=$?\n",
    "hash -t nosuchcommandanywhere\necho st=$?\n",
    "ls >/dev/null 2>&1\nhash -d ls\necho st=$?\nhash\n",
    "hash -d nosuchcommandanywhere\necho st=$?\n",
    /* Operands with `-l` are names to hash, not names to list. */
    "ls >/dev/null 2>&1\nhash -l ls\necho st=$?\n",
];

/// What the option scan refuses, and whether the refusal ends the shell.
const REFUSES: &[&str] = &[
    /* A letter that is not one of the five. */
    "hash -z\necho after=$?\n",
    /* `-p` with no name to give the path to is a usage error, where a
     * `-t` or `-d` with no name is the milder "wanted an argument". The
     * two statuses differ and both shells agree on which is which. */
    "hash -p /bin/true\necho after=$?\n",
    "hash -t\necho after=$?\n",
    "hash -d\necho after=$?\n",
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

/// The `hits`/`command` listing and the count it prints.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn the_listing_counts_what_bash_counts() {
    agrees(LISTS_THE_TABLE);
}

/// `hash -p` writes a path the shell will use without checking it.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn a_pinned_path_stands_in_for_the_search() {
    agrees(PINS_A_PATH);
}

/// `hash -t` and `hash -d` address one entry at a time.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn one_entry_is_read_back_or_forgotten() {
    agrees(READS_AND_FORGETS);
}

/// A letter that is not one of them, and a letter with nothing to take.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn the_option_scan_refuses_what_bash_refuses() {
    agrees(REFUSES);
}

/// The POSIX dialect has no such letters and keeps dash's listing.
///
/// Recorded rather than differential for the reason the module comment
/// gives. Each letter is an illegal option worth status 2, and the shell
/// carries on, so the `echo` after it is what proves the refusal was not
/// fatal.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn the_default_dialect_still_has_no_such_letters() {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    for script in [
        "hash -p /bin/true tt\necho after=$?\n",
        "hash -t ls\necho after=$?\n",
        "hash -d ls\necho after=$?\n",
        "hash -l\necho after=$?\n",
    ] {
        let (stdout, status) = pinned_bash::answer(nsh, &[], script);
        assert_eq!(status, 0, "the shell did not carry on past\n{script}");
        assert_eq!(
            String::from_utf8_lossy(&stdout),
            "after=2\n",
            "status for\n{script}"
        );
    }
    /* dash's listing is the path alone: no header, no count, and the
     * name nowhere on the line. */
    let (stdout, status) = pinned_bash::answer(nsh, &[], "ls >/dev/null 2>&1\nhash\n");
    assert_eq!(status, 0);
    let listed = String::from_utf8_lossy(&stdout);
    assert!(listed.ends_with("/ls\n"), "listed {listed:?}");
    assert!(!listed.contains("hits"), "listed {listed:?}");
}
