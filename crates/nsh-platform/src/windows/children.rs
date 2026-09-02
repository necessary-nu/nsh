//! Making a child process, and reaping it.
//!
//! `fork_process` is the only caller of the native clone, and most of
//! its length is what has to be true on both sides of it: a lock, so two
//! shell instances in one embedding process do not clone at once; a Job
//! Object, so a child tree dies with the child; a broker channel, made
//! before the clone because both processes need an end of it; and a
//! console reattachment in the copy, which inherits an address space but
//! not a console.
//!
//! The child table is here because it is what waiting reads. Windows
//! reports an exit code and nothing else, so a signalled child is
//! recognised by the encoded status the shell's own `send_signal` put
//! there, and the thread that created a child is the only one allowed to
//! reap it.

use std::collections::HashMap;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::atomic::Ordering as AtomicOrdering;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, FILETIME, HANDLE, HANDLE_FLAG_INHERIT,
    SetHandleInformation, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, FreeConsole};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, GetProcessTimes, ResumeThread, TerminateProcess,
    WaitForSingleObject,
};

use crate::{ChildStatus, ForkResult, ProcessId, Signal};

use super::*;

pub(super) struct ChildRecord {
    pub(super) process: OwnedHandle,
    pub(super) job: Option<OwnedHandle>,
    owner: std::thread::ThreadId,
}

pub(super) static CHILDREN: LazyLock<Mutex<HashMap<ProcessId, ChildRecord>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) static PROCESS_CLONE_LOCK: Mutex<()> = Mutex::new(());

pub(super) fn set_descriptor_inherit(fd: &impl AsDescriptor, inherit: bool) -> std::io::Result<()> {
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

pub(super) fn duplicate_owned_handle(raw: HANDLE, inherit: bool) -> std::io::Result<OwnedHandle> {
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

fn reap_ready_child(
    target: Option<ProcessId>,
) -> std::io::Result<Option<(ProcessId, ChildStatus)>> {
    let owner = std::thread::current().id();
    let _clone_guard = PROCESS_CLONE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let ready = {
        let children = CHILDREN
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        children.iter().find_map(|(&pid, child)| {
            if child.owner != owner || target.is_some_and(|target| target != pid) {
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

/// Wait for one named child, and for no other.
///
/// The caller says which process it is entitled to reap, so a process
/// this caller did not create keeps its status. `NotFound` means this
/// thread has no such child registered.
// [spec:nsh:req:embedding-safety.host-children-are-not-reaped]
pub fn wait_for_child(
    process: ProcessId,
    nonblocking: bool,
    _report_stopped: bool,
) -> std::io::Result<Option<(ProcessId, ChildStatus)>> {
    let owner = std::thread::current().id();
    loop {
        if let Some(child) = reap_ready_child(Some(process))? {
            return Ok(Some(child));
        }
        let is_ours = {
            let _clone_guard = PROCESS_CLONE_LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            CHILDREN
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&process)
                .is_some_and(|child| child.owner == owner)
        };
        if !is_ours {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "there is no such child process",
            ));
        }
        if nonblocking {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn wait_for_any_child(
    nonblocking: bool,
    _report_stopped: bool,
) -> std::io::Result<Option<(ProcessId, ChildStatus)>> {
    let owner = std::thread::current().id();
    loop {
        if let Some(child) = reap_ready_child(None)? {
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
            let Some((_, status)) = wait_for_child(pid, false, false)? else {
                continue;
            };
            return Ok(match status {
                ChildStatus::Exited(code) => i32::from(code),
                ChildStatus::Signaled { signal, .. } => 128 + signal.number(),
                ChildStatus::Stopped(signal) => 128 + signal.number(),
                ChildStatus::Continued => 0,
            });
        },
    }
}
