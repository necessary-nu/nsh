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

const RAW_ERROR_API_FRAGMENTS: &[&str] = &[
    "pub fn error_message_code",
    "pub fn not_found_error_code",
    "pub fn permission_denied_error_code",
    "pub const BAD_DESCRIPTOR",
    "pub fn path_error_is(code",
    "pub fn command_exec_failure_status(code",
];

fn inspect_tree(path: &Path, fragments: &[&str], violations: &mut Vec<String>) {
    for entry in std::fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            inspect_tree(&path, fragments, violations);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let source = std::fs::read_to_string(&path).unwrap();
            for fragment in fragments {
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
        inspect_tree(
            &workspace.join(source),
            FORBIDDEN_SOURCE_FRAGMENTS,
            &mut violations,
        );
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

// [spec:nsh:req:idiom.platform-errors/test]
#[test]
fn platform_errors_are_typed() {
    let missing = nsh_platform::platform_error(nsh_platform::PlatformErrorKind::NotFound);
    let denied = nsh_platform::platform_error(nsh_platform::PlatformErrorKind::PermissionDenied);
    let bad_descriptor =
        nsh_platform::platform_error(nsh_platform::PlatformErrorKind::BadDescriptor);

    assert!(nsh_platform::is_path_error(
        &missing,
        nsh_platform::PathErrorKind::NotFound,
    ));
    assert_eq!(missing.kind(), std::io::ErrorKind::NotFound);
    assert_eq!(denied.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(nsh_platform::is_bad_descriptor_error(&bad_descriptor));
    assert_eq!(nsh_platform::command_exec_failure_status(&missing), 127);
    assert_eq!(nsh_platform::command_exec_failure_status(&denied), 126);

    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for source in [
        "crates/nsh-platform/src/locale.rs",
        "crates/nsh-platform/src/unix.rs",
        "crates/nsh-platform/src/windows.rs",
    ] {
        let text = std::fs::read_to_string(workspace.join(source)).unwrap();
        for fragment in RAW_ERROR_API_FRAGMENTS {
            assert!(
                !text.contains(fragment),
                "{source} exposes raw error API fragment {fragment:?}",
            );
        }
    }
}

// [spec:nsh:req:idiom.descriptor-materialization/test]
#[test]
fn descriptor_materialization_is_transactional() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let unix = std::fs::read_to_string(workspace.join("crates/nsh-platform/src/unix.rs")).unwrap();
    let windows =
        std::fs::read_to_string(workspace.join("crates/nsh-platform/src/windows.rs")).unwrap();

    for source in [&unix, &windows] {
        assert!(source.contains("pub struct ProcessDescriptorTransaction"));
        assert!(!source.contains("ProcessFdChanges"));
        let implementation = source
            .split_once("impl ProcessDescriptorTransaction")
            .expect("descriptor transaction has an implementation")
            .1;
        assert!(implementation.contains("pub fn apply(self)"));
    }

    assert_eq!(unix.matches("libc::dup2(").count(), 1);
    assert_eq!(unix.matches("libc::close(*target)").count(), 1);
    let implementation = unix
        .split_once("impl ProcessDescriptorTransaction")
        .unwrap()
        .1;
    assert!(implementation.contains("libc::dup2("));
    assert!(implementation.contains("libc::close(*target)"));
}

// [spec:nsh:req:idiom.filesystem-account-bytes/test]
#[test]
fn filesystem_apis_keep_native_strings() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for source in [
        "crates/nsh-platform/src/unix.rs",
        "crates/nsh-platform/src/windows.rs",
    ] {
        let text = std::fs::read_to_string(workspace.join(source)).unwrap();
        assert!(text.contains("pub fn named_user_home(name: &OsStr) -> Option<PathBuf>"));
        assert!(text.contains("pub name: OsString"));
        assert!(text.contains("pub fn anonymous_file(name: impl AsRef<OsStr>)"));
        assert!(!text.contains("pub fn anonymous_file(name: &CStr"));
        assert!(!text.contains("pub fn create_temporary_file(name: &str"));
    }

    let mut violations = Vec::new();
    inspect_tree(
        &workspace.join("crates/nsh/src"),
        &["anonymous_file(c\""],
        &mut violations,
    );
    assert!(
        violations.is_empty(),
        "core anonymous-file callers expose C strings:\n{}",
        violations.join("\n"),
    );
}
