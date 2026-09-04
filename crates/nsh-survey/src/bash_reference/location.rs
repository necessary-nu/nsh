//! Which checkout of this repository holds the pinned Bash.
//!
//! The oracle is a build artefact, and a linked `git worktree` has no
//! build tree of its own: worktrees share one repository's history and
//! keep their own working files, so `target/` belongs to the checkout
//! that ran the build and to no other. A search anchored on the caller's
//! *source* directory therefore finds nothing from a worktree, on a
//! machine that has a calibrated Bash 5.3.15 one directory across -- and
//! since the WIP budget sends every concurrent worker into a worktree,
//! that is the ordinary case rather than an unusual one.
//!
//! The search is over the checkouts the repository has, then, and not
//! over the one the caller is standing in. Coming up empty is a failure
//! and never a skip: a check that cannot reach its reference has
//! measured nothing. The report names every place, because a single
//! missing path reads as a broken shell and a list of them reads as what
//! it is, a question about layout.
//!
//! `crates/nsh/tests/pinned_bash/mod.rs` answers the same question for
//! the differential tests and is deliberately a second copy of it.
//! Nothing in this workspace lets a test tree and another package's
//! binary share a module: a `#[path]` reaching across packages is what
//! `[spec:nsh:req:idiom.declared-module-tree]` refuses, and the
//! dependency edge that would make it honest is what
//! `struct.differential-helper-crate` exists to add. Until it does,
//! change both or neither -- two answers to "which Bash" is the drift
//! this whole file is about.
// [spec:nsh:req:oracle.cannot-measure-is-a-failure]

use std::path::{Path, PathBuf};
use std::process::Command;

/// Where a build of this repository leaves the pinned Bash, relative to
/// the checkout that ran it.
const UNDER_A_CHECKOUT: &str = "target/bash-reference/bash";

/// The pinned Bash, or a report of every place it was looked for.
///
/// `NSH_FUZZ_BASH` is asked first and is taken as the whole answer: a run
/// that names its own oracle has answered the question, and searching on
/// past a name that is wrong would hide the mistake behind whatever else
/// the machine happens to have. It is the existing way to say where the
/// oracle is, and it keeps that meaning exactly.
///
/// `checkout` is the root of the checkout the caller was compiled in --
/// `CARGO_MANIFEST_DIR/../..` for a crate in this workspace. It is asked
/// before the shared checkout so that a worktree which has built its own
/// reference is judged against that one.
pub fn locate(checkout: &Path) -> Result<PathBuf, String> {
    if let Some(named) = std::env::var_os("NSH_FUZZ_BASH") {
        let named = PathBuf::from(named);
        if named.exists() {
            return Ok(named);
        }
        return Err(nowhere(&[("NSH_FUZZ_BASH names it", named)]));
    }

    let checkout = std::fs::canonicalize(checkout).unwrap_or_else(|_| checkout.to_path_buf());
    let mut tried = vec![("this checkout's own build", checkout.join(UNDER_A_CHECKOUT))];
    if let Some(shared) = main_checkout(&checkout).filter(|shared| *shared != checkout) {
        tried.push((
            "the checkout this worktree shares a repository with",
            shared.join(UNDER_A_CHECKOUT),
        ));
    }
    for (_, candidate) in &tried {
        if candidate.exists() {
            return Ok(candidate.clone());
        }
    }
    Err(nowhere(&tried))
}

/// The checkout git keeps this repository's shared directories in.
///
/// A linked worktree's `.git` is a file pointing into the main
/// checkout's, and `--git-common-dir` is git's own name for the
/// directory the two share; the checkout is that directory's parent.
/// Asked of git rather than parsed here because the pointer may be
/// relative, may be reached through a second worktree, and has been
/// spelled more than one way across the versions of `git worktree` this
/// machine has seen. `--path-format=absolute` is what makes the answer
/// independent of the directory it was asked from.
///
/// `None` for anything that is not a linked worktree of a repository
/// with a working tree, which includes "git is not installed": the
/// caller then reports the one place it did look, which is a better
/// answer than a failure about git.
fn main_checkout(from: &Path) -> Option<PathBuf> {
    let asked = Command::new("git")
        .arg("-C")
        .arg(from)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        .ok()?;
    if !asked.status.success() {
        return None;
    }
    let common = PathBuf::from(String::from_utf8(asked.stdout).ok()?.trim());
    common.parent().map(Path::to_path_buf)
}

/// What to say when none of the places has one.
///
/// Every candidate is named with the reason it was a candidate, because
/// the reader's next question after "it is not there" is "where is
/// there", and a single path invites the answer "then the machine has no
/// Bash" when the machine has one somewhere else.
fn nowhere(tried: &[(&str, PathBuf)]) -> String {
    let mut report = String::from(
        "no pinned Bash. This compares a shell against the GNU Bash this repository \
         pins and calibrated, and none of the places a checkout keeps one has it:\n",
    );
    for (why, path) in tried {
        report.push_str("  ");
        report.push_str(&path.display().to_string());
        report.push_str("\n      ");
        report.push_str(why);
        report.push('\n');
    }
    report.push_str(
        "build it, or name a pinned build this machine already has:\n\
         \x20   cargo run -p nsh-survey -- build-bash-reference\n\
         \x20   NSH_FUZZ_BASH=/path/to/pinned/bash <command>",
    );
    report
}
