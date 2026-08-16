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
///
/// **What a host that grants no `SIGCHLD` handler gives up.** The shell
/// asks for [`Disposition::Catch`] on `SIGCHLD` while it is being built,
/// and everything it learns about a child finishing *without* being
/// blocked in `wait` comes from that handler. A host that declines still
/// gets correct foreground commands — those block in `wait3`, which needs
/// no handler — but it cannot notice a background job finishing between
/// commands, and the `wait` builtin's arm suspends on a signal that will
/// never be delivered. The wait design and this trait are one design, and
/// this is the seam between them.
///
/// **The forked child is not on this trait**, deliberately. The twelve
/// disposition changes the library makes *after* forking go to libc
/// directly: routing them would be an indirect call into embedder code
/// made in a forked — sometimes vforked — child, and under [`NoHost`] a
/// background job would go on taking `^C` because its `ignoresig` had been
/// quietly dropped. `trap::Via` carries the argument;
/// [dec:nsh:fork-child-is-a-terminus] carries the rule.
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

/// The host for a program that *is* the shell.
///
/// Exactly what dash does, because that is the whole specification: a
/// query is `sigaction(signo, NULL, &act)`, an install is `sigaction` with
/// `sigfillset` on `sa_mask` and `sa_flags = 0`, and `exec` is granted.
///
/// Both halves of that install are load bearing rather than incidental.
/// No `SA_RESTART` is why every interruptible syscall returns `EINTR` when
/// a signal arrives, which is why the shell always has a poll site to
/// reach; the full mask is why a handler cannot be re-entered by a second
/// signal while it is storing.
///
/// This is what [`crate::shellmain::main_fn`] gives the shell it builds,
/// and it is not the library acting on its own authority: `main_fn` is the
/// port of `main()`, so a caller of it has already said that this process
/// is the shell. An embedder whose program is something else wants
/// [`NoHost`], which is what [`crate::builder::Builder`] defaults to.
pub struct ProcessHost;

impl Host for ProcessHost {
    /// Nothing to keep. This host is in the same crate as the inbox and
    /// reaches it through [`crate::siginbox::signals`]; a host outside the
    /// crate has to hold the handle it is given here, because
    /// [`SignalSink::raise`] is the only thing its handler may call.
    fn attach(&mut self, _sink: SignalSink) {}

    fn signal(&mut self, signal: Signal) -> std::io::Result<Disposition> {
        unsafe {
            let mut act: libc::sigaction = core::mem::zeroed();
            if libc::sigaction(signal.number(), core::ptr::null(), &mut act) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(crate::trap::disposition_of(&act))
        }
    }

    fn set_signal(&mut self, signal: Signal, to: Disposition) -> std::io::Result<()> {
        unsafe {
            let mut act: libc::sigaction = core::mem::zeroed();
            act.sa_sigaction = match to {
                /* The library's own handler, which is
                 * `SignalSink::raise`'s trampoline. A host outside this
                 * crate installs its own and calls `raise` from it. */
                Disposition::Catch => crate::trap::onsig as *const () as usize,
                Disposition::Ignore => libc::SIG_IGN,
                Disposition::Default => libc::SIG_DFL,
            };
            act.sa_flags = 0;
            libc::sigfillset(&mut act.sa_mask);
            if libc::sigaction(signal.number(), &act, core::ptr::null_mut()) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        }
    }

    fn may_replace_process(&mut self) -> bool {
        true
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

    /// A host that writes down what it was asked for instead of doing it.
    ///
    /// It answers every query `Default`, like [`NoHost`], so the shell's
    /// own `S_RESET` path is what runs and the recorded asks are the ones
    /// dash would have made.
    #[derive(Clone)]
    struct Recorder(std::sync::Arc<std::sync::Mutex<Vec<(libc::c_int, Disposition)>>>);

    impl Recorder {
        fn new() -> Recorder {
            Recorder(std::sync::Arc::new(std::sync::Mutex::new(Vec::new())))
        }

        fn installs(&self) -> Vec<(libc::c_int, Disposition)> {
            self.0.lock().unwrap().clone()
        }
    }

    impl Host for Recorder {
        fn attach(&mut self, _sink: SignalSink) {}

        fn signal(&mut self, _signal: Signal) -> std::io::Result<Disposition> {
            Ok(Disposition::Default)
        }

        fn set_signal(&mut self, signal: Signal, to: Disposition) -> std::io::Result<()> {
            self.0.lock().unwrap().push((signal.number(), to));
            Ok(())
        }

        fn may_replace_process(&mut self) -> bool {
            false
        }
    }

    /// The parent-side entry point reaches the host, and the shell asks
    /// for its `SIGCHLD` handler while it is still being built -- which is
    /// the ask a host declining it is declining.
    #[test]
    fn the_parent_side_entry_point_asks_the_host() {
        unsafe {
            let _g = crate::testutil::lock();
            let rec = Recorder::new();
            let mut sh = crate::context::Shell::builder()
                .host(rec.clone())
                .build()
                .unwrap();
            assert!(
                rec.installs()
                    .contains(&(libc::SIGCHLD, Disposition::Catch)),
                "the shell did not ask for a SIGCHLD handler: {:?}",
                rec.installs()
            );

            crate::trap::setsignal(&mut sh, libc::SIGTERM);
            assert!(
                rec.installs()
                    .contains(&(libc::SIGTERM, Disposition::Default)),
                "setsignal did not route through the host: {:?}",
                rec.installs()
            );
        }
    }

    /// The child-side entry point does not, and that is the whole point of
    /// it being a second entry point: under a host that installs nothing,
    /// a forked child that stopped ignoring `SIGINT` would take `^C` from
    /// the terminal along with the shell that backgrounded it.
    #[test]
    fn the_child_side_entry_point_does_not_ask_the_host() {
        unsafe {
            let _g = crate::testutil::lock();
            let rec = Recorder::new();
            let mut sh = crate::context::Shell::builder()
                .host(rec.clone())
                .build()
                .unwrap();
            let before = rec.installs().len();
            crate::trap::setsignal_in_child(&mut sh, libc::SIGTERM);
            crate::trap::ignoresig_in_child(&mut sh, libc::SIGTERM);
            assert_eq!(
                rec.installs().len(),
                before,
                "a child-side call reached the host: {:?}",
                rec.installs()
            );
        }
    }
}
