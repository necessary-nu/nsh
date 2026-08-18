use std::os::fd::{AsRawFd, OwnedFd};

use crate::{LimitResource, resource_limit};

pub(crate) fn normalize_dupfd_error(
    error: rustix::io::Errno,
    minimum: i32,
) -> std::io::Error {
    let open_file_limit = resource_limit(LimitResource::OpenFiles)
        .ok()
        .and_then(|limit| limit.current);
    normalize_dupfd_error_for_limit(error, minimum, open_file_limit)
}

fn normalize_dupfd_error_for_limit(
    error: rustix::io::Errno,
    minimum: i32,
    open_file_limit: Option<u64>,
) -> std::io::Error {
    // Linux reports EINVAL when F_DUPFD_CLOEXEC's lower bound is at or above
    // RLIMIT_NOFILE. To a caller asking for an owned descriptor in a reserved
    // range, that means the range is exhausted; preserve the useful EMFILE
    // classification rather than leaking this fcntl-specific encoding.
    let minimum = u64::try_from(minimum).ok();
    if error == rustix::io::Errno::INVAL
        && minimum
            .zip(open_file_limit)
            .is_some_and(|(minimum, limit)| minimum >= limit)
    {
        return std::io::Error::from_raw_os_error(
            rustix::io::Errno::MFILE.raw_os_error(),
        );
    }
    std::io::Error::from(error)
}

/// Move an owned descriptor at or above `minimum`, setting close-on-exec.
///
/// The original descriptor is closed automatically when a duplicate is
/// needed. A descriptor already in range is adopted after its close-on-exec
/// flag is verified, avoiding a second allocation for pipe ends and other
/// descriptors that were hidden at creation time.
pub fn move_fd_cloexec(
    fd: OwnedFd,
    minimum: i32,
) -> std::io::Result<OwnedFd> {
    if minimum < 0 {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    if fd.as_raw_fd() < minimum {
        return crate::duplicate_cloexec(&fd, minimum);
    }

    let flags = rustix::io::fcntl_getfd(&fd).map_err(std::io::Error::from)?;
    if !flags.contains(rustix::io::FdFlags::CLOEXEC) {
        rustix::io::fcntl_setfd(&fd, flags | rustix::io::FdFlags::CLOEXEC)
            .map_err(std::io::Error::from)?;
    }
    Ok(fd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dupfd_bound_is_exhaustion() {
        let error = normalize_dupfd_error_for_limit(
            rustix::io::Errno::INVAL,
            10,
            Some(8),
        );

        assert_eq!(
            error.raw_os_error(),
            Some(rustix::io::Errno::MFILE.raw_os_error()),
        );
        assert_eq!(
            normalize_dupfd_error_for_limit(
                rustix::io::Errno::INVAL,
                7,
                Some(8),
            )
            .raw_os_error(),
            Some(rustix::io::Errno::INVAL.raw_os_error()),
        );
        assert_eq!(
            normalize_dupfd_error_for_limit(
                rustix::io::Errno::BADF,
                10,
                Some(8),
            )
            .raw_os_error(),
            Some(rustix::io::Errno::BADF.raw_os_error()),
        );
    }

    #[test]
    fn move_adopts_in_range_cloexec() {
        let source = std::fs::File::open("/dev/null").unwrap();
        let fd = crate::duplicate_cloexec(&source, 10).unwrap();
        let number = fd.as_raw_fd();
        rustix::io::fcntl_setfd(&fd, rustix::io::FdFlags::empty()).unwrap();

        let moved = move_fd_cloexec(fd, 10).unwrap();

        assert_eq!(moved.as_raw_fd(), number);
        assert!(
            rustix::io::fcntl_getfd(&moved)
                .unwrap()
                .contains(rustix::io::FdFlags::CLOEXEC),
        );
    }
}
