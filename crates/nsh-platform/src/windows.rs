//! Windows implementation of the shell's operating-system boundary.
//! Handles stay opaque; POSIX concepts use native Windows primitives.
//!
//! This file is the table of contents. The subjects are its children,
//! and they are the same ones `unix.rs` divides into -- text, paths,
//! descriptors and the endpoints built on them, locale, terminal,
//! errors, signals, process -- plus two that only this host needs:
//! `spawn`, because Windows will not replace a running image, and
//! `broker`, because the process a native clone produces may not create
//! one.
//!
//! Every name published below is published by `unix.rs` too, under the
//! same conditions. The shell names them with no `cfg`, so a host short
//! of one fails in the shell on a target nobody here builds rather than
//! in the file that is short; `nsh-lint`'s
//! `hosts_publish_the_same_surface` compares the two lists so the
//! shortfall is reported where it is.
// [spec:nsh:req:idiom.platform-surface-parity]

/* The private items one child needs from another. A child sees this
 * module's names through `use super::*`, so a helper crosses from the
 * module whose subject it is to the module that calls it here, in one
 * place, rather than by widening it to the whole crate. */
use broker::{
    BROKER_CHANNEL, BROKER_PIPE, CLONE_BROKER, CloneBrokerChild, broker_handle_pair,
    clone_broker_main, execute_through_clone_broker, register_clone_broker,
};
use children::{CHILDREN, PROCESS_CLONE_LOCK, duplicate_owned_handle, set_descriptor_inherit};
use descriptor::{descriptor_from_file, duplicate_at, owned_handle, raw_handle};
use endpoints::{direct_pipe, materialized_standard_handles};
use paths::{DEFAULT_SEARCH_PATH, with_shell_path_separators};
use process::{CHILD_SYSTEM_TICKS, CHILD_USER_TICKS, filetime_ticks};
use signals::SIGNAL_EXIT_BASE;
use spawn::spawn_program_here;

/* The one subject neither host owns; the crate root declares it. */
use crate::{SIGNAL_COUNT, SIGNAL_NAMES};

mod broker;
mod children;
pub use children::{fork_process, run_in_child, wait_for_any_child, wait_for_child};
mod descriptor;
pub use descriptor::{
    AsDescriptor, BorrowedDescriptor, Descriptor, duplicate_cloexec, duplicate_fd, duplicate_file,
    move_fd_cloexec,
};
/* Gated as the POSIX file of this name is, though nothing here needs
 * `nshedit-plat` and the console answers without it. A name published on
 * one host and not the other is what this file's list is checked for,
 * and a name published in more builds is the same drift as a name
 * missing: an embedder that leaves the feature off would find these on
 * one host only. */
#[cfg(feature = "edit")]
mod editor_terminal;
#[cfg(feature = "edit")]
pub use editor_terminal::{
    EditorTerminalAttributes, TerminalApply, TerminalControlCharacter,
    apply_editor_terminal_attributes, editor_terminal_attributes, editor_terminal_size,
    wait_for_terminal_input,
};
mod endpoints;
pub use endpoints::{
    OpenMode, PIPE_BUFFER, ProcessDescriptorTransaction, anonymous_file, create_temporary_file,
    fd_is_regular_file, fd_is_seekable, open_null_device, open_path, open_pseudoterminal, pipe,
    read_exact, read_once, read_to_end, reports_pipe_short_writes, seek_relative, seek_start,
    set_nonblocking, snapshot_process_fd, supports_bidirectional_pseudoterminal_pair, supports_tee,
    take_file_contents, tee, write_all, write_once,
};
mod errors;
pub use errors::{
    PathErrorKind, command_exec_failure_status, is_bad_descriptor_error, is_exec_format_error,
    is_path_error, is_pseudoterminal_end, platform_error,
};
mod facts;
pub use facts::{
    GroupId, UserId, descriptor_limit, effective_gid, effective_uid, host_name, real_uid,
    supplementary_groups, wait_for_input,
};
mod locale;
pub use locale::{Locale, LocaleCategory, LocaleCharacter, LocaleDecode, LocaleDecoder};
mod paths;
pub use paths::{
    AccessMode, DirectoryEntry, FileKind, FileMetadata, absolute_path,
    can_unlink_current_directory, controlling_terminal_path, current_directory,
    default_search_path, effective_access, fallback_shell, logical_path, login_shell,
    named_user_home, null_device_path, open_history_file, path_exists, path_is_directory,
    path_is_file, path_is_same_file, path_metadata, read_directory, read_path, remove_file,
    resolve_command_path, run_editor, search_path_separator, set_current_directory,
    shell_directory_separator, shell_path_has_separator, shell_path_is_absolute,
    shell_path_last_separator, supports_glob_metacharacters_in_filenames,
};
mod process;
pub use process::{
    LimitResource, ProcessTimes, ResourceLimit, creation_mask, current_process_group,
    current_process_id, environment_text, exit_immediately, flush_coverage_profile,
    foreground_process_group, parent_process_id, process_arguments, process_environment,
    process_times, replace_creation_mask, reset_coverage_counters, resource_limit,
    restore_shell_process_runtime_state, set_foreground_process_group, set_process_group,
    set_resource_limit,
};
mod signals;
pub use signals::{
    BlockedSignals, SignalAction, child_signal, configure_here_document_writer_signals,
    continue_signal, hangup_signal, ignore_signal, install_signal_action, interrupt_signal,
    kill_signal, pipe_signal, quit_signal, raise_signal, send_continue_to_process_group,
    send_signal, signal_action, signal_is_blocked, terminal_input_signal, terminal_output_signal,
    terminal_stop_signal, terminate_with_interrupt, termination_signal, unblock_all_signals,
};
mod spawn;
pub use spawn::execute_program;
mod terminal;
pub use terminal::{TerminalSettings, is_terminal, terminal_canonical_mode};
mod text;
pub use text::{
    NativeStrExt, ShellBytesExt, input_newline_width, trim_command_substitution_output,
};

#[cfg(test)]
mod tests {
    use super::paths::{query_windows_directory, with_shell_path_separators};
    use super::*;
    use std::ffi::{OsStr, OsString};
    use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
    use std::path::{Path, PathBuf};
    use windows_sys::Win32::System::SystemInformation::{
        GetSystemDirectoryW, GetWindowsDirectoryW,
    };

    #[test]
    fn native_string_round_trip_preserves_unpaired_surrogates() {
        let original = OsString::from_wide(&[u16::from(b'a'), 0xd800, u16::from(b'z')]);
        let encoded = original.to_shell_bytes();
        assert_eq!(encoded, [b'a', 0xed, 0xa0, 0x80, b'z']);
        assert_eq!(
            encoded
                .try_to_os_string()
                .unwrap()
                .encode_wide()
                .collect::<Vec<_>>(),
            original.encode_wide().collect::<Vec<_>>()
        );
    }

    #[test]
    fn malformed_shell_bytes_are_rejected() {
        for malformed in [
            &[0xff][..],
            &[0xc2][..],
            &[0xc0, 0x80][..],
            &[0xe2, 0x28, 0xa1][..],
            &[0xf4, 0x90, 0x80, 0x80][..],
        ] {
            assert_eq!(
                malformed.try_to_os_string().unwrap_err().kind(),
                std::io::ErrorKind::InvalidData
            );
        }
    }

    #[test]
    fn supplementary_unicode_uses_canonical_utf8() {
        let original = OsString::from_wide(&[0xd83d, 0xde00]);
        let encoded = original.to_shell_bytes();
        assert_eq!(encoded, [0xf0, 0x9f, 0x98, 0x80]);
        assert_eq!(
            encoded
                .try_to_os_string()
                .unwrap()
                .encode_wide()
                .collect::<Vec<_>>(),
            [0xd83d, 0xde00]
        );
    }

    #[test]
    fn slash_is_the_shells_path_separator() {
        assert_eq!(shell_directory_separator(), b'/');
        assert!(shell_path_is_absolute(b"/rooted"));
        assert!(shell_path_is_absolute(b"C:/rooted"));
        assert!(!shell_path_is_absolute(b"relative/path"));
        assert!(!shell_path_is_absolute(br"\rooted"));
    }

    #[test]
    fn crlf_is_one_input_newline() {
        assert_eq!(input_newline_width(None), 1);
        assert_eq!(input_newline_width(Some(b'x')), 1);
        assert_eq!(input_newline_width(Some(b'\r')), 2);
    }

    #[test]
    fn logical_paths_are_lexical_and_absolute() {
        let current = Path::new("C:/one/two");
        assert_eq!(
            logical_path(Some(current), Path::new("./three")),
            Some(PathBuf::from("C:/one/two/three"))
        );
        assert_eq!(
            logical_path(Some(current), Path::new("../three")),
            Some(PathBuf::from("C:/one/three"))
        );
        assert_eq!(
            logical_path(Some(current), Path::new("/three")),
            Some(PathBuf::from("C:/three"))
        );
        assert_eq!(
            logical_path(Some(current), Path::new("C:/three/../four")),
            Some(PathBuf::from("C:/four"))
        );
        assert!(logical_path(Some(current), Path::new("C:relative")).is_none());
    }

    #[test]
    fn default_path_comes_from_windows() {
        let expected: Vec<_> = [
            query_windows_directory(GetSystemDirectoryW),
            query_windows_directory(GetWindowsDirectoryW),
        ]
        .into_iter()
        .flatten()
        .map(PathBuf::from)
        .collect();
        assert!(!expected.is_empty());
        assert_eq!(
            std::env::split_paths(&default_search_path()).collect::<Vec<_>>(),
            expected
        );
        assert!(expected.iter().all(|path| path.is_absolute()));
        assert!(!default_search_path().to_shell_bytes().contains(&b'\\'));
    }

    #[test]
    fn inherited_path_uses_the_shells_canonical_name() {
        let expected =
            with_shell_path_separators(std::env::var_os("PATH").expect("test process has PATH"));
        let inherited: Vec<_> = process_environment()
            .into_iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case(OsStr::new("PATH")))
            .collect();
        assert_eq!(inherited, [(OsString::from("PATH"), expected)]);
        assert!(!inherited[0].1.to_shell_bytes().contains(&b'\\'));
    }

    #[test]
    fn default_path_is_available_in_a_cloned_child() {
        let status = run_in_child(|| {
            exit_immediately(i32::from(default_search_path().is_empty()));
        })
        .unwrap();
        assert_eq!(status, 0);
    }

    #[test]
    fn command_substitution_removes_complete_windows_line_endings() {
        let mut output = b"prefix\r\n\r\n".to_vec();
        trim_command_substitution_output(&mut output, 0);
        assert_eq!(output, b"prefix");
    }

    #[test]
    fn pathext_resolves_an_extensionless_program() {
        let executable = std::env::current_exe().unwrap();
        let mut extensionless = executable.clone();
        extensionless.set_extension("");
        let environment = vec![(OsString::from("PATHEXT"), OsString::from(".exe"))];
        assert_eq!(
            resolve_command_path(&extensionless, &environment),
            executable
        );
    }

    #[test]
    fn a_cloned_process_can_request_another_pipe() {
        let status = run_in_child(|| {
            let result = (|| {
                let (read, write) = pipe()?;
                write_all(&write, b"brokered")?;
                drop(write);
                let bytes = read_to_end(&read)?;
                Ok::<_, std::io::Error>(bytes == b"brokered")
            })();
            exit_immediately(i32::from(!matches!(result, Ok(true))));
        })
        .unwrap();
        assert_eq!(status, 0);
    }
}
