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

/// The shell's signal inbox, as the host receives it.
///
/// This is [`crate::siginbox::SignalSink`] -- the one that already exists
/// -- and not a new type. `siginbox` is §5.3's sink already: it is what a
/// signal handler may touch and the only place it may touch it, and its
/// own module argues that process-wide storage is correct rather than a
/// compromise, because `onsig` is called with `signo` and nothing else and
/// so cannot know which `Shell` a signal was meant for.
///
/// A `&'static` therefore, rather than the sketch's cloneable `Arc`
/// handle: there is one inbox per process because there is one set of
/// dispositions per process, and an `Arc` would buy shareability that the
/// storage does not need.
pub type SignalSink = &'static crate::siginbox::SignalSink;

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
    fn the_default_host_refuses_to_replace_the_process() {
        assert!(!NoHost.may_replace_process());
    }

    /// The refusal is total: reading a disposition answers `Default`,
    /// because that is what a process which has installed nothing has.
    #[test]
    fn the_default_host_reports_every_signal_as_default() {
        let mut h = NoHost;
        assert_eq!(
            h.signal(Signal::from_raw(libc::SIGINT)).unwrap(),
            Disposition::Default
        );
        assert!(h.set_signal(Signal::from_raw(libc::SIGINT), Disposition::Catch).is_ok());
    }
}
