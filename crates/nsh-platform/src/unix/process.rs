//! The life of a process: the state it inherits, forking, replacing its
//! image, the attributes a script can change, and reaping children.
//!
//! It opens with the two pieces of inherited state Rust's runtime
//! destroys before `main` -- an ignored SIGPIPE and closed standard
//! descriptors -- because a shell has to present what it was given
//! rather than what the runtime preferred, and the only place to capture
//! them is a constructor that runs earlier still.
//!
//! The rest is what a process is from the shell's side: its arguments
//! and environment, `fork`, `execve`, `_exit`, `wait`, the process
//! groups job control moves it between, and the three attributes
//! builtins set -- `ulimit`, `umask` and the clocks `times` reports.

use std::ffi::{CString, OsString};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::ExitStatusExt as _;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering as AtomicOrdering};

use super::*;

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

/// Snapshot the process arguments in their native representation.
pub fn process_arguments() -> Vec<OsString> {
    std::env::args_os().collect()
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

/// Replace the current process image.
///
/// The pointer arrays required by `execve` are assembled entirely inside the
/// platform crate from validated, live C strings. Success never returns; the
/// returned value is therefore always the operating-system error.
pub fn execute_program(program: crate::ProgramImage) -> std::io::Error {
    let crate::ProgramImage {
        path,
        arguments,
        environment,
    } = program;
    let path = match CString::new(path.as_os_str().as_bytes()) {
        Ok(path) => path,
        Err(_) => return std::io::Error::from(std::io::ErrorKind::InvalidInput),
    };
    let arguments: Vec<CString> = match arguments
        .iter()
        .map(|argument| CString::new(argument.as_bytes()))
        .collect()
    {
        Ok(arguments) => arguments,
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
    let mut argument_pointers: Vec<*const u8> = arguments
        .iter()
        .map(|argument| argument.as_ptr().cast())
        .collect();
    argument_pointers.push(std::ptr::null());
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
            argument_pointers.as_ptr().cast(),
            environment_pointers.as_ptr().cast(),
        );
    }
    std::io::Error::last_os_error()
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

/// Replace the process file-creation mask and return its previous value.
pub fn replace_creation_mask(mask: u32) -> u32 {
    // SAFETY: `umask` accepts every mode bit pattern and returns the prior mask.
    unsafe { libc::umask(mask as libc::mode_t) as u32 }
}

/// Read the process file-creation mask.
///
/// POSIX gives no way to ask: `umask(2)` installs and reports in one
/// call, so the portable answer is to install zero and put back what
/// came out -- and between those two syscalls every thread in the
/// process creates files with nothing masked off. Deferring interrupts
/// across the pair, which `builtins::umask` does, stops a signal
/// stranding the zero; it does not stop a sibling thread.
///
/// Linux 4.7 and later publish the mask on the `Umask:` line of
/// `/proc/self/status`, which is a read and leaves nothing behind, so
/// that is asked first. The dance is what answers when the line is not
/// there, and which of the two runs is decided by whether this host
/// produced the line -- not by the target it was built for, since a
/// Linux kernel older than 4.7 and a host with no `/proc` mounted both
/// compile as Linux and neither can be asked.
pub fn creation_mask() -> u32 {
    if let Some(published) = published_creation_mask() {
        return published;
    }
    let mask = replace_creation_mask(0);
    replace_creation_mask(mask);
    mask
}

/// The mask as the kernel publishes it, or `None` on a host that does
/// not publish it.
///
/// A host either carries the line or never will, so a host that does
/// not is remembered: rediscovering it would pay, on every call, the
/// open the fallback exists to avoid.
fn published_creation_mask() -> Option<u32> {
    const UNKNOWN: u8 = 0;
    const PUBLISHED: u8 = 1;
    const ABSENT: u8 = 2;
    static KERNEL_PUBLISHES_MASK: AtomicU8 = AtomicU8::new(UNKNOWN);

    if KERNEL_PUBLISHES_MASK.load(AtomicOrdering::Relaxed) == ABSENT {
        return None;
    }
    let published = std::fs::read(PROCESS_STATUS_PATH)
        .ok()
        .and_then(|status| published_mask_field(&status));
    KERNEL_PUBLISHES_MASK.store(
        if published.is_some() {
            PUBLISHED
        } else {
            ABSENT
        },
        AtomicOrdering::Relaxed,
    );
    published
}

/// The `Umask:` field of a `/proc/<pid>/status` body. The value is
/// octal, and reading it as anything else agrees with the kernel only
/// for the masks whose digits happen to be below 8.
fn published_mask_field(status: &[u8]) -> Option<u32> {
    let field = status
        .split(|byte| *byte == b'\n')
        .find_map(|line| line.strip_prefix(b"Umask:".as_slice()))?
        .trim_ascii();
    if field.is_empty() || !field.iter().all(|byte| (b'0'..=b'7').contains(byte)) {
        return None;
    }
    u32::from_str_radix(std::str::from_utf8(field).ok()?, 8).ok()
}

/// Where the kernel publishes what it knows about this process.
const PROCESS_STATUS_PATH: &str = "/proc/self/status";

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

pub fn environment_text(name: &str) -> Option<String> {
    std::env::var(name).ok()
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

fn wait_options(nonblocking: bool, report_stopped: bool) -> rustix::process::WaitOptions {
    let mut options = rustix::process::WaitOptions::empty();
    if nonblocking {
        options.insert(rustix::process::WaitOptions::NOHANG);
    }
    if report_stopped {
        options.insert(rustix::process::WaitOptions::UNTRACED);
    }
    options
}

/// Wait for one named child, and for no other.
///
/// The caller says which process it is entitled to reap, so a process
/// this caller did not fork keeps its status and stays waitable by
/// whoever did. `None` is the successful nonblocking "not yet" case, and
/// `NotFound` means the operating system has no such child of ours --
/// either it was never one or somebody else has already reaped it.
// [spec:nsh:req:embedding-safety.host-children-are-not-reaped]
pub fn wait_for_child(
    process: ProcessId,
    nonblocking: bool,
    report_stopped: bool,
) -> std::io::Result<Option<(ProcessId, ChildStatus)>> {
    let identity = rustix::process::Pid::from_raw(raw_process_id(process)?)
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    rustix::process::waitpid(Some(identity), wait_options(nonblocking, report_stopped))
        .map(|result| result.map(|(_, status)| (process, decode_child_status(status))))
        .map_err(std::io::Error::from)
}

/// Wait for any child. `None` is the successful nonblocking "not yet" case.
///
/// Right for a process the caller owns entirely, and wrong for one it
/// shares: it takes whichever child the operating system offers, which in
/// an embedding host is somebody else's to reap. [`wait_for_child`] is
/// what the shell uses; this remains for a caller that is the whole
/// process.
pub fn wait_for_any_child(
    nonblocking: bool,
    report_stopped: bool,
) -> std::io::Result<Option<(ProcessId, ChildStatus)>> {
    rustix::process::wait(wait_options(nonblocking, report_stopped))
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
            let status = loop {
                match wait_for_child(pid, false, false) {
                    Ok(Some((_, status))) => break status,
                    Ok(None) => continue,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error),
                }
            };
            Ok(match status {
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

    /// The environment name that tells this check it is the copy of
    /// itself running in a process of its own, and the name it re-enters
    /// itself by.
    const OWNED_PROCESS: &str = "NSH_CREATION_MASK_OWNS_THE_PROCESS";
    /// Named without its module path, which the filter does not need
    /// and which moves when the file does; `1 passed` below is what
    /// makes a filter that matched nothing a failure rather than a pass.
    const THIS_CHECK: &str = "reporting_the_creation_mask_never_unmasks_the_process";

    /// Reporting the file-creation mask must not install one. The report
    /// POSIX forces -- install zero, put back what came out -- leaves
    /// the whole process unmasked for the width of two syscalls, and
    /// anything a sibling thread creates in that window is created with
    /// nothing taken off: 0o666 for a redirection that meant 0o644,
    /// 0o777 for a directory. The bill for it was 201 EACCES failures
    /// in 2,000 runs of a completion test that only wanted a temporary
    /// directory.
    ///
    /// Watching for it takes a mask worth removing and a thread making
    /// files while another reports it, and installing a mask is a write
    /// to state every test in this binary shares. So the check re-enters
    /// itself in a process of its own and does it there, where the mask,
    /// the files and the answer are nobody else's. Every host has a mask
    /// and every host can be given one, so there is no host this cannot
    /// be measured on -- only two answers, one for a kernel that
    /// publishes the mask and one for a kernel that does not.
    #[test]
    fn reporting_the_creation_mask_never_unmasks_the_process() {
        if std::env::var_os(OWNED_PROCESS).is_some() {
            watch_a_report_from_inside_this_process();
        } else {
            let binary = std::env::current_exe().expect("the check knows its own binary");
            let run = std::process::Command::new(binary)
                .args([THIS_CHECK, "--test-threads", "1"])
                .env(OWNED_PROCESS, "1")
                .output()
                .expect("a process of this check's own");
            let transcript = String::from_utf8_lossy(&run.stdout).into_owned()
                + &String::from_utf8_lossy(&run.stderr);
            assert!(run.status.success(), "{transcript}");
            assert!(
                transcript.contains("1 passed"),
                "the check took a process of its own and ran nothing in it: {transcript}"
            );
        }
    }

    /// The watch, which only a process that owns its own mask may run.
    /// One thread creates files asking for every permission bit while
    /// another reports the mask, and a report that installed zero is
    /// caught as a file carrying bits the mask should have taken off.
    fn watch_a_report_from_inside_this_process() {
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
        use std::sync::atomic::{AtomicBool, AtomicU64};

        /// Wide enough that a file made under it and one made under
        /// nothing cannot be confused.
        const WATCHED: u32 = 0o077;
        /// Files to see created before the answer is called in. Bounding
        /// the watch by what it witnessed rather than by how many
        /// reports it made is what keeps a starved thread from turning
        /// into a pass.
        const WITNESSES: u64 = 200;
        /// A stop for a host where the files cannot be made at all, so
        /// the failure is the count below rather than a hang.
        const REPORT_LIMIT: u64 = 1_000_000;

        replace_creation_mask(WATCHED);

        let stop = AtomicBool::new(false);
        let unmasked = AtomicU64::new(0);
        let created = AtomicU64::new(0);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let directory = std::env::temp_dir();
                let mut serial = 0_u64;
                while !stop.load(AtomicOrdering::Relaxed) {
                    serial += 1;
                    let path = directory.join(format!(
                        "nsh-creation-mask-watch-{}-{serial}",
                        std::process::id()
                    ));
                    let Ok(file) = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o666)
                        .open(&path)
                    else {
                        continue;
                    };
                    let mode = file
                        .metadata()
                        .expect("a file just created has a mode")
                        .permissions()
                        .mode();
                    drop(file);
                    let _ = std::fs::remove_file(&path);
                    created.fetch_add(1, AtomicOrdering::Relaxed);
                    if mode & WATCHED != 0 {
                        unmasked.fetch_add(1, AtomicOrdering::Relaxed);
                    }
                }
            });
            let mut reports = 0_u64;
            while created.load(AtomicOrdering::Relaxed) < WITNESSES && reports < REPORT_LIMIT {
                assert_eq!(creation_mask(), WATCHED);
                reports += 1;
            }
            stop.store(true, AtomicOrdering::Relaxed);
        });

        let created = created.load(AtomicOrdering::Relaxed);
        let unmasked = unmasked.load(AtomicOrdering::Relaxed);
        assert!(
            created >= WITNESSES,
            "only {created} files could be made to watch the mask with"
        );
        match published_creation_mask() {
            /* The kernel publishes it, so the report is a read and there
             * is no moment at which the mask is other than the one this
             * process installed. */
            Some(_) => assert_eq!(
                unmasked, 0,
                "{unmasked} of {created} files made while the mask was being \
                 reported carried bits 0o{WATCHED:o} should have taken off"
            ),
            /* Nothing to ask, so the report is the POSIX dance and the
             * window is what it costs. Saying so is what this check has
             * to say on such a host: one that quietly gained a way to
             * read the mask would otherwise go on reporting the price of
             * not having one. */
            None => assert!(
                unmasked > 0,
                "this host reports the mask by installing zero, yet none of \
                 the {created} files made while it did saw the process unmasked"
            ),
        }
    }

    /// The published field is octal, and a mask installed is the mask
    /// read back. Installing one is a write to state every test in this
    /// binary shares, so the check takes a process of its own to do it
    /// in rather than putting the old value back and hoping no sibling
    /// looked in between.
    #[test]
    fn an_installed_creation_mask_is_reported_back_whole() {
        let status = run_in_child(|| {
            /* 0o123 is 83; the same digits read as decimal are 123, so
             * a report that misreads the base cannot pass this. */
            replace_creation_mask(0o123);
            if creation_mask() != 0o123 {
                exit_immediately(2);
            }
            if creation_mask() != 0o123 {
                exit_immediately(3);
            }
            if replace_creation_mask(0o022) != 0o123 {
                exit_immediately(4);
            }
        })
        .unwrap();

        assert_eq!(status, 0);
    }
}
