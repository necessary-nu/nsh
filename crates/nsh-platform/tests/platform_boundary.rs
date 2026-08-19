use std::path::Path;

const FORBIDDEN_SOURCE_FRAGMENTS: &[&str] = &[
    "std::os::unix",
    "std::os::windows",
    "std::os::fd",
    "libc::",
    "rustix::",
    "nshedit_plat::",
    "cfg(target_os",
    "cfg(target_family",
    "cfg(target_vendor",
    "cfg!(target_os",
    "cfg!(target_family",
    "cfg!(target_vendor",
];

fn inspect_tree(path: &Path, violations: &mut Vec<String>) {
    for entry in std::fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            inspect_tree(&path, violations);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let source = std::fs::read_to_string(&path).unwrap();
            for fragment in FORBIDDEN_SOURCE_FRAGMENTS {
                if source.contains(fragment) {
                    violations.push(format!("{} contains {fragment:?}", path.display()));
                }
            }
        }
    }
}

#[test]
fn shell_crates_do_not_bypass_the_platform_boundary() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut violations = Vec::new();
    for source in ["crates/nsh/src", "crates/nsh-cli/src"] {
        inspect_tree(&workspace.join(source), &mut violations);
    }

    for manifest in ["crates/nsh/Cargo.toml", "crates/nsh-cli/Cargo.toml"] {
        let text = std::fs::read_to_string(workspace.join(manifest)).unwrap();
        for dependency in ["libc =", "rustix =", "nshedit-plat ="] {
            if text
                .lines()
                .any(|line| line.trim_start().starts_with(dependency))
            {
                violations.push(format!("{manifest} directly depends on {dependency}"));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "platform boundary violations:\n{}",
        violations.join("\n")
    );
}
