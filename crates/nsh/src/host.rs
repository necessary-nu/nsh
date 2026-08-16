//! The host seam: what the library will not do on its own authority.
//!
//! `docs/api-design.md` §5.4, and [dec:nsh:host-owns-signals] and
//! [dec:nsh:host-owns-the-process] as a type. A shell that *is* the
//! process may install signal handlers and replace its own image; a
//! library linked into someone else's process may not, because both are
//! visible to every other part of that process.
//!
//! Not every such power becomes a method. Ending the process was answered
//! with an absence -- `exitshell` returns a status and the caller decides
//! -- because after the status comes back there is nothing left to grant.
//! What is here is what the shell genuinely cannot finish without.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::signames::NSIG;
use crate::status::Signal;

/// What a signal does.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Disposition {
    /// `SIG_DFL`.
    Default,
    /// `SIG_IGN`.
    Ignore,
    /// Deliver to this shell, through the [`SignalSink`] it was given.
    Catch,
}

/// The shell's signal inbox.
///
/// Cheap to clone, safe to hold across threads, and safe to touch from a
/// signal handler. A shell polls it where dash reads `pending_sig`.
#[derive(Clone)]
pub struct SignalSink {
    inbox: Arc<SignalInbox>,
}

impl SignalSink {
    /// Record that `signal` was delivered.
    ///
    /// The only method a signal handler may call, and the reason the type
    /// exists: one relaxed atomic store, no allocation, no lock, no
    /// reentrancy. Everything a handler is not allowed to do -- take a
    /// lock, touch the shell, write a diagnostic -- is impossible from
    /// here rather than merely documented as forbidden.
    ///
    /// A signal outside the table is dropped. That is the same answer the
    /// kernel would have given, since nothing can deliver a number this
    /// platform has no slot for.
    pub fn raise(&self, signal: Signal) {
        if let Ok(i) = usize::try_from(signal.number())
            && i < NSIG
        {
            self.inbox.pending[i].store(true, Ordering::Relaxed);
            self.inbox.any.store(true, Ordering::Relaxed);
        }
    }
}

/// The receiving end of a [`SignalSink`].
///
/// The shell holds this; the host holds the sink. Split so that the half
/// a signal handler touches has exactly one method on it.
pub(crate) struct SignalInbox {
    /// Set by any `raise`, so the common "nothing arrived" poll is one
    /// load rather than `NSIG` of them.
    any: AtomicBool,
    pending: Vec<AtomicBool>,
}

impl SignalInbox {
    pub(crate) fn new() -> Arc<SignalInbox> {
        Arc::new(SignalInbox {
            any: AtomicBool::new(false),
            pending: (0..NSIG).map(|_| AtomicBool::new(false)).collect(),
        })
    }

    /// Whether any signal is waiting. `dash`'s `pending_sig` test.
    pub(crate) fn any_pending(&self) -> bool {
        self.any.load(Ordering::Relaxed)
    }

    /// Take one pending signal, clearing it. `None` when none is waiting.
    ///
    /// Lowest number first, which is only a tie-break: dash handles one
    /// signal per check too, and nothing in the shell depends on the order
    /// two simultaneous signals are seen in.
    pub(crate) fn take_pending(&self) -> Option<Signal> {
        if !self.any_pending() {
            return None;
        }
        self.any.store(false, Ordering::Relaxed);
        for (i, slot) in self.pending.iter().enumerate() {
            if slot.swap(false, Ordering::Relaxed) {
                /* Another signal may have arrived while this ran, so the
                 * summary flag goes back up rather than staying clear. */
                if self.pending.iter().skip(i + 1).any(|s| s.load(Ordering::Relaxed)) {
                    self.any.store(true, Ordering::Relaxed);
                }
                return Some(Signal::from_raw(i as libc::c_int));
            }
        }
        None
    }
}

/// A [`SignalSink`] over an inbox.
pub(crate) fn sink_for(inbox: &Arc<SignalInbox>) -> SignalSink {
    SignalSink {
        inbox: Arc::clone(inbox),
    }
}

/// What the library will not do on its own authority.
///
/// Implemented by whoever owns the process. `crates/nsh-cli` implements it
/// by doing exactly what dash does; an embedder that implements nothing
/// gets [`NoHost`] -- a shell that installs no handler and refuses to
/// `exec`.
///
/// No method takes a [`crate::context::Shell`], and that is load bearing
/// rather than an omission: it makes the host a leaf, so
/// `self.host.set_signal(…)` is a field-disjoint borrow inside a
/// `&mut self` method, and it makes re-entering the shell from a host
/// callback a compile error instead of a documented hazard.
pub trait Host: Send {
    /// Take the shell's signal inbox. Called once, from
    /// [`crate::builder::Builder::build`].
    ///
    /// A host that installs [`Disposition::Catch`] must keep this and must
    /// have its handler do nothing but [`SignalSink::raise`] -- the
    /// handler runs in signal context, where nothing else here is safe.
    fn attach(&mut self, sink: SignalSink);

    /// What is installed for `signal` right now.
    ///
    /// The shell needs this, not only `set_signal`: a signal that was
    /// already ignored when the shell started stays ignored and cannot be
    /// trapped (dash's `S_HARD_IGN`), and that rule cannot be reproduced
    /// without reading the inherited disposition.
    fn signal(&mut self, signal: Signal) -> std::io::Result<Disposition>;

    /// Install a disposition the shell has asked for.
    ///
    /// The shell decides *which*; the host performs it. To be dash the
    /// host must install with all signals blocked in the handler's mask
    /// and no flags -- `sigfillset` on `sa_mask`, `sa_flags = 0`.
    fn set_signal(&mut self, signal: Signal, to: Disposition) -> std::io::Result<()>;

    /// May the shell replace the process image?
    ///
    /// `exec cmd` `execve`s in place. In a frontend that is the point; in
    /// a library it destroys the host. A host that refuses gets the same
    /// diagnostic and status a failed `exec` produces.
    fn may_replace_process(&mut self) -> bool;
}

/// The default host: a shell that touches nothing outside itself.
///
/// Every method is the refusal, and none of them fail -- reading a
/// disposition answers [`Disposition::Default`] because that is what a
/// process that has installed nothing has, and installing one is quietly
/// dropped rather than erroring, because a library shell running a script
/// that sets a trap should run the script, not abort on the trap.
///
/// Under `NoHost`, `trap` still records its action and the shell still
/// runs the EXIT trap, but no signal is ever caught, because nothing
/// installed a handler to catch it.
pub struct NoHost;

impl Host for NoHost {
    fn attach(&mut self, _sink: SignalSink) {}

    fn signal(&mut self, _signal: Signal) -> std::io::Result<Disposition> {
        Ok(Disposition::Default)
    }

    fn set_signal(&mut self, _signal: Signal, _to: Disposition) -> std::io::Result<()> {
        Ok(())
    }

    fn may_replace_process(&mut self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_raised_signal_comes_back_once() {
        let inbox = SignalInbox::new();
        let sink = sink_for(&inbox);
        assert!(!inbox.any_pending());
        sink.raise(Signal::from_raw(libc::SIGINT));
        assert_eq!(inbox.take_pending(), Some(Signal::from_raw(libc::SIGINT)));
        assert_eq!(inbox.take_pending(), None);
    }

    #[test]
    fn two_signals_both_come_back() {
        let inbox = SignalInbox::new();
        let sink = sink_for(&inbox);
        sink.raise(Signal::from_raw(libc::SIGTERM));
        sink.raise(Signal::from_raw(libc::SIGINT));
        let mut got = vec![
            inbox.take_pending().unwrap().number(),
            inbox.take_pending().unwrap().number(),
        ];
        got.sort();
        assert_eq!(got, vec![libc::SIGINT, libc::SIGTERM]);
        assert_eq!(inbox.take_pending(), None);
    }

    /// A number the platform has no slot for cannot be delivered, so
    /// dropping it loses nothing a real signal could have carried.
    #[test]
    fn a_signal_outside_the_table_is_dropped() {
        let inbox = SignalInbox::new();
        let sink = sink_for(&inbox);
        sink.raise(Signal::from_raw(NSIG as libc::c_int + 10));
        sink.raise(Signal::from_raw(-1));
        assert!(!inbox.any_pending());
    }

    #[test]
    fn the_default_host_refuses_to_replace_the_process() {
        assert!(!NoHost.may_replace_process());
    }
}
