//! Signals: naming them, sending them, and the dispositions and mask the
//! shell keeps around them.
//!
//! The numbers are functions rather than constants so that no `SIG*` name
//! reaches the shell crate, and the disposition table is a static array
//! of handler pointers because a signal handler cannot consult anything
//! richer. The two guards over that -- the trampoline that every
//! installed handler goes through, and `BlockedSignals`, which restores
//! the caller's mask when it is dropped -- are the reason the raw signal
//! set never leaves this module.

use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

use super::*;

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
// [spec:dash:sem:trap.sigblockall-fn]
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
