//! The shell's initial standard streams.
//!
//! Implements [dec:nsh:host-owns-streams]: the library snapshots or
//! duplicates the streams it is given into the shell instance. They seed the
//! instance's logical descriptor table; no process-wide installation guard
//! or ambient descriptor swap is involved.

use std::os::fd::{AsFd, OwnedFd};

use crate::fd::SharedFd;

/// The three descriptors that seed a shell's logical stdin, stdout and
/// stderr slots.
#[derive(Debug)]
pub struct Streams {
    owned: Option<[SharedFd; 3]>,
}

impl Streams {
    /// Snapshot process descriptors 0 through 9 when the shell is built.
    ///
    /// The standard three seed the shell's streams; the remaining slots
    /// preserve inherited shell-language descriptors such as an already-open
    /// descriptor 3. Each open descriptor becomes an owned close-on-exec
    /// backing handle, while an inherited closed slot remains logically
    /// closed.
    pub const INHERIT: Streams = Streams { owned: None };

    /// [`Streams::INHERIT`] as a function.
    pub fn inherit() -> Streams {
        Streams::INHERIT
    }

    /// Duplicate three caller-owned descriptors into a shell-owned stream set.
    ///
    /// These descriptors seed logical slots 0, 1 and 2. Consequently shell
    /// redirections, pipelines, builtins and external commands all start from
    /// the supplied streams without changing the host's process descriptors.
    pub fn from_fds(
        stdin: impl AsFd,
        stdout: impl AsFd,
        stderr: impl AsFd,
    ) -> std::io::Result<Streams> {
        Ok(Streams {
            owned: Some([
                SharedFd::from_backing(nsh_platform::duplicate_cloexec(&stdin, 10)?),
                SharedFd::from_backing(nsh_platform::duplicate_cloexec(&stdout, 10)?),
                SharedFd::from_backing(nsh_platform::duplicate_cloexec(&stderr, 10)?),
            ]),
        })
    }

    /// Use `/dev/null` for input and anonymous seekable files for output.
    ///
    /// Seekable files avoid pipe-capacity deadlocks while [`Shell::run`]
    /// executes. Captured bytes include builtins, pipelines and external
    /// commands because all of them resolve logical slots through the same
    /// per-shell table.
    ///
    /// [`Shell::run`]: crate::context::Shell::run
    pub fn capture() -> std::io::Result<Streams> {
        Self::from_owned([
            nsh_platform::open_null_input()?,
            nsh_platform::anonymous_file(c"nsh-stdout")?,
            nsh_platform::anonymous_file(c"nsh-stderr")?,
        ])
    }

    fn from_owned(owned: [OwnedFd; 3]) -> std::io::Result<Streams> {
        let [stdin, stdout, stderr] = owned;
        Ok(Streams {
            owned: Some([
                SharedFd::from_owned(stdin)?,
                SharedFd::from_owned(stdout)?,
                SharedFd::from_owned(stderr)?,
            ]),
        })
    }

    pub(crate) fn initial_descriptors(
        &self,
    ) -> std::io::Result<[Option<SharedFd>; crate::fd::SLOT_COUNT]> {
        if let Some(owned) = &self.owned {
            let mut result: [Option<SharedFd>; crate::fd::SLOT_COUNT] =
                std::array::from_fn(|_| None);
            result[0] = Some(owned[0].clone());
            result[1] = Some(owned[1].clone());
            result[2] = Some(owned[2].clone());
            return Ok(result);
        }

        let mut result: [Option<SharedFd>; crate::fd::SLOT_COUNT] =
            std::array::from_fn(|_| None);
        for (number, slot) in result.iter_mut().enumerate() {
            *slot = nsh_platform::snapshot_process_fd(number as i32, 10)?
                .map(SharedFd::from_backing);
        }
        Ok(result)
    }

    fn original(&self, index: usize) -> Option<SharedFd> {
        self.owned.as_ref().map(|owned| owned[index].clone())
    }
}

impl Default for Streams {
    fn default() -> Self {
        Streams::INHERIT
    }
}

impl crate::context::Shell {
    /// Take everything captured on the shell's stdout since the last call.
    pub fn take_captured_stdout(&mut self) -> std::io::Result<bstr::BString> {
        self.io.flushall();
        self.take_captured_stream(1)
    }

    /// Take everything captured on the shell's stderr since the last call.
    pub fn take_captured_stderr(&mut self) -> std::io::Result<bstr::BString> {
        self.take_captured_stream(2)
    }

    fn take_captured_stream(&self, index: usize) -> std::io::Result<bstr::BString> {
        let fd = self
            .streams
            .original(index)
            .or_else(|| self.fds.get(index as i32).ok().flatten())
            .ok_or_else(crate::fd::bad_descriptor)?;
        nsh_platform::take_file_contents(&fd).map(bstr::BString::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    #[test]
    fn supplied_streams_own_hidden_fds() {
        let source = nsh_platform::anonymous_file(c"streams-owned").unwrap();
        let source_number = source.as_raw_fd();
        let streams = Streams::from_fds(&source, &source, &source).unwrap();
        let descriptors = streams.initial_descriptors().unwrap();
        drop(source);

        for descriptor in &descriptors[..3] {
            let descriptor = descriptor.as_ref().expect("supplied stream is open");
            assert_ne!(descriptor.as_fd().as_raw_fd(), source_number);
            assert!(descriptor.as_fd().as_raw_fd() >= 10);
        }
        assert!(descriptors[3..].iter().all(Option::is_none));
    }

    #[test]
    fn closed_inherited_streams_remain_closed() {
        let status = nsh_platform::run_in_child(|| {
            nsh_platform::ProcessFdChanges::new([(0, None)])
                .unwrap()
                .apply()
                .unwrap();
            let descriptors = Streams::INHERIT.initial_descriptors().unwrap();
            nsh_platform::exit_immediately(if descriptors[0].is_none() { 0 } else { 1 });
        })
        .unwrap();
        assert_eq!(status, 0);
    }
}
