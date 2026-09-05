#![allow(dead_code)]

use bstr::BStr;
use nsh::{Shell, Streams};
use std::ffi::OsStr;
use std::io::Write as _;
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

pub struct Outcome {
    pub stdout: Vec<u8>,
    pub status: i32,
    pub refused: bool,
}

pub fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

pub fn under_nsh(script: &[u8], env: Vec<(Vec<u8>, Vec<u8>)>) -> Option<Outcome> {
    let streams = Streams::capture().ok()?;
    let mut shell = Shell::builder()
        .streams(streams)
        .option(BStr::new(b"bash"), true)
        .env(env)
        .build()
        .ok()?;
    let outcome = shell.run(script);
    let refused = outcome.is_err();
    let status = outcome.map_or(0, |status| status.code().into());
    let stdout = shell.take_captured_stdout().ok()?.to_vec();
    drop(shell.take_captured_stderr());
    Some(Outcome {
        stdout,
        status,
        refused,
    })
}

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
/// `crates/nsh/tests/pinned_bash/mod.rs` and `nsh-survey`'s
/// `bash_reference::location` answer the same question for the
/// differential tests and for the survey gate. This is a third copy for
/// the reason they are two: `fuzz/` is a cargo workspace of its own, so
/// it can reach neither without the dependency edge
/// `struct.differential-helper-crate` exists to add. Change all three or
/// none.
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

/// The pinned Bash every differential target is judged against.
///
/// `[dec:nsh:differential-is-the-oracle]` is only worth anything if the
/// oracle is the Bash the repository pins. This used to be
/// `Command::new("bash")` with the child's PATH fixed to `/usr/bin:/bin`,
/// which is not "whatever is on PATH" so much as "always /usr/bin/bash" --
/// 5.2.37 where `calibrate-bash-5-3-oracle` pins 5.3.15.
///
/// `NSH_FUZZ_BASH` names it and is taken as the whole answer: a run that
/// names its own oracle has answered the question, and searching past a
/// name that is wrong would hide the mistake behind whatever else the
/// machine has. Otherwise the search runs over the checkouts this
/// repository has -- this one, then the one it shares a history with --
/// because the oracle is a build artefact and a linked worktree taken
/// for a campaign has no build tree of its own.
///
/// Wherever it is found, it must not live under `/tmp`: the fuzz
/// containment mounts an empty tmpfs there, so an oracle kept in `/tmp`
/// is invisible to every case, and the pinned build's default location
/// is exactly that.
// [spec:nsh:req:oracle.cannot-measure-is-a-failure]
fn reference_bash() -> &'static PathBuf {
    static SHELL: OnceLock<PathBuf> = OnceLock::new();
    SHELL.get_or_init(|| {
        let tried = std::env::var_os("NSH_FUZZ_BASH").map_or_else(
            || {
                let checkout = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
                let checkout =
                    std::fs::canonicalize(checkout).unwrap_or_else(|_| checkout.to_owned());
                let mut places = vec![checkout.join(UNDER_A_CHECKOUT)];
                if let Some(shared) = main_checkout(&checkout).filter(|shared| *shared != checkout)
                {
                    places.push(shared.join(UNDER_A_CHECKOUT));
                }
                places
            },
            |named| vec![PathBuf::from(named)],
        );
        // Refuse rather than score. A target that cannot reach its
        // reference has measured nothing, and the one thing it must never
        // do is say so quietly. Listing every place it looked is the
        // difference between a reader seeing a broken machine and a
        // reader seeing a question about layout.
        let path = tried
            .iter()
            .find(|candidate| candidate.exists())
            .cloned()
            .unwrap_or_else(|| {
                panic!(
                    "the differential oracle is not usable: no pinned Bash at any of the \
                     places this run looked:\n{}build it, or name a pinned \
                     build this machine already has:\n\
                     \x20   cargo run -p nsh-survey -- build-bash-reference\n\
                     \x20   NSH_FUZZ_BASH=/path/to/pinned/bash <command>",
                    tried
                        .iter()
                        .map(|candidate| format!("  {}\n", candidate.display()))
                        .collect::<String>(),
                )
            });
        if let Err(reason) = nsh::fuzzing::reference::verify(&path) {
            panic!("the differential oracle is not usable: {reason}");
        }
        path
    })
}

/// A script or an output as it goes into a report.
///
/// A fingerprint identifies a divergence across runs and says nothing
/// about what it was, so every triage began by writing a program to
/// recover the script from the artifact. Printable bytes go through as
/// themselves, everything else in octal, and the whole thing is cut at a
/// screenful -- the head of a differing script is what names it.
pub fn shown(bytes: &[u8]) -> String {
    const LIMIT: usize = 600;
    let mut out = String::new();
    for byte in bytes.iter().take(LIMIT) {
        match byte {
            b'\n' => out.push_str("\\n"),
            b'\t' => out.push_str("\\t"),
            b'\\' => out.push_str("\\\\"),
            0x20..=0x7e => out.push(char::from(*byte)),
            _ => out.push_str(&format!("\\{byte:03o}")),
        }
    }
    if bytes.len() > LIMIT {
        out.push_str(&format!("... ({} bytes)", bytes.len()));
    }
    out
}

pub fn under_bash(script: &[u8], env: &[(Vec<u8>, Vec<u8>)]) -> Option<(Vec<u8>, i32)> {
    let shell = reference_bash();
    let mut child = Command::new(shell)
        .arg("-s")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .envs(
            env.iter()
                .map(|(name, value)| (OsStr::from_bytes(name), OsStr::from_bytes(value))),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        // Verified above, so a failure here is the harness breaking
        // rather than an input being uninteresting.
        .unwrap_or_else(|error| {
            panic!("cannot start the pinned Bash {}: {error}", shell.display())
        });

    /* A short write is Bash having stopped reading, which is an answer
     * about the script rather than a reason to measure nothing: the
     * output it produced before it stopped and the status it exited with
     * are both still the reference's. Returning `None` here made every
     * script that makes Bash exit early invisible to the comparison. */
    drop(
        child
            .stdin
            .take()
            .expect("the reference's standard input")
            .write_all(script),
    );

    let out = child.wait_with_output().ok()?;
    Some((out.stdout, out.status.code().unwrap_or(-1)))
}

pub fn assert_matches_bash(label: &str, data: &[u8], script: &[u8], env: Vec<(Vec<u8>, Vec<u8>)>) {
    let ours = under_nsh(script, env.clone())
        .expect("this shell could not be built or captured, which is the harness and not an input");
    let (theirs, their_status) = under_bash(script, &env)
        .expect("the pinned Bash could not be waited for, which is the harness and not an input");

    if ours.refused && their_status != 0 {
        return;
    }
    assert!(
        !ours.refused,
        "{label}: nsh refused a script Bash ran (input={:016x}, bash_status={their_status})\n\
         script: {}\n  bash: {}",
        fingerprint(data),
        shown(script),
        shown(&theirs),
    );
    assert!(
        ours.stdout == theirs && ours.status == their_status,
        "{label}: nsh/Bash disagreement (input={:016x})\n\
         script: {}\n   nsh: {} (status {})\n  bash: {} (status {their_status})",
        fingerprint(data),
        shown(script),
        shown(&ours.stdout),
        ours.status,
        shown(&theirs),
    );
}
