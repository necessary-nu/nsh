/* The POSIX implementation of the shell's operating-system boundary,
 * and the table of contents for the subjects it divides into.
 *
 * There is no `//!` here, and there cannot be: this file is `include!`d
 * into the crate root, where an inner attribute is no longer the first
 * thing in the module. The doc for each subject is on the subject.
 *
 * Two of the children below predate the directory: `descriptor`,
 * `locale`, `terminal` and `editor_terminal` were split out one at a
 * time and sit beside this file in `src/`, while everything this change
 * moved is in `src/unix/`. `signal_names` is shared with the Windows
 * host and belongs in neither. */

mod descriptor;
pub use descriptor::{AsDescriptor, BorrowedDescriptor, Descriptor, move_fd_cloexec};
mod locale;
pub use locale::{Locale, LocaleCategory, LocaleDecode, LocaleDecoder};
mod signal_names;
pub use signal_names::{SIGNAL_COUNT, SIGNAL_NAMES};
mod terminal;
pub use terminal::{TerminalSettings, is_terminal, terminal_canonical_mode};
mod unix_facts;
pub use unix_facts::{
    GroupId, UserId, descriptor_limit, effective_gid, effective_uid, host_name, real_uid,
    supplementary_groups, wait_for_input,
};
/* The only user of `nshedit-plat`, and so the only part of the boundary
 * the `edit` feature gates. */
#[cfg(feature = "edit")]
mod editor_terminal;
#[cfg(feature = "edit")]
pub use editor_terminal::{
    EditorTerminalAttributes, TerminalApply, TerminalControlCharacter,
    apply_editor_terminal_attributes, editor_terminal_attributes, editor_terminal_size,
    wait_for_terminal_input,
};

/* The subject split of this file, in `src/unix/`. This file is
 * `include!`d into the crate root rather than declared as a module, so a
 * bare `mod paths;` here would name `src/paths.rs`; `#[path]` is what
 * puts the children in a directory named for their parent. The include
 * is load-bearing and stays: it is what makes `unix_facts` a descendant
 * of the module declaring `UserId`, which is the asymmetry `ca0fa37`
 * had to work around on the Windows side. */
#[path = "unix/endpoints.rs"]
mod endpoints;
pub use endpoints::{
    OpenMode, PIPE_BUFFER, ProcessDescriptorTransaction, anonymous_file, create_temporary_file,
    duplicate_cloexec, duplicate_fd, duplicate_file, fd_is_regular_file, fd_is_seekable,
    open_null_input, open_path, open_pseudoterminal, pipe, read_exact, read_once, read_to_end,
    reports_pipe_short_writes, seek_relative, seek_start, set_nonblocking, snapshot_process_fd,
    supports_bidirectional_pseudoterminal_pair, supports_tee, take_file_contents, tee, write_all,
    write_once,
};
#[path = "unix/errors.rs"]
mod errors;
pub use errors::{
    PathErrorKind, command_exec_failure_status, is_bad_descriptor_error, is_exec_format_error,
    is_path_error, is_pseudoterminal_end, platform_error,
};
#[path = "unix/paths.rs"]
mod paths;
pub use paths::{
    AccessMode, DirectoryEntry, FileKind, FileMetadata, absolute_path,
    can_unlink_current_directory, controlling_terminal_path, current_directory,
    default_search_path, effective_access, fallback_shell, logical_path, named_user_home,
    null_device_path, open_history_file, path_exists, path_is_directory, path_is_file,
    path_is_same_file, path_metadata, read_directory, read_path, remove_file, resolve_command_path,
    run_editor, search_path_separator, set_current_directory, shell_directory_separator,
    shell_path_has_separator, shell_path_is_absolute, shell_path_last_separator,
    supports_glob_metacharacters_in_filenames,
};
#[path = "unix/process.rs"]
mod process;
pub use process::{
    LimitResource, ProcessTimes, ResourceLimit, creation_mask, current_process_group,
    current_process_id, environment_text, execute_program, exit_immediately,
    flush_coverage_profile, foreground_process_group, fork_process, parent_process_id,
    process_arguments, process_environment, process_times, replace_creation_mask,
    reset_coverage_counters, resource_limit, restore_shell_process_runtime_state, run_in_child,
    set_foreground_process_group, set_process_group, set_resource_limit, wait_for_any_child,
    wait_for_child,
};
#[path = "unix/signals.rs"]
mod signals;
pub use signals::{
    BlockedSignals, SignalAction, child_signal, configure_here_document_writer_signals,
    continue_signal, hangup_signal, ignore_signal, install_signal_action, interrupt_signal,
    kill_signal, pipe_signal, quit_signal, raise_signal, send_continue_to_process_group,
    send_signal, signal_action, signal_is_blocked, terminal_input_signal, terminal_output_signal,
    terminal_stop_signal, terminate_with_interrupt, termination_signal, unblock_all_signals,
};
#[path = "unix/text.rs"]
mod text;
pub use text::{NativeStrExt, ShellBytesExt, input_newline_width, trim_command_substitution_output};

fn raw_process_id(process: ProcessId) -> std::io::Result<i32> {
    i32::try_from(process.get()).map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))
}

fn raw_process_group(group: ProcessGroupId) -> std::io::Result<i32> {
    i32::try_from(group.get()).map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt as _;

    // [spec:nsh:req:idiom.filesystem-account-bytes/test]
    #[test]
    fn native_string_extensions_round_trip_non_utf8_values() {
        let bytes = vec![b'n', b's', b'h', b'-', 0xff];
        let native = bytes.as_slice().try_to_os_string().unwrap();

        assert_eq!(native.to_shell_bytes(), bytes);
        assert_eq!(
            bytes.as_slice().try_to_path_buf().unwrap().as_os_str(),
            native
        );

        let label = OsStr::from_bytes(b"nsh-platform-\xff");
        let (file, path) = create_temporary_file(label).unwrap();
        drop(file);
        assert!(
            path.file_name()
                .unwrap()
                .as_bytes()
                .starts_with(label.as_bytes())
        );
        remove_file(&path).unwrap();

        let anonymous = anonymous_file(label).unwrap();
        write_all(&anonymous, b"native label").unwrap();
        assert_eq!(take_file_contents(&anonymous).unwrap(), b"native label");
    }

    #[test]
    fn os_error_boundaries_are_classified() {
        let error = std::io::Error::from(rustix::io::Errno::NOENT);
        let message = Locale::c().unwrap().error_message(&error);
        assert!(!message.contains("(os error"));
        assert!(!message.is_empty());

        assert!(is_path_error(
            &std::io::Error::from(rustix::io::Errno::NAMETOOLONG),
            PathErrorKind::NameTooLong,
        ));
        assert!(is_path_error(
            &std::io::Error::from(rustix::io::Errno::NOENT),
            PathErrorKind::NotFound,
        ));
        assert!(is_path_error(
            &std::io::Error::from(rustix::io::Errno::NOTDIR),
            PathErrorKind::NotFound,
        ));
        assert!(!is_path_error(
            &std::io::Error::from(rustix::io::Errno::ACCESS),
            PathErrorKind::NotFound,
        ));
    }

    #[test]
    fn temporary_files_are_unique_owned_and_private() {
        let (first, first_path) = create_temporary_file("nsh-platform-test").unwrap();
        let (second, second_path) = create_temporary_file("nsh-platform-test").unwrap();

        assert_ne!(first_path, second_path);
        assert_eq!(
            first.metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            second.metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );

        drop(first);
        drop(second);
        remove_file(&first_path).unwrap();
        remove_file(&second_path).unwrap();
    }

    #[test]
    fn duplicated_descriptor_outlives_source() {
        let source = std::fs::File::open("/dev/null").unwrap();
        let duplicate = duplicate_fd(&source).unwrap();
        drop(source);

        let mut byte = [0];
        assert_eq!(read_once(&duplicate, &mut byte).unwrap(), 0);
    }

    // [spec:nsh:req:idiom.descriptor-materialization/test]
    #[test]
    fn descriptor_transaction_installs_slots() {
        let (read, write) = pipe().unwrap();
        let status = run_in_child(move || {
            let source = duplicate_cloexec(&write, 10).unwrap();
            ProcessDescriptorTransaction::new([(7, Some(source)), (8, None)])
                .unwrap()
                .apply()
                .unwrap();

            let seven = snapshot_process_fd(7, 10).unwrap().unwrap();
            write_all(&seven, b"staged").unwrap();
            if snapshot_process_fd(8, 10).unwrap().is_some() {
                exit_immediately(2);
            }
            exit_immediately(0);
        })
        .unwrap();

        assert_eq!(status, 0);
        assert_eq!(read_exact(&read, 6).unwrap(), b"staged");
    }

    #[test]
    fn descriptor_transaction_validates_targets() {
        assert!(ProcessDescriptorTransaction::new([(-1, None)]).is_err());
        assert!(ProcessDescriptorTransaction::new([(4, None), (4, None)]).is_err());
        let (_, write) = pipe().unwrap();
        let number = write.number();

        assert!(ProcessDescriptorTransaction::new([(-1, Some(write))]).is_err());
        assert!(snapshot_process_fd(number, 10).unwrap().is_none());
    }
}
