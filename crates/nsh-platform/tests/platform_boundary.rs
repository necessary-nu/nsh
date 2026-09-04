use std::path::Path;

/// A module's whole text: the file, and every file in the directory
/// beside it.
///
/// Every assertion below is about how the platform boundary is
/// *written*, not about which file a declaration lands in, so splitting
/// `unix.rs` or `windows.rs` by subject must not change the answer.
/// Reading one file would let a forbidden shape stop being seen by
/// moving it one file down, and would fail a commit that moved code and
/// changed nothing.
fn module(relative: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates");
    let mut text = std::fs::read_to_string(root.join(format!("{relative}.rs"))).unwrap_or_default();
    let directory = root.join(relative);
    let mut nested = Vec::new();
    if directory.is_dir() {
        for entry in std::fs::read_dir(&directory).expect("a module directory is readable") {
            let path = entry.expect("a module entry is readable").path();
            if path.extension().is_some_and(|extension| extension == "rs") {
                nested.push(path);
            }
        }
        nested.sort();
    }
    for path in nested {
        text.push('\n');
        text.push_str(&std::fs::read_to_string(path).expect("a module file is readable"));
    }
    assert!(!text.is_empty(), "no source at crates/{relative}.rs");
    text
}

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

    for source in [
        "nsh-platform/src/unix/locale",
        "nsh-platform/src/unix",
        "nsh-platform/src/windows",
    ] {
        let text = module(source);
        for fragment in RAW_ERROR_API_FRAGMENTS {
            assert!(
                !text.contains(fragment),
                "{source} exposes raw error API fragment {fragment:?}",
            );
        }
    }
}

// [spec:nsh:req:idiom.exec-boundary/test]
#[test]
fn exec_boundary_owns_native_values() {
    let image = nsh_platform::ProgramImage::new(
        "utility".into(),
        vec!["utility".into(), "argument".into()],
        vec![("NAME".into(), "value".into())],
    );
    let _: fn(nsh_platform::ProgramImage) -> std::io::Error = nsh_platform::execute_program;
    drop(image);

    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let core = std::fs::read_to_string(workspace.join("crates/nsh/src/execution.rs")).unwrap();
    let unix = module("nsh-platform/src/unix");
    let windows = module("nsh-platform/src/windows");

    assert!(core.contains("ProgramImage::new("));
    assert!(!core.contains("CString"));
    assert!(!core.contains("argument_pointers"));
    assert!(unix.contains("argument_pointers.push(std::ptr::null())"));
    assert!(unix.contains("environment_pointers.push(std::ptr::null())"));
    for source in [&unix, &windows] {
        assert!(source.contains("execute_program(program: crate::ProgramImage)"));
    }
}

// [spec:nsh:req:idiom.descriptor-materialization/test]
#[test]
fn descriptor_materialization_is_transactional() {
    let unix = module("nsh-platform/src/unix");
    let windows = module("nsh-platform/src/windows");

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
    for source in ["nsh-platform/src/unix", "nsh-platform/src/windows"] {
        let text = module(source);
        assert!(text.contains("pub fn named_user_home(name: &OsStr) -> Option<PathBuf>"));
        assert!(text.contains("pub name: OsString"));
        assert!(text.contains("pub fn anonymous_file(name: impl AsRef<OsStr>)"));
        assert!(!text.contains("pub fn anonymous_file(name: &CStr"));
        assert!(!text.contains("pub fn create_temporary_file(name: &str"));
    }

    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
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

// [spec:posix:req:builtin.cd.step8-canonical-form-dot-dot/test]
#[test]
fn logical_parent_requires_directory() {
    for source in ["nsh-platform/src/unix", "nsh-platform/src/windows"] {
        let text = module(source);
        let parent_arm = text
            .split_once("[spec:posix:req:builtin.cd.step8-canonical-form-dot-dot]")
            .expect("logical parent handling carries its POSIX rule")
            .1;
        assert!(parent_arm.contains("path_is_directory"));
    }
}

// [spec:nsh:def:idiom.process-identity/test]
// [spec:nsh:req:idiom.process-group-zero-state/test]
#[test]
fn process_identities_are_typed() {
    assert!(nsh_platform::ProcessId::new(0).is_none());
    assert!(nsh_platform::ProcessGroupId::new(0).is_none());

    let process = nsh_platform::current_process_id();
    let group = nsh_platform::ProcessGroupId::from_leader(process);
    assert_eq!(process.get(), group.get());
    assert_ne!(
        nsh_platform::ProcessGroupState::OutsideNamespace,
        nsh_platform::ProcessGroupState::Visible(group),
    );

    let targets = [
        nsh_platform::ProcessTarget::Process(process),
        nsh_platform::ProcessTarget::CurrentProcessGroup,
        nsh_platform::ProcessTarget::ProcessGroup(group),
        nsh_platform::ProcessTarget::AllProcesses,
    ];
    assert_eq!(targets.len(), 4);

    let _: fn(nsh_platform::ProcessTarget, nsh_platform::SignalRequest) -> std::io::Result<()> =
        nsh_platform::send_signal;
    let _: fn() -> nsh_platform::ProcessId = nsh_platform::current_process_id;
    let _: fn() -> Option<nsh_platform::ProcessId> = nsh_platform::parent_process_id;
    let _: fn() -> nsh_platform::ProcessGroupState = nsh_platform::current_process_group;
    // The written-out signature is the assertion: factoring it into a
    // type alias would hide the very shape this test exists to pin.
    #[expect(
        clippy::type_complexity,
        reason = "the spelled-out signature is what this test pins"
    )]
    let _: fn(
        bool,
        bool,
    )
        -> std::io::Result<Option<(nsh_platform::ProcessId, nsh_platform::ChildStatus)>> =
        nsh_platform::wait_for_any_child;
    /* The wait a shell uses. It names the process, so the identity is in
     * the signature rather than in the caller's head, and a status this
     * caller is not entitled to is one it cannot ask for. */
    #[expect(
        clippy::type_complexity,
        reason = "the spelled-out signature is what this test pins"
    )]
    let _: fn(
        nsh_platform::ProcessId,
        bool,
        bool,
    )
        -> std::io::Result<Option<(nsh_platform::ProcessId, nsh_platform::ChildStatus)>> =
        nsh_platform::wait_for_child;
    let _: fn(
        nsh_platform::ProcessSelector,
        nsh_platform::ProcessGroupState,
    ) -> std::io::Result<()> = nsh_platform::set_process_group;

    for source in ["nsh-platform/src/unix", "nsh-platform/src/windows"] {
        let text = module(source);
        for fragment in [
            "pub fn send_signal(pid: i32",
            "pub fn set_process_group(pid: i32",
            "pub fn set_foreground_process_group(fd: &impl AsDescriptor, group: i32",
            "pub fn current_process_id() -> i32",
            "pub fn wait_for_any_child(\n    nonblocking: bool,\n    report_stopped: bool,\n) -> std::io::Result<Option<(i32",
        ] {
            assert!(
                !text.contains(fragment),
                "{source} exposes raw process API fragment {fragment:?}",
            );
        }
    }

    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut violations = Vec::new();
    inspect_tree(
        &workspace.join("crates/nsh/src"),
        &[
            "pid: i32",
            "pid: c_int",
            "pgrp: i32",
            "pgrp: c_int",
            "backgndpid: i32",
            "root_pid: i32",
        ],
        &mut violations,
    );
    assert!(
        violations.is_empty(),
        "core process identities are raw integers:\n{}",
        violations.join("\n"),
    );
}

// [spec:nsh:def:idiom.signal-wait/test]
#[test]
fn signal_and_wait_values_are_typed() {
    assert!(nsh_platform::Signal::new(0).is_none());
    assert!(nsh_platform::Signal::new(-1).is_none());

    let interrupt = nsh_platform::interrupt_signal();
    assert!(interrupt.number() > 0);
    let requests = [
        nsh_platform::SignalRequest::Probe,
        nsh_platform::SignalRequest::Deliver(interrupt),
    ];
    assert_eq!(requests.len(), 2);

    let statuses = [
        nsh_platform::ChildStatus::Exited(0),
        nsh_platform::ChildStatus::Signaled {
            signal: interrupt,
            core_dumped: false,
        },
        nsh_platform::ChildStatus::Stopped(interrupt),
        nsh_platform::ChildStatus::Continued,
    ];
    assert_eq!(statuses.len(), 4);

    let _: fn(nsh_platform::ProcessTarget, nsh_platform::SignalRequest) -> std::io::Result<()> =
        nsh_platform::send_signal;
    let _: fn(nsh_platform::Signal) -> std::io::Result<nsh_platform::SignalAction> =
        nsh_platform::signal_action;
    // The written-out signature is the assertion: factoring it into a
    // type alias would hide the very shape this test exists to pin.
    #[expect(
        clippy::type_complexity,
        reason = "the spelled-out signature is what this test pins"
    )]
    let _: fn(
        bool,
        bool,
    )
        -> std::io::Result<Option<(nsh_platform::ProcessId, nsh_platform::ChildStatus)>> =
        nsh_platform::wait_for_any_child;

    for source in ["nsh-platform/src/unix", "nsh-platform/src/windows"] {
        let text = module(source);
        for fragment in [
            "pub fn interrupt_signal() -> i32",
            "pub fn send_signal(target: ProcessTarget, signal: i32",
            "pub fn signal_action(signal: i32",
            "pub fn wait_status_",
            "Option<(ProcessId, i32)>",
        ] {
            assert!(
                !text.contains(fragment),
                "{source} exposes raw signal/wait API fragment {fragment:?}",
            );
        }
    }

    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut violations = Vec::new();
    inspect_tree(
        &workspace.join("crates/nsh/src"),
        &["wait_status_"],
        &mut violations,
    );
    assert!(
        violations.is_empty(),
        "core decodes raw wait statuses:\n{}",
        violations.join("\n"),
    );
}
