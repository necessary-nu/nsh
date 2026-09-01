//! What a shell does with an open endpoint: make one, hand one on, and
//! move bytes through it.
//!
//! The owned [`Descriptor`] lives in `descriptor.rs`, whose subject is
//! ownership. This is the operations, and the table that gives exact
//! numbers a meaning: standard slots are passed to an external image
//! explicitly, while higher slots stay a compatibility namespace private
//! to a tree of nsh processes. A cloned process cannot make a pipe on
//! its own, so `pipe` asks the broker first and only then falls back to
//! `CreatePipe`.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::{Read as _, Seek as _, Write as _};
use std::os::windows::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use windows_sys::Win32::Foundation::{
    DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_INVALID_HANDLE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_DELETE_ON_CLOSE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
};
use windows_sys::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Pipes::{
    CreatePipe, PIPE_NOWAIT, PIPE_WAIT, SetNamedPipeHandleState,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

use super::*;

pub fn fd_is_seekable(fd: &impl AsDescriptor) -> bool {
    duplicate_file(fd)
        .and_then(|mut file| file.stream_position())
        .is_ok()
}

pub fn seek_relative(fd: &impl AsDescriptor, offset: i64) -> std::io::Result<u64> {
    duplicate_file(fd)?.seek(std::io::SeekFrom::Current(offset))
}

pub fn seek_start(fd: &impl AsDescriptor) -> std::io::Result<u64> {
    duplicate_file(fd)?.seek(std::io::SeekFrom::Start(0))
}

thread_local! {
    // Exact POSIX descriptor numbers are a shell compatibility namespace on
    // Windows. Standard slots are passed explicitly when an external image is
    // created; higher slots remain available to nsh descendants without
    // pretending they are CRT file descriptors.
    static PROCESS_FDS: RefCell<HashMap<i32, Descriptor>> = RefCell::new(HashMap::new());
    static PROCESS_FDS_MATERIALIZED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(super) fn materialized_standard_handles() -> [HANDLE; 3] {
    if PROCESS_FDS_MATERIALIZED.with(std::cell::Cell::get) {
        return PROCESS_FDS.with(|slots| {
            let slots = slots.borrow();
            [0, 1, 2].map(|number| slots.get(&number).map_or(std::ptr::null_mut(), raw_handle))
        });
    }
    // SAFETY: each argument is a standard-handle selector and the returned
    // handles remain process-owned through CreateProcessW.
    unsafe {
        [
            GetStdHandle(STD_INPUT_HANDLE),
            GetStdHandle(STD_OUTPUT_HANDLE),
            GetStdHandle(STD_ERROR_HANDLE),
        ]
    }
}

fn duplicate_windows_handle(raw: HANDLE, minimum: i32) -> std::io::Result<Descriptor> {
    if raw.is_null() || raw == INVALID_HANDLE_VALUE || minimum < 0 {
        return Err(std::io::Error::from_raw_os_error(
            ERROR_INVALID_HANDLE as i32,
        ));
    }
    let mut duplicate = std::ptr::null_mut();
    // SAFETY: source and target process pseudo-handles are always valid and
    // the output slot receives one newly owned handle on success.
    let succeeded = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            raw,
            GetCurrentProcess(),
            &mut duplicate,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if succeeded == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        owned_handle(duplicate, minimum)
    }
}

// [spec:nsh:req:idiom.descriptor-materialization]
#[derive(Debug)]
pub struct ProcessDescriptorTransaction {
    changes: Vec<(i32, Option<Descriptor>)>,
}

impl ProcessDescriptorTransaction {
    pub fn new(
        changes: impl IntoIterator<Item = (i32, Option<Descriptor>)>,
    ) -> std::io::Result<Self> {
        let changes: Vec<_> = changes.into_iter().collect();
        let mut targets = BTreeSet::new();
        for (target, _) in &changes {
            if *target < 0 || !targets.insert(*target) {
                return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
            }
        }
        Ok(Self { changes })
    }

    pub fn apply(self) -> std::io::Result<()> {
        let result = PROCESS_FDS.with(|slots| {
            let mut slots = slots.borrow_mut();
            for (target, source) in self.changes {
                if let Some(source) = source {
                    let stored = duplicate_at(&source, 10)?;
                    slots.insert(target, stored);
                } else {
                    slots.remove(&target);
                }
            }
            Ok(())
        });
        if result.is_ok() {
            PROCESS_FDS_MATERIALIZED.with(|materialized| materialized.set(true));
        }
        result
    }
}

fn standard_handle_kind(number: i32) -> Option<u32> {
    match number {
        0 => Some(STD_INPUT_HANDLE),
        1 => Some(STD_OUTPUT_HANDLE),
        2 => Some(STD_ERROR_HANDLE),
        _ => None,
    }
}

pub fn snapshot_process_fd(number: i32, minimum: i32) -> std::io::Result<Option<Descriptor>> {
    if number < 0 || minimum < 0 {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    if PROCESS_FDS_MATERIALIZED.with(std::cell::Cell::get) {
        return PROCESS_FDS.with(|slots| {
            slots
                .borrow()
                .get(&number)
                .map(|source| duplicate_at(source, minimum))
                .transpose()
        });
    }
    let Some(kind) = standard_handle_kind(number) else {
        return Ok(None);
    };
    // SAFETY: the kind is a standard-handle selector and the returned value
    // is borrowed from the process table until explicitly duplicated below.
    let raw = unsafe { GetStdHandle(kind) };
    if raw.is_null() || raw == INVALID_HANDLE_VALUE {
        Ok(None)
    } else {
        duplicate_windows_handle(raw, minimum).map(Some)
    }
}

pub fn open_null_input() -> std::io::Result<Descriptor> {
    descriptor_from_file(File::open(null_device_path())?, 0)
}

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
}

pub fn open_path(path: &Path, mode: OpenMode) -> std::io::Result<Descriptor> {
    let mut options = OpenOptions::new();
    match mode {
        OpenMode::ReadOnly => {
            options.read(true);
        }
        OpenMode::ReadWrite => {
            options.read(true).write(true);
        }
        OpenMode::ReadWriteCreate => {
            options.read(true).write(true).create(true);
        }
        OpenMode::WriteOnly => {
            options.write(true);
        }
        OpenMode::WriteCreateExclusive => {
            options.write(true).create_new(true);
        }
        OpenMode::WriteCreateTruncate => {
            options.write(true).create(true).truncate(true);
        }
        OpenMode::WriteCreateAppend => {
            options.append(true).create(true);
        }
    }
    descriptor_from_file(options.open(path)?, 0)
}

pub fn fd_is_regular_file(fd: &impl AsDescriptor) -> std::io::Result<bool> {
    Ok(duplicate_file(fd)?.metadata()?.is_file())
}

static TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn create_temporary_file(name: impl AsRef<OsStr>) -> std::io::Result<(File, PathBuf)> {
    for _ in 0..256 {
        let unique = TEMPORARY_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
        let mut path = std::env::temp_dir().join(name.as_ref());
        path.as_mut_os_string().push(format!("-{unique:016x}.tmp"));
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .open(&path)
        {
            Ok(file) => return Ok((file, path)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not create a unique temporary file",
    ))
}

// [spec:nsh:req:idiom.filesystem-account-bytes]
pub fn anonymous_file(name: impl AsRef<OsStr>) -> std::io::Result<Descriptor> {
    for _ in 0..256 {
        let unique = TEMPORARY_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
        let mut path = std::env::temp_dir().join(name.as_ref());
        path.as_mut_os_string().push(format!("-{unique:016x}.tmp"));
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_DELETE_ON_CLOSE)
            .open(&path)
        {
            Ok(file) => return descriptor_from_file(file, 0),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not create an anonymous temporary file",
    ))
}

pub fn take_file_contents(fd: &impl AsDescriptor) -> std::io::Result<Vec<u8>> {
    let mut file = duplicate_file(fd)?;
    file.seek(std::io::SeekFrom::Start(0))?;
    let mut output = Vec::new();
    file.read_to_end(&mut output)?;
    file.set_len(0)?;
    file.seek(std::io::SeekFrom::Start(0))?;
    Ok(output)
}

pub fn pipe() -> std::io::Result<(Descriptor, Descriptor)> {
    if let Some(result) = broker_handle_pair(BROKER_PIPE) {
        return result;
    }
    direct_pipe()
}

pub(super) fn direct_pipe() -> std::io::Result<(Descriptor, Descriptor)> {
    let mut read = std::ptr::null_mut();
    let mut write = std::ptr::null_mut();
    // SAFETY: both output slots are writable; null security attributes make
    // the returned handles non-inheritable until nsh explicitly snapshots
    // them for a child.
    if unsafe { CreatePipe(&mut read, &mut write, std::ptr::null(), 0) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let read = owned_handle(read, 0)?;
    match owned_handle(write, 0) {
        Ok(write) => Ok((read, write)),
        Err(error) => Err(error),
    }
}

pub fn write_all(fd: &impl AsDescriptor, bytes: &[u8]) -> std::io::Result<()> {
    duplicate_file(fd)?.write_all(bytes)
}

pub fn write_once(fd: &impl AsDescriptor, bytes: &[u8]) -> std::io::Result<usize> {
    duplicate_file(fd)?.write(bytes)
}

pub fn read_exact(fd: &impl AsDescriptor, length: usize) -> std::io::Result<Vec<u8>> {
    let mut output = vec![0; length];
    duplicate_file(fd)?.read_exact(&mut output)?;
    Ok(output)
}

pub fn read_to_end(fd: &impl AsDescriptor) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    duplicate_file(fd)?.read_to_end(&mut output)?;
    Ok(output)
}

pub fn read_once(fd: &impl AsDescriptor, bytes: &mut [u8]) -> std::io::Result<usize> {
    duplicate_file(fd)?.read(bytes)
}

pub fn tee(
    _fd_in: &impl AsDescriptor,
    _fd_out: &impl AsDescriptor,
    _length: usize,
) -> std::io::Result<usize> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

pub const fn supports_tee() -> bool {
    false
}

pub fn set_nonblocking(fd: &impl AsDescriptor, enabled: bool) -> std::io::Result<()> {
    let mut mode = if enabled { PIPE_NOWAIT } else { PIPE_WAIT };
    // SAFETY: the descriptor is borrowed and mode is a valid pipe mode; the
    // optional size and timeout pointers are intentionally absent.
    if unsafe {
        SetNamedPipeHandleState(
            raw_handle(fd),
            &mut mode,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub const PIPE_BUFFER: usize = 4096;

pub const fn reports_pipe_short_writes() -> bool {
    false
}

pub fn open_pseudoterminal() -> std::io::Result<(File, File)> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "a ConPTY is directional and cannot be represented by the legacy two-file PTY API",
    ))
}

pub const fn supports_bidirectional_pseudoterminal_pair() -> bool {
    false
}
