//! The pinned GNU Bash 5.3 the differential tests are judged against.
//!
//! [`dec:nsh:differential-is-the-oracle`] only means something if the
//! oracle is the Bash this repository pins. `calibrate-bash-5-3-oracle`
//! pinned 5.3 and recorded its identity beside the survey corpus, while
//! the ambient `/usr/bin/bash` on a development machine is typically 5.2
//! and is not an answer here.
//!
//! A reference that is not there is a failure and not a pass, so every
//! path out of this module is an assertion rather than an `Option` a
//! caller could skip on.
// [spec:nsh:req:oracle.cannot-measure-is-a-failure]
// [spec:nsh:req:compat.bash.reference-profile]

use std::path::{Path, PathBuf};
use std::process::Command;

/// Where a build of this repository leaves the pinned Bash, relative to
/// the checkout that ran it.
const UNDER_A_CHECKOUT: &str = "target/bash-reference/bash";

/// The checkout git keeps this repository's shared directories in.
///
/// A linked worktree has no build tree of its own -- worktrees share one
/// repository's history and keep their own working files -- so a build
/// artefact lives in whichever checkout ran the build, and from a
/// worktree that is the one git calls the main one. `--git-common-dir`
/// names its `.git`, and the checkout is that directory's parent. Asked
/// of git rather than parsed here because a worktree's pointer may be
/// relative, may be reached through a second worktree, and has been
/// spelled more than one way; `--path-format=absolute` is what makes the
/// answer independent of where it was asked from.
///
/// `None` covers everything that is not a worktree of a repository with
/// a working tree, "git is not installed" included, and the caller then
/// reports the one place it did look.
///
/// `nsh-survey`'s `bash_reference::location` answers the same question
/// for the survey gate and is deliberately a second copy: nothing in
/// this workspace lets a test tree and another package's binary share a
/// module without a dependency edge, and `struct.differential-helper-crate`
/// is the node that will make one. Change both or neither.
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

/// The pinned Bash, named by `NSH_FUZZ_BASH` or found in a checkout of
/// this repository, and checked against the version the calibration
/// record holds.
///
/// `NSH_FUZZ_BASH` is taken as the whole answer: a run that names its own
/// oracle has answered the question, and searching past a name that is
/// wrong would hide the mistake behind whatever else the machine has.
///
/// The pin itself is read out of that record by the same string search
/// `nsh::fuzzing::reference` uses, so the two cannot drift apart; that
/// module sits behind a feature these tests do not turn on.
///
/// The record is read from *this* checkout while the shell may come from
/// another, and that is deliberate: the record is a tracked source file
/// and belongs to the tree under test, the shell is a build artefact and
/// belongs to whichever checkout built one. The version comparison below
/// is what holds the two together.
pub fn path() -> PathBuf {
    let tried = std::env::var_os("NSH_FUZZ_BASH").map_or_else(
        || {
            let checkout = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
            let checkout = std::fs::canonicalize(checkout).unwrap_or_else(|_| checkout.to_owned());
            let mut places = vec![checkout.join(UNDER_A_CHECKOUT)];
            if let Some(shared) = main_checkout(&checkout).filter(|shared| *shared != checkout) {
                places.push(shared.join(UNDER_A_CHECKOUT));
            }
            places
        },
        |named| vec![PathBuf::from(named)],
    );
    let path = tried
        .iter()
        .find(|candidate| candidate.exists())
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "no pinned Bash, so what this file records cannot be checked against the \
                 reference that produced it. None of the places a checkout of this \
                 repository keeps one has it:\n{}build it, or name a pinned build this \
                 machine already has:\n\x20   cargo run -p nsh-survey -- build-bash-reference\
                 \n\x20   NSH_FUZZ_BASH=/path/to/pinned/bash <command>",
                tried
                    .iter()
                    .map(|candidate| format!("  {}\n", candidate.display()))
                    .collect::<String>(),
            )
        });

    let record = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/surveys/oils/BASH_REFERENCE_CASES.json"
    ))
    .expect("read the Bash calibration record");
    let at = record
        .find("\"oracle_version\"")
        .expect("the record names an oracle_version");
    let tail = &record[at..];
    let open = tail[16..].find('"').expect("a quoted oracle_version") + 17;
    let close = tail[open..].find('"').expect("a terminated oracle_version");
    let pinned = &tail[open..open + close];

    let reported = Command::new(&path)
        .arg("--version")
        .output()
        .expect("run the pinned Bash");
    let reported = String::from_utf8_lossy(&reported.stdout);
    let first = reported.lines().next().unwrap_or_default();
    assert!(
        first.contains(pinned),
        "{} reports {first:?}, which is not the pinned {pinned:?}",
        path.display()
    );
    path
}

/// Run one script through one shell and return its standard output and status.
///
/// Three differential tests grew their own copy of this before the
/// duplication gate noticed the third. It lives here because it is the
/// other half of what these tests need from a pinned oracle: `path()`
/// says which Bash, and this says how a script is put to a shell so the
/// two answers are comparable. The environment is cleared to the same
/// three variables on both sides, because a differential test that lets
/// the caller's environment through is comparing two shells and a
/// terminal.
/// Included by test binaries that want only `path()`, so it is dead in
/// theirs and live in the three that compare two shells. Narrower than a
/// module-wide allowance, which `[spec:nsh:req:idiom.strict-lints]`
/// reserves for nothing at all.
#[allow(dead_code)]
pub fn answer(shell: &std::path::Path, dialect: &[&str], script: &str) -> (Vec<u8>, i32) {
    answer_with_env(shell, dialect, &[], script)
}

/// The same, with `environment` added to the cleared environment first.
///
/// Whether a shell *inherited* a name is a different question from
/// whether it published a default for one, and no script can tell the
/// two apart from inside a single shell: `TERM=dumb` reads the same
/// either way. The only way to ask is to start the shell twice.
#[allow(dead_code)]
pub fn answer_with_env(
    shell: &std::path::Path,
    dialect: &[&str],
    environment: &[(&str, &str)],
    script: &str,
) -> (Vec<u8>, i32) {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let mut child = Command::new(shell)
        .args(dialect)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .envs(environment.iter().copied())
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
