//! Windows implementation of the shell's operating-system boundary.
//! Handles stay opaque; POSIX concepts use native Windows primitives.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io::{Read as _, Seek as _, Write as _};
use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::io::{AsHandle, AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    CloseHandle, DUPLICATE_CLOSE_SOURCE, DUPLICATE_SAME_ACCESS, DuplicateHandle,
    ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_BAD_EXE_FORMAT, ERROR_FILE_NOT_FOUND,
    ERROR_FILENAME_EXCED_RANGE, ERROR_INVALID_HANDLE, ERROR_PATH_NOT_FOUND, FILETIME, HANDLE,
    HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_DELETE_ON_CLOSE,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle, ReadFile,
    WriteFile,
};
use windows_sys::Win32::System::Console::{
    ATTACH_PARENT_PROCESS, AttachConsole, CONSOLE_SCREEN_BUFFER_INFO, CTRL_BREAK_EVENT,
    CTRL_C_EVENT, ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT, FreeConsole,
    GetConsoleMode, GetConsoleScreenBufferInfo, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE,
    STD_OUTPUT_HANDLE, SetConsoleCtrlHandler, SetConsoleMode,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Pipes::{
    CreatePipe, PIPE_NOWAIT, PIPE_WAIT, SetNamedPipeHandleState,
};
use windows_sys::Win32::System::SystemInformation::{GetSystemDirectoryW, GetWindowsDirectoryW};
use windows_sys::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, ExitProcess, GetCurrentProcess, GetCurrentProcessId,
    GetExitCodeProcess, GetProcessTimes, INFINITE, InitializeProcThreadAttributeList,
    LPPROC_THREAD_ATTRIBUTE_LIST, OpenProcess, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, ResumeThread,
    STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW, TerminateProcess,
    UpdateProcThreadAttribute, WaitForSingleObject,
};

use crate::{
    ChildStatus, ForkResult, ProcessGroupId, ProcessGroupState, ProcessId, ProcessSelector,
    ProcessTarget, Signal, SignalRequest,
};

#[path = "signal_names.rs"]
mod signal_names;
pub use signal_names::{SIGNAL_COUNT, SIGNAL_NAMES};

pub fn restore_shell_process_runtime_state() {}

pub fn flush_coverage_profile() {}

pub fn reset_coverage_counters() {}

pub fn named_user_home(name: &OsStr) -> Option<PathBuf> {
    let current = std::env::var_os("USERNAME")?;
    if current
        .to_string_lossy()
        .eq_ignore_ascii_case(&name.to_string_lossy())
    {
        std::env::var_os("USERPROFILE").map(PathBuf::from)
    } else {
        None
    }
}

/// Shell-specific operations on native strings without exposing Windows'
/// UTF-16 representation to the shell crate.
pub trait NativeStrExt {
    fn to_shell_bytes(&self) -> Vec<u8>;
}

impl NativeStrExt for OsStr {
    fn to_shell_bytes(&self) -> Vec<u8> {
        encode_wtf8(self)
    }
}

impl NativeStrExt for Path {
    fn to_shell_bytes(&self) -> Vec<u8> {
        self.as_os_str().to_shell_bytes()
    }
}

pub trait ShellBytesExt {
    fn try_to_os_string(&self) -> std::io::Result<OsString>;
    fn try_to_path_buf(&self) -> std::io::Result<PathBuf>;
}

impl ShellBytesExt for [u8] {
    fn try_to_os_string(&self) -> std::io::Result<OsString> {
        decode_wtf8(self).map(|wide| OsString::from_wide(&wide))
    }

    fn try_to_path_buf(&self) -> std::io::Result<PathBuf> {
        self.try_to_os_string().map(PathBuf::from)
    }
}

/// Encode Windows' potentially ill-formed UTF-16 as the shell's stable WTF-8
/// interchange representation. This deliberately does not depend on Rust's
/// unspecified internal `OsStr` encoding.
fn encode_wtf8(value: &OsStr) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut wide = value.encode_wide().peekable();
    while let Some(unit) = wide.next() {
        let scalar = if (0xd800..=0xdbff).contains(&unit)
            && wide
                .peek()
                .is_some_and(|next| (0xdc00..=0xdfff).contains(next))
        {
            let low = wide.next().expect("peeked low surrogate exists");
            0x10000 + ((u32::from(unit) - 0xd800) << 10) + (u32::from(low) - 0xdc00)
        } else {
            u32::from(unit)
        };
        match scalar {
            0x0000..=0x007f => bytes.push(scalar as u8),
            0x0080..=0x07ff => {
                bytes.push(0xc0 | (scalar >> 6) as u8);
                bytes.push(0x80 | (scalar & 0x3f) as u8);
            }
            0x0800..=0xffff => {
                bytes.push(0xe0 | (scalar >> 12) as u8);
                bytes.push(0x80 | ((scalar >> 6) & 0x3f) as u8);
                bytes.push(0x80 | (scalar & 0x3f) as u8);
            }
            _ => {
                bytes.push(0xf0 | (scalar >> 18) as u8);
                bytes.push(0x80 | ((scalar >> 12) & 0x3f) as u8);
                bytes.push(0x80 | ((scalar >> 6) & 0x3f) as u8);
                bytes.push(0x80 | (scalar & 0x3f) as u8);
            }
        }
    }
    bytes
}

/// Decode the shell's WTF-8 interchange representation. Shell-authored byte
/// sequences which are not valid WTF-8 cannot name a Windows native string
/// and are rejected instead of being silently redirected to a different name.
fn decode_wtf8(bytes: &[u8]) -> std::io::Result<Vec<u16>> {
    let mut wide = Vec::with_capacity(bytes.len());
    let mut offset = 0;
    while offset < bytes.len() {
        let first = bytes[offset];
        let (length, mut scalar, minimum) = match first {
            0x00..=0x7f => (1, u32::from(first), 0),
            0xc2..=0xdf => (2, u32::from(first & 0x1f), 0x80),
            0xe0..=0xef => (3, u32::from(first & 0x0f), 0x800),
            0xf0..=0xf4 => (4, u32::from(first & 0x07), 0x10000),
            _ => return Err(invalid_shell_string()),
        };
        if offset + length > bytes.len()
            || bytes[offset + 1..offset + length]
                .iter()
                .any(|byte| byte & 0xc0 != 0x80)
        {
            return Err(invalid_shell_string());
        }
        for byte in &bytes[offset + 1..offset + length] {
            scalar = (scalar << 6) | u32::from(byte & 0x3f);
        }
        if scalar < minimum || scalar > 0x10ffff {
            return Err(invalid_shell_string());
        }
        if scalar <= 0xffff {
            wide.push(scalar as u16);
        } else {
            let scalar = scalar - 0x10000;
            wide.push(0xd800 | ((scalar >> 10) as u16));
            wide.push(0xdc00 | ((scalar & 0x3ff) as u16));
        }
        offset += length;
    }
    Ok(wide)
}

fn invalid_shell_string() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "shell bytes are not valid WTF-8",
    )
}

pub fn process_arguments() -> Vec<OsString> {
    std::env::args_os().collect()
}

pub fn process_environment() -> Vec<(OsString, OsString)> {
    std::env::vars_os()
        .map(|(name, value)| {
            if name.eq_ignore_ascii_case(OsStr::new("PATH")) {
                (OsString::from("PATH"), with_shell_path_separators(value))
            } else {
                (name, value)
            }
        })
        .collect()
}

fn with_shell_path_separators(value: OsString) -> OsString {
    OsString::from_wide(
        &value
            .encode_wide()
            .map(|unit| {
                if unit == u16::from(b'\\') {
                    u16::from(b'/')
                } else {
                    unit
                }
            })
            .collect::<Vec<_>>(),
    )
}

static DEFAULT_SEARCH_PATH: LazyLock<OsString> = LazyLock::new(build_default_search_path);

pub fn default_search_path() -> OsString {
    DEFAULT_SEARCH_PATH.clone()
}

fn build_default_search_path() -> OsString {
    let system = query_windows_directory(GetSystemDirectoryW);
    let windows = query_windows_directory(GetWindowsDirectoryW);
    let mut directories: Vec<OsString> = [system, windows].into_iter().flatten().collect();

    if directories.is_empty()
        && let Some(root) = std::env::var_os("SystemRoot")
    {
        directories.push(Path::new(&root).join("System32").into_os_string());
        directories.push(root);
    }

    let mut path = OsString::new();
    for (index, directory) in directories.into_iter().enumerate() {
        if index != 0 {
            path.push(";");
        }
        path.push(directory);
    }
    with_shell_path_separators(path)
}

type WindowsDirectoryQuery = unsafe extern "system" fn(*mut u16, u32) -> u32;

fn query_windows_directory(query: WindowsDirectoryQuery) -> Option<OsString> {
    let mut size = 260_u32;
    loop {
        let mut buffer = vec![0_u16; size as usize];
        // SAFETY: `buffer` is writable for `size` UTF-16 units and the two
        // accepted query functions have the same buffer contract.
        let length = unsafe { query(buffer.as_mut_ptr(), size) };
        if length == 0 {
            return None;
        }
        if length < size {
            buffer.truncate(length as usize);
            return Some(OsString::from_wide(&buffer));
        }
        size = length.checked_add(1)?;
    }
}

pub fn fallback_shell() -> &'static OsStr {
    OsStr::new("nsh.exe")
}

pub fn controlling_terminal_path() -> &'static Path {
    Path::new("CONIN$")
}

pub fn null_device_path() -> &'static Path {
    Path::new("NUL")
}

pub const fn search_path_separator() -> u8 {
    b';'
}

pub const fn shell_directory_separator() -> u8 {
    b'/'
}

pub const fn input_newline_width(previous: Option<u8>) -> usize {
    match previous {
        Some(b'\r') => 2,
        _ => 1,
    }
}

pub fn resolve_command_path(path: &Path, environment: &[(OsString, OsString)]) -> PathBuf {
    if path.is_file() || path.extension().is_some() {
        return path.to_path_buf();
    }
    let extensions = environment
        .iter()
        .find(|(name, _)| name.to_string_lossy().eq_ignore_ascii_case("PATHEXT"))
        .map_or_else(
            || OsString::from(".COM;.EXE;.BAT;.CMD"),
            |(_, value)| value.clone(),
        );
    let wide: Vec<u16> = extensions.encode_wide().collect();
    for extension in wide.split(|unit| *unit == u16::from(b';')) {
        if extension.is_empty() {
            continue;
        }
        let mut candidate = path.as_os_str().to_os_string();
        candidate.push(OsString::from_wide(extension));
        let candidate = PathBuf::from(candidate);
        if candidate.is_file() {
            return candidate;
        }
    }
    path.to_path_buf()
}

pub fn trim_command_substitution_output(output: &mut Vec<u8>, start: usize) {
    while output.len() > start && output.last() == Some(&b'\n') {
        output.pop();
        if output.len() > start && output.last() == Some(&b'\r') {
            output.pop();
        }
    }
}

pub const fn supports_glob_metacharacters_in_filenames() -> bool {
    false
}

pub fn shell_path_has_separator(path: &[u8]) -> bool {
    path.iter().any(|byte| matches!(byte, b'/' | b'\\'))
}

pub fn shell_path_is_absolute(path: &[u8]) -> bool {
    path.first() == Some(&b'/') || path.try_to_path_buf().is_ok_and(|path| path.is_absolute())
}

pub fn shell_path_last_separator(path: &[u8]) -> Option<usize> {
    path.iter().rposition(|byte| matches!(byte, b'/' | b'\\'))
}

#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessMode {
    READ_OK,
    WRITE_OK,
    EXEC_OK,
}

pub fn effective_access(path: &Path, access: AccessMode) -> bool {
    match access {
        AccessMode::READ_OK => File::open(path).is_ok() || path.is_dir(),
        AccessMode::WRITE_OK => OpenOptions::new().write(true).open(path).is_ok(),
        // Search permission on a directory is the shell meaning of `-x`.
        // Windows ACL enforcement occurs when it is traversed/opened.
        AccessMode::EXEC_OK => path.is_dir() || path.is_file(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileKind {
    Regular,
    Directory,
    CharacterDevice,
    BlockDevice,
    Fifo,
    Socket,
    Symlink,
    Other,
}

#[derive(Clone, Copy, Debug)]
pub struct FileMetadata {
    pub kind: FileKind,
    pub mode: u32,
    pub size: u64,
    pub user: u32,
    pub group: u32,
    pub device: u64,
    pub inode: u64,
    pub modified_seconds: i64,
    pub modified_nanoseconds: i64,
}

fn file_information(path: &Path) -> Option<BY_HANDLE_FILE_INFORMATION> {
    let file = File::open(path).ok()?;
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `file` owns a live handle and the output record is writable.
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut information) };
    (succeeded != 0).then_some(information)
}

pub fn path_metadata(path: &Path, follow_links: bool) -> std::io::Result<FileMetadata> {
    let metadata = if follow_links {
        std::fs::metadata(path)
    } else {
        std::fs::symlink_metadata(path)
    }?;
    let file_type = metadata.file_type();
    let kind = if file_type.is_symlink() {
        FileKind::Symlink
    } else if file_type.is_dir() {
        FileKind::Directory
    } else if file_type.is_file() {
        FileKind::Regular
    } else {
        FileKind::Other
    };
    let information = file_information(path);
    let (device, inode) = information
        .map(|value| {
            (
                u64::from(value.dwVolumeSerialNumber),
                (u64::from(value.nFileIndexHigh) << 32) | u64::from(value.nFileIndexLow),
            )
        })
        .unwrap_or((0, 0));
    let modified = metadata.modified().ok();
    let duration = modified
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .unwrap_or_default();
    let attributes = information.map_or(0, |value| value.dwFileAttributes);
    let readonly = metadata.permissions().readonly();
    let mut mode = if kind == FileKind::Directory {
        0o040000
    } else {
        0o100000
    };
    mode |= if readonly { 0o444 } else { 0o666 };
    if kind == FileKind::Directory
        || path.extension().is_some_and(|extension| {
            ["exe", "com", "bat", "cmd"]
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
    {
        mode |= 0o111;
    }
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 && !follow_links {
        mode = 0o120777;
    }
    Ok(FileMetadata {
        kind,
        mode,
        size: metadata.len(),
        user: 0,
        group: 0,
        device,
        inode,
        modified_seconds: duration.as_secs() as i64,
        modified_nanoseconds: i64::from(duration.subsec_nanos()),
    })
}

pub fn path_exists(path: &Path) -> bool {
    path.exists()
}

pub fn path_is_file(path: &Path) -> bool {
    path.is_file()
}

pub fn path_is_directory(path: &Path) -> bool {
    path.is_dir()
}

pub fn path_is_same_file(left: &Path, right: &Path) -> bool {
    match (file_information(left), file_information(right)) {
        (Some(left), Some(right)) => {
            left.dwVolumeSerialNumber == right.dwVolumeSerialNumber
                && left.nFileIndexHigh == right.nFileIndexHigh
                && left.nFileIndexLow == right.nFileIndexLow
        }
        _ => false,
    }
}

pub fn current_directory() -> std::io::Result<PathBuf> {
    std::env::current_dir()
}

pub fn set_current_directory(path: &Path) -> std::io::Result<()> {
    std::env::set_current_dir(path)
}

pub fn absolute_path(path: &Path) -> std::io::Result<PathBuf> {
    std::path::absolute(path)
}

/// Construct a logical Windows path while retaining drive and UNC roots and
/// without resolving reparse points.
pub fn logical_path(current: Option<&Path>, directory: &Path) -> Option<PathBuf> {
    let combined = if directory.is_absolute() {
        directory.to_path_buf()
    } else if directory.has_root() {
        match current {
            Some(current) => current.join(directory),
            None => absolute_path(directory).ok()?,
        }
    } else {
        current?.join(directory)
    };
    let mut normalized = PathBuf::new();
    for component in combined.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    normalized.is_absolute().then_some(normalized)
}

/// Windows keeps an open reference to the process cwd and rejects deleting it.
pub const fn can_unlink_current_directory() -> bool {
    false
}

#[derive(Clone, Debug)]
pub struct DirectoryEntry {
    pub name: OsString,
    pub is_directory: bool,
    pub may_descend: bool,
}

pub fn read_directory(path: &Path) -> std::io::Result<Vec<DirectoryEntry>> {
    std::fs::read_dir(path)?
        .map(|entry| {
            let entry = entry?;
            let kind = entry.file_type()?;
            Ok(DirectoryEntry {
                name: entry.file_name(),
                is_directory: kind.is_dir(),
                may_descend: kind.is_dir() || kind.is_symlink(),
            })
        })
        .collect()
}

pub fn open_history_file(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
}

#[derive(Debug)]
pub struct Descriptor {
    handle: OwnedHandle,
    number: i32,
}

#[derive(Clone, Copy)]
pub struct BorrowedDescriptor<'a>(BorrowedHandle<'a>);

pub trait AsDescriptor {
    #[doc(hidden)]
    fn as_platform_descriptor(&self) -> BorrowedDescriptor<'_>;
}

impl AsDescriptor for Descriptor {
    fn as_platform_descriptor(&self) -> BorrowedDescriptor<'_> {
        BorrowedDescriptor(self.handle.as_handle())
    }
}

impl AsDescriptor for &Descriptor {
    fn as_platform_descriptor(&self) -> BorrowedDescriptor<'_> {
        BorrowedDescriptor(self.handle.as_handle())
    }
}

impl<T: AsHandle> AsDescriptor for T {
    fn as_platform_descriptor(&self) -> BorrowedDescriptor<'_> {
        BorrowedDescriptor(self.as_handle())
    }
}

static NEXT_DESCRIPTOR: AtomicU32 = AtomicU32::new(10);

fn descriptor_number(minimum: i32) -> std::io::Result<i32> {
    if minimum < 0 {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    let minimum = u32::try_from(minimum.max(10))
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let mut current = NEXT_DESCRIPTOR.load(AtomicOrdering::Relaxed);
    loop {
        let number = current.max(minimum);
        let next = number.checked_add(1).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::OutOfMemory,
                "descriptor namespace exhausted",
            )
        })?;
        match NEXT_DESCRIPTOR.compare_exchange_weak(
            current,
            next,
            AtomicOrdering::Relaxed,
            AtomicOrdering::Relaxed,
        ) {
            Ok(_) => return i32::try_from(number).map_err(std::io::Error::other),
            Err(observed) => current = observed,
        }
    }
}

fn owned_handle(raw: HANDLE, minimum: i32) -> std::io::Result<Descriptor> {
    if raw.is_null() || raw == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    // RtlCloneUserProcess only copies inheritable handles. Descriptor
    // lifetime remains owned here; this flag exists for the clone boundary,
    // while normal image creation is constrained by the standard-handle list.
    // SAFETY: `raw` is a live handle and both flag arguments are valid.
    if unsafe { SetHandleInformation(raw, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } == 0 {
        let error = std::io::Error::last_os_error();
        // SAFETY: failure leaves the freshly returned handle owned here.
        unsafe { CloseHandle(raw) };
        return Err(error);
    }
    // SAFETY: a successful Windows creation/duplication API returned a fresh
    // owned handle which is transferred into `OwnedHandle` exactly once.
    let handle = unsafe { OwnedHandle::from_raw_handle(raw.cast()) };
    Ok(Descriptor {
        handle,
        number: descriptor_number(minimum)?,
    })
}

fn raw_handle(fd: &impl AsDescriptor) -> HANDLE {
    fd.as_platform_descriptor().0.as_raw_handle() as HANDLE
}

fn descriptor_from_file(file: File, minimum: i32) -> std::io::Result<Descriptor> {
    let raw = file.as_raw_handle() as HANDLE;
    // SAFETY: `file` keeps the handle live and both flag values are valid.
    if unsafe { SetHandleInformation(raw, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let handle: OwnedHandle = file.into();
    Ok(Descriptor {
        handle,
        number: descriptor_number(minimum)?,
    })
}

fn duplicate_at(fd: &impl AsDescriptor, minimum: i32) -> std::io::Result<Descriptor> {
    if minimum < 0 {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    let mut duplicate = std::ptr::null_mut();
    // SAFETY: source and target process pseudo-handles are always valid, the
    // source is borrowed for this call, and `duplicate` is writable.
    let succeeded = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            raw_handle(fd),
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

impl Descriptor {
    // [spec:nsh:req:idiom.no-raw-fd-core]
    pub(crate) fn number(&self) -> i32 {
        self.number
    }

    pub fn into_file(self) -> File {
        self.handle.into()
    }
}

pub fn duplicate_cloexec(fd: &impl AsDescriptor, minimum: i32) -> std::io::Result<Descriptor> {
    duplicate_at(fd, minimum)
}

pub fn duplicate_fd(fd: &impl AsDescriptor) -> std::io::Result<Descriptor> {
    duplicate_at(fd, 0)
}

pub fn duplicate_file(fd: &impl AsDescriptor) -> std::io::Result<File> {
    duplicate_at(fd, 0).map(Descriptor::into_file)
}

pub fn move_fd_cloexec(fd: Descriptor, minimum: i32) -> std::io::Result<Descriptor> {
    if minimum < 0 {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    if fd.number() >= minimum {
        Ok(fd)
    } else {
        duplicate_at(&fd, minimum)
    }
}

pub enum LocaleDecode {
    Incomplete,
    Complete(i32),
    Invalid,
}

pub struct LocaleDecoder {
    bytes: Vec<u8>,
}

impl LocaleDecoder {
    pub fn push(&mut self, byte: u8) -> LocaleDecode {
        self.bytes.push(byte);
        let expected = match self.bytes[0] {
            0x00..=0x7f => 1,
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => {
                self.bytes.clear();
                return LocaleDecode::Invalid;
            }
        };
        if self.bytes.len() < expected {
            return LocaleDecode::Incomplete;
        }
        let result = std::str::from_utf8(&self.bytes)
            .ok()
            .and_then(|value| value.chars().next())
            .map(|value| LocaleDecode::Complete(value as i32))
            .unwrap_or(LocaleDecode::Invalid);
        self.bytes.clear();
        result
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocaleCategory {
    Collate,
    Ctype,
    Messages,
    Monetary,
    Numeric,
    Time,
}

/// Windows builds use Unicode classification and UTF-8 shell text. The name
/// is retained so locale environment variables remain observable, while no
/// process-global C locale is changed.
#[derive(Clone)]
pub struct Locale {
    _name: Vec<u8>,
}

impl Locale {
    pub fn new(base: &[u8], overrides: &[(LocaleCategory, &[u8])]) -> std::io::Result<Self> {
        fn supported(name: &[u8]) -> bool {
            let Ok(name) = std::str::from_utf8(name) else {
                return false;
            };
            name.eq_ignore_ascii_case("C")
                || name.eq_ignore_ascii_case("POSIX")
                || name.eq_ignore_ascii_case("UTF-8")
                || name.eq_ignore_ascii_case("UTF8")
                || name.to_ascii_uppercase().ends_with(".UTF-8")
                || name.to_ascii_uppercase().ends_with(".UTF8")
        }

        if !supported(base) || overrides.iter().any(|(_, name)| !supported(name)) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Windows nsh supports C, POSIX, and UTF-8 locales",
            ));
        }
        Ok(Self {
            _name: overrides.last().map_or(base, |(_, name)| *name).to_vec(),
        })
    }

    pub fn c() -> std::io::Result<Self> {
        Self::new(b"C", &[])
    }

    pub fn decoder(&self) -> LocaleDecoder {
        LocaleDecoder { bytes: Vec::new() }
    }

    pub fn is_alpha(&self, byte: u8) -> bool {
        byte.is_ascii_alphabetic()
    }

    pub fn is_alphanumeric(&self, byte: u8) -> bool {
        byte.is_ascii_alphanumeric()
    }

    pub fn is_space(&self, byte: u8) -> bool {
        matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
    }

    pub fn wide_is_blank(&self, wide: i32) -> bool {
        char::from_u32(wide as u32).is_some_and(|value| matches!(value, ' ' | '\t'))
    }

    pub fn wide_is_space(&self, wide: i32) -> bool {
        char::from_u32(wide as u32).is_some_and(char::is_whitespace)
    }

    pub fn multibyte_len(&self, bytes: &[u8]) -> Option<usize> {
        let first = *bytes.first()?;
        let length = match first {
            0x00..=0x7f => 1,
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => return None,
        };
        (bytes.len() >= length && std::str::from_utf8(&bytes[..length]).is_ok()).then_some(length)
    }

    pub fn decode_exact(&self, bytes: &[u8], expected_len: usize) -> Option<i32> {
        let value = std::str::from_utf8(bytes.get(..expected_len)?).ok()?;
        let mut chars = value.chars();
        let first = chars.next()?;
        (first.len_utf8() == expected_len).then_some(first as i32)
    }

    pub fn wide_class_matches(
        &self,
        name: &[u8],
        bytes: &[u8],
        expected_len: usize,
    ) -> Option<bool> {
        let wide = self.decode_exact(bytes, expected_len)?;
        let value = char::from_u32(wide as u32)?;
        Some(match name {
            b"alnum" => value.is_alphanumeric(),
            b"alpha" => value.is_alphabetic(),
            b"blank" => matches!(value, ' ' | '\t'),
            b"cntrl" => value.is_control(),
            b"digit" => value.is_ascii_digit(),
            b"graph" => !value.is_control() && !value.is_whitespace(),
            b"lower" => value.is_lowercase(),
            b"print" => !value.is_control(),
            b"punct" => value.is_ascii_punctuation(),
            b"space" => value.is_whitespace(),
            b"upper" => value.is_uppercase(),
            b"xdigit" => value.is_ascii_hexdigit(),
            _ => return None,
        })
    }

    pub fn wide_chars(&self, bytes: &[u8]) -> (usize, Vec<i32>) {
        if bytes.is_empty() {
            return (0, Vec::new());
        }
        let first_len = self.multibyte_len(bytes).unwrap_or(1);
        let mut decoded = vec![0_i32; bytes.len() + 1];
        if let Ok(text) = std::str::from_utf8(bytes) {
            for (slot, value) in decoded.iter_mut().zip(text.chars()) {
                *slot = value as i32;
            }
        }
        (first_len, decoded)
    }

    pub fn collate(&self, left: &[u8], right: &[u8]) -> Ordering {
        left.cmp(right)
    }

    pub fn collating_bracket_matches(&self, pattern: &[u8], subject: &[u8]) -> bool {
        if pattern.len() >= 7 && pattern.starts_with(b"[[.") && pattern.ends_with(b".]]") {
            let member = &pattern[3..pattern.len() - 3];
            return member.len() == 1 && member == subject;
        }
        if pattern.len() >= 7 && pattern.starts_with(b"[[=") && pattern.ends_with(b"=]]") {
            let member = &pattern[3..pattern.len() - 3];
            return member.len() == 1 && member == subject;
        }
        false
    }

    pub fn error_message(&self, error: &std::io::Error) -> String {
        let Some(code) = error.raw_os_error() else {
            return error.to_string();
        };
        let rendered = error.to_string();
        let suffix = format!(" (os error {code})");
        rendered
            .strip_suffix(&suffix)
            .unwrap_or(&rendered)
            .to_owned()
    }

    pub fn range_error_message(&self) -> String {
        "Result too large".to_owned()
    }

    pub fn signal_description(&self, signal: Signal) -> Vec<u8> {
        let description = match signal.number() {
            1 => "Hangup",
            2 => "Interrupt",
            3 => "Quit",
            9 => "Killed",
            13 => "Broken pipe",
            15 => "Terminated",
            17 => "Child status changed",
            18 => "Continued",
            20 => "Terminal stop",
            _ => return signal.number().to_string().into_bytes(),
        };
        description.as_bytes().to_vec()
    }
}

#[derive(Clone, Copy)]
pub struct TerminalSettings(u32);

impl TerminalSettings {
    pub fn capture(fd: &impl AsDescriptor) -> std::io::Result<Self> {
        let mut mode = 0;
        // SAFETY: the descriptor is borrowed and `mode` is writable.
        if unsafe { GetConsoleMode(raw_handle(fd), &mut mode) } == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(Self(mode))
        }
    }

    pub fn apply(&self, fd: &impl AsDescriptor) -> std::io::Result<()> {
        // SAFETY: the descriptor is borrowed and the mode came from the
        // console API for this class of handle.
        if unsafe { SetConsoleMode(raw_handle(fd), self.0) } == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalApply {
    AfterOutput,
    AfterOutputAndDiscardInput,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalControlCharacter {
    Erase,
    Kill,
    EndOfFile,
    WordErase,
    LiteralNext,
    Reprint,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EditorTerminalAttributes(u32);

impl EditorTerminalAttributes {
    pub fn for_editing(self) -> Self {
        Self(self.0 & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT))
    }

    pub fn for_quoted_input(self) -> Self {
        Self(self.0 & !ENABLE_PROCESSED_INPUT)
    }

    pub fn control_character(self, character: TerminalControlCharacter) -> u8 {
        match character {
            TerminalControlCharacter::Erase => 8,
            TerminalControlCharacter::Kill => 21,
            TerminalControlCharacter::EndOfFile => 26,
            TerminalControlCharacter::WordErase => 23,
            TerminalControlCharacter::LiteralNext => 22,
            TerminalControlCharacter::Reprint => 18,
        }
    }
}

pub fn editor_terminal_attributes(
    input: &impl AsDescriptor,
) -> std::io::Result<EditorTerminalAttributes> {
    TerminalSettings::capture(input).map(|settings| EditorTerminalAttributes(settings.0))
}

pub fn apply_editor_terminal_attributes(
    input: &impl AsDescriptor,
    _when: TerminalApply,
    attributes: &EditorTerminalAttributes,
) -> std::io::Result<()> {
    TerminalSettings(attributes.0).apply(input)
}

pub fn editor_terminal_size(output: &impl AsDescriptor) -> std::io::Result<(usize, usize)> {
    let mut information = CONSOLE_SCREEN_BUFFER_INFO::default();
    // SAFETY: the output handle is borrowed and the record is writable.
    if unsafe { GetConsoleScreenBufferInfo(raw_handle(output), &mut information) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let columns = i32::from(information.srWindow.Right - information.srWindow.Left + 1);
    let rows = i32::from(information.srWindow.Bottom - information.srWindow.Top + 1);
    Ok((columns.max(0) as usize, rows.max(0) as usize))
}

pub fn wait_for_terminal_input(
    input: &impl AsDescriptor,
    timeout: Duration,
) -> std::io::Result<bool> {
    let milliseconds = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX - 1);
    // SAFETY: the descriptor is borrowed for the duration of the wait.
    match unsafe { WaitForSingleObject(raw_handle(input), milliseconds) } {
        WAIT_OBJECT_0 => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        _ => Err(std::io::Error::last_os_error()),
    }
}

pub fn terminal_canonical_mode(fd: &impl AsDescriptor) -> Option<bool> {
    TerminalSettings::capture(fd)
        .ok()
        .map(|settings| settings.0 & ENABLE_LINE_INPUT != 0)
}

pub fn is_terminal(fd: &impl AsDescriptor) -> bool {
    TerminalSettings::capture(fd).is_ok()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserId(u32);

impl UserId {
    pub fn is_root(self) -> bool {
        false
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

pub fn effective_uid() -> UserId {
    UserId(1)
}

pub fn effective_gid() -> GroupId {
    GroupId(1)
}

pub fn supplementary_groups() -> std::io::Result<Vec<GroupId>> {
    Ok(Vec::new())
}

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

pub fn execute_program(
    path: &OsStr,
    argv: &[OsString],
    environment: &[(OsString, OsString)],
) -> std::io::Error {
    if let Some(result) = execute_through_clone_broker(path, argv, environment) {
        return match result {
            Ok(code) => exit_immediately(code as i32),
            Err(error) => error,
        };
    }
    execute_program_here(
        path,
        argv,
        environment,
        materialized_standard_handles(),
        None,
    )
}

fn execute_program_here(
    path: &OsStr,
    argv: &[OsString],
    environment: &[(OsString, OsString)],
    handles: [HANDLE; 3],
    job: Option<HANDLE>,
) -> std::io::Error {
    match spawn_program_here(path, argv, environment, handles, job) {
        Ok(code) => exit_immediately(code as i32),
        Err(error) => error,
    }
}

fn spawn_program_here(
    path: &OsStr,
    argv: &[OsString],
    environment: &[(OsString, OsString)],
    handles: [HANDLE; 3],
    job: Option<HANDLE>,
) -> std::io::Result<u32> {
    let is_batch = Path::new(path).extension().is_some_and(|extension| {
        let extension = extension.to_string_lossy();
        extension.eq_ignore_ascii_case("bat") || extension.eq_ignore_ascii_case("cmd")
    });
    let application_path = if is_batch {
        environment
            .iter()
            .find(|(name, _)| name.to_string_lossy().eq_ignore_ascii_case("COMSPEC"))
            .map_or_else(|| OsString::from("cmd.exe"), |(_, value)| value.clone())
    } else {
        path.to_os_string()
    };
    let batch_path = is_batch.then(|| {
        OsString::from_wide(
            &path
                .encode_wide()
                .map(|unit| {
                    if unit == u16::from(b'/') {
                        u16::from(b'\\')
                    } else {
                        unit
                    }
                })
                .collect::<Vec<_>>(),
        )
    });
    let mut application: Vec<u16> = application_path.encode_wide().collect();
    if application.contains(&0) {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    application.push(0);

    let mut command_line = Vec::new();
    if is_batch {
        append_windows_argument(&mut command_line, &application_path)?;
        for option in ["/d", "/s", "/c"] {
            command_line.push(u16::from(b' '));
            command_line.extend(option.encode_utf16());
        }
        command_line.extend([u16::from(b' '), u16::from(b'"')]);
        append_windows_argument(
            &mut command_line,
            batch_path
                .as_deref()
                .expect("batch path exists for a batch file"),
        )?;
        for argument in argv.iter().skip(1) {
            command_line.push(u16::from(b' '));
            append_windows_argument(&mut command_line, argument)?;
        }
        command_line.push(u16::from(b'"'));
    } else {
        for (index, argument) in argv.iter().enumerate() {
            if index != 0 {
                command_line.push(u16::from(b' '));
            }
            append_windows_argument(&mut command_line, argument)?;
        }
    }
    command_line.push(0);

    let environment = match windows_environment_block(environment) {
        Ok(environment) => environment,
        Err(error) => return Err(error),
    };
    let mut inherited_handles: Vec<_> = handles
        .into_iter()
        .filter(|handle| !handle.is_null() && *handle != INVALID_HANDLE_VALUE)
        .collect();
    inherited_handles.sort_unstable_by_key(|handle| *handle as usize);
    inherited_handles.dedup();
    let attribute_list = if inherited_handles.is_empty() {
        None
    } else {
        Some(ProcessAttributeList::for_handles(&inherited_handles)?)
    };
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo = STARTUPINFOW {
        cb: if attribute_list.is_some() {
            std::mem::size_of::<STARTUPINFOEXW>() as u32
        } else {
            std::mem::size_of::<STARTUPINFOW>() as u32
        },
        dwFlags: STARTF_USESTDHANDLES,
        hStdInput: handles[0],
        hStdOutput: handles[1],
        hStdError: handles[2],
        ..STARTUPINFOW::default()
    };
    startup.lpAttributeList = attribute_list
        .as_ref()
        .map_or(std::ptr::null_mut(), |list| list.pointer);
    let mut information = PROCESS_INFORMATION::default();
    // SAFETY: application, command line, and environment are terminated and
    // live for the call; the command line is writable as CreateProcessW
    // requires; startup references live inheritable handles; and the process
    // information record is writable.
    let succeeded = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            i32::from(attribute_list.is_some()),
            CREATE_UNICODE_ENVIRONMENT
                | windows_sys::Win32::System::Threading::CREATE_SUSPENDED
                | if attribute_list.is_some() {
                    EXTENDED_STARTUPINFO_PRESENT
                } else {
                    0
                },
            environment.as_ptr().cast(),
            std::ptr::null(),
            &startup.StartupInfo,
            &mut information,
        )
    };
    if succeeded == 0 {
        return Err(std::io::Error::last_os_error());
    }
    if let Some(job) = job {
        // SAFETY: both handles were kept live by the broker and the process
        // is still suspended, so no descendant can escape before assignment.
        if unsafe { AssignProcessToJobObject(job, information.hProcess) } == 0 {
            let error = std::io::Error::last_os_error();
            // SAFETY: both handles are owned here and the suspended process
            // must not be left behind on an assignment failure.
            unsafe {
                TerminateProcess(information.hProcess, 1);
                CloseHandle(information.hThread);
                CloseHandle(information.hProcess);
            }
            return Err(error);
        }
    }
    // SAFETY: CreateProcessW returned the initial thread suspended.
    if unsafe { ResumeThread(information.hThread) } == u32::MAX {
        let error = std::io::Error::last_os_error();
        // SAFETY: both process-information handles remain owned here.
        unsafe {
            TerminateProcess(information.hProcess, 1);
            CloseHandle(information.hThread);
            CloseHandle(information.hProcess);
        }
        return Err(error);
    }
    // SAFETY: CreateProcessW initialized both owned handles. The initial
    // thread need not remain open while the process runs.
    unsafe { CloseHandle(information.hThread) };
    // SAFETY: the process handle remains live through the wait.
    if unsafe { WaitForSingleObject(information.hProcess, INFINITE) } != WAIT_OBJECT_0 {
        let error = std::io::Error::last_os_error();
        // SAFETY: this is the one remaining owned process handle.
        unsafe { CloseHandle(information.hProcess) };
        return Err(error);
    }
    let mut code = 1;
    // SAFETY: the signalled process has a final status and `code` is writable.
    let got_status = unsafe { GetExitCodeProcess(information.hProcess, &mut code) };
    // SAFETY: waiting and status collection are complete.
    unsafe { CloseHandle(information.hProcess) };
    if got_status == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(code)
}

struct ProcessAttributeList {
    _storage: Vec<usize>,
    pointer: LPPROC_THREAD_ATTRIBUTE_LIST,
}

impl ProcessAttributeList {
    fn for_handles(handles: &[HANDLE]) -> std::io::Result<Self> {
        let mut bytes = 0;
        // SAFETY: a null first call asks Windows for the required byte count.
        unsafe {
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut bytes);
        }
        if bytes == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut storage = vec![0_usize; bytes.div_ceil(std::mem::size_of::<usize>())];
        let pointer = storage.as_mut_ptr().cast();
        // SAFETY: `storage` is aligned and has at least the requested size.
        if unsafe { InitializeProcThreadAttributeList(pointer, 1, 0, &mut bytes) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: the list is initialized, and the handle slice remains live
        // until CreateProcessW returns.
        if unsafe {
            UpdateProcThreadAttribute(
                pointer,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_ptr().cast(),
                std::mem::size_of_val(handles),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        } == 0
        {
            let error = std::io::Error::last_os_error();
            // SAFETY: initialization succeeded, so the list must be deleted.
            unsafe { DeleteProcThreadAttributeList(pointer) };
            return Err(error);
        }
        Ok(Self {
            _storage: storage,
            pointer,
        })
    }
}

impl Drop for ProcessAttributeList {
    fn drop(&mut self) {
        // SAFETY: `pointer` remains backed by `_storage` and is deleted once.
        unsafe { DeleteProcThreadAttributeList(self.pointer) };
    }
}

struct CloneBrokerChild {
    request: Descriptor,
    response: Descriptor,
}

const BROKER_SPAWN: u32 = 1;
const BROKER_PIPE: u32 = 2;
const BROKER_CHANNEL: u32 = 3;
const BROKER_REGISTER: u32 = 4;

thread_local! {
    static CLONE_BROKER: RefCell<Option<CloneBrokerChild>> = const { RefCell::new(None) };
}

fn execute_through_clone_broker(
    path: &OsStr,
    argv: &[OsString],
    environment: &[(OsString, OsString)],
) -> Option<std::io::Result<u32>> {
    let request = match encode_spawn_request(path, argv, environment) {
        Ok(request) => request,
        Err(error) => return Some(Err(error)),
    };
    let response = match clone_broker_exchange(&request, 8)? {
        Ok(response) => response,
        Err(error) => return Some(Err(error)),
    };
    let kind = u32::from_le_bytes(response[..4].try_into().unwrap());
    let value = u32::from_le_bytes(response[4..].try_into().unwrap());
    Some(if kind == 0 {
        Ok(value)
    } else {
        Err(std::io::Error::from_raw_os_error(value as i32))
    })
}

fn clone_broker_exchange(
    payload: &[u8],
    response_length: usize,
) -> Option<std::io::Result<Vec<u8>>> {
    CLONE_BROKER.with(|broker| {
        let broker = broker.borrow();
        let broker = broker.as_ref()?;
        Some(broker_exchange_on(
            &broker.request,
            &broker.response,
            payload,
            response_length,
        ))
    })
}

fn broker_exchange_on(
    request: &Descriptor,
    response: &Descriptor,
    payload: &[u8],
    response_length: usize,
) -> std::io::Result<Vec<u8>> {
    let length = u32::try_from(payload.len()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "broker request is too large",
        )
    })?;
    let mut framed = Vec::with_capacity(payload.len() + 4);
    framed.extend_from_slice(&length.to_le_bytes());
    framed.extend_from_slice(payload);
    write_all(request, &framed)?;
    read_exact(response, response_length)
}

fn broker_handle_pair(operation: u32) -> Option<std::io::Result<(Descriptor, Descriptor)>> {
    let response = match clone_broker_exchange(&operation.to_le_bytes(), 24)? {
        Ok(response) => response,
        Err(error) => return Some(Err(error)),
    };
    let kind = u32::from_le_bytes(response[..4].try_into().unwrap());
    let value = u32::from_le_bytes(response[4..8].try_into().unwrap());
    if kind != 0 {
        return Some(Err(std::io::Error::from_raw_os_error(value as i32)));
    }
    let first = u64::from_le_bytes(response[8..16].try_into().unwrap());
    let second = u64::from_le_bytes(response[16..24].try_into().unwrap());
    let result = (|| {
        let first = usize::try_from(first)
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))?
            as HANDLE;
        let second = usize::try_from(second)
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))?
            as HANDLE;
        let first = owned_handle(first, 0)?;
        match owned_handle(second, 0) {
            Ok(second) => Ok((first, second)),
            Err(error) => Err(error),
        }
    })();
    Some(result)
}

fn register_clone_broker(
    request: &Descriptor,
    response: &Descriptor,
    process: HANDLE,
    job: Option<HANDLE>,
) -> std::io::Result<()> {
    let mut payload = Vec::with_capacity(20);
    payload.extend_from_slice(&BROKER_REGISTER.to_le_bytes());
    payload.extend_from_slice(&(process as usize as u64).to_le_bytes());
    payload.extend_from_slice(&(job.unwrap_or(std::ptr::null_mut()) as usize as u64).to_le_bytes());
    let response = broker_exchange_on(request, response, &payload, 8)?;
    let kind = u32::from_le_bytes(response[..4].try_into().unwrap());
    let value = u32::from_le_bytes(response[4..].try_into().unwrap());
    if kind == 0 {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(value as i32))
    }
}

fn encode_spawn_request(
    path: &OsStr,
    argv: &[OsString],
    environment: &[(OsString, OsString)],
) -> std::io::Result<Vec<u8>> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&BROKER_SPAWN.to_le_bytes());
    encode_wide_value(&mut payload, path)?;
    encode_count(&mut payload, argv.len())?;
    for argument in argv {
        encode_wide_value(&mut payload, argument)?;
    }
    encode_count(&mut payload, environment.len())?;
    for (name, value) in environment {
        encode_wide_value(&mut payload, name)?;
        encode_wide_value(&mut payload, value)?;
    }
    for handle in materialized_standard_handles() {
        payload.extend_from_slice(&(handle as usize as u64).to_le_bytes());
    }
    Ok(payload)
}

fn encode_count(output: &mut Vec<u8>, count: usize) -> std::io::Result<()> {
    let count =
        u32::try_from(count).map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    output.extend_from_slice(&count.to_le_bytes());
    Ok(())
}

fn encode_wide_value(output: &mut Vec<u8>, value: &OsStr) -> std::io::Result<()> {
    let wide: Vec<u16> = value.encode_wide().collect();
    if wide.contains(&0) {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    encode_count(output, wide.len())?;
    for unit in wide {
        output.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(())
}

struct SpawnRequest {
    path: OsString,
    argv: Vec<OsString>,
    environment: Vec<(OsString, OsString)>,
    handles: [u64; 3],
}

struct RequestCursor<'a> {
    remaining: &'a [u8],
}

impl<'a> RequestCursor<'a> {
    fn u32(&mut self) -> std::io::Result<u32> {
        let value = self.take(4)?;
        Ok(u32::from_le_bytes(value.try_into().unwrap()))
    }

    fn u64(&mut self) -> std::io::Result<u64> {
        let value = self.take(8)?;
        Ok(u64::from_le_bytes(value.try_into().unwrap()))
    }

    fn native(&mut self) -> std::io::Result<OsString> {
        let length = usize::try_from(self.u32()?)
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))?;
        let bytes = self.take(
            length
                .checked_mul(2)
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidData))?,
        )?;
        let wide: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
            .collect();
        Ok(OsString::from_wide(&wide))
    }

    fn take(&mut self, length: usize) -> std::io::Result<&'a [u8]> {
        let (value, remaining) = self.remaining.split_at_checked(length).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "truncated spawn request")
        })?;
        self.remaining = remaining;
        Ok(value)
    }
}

fn decode_spawn_request(payload: &[u8]) -> std::io::Result<SpawnRequest> {
    let mut cursor = RequestCursor { remaining: payload };
    let path = cursor.native()?;
    let argument_count = usize::try_from(cursor.u32()?)
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))?;
    let mut argv = Vec::with_capacity(argument_count.min(4096));
    for _ in 0..argument_count {
        argv.push(cursor.native()?);
    }
    let environment_count = usize::try_from(cursor.u32()?)
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))?;
    let mut environment = Vec::with_capacity(environment_count.min(16_384));
    for _ in 0..environment_count {
        environment.push((cursor.native()?, cursor.native()?));
    }
    let handles = [cursor.u64()?, cursor.u64()?, cursor.u64()?];
    if !cursor.remaining.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "spawn request has trailing data",
        ));
    }
    Ok(SpawnRequest {
        path,
        argv,
        environment,
        handles,
    })
}

fn duplicate_handle_from_process(
    process: &OwnedHandle,
    raw: u64,
    inherit: bool,
) -> std::io::Result<Option<OwnedHandle>> {
    if raw == 0 || raw == usize::MAX as u64 {
        return Ok(None);
    }
    let raw = usize::try_from(raw)
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))?
        as HANDLE;
    let mut duplicate = std::ptr::null_mut();
    // SAFETY: the broker owns a query/duplication-capable child process
    // handle, the raw handle was supplied by that child, and the output slot
    // receives one parent-owned inheritable duplicate on success.
    if unsafe {
        DuplicateHandle(
            process.as_raw_handle() as HANDLE,
            raw,
            GetCurrentProcess(),
            &mut duplicate,
            0,
            i32::from(inherit),
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        // SAFETY: DuplicateHandle returned one newly owned handle.
        Ok(Some(unsafe {
            OwnedHandle::from_raw_handle(duplicate.cast())
        }))
    }
}

fn duplicate_handle_into_process(
    process: &OwnedHandle,
    source: &impl AsDescriptor,
) -> std::io::Result<u64> {
    let mut remote = std::ptr::null_mut();
    // SAFETY: both process handles are valid, `source` is live, and the
    // returned handle is created in the target process's table.
    if unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            raw_handle(source),
            process.as_raw_handle() as HANDLE,
            &mut remote,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(remote as usize as u64)
    }
}

fn close_handle_in_process(process: &OwnedHandle, raw: u64) {
    let Ok(raw) = usize::try_from(raw) else {
        return;
    };
    // SAFETY: DUPLICATE_CLOSE_SOURCE closes the target-process handle; no
    // local duplicate is requested.
    unsafe {
        DuplicateHandle(
            process.as_raw_handle() as HANDLE,
            raw as HANDLE,
            GetCurrentProcess(),
            std::ptr::null_mut(),
            0,
            0,
            DUPLICATE_CLOSE_SOURCE,
        );
    }
}

fn pipe_handles_for_process(process: &OwnedHandle) -> std::io::Result<[u64; 2]> {
    let (read, write) = direct_pipe()?;
    let remote_read = duplicate_handle_into_process(process, &read)?;
    match duplicate_handle_into_process(process, &write) {
        Ok(remote_write) => Ok([remote_read, remote_write]),
        Err(error) => {
            close_handle_in_process(process, remote_read);
            Err(error)
        }
    }
}

fn broker_result_message(result: std::io::Result<u32>) -> Vec<u8> {
    let (kind, value) = match result {
        Ok(value) => (0_u32, value),
        Err(error) => (1_u32, error.raw_os_error().unwrap_or(1) as u32),
    };
    let mut message = Vec::with_capacity(8);
    message.extend_from_slice(&kind.to_le_bytes());
    message.extend_from_slice(&value.to_le_bytes());
    message
}

fn broker_handles_message(result: std::io::Result<[u64; 2]>) -> Vec<u8> {
    let mut message = Vec::with_capacity(24);
    match result {
        Ok(handles) => {
            message.extend_from_slice(&0_u32.to_le_bytes());
            message.extend_from_slice(&0_u32.to_le_bytes());
            message.extend_from_slice(&handles[0].to_le_bytes());
            message.extend_from_slice(&handles[1].to_le_bytes());
        }
        Err(error) => {
            message.extend_from_slice(&1_u32.to_le_bytes());
            message.extend_from_slice(&(error.raw_os_error().unwrap_or(1) as u32).to_le_bytes());
            message.extend_from_slice(&0_u64.to_le_bytes());
            message.extend_from_slice(&0_u64.to_le_bytes());
        }
    }
    message
}

fn clone_broker_main(
    request: Descriptor,
    response: Descriptor,
    mut process: OwnedHandle,
    mut job: Option<OwnedHandle>,
) {
    loop {
        let length = match broker_read_exact(&request, 4) {
            Ok(length) => u32::from_le_bytes(length.try_into().unwrap()) as usize,
            Err(_) => return,
        };
        if !(4..=64 * 1024 * 1024).contains(&length) {
            return;
        }
        let payload = match broker_read_exact(&request, length) {
            Ok(payload) => payload,
            Err(_) => return,
        };
        let operation = u32::from_le_bytes(payload[..4].try_into().unwrap());
        let body = &payload[4..];
        let message = match operation {
            BROKER_SPAWN => {
                let result = decode_spawn_request(body).and_then(|request| {
                    let input = duplicate_handle_from_process(&process, request.handles[0], true)?;
                    let output = duplicate_handle_from_process(&process, request.handles[1], true)?;
                    let error = duplicate_handle_from_process(&process, request.handles[2], true)?;
                    let handles = [
                        input.as_ref().map_or(std::ptr::null_mut(), |handle| {
                            handle.as_raw_handle() as HANDLE
                        }),
                        output.as_ref().map_or(std::ptr::null_mut(), |handle| {
                            handle.as_raw_handle() as HANDLE
                        }),
                        error.as_ref().map_or(std::ptr::null_mut(), |handle| {
                            handle.as_raw_handle() as HANDLE
                        }),
                    ];
                    spawn_program_here(
                        &request.path,
                        &request.argv,
                        &request.environment,
                        handles,
                        job.as_ref().map(|job| job.as_raw_handle() as HANDLE),
                    )
                });
                broker_result_message(result)
            }
            BROKER_PIPE => broker_handles_message(pipe_handles_for_process(&process)),
            BROKER_CHANNEL => {
                let result = (|| {
                    let (nested_request, client_request) = direct_pipe()?;
                    let (client_response, nested_response) = direct_pipe()?;
                    set_descriptor_inherit(&nested_request, false)?;
                    set_descriptor_inherit(&nested_response, false)?;
                    let remote_request = duplicate_handle_into_process(&process, &client_request)?;
                    let remote_response =
                        match duplicate_handle_into_process(&process, &client_response) {
                            Ok(handle) => handle,
                            Err(error) => {
                                close_handle_in_process(&process, remote_request);
                                return Err(error);
                            }
                        };
                    let nested_process =
                        duplicate_owned_handle(process.as_raw_handle() as HANDLE, false)?;
                    let nested_job = job
                        .as_ref()
                        .map(|job| duplicate_owned_handle(job.as_raw_handle() as HANDLE, false))
                        .transpose()?;
                    if let Err(error) = std::thread::Builder::new()
                        .name("nsh-spawn-broker".into())
                        .spawn(move || {
                            clone_broker_main(
                                nested_request,
                                nested_response,
                                nested_process,
                                nested_job,
                            )
                        })
                    {
                        close_handle_in_process(&process, remote_request);
                        close_handle_in_process(&process, remote_response);
                        return Err(error);
                    }
                    Ok([remote_request, remote_response])
                })();
                broker_handles_message(result)
            }
            BROKER_REGISTER => {
                let result = (|| {
                    let mut cursor = RequestCursor { remaining: body };
                    let child_process =
                        duplicate_handle_from_process(&process, cursor.u64()?, false)?
                            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidData))?;
                    let child_job = duplicate_handle_from_process(&process, cursor.u64()?, false)?;
                    if !cursor.remaining.is_empty() {
                        return Err(std::io::Error::from(std::io::ErrorKind::InvalidData));
                    }
                    process = child_process;
                    if child_job.is_some() {
                        job = child_job;
                    }
                    Ok(0)
                })();
                broker_result_message(result)
            }
            _ => broker_result_message(Err(std::io::Error::from(std::io::ErrorKind::InvalidInput))),
        };
        if broker_write_all(&response, &message).is_err() {
            return;
        }
    }
}

// Broker I/O deliberately avoids making temporary inheritable duplicates.
// Another shell child may be cloned while this thread is blocked on a pipe.
fn broker_read_exact(fd: &impl AsDescriptor, length: usize) -> std::io::Result<Vec<u8>> {
    let mut output = vec![0; length];
    let mut offset = 0;
    while offset < output.len() {
        let chunk = (output.len() - offset).min(u32::MAX as usize) as u32;
        let mut read = 0;
        // SAFETY: the descriptor remains borrowed for the call and the
        // remaining output slice is writable for `chunk` bytes.
        if unsafe {
            ReadFile(
                raw_handle(fd),
                output[offset..].as_mut_ptr(),
                chunk,
                &mut read,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        if read == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
        }
        offset += read as usize;
    }
    Ok(output)
}

fn broker_write_all(fd: &impl AsDescriptor, bytes: &[u8]) -> std::io::Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        let chunk = (bytes.len() - offset).min(u32::MAX as usize) as u32;
        let mut written = 0;
        // SAFETY: the descriptor remains borrowed for the call and the input
        // slice is readable for `chunk` bytes.
        if unsafe {
            WriteFile(
                raw_handle(fd),
                bytes[offset..].as_ptr(),
                chunk,
                &mut written,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        if written == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::WriteZero));
        }
        offset += written as usize;
    }
    Ok(())
}

fn append_windows_argument(output: &mut Vec<u16>, argument: &OsStr) -> std::io::Result<()> {
    let argument: Vec<u16> = argument.encode_wide().collect();
    if argument.contains(&0) {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    let quoted = argument.is_empty()
        || argument
            .iter()
            .any(|value| matches!(*value, 0x09 | 0x20 | 0x22));
    if !quoted {
        output.extend_from_slice(&argument);
        return Ok(());
    }
    output.push(u16::from(b'"'));
    let mut backslashes = 0_usize;
    for value in argument {
        if value == u16::from(b'\\') {
            backslashes += 1;
            continue;
        }
        if value == u16::from(b'"') {
            output.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes * 2 + 1));
            output.push(value);
        } else {
            output.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes));
            output.push(value);
        }
        backslashes = 0;
    }
    output.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes * 2));
    output.push(u16::from(b'"'));
    Ok(())
}

fn windows_environment_block(environment: &[(OsString, OsString)]) -> std::io::Result<Vec<u16>> {
    let mut entries: Vec<Vec<u16>> = environment
        .iter()
        .map(|(name, value)| {
            let mut entry: Vec<u16> = name.encode_wide().collect();
            entry.push(u16::from(b'='));
            entry.extend(value.encode_wide());
            if entry.contains(&0) {
                Err(std::io::Error::from(std::io::ErrorKind::InvalidInput))
            } else {
                Ok(entry)
            }
        })
        .collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|entry| {
        entry
            .iter()
            .map(|value| char::from_u32(u32::from(*value)).unwrap_or('\u{fffd}'))
            .flat_map(char::to_lowercase)
            .collect::<String>()
    });
    let mut block = Vec::new();
    for entry in entries {
        block.extend(entry);
        block.push(0);
    }
    block.push(0);
    if block.len() == 1 {
        block.push(0);
    }
    Ok(block)
}

pub fn is_exec_format_error(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(ERROR_BAD_EXE_FORMAT as i32)
}

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

fn materialized_standard_handles() -> [HANDLE; 3] {
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

pub fn read_path(path: &Path) -> std::io::Result<Vec<u8>> {
    std::fs::read(path)
}

pub fn remove_file(path: &Path) -> std::io::Result<()> {
    std::fs::remove_file(path)
}

pub fn run_editor(editor: &OsStr, path: &Path) -> std::io::Result<std::process::ExitStatus> {
    std::process::Command::new(editor).arg(path).status()
}

pub fn environment_text(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

pub fn pipe() -> std::io::Result<(Descriptor, Descriptor)> {
    if let Some(result) = broker_handle_pair(BROKER_PIPE) {
        return result;
    }
    direct_pipe()
}

fn direct_pipe() -> std::io::Result<(Descriptor, Descriptor)> {
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

pub fn exit_immediately(status: i32) -> ! {
    // SAFETY: this is the Windows process-terminating primitive and never
    // returns into Rust with destructors skipped.
    unsafe { ExitProcess(status as u32) }
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

pub fn is_pseudoterminal_end(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::UnexpectedEof
    )
}

pub fn is_bad_descriptor_error(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(ERROR_INVALID_HANDLE as i32)
}

pub fn interrupt_signal() -> Signal {
    Signal::new(2).expect("SIGINT is positive")
}

pub fn quit_signal() -> Signal {
    Signal::new(3).expect("SIGQUIT is positive")
}

pub fn termination_signal() -> Signal {
    Signal::new(15).expect("SIGTERM is positive")
}

pub fn kill_signal() -> Signal {
    Signal::new(9).expect("SIGKILL is positive")
}

pub fn child_signal() -> Signal {
    Signal::new(17).expect("SIGCHLD is positive")
}

pub fn pipe_signal() -> Signal {
    Signal::new(13).expect("SIGPIPE is positive")
}

pub fn hangup_signal() -> Signal {
    Signal::new(1).expect("SIGHUP is positive")
}

pub fn terminal_stop_signal() -> Signal {
    Signal::new(20).expect("SIGTSTP is positive")
}

pub fn terminal_input_signal() -> Signal {
    Signal::new(21).expect("SIGTTIN is positive")
}

pub fn terminal_output_signal() -> Signal {
    Signal::new(22).expect("SIGTTOU is positive")
}

pub fn continue_signal() -> Signal {
    Signal::new(18).expect("SIGCONT is positive")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignalAction {
    Default,
    Ignore,
    Catch,
}

static SIGNAL_ACTIONS: [AtomicUsize; SIGNAL_COUNT] = [const { AtomicUsize::new(0) }; SIGNAL_COUNT];
static SIGNAL_HANDLERS: [AtomicUsize; SIGNAL_COUNT] = [const { AtomicUsize::new(0) }; SIGNAL_COUNT];
static PENDING_SIGNALS: [AtomicUsize; SIGNAL_COUNT] = [const { AtomicUsize::new(0) }; SIGNAL_COUNT];
static SIGNAL_BLOCK_DEPTH: AtomicUsize = AtomicUsize::new(0);
static CONSOLE_HANDLER_INSTALLED: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn console_control_handler(event: u32) -> i32 {
    let signal = match event {
        CTRL_C_EVENT => interrupt_signal(),
        CTRL_BREAK_EVENT => quit_signal(),
        _ => return 0,
    };
    dispatch_signal(signal) as i32
}

fn dispatch_signal(signal: Signal) -> bool {
    let index = signal.number() as usize;
    let Some(action) = SIGNAL_ACTIONS.get(index) else {
        return false;
    };
    if SIGNAL_BLOCK_DEPTH.load(AtomicOrdering::Acquire) != 0 {
        PENDING_SIGNALS[index].store(1, AtomicOrdering::Release);
        return true;
    }
    match action.load(AtomicOrdering::Relaxed) {
        1 => true,
        2 => {
            let address = SIGNAL_HANDLERS[index].load(AtomicOrdering::Relaxed);
            if address != 0 {
                // SAFETY: only `install_signal_action` stores addresses here,
                // and its parameter has exactly this ABI and static lifetime.
                let handler: fn(Signal) = unsafe { std::mem::transmute(address) };
                handler(signal);
            }
            true
        }
        _ => false,
    }
}

fn ensure_console_handler() -> std::io::Result<()> {
    if CONSOLE_HANDLER_INSTALLED
        .compare_exchange(0, 1, AtomicOrdering::AcqRel, AtomicOrdering::Acquire)
        .is_ok()
    {
        // SAFETY: the callback has the required system ABI and static
        // lifetime. Windows retains only that function pointer.
        if unsafe { SetConsoleCtrlHandler(Some(console_control_handler), 1) } == 0 {
            CONSOLE_HANDLER_INSTALLED.store(0, AtomicOrdering::Release);
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

pub fn signal_action(signal: Signal) -> std::io::Result<SignalAction> {
    let index = usize::try_from(signal.number())
        .ok()
        .filter(|index| *index < SIGNAL_COUNT)
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    Ok(match SIGNAL_ACTIONS[index].load(AtomicOrdering::Relaxed) {
        0 => SignalAction::Default,
        1 => SignalAction::Ignore,
        _ => SignalAction::Catch,
    })
}

pub fn install_signal_action(
    signal: Signal,
    action: SignalAction,
    handler: fn(Signal),
) -> std::io::Result<()> {
    let index = usize::try_from(signal.number())
        .ok()
        .filter(|index| *index > 0 && *index < SIGNAL_COUNT)
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    ensure_console_handler()?;
    SIGNAL_HANDLERS[index].store(handler as usize, AtomicOrdering::Relaxed);
    SIGNAL_ACTIONS[index].store(
        match action {
            SignalAction::Default => 0,
            SignalAction::Ignore => 1,
            SignalAction::Catch => 2,
        },
        AtomicOrdering::Release,
    );
    Ok(())
}

fn ignored_signal_placeholder(_: Signal) {}

pub fn ignore_signal(signal: Signal) -> std::io::Result<()> {
    install_signal_action(signal, SignalAction::Ignore, ignored_signal_placeholder)
}

pub struct BlockedSignals;

impl BlockedSignals {
    pub fn all() -> std::io::Result<Self> {
        SIGNAL_BLOCK_DEPTH.fetch_add(1, AtomicOrdering::AcqRel);
        Ok(Self)
    }

    pub fn suspend(&self) -> std::io::Result<()> {
        std::thread::sleep(Duration::from_millis(10));
        Ok(())
    }
}

impl Drop for BlockedSignals {
    fn drop(&mut self) {
        if SIGNAL_BLOCK_DEPTH.fetch_sub(1, AtomicOrdering::AcqRel) == 1 {
            for signal in 1..SIGNAL_COUNT {
                if PENDING_SIGNALS[signal].swap(0, AtomicOrdering::AcqRel) != 0 {
                    let signal =
                        Signal::new(signal as i32).expect("pending signal indices start at one");
                    let _ = dispatch_signal(signal);
                }
            }
        }
    }
}

pub fn signal_is_blocked(_signal: Signal) -> std::io::Result<bool> {
    Ok(SIGNAL_BLOCK_DEPTH.load(AtomicOrdering::Acquire) != 0)
}

pub fn unblock_all_signals() -> std::io::Result<()> {
    SIGNAL_BLOCK_DEPTH.store(0, AtomicOrdering::Release);
    Ok(())
}

const SIGNAL_EXIT_BASE: u32 = 0xe000_0000;

pub fn send_signal(target: ProcessTarget, request: SignalRequest) -> std::io::Result<()> {
    match target {
        ProcessTarget::Process(process) => send_signal_to_process(process, request),
        ProcessTarget::CurrentProcessGroup => match request {
            SignalRequest::Deliver(signal) => raise_signal(signal),
            SignalRequest::Probe => Err(std::io::Error::from(std::io::ErrorKind::InvalidInput)),
        },
        ProcessTarget::ProcessGroup(group) => send_signal_to_process(
            ProcessId::new(group.get()).expect("a process group leader is a process identity"),
            request,
        ),
        ProcessTarget::AllProcesses => Err(std::io::Error::from(std::io::ErrorKind::InvalidInput)),
    }
}

fn send_signal_to_process(pid: ProcessId, request: SignalRequest) -> std::io::Result<()> {
    let SignalRequest::Deliver(signal) = request else {
        return process_exists(pid);
    };
    let _clone_guard = PROCESS_CLONE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let children = CHILDREN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(child) = children.get(&pid) {
        let code = SIGNAL_EXIT_BASE | signal.number() as u32;
        // SAFETY: the handles are owned by the record for the whole call.
        let succeeded = unsafe {
            child.job.as_ref().map_or_else(
                || TerminateProcess(child.process.as_raw_handle() as HANDLE, code),
                |job| TerminateJobObject(job.as_raw_handle() as HANDLE, code),
            )
        };
        return if succeeded == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        };
    }
    drop(children);
    // SAFETY: access rights are bounded to querying/termination and the PID
    // is a validated positive scalar.
    let raw = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE,
            0,
            pid.get(),
        )
    };
    if raw.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: OpenProcess returned one owned handle.
    let process = unsafe { OwnedHandle::from_raw_handle(raw.cast()) };
    let code = SIGNAL_EXIT_BASE | signal.number() as u32;
    // SAFETY: `process` remains live for this call.
    if unsafe { TerminateProcess(process.as_raw_handle() as HANDLE, code) } == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn process_exists(pid: ProcessId) -> std::io::Result<()> {
    // SAFETY: the requested right is query-only and PID is positive.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid.get()) };
    if raw.is_null() {
        Err(std::io::Error::last_os_error())
    } else {
        // SAFETY: this is the one owned handle returned above.
        unsafe { CloseHandle(raw) };
        Ok(())
    }
}

pub fn raise_signal(signal: Signal) -> std::io::Result<()> {
    if signal.number() as usize >= SIGNAL_COUNT {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    if dispatch_signal(signal) {
        Ok(())
    } else {
        exit_immediately(128 + signal.number())
    }
}

pub fn send_continue_to_process_group(_process_group: ProcessGroupId) -> std::io::Result<()> {
    Ok(())
}

pub fn terminate_with_interrupt() -> ! {
    exit_immediately(128 + interrupt_signal().number())
}

pub fn configure_here_document_writer_signals() {
    for signal in [
        interrupt_signal(),
        quit_signal(),
        hangup_signal(),
        terminal_stop_signal(),
    ] {
        let _ = ignore_signal(signal);
    }
}

#[derive(Clone, Copy, Debug)]
pub enum LimitResource {
    Cpu,
    FileSize,
    Data,
    Stack,
    Core,
    ResidentSet,
    LockedMemory,
    Processes,
    OpenFiles,
    AddressSpace,
    Locks,
    RealtimePriority,
}

#[derive(Clone, Copy, Debug)]
pub struct ResourceLimit {
    pub current: Option<u64>,
    pub maximum: Option<u64>,
}

pub fn resource_limit(resource: LimitResource) -> std::io::Result<ResourceLimit> {
    match resource {
        LimitResource::OpenFiles => Ok(ResourceLimit {
            current: Some(16_384),
            maximum: Some(16_384),
        }),
        _ => Err(std::io::Error::from(std::io::ErrorKind::Unsupported)),
    }
}

pub fn set_resource_limit(_resource: LimitResource, _limit: ResourceLimit) -> std::io::Result<()> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

static CREATION_MASK: AtomicU32 = AtomicU32::new(0o022);

pub fn replace_creation_mask(mask: u32) -> u32 {
    CREATION_MASK.swap(mask & 0o777, AtomicOrdering::Relaxed)
}

pub fn creation_mask() -> u32 {
    CREATION_MASK.load(AtomicOrdering::Relaxed)
}

#[derive(Clone, Copy, Debug)]
pub struct ProcessTimes {
    pub user: f64,
    pub system: f64,
    pub children_user: f64,
    pub children_system: f64,
}

static CHILD_USER_TICKS: AtomicU64 = AtomicU64::new(0);
static CHILD_SYSTEM_TICKS: AtomicU64 = AtomicU64::new(0);

fn filetime_ticks(value: FILETIME) -> u64 {
    (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
}

pub fn process_times() -> ProcessTimes {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: the current-process pseudo-handle is valid and each output
    // record is writable.
    let succeeded = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    let scale = 10_000_000_f64;
    ProcessTimes {
        user: if succeeded == 0 {
            0.0
        } else {
            filetime_ticks(user) as f64 / scale
        },
        system: if succeeded == 0 {
            0.0
        } else {
            filetime_ticks(kernel) as f64 / scale
        },
        children_user: CHILD_USER_TICKS.load(AtomicOrdering::Relaxed) as f64 / scale,
        children_system: CHILD_SYSTEM_TICKS.load(AtomicOrdering::Relaxed) as f64 / scale,
    }
}

struct ChildRecord {
    process: OwnedHandle,
    job: Option<OwnedHandle>,
    owner: std::thread::ThreadId,
}

static CHILDREN: LazyLock<Mutex<HashMap<ProcessId, ChildRecord>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static PROCESS_CLONE_LOCK: Mutex<()> = Mutex::new(());

fn set_descriptor_inherit(fd: &impl AsDescriptor, inherit: bool) -> std::io::Result<()> {
    // SAFETY: the descriptor is live and the mask/value pair only changes its
    // inheritance flag.
    if unsafe {
        SetHandleInformation(
            raw_handle(fd),
            HANDLE_FLAG_INHERIT,
            if inherit { HANDLE_FLAG_INHERIT } else { 0 },
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn duplicate_owned_handle(raw: HANDLE, inherit: bool) -> std::io::Result<OwnedHandle> {
    let mut duplicate = std::ptr::null_mut();
    // SAFETY: `raw` is live in the current process and the output slot receives
    // one newly owned current-process handle.
    if unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            raw,
            GetCurrentProcess(),
            &mut duplicate,
            0,
            i32::from(inherit),
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        // SAFETY: DuplicateHandle returned one newly owned handle.
        Ok(unsafe { OwnedHandle::from_raw_handle(duplicate.cast()) })
    }
}

fn child_job(process: HANDLE) -> Option<OwnedHandle> {
    // SAFETY: null attributes and name request one private Job Object handle.
    let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if raw.is_null() {
        return None;
    }
    // SAFETY: CreateJobObjectW returned one newly owned handle.
    let job = unsafe { OwnedHandle::from_raw_handle(raw.cast()) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: both handles are live and `limits` is a correctly sized record
    // for the selected information class.
    let configured = unsafe {
        SetInformationJobObject(
            job.as_raw_handle() as HANDLE,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if configured == 0 {
        return None;
    }
    // SAFETY: both handles remain live through assignment. Nested jobs are
    // supported on current Windows; an embedding host may still forbid this,
    // in which case process-handle termination remains available.
    if unsafe { AssignProcessToJobObject(job.as_raw_handle() as HANDLE, process) } == 0 {
        None
    } else {
        Some(job)
    }
}

/// Clone the current process using Windows' native copy-on-write user-process
/// clone. This is the Windows analogue needed by a shell's fork-then-exec
/// control flow; the child must promptly run shell work or create a new image.
pub fn fork_process() -> std::io::Result<ForkResult> {
    use ntapi::ntrtl::{
        RTL_CLONE_PROCESS_FLAGS_CREATE_SUSPENDED, RTL_CLONE_PROCESS_FLAGS_INHERIT_HANDLES,
        RTL_USER_PROCESS_INFORMATION, RtlCloneUserProcess, RtlNtStatusToDosError,
    };

    // A native clone cannot safely make the Win32 directory queries used to
    // initialize the fallback PATH. Finish that one-time initialization while
    // this is still the ordinary parent process; the child receives the
    // completed owned string through copy-on-write memory.
    LazyLock::force(&DEFAULT_SEARCH_PATH);

    let (server_channel, request_write, response_read) =
        if let Some(channel) = broker_handle_pair(BROKER_CHANNEL) {
            let (request, response) = channel?;
            (None, request, response)
        } else {
            let (request_read, request_write) = direct_pipe()?;
            let (response_read, response_write) = direct_pipe()?;
            (
                Some((request_read, response_write)),
                request_write,
                response_read,
            )
        };
    // RtlCloneUserProcess synchronizes process-wide runtime structures. Keep
    // simultaneous shell instances in one embedding process from entering it
    // concurrently; the copied child unlocks its private copy on return.
    let _clone_guard = PROCESS_CLONE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    // SAFETY: this plain C record is initialized below before the native call.
    let mut information: RTL_USER_PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    information.Length = std::mem::size_of::<RTL_USER_PROCESS_INFORMATION>() as u32;
    // SAFETY: every optional pointer is null, `information` is writable and
    // correctly sized, and the result is immediately reduced to owned handles
    // or the child/parent discriminator.
    let status = unsafe {
        RtlCloneUserProcess(
            // Let ntdll synchronize its loader, heap, TLS/FLS, and thread-pool
            // state around the clone. NO_SYNCHRONIZE is only suitable for a
            // fork server that can prove none of that state will be touched.
            RTL_CLONE_PROCESS_FLAGS_CREATE_SUSPENDED | RTL_CLONE_PROCESS_FLAGS_INHERIT_HANDLES,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut information,
        )
    };
    if status == windows_sys::Win32::Foundation::STATUS_PROCESS_CLONED {
        drop(server_channel);
        CHILDREN
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        CLONE_BROKER.with(|broker| {
            *broker.borrow_mut() = Some(CloneBrokerChild {
                request: request_write,
                response: response_read,
            });
        });
        // RtlCloneUserProcess copies the address space but does not register
        // an independent console connection with CSRSS. Reattach the cloned
        // process before any Win32 image creation; logical descriptors are
        // materialized again afterwards and therefore remain authoritative.
        // SAFETY: both calls affect only the calling cloned process. Failure
        // is expected for a shell that was launched without a console.
        unsafe {
            FreeConsole();
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
        reset_coverage_counters();
        return Ok(ForkResult::Child);
    }
    if status < 0 {
        // SAFETY: the conversion accepts any NTSTATUS and returns the closest
        // public Win32 error code.
        let code = unsafe { RtlNtStatusToDosError(status) };
        return Err(std::io::Error::from_raw_os_error(code as i32));
    }

    let pid = u32::try_from(information.ClientId.UniqueProcess as usize)
        .ok()
        .and_then(ProcessId::new);
    if pid.is_none() || information.Process.is_null() {
        return Err(std::io::Error::other(
            "RtlCloneUserProcess returned no child process",
        ));
    }
    let pid = pid.expect("the child process identity was validated above");
    if information.Thread.is_null() {
        // SAFETY: the successful clone still returned one process handle.
        unsafe { CloseHandle(information.Process.cast()) };
        return Err(std::io::Error::other(
            "RtlCloneUserProcess returned no child thread",
        ));
    }
    // SAFETY: the parent receives one owned process handle on successful clone.
    let process = unsafe { OwnedHandle::from_raw_handle(information.Process.cast()) };
    let job = child_job(process.as_raw_handle() as HANDLE);
    let broker_result = if let Some((request_read, response_write)) = server_channel {
        drop(request_write);
        drop(response_read);
        (|| {
            // These endpoints were inheritable for the native clone only.
            set_descriptor_inherit(&request_read, false)?;
            set_descriptor_inherit(&response_write, false)?;
            let broker_process = duplicate_owned_handle(process.as_raw_handle() as HANDLE, false)?;
            let broker_job = job
                .as_ref()
                .map(|job| duplicate_owned_handle(job.as_raw_handle() as HANDLE, false))
                .transpose()?;
            std::thread::Builder::new()
                .name("nsh-spawn-broker".into())
                .spawn(move || {
                    clone_broker_main(request_read, response_write, broker_process, broker_job)
                })?;
            Ok(())
        })()
    } else {
        let result = register_clone_broker(
            &request_write,
            &response_read,
            process.as_raw_handle() as HANDLE,
            job.as_ref().map(|job| job.as_raw_handle() as HANDLE),
        );
        drop(request_write);
        drop(response_read);
        result
    };
    if let Err(error) = broker_result {
        // SAFETY: the clone is still suspended. `process` closes separately.
        unsafe {
            TerminateProcess(process.as_raw_handle() as HANDLE, 1);
            CloseHandle(information.Thread.cast());
        }
        return Err(error);
    }
    // SAFETY: CREATE_SUSPENDED returned one live initial-thread handle.
    if unsafe { ResumeThread(information.Thread.cast()) } == u32::MAX {
        let error = std::io::Error::last_os_error();
        // SAFETY: neither returned handle has been transferred yet.
        unsafe {
            CloseHandle(information.Thread.cast());
        }
        return Err(error);
    }
    // SAFETY: the resumed thread remains owned by its process.
    unsafe { CloseHandle(information.Thread.cast()) };
    CHILDREN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            pid,
            ChildRecord {
                process,
                job,
                owner: std::thread::current().id(),
            },
        );
    Ok(ForkResult::Parent(pid))
}

pub fn parent_process_id() -> Option<ProcessId> {
    use ntapi::ntpsapi::{
        NtQueryInformationProcess, PROCESS_BASIC_INFORMATION, ProcessBasicInformation,
    };

    // SAFETY: the record is output-only for the native query.
    let mut information: PROCESS_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: the current-process pseudo-handle is always valid, the class
    // matches the record type, and the record is writable for its full size.
    let status = unsafe {
        NtQueryInformationProcess(
            GetCurrentProcess().cast(),
            ProcessBasicInformation,
            (&mut information as *mut PROCESS_BASIC_INFORMATION).cast(),
            std::mem::size_of::<PROCESS_BASIC_INFORMATION>() as u32,
            std::ptr::null_mut(),
        )
    };
    if status < 0 {
        None
    } else {
        u32::try_from(information.InheritedFromUniqueProcessId as usize)
            .ok()
            .and_then(ProcessId::new)
    }
}

pub fn current_process_id() -> ProcessId {
    // SAFETY: this function takes no arguments and cannot fail.
    ProcessId::new(unsafe { GetCurrentProcessId() })
        .expect("the current process has a positive identity")
}

static FOREGROUND_PROCESS_GROUP: AtomicU32 = AtomicU32::new(0);

pub fn current_process_group() -> ProcessGroupState {
    ProcessGroupState::Visible(ProcessGroupId::from_leader(current_process_id()))
}

pub fn foreground_process_group(_fd: &impl AsDescriptor) -> std::io::Result<ProcessGroupState> {
    let group = FOREGROUND_PROCESS_GROUP.load(AtomicOrdering::Relaxed);
    Ok(if let Some(group) = ProcessGroupId::new(group) {
        ProcessGroupState::Visible(group)
    } else {
        current_process_group()
    })
}

pub fn set_process_group(_: ProcessSelector, _: ProcessGroupState) -> std::io::Result<()> {
    // Every cloned child already owns a Job Object. Its PID is the public
    // process-group key used by the shell's job table.
    Ok(())
}

pub fn set_foreground_process_group(
    _fd: &impl AsDescriptor,
    group: ProcessGroupState,
) -> std::io::Result<()> {
    FOREGROUND_PROCESS_GROUP.store(group.nonnegative_platform_value(), AtomicOrdering::Relaxed);
    Ok(())
}

fn decode_child_status(exit_code: u32) -> ChildStatus {
    if exit_code & 0xff00_0000 == SIGNAL_EXIT_BASE {
        ChildStatus::Signaled {
            signal: Signal::new((exit_code & 0x7f) as i32)
                .expect("synthetic signal exit codes contain a positive signal"),
            core_dumped: false,
        }
    } else {
        ChildStatus::Exited((exit_code & 0xff) as u8)
    }
}

fn accumulate_child_times(process: HANDLE) {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: the process handle remains owned by the caller and all output
    // records are writable.
    if unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) } != 0 {
        CHILD_USER_TICKS.fetch_add(filetime_ticks(user), AtomicOrdering::Relaxed);
        CHILD_SYSTEM_TICKS.fetch_add(filetime_ticks(kernel), AtomicOrdering::Relaxed);
    }
}

fn reap_ready_child() -> std::io::Result<Option<(ProcessId, ChildStatus)>> {
    let owner = std::thread::current().id();
    let _clone_guard = PROCESS_CLONE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let ready = {
        let children = CHILDREN
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        children.iter().find_map(|(&pid, child)| {
            if child.owner != owner {
                return None;
            }
            // SAFETY: each handle is owned by the locked record for this call.
            (unsafe { WaitForSingleObject(child.process.as_raw_handle() as HANDLE, 0) }
                == WAIT_OBJECT_0)
                .then_some(pid)
        })
    };
    let Some(pid) = ready else {
        return Ok(None);
    };
    let child = CHILDREN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&pid)
        .expect("a ready child remains registered until this reaper removes it");
    let mut code = 0;
    // SAFETY: the record owns the process handle and the output scalar is
    // writable. A signalled process has a final exit code.
    if unsafe { GetExitCodeProcess(child.process.as_raw_handle() as HANDLE, &mut code) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    accumulate_child_times(child.process.as_raw_handle() as HANDLE);
    Ok(Some((pid, decode_child_status(code))))
}

pub fn wait_for_any_child(
    nonblocking: bool,
    _report_stopped: bool,
) -> std::io::Result<Option<(ProcessId, ChildStatus)>> {
    let owner = std::thread::current().id();
    loop {
        if let Some(child) = reap_ready_child()? {
            return Ok(Some(child));
        }
        let has_children = {
            let _clone_guard = PROCESS_CLONE_LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            CHILDREN
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .values()
                .any(|child| child.owner == owner)
        };
        if !has_children {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "there are no child processes",
            ));
        }
        if nonblocking {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn run_in_child(body: impl FnOnce()) -> std::io::Result<i32> {
    match fork_process()? {
        ForkResult::Child => {
            body();
            exit_immediately(0);
        }
        ForkResult::Parent(pid) => loop {
            let Some((reaped, status)) = wait_for_any_child(false, false)? else {
                continue;
            };
            if reaped == pid {
                return Ok(match status {
                    ChildStatus::Exited(code) => i32::from(code),
                    ChildStatus::Signaled { signal, .. } => 128 + signal.number(),
                    ChildStatus::Stopped(signal) => 128 + signal.number(),
                    ChildStatus::Continued => 0,
                });
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
