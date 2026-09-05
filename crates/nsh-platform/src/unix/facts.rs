//! Who the process is, what the host is called, and whether a
//! descriptor has anything to read.
//!
//! Questions that share nothing with each other except that they are
//! the ones this platform answers for the Bash-compatibility surface.
//! `windows::facts` answers the same list, which is why the user and
//! group identities live here rather than beside the process code that
//! reads them: a constructor belongs with the private field it fills,
//! and on the Windows side that field can only be reached from the
//! facts module.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt as _;

use super::AsDescriptor;

#[inline]
pub fn effective_uid() -> UserId {
    UserId(rustix::process::geteuid().as_raw())
}

#[inline]
pub fn effective_gid() -> GroupId {
    GroupId(rustix::process::getegid().as_raw())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserId(u32);

impl UserId {
    pub fn is_root(self) -> bool {
        self.0 == 0
    }

    pub fn as_raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GroupId(u32);

impl GroupId {
    pub fn as_raw(self) -> u32 {
        self.0
    }
}

pub fn supplementary_groups() -> std::io::Result<Vec<GroupId>> {
    rustix::process::getgroups()
        .map(|groups| {
            groups
                .into_iter()
                .map(|group| GroupId(group.as_raw()))
                .collect()
        })
        .map_err(std::io::Error::from)
}

/// The identity the process was started with, which `$UID` reports.
#[inline]
#[must_use]
pub fn real_uid() -> UserId {
    UserId(rustix::process::getuid().as_raw())
}

/// The first descriptor number the process cannot use.
///
/// The shell needs this because POSIX leaves the highest nameable
/// descriptor implementation-defined and the honest answer is the host's:
/// `exec 1000000>file` has to be refused where `exec 42>file` is not, and
/// the line between them is `RLIMIT_NOFILE`, not a constant.
#[must_use]
pub fn descriptor_limit() -> u32 {
    rustix::process::getrlimit(rustix::process::Resource::Nofile)
        .current
        .and_then(|limit| u32::try_from(limit).ok())
        .unwrap_or(u32::MAX)
}

/// The host's own name, for `$HOSTNAME`.
#[must_use]
pub fn host_name() -> Option<OsString> {
    let name = rustix::system::uname();
    let bytes = name.nodename().to_bytes();
    (!bytes.is_empty()).then(|| OsString::from_vec(bytes.to_vec()))
}

/// Whether a descriptor has input available, waiting at most `timeout`
/// seconds for it.
///
/// A negative or zero timeout polls without blocking, which is what
/// `read -t 0` asks. `None` waits indefinitely. End of file counts as
/// available: the reader has something to observe, even if it is only
/// the end.
// [spec:nsh:req:idiom.platform-errors]
pub fn wait_for_input(fd: &impl AsDescriptor, timeout: Option<f64>) -> std::io::Result<bool> {
    let borrowed = fd.as_platform_descriptor().0;
    let mut poll = [rustix::event::PollFd::new(
        &borrowed,
        rustix::event::PollFlags::IN,
    )];
    let wait = match timeout {
        None => None,
        Some(seconds) if seconds <= 0.0 => Some(rustix::event::Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        }),
        Some(seconds) => {
            let bounded = seconds.min(86_400.0);
            Some(rustix::event::Timespec {
                tv_sec: bounded as i64,
                tv_nsec: ((bounded.fract()) * 1e9) as i64,
            })
        }
    };
    loop {
        return match rustix::event::poll(&mut poll, wait.as_ref()) {
            Ok(ready) => Ok(ready != 0),
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => Err(std::io::Error::from(error)),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The nameable-descriptor ceiling is the host's, not a constant of
    /// ours: it has to admit the numbers a script really uses and refuse
    /// the ones `dup2` would.
    #[test]
    fn the_descriptor_limit_is_the_host_limit() {
        let limit = descriptor_limit();
        let expected = rustix::process::getrlimit(rustix::process::Resource::Nofile).current;
        match expected.and_then(|value| u32::try_from(value).ok()) {
            Some(value) => assert_eq!(limit, value),
            None => assert_eq!(limit, u32::MAX),
        }
        // Slots a shell script names in practice are all below it.
        assert!(limit > 64, "a usable host offers more than 64 descriptors");
    }

    /// The identities are the ones the kernel reports, and the host has
    /// a name of some length.
    #[test]
    fn host_identity_comes_from_the_kernel() {
        assert_eq!(real_uid().as_raw(), rustix::process::getuid().as_raw());
        let name = host_name().expect("a host has a name");
        assert!(!name.is_empty());
    }

    /// A descriptor with bytes waiting is ready at once; one that will
    /// never produce any is not ready, and says so rather than blocking.
    #[test]
    fn readiness_is_answered_without_consuming() {
        let (read, write) = super::super::pipe().expect("pipe");
        assert!(!wait_for_input(&read, Some(0.0)).expect("poll empty"));
        super::super::write_all(&write, b"x").expect("write");
        assert!(wait_for_input(&read, Some(0.0)).expect("poll ready"));
        // Asking did not take the byte.
        let mut byte = [0_u8; 1];
        assert_eq!(super::super::read_once(&read, &mut byte).expect("read"), 1);
        assert_eq!(byte, *b"x");
        assert!(!wait_for_input(&read, Some(0.01)).expect("poll drained"));
    }

    /// End of input counts as available: the reader has something to
    /// observe, even if it is only the end.
    ///
    /// A pipe is at end of input when the last write end *anywhere*
    /// closes, and `fork` copies the whole descriptor table, so a
    /// sibling test's child forked between this pipe's creation and the
    /// closing of its write end holds one this thread can neither see
    /// nor close. The pipe is then not at end of input, `wait_for_input`
    /// says so correctly, and the assertion reads as a defect in `poll`.
    /// No flag makes a descriptor un-inheritable across `fork`, so the
    /// check takes a process of its own and builds the pipe inside it,
    /// where nothing else can hold an end.
    #[test]
    fn end_of_input_counts_as_available() {
        let status = super::super::run_in_child(|| {
            let Ok((read, write)) = super::super::pipe() else {
                super::super::exit_immediately(2);
            };
            drop(write);
            match wait_for_input(&read, Some(0.0)) {
                Ok(true) => {}
                Ok(false) => super::super::exit_immediately(3),
                Err(_) => super::super::exit_immediately(4),
            }
        })
        .expect("a process of its own to hold the pipe in");

        assert_eq!(
            status,
            0,
            "{}",
            match status {
                2 => "the child could not make a pipe to ask about",
                3 => "a pipe past its last write end was reported as having nothing to observe",
                4 => "asking a pipe past its last write end failed outright",
                _ => "the child died without reporting why",
            }
        );
    }
}
