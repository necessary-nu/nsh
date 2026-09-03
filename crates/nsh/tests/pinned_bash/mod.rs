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

use std::path::PathBuf;
use std::process::Command;

/// The pinned Bash, named by `NSH_FUZZ_BASH` or found beside the build
/// tree, and checked against the version the calibration record holds.
///
/// The pin itself is read out of that record by the same string search
/// `nsh::fuzzing::reference` uses, so the two cannot drift apart; that
/// module sits behind a feature these tests do not turn on.
pub fn path() -> PathBuf {
    let path = std::env::var_os("NSH_FUZZ_BASH").map_or_else(
        || {
            PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../target/bash-reference/bash"
            ))
        },
        PathBuf::from,
    );
    assert!(
        path.exists(),
        "no pinned Bash at {}, so what this file records cannot be checked \
         against the reference that produced it\n\
         build it and name it to the run:\n\
         \x20   cargo run -p nsh-survey -- build-bash-reference\n\
         \x20   (or point NSH_FUZZ_BASH at an existing pinned build)",
        path.display()
    );

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
