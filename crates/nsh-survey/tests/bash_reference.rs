use std::path::Path;
use std::process::Command;

// [spec:nsh:req:compat.bash.reference-profile/test]
#[test]
fn committed_bash_reference_profile_is_complete() {
    let survey = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/surveys/oils");
    let output = Command::new(env!("CARGO_BIN_EXE_nsh-survey"))
        .arg("verify-bash-reference")
        .arg(survey)
        .output()
        .expect("run the Bash reference verifier");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("verified Bash 5.3.15(1)-release reference"),
        "stdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
}
