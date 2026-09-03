//! The shell a run measures: its own copy of it, and whether it was
//! built from the tree it is about to be scored against.
//!
//! `target/gate/bash` AND `target/bash-mode/bash` ARE SHARED MUTABLE
//! FILES, and both were the documented recipe. `gate-bash` refuses a
//! shell whose basename is not exactly `bash`, because `argv[0]` selects
//! the dialect, so every README told the reader to copy
//! `target/release/nsh` to one fixed path first. In a checkout several
//! sessions share, that path is as shared as any other build output.
//!
//! Measured on 2026-09-02: `target/gate/bash` was written at 12:21 with
//! sha `18bbaf3c...` and by 12:22 it was `73d97...`, another session's
//! build from a source state that was not mine. Two `run-oils --baseline`
//! runs a minute apart disagreed about which cases were failing, and
//! neither answer was about the binary I had built. The run header's
//! `shell sha256` is the only witness; the summary, the gate verdict and
//! the baseline comparison all read as a clean result for whatever
//! happened to be at the path.
//!
//! SO THE TOOL MAKES THE COPY. `--shell` names the binary to measure and
//! the runner installs it, under the name the run needs it to have, in a
//! directory nothing else writes and that is removed when the run ends.
//! There is no fixed path left to collide on, the `cp` leaves the
//! documented recipe, and a build that lands mid-run cannot change what a
//! run is scoring -- the copy is taken once, before the first case.
//!
//! AND A SHELL OLDER THAN ITS SOURCES IS NOT THE SHELL YOU MEANT. The
//! copy fixes two runs disagreeing; it does nothing about one run
//! measuring a binary that predates the change it is supposed to be
//! testing. `cargo build --release -p nsh` leaves `target/release/nsh`
//! untouched -- the binary belongs to `nsh-cli` -- and a measurement then
//! silently describes the previous build. That is a standing house rule
//! precisely because it has happened, and a rule is a worse place for it
//! than a check.

use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::process::ScratchTree;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// The shell under test, installed where only this run can reach it.
///
/// The directory goes when this value does, so a run that ends any way at
/// all leaves nothing behind for the next one to pick up by mistake.
pub(crate) struct ShellUnderTest {
    scratch: ScratchTree,
    name: OsString,
}

impl ShellUnderTest {
    /// Copy `shell` somewhere private and call it `name`.
    ///
    /// The source is resolved first, so a symlink is installed as the
    /// bytes it points at rather than as a link whose target another
    /// session can swing. `fs::copy` carries the permission bits, which
    /// is what makes the copy executable.
    pub(crate) fn install(shell: &Path, name: &OsStr) -> Result<Self> {
        let source = fs::canonicalize(shell).map_err(|error| {
            format!(
                "cannot resolve shell {}: {error}; build it with \
                 `cargo build --release -p nsh-cli`",
                shell.display()
            )
        })?;
        let scratch = ScratchTree::new()?;
        let installed = Self {
            scratch,
            name: name.to_owned(),
        };
        fs::copy(&source, installed.path()).map_err(|error| {
            format!(
                "cannot install {} as {}: {error}",
                source.display(),
                installed.path().display()
            )
        })?;
        Ok(installed)
    }

    pub(crate) fn path(&self) -> PathBuf {
        self.scratch.path().join(&self.name)
    }
}

/// The name a run measuring against `expectation` must run its shell
/// under.
///
/// `argv[0]` selects the dialect, so a shell scored against Bash's
/// recorded answers has to be called `bash` or it answers a different
/// question -- which is where the 793 in `bash_gate`'s log came from: 80
/// of 873 eligible cases passing, reported as a measurement of Bash
/// compatibility. Every other expectation namespace keeps whatever name
/// the caller's binary already has, because a POSIX run wants the POSIX
/// dialect.
pub(crate) fn name_for(expectation: &str, shell: &Path) -> OsString {
    if expectation == "bash" {
        return OsString::from("bash");
    }
    shell
        .file_name()
        .map_or_else(|| OsString::from("nsh"), OsStr::to_owned)
}

/// Files a shell built in this workspace is built from.
///
/// Deliberately coarse: every Rust source and manifest under `crates`,
/// plus the workspace manifest and lock. A finer list would have to know
/// which crate the shell came from and which of them `include_str!` each
/// other, and being wrong in that direction is a check that passes when
/// it should not.
fn is_a_source(path: &Path) -> bool {
    path.extension() == Some(OsStr::new("rs")) || path.file_name() == Some(OsStr::new("Cargo.toml"))
}

/// The newest moment any of those was written, and which one it was.
fn newest_source_change(root: &Path) -> Result<Option<(PathBuf, SystemTime)>> {
    let mut newest: Option<(PathBuf, SystemTime)> = None;
    let mut consider = |path: PathBuf| -> Result<()> {
        let changed = fs::metadata(&path)?.modified()?;
        if newest.as_ref().is_none_or(|(_, seen)| changed > *seen) {
            newest = Some((path, changed));
        }
        Ok(())
    };
    for name in ["Cargo.toml", "Cargo.lock"] {
        let path = root.join(name);
        if path.is_file() {
            consider(path)?;
        }
    }
    let mut directories = vec![root.join("crates")];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                directories.push(path);
            } else if is_a_source(&path) {
                consider(path)?;
            }
        }
    }
    Ok(newest)
}

/// How far a shell may lag its sources before it is a different build.
///
/// Not zero: `cargo` writes the binary while the sources sit still, so
/// the binary is normally the newer of the two by seconds. A minute is
/// wide enough that a build finishing while another session saves a file
/// is not a complaint, and narrow enough that the case this exists for --
/// `-p nsh` instead of `-p nsh-cli`, leaving the previous build in place
/// entirely -- is minutes to hours out.
const LAG: Duration = Duration::from_secs(60);

/// What is wrong with measuring this shell, if it predates its sources.
pub(crate) fn built_before_its_sources(shell: &Path) -> Result<Option<String>> {
    judged_against(&crate::provenance::checkout_root()?, shell)
}

/// The judgement itself, against a named checkout so it can be asked of
/// one built for the question.
///
/// Only a shell under that checkout's `target/` is judged, because only
/// those claim to have been built here. `target/bash-reference/` is
/// exempt: it holds GNU Bash built from Bash's own sources by
/// `build-bash-reference`, its provenance is `BASH_REFERENCE.toml` and a
/// build receipt, and it is older than every Rust file here by design.
///
/// BOTH SIDES OF THAT COMPARISON ARE RESOLVED, and the reason is a bug it
/// had. `target` is a symbolic link in any checkout whose build tree was
/// moved to another filesystem -- which is what a full root disk drives
/// people to, and the house rules point every session at a private
/// worktree -- and a shell reached through the link canonicalizes outside
/// the unresolved prefix. `starts_with` then said the binary belonged to
/// somebody else and the staleness check declined to judge it, silently,
/// because "not ours" is a legitimate answer for `/bin/sh`.
fn judged_against(root: &Path, shell: &Path) -> Result<Option<String>> {
    let Ok(shell) = fs::canonicalize(shell) else {
        return Ok(None);
    };
    let Ok(builds) = fs::canonicalize(root.join("target")) else {
        return Ok(None);
    };
    if !shell.starts_with(&builds) {
        return Ok(None);
    }
    if fs::canonicalize(builds.join("bash-reference"))
        .is_ok_and(|reference| shell.starts_with(reference))
    {
        return Ok(None);
    }
    let built = fs::metadata(&shell)?.modified()?;
    let Some((source, changed)) = newest_source_change(root)? else {
        return Ok(None);
    };
    let Ok(lag) = changed.duration_since(built) else {
        return Ok(None);
    };
    if lag <= LAG {
        return Ok(None);
    }
    Ok(Some(format!(
        "{} was written {} seconds before {} was last changed, so it was not built from \
         the tree it is about to be measured against. Rebuild it with \
         `cargo build --release -p nsh-cli` -- `-p nsh` leaves target/release/nsh alone, \
         and every number taken from it then describes the previous build.",
        shell.display(),
        lag.as_secs(),
        source.display(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dialect is chosen by the name, so the name is chosen by the
    /// expectation and not by the caller's spelling.
    // [spec:nsh:req:compat.bash.survey-closure/test]
    #[test]
    fn a_bash_expectation_runs_the_shell_as_bash() {
        assert_eq!(
            name_for("bash", Path::new("target/release/nsh")),
            OsString::from("bash")
        );
        assert_eq!(
            name_for("osh", Path::new("target/release/nsh")),
            OsString::from("nsh")
        );
        assert_eq!(
            name_for("dash", Path::new("/usr/bin/dash")),
            OsString::from("dash")
        );
    }

    /// The copy is the run's own, resolves a link, and goes when the run
    /// does.
    ///
    /// The link matters because the collision this exists for is a path
    /// another session rewrites: installing a link would leave the
    /// swinging target in the loop.
    // [spec:nsh:req:compat.bash.survey-closure/test]
    #[test]
    fn each_run_gets_its_own_named_copy() {
        let scratch = ScratchTree::new().unwrap();
        let real = scratch.path().join("nsh");
        fs::write(&real, b"#!/bin/sh\nexit 7\n").unwrap();
        let link = scratch.path().join("linked");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let installed = ShellUnderTest::install(&link, OsStr::new("bash")).unwrap();
        let path = installed.path();
        assert_eq!(path.file_name(), Some(OsStr::new("bash")));
        assert_eq!(fs::read(&path).unwrap(), b"#!/bin/sh\nexit 7\n");
        assert!(!path.is_symlink(), "the run installed a link, not a shell");

        let second = ShellUnderTest::install(&link, OsStr::new("bash")).unwrap();
        assert_ne!(
            path,
            second.path(),
            "two runs were given the same shell path to collide on",
        );

        drop(second);
        drop(installed);
        assert!(!path.exists(), "the run left its shell behind");
    }

    /// A shell older than the sources it claims to be built from is
    /// named, and one that is merely not under `target/` is nobody's
    /// business.
    // [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
    #[test]
    fn a_shell_older_than_its_sources_is_named() {
        assert!(
            built_before_its_sources(Path::new("/bin/sh"))
                .unwrap()
                .is_none(),
            "a shell outside this checkout's target was judged against our sources",
        );

        let root = crate::provenance::checkout_root().unwrap();
        let (newest, _) = newest_source_change(&root)
            .unwrap()
            .expect("crates has sources");
        assert!(
            newest.starts_with(&root),
            "the newest source is outside the checkout: {}",
            newest.display(),
        );

        let scratch = ScratchTree::new().unwrap();
        let stale = scratch.path().join("nsh");
        fs::write(&stale, b"shell").unwrap();
        /* Under `target/`, so it is judged; written in 1971, so it loses.
         * The scratch tree lives under target/ for exactly this reason. */
        let ancient = SystemTime::UNIX_EPOCH + Duration::from_secs(60 * 60 * 24 * 365);
        fs::File::open(&stale)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(ancient))
            .unwrap();
        let complaint = built_before_its_sources(&stale)
            .unwrap()
            .expect("a shell from 1971 was accepted as built from today's tree");
        assert!(complaint.contains("nsh-cli"), "{complaint}");
        assert!(
            complaint.contains(&newest.display().to_string()),
            "the complaint did not name the source it lost to: {complaint}",
        );
    }

    /// A checkout built for the question, whose `target` is a directory
    /// or a link to one, holding a shell written at `built`.
    fn judge_a_scratch_checkout(root: &Path, link: bool, built: SystemTime) -> Option<String> {
        fs::create_dir_all(root.join("crates/nsh/src")).unwrap();
        fs::write(root.join("crates/nsh/src/main.rs"), b"fn main() {}").unwrap();
        let builds = if link {
            let elsewhere = root.join("build-tree");
            fs::create_dir(&elsewhere).unwrap();
            std::os::unix::fs::symlink(&elsewhere, root.join("target")).unwrap();
            elsewhere
        } else {
            let here = root.join("target");
            fs::create_dir(&here).unwrap();
            here
        };
        let shell = builds.join("nsh");
        fs::write(&shell, b"shell").unwrap();
        fs::File::open(&shell)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(built))
            .unwrap();
        judged_against(root, &root.join("target/nsh")).unwrap()
    }

    /// A shell is judged the same through a linked `target` as through a
    /// real one, in both directions.
    ///
    /// A checkout whose build tree was moved to another filesystem reaches
    /// it through a symbolic link, which is what a full root disk drives
    /// people to and what two sessions did on 2026-09-02. Resolving the
    /// shell and not the prefix it is compared against made every shell in
    /// such a checkout "not ours", so nothing was ever judged -- and "not
    /// ours" is a legitimate answer for `/bin/sh`, so it said nothing.
    // [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
    #[test]
    fn a_linked_target_is_judged_like_a_directory() {
        let scratch = ScratchTree::new().unwrap();
        let ancient = SystemTime::UNIX_EPOCH + Duration::from_secs(60 * 60 * 24 * 365);
        for (name, link) in [("plain", false), ("linked", true)] {
            let stale = scratch.path().join(format!("{name}-stale"));
            assert!(
                judge_a_scratch_checkout(&stale, link, ancient).is_some(),
                "a shell from 1971 under a {name} target was accepted as built from today's tree",
            );
            let current = scratch.path().join(format!("{name}-current"));
            assert!(
                judge_a_scratch_checkout(&current, link, SystemTime::now()).is_none(),
                "a shell built now under a {name} target was called stale",
            );
        }
    }
}
