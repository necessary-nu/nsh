//! What this host calls a path, and what it will tell you about one.
//!
//! Two things a shell cannot keep apart. The spellings first: the byte
//! between path components, the byte between `PATH` entries, the name of
//! the null device and of the controlling terminal, whether a glob
//! metacharacter can be part of a filename. Then the questions asked of
//! a name once it is spelled -- `test`'s file predicates, the metadata
//! behind them, `cd`'s logical form, reading a directory, and opening or
//! removing a file by name.
//!
//! `named_user_home` is here for the same reason: `~user` is a question
//! about where a name points, and the passwd database is this host's
//! answer to it. `login_shell` asks the same database the other question
//! a shell has of it: which shell this account is meant to run.

use std::ffi::{CStr, CString, OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use rustix::fs::{AtFlags, CWD, accessat};

/// One field of one `passwd` record, copied out before the buffer
/// backing it is dropped.
///
/// `lookup` performs the `getpw*_r` call and `field` names the member to
/// take. The two questions this host is asked of the database differ
/// only in the key; the growing buffer, the `ERANGE` retry, the two null
/// checks and the copy are the whole of the rest, so they are written
/// once rather than twice.
///
/// A `_r` lookup writes strings *into the caller's buffer*, so nothing
/// borrowed from `record` may outlive `storage`. That is why the return
/// is owned and why the copy happens here and not at either call site.
fn passwd_field(
    lookup: impl Fn(*mut libc::passwd, *mut u8, usize, *mut *mut libc::passwd) -> i32,
    field: impl Fn(&libc::passwd) -> *const libc::c_char,
) -> Option<OsString> {
    let mut size = 1024_usize;
    loop {
        let mut record = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        let mut storage = vec![0_u8; size];
        let error = lookup(
            record.as_mut_ptr(),
            storage.as_mut_ptr(),
            storage.len(),
            &mut result,
        );
        if error == libc::ERANGE {
            size = size.checked_mul(2)?;
            continue;
        }
        if error != 0 || result.is_null() {
            return None;
        }
        // SAFETY: a zero return with a non-null result initialized `record`.
        let record = unsafe { record.assume_init() };
        let value = field(&record);
        if value.is_null() {
            return None;
        }
        // SAFETY: a passwd field is a terminated string inside the live
        // `storage`, and it is copied before `storage` is dropped.
        return Some(OsString::from_vec(
            unsafe { CStr::from_ptr(value) }.to_bytes().to_vec(),
        ));
    }
}

/// Look up the home directory named by a `~user` expansion.
///
/// `std::env::home_dir` answers only for the current user, while this lookup
/// deliberately handles a different named account. Bare `~` never reaches
/// this function: the shell expands it from that shell instance's `HOME`.
pub fn named_user_home(name: &OsStr) -> Option<PathBuf> {
    let name = CString::new(name.as_bytes()).ok()?;
    passwd_field(
        |record, storage, length, result| {
            // SAFETY: the name is terminated, and `record`, `storage` and
            // `result` are writable for the length passed with them.
            unsafe { libc::getpwnam_r(name.as_ptr(), record, storage.cast(), length, result) }
        },
        |record| record.pw_dir.cast_const(),
    )
    .map(PathBuf::from)
}

/// The shell this account is meant to run, as the passwd database has it.
///
/// It is not this program: `$SHELL` answers "which shell does this user
/// use", which a script hands to `$SHELL -c` and an editor spawns, and
/// the reference reads it out of the login entry rather than out of
/// `argv[0]`. An account whose entry names no shell has none to report.
pub fn login_shell() -> Option<OsString> {
    passwd_field(
        |record, storage, length, result| {
            // SAFETY: `record`, `storage` and `result` are writable for
            // the length passed with them, and the uid is a plain value.
            unsafe { libc::getpwuid_r(libc::getuid(), record, storage.cast(), length, result) }
        },
        |record| record.pw_shell.cast_const(),
    )
    .filter(|shell| !shell.is_empty())
}

pub fn default_search_path() -> OsString {
    OsString::from("/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
}

pub fn fallback_shell() -> &'static OsStr {
    OsStr::new("/bin/sh")
}

pub fn controlling_terminal_path() -> &'static Path {
    Path::new("/dev/tty")
}

pub fn null_device_path() -> &'static Path {
    Path::new("/dev/null")
}

pub const fn search_path_separator() -> u8 {
    b':'
}

pub const fn shell_directory_separator() -> u8 {
    b'/'
}

pub fn resolve_command_path(path: &Path, _environment: &[(OsString, OsString)]) -> PathBuf {
    path.to_path_buf()
}

pub const fn supports_glob_metacharacters_in_filenames() -> bool {
    true
}

pub fn shell_path_has_separator(path: &[u8]) -> bool {
    path.contains(&b'/')
}

pub fn shell_path_is_absolute(path: &[u8]) -> bool {
    path.first() == Some(&b'/')
}

pub fn shell_path_last_separator(path: &[u8]) -> Option<usize> {
    path.iter().rposition(|byte| *byte == b'/')
}

/// The access questions supported by shell `test` and command lookup.
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessMode {
    READ_OK,
    WRITE_OK,
    EXEC_OK,
}

impl AccessMode {
    fn native(self) -> rustix::fs::Access {
        match self {
            Self::READ_OK => rustix::fs::Access::READ_OK,
            Self::WRITE_OK => rustix::fs::Access::WRITE_OK,
            Self::EXEC_OK => rustix::fs::Access::EXEC_OK,
        }
    }
}

/// Test access using effective rather than real credentials.
pub fn effective_access(path: &Path, access: AccessMode) -> bool {
    accessat(CWD, path, access.native(), AtFlags::EACCESS).is_ok()
}

/// The portable file-kind information used by shell predicates.
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

/// A platform-owned metadata snapshot containing only shell semantics.
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

/// Read path metadata without exposing the host metadata extensions.
pub fn path_metadata(path: &Path, follow_links: bool) -> std::io::Result<FileMetadata> {
    use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};

    let metadata = if follow_links {
        std::fs::metadata(path)
    } else {
        std::fs::symlink_metadata(path)
    }?;
    let kind = match metadata.file_type() {
        kind if kind.is_file() => FileKind::Regular,
        kind if kind.is_dir() => FileKind::Directory,
        kind if kind.is_char_device() => FileKind::CharacterDevice,
        kind if kind.is_block_device() => FileKind::BlockDevice,
        kind if kind.is_fifo() => FileKind::Fifo,
        kind if kind.is_socket() => FileKind::Socket,
        kind if kind.is_symlink() => FileKind::Symlink,
        _ => FileKind::Other,
    };
    Ok(FileMetadata {
        kind,
        mode: metadata.mode(),
        size: metadata.size(),
        user: metadata.uid(),
        group: metadata.gid(),
        device: metadata.dev(),
        inode: metadata.ino(),
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
    })
}

pub fn path_exists(path: &Path) -> bool {
    path_metadata(path, true).is_ok()
}

pub fn path_is_file(path: &Path) -> bool {
    path_metadata(path, true).is_ok_and(|metadata| metadata.kind == FileKind::Regular)
}

pub fn path_is_directory(path: &Path) -> bool {
    path_metadata(path, true).is_ok_and(|metadata| metadata.kind == FileKind::Directory)
}

pub fn path_is_same_file(left: &Path, right: &Path) -> bool {
    match (path_metadata(left, true), path_metadata(right, true)) {
        (Ok(left), Ok(right)) => left.device == right.device && left.inode == right.inode,
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

/// Construct the logical spelling used for `PWD` without resolving symbolic
/// links. The platform owns pathname roots and separators; the shell owns only
/// the `cd` policy that selects logical versus physical mode.
pub fn logical_path(current: Option<&Path>, directory: &Path) -> Option<PathBuf> {
    let dir = directory.as_os_str().as_bytes();
    let mut limit = 1_usize;
    let mut new = Vec::new();
    if !directory.is_absolute() {
        new.extend_from_slice(current?.as_os_str().as_bytes());
    }
    new.reserve(dir.len() + 2);
    if !directory.is_absolute() {
        if new.last() != Some(&b'/') {
            new.push(b'/');
        }
        if new.len() > limit && new[limit] == b'/' {
            limit += 1;
        }
    } else {
        new.push(b'/');
        if dir.get(1) == Some(&b'/') && dir.get(2) != Some(&b'/') {
            new.push(b'/');
            limit += 1;
        }
    }
    for component in dir.split(|byte| *byte == b'/') {
        if component.is_empty() || component == b"." {
            continue;
        }
        if component == b".." {
            // [spec:posix:req:builtin.cd.step8-canonical-form-dot-dot]
            if new.len() > limit && !path_is_directory(Path::new(OsStr::from_bytes(&new))) {
                return None;
            }
            while new.len() > limit {
                new.pop();
                if new.last() == Some(&b'/') {
                    break;
                }
            }
        } else {
            new.extend_from_slice(component);
            new.push(b'/');
        }
    }
    if new.len() > limit {
        new.pop();
    }
    Some(PathBuf::from(OsString::from_vec(new)))
}

/// Whether the host permits removing the directory used as a process's cwd.
pub const fn can_unlink_current_directory() -> bool {
    true
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

pub fn open_history_file(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
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
