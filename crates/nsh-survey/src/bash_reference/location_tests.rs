//! Where the pinned Bash is looked for, measured against a real
//! `git worktree` rather than a hand-built imitation of one.
//!
//! The layout under test is git's, not this crate's: a linked worktree's
//! `.git` is a file, the directory it points into is shared, and only
//! git can be trusted to say how that is spelled on the version
//! installed. A test that wrote those files itself would measure this
//! file's idea of a worktree and pass while the real thing failed.

use std::fs;
use std::path::{Path, PathBuf};

use super::location;
use crate::process::ScratchTree;
use crate::provenance::tests::git;

/// A checkout these tests can hang a worktree off.
///
/// The commit is not decoration: `git worktree add` needs a HEAD to
/// detach from, so a repository with nothing in it cannot be given the
/// layout under test.
fn checkout(root: &Path) {
    fs::create_dir_all(root).unwrap();
    git(root, &["init", "-q"]);
    fs::write(root.join("kept.rs"), b"fn a() {}\n").unwrap();
    git(root, &["add", "kept.rs"]);
    git(root, &["commit", "-q", "-m", "one"]);
}

/// A linked worktree of `main`, made by git so that its `.git` is
/// whatever git writes there rather than whatever this file believes.
fn linked_worktree(main: &Path) -> PathBuf {
    let worktree = main.parent().unwrap().join("linked");
    let named = worktree.to_str().unwrap();
    git(main, &["worktree", "add", "-q", "--detach", named]);
    worktree
}

/// A stand-in for the pinned build, where a build of this repository
/// leaves it.
fn built_oracle(checkout: &Path) -> PathBuf {
    let path = checkout.join("target/bash-reference/bash");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"#!/bin/sh\n").unwrap();
    path
}

/// The defect this exists for: a worktree has no build tree of its own,
/// so it must reach the one the repository has.
///
/// The oracle is put only in the main checkout, and the worktree is
/// asked. Before this resolution existed the answer was a missing path
/// inside the worktree, which is what made every differential test in
/// the workspace fail at once on a machine that had a pinned Bash.
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn a_worktree_reaches_the_build_its_repository_has() {
    let scratch = ScratchTree::new().unwrap();
    let main = scratch.path().join("main");
    checkout(&main);
    let built = built_oracle(&main);

    let worktree = linked_worktree(&main);
    assert!(
        !worktree.join("target").exists(),
        "a fresh worktree was expected to have no build tree",
    );

    let found = location::locate(&worktree).expect("the shared build was not reached");
    assert_eq!(
        fs::canonicalize(found).unwrap(),
        fs::canonicalize(built).unwrap()
    );
}

/// A worktree that has built its own reference is judged against that
/// one, because it is the build that belongs to the tree under test.
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn a_checkout_prefers_the_build_it_made_itself() {
    let scratch = ScratchTree::new().unwrap();
    let main = scratch.path().join("main");
    checkout(&main);
    built_oracle(&main);

    let worktree = linked_worktree(&main);
    let own = built_oracle(&worktree);

    let found = location::locate(&worktree).expect("a checkout could not find its own build");
    assert_eq!(
        fs::canonicalize(found).unwrap(),
        fs::canonicalize(own).unwrap()
    );
}

/// No reference anywhere is a failure that names every place, and never
/// a quiet pass.
///
/// The two paths are the whole point: one missing path reads as a broken
/// machine, and the list reads as the question it is -- which checkout
/// was supposed to have built this.
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn nowhere_to_look_names_everywhere_it_looked() {
    let scratch = ScratchTree::new().unwrap();
    let main = scratch.path().join("main");
    checkout(&main);

    let worktree = linked_worktree(&main);

    let refusal = location::locate(&worktree).expect_err("a missing reference was accepted");
    for named in [
        worktree.join("target/bash-reference/bash"),
        main.join("target/bash-reference/bash"),
    ] {
        assert!(
            refusal.contains(&named.display().to_string()),
            "{} is not in:\n{refusal}",
            named.display(),
        );
    }
    assert!(refusal.contains("build-bash-reference"), "{refusal}");
    assert!(refusal.contains("NSH_FUZZ_BASH"), "{refusal}");
}

/// A checkout that is nobody's worktree has exactly one place to look,
/// and is not told about itself twice.
///
/// `--git-common-dir` answers for a main checkout as readily as for a
/// worktree, and there it names that checkout's own `.git`. A report
/// that listed the same path under two headings would be inviting the
/// reader to go and look somewhere they have already looked.
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn a_checkout_that_is_no_worktree_looks_once() {
    let scratch = ScratchTree::new().unwrap();
    let main = scratch.path().join("main");
    checkout(&main);

    let refusal = location::locate(&main).expect_err("a missing reference was accepted");
    let wanted = main.join("target/bash-reference/bash");
    assert!(refusal.contains(&wanted.display().to_string()), "{refusal}");
    assert_eq!(
        refusal.matches("bash-reference/bash").count(),
        1,
        "one checkout was reported as two places:\n{refusal}",
    );

    let built = built_oracle(&main);
    assert_eq!(
        fs::canonicalize(location::locate(&main).unwrap()).unwrap(),
        fs::canonicalize(built).unwrap(),
    );
}
