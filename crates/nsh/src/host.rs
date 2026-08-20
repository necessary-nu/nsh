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
/// This is [`crate::signal_inbox::SignalSink`] -- the one that already exists
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
pub type SignalSink = &'static crate::signal_inbox::SignalSink;

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
/// gets correct foreground commands — those block waiting for a child, which needs
/// no handler — but it cannot notice a background job finishing between
/// commands, and the `wait` builtin's arm suspends on a signal that will
/// never be delivered. The wait design and this trait are one design, and
/// this is the seam between them.
///
/// **The forked child is not on this trait**, deliberately. Disposition
/// changes made after forking go through the platform boundary directly:
/// routing them would be an indirect call into embedder code made in a
/// forked child, and under [`NoHost`] a
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

    /// May the shell take the terminal and the process group?
    ///
    /// This is `set -m`, and it is three operations on the *host's*
    /// process: `setpgid(0, rootpid)`, `tcsetpgrp`, and on the way there
    /// possibly a `killpg(0, SIGTTIN)` that stops the host and every
    /// sibling with it. A frontend says yes, because job control is what a
    /// shell is for; anything else says no, and `set -m` in an ungranted
    /// shell is quietly ineffective.
    ///
    /// **Quietly, and that is the answer to `docs/api-design.md` §11.5's
    /// open question.** It asked whether the grant belongs on the builder
    /// and whether an ungranted `set -m` should warn the way `can't access
    /// tty` does. It belongs here rather than on the builder because it is
    /// the same kind of thing as the other three -- a power over a process
    /// the library did not create -- and a second gate in a second place
    /// would let the two disagree. It is silent because `set -m` is
    /// executed by the *script*, not by the embedder: `optschanged`
    /// reaches it from `poplocalvars` restoring a `local -`, so a warning
    /// would fire on a line no one wrote, and dash itself is silent when
    /// it cannot get the tty.
    fn may_control_terminal(&mut self) -> bool;
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

    fn may_control_terminal(&mut self) -> bool {
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
/// This is what the command-line frontend gives its [`crate::Shell`], and it
/// is not the library acting on its own authority: selecting this host says
/// that the process is the shell. An embedder whose program is something else wants
/// [`NoHost`], which is what [`crate::builder::Builder`] defaults to.
pub struct ProcessHost;

impl Host for ProcessHost {
    /// Nothing to keep. This host is in the same crate as the inbox and
    /// reaches it through [`crate::signal_inbox::signals`]; a host outside the
    /// crate has to hold the handle it is given here, because
    /// [`SignalSink::raise`] is the only thing its handler may call.
    fn attach(&mut self, _sink: SignalSink) {}

    fn signal(&mut self, signal: Signal) -> std::io::Result<Disposition> {
        nsh_platform::signal_action(signal.platform()).map(|action| match action {
            nsh_platform::SignalAction::Default => Disposition::Default,
            nsh_platform::SignalAction::Ignore => Disposition::Ignore,
            nsh_platform::SignalAction::Catch => Disposition::Catch,
        })
    }

    fn set_signal(&mut self, signal: Signal, to: Disposition) -> std::io::Result<()> {
        let action = match to {
            Disposition::Default => nsh_platform::SignalAction::Default,
            Disposition::Ignore => nsh_platform::SignalAction::Ignore,
            Disposition::Catch => nsh_platform::SignalAction::Catch,
        };
        nsh_platform::install_signal_action(
            signal.platform(),
            action,
            crate::trap::mark_signal_pending,
        )
    }

    fn may_replace_process(&mut self) -> bool {
        true
    }

    fn may_control_terminal(&mut self) -> bool {
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
            h.signal(Signal::from(nsh_platform::interrupt_signal()))
                .unwrap(),
            Disposition::Default
        );
        assert!(
            h.set_signal(
                Signal::from(nsh_platform::interrupt_signal()),
                Disposition::Catch,
            )
            .is_ok()
        );
    }

    /// A host that writes down what it was asked for instead of doing it.
    ///
    /// It answers every query `Default`, like [`NoHost`], so the shell's
    /// own `S_RESET` path is what runs and the recorded asks are the ones
    /// dash would have made.
    #[derive(Clone)]
    struct Recorder(std::sync::Arc<std::sync::Mutex<Vec<(i32, Disposition)>>>);

    impl Recorder {
        fn new() -> Recorder {
            Recorder(std::sync::Arc::new(std::sync::Mutex::new(Vec::new())))
        }

        fn installs(&self) -> Vec<(i32, Disposition)> {
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

        fn may_control_terminal(&mut self) -> bool {
            false
        }
    }

    /// The parent-side entry point reaches the host, and the shell asks
    /// for its `SIGCHLD` handler while it is still being built -- which is
    /// the ask a host declining it is declining.
    #[test]
    fn the_parent_side_entry_point_asks_the_host() {
        let _g = crate::test_support::lock();
        let rec = Recorder::new();
        let mut shell = crate::context::Shell::builder()
            .host(rec.clone())
            .build()
            .unwrap();
        assert!(
            rec.installs()
                .contains(&(nsh_platform::child_signal().number(), Disposition::Catch,)),
            "the shell did not ask for a SIGCHLD handler: {:?}",
            rec.installs()
        );

        crate::trap::configure_signal(&mut shell, nsh_platform::termination_signal().into());
        assert!(
            rec.installs().contains(&(
                nsh_platform::termination_signal().number(),
                Disposition::Default,
            )),
            "setsignal did not route through the host: {:?}",
            rec.installs()
        );
    }

    /// `set -m` in an ungranted shell leaves the host's process group and
    /// terminal alone, and is silent about it.
    ///
    /// The observable is `jobctl`: `setjobctl` is the only thing that ever
    /// sets it, and the only thing that ever sets `ttyfd`, so a zero here
    /// is also `forkchild`'s handoff, `waitforjob`'s hand-back and `fg`'s
    /// not happening. The corpus cannot see this -- it has no controlling
    /// terminal, so `jobctl` stays 0 in both shells there -- which is why
    /// it is pinned here.
    #[test]
    fn set_m_without_a_grant_leaves_the_hosts_terminal_alone() {
        let _g = crate::test_support::lock();
        let mut shell = crate::context::Shell::builder().build().unwrap();
        shell.run(b"set -m").unwrap();
        // [spec:nsh:def:idiom.job-control-model]
        assert!(
            !shell.jobs.job_control,
            "an ungranted shell took job control"
        );
        assert!(shell.run(b"echo still running").is_ok());
    }

    /// The child-side entry point does not, and that is the whole point of
    /// it being a second entry point: under a host that installs nothing,
    /// a forked child that stopped ignoring `SIGINT` would take `^C` from
    /// the terminal along with the shell that backgrounded it.
    #[test]
    fn the_child_side_entry_point_does_not_ask_the_host() {
        let _g = crate::test_support::lock();
        let rec = Recorder::new();
        let mut shell = crate::context::Shell::builder()
            .host(rec.clone())
            .build()
            .unwrap();
        let before = rec.installs().len();
        crate::trap::configure_signal_in_child(
            &mut shell,
            nsh_platform::termination_signal().into(),
        );
        crate::trap::ignore_signal_in_child(&mut shell, nsh_platform::termination_signal().into());
        assert_eq!(
            rec.installs().len(),
            before,
            "a child-side call reached the host: {:?}",
            rec.installs()
        );
    }
}
