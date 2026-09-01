//! Turning this host's error numbers into the answers the shell needs.
//!
//! Every question here is asked about an `io::Error` that has already
//! crossed the boundary, which is why they are predicates rather than
//! constants: a Windows error code is one of the things the shell crate
//! must never see, and a shell nevertheless has to tell "not found" from
//! "found but not executable" to choose between exit status 127 and 126.

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_BAD_EXE_FORMAT, ERROR_FILE_NOT_FOUND,
    ERROR_FILENAME_EXCED_RANGE, ERROR_INVALID_HANDLE, ERROR_PATH_NOT_FOUND,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathErrorKind {
    NotFound,
    NameTooLong,
}

// [spec:nsh:req:idiom.platform-errors]
pub fn is_path_error(error: &std::io::Error, kind: PathErrorKind) -> bool {
    let Some(code) = error.raw_os_error() else {
        return kind == PathErrorKind::NotFound && error.kind() == std::io::ErrorKind::NotFound;
    };
    match kind {
        PathErrorKind::NotFound => {
            code == ERROR_FILE_NOT_FOUND as i32 || code == ERROR_PATH_NOT_FOUND as i32
        }
        PathErrorKind::NameTooLong => code == ERROR_FILENAME_EXCED_RANGE as i32,
    }
}

pub fn platform_error(kind: crate::PlatformErrorKind) -> std::io::Error {
    let code = match kind {
        crate::PlatformErrorKind::AlreadyExists => ERROR_ALREADY_EXISTS,
        crate::PlatformErrorKind::BadDescriptor => ERROR_INVALID_HANDLE,
        crate::PlatformErrorKind::NotFound => ERROR_FILE_NOT_FOUND,
        crate::PlatformErrorKind::PermissionDenied => ERROR_ACCESS_DENIED,
    };
    std::io::Error::from_raw_os_error(code as i32)
}

pub fn command_exec_failure_status(error: &std::io::Error) -> i32 {
    if is_path_error(error, PathErrorKind::NotFound) {
        127
    } else {
        126
    }
}

pub fn is_exec_format_error(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(ERROR_BAD_EXE_FORMAT as i32)
}

pub fn is_pseudoterminal_end(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::UnexpectedEof
    )
}

pub fn is_bad_descriptor_error(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(ERROR_INVALID_HANDLE as i32)
}
