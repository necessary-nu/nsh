use std::collections::BTreeSet;
use std::collections::hash_map::RandomState;
use std::ffi::{CStr, CString, OsStr, OsString};
use std::hash::BuildHasher as _;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::OpenOptionsExt as _;
use std::os::unix::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

use rustix::fs::{AtFlags, CWD, accessat};

mod locale;
pub use locale::{Locale, LocaleCategory, LocaleDecode, LocaleDecoder};
mod descriptor;
pub use descriptor::{AsDescriptor, BorrowedDescriptor, Descriptor, move_fd_cloexec};
mod terminal;
pub use terminal::TerminalSettings;
mod editor_terminal;
pub use editor_terminal::{
    EditorTerminalAttributes, TerminalApply, TerminalControlCharacter,
    apply_editor_terminal_attributes, editor_terminal_attributes, editor_terminal_size,
    wait_for_terminal_input,
};
mod signal_names;
pub use signal_names::{SIGNAL_COUNT, SIGNAL_NAMES};

// Rust's runtime changes three pieces of inherited process state before
// `main`: it ignores SIGPIPE, installs stack-overflow handlers, and opens
// /dev/null over closed standard descriptors. A shell must present the state
// it inherited instead. The two values Rust would otherwise destroy are
// captured by this constructor before the runtime initializes.
static INHERITED_SIGPIPE: AtomicUsize = AtomicUsize::new(usize::MAX);
static CLOSED_STANDARD_FDS: AtomicUsize = AtomicUsize::new(0);

#[used]
#[cfg_attr(not(target_vendor = "apple"), unsafe(link_section = ".init_array"))]
#[cfg_attr(
    target_vendor = "apple",
    unsafe(link_section = "__DATA,__mod_init_func")
)]
static CAPTURE_PRE_RUNTIME_STATE: extern "C" fn() = capture_pre_runtime_state;

extern "C" fn capture_pre_runtime_state() {
    // SAFETY: this constructor runs single-threaded before Rust's runtime.
    // Both calls only inspect process state and write initialized locals.
    unsafe {
        let mut old: libc::sigaction = std::mem::zeroed();
        if libc::sigaction(libc::SIGPIPE, std::ptr::null(), &mut old) == 0 {
            INHERITED_SIGPIPE.store(old.sa_sigaction, AtomicOrdering::Relaxed);
        }

        let mut closed = 0_usize;
        for fd in 0..3 {
            if libc::fcntl(fd, libc::F_GETFD) == -1 {
                closed |= 1 << fd;
            }
        }
        CLOSED_STANDARD_FDS.store(closed, AtomicOrdering::Relaxed);
    }
}

/// Undo process-state changes made by Rust's runtime before `main`.
///
/// This restores an inherited default/ignored SIGPIPE disposition, removes
/// Rust's SIGSEGV/SIGBUS stack-overflow handlers, and closes standard
/// descriptors which were closed when the process started.
pub fn restore_shell_process_runtime_state() {
    // SAFETY: only valid signal dispositions captured in this image are
    // restored.
    unsafe {
        let inherited = INHERITED_SIGPIPE.load(AtomicOrdering::Relaxed);
        if inherited == libc::SIG_DFL || inherited == libc::SIG_IGN {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = inherited;
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaction(libc::SIGPIPE, &action, std::ptr::null_mut());
        }

        libc::signal(libc::SIGSEGV, libc::SIG_DFL);
        libc::signal(libc::SIGBUS, libc::SIG_DFL);
    }

    let closed = CLOSED_STANDARD_FDS.load(AtomicOrdering::Relaxed);
    let changes = (0..3)
        .filter(|fd| closed & (1 << fd) != 0)
        .map(|fd| (fd, None));
    if let Ok(changes) = ProcessDescriptorTransaction::new(changes) {
        let _ = changes.apply();
    }
}

#[cfg(coverage)]
unsafe extern "C" {
    fn __llvm_profile_write_file() -> core::ffi::c_int;
    fn __llvm_profile_reset_counters();
}

/// Flush LLVM's coverage profile before immediate process termination.
pub fn flush_coverage_profile() {
    #[cfg(coverage)]
    // SAFETY: this is the coverage runtime's process-global flush entry.
    unsafe {
        __llvm_profile_write_file();
    }
}

/// Clear inherited LLVM coverage counters in a newly forked child.
pub fn reset_coverage_counters() {
    #[cfg(coverage)]
    // SAFETY: this is the coverage runtime's process-global reset entry.
    unsafe {
        __llvm_profile_reset_counters();
    }
}

/// Look up the home directory named by a `~user` expansion.
///
/// `std::env::home_dir` answers only for the current user, while this lookup
/// deliberately handles a different named account. Bare `~` never reaches
/// this function: the shell expands it from that shell instance's `HOME`.
pub fn named_user_home(name: &OsStr) -> Option<PathBuf> {
    let name = CString::new(name.as_bytes()).ok()?;
    let mut size = 1024_usize;
    loop {
        let mut record = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        let mut storage = vec![0_u8; size];
        // SAFETY: the name is terminated; `record`, `result`, and `storage`
        // are writable for the call. Any returned strings point into
        // `storage` and are copied before it is dropped.
        let error = unsafe {
            libc::getpwnam_r(
                name.as_ptr(),
                record.as_mut_ptr(),
                storage.as_mut_ptr().cast(),
                storage.len(),
                &mut result,
            )
        };
        if error == libc::ERANGE {
            size = size.checked_mul(2)?;
            continue;
        }
        if error != 0 || result.is_null() {
            return None;
        }
        // SAFETY: success initialized `record`; `pw_dir` is either NULL or
        // a terminated string within live `storage`.
        let directory = unsafe { record.assume_init().pw_dir };
        if directory.is_null() {
            return None;
        }
        // SAFETY: `pw_dir` is a terminated passwd field and is copied now.
        return Some(PathBuf::from(OsString::from_vec(
            unsafe { CStr::from_ptr(directory) }.to_bytes().to_vec(),
        )));
    }
}

/// Shell-specific operations on native strings without exposing the host
/// representation to the shell crate.
pub trait NativeStrExt {
    fn to_shell_bytes(&self) -> Vec<u8>;
}

impl NativeStrExt for OsStr {
    fn to_shell_bytes(&self) -> Vec<u8> {
        self.as_bytes().to_vec()
    }
}

impl NativeStrExt for Path {
    fn to_shell_bytes(&self) -> Vec<u8> {
        self.as_os_str().to_shell_bytes()
    }
}

/// Native-string conversions for byte-oriented shell values.
pub trait ShellBytesExt {
    fn try_to_os_string(&self) -> std::io::Result<OsString>;
    fn try_to_path_buf(&self) -> std::io::Result<PathBuf>;
}

impl ShellBytesExt for [u8] {
    fn try_to_os_string(&self) -> std::io::Result<OsString> {
        Ok(OsString::from_vec(self.to_vec()))
    }

    fn try_to_path_buf(&self) -> std::io::Result<PathBuf> {
        self.try_to_os_string().map(PathBuf::from)
    }
}

/// Snapshot the process arguments in their native representation.
pub fn process_arguments() -> Vec<OsString> {
    std::env::args_os().collect()
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

pub const fn input_newline_width(_previous: Option<u8>) -> usize {
    1
}

pub fn resolve_command_path(path: &Path, _environment: &[(OsString, OsString)]) -> PathBuf {
    path.to_path_buf()
}

pub fn trim_command_substitution_output(output: &mut Vec<u8>, start: usize) {
    while output.len() > start && output.last() == Some(&b'\n') {
        output.pop();
    }
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
        .open(path)
}

/// Whether a terminal descriptor is in canonical input mode. `None` means
/// the descriptor is not a terminal (or its attributes cannot be queried).
pub fn terminal_canonical_mode(fd: &impl AsDescriptor) -> Option<bool> {
    let attributes = rustix::termios::tcgetattr(fd.as_platform_descriptor().0).ok()?;
    Some(
        attributes
            .local_modes
            .contains(rustix::termios::LocalModes::ICANON),
    )
}

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

/// Snapshot the calling process's environment as owned values.
///
/// Ownership is intentional: a shell instance must not retain pointers
/// into the process-global environment, which can be replaced by another
/// thread or by the host after construction.
// [spec:nsh:req:embedding-safety.process-environment-is-read-only]
pub fn process_environment() -> Vec<(OsString, OsString)> {
    std::env::vars_os().collect()
}

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

/// Replace the current process image.
///
/// The pointer arrays required by `execve` are assembled entirely inside the
/// platform crate from validated, live C strings. Success never returns; the
/// returned value is therefore always the operating-system error.
pub fn execute_program(
    path: &OsStr,
    argv: &[OsString],
    environment: &[(OsString, OsString)],
) -> std::io::Error {
    let path = match CString::new(path.as_bytes()) {
        Ok(path) => path,
        Err(_) => return std::io::Error::from(std::io::ErrorKind::InvalidInput),
    };
    let argv: Vec<CString> = match argv
        .iter()
        .map(|argument| CString::new(argument.as_bytes()))
        .collect()
    {
        Ok(argv) => argv,
        Err(_) => return std::io::Error::from(std::io::ErrorKind::InvalidInput),
    };
    let environment: Vec<CString> = match environment
        .iter()
        .map(|(name, value)| {
            let mut entry = Vec::with_capacity(name.as_bytes().len() + value.as_bytes().len() + 1);
            entry.extend_from_slice(name.as_bytes());
            entry.push(b'=');
            entry.extend_from_slice(value.as_bytes());
            CString::new(entry)
        })
        .collect()
    {
        Ok(environment) => environment,
        Err(_) => return std::io::Error::from(std::io::ErrorKind::InvalidInput),
    };
    let mut argv_pointers: Vec<*const u8> = argv
        .iter()
        .map(|argument| argument.as_ptr().cast())
        .collect();
    argv_pointers.push(std::ptr::null());
    let mut environment_pointers: Vec<*const u8> = environment
        .iter()
        .map(|entry| entry.as_ptr().cast())
        .collect();
    environment_pointers.push(std::ptr::null());

    // SAFETY: both pointer arrays are terminated, every preceding pointer
    // names a live NUL-terminated string, and `execve` retains nothing when
    // it reports an error. On success the process image is replaced.
    unsafe {
        libc::execve(
            path.as_ptr(),
            argv_pointers.as_ptr().cast(),
            environment_pointers.as_ptr().cast(),
        );
    }
    std::io::Error::last_os_error()
}

pub fn is_exec_format_error(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(rustix::io::Errno::NOEXEC.raw_os_error())
}

pub fn interrupt_signal() -> Signal {
    Signal::new(rustix::process::Signal::INT.as_raw()).expect("SIGINT is positive")
}

pub fn quit_signal() -> Signal {
    Signal::new(rustix::process::Signal::QUIT.as_raw()).expect("SIGQUIT is positive")
}

pub fn termination_signal() -> Signal {
    Signal::new(rustix::process::Signal::TERM.as_raw()).expect("SIGTERM is positive")
}

pub fn kill_signal() -> Signal {
    Signal::new(rustix::process::Signal::KILL.as_raw()).expect("SIGKILL is positive")
}

pub fn child_signal() -> Signal {
    Signal::new(rustix::process::Signal::CHILD.as_raw()).expect("SIGCHLD is positive")
}

pub fn pipe_signal() -> Signal {
    Signal::new(rustix::process::Signal::PIPE.as_raw()).expect("SIGPIPE is positive")
}

pub fn hangup_signal() -> Signal {
    Signal::new(rustix::process::Signal::HUP.as_raw()).expect("SIGHUP is positive")
}

pub fn terminal_stop_signal() -> Signal {
    Signal::new(rustix::process::Signal::TSTP.as_raw()).expect("SIGTSTP is positive")
}

pub fn terminal_input_signal() -> Signal {
    Signal::new(rustix::process::Signal::TTIN.as_raw()).expect("SIGTTIN is positive")
}

pub fn terminal_output_signal() -> Signal {
    Signal::new(rustix::process::Signal::TTOU.as_raw()).expect("SIGTTOU is positive")
}

pub fn continue_signal() -> Signal {
    Signal::new(rustix::process::Signal::CONT.as_raw()).expect("SIGCONT is positive")
}

fn raw_process_id(process: ProcessId) -> std::io::Result<i32> {
    i32::try_from(process.get()).map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))
}

fn raw_process_group(group: ProcessGroupId) -> std::io::Result<i32> {
    i32::try_from(group.get()).map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))
}

pub fn send_signal(target: ProcessTarget, request: SignalRequest) -> std::io::Result<()> {
    let target = match target {
        ProcessTarget::Process(process) => raw_process_id(process)?,
        ProcessTarget::CurrentProcessGroup => 0,
        ProcessTarget::ProcessGroup(group) => -raw_process_group(group)?,
        ProcessTarget::AllProcesses => -1,
    };
    let signal = match request {
        SignalRequest::Probe => 0,
        SignalRequest::Deliver(signal) => signal.number(),
    };
    // SAFETY: `kill` consumes only the scalar target encoding assembled from
    // validated identities above and a typed delivery or probe request.
    if unsafe { libc::kill(target, signal) } < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn raise_signal(signal: Signal) -> std::io::Result<()> {
    let signal = rustix::process::Signal::from_named_raw(signal.number())
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    rustix::process::kill_process(rustix::process::getpid(), signal).map_err(std::io::Error::from)
}

pub fn send_continue_to_process_group(process_group: ProcessGroupId) -> std::io::Result<()> {
    let process_group = rustix::process::Pid::from_raw(raw_process_group(process_group)?)
        .expect("a validated positive process group fits pid_t");
    rustix::process::kill_process_group(process_group, rustix::process::Signal::CONT)
        .map_err(std::io::Error::from)
}

pub fn terminate_with_interrupt() -> ! {
    let _ = install_signal_action(
        interrupt_signal(),
        SignalAction::Default,
        ignored_signal_placeholder,
    );
    let _ = raise_signal(interrupt_signal());
    std::process::abort()
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

fn limit_resource(resource: LimitResource) -> Option<rustix::process::Resource> {
    Some(match resource {
        LimitResource::Cpu => rustix::process::Resource::Cpu,
        LimitResource::FileSize => rustix::process::Resource::Fsize,
        LimitResource::Data => rustix::process::Resource::Data,
        LimitResource::Stack => rustix::process::Resource::Stack,
        LimitResource::Core => rustix::process::Resource::Core,
        LimitResource::ResidentSet => rustix::process::Resource::Rss,
        LimitResource::LockedMemory => rustix::process::Resource::Memlock,
        LimitResource::Processes => rustix::process::Resource::Nproc,
        LimitResource::OpenFiles => rustix::process::Resource::Nofile,
        LimitResource::AddressSpace => rustix::process::Resource::As,
        #[cfg(not(target_vendor = "apple"))]
        LimitResource::Locks => rustix::process::Resource::Locks,
        #[cfg(not(target_vendor = "apple"))]
        LimitResource::RealtimePriority => rustix::process::Resource::Rtprio,
        #[cfg(target_vendor = "apple")]
        LimitResource::Locks | LimitResource::RealtimePriority => return None,
    })
}

pub fn resource_limit(resource: LimitResource) -> std::io::Result<ResourceLimit> {
    let native = limit_resource(resource)
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::Unsupported))?;
    let raw = rustix::process::getrlimit(native);
    Ok(ResourceLimit {
        current: raw.current,
        maximum: raw.maximum,
    })
}

pub fn set_resource_limit(resource: LimitResource, limit: ResourceLimit) -> std::io::Result<()> {
    let native = limit_resource(resource)
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::Unsupported))?;
    let raw = rustix::process::Rlimit {
        current: limit.current,
        maximum: limit.maximum,
    };
    rustix::process::setrlimit(native, raw).map_err(std::io::Error::from)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignalAction {
    Default,
    Ignore,
    Catch,
}

static SIGNAL_HANDLERS: [AtomicUsize; SIGNAL_COUNT] = [const { AtomicUsize::new(0) }; SIGNAL_COUNT];

extern "C" fn signal_trampoline(number: i32) {
    let Some(signal) = Signal::new(number) else {
        return;
    };
    let Some(slot) = SIGNAL_HANDLERS.get(number as usize) else {
        return;
    };
    let address = slot.load(AtomicOrdering::Relaxed);
    if address == 0 {
        return;
    }
    // SAFETY: only `install_signal_action` stores addresses in this table,
    // and its handler parameter has this exact function type.
    let handler: fn(Signal) = unsafe { std::mem::transmute(address) };
    handler(signal);
}

pub fn signal_action(signal: Signal) -> std::io::Result<SignalAction> {
    let mut action = std::mem::MaybeUninit::<libc::sigaction>::uninit();
    // SAFETY: a null new action queries the disposition and initializes the
    // output record on success.
    if unsafe { libc::sigaction(signal.number(), std::ptr::null(), action.as_mut_ptr()) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: the successful query initialized the record.
    let action = unsafe { action.assume_init() };
    Ok(if action.sa_sigaction == libc::SIG_DFL {
        SignalAction::Default
    } else if action.sa_sigaction == libc::SIG_IGN {
        SignalAction::Ignore
    } else {
        SignalAction::Catch
    })
}

/// Guard that blocks every signal and restores the caller's prior mask when
/// dropped. The raw signal-set representation never leaves this crate.
pub struct BlockedSignals(libc::sigset_t);

impl BlockedSignals {
    pub fn all() -> std::io::Result<Self> {
        // SAFETY: both sets are initialized local storage and
        // `sigprocmask` copies their contents synchronously.
        unsafe {
            let mut all: libc::sigset_t = std::mem::zeroed();
            let mut old: libc::sigset_t = std::mem::zeroed();
            libc::sigfillset(&mut all);
            if libc::sigprocmask(libc::SIG_BLOCK, &all, &mut old) < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(Self(old))
            }
        }
    }

    /// Atomically restore the saved mask while waiting for a signal, then
    /// return with every signal blocked again.
    pub fn suspend(&self) -> std::io::Result<()> {
        // SAFETY: the saved set was initialized by `sigprocmask`; the call
        // copies it and retains no pointer.
        unsafe {
            libc::sigsuspend(&self.0);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted {
            Ok(())
        } else {
            Err(error)
        }
    }
}

impl Drop for BlockedSignals {
    fn drop(&mut self) {
        // SAFETY: the saved set was initialized by a successful
        // `sigprocmask` call and is copied synchronously.
        unsafe {
            libc::sigprocmask(libc::SIG_SETMASK, &self.0, std::ptr::null_mut());
        }
    }
}

pub fn install_signal_action(
    signal: Signal,
    action: SignalAction,
    handler: fn(Signal),
) -> std::io::Result<()> {
    let Some(slot) = SIGNAL_HANDLERS.get(signal.number() as usize) else {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    };
    slot.store(handler as usize, AtomicOrdering::Relaxed);
    // SAFETY: the action is fully initialized, `signal_trampoline` has the
    // platform signal ABI and static lifetime, and sigaction copies the record.
    unsafe {
        let mut raw: libc::sigaction = std::mem::zeroed();
        raw.sa_sigaction = match action {
            SignalAction::Default => libc::SIG_DFL,
            SignalAction::Ignore => libc::SIG_IGN,
            SignalAction::Catch => signal_trampoline as *const () as usize,
        };
        raw.sa_flags = 0;
        libc::sigfillset(&mut raw.sa_mask);
        if libc::sigaction(signal.number(), &raw, std::ptr::null_mut()) < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

fn ignored_signal_placeholder(_: Signal) {}

pub fn ignore_signal(signal: Signal) -> std::io::Result<()> {
    // `SIG_IGN` needs no callback; the placeholder is never installed.
    install_signal_action(signal, SignalAction::Ignore, ignored_signal_placeholder)
}

/// Whether `signal` is blocked in the calling thread's current mask.
pub fn signal_is_blocked(signal: Signal) -> std::io::Result<bool> {
    // SAFETY: a null new mask queries the current mask and initializes
    // `current`; `sigismember` only reads that initialized set.
    unsafe {
        let mut current: libc::sigset_t = std::mem::zeroed();
        if libc::sigprocmask(libc::SIG_SETMASK, std::ptr::null(), &mut current) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let result = libc::sigismember(&current, signal.number());
        if result < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(result != 0)
        }
    }
}

/// Replace the process file-creation mask and return its previous value.
pub fn replace_creation_mask(mask: u32) -> u32 {
    // SAFETY: `umask` accepts every mode bit pattern and returns the prior mask.
    unsafe { libc::umask(mask as libc::mode_t) as u32 }
}

/// Read the process file-creation mask, restoring it before returning.
pub fn creation_mask() -> u32 {
    let mask = replace_creation_mask(0);
    replace_creation_mask(mask);
    mask
}

/// CPU time consumed by this process and by waited-for children, in seconds.
#[derive(Clone, Copy, Debug)]
pub struct ProcessTimes {
    pub user: f64,
    pub system: f64,
    pub children_user: f64,
    pub children_system: f64,
}

/// Read the values reported by POSIX `times(2)`.
pub fn process_times() -> ProcessTimes {
    let mut raw = std::mem::MaybeUninit::<libc::tms>::uninit();
    // SAFETY: `times` initializes the pointed-to `tms` record.
    let raw = unsafe {
        libc::times(raw.as_mut_ptr());
        raw.assume_init()
    };
    let ticks_per_second = rustix::param::clock_ticks_per_second() as f64;
    ProcessTimes {
        user: raw.tms_utime as f64 / ticks_per_second,
        system: raw.tms_stime as f64 / ticks_per_second,
        children_user: raw.tms_cutime as f64 / ticks_per_second,
        children_system: raw.tms_cstime as f64 / ticks_per_second,
    }
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

/// Whether an endpoint is attached to a terminal.
pub fn is_terminal(fd: &impl AsDescriptor) -> bool {
    rustix::termios::tcgetattr(fd.as_platform_descriptor().0).is_ok()
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

/// Terminate the current process immediately, without running destructors.
pub fn exit_immediately(status: i32) -> ! {
    // `std::process::exit` runs Rust runtime cleanup. After `fork` in a
    // multithreaded host that cleanup can wait forever on a mutex whose
    // owner existed only in the parent. The syscall wrapper is safe and
    // performs exactly the child-terminating operation required here.
    // SAFETY: `_exit` terminates the calling process without Rust cleanup.
    unsafe { libc::_exit(status) }
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

pub fn is_pseudoterminal_end(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error())
}

pub fn is_bad_descriptor_error(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(rustix::io::Errno::BADF.raw_os_error())
}

/// Unblock every signal in the calling thread.
pub fn unblock_all_signals() -> std::io::Result<()> {
    // SAFETY: both signal-set objects are initialized before use and the
    // call retains neither pointer.
    let result = unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigprocmask(libc::SIG_SETMASK, &set, std::ptr::null_mut())
    };
    if result < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Signal dispositions used by the child that feeds a large here-document.
pub fn configure_here_document_writer_signals() {
    for signal in [
        interrupt_signal(),
        quit_signal(),
        hangup_signal(),
        terminal_stop_signal(),
    ] {
        let _ = ignore_signal(signal);
    }
    let _ = install_signal_action(
        pipe_signal(),
        SignalAction::Default,
        ignored_signal_placeholder,
    );
}

#[inline]
pub fn parent_process_id() -> Option<ProcessId> {
    ProcessId::new(rustix::process::Pid::as_raw(rustix::process::getppid()) as u32)
}

#[inline]
pub fn current_process_id() -> ProcessId {
    ProcessId::new(rustix::process::getpid().as_raw_pid() as u32)
        .expect("the current process has a positive identity")
}

/// Fork the calling process. The child is expected to terminate or replace
/// its image rather than return to an embedding host.
pub fn fork_process() -> std::io::Result<ForkResult> {
    // SAFETY: the raw fork result is immediately converted to an enum; no
    // borrowed storage crosses the boundary.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        Err(std::io::Error::last_os_error())
    } else if pid == 0 {
        Ok(ForkResult::Child)
    } else {
        Ok(ForkResult::Parent(
            ProcessId::new(pid as u32).expect("fork returned a positive child identity"),
        ))
    }
}

pub fn current_process_group() -> ProcessGroupState {
    // SAFETY: `getpgrp` takes no arguments and cannot fail. Keep the raw
    // result because a process group inherited from outside a fresh PID
    // namespace is reported as zero until the shell establishes its own;
    // PID newtypes deliberately cannot represent that observable state.
    let group = unsafe { libc::getpgrp() };
    ProcessGroupState::from_platform_value(group)
        .expect("getpgrp returns a nonnegative process group")
}

pub fn foreground_process_group(fd: &impl AsDescriptor) -> std::io::Result<ProcessGroupState> {
    // SAFETY: the kernel validates the descriptor and returns only an
    // integer process-group id. As above, zero is a meaningful transient
    // result in a PID namespace and must not be rejected by a PID newtype.
    let group = unsafe { libc::tcgetpgrp(fd.as_platform_descriptor().0.as_raw_fd()) };
    if group < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(ProcessGroupState::from_platform_value(group)
            .expect("tcgetpgrp returned a nonnegative process group"))
    }
}

pub fn set_process_group(
    process: ProcessSelector,
    group: ProcessGroupState,
) -> std::io::Result<()> {
    let process = match process {
        ProcessSelector::CurrentProcess => None,
        ProcessSelector::Process(process) => {
            Some(rustix::process::Pid::from_raw(raw_process_id(process)?).unwrap())
        }
    };
    let group = rustix::process::Pid::from_raw(group.platform_value()?);
    rustix::process::setpgid(process, group).map_err(std::io::Error::from)
}

pub fn set_foreground_process_group(
    fd: &impl AsDescriptor,
    group: ProcessGroupState,
) -> std::io::Result<()> {
    let group = group.platform_value()?;
    // SAFETY: both arguments are scalar values validated above.
    if unsafe { libc::tcsetpgrp(fd.as_platform_descriptor().0.as_raw_fd(), group) } < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn decode_child_status(status: rustix::process::WaitStatus) -> ChildStatus {
    if let Some(code) = status.exit_status() {
        return ChildStatus::Exited(u8::try_from(code).expect("wait exit status fits eight bits"));
    }
    if let Some(number) = status.stopping_signal() {
        return ChildStatus::Stopped(
            Signal::new(number).expect("a stopping signal number is positive"),
        );
    }
    if status.continued() {
        return ChildStatus::Continued;
    }
    let number = status
        .terminating_signal()
        .expect("wait returned a recognized child state");
    ChildStatus::Signaled {
        signal: Signal::new(number).expect("a terminating signal number is positive"),
        core_dumped: std::process::ExitStatus::from_raw(status.as_raw()).core_dumped(),
    }
}

/// Wait for any child. `None` is the successful nonblocking "not yet" case.
pub fn wait_for_any_child(
    nonblocking: bool,
    report_stopped: bool,
) -> std::io::Result<Option<(ProcessId, ChildStatus)>> {
    let mut options = rustix::process::WaitOptions::empty();
    if nonblocking {
        options.insert(rustix::process::WaitOptions::NOHANG);
    }
    if report_stopped {
        options.insert(rustix::process::WaitOptions::UNTRACED);
    }
    rustix::process::wait(options)
        .map(|result| {
            result.map(|(pid, status)| {
                (
                    ProcessId::new(pid.as_raw_pid() as u32)
                        .expect("wait returned a positive child identity"),
                    decode_child_status(status),
                )
            })
        })
        .map_err(std::io::Error::from)
}

/// Run a closure in a forked child and return the shell-style status observed
/// by the parent. This is primarily useful for tests of process-terminating
/// entry points.
pub fn run_in_child(body: impl FnOnce()) -> std::io::Result<i32> {
    match fork_process()? {
        ForkResult::Child => {
            body();
            exit_immediately(0);
        }
        ForkResult::Parent(pid) => {
            let pid = rustix::process::Pid::from_raw(raw_process_id(pid)?)
                .expect("a forked child identity fits pid_t");
            let status = loop {
                match rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::empty()) {
                    Ok(Some((_, status))) => break status,
                    Ok(None) => continue,
                    Err(rustix::io::Errno::INTR) => continue,
                    Err(error) => return Err(std::io::Error::from(error)),
                }
            };
            Ok(match decode_child_status(status) {
                ChildStatus::Exited(code) => i32::from(code),
                ChildStatus::Signaled { signal, .. } => 128 + signal.number(),
                ChildStatus::Stopped(signal) => 128 + signal.number(),
                ChildStatus::Continued => 0,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    // [spec:nsh:req:idiom.filesystem-account-bytes/test]
    #[test]
    fn native_string_extensions_round_trip_non_utf8_values() {
        let bytes = vec![b'n', b's', b'h', b'-', 0xff];
        let native = bytes.as_slice().try_to_os_string().unwrap();

        assert_eq!(native.to_shell_bytes(), bytes);
        assert_eq!(
            bytes.as_slice().try_to_path_buf().unwrap().as_os_str(),
            native
        );

        let label = OsStr::from_bytes(b"nsh-platform-\xff");
        let (file, path) = create_temporary_file(label).unwrap();
        drop(file);
        assert!(
            path.file_name()
                .unwrap()
                .as_bytes()
                .starts_with(label.as_bytes())
        );
        remove_file(&path).unwrap();

        let anonymous = anonymous_file(label).unwrap();
        write_all(&anonymous, b"native label").unwrap();
        assert_eq!(take_file_contents(&anonymous).unwrap(), b"native label");
    }

    #[test]
    fn os_error_boundaries_are_classified() {
        let error = std::io::Error::from(rustix::io::Errno::NOENT);
        let message = Locale::c().unwrap().error_message(&error);
        assert!(!message.contains("(os error"));
        assert!(!message.is_empty());

        assert!(is_path_error(
            &std::io::Error::from(rustix::io::Errno::NAMETOOLONG),
            PathErrorKind::NameTooLong,
        ));
        assert!(is_path_error(
            &std::io::Error::from(rustix::io::Errno::NOENT),
            PathErrorKind::NotFound,
        ));
        assert!(is_path_error(
            &std::io::Error::from(rustix::io::Errno::NOTDIR),
            PathErrorKind::NotFound,
        ));
        assert!(!is_path_error(
            &std::io::Error::from(rustix::io::Errno::ACCESS),
            PathErrorKind::NotFound,
        ));
    }

    #[test]
    fn temporary_files_are_unique_owned_and_private() {
        let (first, first_path) = create_temporary_file("nsh-platform-test").unwrap();
        let (second, second_path) = create_temporary_file("nsh-platform-test").unwrap();

        assert_ne!(first_path, second_path);
        assert_eq!(
            first.metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            second.metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );

        drop(first);
        drop(second);
        remove_file(&first_path).unwrap();
        remove_file(&second_path).unwrap();
    }

    #[test]
    fn duplicated_descriptor_outlives_source() {
        let source = std::fs::File::open("/dev/null").unwrap();
        let duplicate = duplicate_fd(&source).unwrap();
        drop(source);

        let mut byte = [0];
        assert_eq!(read_once(&duplicate, &mut byte).unwrap(), 0);
    }

    // [spec:nsh:req:idiom.descriptor-materialization/test]
    #[test]
    fn descriptor_transaction_installs_slots() {
        let (read, write) = pipe().unwrap();
        let status = run_in_child(move || {
            let source = duplicate_cloexec(&write, 10).unwrap();
            ProcessDescriptorTransaction::new([(7, Some(source)), (8, None)])
                .unwrap()
                .apply()
                .unwrap();

            let seven = snapshot_process_fd(7, 10).unwrap().unwrap();
            write_all(&seven, b"staged").unwrap();
            if snapshot_process_fd(8, 10).unwrap().is_some() {
                exit_immediately(2);
            }
            exit_immediately(0);
        })
        .unwrap();

        assert_eq!(status, 0);
        assert_eq!(read_exact(&read, 6).unwrap(), b"staged");
    }

    #[test]
    fn descriptor_transaction_validates_targets() {
        assert!(ProcessDescriptorTransaction::new([(-1, None)]).is_err());
        assert!(ProcessDescriptorTransaction::new([(4, None), (4, None)]).is_err());
        let (_, write) = pipe().unwrap();
        let number = write.number();

        assert!(ProcessDescriptorTransaction::new([(-1, Some(write))]).is_err());
        assert!(snapshot_process_fd(number, 10).unwrap().is_none());
    }
}
