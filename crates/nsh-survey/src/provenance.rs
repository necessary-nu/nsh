//! Which commit the shell a survey run measured can be attributed to.
//!
//! A generated register is a claim about a build. `run-oils
//! --update-baseline` re-records `BASH_COMPARISON_FAILURES.toml` from
//! whatever `target/release/nsh` happens to hold, and in a checkout
//! several sessions share that binary is everyone's work at once. The
//! refresh has no way to know whose changes it just measured, and the
//! file it writes says nothing about it.
//!
//! IT HAS ALREADY PUT A FALSE STATEMENT IN THE PERMANENT RECORD.
//! `ee98cec` removed `assign.test.sh:19` and `assign.test.sh:45` from the
//! list and its message attributed both to "the associative-assignment
//! work". They did not pass at its parent: a shell built from `b028f47`
//! alone matches all 473 recorded ids, and the same shell built with one
//! other session's then-uncommitted files matches 471. The two ids left
//! the failing set because of a change nothing had committed, and the
//! refresh enshrined it under another node's name. The reverse is the
//! dangerous direction -- a refresh taken over half-finished work can
//! *add* a failing id that no commit explains, and the next person hunts
//! it in the wrong file.
//!
//! It is not a discipline problem. Every session in a shared checkout
//! builds the same `target/release/nsh` from the same working tree by
//! design; that is what `cargo build --release -p nsh-cli` does, and the
//! README's refresh command is the one everybody runs.
//!
//! So the refresh asks git what the checkout can vouch for, refuses a
//! tree it cannot, and -- when the caller insists, because their own fix
//! is necessarily uncommitted at the moment they re-record -- writes the
//! commit and every uncommitted path into the file it generates. A later
//! reader can then see which files' effects are in the list instead of
//! having to guess, which is the whole of what went wrong above.
//!
//! WHAT IT DOES NOT CLAIM. Git is asked about *this crate's* checkout,
//! which is the tree `cargo` builds from. A run that points `--shell` at
//! a binary from somewhere else is not detected, and cannot be: the
//! baseline records `shell_sha256` beside the commit so the bytes are at
//! least named.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// The one part of the tree whose state cannot reach a survey run.
///
/// nplan rewrites the plan on every `nplan start`, so a checkout is
/// modified here throughout the work that would want a refresh. Nothing
/// under it is compiled into a shell, read by the runner or copied into
/// the corpus, so counting it would make every refresh unvouched for a
/// reason that is never the reason -- and a refusal everybody expects is
/// a refusal nobody reads.
const NOT_AN_INPUT: &str = "plan/";

/// Run git in a checkout and hand back its bytes.
///
/// `--no-optional-locks` because the survey runs behind
/// `scripts/sandboxed`, which binds `/` read-only: git must not try to
/// refresh the index on the way past.
pub(crate) fn git_output(checkout: &Path, arguments: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(checkout)
        .args(arguments)
        .output()
        .map_err(|error| {
            format!(
                "cannot run git {} in {}: {error}",
                arguments.join(" "),
                checkout.display()
            )
        })?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed for {}: {}",
            arguments.join(" "),
            checkout.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(output.stdout)
}

/// What the checkout a measurement was taken in can vouch for.
#[derive(Debug)]
pub(crate) struct Provenance {
    /// The commit HEAD was on.
    pub(crate) commit: String,
    /// Every path in the tree that commit does not account for.
    pub(crate) uncommitted: Vec<String>,
}

impl Provenance {
    /// Read what the checkout holding `checkout` says, ignoring
    /// `subject` itself.
    ///
    /// The file about to be written is exempt because it is the output:
    /// a refresh that refused because its own target had been
    /// re-recorded would be refusing the thing it was asked to fix.
    /// Whether *that* difference is somebody else's work is a separate
    /// question, asked separately.
    pub(crate) fn read(checkout: &Path, subject: &Path) -> Result<Self> {
        let root = String::from_utf8(git_output(checkout, &["rev-parse", "--show-toplevel"])?)?
            .trim()
            .to_owned();
        let root = PathBuf::from(root);
        let commit = String::from_utf8(git_output(&root, &["rev-parse", "HEAD"])?)?
            .trim()
            .to_owned();
        let exempt = relative_to(&root, subject);
        let status = git_output(&root, &["status", "--porcelain=v1", "-z"])?;
        let mut uncommitted = changed_paths(&status)?;
        uncommitted.retain(|path| {
            !path.starts_with(NOT_AN_INPUT) && exempt.as_deref() != Some(path.as_str())
        });
        uncommitted.sort();
        uncommitted.dedup();
        Ok(Self {
            commit,
            uncommitted,
        })
    }

    /// This reading, if the refresh may stand on it.
    ///
    /// `allow_uncommitted` is `--update-baseline-from-dirty-tree`, and it
    /// is spelled out that long on purpose: in the ordinary case the
    /// caller's own fix is the uncommitted work, so the flag is not an
    /// escape hatch but the normal way to say "yes, and record what was
    /// in the tree". What the refusal buys is that the list of paths gets
    /// read before it is accepted -- `ee98cec` named seventeen files
    /// belonging to other sessions in its own violation trailer and still
    /// attributed the baseline move to one commit.
    fn vouched(self, subject: &Path, allow_uncommitted: bool) -> Result<Self> {
        if allow_uncommitted || self.uncommitted.is_empty() {
            return Ok(self);
        }
        Err(format!(
            "refusing to re-record {} from a checkout with {} uncommitted path(s):\n{}\n\
             The shell this run measured was built from this tree, so those paths' effects \
             are in the list and no commit explains them. Commit them and run again, or pass \
             --update-baseline-from-dirty-tree to record the list with every path above \
             named inside it.",
            subject.display(),
            self.uncommitted.len(),
            self.uncommitted
                .iter()
                .map(|path| format!("  {path}"))
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .into())
    }
}

/// The path `subject` names inside `root`, in git's own spelling.
fn relative_to(root: &Path, subject: &Path) -> Option<String> {
    let absolute = std::path::absolute(subject).ok()?;
    let relative = absolute.strip_prefix(root).ok()?;
    Some(relative.to_string_lossy().replace('\\', "/"))
}

/// Every path `git status --porcelain=v1 -z` named, renames included.
///
/// The `-z` form is the only one that cannot lie about a filename: the
/// default quotes anything unusual, and a survey that parsed the quoted
/// form would silently drop the paths hardest to notice. A rename emits
/// its two halves as two records; both are uncommitted, so both are kept
/// and the order between them does not matter.
fn changed_paths(status: &[u8]) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    let mut renamed = false;
    for record in status.split(|byte| *byte == 0) {
        if record.is_empty() {
            continue;
        }
        if renamed {
            renamed = false;
            paths.push(String::from_utf8(record.to_vec())?);
            continue;
        }
        if record.len() < 4 || record[2] != b' ' {
            return Err(format!(
                "cannot read git status record {:?}",
                String::from_utf8_lossy(record)
            )
            .into());
        }
        renamed = record[0] == b'R' || record[0] == b'C';
        paths.push(String::from_utf8(record[3..].to_vec())?);
    }
    Ok(paths)
}

/// Read this crate's checkout, refusing one that cannot vouch for the
/// shell the run just measured.
pub(crate) fn vouch(subject: &Path, allow_uncommitted: bool) -> Result<Provenance> {
    Provenance::read(Path::new(env!("CARGO_MANIFEST_DIR")), subject)?
        .vouched(subject, allow_uncommitted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::ScratchTree;
    use std::fs;

    /// git, in a checkout made for one test.
    ///
    /// Identity and default branch are given on the command line so the
    /// machine's own git configuration cannot decide what this test
    /// measures.
    fn git(repo: &Path, arguments: &[&str]) {
        let mut command = Command::new("git");
        command.arg("-C").arg(repo);
        for setting in [
            "init.defaultBranch=main",
            "user.email=survey@example.invalid",
            "user.name=survey",
            "commit.gpgsign=false",
        ] {
            command.arg("-c").arg(setting);
        }
        let status = command.args(arguments).output().expect("git runs");
        assert!(
            status.status.success(),
            "git {arguments:?}: {}",
            String::from_utf8_lossy(&status.stderr)
        );
    }

    /// The parse is the part that can silently lose a path.
    ///
    /// A modification, an addition, an untracked file and a rename, in
    /// the byte form git actually emits.
    // [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
    #[test]
    fn every_changed_path_survives_the_status_parse() {
        let status = b" M crates/nsh/src/variables.rs\0A  crates/nsh/src/new.rs\0\
            ?? crates/nsh/src/probe.rs\0R  after.rs\0before.rs\0";
        assert_eq!(
            changed_paths(status).unwrap(),
            [
                "crates/nsh/src/variables.rs",
                "crates/nsh/src/new.rs",
                "crates/nsh/src/probe.rs",
                "after.rs",
                "before.rs",
            ]
        );
    }

    /// A record git could not have written is an error rather than a
    /// silently short list.
    // [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
    #[test]
    fn an_unreadable_status_record_is_refused() {
        assert!(changed_paths(b"nonsense\0").is_err());
        assert!(changed_paths(b"").unwrap().is_empty());
    }

    /// This checkout is a git checkout, so the reading must work in it.
    // [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
    #[test]
    fn the_checkout_names_the_commit_it_is_on() {
        let baseline = crate::survey_root().join("BASH_COMPARISON_FAILURES.toml");
        let provenance =
            Provenance::read(Path::new(env!("CARGO_MANIFEST_DIR")), &baseline).unwrap();
        assert_eq!(provenance.commit.len(), 40, "{}", provenance.commit);
        assert!(
            provenance
                .commit
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        );
        assert!(
            !provenance
                .uncommitted
                .iter()
                .any(|path| path.starts_with("plan/")),
            "the plan was counted as an input: {:?}",
            provenance.uncommitted,
        );
        assert!(
            !provenance
                .uncommitted
                .iter()
                .any(|path| path.ends_with("BASH_COMPARISON_FAILURES.toml")),
            "the file being written was counted against itself: {:?}",
            provenance.uncommitted,
        );
    }

    /// The refusal this node exists for, against real git in a real
    /// checkout.
    ///
    /// A hand-built reading would exercise the arithmetic and not the
    /// half that has to agree with git, which is where a silently short
    /// list would come from.
    // [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
    #[test]
    fn a_checkout_carrying_uncommitted_work_is_refused() {
        let scratch = ScratchTree::new().unwrap();
        let repo = scratch.path();
        git(repo, &["init", "-q"]);
        fs::create_dir(repo.join("plan")).unwrap();
        fs::write(repo.join("kept.rs"), b"fn a() {}\n").unwrap();
        fs::write(repo.join("plan/main.styx"), b"one\n").unwrap();
        git(repo, &["add", "kept.rs", "plan/main.styx"]);
        git(repo, &["commit", "-q", "-m", "one"]);

        /* The file the refresh is about to write is its own output, and
         * an untracked one at that. It must not count against itself. */
        let baseline = repo.join("BASELINE.toml");
        fs::write(&baseline, b"# generated\n").unwrap();
        let clean = Provenance::read(repo, &baseline).unwrap();
        assert!(clean.uncommitted.is_empty(), "{:?}", clean.uncommitted);
        assert_eq!(clean.commit.len(), 40);
        assert!(clean.vouched(&baseline, false).is_ok());

        fs::write(repo.join("plan/main.styx"), b"two\n").unwrap();
        let planned = Provenance::read(repo, &baseline).unwrap();
        assert!(
            planned.uncommitted.is_empty(),
            "starting a node made the tree unvouchable: {:?}",
            planned.uncommitted,
        );

        fs::write(repo.join("kept.rs"), b"fn a() { b() }\n").unwrap();
        fs::write(repo.join("probe.rs"), b"fn b() {}\n").unwrap();
        let dirty = Provenance::read(repo, &baseline).unwrap();
        assert_eq!(dirty.uncommitted, ["kept.rs", "probe.rs"]);
        let refusal = dirty
            .vouched(&baseline, false)
            .expect_err("a refresh over two uncommitted files was allowed")
            .to_string();
        for named in ["kept.rs", "probe.rs", "--update-baseline-from-dirty-tree"] {
            assert!(refusal.contains(named), "{refusal}");
        }

        let recorded = Provenance::read(repo, &baseline)
            .unwrap()
            .vouched(&baseline, true)
            .expect("the spelled-out override was still refused");
        assert_eq!(
            recorded.uncommitted,
            ["kept.rs", "probe.rs"],
            "the override dropped what it was supposed to record",
        );
    }
}
