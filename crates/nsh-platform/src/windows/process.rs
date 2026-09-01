//! The life of this process: what it was given, what a script may change
//! about it, and how it ends.
//!
//! The three entry points at the top are empty on purpose. Windows has
//! no runtime state for the shell to put back before `main` and no
//! coverage counters to flush across a clone, and answering "nothing to
//! do" here is what lets the shell call them unconditionally.
//!
//! What remains is what a process is from the shell's side: its
//! arguments and environment, its identity, the process groups job
//! control would move it between -- which this host does not have, so
//! they are answered rather than performed -- the attributes `ulimit`
//! and `umask` set, the clocks `times` reports, and `_exit`.

use std::ffi::{OsStr, OsString};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering as AtomicOrdering};

use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::Threading::{
    ExitProcess, GetCurrentProcess, GetCurrentProcessId, GetProcessTimes,
};

use crate::{ProcessGroupId, ProcessGroupState, ProcessId, ProcessSelector};

use super::*;

pub fn restore_shell_process_runtime_state() {}

pub fn flush_coverage_profile() {}

pub fn reset_coverage_counters() {}

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

pub fn environment_text(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

pub fn exit_immediately(status: i32) -> ! {
    // SAFETY: this is the Windows process-terminating primitive and never
    // returns into Rust with destructors skipped.
    unsafe { ExitProcess(status as u32) }
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

pub(super) static CHILD_USER_TICKS: AtomicU64 = AtomicU64::new(0);

pub(super) static CHILD_SYSTEM_TICKS: AtomicU64 = AtomicU64::new(0);

pub(super) fn filetime_ticks(value: FILETIME) -> u64 {
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
