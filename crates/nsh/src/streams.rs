//! The shell's initial standard streams.
//!
//! Implements [dec:nsh:host-owns-streams]: the library snapshots or
//! duplicates the streams it is given into the shell instance. They seed the
//! instance's logical descriptor table; no process-wide installation guard
//! or ambient descriptor swap is involved.

use nsh_platform::{AsDescriptor, Descriptor};

use crate::descriptors::{LogicalDescriptor, SharedDescriptor};

/// The three descriptors that seed a shell's logical stdin, stdout and
/// stderr slots.
#[derive(Debug)]
pub struct Streams {
    owned: Option<[SharedDescriptor; 3]>,
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
        stdin: impl AsDescriptor,
        stdout: impl AsDescriptor,
        stderr: impl AsDescriptor,
    ) -> std::io::Result<Streams> {
        Ok(Streams {
            owned: Some([
                SharedDescriptor::from(nsh_platform::duplicate_cloexec(
                    &stdin,
                    LogicalDescriptor::INHERITED as i32,
                )?),
                SharedDescriptor::from(nsh_platform::duplicate_cloexec(
                    &stdout,
                    LogicalDescriptor::INHERITED as i32,
                )?),
                SharedDescriptor::from(nsh_platform::duplicate_cloexec(
                    &stderr,
                    LogicalDescriptor::INHERITED as i32,
                )?),
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
    // [spec:nsh:req:idiom.filesystem-account-bytes]
    pub fn capture() -> std::io::Result<Streams> {
        Self::from_owned([
            nsh_platform::open_null_device(false)?,
            nsh_platform::anonymous_file("nsh-stdout")?,
            nsh_platform::anonymous_file("nsh-stderr")?,
        ])
    }

    /// The null device on all three: a shell that reads nothing and whose
    /// output goes nowhere.
    ///
    /// What [`crate::script::Reader`] builds its shell on. A reader never
    /// runs anything, but the parser reports a syntax error as well as
    /// returning it, and the report has to have somewhere to go that is
    /// not the host's stderr and does not fill up.
    pub(crate) fn discarding() -> std::io::Result<Streams> {
        Self::from_owned([
            nsh_platform::open_null_device(false)?,
            nsh_platform::open_null_device(true)?,
            nsh_platform::open_null_device(true)?,
        ])
    }

    fn from_owned(owned: [Descriptor; 3]) -> std::io::Result<Streams> {
        let [stdin, stdout, stderr] = owned;
        Ok(Streams {
            owned: Some([
                SharedDescriptor::from_owned(stdin)?,
                SharedDescriptor::from_owned(stdout)?,
                SharedDescriptor::from_owned(stderr)?,
            ]),
        })
    }

    // [spec:nsh:def:idiom.logical-descriptors]
    pub(crate) fn initial_descriptors(
        &self,
    ) -> std::io::Result<[Option<SharedDescriptor>; LogicalDescriptor::INHERITED]> {
        if let Some(owned) = &self.owned {
            let mut result: [Option<SharedDescriptor>; LogicalDescriptor::INHERITED] =
                std::array::from_fn(|_| None);
            result[LogicalDescriptor::STDIN.index()] = Some(owned[0].clone());
            result[LogicalDescriptor::STDOUT.index()] = Some(owned[1].clone());
            result[LogicalDescriptor::STDERR.index()] = Some(owned[2].clone());
            return Ok(result);
        }

        let mut result: [Option<SharedDescriptor>; LogicalDescriptor::INHERITED] =
            std::array::from_fn(|_| None);
        for (number, slot) in result.iter_mut().enumerate() {
            let descriptor = LogicalDescriptor::from_index(number)
                .expect("the stream table contains only logical descriptors");
            *slot = nsh_platform::snapshot_process_fd(
                descriptor.as_i32(),
                LogicalDescriptor::INHERITED as i32,
            )?
            .map(SharedDescriptor::from);
        }
        Ok(result)
    }

    fn original(&self, index: usize) -> Option<SharedDescriptor> {
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
        self.io.flush_all()?;
        self.take_captured_stream(1)
    }

    /// Take everything captured on the shell's stderr since the last call.
    pub fn take_captured_stderr(&mut self) -> std::io::Result<bstr::BString> {
        self.take_captured_stream(2)
    }

    // [spec:nsh:req:idiom.platform-errors]
    fn take_captured_stream(&self, index: usize) -> std::io::Result<bstr::BString> {
        let descriptor = self
            .streams
            .original(index)
            .or_else(|| {
                LogicalDescriptor::from_index(index)
                    .and_then(|descriptor| self.descriptors.get(descriptor))
            })
            .ok_or_else(|| {
                nsh_platform::platform_error(nsh_platform::PlatformErrorKind::BadDescriptor)
            })?;
        nsh_platform::take_file_contents(&descriptor).map(bstr::BString::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:nsh:req:idiom.no-raw-fd-core/test]
    #[test]
    fn supplied_streams_outlive_source() {
        let source = nsh_platform::anonymous_file("streams-owned").unwrap();
        let streams = Streams::from_fds(&source, &source, &source).unwrap();
        let descriptors = streams.initial_descriptors().unwrap();
        drop(source);

        for descriptor in &descriptors[..3] {
            let descriptor = descriptor.as_ref().expect("supplied stream is open");
            nsh_platform::write_all(descriptor, b"x").unwrap();
        }
        assert_eq!(
            nsh_platform::take_file_contents(descriptors[0].as_ref().unwrap()).unwrap(),
            b"xxx"
        );
        assert!(descriptors[3..].iter().all(Option::is_none));
    }

    // [spec:nsh:req:idiom.descriptor-materialization/test]
    #[test]
    fn closed_inherited_streams_remain_closed() {
        let status = nsh_platform::run_in_child(|| {
            nsh_platform::ProcessDescriptorTransaction::new([(
                LogicalDescriptor::STDIN.as_i32(),
                None,
            )])
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
