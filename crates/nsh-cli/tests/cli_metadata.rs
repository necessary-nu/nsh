use std::process::Command;

const NSH: &str = env!("CARGO_BIN_EXE_nsh");

// [spec:nsh:req:cli.metadata-options/test]
#[test]
fn help_is_successful() {
    let output = Command::new(NSH).arg("--help").output().expect("run nsh");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(!output.stdout.is_empty());
    assert!(output.stdout.windows(6).any(|bytes| bytes == b"Usage:"));
}

// [spec:nsh:req:cli.metadata-options/test]
#[test]
fn version_comes_from_the_package() {
    let output = Command::new(NSH)
        .arg("--version")
        .output()
        .expect("run nsh");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        output.stdout,
        format!("nsh {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
    );
}

// [spec:nsh:req:cli.metadata-options/test]
#[test]
fn metadata_spellings_after_c_are_arguments() {
    let output = Command::new(NSH)
        .args([
            "-c",
            "printf '<%s>\\n' \"$0\" \"$@\"",
            "--help",
            "--version",
            "-h",
        ])
        .output()
        .expect("run nsh");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, b"<--help>\n<--version>\n<-h>\n");
}

// [spec:nsh:req:cli.metadata-options/test]
#[test]
fn short_h_retains_the_hashall_option() {
    let output = Command::new(NSH)
        .args(["-h", "-c", "case $- in *h*) echo hashall;; esac"])
        .output()
        .expect("run nsh");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, b"hashall\n");
}
