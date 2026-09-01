//! Turning this host's error numbers into the answers the shell needs.
//!
//! Every question here is asked about an `io::Error` that has already
//! crossed the boundary, which is why they are predicates rather than
//! constants: `errno` values are the one part of the platform the shell
//! crate must never see, and a shell nevertheless has to distinguish
//! "not found" from "found but not executable" to choose between exit
//! status 127 and 126.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathErrorKind {
    NotFound,
    NameTooLong,
}

// [spec:nsh:req:idiom.platform-errors]
/// Classify an I/O error without exposing platform errno constants to core.
pub fn is_path_error(error: &std::io::Error, kind: PathErrorKind) -> bool {
    let Some(code) = error.raw_os_error() else {
        return kind == PathErrorKind::NotFound && error.kind() == std::io::ErrorKind::NotFound;
    };
    match kind {
        PathErrorKind::NotFound => [rustix::io::Errno::NOENT, rustix::io::Errno::NOTDIR]
            .iter()
            .any(|error| code == error.raw_os_error()),
        PathErrorKind::NameTooLong => code == rustix::io::Errno::NAMETOOLONG.raw_os_error(),
    }
}

/// Construct a typed I/O error for a platform-independent shell condition.
pub fn platform_error(kind: crate::PlatformErrorKind) -> std::io::Error {
    let error = match kind {
        crate::PlatformErrorKind::AlreadyExists => rustix::io::Errno::EXIST,
        crate::PlatformErrorKind::BadDescriptor => rustix::io::Errno::BADF,
        crate::PlatformErrorKind::NotFound => rustix::io::Errno::NOENT,
        crate::PlatformErrorKind::PermissionDenied => rustix::io::Errno::ACCESS,
    };
    std::io::Error::from(error)
}

/// POSIX distinguishes "command not found" (127) from a command that was
/// found but could not be executed (126).
pub fn command_exec_failure_status(error: &std::io::Error) -> i32 {
    let not_found = error.raw_os_error().is_some_and(|code| {
        [
            rustix::io::Errno::LOOP,
            rustix::io::Errno::NAMETOOLONG,
            rustix::io::Errno::NOENT,
            rustix::io::Errno::NOTDIR,
        ]
        .iter()
        .any(|error| error.raw_os_error() == code)
    }) || error.kind() == std::io::ErrorKind::NotFound;
    if not_found { 127 } else { 126 }
}

pub fn is_exec_format_error(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(rustix::io::Errno::NOEXEC.raw_os_error())
}

pub fn is_pseudoterminal_end(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error())
}

pub fn is_bad_descriptor_error(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(rustix::io::Errno::BADF.raw_os_error())
}
