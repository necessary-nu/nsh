//! Signals on a host that does not have them.
//!
//! Windows delivers exactly two things a POSIX signal could be -- Ctrl+C
//! and Ctrl+Break, to a console control handler -- so everything else
//! here is a shell-level construction: a disposition per signal, a set
//! of pending ones, a block depth that defers delivery, and a console
//! handler installed only once anything asks for a disposition.
//!
//! Delivery therefore has three shapes. To this process, it is a direct
//! call of the shell's own handler. To another nsh process it is a
//! console event where one exists, and otherwise a termination whose
//! exit code encodes the signal number, which is how the waiting side
//! recognises a child as signalled rather than exited.

use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::time::Duration;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler};
use windows_sys::Win32::System::JobObjects::TerminateJobObject;
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, TerminateProcess,
};

use crate::{ProcessGroupId, ProcessId, ProcessTarget, Signal, SignalRequest};

use super::*;

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

pub(super) const SIGNAL_EXIT_BASE: u32 = 0xe000_0000;

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
