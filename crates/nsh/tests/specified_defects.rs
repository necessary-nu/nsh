//! Regression gate for the defect-authority boundary.

use std::fs;
use std::path::{Path, PathBuf};

fn rust_sources(root: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).expect("read Rust source directory") {
        let path = entry.expect("read source entry").path();
        if path.is_dir() {
            rust_sources(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

// [spec:nsh:sem:idiom.specified-defects+1/test]
#[test]
fn core_has_no_defect_preservation_directives() {
    const FORBIDDEN: &[&str] = &[
        "bug-for-bug",
        "Reproduced verbatim, not fixed",
        "reproduce the dangling return",
        "reproduce the fall-through",
        "reproduce the dead test",
        "reproduce the arithmetic, not the intent",
    ];

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_sources(&root, &mut files);
    let mut findings = Vec::new();
    for path in files {
        let source = fs::read_to_string(&path).expect("Rust source is UTF-8");
        for forbidden in FORBIDDEN {
            if source.contains(forbidden) {
                findings.push(format!("{} contains {forbidden:?}", path.display()));
            }
        }
    }

    assert!(
        findings.is_empty(),
        "Dash defects may be described as provenance, but not prescribed in the core:\n{}",
        findings.join("\n")
    );
}
