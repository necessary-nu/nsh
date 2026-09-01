//! What Windows calls a path, and what it will tell you about one.
//!
//! The shell's spelling and the host's differ here, which is why this is
//! the longer of the two hosts' path modules. `/` separates components
//! and `;` separates search-path entries, so every native string that
//! reaches the shell is rewritten on the way through, and the fallback
//! `PATH` has to be assembled from the system and Windows directories
//! rather than named as a constant. `resolve_command_path` is the other
//! question only this host asks: a name without an extension is not a
//! program until `PATHEXT` says which extension makes it one.
//!
//! The rest is what every host is asked -- the file predicates behind
//! `test`, the metadata behind them, `cd`'s logical form, reading a
//! directory, and opening or removing a file by name -- and
//! `named_user_home`, because `~user` is a question about where a name
//! points.

use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, GetFileInformationByHandle,
};
use windows_sys::Win32::System::SystemInformation::{GetSystemDirectoryW, GetWindowsDirectoryW};

use super::*;

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

pub(super) fn with_shell_path_separators(value: OsString) -> OsString {
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

pub(super) static DEFAULT_SEARCH_PATH: LazyLock<OsString> =
    LazyLock::new(build_default_search_path);

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

pub(super) fn query_windows_directory(query: WindowsDirectoryQuery) -> Option<OsString> {
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
    let combined = match (directory.is_absolute(), current) {
        (true, _) => directory.to_path_buf(),
        (false, Some(current)) => current.join(directory),
        (false, None) if directory.has_root() => absolute_path(directory).ok()?,
        (false, None) => return None,
    };
    let mut normalized = PathBuf::new();
    for component in combined.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                // [spec:posix:req:builtin.cd.step8-canonical-form-dot-dot]
                if normalized.parent().is_some() && !path_is_directory(&normalized) {
                    return None;
                }
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

pub fn read_path(path: &Path) -> std::io::Result<Vec<u8>> {
    std::fs::read(path)
}

pub fn remove_file(path: &Path) -> std::io::Result<()> {
    std::fs::remove_file(path)
}

pub fn run_editor(editor: &OsStr, path: &Path) -> std::io::Result<std::process::ExitStatus> {
    std::process::Command::new(editor).arg(path).status()
}
