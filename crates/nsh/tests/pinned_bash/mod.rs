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
