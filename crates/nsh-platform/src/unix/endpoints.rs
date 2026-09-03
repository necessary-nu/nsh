//! What a shell does with an open endpoint: make one, hand one on, and
//! move bytes through it.
//!
//! The owned `Descriptor` type lives in `descriptor.rs`, whose subject is
//! ownership. This is the operations: opening a redirection target,
//! duplicating a descriptor above a floor, creating a pipe, the read and
//! write loops that retry an interrupted transfer, and the transaction
//! that installs exact process slots in a child about to `exec` -- the
//! one place in the crate where a descriptor is moved by number.

use std::collections::BTreeSet;
use std::collections::hash_map::RandomState;
use std::ffi::{CString, OsStr, OsString};
use std::hash::BuildHasher as _;
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};

use super::*;

/// Probe whether the descriptor has a current seek position.
pub fn fd_is_seekable(fd: &impl AsDescriptor) -> bool {
    rustix::fs::seek(
        fd.as_platform_descriptor().0,
        rustix::fs::SeekFrom::Current(0),
    )
    .map(|_| ())
    .map_err(std::io::Error::from)
    .is_ok()
}

/// Move a descriptor's current position relative to where it is now.
pub fn seek_relative(fd: &impl AsDescriptor, offset: i64) -> std::io::Result<u64> {
    rustix::fs::seek(
        fd.as_platform_descriptor().0,
        rustix::fs::SeekFrom::Current(offset),
    )
    .map_err(std::io::Error::from)
}

/// Rewind a descriptor to its beginning.
pub fn seek_start(fd: &impl AsDescriptor) -> std::io::Result<u64> {
    rustix::fs::seek(
        fd.as_platform_descriptor().0,
        rustix::fs::SeekFrom::Start(0),
    )
    .map_err(std::io::Error::from)
}

/// A staged set of changes to exact process descriptor-table slots.
///
/// Ordinary descriptor ownership belongs to [`Descriptor`]. Exact slot numbers
/// are different: `exec` must make the shell's logical descriptor 2 become
/// process descriptor 2, and a logically closed descriptor must be absent
/// from the process table. This object is the safe boundary for that final
/// operation. It owns every source, moves sources above every target before
/// changing anything, rejects duplicate or negative targets, and consumes
/// itself when the changes are applied.
///
/// Applying changes is process-wide. It is intended for a forked child just
/// before `exec`, or for a process whose owner has explicitly granted image
/// replacement. No Rust descriptor object may own one of the target slots.
// [spec:nsh:req:idiom.descriptor-materialization]
#[derive(Debug)]
pub struct ProcessDescriptorTransaction {
    changes: Vec<(i32, Option<Descriptor>)>,
}

impl ProcessDescriptorTransaction {
    /// Validate and stage exact-slot changes without modifying the process.
    ///
    /// `Some(fd)` duplicates `fd` into the target when [`apply`](Self::apply)
    /// runs; `None` closes the target. Every source is first moved above the
    /// highest target with close-on-exec set, so target/source aliasing and
    /// cycles cannot affect the result.
    pub fn new(
        changes: impl IntoIterator<Item = (i32, Option<Descriptor>)>,
    ) -> std::io::Result<Self> {
        let changes: Vec<_> = changes.into_iter().collect();
        let mut targets = BTreeSet::new();
        let mut highest = -1;
        for (target, _) in &changes {
            if *target < 0 || !targets.insert(*target) {
                return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
            }
            highest = highest.max(*target);
        }

        let minimum = highest.saturating_add(1).max(10);
        let mut staged = Vec::with_capacity(changes.len());
        for (target, source) in changes {
            let source = match source {
                Some(source) if source.number() < minimum => {
                    Some(duplicate_cloexec(&source, minimum)?)
                }
                source => source,
            };
            staged.push((target, source));
        }
        Ok(Self { changes: staged })
    }

    /// Apply all staged changes to the process descriptor table.
    ///
    /// Resource failures happen during [`new`](Self::new), before any target
    /// is touched. Once this begins, the caller is at a process terminus: a
    /// later syscall error can leave a prefix applied and must be followed by
    /// process termination rather than returning to ordinary Rust code.
    pub fn apply(self) -> std::io::Result<()> {
        for (target, source) in &self.changes {
            match source {
                Some(source) => loop {
                    // SAFETY: `source` is a live owned descriptor staged above
                    // every target. The target is a validated non-negative
                    // process-table number, and the kernel retains no borrow.
                    let result = unsafe { libc::dup2(source.number(), *target) };
                    if result >= 0 {
                        break;
                    }
                    let error = std::io::Error::last_os_error();
                    if error.kind() != std::io::ErrorKind::Interrupted {
                        return Err(error);
                    }
                },
                None => {
                    // SAFETY: exact closed slots have no Rust owner. Closing
                    // an already absent slot is the requested final state.
                    if unsafe { libc::close(*target) } < 0 {
                        let error = std::io::Error::last_os_error();
                        if !is_bad_descriptor_error(&error) {
                            return Err(error);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Duplicate an exact process descriptor into an owned close-on-exec handle.
///
/// A closed slot is `Ok(None)`. This is used only to snapshot inherited
/// process state into an owning logical descriptor table; callers never
/// receive a borrowed view of a possibly closed slot.
pub fn snapshot_process_fd(number: i32, minimum: i32) -> std::io::Result<Option<Descriptor>> {
    if number < 0 || minimum < 0 {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    // SAFETY: `fcntl` receives scalar descriptor numbers and returns a new
    // owned descriptor on success. It retains no pointer or Rust borrow.
    let result = unsafe { libc::fcntl(number, libc::F_DUPFD_CLOEXEC, minimum) };
    if result >= 0 {
        // SAFETY: a successful F_DUPFD_CLOEXEC returns a fresh descriptor
        // whose ownership is transferred to the caller.
        return Ok(Some(Descriptor::from(unsafe {
            OwnedFd::from_raw_fd(result)
        })));
    }
    let error = std::io::Error::last_os_error();
    if is_bad_descriptor_error(&error) {
        Ok(None)
    } else {
        Err(error)
    }
}

/// Open `/dev/null` for reading and transfer ownership of the descriptor.
pub fn open_null_input() -> std::io::Result<Descriptor> {
    std::fs::File::open("/dev/null")
        .map(OwnedFd::from)
        .map(Descriptor::from)
}

/// The finite set of open modes used by shell redirections.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenMode {
    ReadOnly,
    ReadWrite,
    ReadWriteCreate,
    WriteOnly,
    WriteCreateExclusive,
    WriteCreateTruncate,
    WriteCreateAppend,
}

impl OpenMode {
    pub fn creates(self) -> bool {
        matches!(
            self,
            Self::ReadWriteCreate
                | Self::WriteCreateExclusive
                | Self::WriteCreateTruncate
                | Self::WriteCreateAppend
        )
    }

    fn flags(self) -> rustix::fs::OFlags {
        use rustix::fs::OFlags;
        match self {
            Self::ReadOnly => OFlags::RDONLY,
            Self::ReadWrite => OFlags::RDWR,
            Self::ReadWriteCreate => OFlags::RDWR | OFlags::CREATE,
            Self::WriteOnly => OFlags::WRONLY,
            Self::WriteCreateExclusive => OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL,
            Self::WriteCreateTruncate => OFlags::WRONLY | OFlags::CREATE | OFlags::TRUNC,
            Self::WriteCreateAppend => OFlags::WRONLY | OFlags::CREATE | OFlags::APPEND,
        }
    }
}

/// Open a redirection target and transfer ownership of the descriptor number.
pub fn open_path(path: &Path, mode: OpenMode) -> std::io::Result<Descriptor> {
    rustix::fs::open(
        path,
        mode.flags() | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::from_bits_retain(0o666),
    )
    .map(Descriptor::from)
    .map_err(std::io::Error::from)
}

/// Whether an open descriptor refers to a regular file.
pub fn fd_is_regular_file(fd: &impl AsDescriptor) -> std::io::Result<bool> {
    let metadata =
        rustix::fs::fstat(fd.as_platform_descriptor().0).map_err(std::io::Error::from)?;
    Ok(rustix::fs::FileType::from_raw_mode(metadata.st_mode).is_file())
}

/// Create an anonymous in-memory file and transfer ownership of its descriptor.
// [spec:nsh:req:idiom.filesystem-account-bytes]
pub fn anonymous_file(name: impl AsRef<OsStr>) -> std::io::Result<Descriptor> {
    let name = name.as_ref();
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let name = CString::new(name.as_bytes())
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        rustix::fs::memfd_create(&name, rustix::fs::MemfdFlags::CLOEXEC)
            .map(Descriptor::from)
            .map_err(std::io::Error::from)
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let (file, path) = create_temporary_file(name)?;
        remove_file(&path)?;
        Ok(Descriptor::from(OwnedFd::from(file)))
    }
}

/// Create a temporary file in the host temporary directory.
/// The returned file owns the descriptor and is created atomically with mode
/// `0600`; an existing path is never followed or replaced.
pub fn create_temporary_file(name: impl AsRef<OsStr>) -> std::io::Result<(std::fs::File, PathBuf)> {
    const PLACEHOLDER: &[u8] = b"XXXXXX";
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

    let mut template = std::env::temp_dir().join(name.as_ref());
    template
        .as_mut_os_string()
        .push(format!("-{}-XXXXXX", std::process::id()));
    let template = template.as_os_str().as_bytes();
    let prefix = template
        .strip_suffix(PLACEHOLDER)
        .expect("the platform constructs the placeholder suffix");
    let random = RandomState::new();
    for attempt in 0_u8..=u8::MAX {
        let mut value = random.hash_one((std::process::id(), attempt));
        let mut path = Vec::with_capacity(prefix.len() + PLACEHOLDER.len());
        path.extend_from_slice(prefix);
        for _ in PLACEHOLDER {
            path.push(ALPHABET[value as usize % ALPHABET.len()]);
            value /= ALPHABET.len() as u64;
        }
        let path = OsString::from_vec(path);
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => return Ok((file, PathBuf::from(path))),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not create a unique temporary file",
    ))
}

/// Read a seekable descriptor from the beginning, then truncate and rewind it.
pub fn take_file_contents(fd: &impl AsDescriptor) -> std::io::Result<Vec<u8>> {
    let fd = fd.as_platform_descriptor().0;
    rustix::fs::seek(fd, rustix::fs::SeekFrom::Start(0)).map_err(std::io::Error::from)?;
    let mut out = Vec::new();
    let mut buf = [0_u8; 8192];
    loop {
        let count = rustix::io::read(fd, &mut buf).map_err(std::io::Error::from)?;
        if count == 0 {
            break;
        }
        out.extend_from_slice(&buf[..count]);
    }
    rustix::fs::ftruncate(fd, 0).map_err(std::io::Error::from)?;
    rustix::fs::seek(fd, rustix::fs::SeekFrom::Start(0)).map_err(std::io::Error::from)?;
    Ok(out)
}

/// Duplicate a descriptor at or above `minimum`, setting close-on-exec.
pub fn duplicate_cloexec(fd: &impl AsDescriptor, minimum: i32) -> std::io::Result<Descriptor> {
    rustix::io::fcntl_dupfd_cloexec(fd.as_platform_descriptor().0, minimum)
        .map(Descriptor::from)
        .map_err(|error| descriptor::normalize_dupfd_error(error, minimum))
}

/// Duplicate a descriptor to the lowest available descriptor number, with
/// close-on-exec set.
pub fn duplicate_fd(fd: &impl AsDescriptor) -> std::io::Result<Descriptor> {
    rustix::io::fcntl_dupfd_cloexec(fd.as_platform_descriptor().0, 0)
        .map(Descriptor::from)
        .map_err(std::io::Error::from)
}

/// Duplicate a descriptor with close-on-exec and return an owning Rust file.
pub fn duplicate_file(fd: &impl AsDescriptor) -> std::io::Result<std::fs::File> {
    rustix::io::fcntl_dupfd_cloexec(fd.as_platform_descriptor().0, 0)
        .map(std::fs::File::from)
        .map_err(std::io::Error::from)
}

/// Create a pipe and return both owned ends.
pub fn pipe() -> std::io::Result<(Descriptor, Descriptor)> {
    #[cfg(not(target_vendor = "apple"))]
    {
        rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)
            .map(|(read, write)| (Descriptor::from(read), Descriptor::from(write)))
            .map_err(std::io::Error::from)
    }
    #[cfg(target_vendor = "apple")]
    {
        let (read, write) = rustix::pipe::pipe().map_err(std::io::Error::from)?;
        for fd in [&read, &write] {
            let flags = rustix::io::fcntl_getfd(fd).map_err(std::io::Error::from)?;
            rustix::io::fcntl_setfd(fd, flags | rustix::io::FdFlags::CLOEXEC)
                .map_err(std::io::Error::from)?;
        }
        Ok((Descriptor::from(read), Descriptor::from(write)))
    }
}

/// Write the complete buffer to a descriptor.
pub fn write_all(fd: &impl AsDescriptor, mut bytes: &[u8]) -> std::io::Result<()> {
    let fd = fd.as_platform_descriptor().0;
    while !bytes.is_empty() {
        let count = match rustix::io::write(fd, bytes) {
            Ok(count) => count,
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => return Err(std::io::Error::from(error)),
        };
        if count == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "descriptor write returned zero",
            ));
        }
        bytes = &bytes[count..];
    }
    Ok(())
}

/// Perform one descriptor write, retrying only when interrupted.
pub fn write_once(fd: &impl AsDescriptor, bytes: &[u8]) -> std::io::Result<usize> {
    let fd = fd.as_platform_descriptor().0;
    loop {
        match rustix::io::write(fd, bytes) {
            Ok(count) => return Ok(count),
            Err(rustix::io::Errno::INTR) => {}
            Err(error) => return Err(std::io::Error::from(error)),
        }
    }
}

/// Read exactly `length` bytes from a descriptor.
pub fn read_exact(fd: &impl AsDescriptor, length: usize) -> std::io::Result<Vec<u8>> {
    let fd = fd.as_platform_descriptor().0;
    let mut out = vec![0_u8; length];
    let mut filled = 0;
    while filled < length {
        let count = rustix::io::read(fd, &mut out[filled..]).map_err(std::io::Error::from)?;
        if count == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "descriptor reached EOF",
            ));
        }
        filled += count;
    }
    Ok(out)
}

/// Read until EOF from a descriptor.
pub fn read_to_end(fd: &impl AsDescriptor) -> std::io::Result<Vec<u8>> {
    let fd = fd.as_platform_descriptor().0;
    let mut out = Vec::new();
    let mut buf = [0_u8; 8192];
    loop {
        let count = rustix::io::read(fd, &mut buf).map_err(std::io::Error::from)?;
        if count == 0 {
            return Ok(out);
        }
        out.extend_from_slice(&buf[..count]);
    }
}

/// Read at most `bytes.len()` bytes from a descriptor.
pub fn read_once(fd: &impl AsDescriptor, bytes: &mut [u8]) -> std::io::Result<usize> {
    rustix::io::read(fd.as_platform_descriptor().0, bytes).map_err(std::io::Error::from)
}

/// Copy bytes from one pipe to another without consuming the source.
pub fn tee(
    fd_in: &impl AsDescriptor,
    fd_out: &impl AsDescriptor,
    length: usize,
) -> std::io::Result<usize> {
    #[cfg(target_os = "linux")]
    {
        rustix::pipe::tee(
            fd_in.as_platform_descriptor().0,
            fd_out.as_platform_descriptor().0,
            length,
            rustix::pipe::SpliceFlags::empty(),
        )
        .map_err(std::io::Error::from)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (fd_in, fd_out, length);
        Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
    }
}

pub const fn supports_tee() -> bool {
    cfg!(target_os = "linux")
}

/// Add or remove nonblocking mode on a descriptor.
pub fn set_nonblocking(fd: &impl AsDescriptor, enabled: bool) -> std::io::Result<()> {
    let fd = fd.as_platform_descriptor().0;
    let mut flags = rustix::fs::fcntl_getfl(fd).map_err(std::io::Error::from)?;
    flags.set(rustix::fs::OFlags::NONBLOCK, enabled);
    rustix::fs::fcntl_setfl(fd, flags).map_err(std::io::Error::from)
}

pub const PIPE_BUFFER: usize = rustix::pipe::PIPE_BUF;

pub const fn reports_pipe_short_writes() -> bool {
    cfg!(target_os = "linux")
}

/// Create a controller/terminal pair for integration testing interactive I/O.
pub fn open_pseudoterminal() -> std::io::Result<(std::fs::File, std::fs::File)> {
    let controller =
        rustix::pty::openpt(rustix::pty::OpenptFlags::RDWR | rustix::pty::OpenptFlags::NOCTTY)
            .map_err(std::io::Error::from)?;
    rustix::pty::grantpt(&controller).map_err(std::io::Error::from)?;
    rustix::pty::unlockpt(&controller).map_err(std::io::Error::from)?;
    let slave_name = rustix::pty::ptsname(&controller, Vec::new()).map_err(std::io::Error::from)?;
    let terminal = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(OsStr::from_bytes(slave_name.to_bytes()))?;
    Ok((controller.into(), terminal))
}

pub const fn supports_bidirectional_pseudoterminal_pair() -> bool {
    true
}
