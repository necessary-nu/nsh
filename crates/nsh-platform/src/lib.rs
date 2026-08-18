//! The operating-system and C-library boundary for `nsh`.
//!
//! Public functions in this crate are safe. The raw ABI details they
//! validate and contain are deliberately kept out of the shell crate, so
//! `unsafe` in `nsh` can eventually be denied rather than normalized.

#![deny(unsafe_op_in_unsafe_fn)]

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::collections::hash_map::RandomState;
use std::ffi::{CStr, CString, OsStr, OsString};
use std::hash::BuildHasher as _;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::OpenOptionsExt as _;
use std::os::unix::process::ExitStatusExt as _;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

use rustix::fs::{Access, AtFlags, CWD, accessat};
use rustix::process::{Gid, Uid};

mod locale;
pub use locale::{Locale, LocaleCategory, LocaleDecode, LocaleDecoder};

// Rust's runtime changes three pieces of inherited process state before
// `main`: it ignores SIGPIPE, installs stack-overflow handlers, and opens
// /dev/null over closed standard descriptors. A shell must present the state
// it inherited instead. The two values Rust would otherwise destroy are
// captured by this constructor before the runtime initializes.
static INHERITED_SIGPIPE: AtomicUsize = AtomicUsize::new(usize::MAX);
static CLOSED_STANDARD_FDS: AtomicUsize = AtomicUsize::new(0);

#[used]
#[unsafe(link_section = ".init_array")]
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
        .map(|fd| (fd as i32, None));
    if let Ok(changes) = ProcessFdChanges::new(changes) {
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

unsafe extern "C" {
    fn wctype(name: *const core::ffi::c_char) -> core::ffi::c_ulong;
    fn iswctype(wc: core::ffi::c_uint, desc: core::ffi::c_ulong) -> core::ffi::c_int;
    fn mbrtowc(
        wide: *mut i32,
        bytes: *const core::ffi::c_char,
        len: usize,
        state: *mut libc::mbstate_t,
    ) -> usize;
    fn mbrlen(
        bytes: *const core::ffi::c_char,
        len: usize,
        state: *mut libc::mbstate_t,
    ) -> usize;
    fn iswblank(wc: core::ffi::c_uint) -> core::ffi::c_int;
    fn iswspace(wc: core::ffi::c_uint) -> core::ffi::c_int;
}

pub fn locale_wide_is_blank(wide: i32) -> bool {
    // SAFETY: every `i32` value can be passed as a `wint_t`; invalid values
    // simply do not match.
    unsafe { iswblank(wide as core::ffi::c_uint) != 0 }
}

/// Length of the first complete multibyte character in the process locale.
/// Invalid and incomplete byte sequences return `None`.
pub fn locale_multibyte_len(bytes: &[u8]) -> Option<usize> {
    let mut state = unsafe { std::mem::zeroed() };
    // SAFETY: the input is bounded by the slice and the conversion state is
    // initialized local storage retained only for this call.
    let length = unsafe { mbrlen(bytes.as_ptr().cast(), bytes.len(), &mut state) };
    if length == usize::MAX || length == usize::MAX - 1 {
        None
    } else {
        Some(length)
    }
}

/// Decode exactly one locale multibyte character of `expected_len` bytes.
pub fn locale_decode_exact(bytes: &[u8], expected_len: usize) -> Option<i32> {
    if expected_len > bytes.len() {
        return None;
    }
    let mut state = unsafe { std::mem::zeroed() };
    let mut wide = 0_i32;
    // SAFETY: the byte count is bounded by the slice and both output records
    // are initialized local storage retained only for this call.
    let converted = unsafe {
        mbrtowc(
            &mut wide,
            bytes.as_ptr().cast(),
            expected_len,
            &mut state,
        )
    };
    (converted == expected_len).then_some(wide)
}

pub fn locale_wide_is_space(wide: i32) -> bool {
    // SAFETY: every `i32` value is accepted as a `wint_t`; invalid values
    // simply do not match.
    unsafe { iswspace(wide as core::ffi::c_uint) != 0 }
}

/// Look up the home directory named by a `~user` expansion.
///
/// `std::env::home_dir` answers only for the current user, while this lookup
/// deliberately handles a different named account. Bare `~` never reaches
/// this function: the shell expands it from that shell instance's `HOME`.
pub fn named_user_home(name: &[u8]) -> Option<OsString> {
    let name = CString::new(name).ok()?;
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
        return Some(OsString::from_vec(
            unsafe { CStr::from_ptr(directory) }.to_bytes().to_vec(),
        ));
    }
}

/// Test one locale multibyte character against a POSIX wide-character class.
/// `None` means the class name is unknown; a known class with malformed
/// character bytes is a non-match.
pub fn wide_class_matches(name: &[u8], bytes: &[u8], expected_len: usize) -> Option<bool> {
    let name = CString::new(name).ok()?;
    // SAFETY: the class name is terminated, the output/state pointers name
    // initialized local storage, and the byte count is bounded by `bytes`.
    unsafe {
        let class = wctype(name.as_ptr());
        if class == 0 {
            return None;
        }
        let mut wide = 0_i32;
        let mut state = std::mem::zeroed();
        let converted = mbrtowc(
            &mut wide,
            bytes.as_ptr().cast(),
            expected_len.min(bytes.len()),
            &mut state,
        );
        Some(converted == expected_len && iswctype(wide as core::ffi::c_uint, class) != 0)
    }
}

/// Decode locale multibyte bytes for the shell's IFS cache.
pub fn locale_wide_chars(bytes: &[u8]) -> (usize, Vec<i32>) {
    if bytes.is_empty() {
        return (0, Vec::new());
    }
    // SAFETY: every conversion is bounded by the remaining slice and writes
    // only to local initialized storage.
    unsafe {
        let mut first_state = std::mem::zeroed();
        let mut first = 0_i32;
        let first_len = mbrtowc(
            &mut first,
            bytes.as_ptr().cast(),
            bytes.len(),
            &mut first_state,
        );
        let first_len = if first_len == usize::MAX || first_len == usize::MAX - 1 {
            1
        } else {
            first_len
        };

        let mut decoded = vec![0_i32; bytes.len() + 1];
        let mut state = std::mem::zeroed();
        let mut offset = 0;
        let mut output = 0;
        while offset < bytes.len() {
            let mut wide = 0_i32;
            let count = mbrtowc(
                &mut wide,
                bytes[offset..].as_ptr().cast(),
                bytes.len() - offset,
                &mut state,
            );
            if count == usize::MAX || count == usize::MAX - 1 || count == 0 {
                break;
            }
            decoded[output] = wide;
            output += 1;
            offset += count;
        }
        (first_len, decoded)
    }
}

/// Compare two byte strings with the process locale's collation rules.
///
/// The shell cannot contain a NUL in an argument; accepting slices here
/// makes that invariant explicit at the boundary. If a caller violates it,
/// compare the C-visible prefixes, which is what `strcoll` would have seen.
pub fn collate(left: &[u8], right: &[u8]) -> Ordering {
    fn c_string(bytes: &[u8]) -> CString {
        let visible = bytes.split(|&byte| byte == 0).next().unwrap_or_default();
        CString::new(visible).expect("the C-visible prefix contains no NUL")
    }

    let left = c_string(left);
    let right = c_string(right);
    // SAFETY: both pointers name live, NUL-terminated strings for the call.
    unsafe { libc::strcoll(left.as_ptr(), right.as_ptr()).cmp(&0) }
}

/// Test access using effective rather than real credentials.
pub fn effective_access(path: &OsStr, access: Access) -> bool {
    accessat(CWD, path, access, AtFlags::EACCESS).is_ok()
}

/// Whether a terminal descriptor is in canonical input mode. `None` means
/// the descriptor is not a terminal (or its attributes cannot be queried).
pub fn terminal_canonical_mode(fd: impl AsFd) -> Option<bool> {
    let attributes = rustix::termios::tcgetattr(fd.as_fd()).ok()?;
    Some(
        attributes
            .local_modes
            .contains(rustix::termios::LocalModes::ICANON),
    )
}

/// Probe whether the descriptor has a current seek position.
pub fn fd_is_seekable(fd: impl AsFd) -> bool {
    rustix::fs::seek(fd.as_fd(), rustix::fs::SeekFrom::Current(0))
        .map(|_| ())
        .map_err(std::io::Error::from)
        .is_ok()
}

/// Move a descriptor's current position relative to where it is now.
pub fn seek_relative(
    fd: impl AsFd,
    offset: i64,
) -> std::io::Result<u64> {
    rustix::fs::seek(fd.as_fd(), rustix::fs::SeekFrom::Current(offset))
        .map_err(std::io::Error::from)
}

/// Rewind a descriptor to its beginning.
pub fn seek_start(fd: impl AsFd) -> std::io::Result<u64> {
    rustix::fs::seek(fd.as_fd(), rustix::fs::SeekFrom::Start(0))
        .map_err(std::io::Error::from)
}

#[inline]
pub fn effective_uid() -> Uid {
    rustix::process::geteuid()
}

#[inline]
pub fn effective_gid() -> Gid {
    rustix::process::getegid()
}

pub fn supplementary_groups() -> std::io::Result<Vec<Gid>> {
    rustix::process::getgroups().map_err(std::io::Error::from)
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

#[inline]
pub fn process_id() -> u32 {
    std::process::id()
}

#[inline]
pub fn locale_is_alpha(byte: u8) -> bool {
    // SAFETY: `isalpha` accepts EOF or an unsigned-char value; `u8` is in range.
    unsafe { libc::isalpha(byte.into()) != 0 }
}

#[inline]
pub fn locale_is_alphanumeric(byte: u8) -> bool {
    // SAFETY: `isalnum` accepts EOF or an unsigned-char value; `u8` is in range.
    unsafe { libc::isalnum(byte.into()) != 0 }
}

pub fn range_error_message() -> String {
    os_error_message_code(rustix::io::Errno::RANGE.raw_os_error())
}

pub fn os_error_message(error: &std::io::Error) -> String {
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

pub fn os_error_message_code(code: i32) -> String {
    os_error_message(&std::io::Error::from_raw_os_error(code))
}

pub fn is_path_not_found_error(code: i32) -> bool {
    matches!(
        code,
        value if value == rustix::io::Errno::NOENT.raw_os_error()
            || value == rustix::io::Errno::NOTDIR.raw_os_error()
    )
}

/// The error number used when a command candidate exists but is not
/// executable. Keeping the numeric ABI detail here lets the shell carry an
/// ordinary `i32` only for rendering the historical diagnostic.
pub fn permission_denied_error_code() -> i32 {
    rustix::io::Errno::ACCESS.raw_os_error()
}

pub fn already_exists_error() -> std::io::Error {
    std::io::Error::from(rustix::io::Errno::EXIST)
}

/// The initial error for a PATH search before any candidate has supplied a
/// more informative failure.
pub fn not_found_error_code() -> i32 {
    rustix::io::Errno::NOENT.raw_os_error()
}

/// POSIX distinguishes "command not found" (127) from a command that was
/// found but could not be executed (126).
pub fn command_exec_failure_status(code: i32) -> i32 {
    if [
        rustix::io::Errno::LOOP,
        rustix::io::Errno::NAMETOOLONG,
        rustix::io::Errno::NOENT,
        rustix::io::Errno::NOTDIR,
    ]
    .iter()
    .any(|error| error.raw_os_error() == code)
    {
        127
    } else {
        126
    }
}

/// Replace the current process image.
///
/// The pointer arrays required by `execve` are assembled entirely inside the
/// platform crate from validated, live C strings. Success never returns; the
/// returned value is therefore always the operating-system error.
pub fn execute_program(path: &CStr, argv: &[&CStr], environment: &[CString]) -> std::io::Error {
    let mut argv_pointers: Vec<*const u8> =
        argv.iter().map(|argument| argument.as_ptr().cast()).collect();
    argv_pointers.push(std::ptr::null());
    let mut environment_pointers: Vec<*const u8> = environment
        .iter()
        .map(|entry| entry.as_ptr().cast())
        .collect();
    environment_pointers.push(std::ptr::null());

    // SAFETY: both pointer arrays are terminated, every preceding pointer
    // names a live NUL-terminated string, and `execve` retains nothing when
    // it reports an error. On success the process image is replaced.
    let error = unsafe {
        rustix::runtime::execve(
            path,
            argv_pointers.as_ptr(),
            environment_pointers.as_ptr(),
        )
    };
    std::io::Error::from_raw_os_error(error.raw_os_error())
}

pub fn is_exec_format_error(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(rustix::io::Errno::NOEXEC.raw_os_error())
}

pub fn interrupt_signal() -> i32 {
    rustix::process::Signal::INT.as_raw()
}

pub fn quit_signal() -> i32 {
    rustix::process::Signal::QUIT.as_raw()
}

pub fn termination_signal() -> i32 {
    rustix::process::Signal::TERM.as_raw()
}

pub fn kill_signal() -> i32 {
    rustix::process::Signal::KILL.as_raw()
}

pub fn child_signal() -> i32 {
    rustix::process::Signal::CHILD.as_raw()
}

pub fn pipe_signal() -> i32 {
    rustix::process::Signal::PIPE.as_raw()
}

pub fn hangup_signal() -> i32 {
    rustix::process::Signal::HUP.as_raw()
}

pub fn terminal_stop_signal() -> i32 {
    rustix::process::Signal::TSTP.as_raw()
}

pub fn terminal_input_signal() -> i32 {
    rustix::process::Signal::TTIN.as_raw()
}

pub fn terminal_output_signal() -> i32 {
    rustix::process::Signal::TTOU.as_raw()
}

pub fn continue_signal() -> i32 {
    rustix::process::Signal::CONT.as_raw()
}

pub fn send_signal(pid: i32, signal: i32) -> std::io::Result<()> {
    // SAFETY: `kill` consumes only integer process and signal identifiers.
    if unsafe { libc::kill(pid, signal) } < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn send_signal_to_process_group(group: i32, signal: i32) -> std::io::Result<()> {
    let signal = rustix::process::Signal::from_named_raw(signal)
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    if group == 0 {
        rustix::process::kill_current_process_group(signal).map_err(std::io::Error::from)
    } else {
        let group = rustix::process::Pid::from_raw(group)
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        rustix::process::kill_process_group(group, signal).map_err(std::io::Error::from)
    }
}

pub fn raise_signal(signal: i32) -> std::io::Result<()> {
    let signal = rustix::process::Signal::from_named_raw(signal)
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    rustix::process::kill_process(rustix::process::getpid(), signal)
        .map_err(std::io::Error::from)
}

pub fn send_continue_to_process_group(process_group: i32) -> std::io::Result<()> {
    let process_group = rustix::process::Pid::from_raw(process_group)
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    rustix::process::kill_process_group(process_group, rustix::process::Signal::CONT)
        .map_err(std::io::Error::from)
}

pub fn wait_status_is_stopped(status: i32) -> bool {
    std::process::ExitStatus::from_raw(status)
        .stopped_signal()
        .is_some()
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

fn limit_resource(resource: LimitResource) -> rustix::process::Resource {
    match resource {
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
        LimitResource::Locks => rustix::process::Resource::Locks,
        LimitResource::RealtimePriority => rustix::process::Resource::Rtprio,
    }
}

pub fn resource_limit(resource: LimitResource) -> std::io::Result<ResourceLimit> {
    let raw = rustix::process::getrlimit(limit_resource(resource));
    Ok(ResourceLimit {
        current: raw.current,
        maximum: raw.maximum,
    })
}

pub fn set_resource_limit(resource: LimitResource, limit: ResourceLimit) -> std::io::Result<()> {
    let raw = rustix::process::Rlimit {
        current: limit.current,
        maximum: limit.maximum,
    };
    rustix::process::setrlimit(limit_resource(resource), raw).map_err(std::io::Error::from)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignalAction {
    Default,
    Ignore,
    Catch,
}

pub fn signal_action(signal: i32) -> std::io::Result<SignalAction> {
    let mut action = std::mem::MaybeUninit::<libc::sigaction>::uninit();
    // SAFETY: a null new action queries the disposition and initializes the
    // output record on success.
    if unsafe { libc::sigaction(signal, std::ptr::null(), action.as_mut_ptr()) } < 0 {
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
    signal: i32,
    action: SignalAction,
    handler: extern "C" fn(i32),
) -> std::io::Result<()> {
    // SAFETY: the action is fully initialized, the handler has the platform
    // signal ABI and static lifetime, and sigaction copies the record.
    unsafe {
        let mut raw: libc::sigaction = std::mem::zeroed();
        raw.sa_sigaction = match action {
            SignalAction::Default => libc::SIG_DFL,
            SignalAction::Ignore => libc::SIG_IGN,
            SignalAction::Catch => handler as *const () as usize,
        };
        raw.sa_flags = 0;
        libc::sigfillset(&mut raw.sa_mask);
        if libc::sigaction(signal, &raw, std::ptr::null_mut()) < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

extern "C" fn ignored_signal_placeholder(_: i32) {}

pub fn ignore_signal(signal: i32) -> std::io::Result<()> {
    // `SIG_IGN` needs no callback; the placeholder is never installed.
    install_signal_action(signal, SignalAction::Ignore, ignored_signal_placeholder)
}

/// Whether `signal` is blocked in the calling thread's current mask.
pub fn signal_is_blocked(signal: i32) -> std::io::Result<bool> {
    // SAFETY: a null new mask queries the current mask and initializes
    // `current`; `sigismember` only reads that initialized set.
    unsafe {
        let mut current: libc::sigset_t = std::mem::zeroed();
        if libc::sigprocmask(libc::SIG_SETMASK, std::ptr::null(), &mut current) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let result = libc::sigismember(&current, signal);
        if result < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(result != 0)
        }
    }
}

/// Replace the process file-creation mask and return its previous value.
pub fn replace_creation_mask(mask: u32) -> u32 {
    rustix::process::umask(rustix::fs::Mode::from_bits_retain(mask)).bits()
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
/// Ordinary descriptor ownership belongs to [`OwnedFd`]. Exact slot numbers
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
#[derive(Debug)]
pub struct ProcessFdChanges {
    changes: Vec<(i32, Option<OwnedFd>)>,
}

impl ProcessFdChanges {
    /// Validate and stage exact-slot changes without modifying the process.
    ///
    /// `Some(fd)` duplicates `fd` into the target when [`apply`](Self::apply)
    /// runs; `None` closes the target. Every source is first moved above the
    /// highest target with close-on-exec set, so target/source aliasing and
    /// cycles cannot affect the result.
    pub fn new(
        changes: impl IntoIterator<Item = (i32, Option<OwnedFd>)>,
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
                Some(source) if source.as_raw_fd() < minimum => {
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
                    let result = unsafe { libc::dup2(source.as_raw_fd(), *target) };
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
pub fn snapshot_process_fd(number: i32, minimum: i32) -> std::io::Result<Option<OwnedFd>> {
    if number < 0 || minimum < 0 {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    // SAFETY: `fcntl` receives scalar descriptor numbers and returns a new
    // owned descriptor on success. It retains no pointer or Rust borrow.
    let result = unsafe { libc::fcntl(number, libc::F_DUPFD_CLOEXEC, minimum) };
    if result >= 0 {
        // SAFETY: a successful F_DUPFD_CLOEXEC returns a fresh descriptor
        // whose ownership is transferred to the caller.
        return Ok(Some(unsafe { OwnedFd::from_raw_fd(result) }));
    }
    let error = std::io::Error::last_os_error();
    if is_bad_descriptor_error(&error) {
        Ok(None)
    } else {
        Err(error)
    }
}

/// Open `/dev/null` for reading and transfer ownership of the descriptor.
pub fn open_null_input() -> std::io::Result<OwnedFd> {
    std::fs::File::open("/dev/null").map(OwnedFd::from)
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
pub fn open_path(path: &OsStr, mode: OpenMode) -> std::io::Result<OwnedFd> {
    rustix::fs::open(
        path,
        mode.flags() | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::from_bits_retain(0o666),
    )
    .map_err(std::io::Error::from)
}

/// Whether an open descriptor refers to a regular file.
pub fn fd_is_regular_file(fd: impl AsFd) -> std::io::Result<bool> {
    let metadata = rustix::fs::fstat(fd.as_fd()).map_err(std::io::Error::from)?;
    Ok(rustix::fs::FileType::from_raw_mode(metadata.st_mode).is_file())
}

/// Create an anonymous in-memory file and transfer ownership of its descriptor.
pub fn anonymous_file(name: &std::ffi::CStr) -> std::io::Result<OwnedFd> {
    rustix::fs::memfd_create(name, rustix::fs::MemfdFlags::CLOEXEC)
        .map_err(std::io::Error::from)
}

/// Create a temporary file from a template ending in `XXXXXX`.
/// The returned file owns the descriptor and is created atomically with mode
/// `0600`; an existing path is never followed or replaced.
pub fn create_temporary_file(template: &OsStr) -> std::io::Result<(std::fs::File, OsString)> {
    const PLACEHOLDER: &[u8] = b"XXXXXX";
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

    let Some(prefix) = template.as_bytes().strip_suffix(PLACEHOLDER) else {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    };
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

/// Read a seekable descriptor from the beginning, then truncate and rewind it.
pub fn take_file_contents(fd: impl AsFd) -> std::io::Result<Vec<u8>> {
    let fd = fd.as_fd();
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
pub fn duplicate_cloexec(
    fd: impl AsFd,
    minimum: i32,
) -> std::io::Result<OwnedFd> {
    rustix::io::fcntl_dupfd_cloexec(fd.as_fd(), minimum)
        .map_err(std::io::Error::from)
}

/// Duplicate a descriptor to the lowest available descriptor number, with
/// close-on-exec set.
pub fn duplicate_fd(fd: impl AsFd) -> std::io::Result<OwnedFd> {
    rustix::io::fcntl_dupfd_cloexec(fd.as_fd(), 0).map_err(std::io::Error::from)
}

/// Duplicate a descriptor with close-on-exec and return an owning Rust file.
pub fn duplicate_file(fd: impl AsFd) -> std::io::Result<std::fs::File> {
    rustix::io::fcntl_dupfd_cloexec(fd.as_fd(), 0)
        .map(std::fs::File::from)
        .map_err(std::io::Error::from)
}

/// Create a pipe and return both owned ends.
pub fn pipe() -> std::io::Result<(OwnedFd, OwnedFd)> {
    rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)
        .map_err(std::io::Error::from)
}

/// Write the complete buffer to a descriptor.
pub fn write_all(fd: impl AsFd, mut bytes: &[u8]) -> std::io::Result<()> {
    let fd = fd.as_fd();
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
pub fn write_once(fd: impl AsFd, bytes: &[u8]) -> std::io::Result<usize> {
    let fd = fd.as_fd();
    loop {
        match rustix::io::write(fd, bytes) {
            Ok(count) => return Ok(count),
            Err(rustix::io::Errno::INTR) => {}
            Err(error) => return Err(std::io::Error::from(error)),
        }
    }
}

/// Read exactly `length` bytes from a descriptor.
pub fn read_exact(fd: impl AsFd, length: usize) -> std::io::Result<Vec<u8>> {
    let fd = fd.as_fd();
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
pub fn read_to_end(fd: impl AsFd) -> std::io::Result<Vec<u8>> {
    let fd = fd.as_fd();
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
pub fn read_once(
    fd: impl AsFd,
    bytes: &mut [u8],
) -> std::io::Result<usize> {
    rustix::io::read(fd.as_fd(), bytes).map_err(std::io::Error::from)
}

/// Copy bytes from one pipe to another without consuming the source.
#[cfg(target_os = "linux")]
pub fn tee(
    fd_in: impl AsFd,
    fd_out: impl AsFd,
    length: usize,
) -> std::io::Result<usize> {
    rustix::pipe::tee(
        fd_in.as_fd(),
        fd_out.as_fd(),
        length,
        rustix::pipe::SpliceFlags::empty(),
    )
    .map_err(std::io::Error::from)
}

/// Add or remove nonblocking mode on a descriptor.
pub fn set_nonblocking(
    fd: impl AsFd,
    enabled: bool,
) -> std::io::Result<()> {
    let fd = fd.as_fd();
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
    rustix::runtime::exit_group(status)
}

pub const BAD_DESCRIPTOR: i32 = rustix::io::Errno::BADF.raw_os_error();
pub const PIPE_BUFFER: usize = rustix::pipe::PIPE_BUF;

pub fn is_bad_descriptor_error(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(BAD_DESCRIPTOR)
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
pub fn parent_process_id() -> i32 {
    rustix::process::Pid::as_raw(rustix::process::getppid())
}

#[inline]
pub fn current_process_id() -> i32 {
    rustix::process::getpid().as_raw_pid()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForkResult {
    Child,
    Parent(i32),
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
        Ok(ForkResult::Parent(pid))
    }
}

pub fn current_process_group() -> i32 {
    // SAFETY: `getpgrp` takes no arguments and cannot fail. Keep the raw
    // result because a process group inherited from outside a fresh PID
    // namespace is reported as zero until the shell establishes its own;
    // PID newtypes deliberately cannot represent that observable state.
    unsafe { libc::getpgrp() }
}

pub fn foreground_process_group(fd: impl AsFd) -> std::io::Result<i32> {
    // SAFETY: the kernel validates the descriptor and returns only an
    // integer process-group id. As above, zero is a meaningful transient
    // result in a PID namespace and must not be rejected by a PID newtype.
    let group = unsafe { libc::tcgetpgrp(fd.as_fd().as_raw_fd()) };
    if group < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(group)
    }
}

pub fn set_process_group(pid: i32, group: i32) -> std::io::Result<()> {
    if pid < 0 || group < 0 {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    rustix::process::setpgid(
        rustix::process::Pid::from_raw(pid),
        rustix::process::Pid::from_raw(group),
    )
    .map_err(std::io::Error::from)
}

pub fn set_foreground_process_group(
    fd: impl AsFd,
    group: i32,
) -> std::io::Result<()> {
    if group < 0 {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    // SAFETY: both arguments are scalar values validated above. Passing
    // group zero preserves the kernel's ESRCH result during teardown in a
    // fresh PID namespace, matching the underlying terminal API exactly.
    if unsafe { libc::tcsetpgrp(fd.as_fd().as_raw_fd(), group) } < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn wait_status_is_exited(status: i32) -> bool {
    std::process::ExitStatus::from_raw(status).code().is_some()
}

pub fn wait_status_exit_code(status: i32) -> i32 {
    std::process::ExitStatus::from_raw(status).code().unwrap_or(0)
}

pub fn wait_status_stop_signal(status: i32) -> i32 {
    std::process::ExitStatus::from_raw(status)
        .stopped_signal()
        .unwrap_or(0)
}

pub fn wait_status_term_signal(status: i32) -> i32 {
    std::process::ExitStatus::from_raw(status).signal().unwrap_or(0)
}

pub fn wait_status_core_dumped(status: i32) -> bool {
    std::process::ExitStatus::from_raw(status).core_dumped()
}

pub fn signal_description(signal: i32) -> Vec<u8> {
    // SAFETY: `strsignal` returns a process-owned terminated string for a
    // signal number; copy it before returning.
    let description = unsafe { libc::strsignal(signal) };
    if description.is_null() {
        signal.to_string().into_bytes()
    } else {
        // SAFETY: a non-null `strsignal` result is terminated.
        unsafe { CStr::from_ptr(description) }.to_bytes().to_vec()
    }
}

/// Wait for any child. `None` is the successful nonblocking "not yet" case.
pub fn wait_for_any_child(
    nonblocking: bool,
    report_stopped: bool,
) -> std::io::Result<Option<(i32, i32)>> {
    let mut options = rustix::process::WaitOptions::empty();
    if nonblocking {
        options.insert(rustix::process::WaitOptions::NOHANG);
    }
    if report_stopped {
        options.insert(rustix::process::WaitOptions::UNTRACED);
    }
    rustix::process::wait(options)
        .map(|result| result.map(|(pid, status)| (pid.as_raw_pid(), status.as_raw())))
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
            let pid = rustix::process::Pid::from_raw(pid)
                .expect("fork returned a positive child process id");
            let status = loop {
                match rustix::process::waitpid(
                    Some(pid),
                    rustix::process::WaitOptions::empty(),
                ) {
                    Ok(Some((_, status))) => break status,
                    Ok(None) => continue,
                    Err(rustix::io::Errno::INTR) => continue,
                    Err(error) => return Err(std::io::Error::from(error)),
                }
            };
            Ok(if let Some(code) = status.exit_status() {
                code
            } else {
                128 + status.terminating_signal().unwrap_or(0)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    #[test]
    fn os_error_text_omits_rusts_numeric_suffix() {
        let error = std::io::Error::from(rustix::io::Errno::NOENT);
        let message = os_error_message(&error);
        assert!(!message.contains("(os error"));
        assert!(!message.is_empty());
    }

    #[test]
    fn temporary_files_are_unique_owned_and_private() {
        let mut template = std::env::temp_dir();
        template.push(format!("nsh-platform-test-{}-XXXXXX", std::process::id()));
        let (first, first_path) = create_temporary_file(template.as_os_str()).unwrap();
        let (second, second_path) = create_temporary_file(template.as_os_str()).unwrap();

        assert_ne!(first_path, second_path);
        assert_eq!(first.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(second.metadata().unwrap().permissions().mode() & 0o777, 0o600);

        drop(first);
        drop(second);
        std::fs::remove_file(first_path).unwrap();
        std::fs::remove_file(second_path).unwrap();
    }

    #[test]
    fn a_temporary_file_template_requires_six_placeholders() {
        let error = create_temporary_file(OsStr::new("/tmp/not-a-template")).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn duplicated_descriptor_outlives_source() {
        let source = std::fs::File::open("/dev/null").unwrap();
        let duplicate = duplicate_fd(&source).unwrap();
        drop(source);

        let mut byte = [0];
        assert_eq!(read_once(&duplicate, &mut byte).unwrap(), 0);
    }

    #[test]
    fn fd_changes_install_and_close_slots() {
        let (read, write) = pipe().unwrap();
        let status = run_in_child(move || {
            let source = duplicate_cloexec(&write, 10).unwrap();
            ProcessFdChanges::new([(7, Some(source)), (8, None)])
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
    fn fd_changes_reject_invalid_targets() {
        assert!(ProcessFdChanges::new([(-1, None)]).is_err());
        assert!(ProcessFdChanges::new([(4, None), (4, None)]).is_err());
    }
}

pub use rustix::fs::Access as AccessMode;
pub use rustix::process::{Gid as GroupId, Uid as UserId};
