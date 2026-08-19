use std::path::Path;
use std::process::Command;

// [spec:nsh:req:compat.smoosh.ifs-launch/test]
#[test]
fn pinned_ifs_reaches_three_shells() {
    let survey = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/surveys/smoosh");
    let output = Command::new(env!("CARGO_BIN_EXE_nsh-survey"))
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
        .arg(survey)
        .output()
        .expect("run the contained Smoosh IFS case");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("selected=186 executed=1 pass=1 fail=0 timeout=0 error=0 skip=185"),
        "stdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
}
