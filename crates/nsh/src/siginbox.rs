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
use libc::{c_int, sigset_t};

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
    /// `jobs.rs`'s `vforked` — the pid of the process that called
    /// `vfork`, or zero.
    ///
    /// Not shell state and not a `Shell` field. `vforkexec` sets it in
    /// the *parent* before the call and clears it after; the child reads
    /// it out of the address space it shares to learn that it is the
    /// child (`getpid() != vforked`). That is a property of an address
    /// space rather than of a shell.
    vforked: AtomicI32,
}

static SINK: SignalSink = SignalSink {
    trapped: [const { AtomicBool::new(false) }; NSIG],
    vforked: AtomicI32::new(0),
};

/// The process's signal inbox.
#[inline]
pub fn signals() -> &'static SignalSink {
    &SINK
}

impl SignalSink {
    /// Is a trap set for `signo`? The handler's question.
    ///
    /// "Set" is dash's `trap[signo] != NULL`, which includes the empty
    /// action that means *ignore* — the C's three states are `NULL`, `""`
    /// and an action, and this bit separates the first from the other
    /// two. A mirror keyed on "has a non-empty action" would answer
    /// differently for `trap '' INT`.
    #[inline]
    pub fn is_trapped(&self, signo: c_int) -> bool {
        self.trapped[signo as usize].load(Ordering::Relaxed)
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

    /// The pid that called `vfork`, or zero outside the window.
    #[inline]
    pub fn vforked(&self) -> c_int {
        self.vforked.load(Ordering::Relaxed)
    }

    /// Deliver a signal to the shell. The only thing a host's handler may
    /// do, and the whole of what it may do.
    ///
    /// The body is [`crate::trap::onsig`], which is where it belongs: what
    /// a delivery *does* is store into four locations — `gotsig`,
    /// `pending_sig`, `gotsigchld` and `error::intpending` — and those
    /// live beside the code that polls them, not here. This is the name
    /// the host knows it by, so an embedder's handler needs no access to
    /// the crate's internals to be correct.
    ///
    /// Everything it does is async-signal-safe by construction: it reads
    /// two atomics, compares a pid, and stores. It does not allocate, does
    /// not take a lock, and does not unwind — the last one is a fix rather
    /// than a precaution, and `onsig`'s own comment records the SIGABRT it
    /// was.
    ///
    /// # Safety
    ///
    /// Call it from a signal handler and from nowhere else. `signo` must
    /// be the number the handler was invoked with.
    #[inline]
    pub unsafe fn raise(&self, signo: c_int) {
        crate::trap::onsig(signo);
    }

    /// Enter or leave the `vfork` window.
    ///
    /// Needs no [`SignalsBlocked`] bracket, and the reason is the
    /// handler's own test: `vforked != 0 && getpid() != vforked`. In
    /// either window — after the store and before `vfork`, or after
    /// `vfork` and before the clear — the parent reads
    /// `getpid() == vforked` and carries on, which is what it would have
    /// done with the store ordered the other way.
    #[inline]
    pub fn set_vforked(&self, pid: c_int) {
        self.vforked.store(pid, Ordering::Relaxed);
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
pub(crate) struct SignalsBlocked(sigset_t);

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
    pub(crate) unsafe fn new() -> Self {
        let mut old: sigset_t = core::mem::zeroed();
        crate::trap::sigblockall(&mut old);
        SignalsBlocked(old)
    }
}

impl Drop for SignalsBlocked {
    fn drop(&mut self) {
        /* `clear_traps` runs in a forked child, where destructors are
         * ordinarily suspect -- but not in a *vforked* one: `forkchild`
         * gates `forkreset` on `lvforked == 0`, so this never runs on the
         * shared-address-space path. `sigprocmask` is async-signal-safe
         * regardless. */
        unsafe {
            libc::sigprocmask(libc::SIG_SETMASK, &self.0, core::ptr::null_mut());
        }
    }
}
