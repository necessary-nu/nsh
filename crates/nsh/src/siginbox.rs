//! The signal inbox: everything a signal handler touches, and the only
//! place it may touch it.
//!
//! `docs/api-design.md` §5.3 specifies this as a `SignalSink` the host is
//! handed at build time. What lives here is its storage and its
//! discipline; `public-api` gives it ownership (the `Arc`, `Host::attach`,
//! a field on `Shell`). The order is forced — `attach` needs `Shell` to
//! own a `Box<dyn Host>`, which needs the builder.
//!
//! **Why a process-wide `static` is not a compromise.** A disposition is
//! installed per *process* and `onsig` is called with `signo` and nothing
//! else, so it cannot know which `Shell` a signal was meant for. The
//! `Arc` §5.3 describes buys shareability, not per-shell-ness; §6 records
//! the inbox as a process-wide fact beside the locale and `getopt`. The
//! contents are atomics rather than `static mut`, so the shell reaches
//! them through a shared reference and the declarations stop counting
//! against `[dec:nsh:minimal-unsafe]`.
//!
//! **Relaxed is the right ordering, not the lazy one.** A signal is
//! delivered on the thread that was running, so the handler and the code
//! it interrupted are the same thread; atomic accesses to one location
//! from one thread are coherent whatever the ordering. Relaxed is what
//! `volatile sig_atomic_t` bought the C, which is what these were.

use core::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use crate::status::Signal;
use crate::trap::NSIG;

/// What `onsig` may read and write without a receiver.
pub struct SignalSink {
    /// `trap[n].is_some()`, mirrored so the handler can ask "is a trap
    /// set for N?" without reaching the table.
    ///
    /// The handler asks about exactly two signals — `SIGCHLD` and
    /// `SIGINT` — and both of dash's reads are presence tests rather than
    /// reads of the action. The array is NSIG-wide because the writers
    /// index by `signo`, not because the other 63 bits are read.
    ///
    /// Written only by [`crate::trap::TrapTable::set`], and only with
    /// signals blocked: see [`SignalsBlocked`].
    trapped: [AtomicBool; NSIG],
    /// Signals awaiting their trap action, indexed by `signo - 1`.
    caught: [AtomicBool; NSIG - 1],
    /// The most recently delivered trapped signal, or zero.
    pending: AtomicI32,
    /// Whether SIGCHLD has arrived since the last reap attempt.
    child_pending: AtomicBool,
    /// An untrapped SIGINT waiting for a synchronous poll site.
    interrupt_pending: AtomicBool,
}

static SINK: SignalSink = SignalSink {
    trapped: [const { AtomicBool::new(false) }; NSIG],
    caught: [const { AtomicBool::new(false) }; NSIG - 1],
    pending: AtomicI32::new(0),
    child_pending: AtomicBool::new(false),
    interrupt_pending: AtomicBool::new(false),
};

/// The process's signal inbox.
#[inline]
pub fn signals() -> &'static SignalSink {
    &SINK
}

impl SignalSink {
    #[inline]
    pub(crate) fn signal_pending(&self, signal: Signal) -> bool {
        self.caught[(signal.number() - 1) as usize].load(Ordering::Relaxed)
    }

    #[inline]
    pub(crate) fn set_signal_pending(&self, signal: Signal, pending: bool) {
        self.caught[(signal.number() - 1) as usize].store(pending, Ordering::Relaxed);
    }

    #[inline]
    pub(crate) fn pending_signal(&self) -> Option<Signal> {
        Signal::from_number(self.pending.load(Ordering::Relaxed))
    }

    #[inline]
    pub(crate) fn set_pending_signal(&self, signal: Option<Signal>) {
        self.pending
            .store(signal.map_or(0, Signal::number), Ordering::Relaxed);
    }

    #[inline]
    pub(crate) fn child_pending(&self) -> bool {
        self.child_pending.load(Ordering::Relaxed)
    }

    #[inline]
    pub(crate) fn set_child_pending(&self, pending: bool) {
        self.child_pending.store(pending, Ordering::Relaxed);
    }

    #[inline]
    pub(crate) fn interrupt_pending(&self) -> bool {
        self.interrupt_pending.load(Ordering::Relaxed)
    }

    #[inline]
    pub(crate) fn set_interrupt_pending(&self, pending: bool) {
        self.interrupt_pending.store(pending, Ordering::Relaxed);
    }

    /// Is a trap set for `signo`? The handler's question.
    ///
    /// "Set" is dash's `trap[signo] != NULL`, which includes the empty
    /// action that means *ignore* — the C's three states are `NULL`, `""`
    /// and an action, and this bit separates the first from the other
    /// two. A mirror keyed on "has a non-empty action" would answer
    /// differently for `trap '' INT`.
    #[inline]
    pub fn is_trapped(&self, signal: Signal) -> bool {
        self.trapped[signal.number() as usize].load(Ordering::Relaxed)
    }

    /// Publish `trap[signo].is_some()`.
    ///
    /// `pub(crate)` and called from one place, because a slot and its bit
    /// disagreeing is observable in both directions — see
    /// [`crate::trap::TrapTable::set`], which is that place.
    #[inline]
    pub(crate) fn set_trapped(&self, signo: usize, to: bool) {
        self.trapped[signo].store(to, Ordering::Relaxed);
    }

    /// Deliver a signal to the shell. The only thing a host's handler may
    /// do, and the whole of what it may do.
    ///
    /// A delivery stores into the inbox's caught, pending, child, and
    /// interrupt atomics. This is the name the host knows it by, so an
    /// embedder's handler needs no access to the crate's internals to be
    /// correct.
    ///
    /// Everything it does is async-signal-safe by construction: it reads
    /// two atomics, compares a pid, and stores. It does not allocate, does
    /// not take a lock, and does not unwind — the last one is a fix rather
    /// than a precaution, and `onsig`'s own comment records the SIGABRT it
    /// was.
    ///
    /// # Safety
    ///
    /// Call it from a signal handler and from nowhere else.
    #[inline]
    pub fn raise(&self, signal: Signal) {
        if signal == Signal::from(nsh_platform::child_signal()) {
            self.set_child_pending(true);
            if !self.is_trapped(signal) {
                return;
            }
        }

        self.set_signal_pending(signal, true);
        self.set_pending_signal(Some(signal));

        if signal == Signal::from(nsh_platform::interrupt_signal()) && !self.is_trapped(signal) {
            /* The handler stores, and the shell takes delivery at a poll
             * site it reaches on its own: an EINTR return or `dotrap`.
             * The platform installs handlers without SA_RESTART, matching
             * dash, so an interruptible syscall always supplies that poll. */
            self.set_interrupt_pending(true);
        }
    }
}

/// Every signal blocked, and the witness that they are.
///
/// **What this is for.** Writing `trap[n]` and its mirror bit is two
/// stores, and a handler that runs between them reads a pair the C never
/// produces: dash's `trap[signo]` is one pointer, so `onsig` sees either
/// the old value or the new one. Both halves of the disagreement are
/// observable, and in opposite senses, so there is no "safe side" to
/// write first:
///
/// * mirror says trapped, table says none — the `^C` is swallowed
///   (`dotrap` clears `gotsig` and finds no action), and for `SIGCHLD`
///   `pending_sig` is set, which makes `wait` answer `128 + SIGCHLD`.
/// * mirror says none, table says trapped — `intpending` is set and the
///   shell takes the interrupt instead of running the user's trap, and
///   the `SIGCHLD` trap misses a delivery.
///
/// `INTOFF`/`INTON` cannot do this job. They defer *taking* an interrupt;
/// they do not stop the handler running, and since `errors-are-values`
/// step F `INTON` is not a delivery point at all. Blocking is what makes
/// the pair atomic against delivery, and a signal blocked here is pending
/// in the kernel rather than lost.
///
/// `jobs::xtcsetpgrp` brackets `tcsetpgrp` the same way for the same
/// reason; this one restores the saved mask rather than clearing, so it
/// composes with a caller that had signals blocked already.
pub(crate) struct SignalsBlocked(nsh_platform::BlockedSignals);

impl SignalsBlocked {
    /// Block everything until the guard is dropped.
    ///
    /// Hoist it: one guard per `trap` command and one per fork reset, not
    /// one per slot.
    ///
    /// Not for the syscall count — `clear_traps` skips every slot without
    /// an action, so a shell with no traps would block zero times either
    /// way. For the region: `clear_traps` clears a slot, calls
    /// `setsignal`, and in the `simplecmd` case puts the action back, and
    /// a per-write guard makes each half atomic while leaving the shell
    /// observably untrapped across the pair.
    pub(crate) fn new() -> Self {
        SignalsBlocked(
            nsh_platform::BlockedSignals::all()
                .expect("blocking signals for an atomic trap update failed"),
        )
    }
}
