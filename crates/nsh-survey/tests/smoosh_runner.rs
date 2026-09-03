use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn survey_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/surveys/smoosh")
}

fn run_pinned_ifs_case(binary: &Path) -> Output {
    Command::new(binary)
        .args([
            "run-smoosh",
            "--group",
            "full",
            "--shell",
            "/bin/sh",
            "--test",
            "sh.set.ifs.test",
            "--timeout-ms",
            "2000",
            "--format",
            "text",
        ])
        .arg(survey_root())
        .output()
        .expect("run the contained Smoosh IFS case")
}

fn assert_case_passed(output: &Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("selected=186 executed=1 pass=1 fail=0 timeout=0 error=0 skip=185"),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

// [spec:nsh:req:compat.smoosh.ifs-launch/test]
#[test]
fn pinned_ifs_reaches_three_shells() {
    assert_case_passed(&run_pinned_ifs_case(Path::new(env!(
        "CARGO_BIN_EXE_nsh-survey"
    ))));
}

/// Where the survey was built is not a property of what it measures.
///
/// The containment mounts an empty tmpfs over `/tmp` and the fixture's
/// `$TEST_SHELL` is the survey binary itself, so a survey built with
/// `CARGO_TARGET_DIR` under `/tmp` -- which is what a full root filesystem
/// forces -- has no shell at all inside the boundary. Every case then
/// exits 127 with no output, which the report cannot tell from a corpus
/// the shell genuinely fails.
///
/// `/tmp` is named literally rather than taken from `TMPDIR` because the
/// boundary names it literally too; a copy anywhere else would be masked
/// by nothing and the check would assert nothing.
// [spec:nsh:req:compat.smoosh.ifs-launch/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn a_survey_built_under_masked_tmp_still_measures() {
    let masked = Path::new("/tmp");
    assert!(
        masked.is_dir(),
        "the boundary masks /tmp, so /tmp must exist for this check to mean anything",
    );
    let directory = masked.join(format!("nsh-survey-masked-build-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let relocated = directory.join("nsh-survey");
    fs::copy(env!("CARGO_BIN_EXE_nsh-survey"), &relocated).unwrap();

    let output = run_pinned_ifs_case(&relocated);
    drop(fs::remove_dir_all(&directory));
    assert_case_passed(&output);
}
