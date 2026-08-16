//! Literal port of `src/trap.c` / `src/trap.h`.
//! Rules: `docs/spec/port/src/trap.md`.
//!
//! Note `sigmode`/`gotsig` are indexed by `signo - 1` while `trap` is indexed
//! by `signo`, slot 0 being the `EXIT` trap.

use bstr::{BStr, BString};
use core::ptr::{addr_of_mut, null, null_mut};
use libc::{c_char, c_int, sigset_t};
use std::ffi::CStr;
use std::io::Write;

/// `sig_atomic_t` — `int` on every platform dash supports.
pub type sig_atomic_t = c_int;

use crate::error::{INTOFF, INTON};
use crate::error::Error;
use crate::eval::{Flow, SKIPFUNC, SKIPFUNCDEF};
use crate::mystring::nullstr;
use crate::nodes::Node;
use crate::options::Options;

/// glibc's `NSIG` (`_NSIG`) on Linux.
pub const NSIG: usize = 65;

/*
 * Sigmode records the current value of the signal handlers for the various
 * modes.  A value of zero means that the current handler is not known.
 * S_HARD_IGN indicates that the signal was ignored on entry to the shell,
 */

const S_DFL: c_char = 1; /* default signal handling (SIG_DFL) */
const S_CATCH: c_char = 2; /* signal is caught */
const S_IGN: c_char = 3; /* signal is ignored (SIG_IGN) */
const S_HARD_IGN: c_char = 4; /* signal is ignored permenantly */
const S_RESET: c_char = 5; /* temporary - to reset a hard ignored sig */

/* indicates specified signal received */
static mut gotsig: [c_char; NSIG - 1] = [0; NSIG - 1];
/* last pending signal */
pub static mut pending_sig: sig_atomic_t = 0;
/* received SIGCHLD */
pub static mut gotsigchld: sig_atomic_t = 0;

/// The trap actions, the disposition cache, and the two counters that go
/// with them: `trap.c`'s `trap`, `ptrap`, `trapcnt` and `sigmode`.
///
/// This could not become a field until `onsig` stopped reading it. The
/// handler asked the table one question — *is a trap set for N?* — at two
/// indices, and a handler has no receiver. The answer is now a mirror in
/// the signal inbox, published by [`TrapTable::set`], which is why that is
/// the only writer of a slot and why it demands a
/// [`crate::siginbox::SignalsBlocked`] witness.
pub struct TrapTable {
    /// The action for each signal, slot 0 being the `EXIT` trap.
    ///
    /// The C's three states are `NULL` (no trap), `""` (the signal is
    /// ignored) and an action; `None` and an empty `BString` keep them
    /// apart. The presence bit the handler reads is `is_some()`, so an
    /// *ignored* signal counts as trapped — which is what dash's
    /// `trap[signo] != NULL` said.
    action: [Option<BString>; NSIG],
    /// traps have not been fully cleared
    pub(crate) ptrap: c_int,
    /// number of non-null traps
    pub(crate) trapcnt: c_int,
    /// current value of signal, indexed by `signo - 1`
    sigmode: [c_char; NSIG - 1],
}

impl TrapTable {
    /// What the four statics were declared with, which is what a shell
    /// starts with.
    pub(crate) fn new() -> Self {
        /* The mirror is the inbox's and the inbox is the process's, so a
         * second `Shell` in one process resets the first one's bits. That
         * is api-design 6's limit rather than a bug here: one process has
         * one handler and it reports to one inbox. A fresh table has no
         * traps, so clearing is also simply correct for the only case
         * that is not that limit. */
        let sink = crate::siginbox::signals();
        for signo in 0..NSIG {
            sink.set_trapped(signo, false);
        }
        TrapTable {
            action: [const { None }; NSIG],
            ptrap: 0,
            trapcnt: 0,
            sigmode: [0; NSIG - 1],
        }
    }

    /// The action set for `signo`, if any.
    #[inline]
    pub(crate) fn action(&self, signo: usize) -> Option<&BString> {
        self.action[signo].as_ref()
    }

    /// Replace `trap[signo]`, publishing the handler's presence bit with
    /// it. The only writer of either, and it returns what was there.
    ///
    /// The `SignalsBlocked` argument is the whole point of routing every
    /// write through one function: the slot and its bit are two stores,
    /// and a handler that runs between them reads a pair dash cannot
    /// produce — its `trap[signo]` is a single pointer. Both halves of the
    /// disagreement are observable and in opposite senses, so there is no
    /// safe order to write them in and the window has to be closed rather
    /// than chosen. `siginbox::SignalsBlocked` carries the argument;
    /// `docs/api-design.md` 5.3 carries the table.
    pub(crate) fn set(
        &mut self,
        _blocked: &crate::siginbox::SignalsBlocked,
        signo: usize,
        to: Option<BString>,
    ) -> Option<BString> {
        let was = core::mem::replace(&mut self.action[signo], to);
        crate::siginbox::signals().set_trapped(signo, self.action[signo].is_some());
        was
    }

    /// Take the `EXIT` action.
    ///
    /// Slot 0 is `EXIT`, which is not a signal number: `onsig` is never
    /// called with 0 and never reads the slot, so this needs neither the
    /// bracket nor the bit. Separating it is what keeps `exitshell` off
    /// the guarded path.
    pub(crate) fn take_exit_action(&mut self) -> Option<BString> {
        self.action[0].take()
    }
}

/// A trap action with the terminator its readers — `single_quote`, and
/// `evalstring` by way of `strlen` — read up to.
pub(crate) fn cbytes(s: &BString) -> Vec<u8> {
    let mut v = s.to_vec();
    v.push(0);
    v
}

// [spec:dash:def:trap.have-traps-fn]
// [spec:dash:sem:trap.have-traps-fn]
pub unsafe fn have_traps(sh: &crate::context::Shell) -> c_int {
    sh.traps.trapcnt
}

/* mkinit INIT fragment from src/trap.c:94-97. */
pub unsafe fn mkinit_init(sh: &mut crate::context::Shell) {
    sh.traps.sigmode[(libc::SIGCHLD - 1) as usize] = S_DFL;
    setsignal(sh, libc::SIGCHLD);
}

/* mkinit FORKRESET fragment from src/trap.c:99-101. */
pub unsafe fn mkinit_forkreset(sh: &mut crate::context::Shell, n: Option<&Node>) {
    clear_traps(sh, n);
}

/*
 * The trap builtin.
 */

/*
 * Clear traps on a fork.
 */

/// Clear the traps a fork inherited, and put back the dispositions that
/// go with having none.
///
/// **Its `setsignal` is child-side at every reachable call site**, which
/// is why it takes [`setsignal_in_child`] and needs no split. The seam was
/// recorded as "on both paths"; counted through, it is not:
///
/// * `mkinit_forkreset` ← `init::forkreset` ← `jobs::forkchild` is the
///   child.
/// * `init::forkreset`'s other caller is `evalsubshell`'s no-fork arm,
///   which runs in the shell's own process — but it is guarded by
///   `have_traps(sh) == 0`, and `trapcnt` counts exactly the slots with a
///   non-empty action, which is exactly what the loop below skips. The
///   loop body is unreachable from there. (It is also `EV_EXIT`-only, so
///   `Shell::run` cannot reach it at all.)
/// * `builtins::trap::trapcmd` calls it under `ptrap != 0`, and only this
///   function ever writes `ptrap`, from `simplecmd` — which is non-zero
///   only when a fork was made *for* a `trap` command. So that `trapcmd`
///   is running in that child, and the parent's `ptrap` stays 0.
// [spec:dash:def:trap.clear-traps-fn]
// [spec:dash:sem:trap.clear-traps-fn]
pub unsafe fn clear_traps(sh: &mut crate::context::Shell, n: Option<&Node>) {
    let simplecmd: c_int;

    simplecmd = crate::parser::issimplecmd(n, crate::builtins::TRAPCMD.name.as_ptr());

    INTOFF();
    /* One guard for the whole loop -- the fork's single pair -- rather
     * than one per slot. The `simplecmd` arm below clears a slot and puts
     * it back with a `setsignal` in between, and a per-write guard would
     * make each half atomic while leaving the shell observably untrapped
     * across the pair. Hoisting closes that too, and costs one
     * `sigprocmask` pair per fork against the ~100us of the fork itself. */
    let blocked = crate::siginbox::SignalsBlocked::new();
    for signo in 0..NSIG {
        /* trap not NULL or SIG_IGN */
        match sh.traps.action(signo) {
            Some(t) if !t.is_empty() => {}
            _ => continue,
        }
        let otp = sh.traps.set(&blocked, signo, None);
        if signo != 0 {
            setsignal_in_child(sh, signo as c_int);
        }

        if simplecmd != 0 {
            drop(sh.traps.set(&blocked, signo, otp));
        }
        /* The C's else arm is `ckfree(*tp)` after `*tp = NULL`, so it frees
         * NULL and leaks `otp` (src/trap.c:189).  Dropping `otp` here frees
         * it instead, which no reader can tell apart: `dotrap` and
         * `exitshell` are the only readers of an action and both take a
         * copy before running it. */
    }
    sh.traps.trapcnt = 0;
    sh.traps.ptrap = simplecmd;
    drop(blocked);
    INTON();
}

/// Which side of a `fork` a disposition change is being made on, and so
/// what performs it.
///
/// The call site chooses, and that is the design rather than a shortcut:
/// whether a caller runs in a forked child is a static property of the
/// *path*, not a dynamic property of the shell — a child's `Shell` is
/// bit-for-bit the one that forked it, so there is nothing in shell state
/// a flag could have been read from.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Via {
    /// The parent, where the process belongs to whoever linked the library
    /// in. [dec:nsh:host-owns-signals]: the shell decides *which*
    /// disposition, and the host is what installs it.
    Host,
    /// A forked child, which goes to libc directly. Two reasons, each
    /// sufficient on its own:
    ///
    /// * Routing it would be an indirect call into embedder code made in
    ///   a forked — sometimes *vforked* — child, which
    ///   [dec:nsh:fork-child-is-a-terminus] forbids.
    /// * Under [`crate::host::NoHost`] a routed call installs nothing, so
    ///   a background job would go on taking `^C` from the terminal
    ///   because its `ignoresig(SIGINT)` had been quietly dropped. The
    ///   child *is* the whole process, so there is no third party for the
    ///   host to be protecting.
    Libc,
}

/// The `struct sigaction` a query filled in, read as a [`Disposition`].
///
/// dash asks only "is this `SIG_IGN`", so `Default` and `Catch` are one
/// answer as far as [`setsignal`] is concerned. They are kept apart
/// because [`crate::host::Host`] is what an embedder implements, and an
/// embedder can tell them apart.
pub(crate) fn disposition_of(act: &libc::sigaction) -> crate::host::Disposition {
    if act.sa_sigaction == libc::SIG_IGN {
        crate::host::Disposition::Ignore
    } else if act.sa_sigaction == libc::SIG_DFL {
        crate::host::Disposition::Default
    } else {
        crate::host::Disposition::Catch
    }
}

/// What is installed for `signo` right now, or `Err` if it cannot be read.
unsafe fn current_disposition(
    sh: &mut crate::context::Shell,
    signo: c_int,
    via: Via,
) -> std::io::Result<crate::host::Disposition> {
    match via {
        Via::Host => sh.host.signal(crate::status::Signal::from_raw(signo)),
        Via::Libc => {
            let mut act: libc::sigaction = core::mem::zeroed();
            if libc::sigaction(signo, null(), &mut act) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(disposition_of(&act))
        }
    }
}

/// Install `to` for `signo`.
///
/// The `struct sigaction` the query filled in is not carried into the
/// install the way the C carries it, and nothing observable is lost:
/// every field the C reused is overwritten before the second call —
/// `sa_sigaction` by the choice below, `sa_flags` by the `0`, `sa_mask` by
/// `sigfillset` — and `sa_restorer` never reaches the kernel at all,
/// because glibc's `sigaction` writes its own trampoline into the
/// kernel-facing copy unconditionally. What is left is the signal number
/// and the disposition, which is exactly what the host is asked for.
unsafe fn install_disposition(
    sh: &mut crate::context::Shell,
    signo: c_int,
    to: crate::host::Disposition,
    via: Via,
) {
    match via {
        Via::Host => {
            /* The C ignores `sigaction`'s return value here, so a shell
             * that cannot install a disposition carries on with the one it
             * has; a host that refuses reads the same way. */
            let _ = sh
                .host
                .set_signal(crate::status::Signal::from_raw(signo), to);
        }
        Via::Libc => {
            let mut act: libc::sigaction = core::mem::zeroed();
            act.sa_sigaction = match to {
                crate::host::Disposition::Catch => onsig as *const () as usize,
                crate::host::Disposition::Ignore => libc::SIG_IGN,
                crate::host::Disposition::Default => libc::SIG_DFL,
            };
            act.sa_flags = 0;
            libc::sigfillset(&mut act.sa_mask);
            libc::sigaction(signo, &act, null_mut());
        }
    }
}

/*
 * Set the signal handler for the specified signal.  The routine figures
 * out what it should be set to.
 */

/// The parent's entry point: the host installs what the shell decided.
// [spec:dash:def:trap.setsignal-fn]
// [spec:dash:sem:trap.setsignal-fn]
pub unsafe fn setsignal(sh: &mut crate::context::Shell, signo: c_int) {
    setsignal_via(sh, signo, Via::Host)
}

/// The forked child's entry point: identical policy, installed directly.
///
/// See [`Via::Libc`] for why this is a second entry point rather than an
/// argument the shell could have answered for itself.
pub unsafe fn setsignal_in_child(sh: &mut crate::context::Shell, signo: c_int) {
    setsignal_via(sh, signo, Via::Libc)
}

unsafe fn setsignal_via(sh: &mut crate::context::Shell, signo: c_int, via: Via) {
    let mut action: c_int;
    let lvforked: c_int;
    let mut tsig: c_char;

    lvforked = crate::siginbox::signals().vforked();

    action = match sh.traps.action(signo as usize) {
        None => S_DFL as c_int,
        Some(t) if !t.is_empty() => S_CATCH as c_int,
        Some(_) => S_IGN as c_int,
    };
    if crate::shellmain::rootshell() != 0 && action == S_DFL as c_int && lvforked == 0 {
        match signo {
            libc::SIGINT => {
                if sh.options.flag(crate::options::iflag) != 0
                    || !sh.options.minusc.is_null()
                    || sh.options.flag(crate::options::sflag) == 0
                {
                    action = S_CATCH as c_int;
                }
            }
            libc::SIGQUIT => {
                /* #ifdef DEBUG: if (debug) break; */
                if crate::shell::DEBUG && sh.options.flag(crate::options::debug) != 0 {
                    /* break */
                } else if sh.options.flag(crate::options::iflag) != 0 {
                    action = S_IGN as c_int;
                }
            }
            libc::SIGTERM => {
                if sh.options.flag(crate::options::iflag) != 0 {
                    action = S_IGN as c_int;
                }
            }
            /* #if JOBS */
            libc::SIGTSTP | libc::SIGTTOU => {
                if sh.options.flag(crate::options::mflag) != 0 {
                    action = S_IGN as c_int;
                }
            }
            _ => {}
        }
    }

    if signo == libc::SIGCHLD {
        action = S_CATCH as c_int;
    }

    /* The C keeps a `char *tp` into `sigmode[]` across the two
     * `sigaction` calls below. An index says the same thing and does not
     * hold a raw pointer into `sh` while `sh.options` is read. */
    let tp = (signo - 1) as usize;
    tsig = sh.traps.sigmode[tp];
    if tsig == 0 {
        /*
         * current setting unknown
         */
        let current = match current_disposition(sh, signo, via) {
            Ok(d) => d,
            Err(_) => {
                /*
                 * Pretend it worked; maybe we should give a warning
                 * here, but other shells don't. We don't alter
                 * sigmode, so that we retry every time.
                 */
                return;
            }
        };
        /* This test is the whole reason `Host` has a `signal` as well as
         * a `set_signal`: a signal already ignored when the shell started
         * is hard-ignored and can never be trapped, and that rule cannot
         * be reproduced without reading the inherited disposition. */
        if current == crate::host::Disposition::Ignore {
            if sh.options.flag(crate::options::mflag) != 0
                && (signo == libc::SIGTSTP || signo == libc::SIGTTIN || signo == libc::SIGTTOU)
            {
                tsig = S_IGN; /* don't hard ignore these */
            } else {
                tsig = S_HARD_IGN;
            }
        } else {
            tsig = S_RESET; /* force to be set */
        }
    }
    if tsig == S_HARD_IGN || tsig as c_int == action {
        return;
    }
    let want = match action {
        x if x == S_CATCH as c_int => crate::host::Disposition::Catch,
        x if x == S_IGN as c_int => crate::host::Disposition::Ignore,
        _ => crate::host::Disposition::Default,
    };
    if lvforked == 0 {
        sh.traps.sigmode[tp] = action as c_char;
    }
    install_disposition(sh, signo, want, via);
}

/*
 * Ignore a signal.
 */

/// Ignore a signal, in a forked child, directly.
///
/// There is no parent-side twin, because there is no parent-side caller:
/// both call sites are `forkchild`'s `FORK_BG` arm, where the child must
/// genuinely stop taking `^C` from the terminal. [`Via::Libc`] carries the
/// argument; a parent-side caller appearing later needs a twin routed
/// through the host, and the name here is what should make that obvious.
///
/// `signal` rather than `sigaction` is dash's spelling and is kept, which
/// costs nothing: `SIG_IGN` runs no handler, so the flags and mask the two
/// calls disagree about have nothing to apply to.
// [spec:dash:def:trap.ignoresig-fn]
// [spec:dash:sem:trap.ignoresig-fn]
pub unsafe fn ignoresig_in_child(sh: &mut crate::context::Shell, signo: c_int) {
    let mode = sh.traps.sigmode[(signo - 1) as usize];
    if mode == S_IGN || mode == S_HARD_IGN {
        return;
    }
    libc::signal(signo, libc::SIG_IGN);
    if crate::siginbox::signals().vforked() == 0 {
        sh.traps.sigmode[(signo - 1) as usize] = S_IGN;
    }
}

/*
 * Signal handler.
 */

// [spec:dash:def:trap.onsig-fn]
// [spec:dash:sem:trap.onsig-fn]
/* `extern "C"` again, and getting back here is one of the things step F
 * was for.
 *
 * This function used to be `extern "C-unwind"`, and the comment it
 * carried is worth keeping because it records a real bug rather than a
 * precaution. In the C, `onint()` does not return: it `longjmp`s out of
 * the signal handler to whichever handler is armed, which for an
 * interactive shell is `main_handler`. The port raised that jump as an
 * unwind, Rust makes unwinding out of an `extern "C"` frame an abort,
 * and so an interactive port died of SIGABRT with status 134 on
 * `kill -INT $$` where dash printed a fresh prompt. `extern "C-unwind"`
 * was the fix: the unwinder walks the kernel signal frame through the
 * `__restore_rt` trampoline's CFI, the same mechanism that lets a C++
 * exception leave a handler under `-fnon-call-exceptions`.
 *
 * Depending on that is not something a library may ask of an embedder,
 * and it is incompatible with `panic = "abort"` — which is the profile
 * constraint [dec:nsh:errors-are-values] exists to remove, so leaving
 * the interrupt as an unwind would have made the decision's first
 * accepted consequence false in the one case a user is most likely to
 * hit. The handler now does one store and returns, which is
 * async-signal-safe by construction and is what
 * [dec:nsh:host-owns-signals]'s `SignalSink` will formalise. */
pub unsafe extern "C" fn onsig(signo: c_int) {
    /* Read once. The C loads the global twice and cannot observe the
     * difference, because the only writer is the process this test is
     * distinguishing itself from; one load says so. */
    let vforked = crate::siginbox::signals().vforked();
    if vforked != 0 && libc::getpid() != vforked {
        return;
    }

    if signo == libc::SIGCHLD {
        gotsigchld = 1;
        if !crate::siginbox::signals().is_trapped(libc::SIGCHLD) {
            return;
        }
    }

    gotsig[(signo - 1) as usize] = 1;
    pending_sig = signo;

    if signo == libc::SIGINT && !crate::siginbox::signals().is_trapped(libc::SIGINT) {
        /* `if (!suppressint) onint();` is gone. The C had two delivery
         * modes and only one of them was asynchronous; now neither is.
         * The handler stores, and the shell takes delivery at a poll
         * site it reached on its own -- one of the EINTR returns, or
         * `dotrap`, which `evaltree` calls on every command.
         *
         * `sa_flags = 0` at `setsignal` below is why there is always a
         * poll site to reach: dash never sets SA_RESTART, so every
         * interruptible syscall returns EINTR when a signal arrives. */
        crate::error::intpending = 1;
    }
}

/*
 * Called to execute a trap.  Perhaps we should avoid entering new trap
 * handlers while we are executing a trap handler.
 */

// [spec:dash:def:trap.dotrap-fn]
// [spec:dash:sem:trap.dotrap-fn]
pub unsafe fn dotrap(sh: &mut crate::context::Shell) -> Result<Flow, Error> {
    let mut q: *mut c_char;
    let mut i: c_int;
    let mut status: c_int;
    let last_status: c_int;

    /* The poll site the shell reaches most often: `evaltree` calls
     * `dotrap` before every command and again at its `out:`, so an
     * interrupt taken anywhere the shell was not looking is delivered at
     * the next command boundary at the latest. It is tested before
     * `pending_sig`, because an *untrapped* SIGINT sets `intpending` and
     * has no trap action for the loop below to run. */
    if let Some(e) = crate::error::poll_interrupt(sh) {
        return Err(e);
    }

    if pending_sig == 0 {
        return Ok(Flow::Done(0));
    }

    status = sh.eval.savestatus;
    last_status = status;
    if status < 0 {
        status = sh.status;
        sh.eval.savestatus = status;
    }
    pending_sig = 0;
    crate::error::barrier();

    i = 0;
    q = addr_of_mut!(gotsig) as *mut c_char;
    while i < NSIG as c_int - 1 {
        if *q == 0 {
            i += 1;
            q = q.add(1);
            continue;
        }

        if sh.eval.evalskip != 0 {
            pending_sig = i + 1;
            break;
        }

        *q = 0;

        /* The action is copied out because `evalstring` parses from the
         * buffer it is handed and the action it runs may `trap` over this
         * very slot; the C passes the slot's own pointer and keeps reading
         * it after `trapcmd` has freed it. */
        let mut p = match sh.traps.action((i + 1) as usize) {
            Some(t) => cbytes(t),
            None => {
                i += 1;
                q = q.add(1);
                continue;
            }
        };
        /* A trap action is shell code and can do anything shell code can,
         * including `exit`. The C's `exit` left here by longjmp, straight
         * past the `savestatus = last_status` below; a `Flow::Exit`
         * returned from here skips it in exactly the same way, which is
         * what leaves `savestatus` holding what `exit` was told. */
        match crate::eval::evalstring(sh, p.as_mut_ptr() as *mut c_char, 0)? {
            Flow::Done(_) => {}
            exit @ Flow::Exit { .. } => return Ok(exit),
        }
        if sh.eval.evalskip != SKIPFUNC {
            sh.status = status;
        }
        i += 1;
        q = q.add(1);
    }

    sh.eval.savestatus = last_status;
    Ok(Flow::Done(sh.status))
}

/*
 * Controls whether the shell is interactive or not.
 */

// [spec:dash:def:trap.setinteractive-fn]
// [spec:dash:sem:trap.setinteractive-fn]
pub unsafe fn setinteractive(sh: &mut crate::context::Shell, on: c_int) {
    static mut is_interactive: c_int = 0;

    let on = on + 1;
    if on == is_interactive {
        return;
    }
    is_interactive = on;
    setsignal(sh, libc::SIGINT);
    setsignal(sh, libc::SIGQUIT);
    setsignal(sh, libc::SIGTERM);
}

/*
 * Called to exit the shell.
 */

// [spec:dash:def:trap.exitshell-fn]
// [spec:dash:sem:trap.exitshell-fn]
/// Run the EXIT trap, tear job control down, and **return** the status
/// the shell leaves with.
///
/// It used to end in `_exit`, and that was the one `_exit` in the crate
/// that ended the *host's* process rather than a child the library
/// forked. `[dec:nsh:host-owns-the-process]` puts ending the process
/// outside what a library may do on its own authority, and answers it
/// with an absence rather than a grant: there is no `Host` method for it
/// because after this there is nothing to grant — the status is returned,
/// and whoever owns the process decides what to do with it. `nsh-cli`
/// calls `std::process::exit`.
///
/// The other three `_exit`s stay, and are correct: `shellmain`'s
/// `exit_from_child`, `jobs`' `forkchild_fatal` and `redir.rs:483` all
/// end a child the library forked, which `[dec:nsh:fork-child-is-a-terminus]`
/// says is a terminus rather than a frame.
pub unsafe fn exitshell(sh: &mut crate::context::Shell) -> crate::status::ExitStatus {
    sh.eval.savestatus = sh.status;
    /* `TRACE(("pid %d, exitshell(%d)\n", getpid(), savestatus));` —
     * `#ifdef DEBUG` in `shell.h`, and the dash build does not define it. */
    /* Whether the EXIT trap ended by running out or by calling `exit`
     * itself. It is the C's `exception == EXEXIT` at the `out:` label, and
     * the two ways of reaching `out:` are exactly the two things
     * `exitreset` tests: the trap ran to the end, which is the
     * `evalskip = SKIPFUNCDEF` below, or the trap exited, which is this. */
    let mut by_exitcmd = false;
    'out: {
        /* `trap[0] = NULL` with no free: the C leaks the EXIT action on
         * purpose so `evalstring` can still read it.  Taking it keeps the
         * action alive for exactly as long and gives the buffer back. */
        let p = sh.traps.take_exit_action();
        if let Some(p) = p {
            if sh.traps.ptrap != 0 {
                break 'out;
            }
            sh.eval.evalskip = 0;
            let mut p = cbytes(&p);
            /* An error in the EXIT trap is reported and dropped -- the
             * shell is already exiting, and the C's `longjmp` landed at
             * `out:` with nothing left to inspect it. What must not be
             * dropped is an `exit` *inside* the trap, because it names the
             * status the shell leaves with. */
            match crate::eval::evalstring(sh, p.as_mut_ptr() as *mut c_char, 0) {
                Ok(crate::eval::Flow::Exit { by_exitcmd: b }) => {
                    by_exitcmd = b;
                    break 'out;
                }
                Ok(crate::eval::Flow::Done(_)) => {}
                Err(e) => {
                    /* The EXIT trap failed. `_exit(exitstatus)` below is
                     * what leaves, and the status it leaves with is this
                     * error's -- written here because the raise no longer
                     * writes it. `trap 'nosuchcmd' EXIT; exit 3` exits
                     * with the trap's status, not 3, and that is dash. */
                    sh.status = e.status();
                    drop(e);
                    break 'out;
                }
            }
            sh.eval.evalskip = SKIPFUNCDEF;
        }
    }
    /* out: */
    crate::init::exitreset(sh, by_exitcmd);
    crate::init::postexitreset(sh);
    /*
     * Disable job control so that whoever had the foreground before we
     * started can get it back.
     */
    /* The C wraps this in a second `setjmp(loc.loc)` for one reason: a
     * raise inside the job-control teardown must not prevent the `_exit`
     * below. Dropping the diagnostic is that frame, exactly -- it caught
     * and went on -- and it is why the frame itself can go. */
    drop(crate::jobs::setjobctl(sh, 0));
    sh.io.flushall();
    crate::shell::flush_coverage();
    crate::status::ExitStatus::from_raw(sh.status)
}

// [spec:dash:def:trap.decode-signum-fn]
// [spec:dash:sem:trap.decode-signum-fn]
pub(crate) unsafe fn decode_signum(string: *const c_char) -> c_int {
    let mut signo: c_int = -1;

    if crate::mystring::is_number(string) != 0 {
        signo = libc::atoi(string);
        if signo >= NSIG as c_int {
            signo = -1;
        }
    }

    signo
}

// [spec:dash:def:trap.decode-signal-fn]
// [spec:dash:sem:trap.decode-signal-fn]
pub unsafe fn decode_signal(string: *const c_char, minsig: c_int) -> c_int {
    let mut signo: c_int;

    signo = decode_signum(string);
    if signo >= 0 {
        return signo;
    }

    signo = minsig;
    while signo < NSIG as c_int {
        if CStr::from_ptr(string)
            .to_bytes()
            .eq_ignore_ascii_case(crate::signames::signal_names[signo as usize].to_bytes())
        {
            return signo;
        }
        signo += 1;
    }

    -1
}

// [spec:dash:def:trap.sigblockall-fn]
// [spec:dash:sem:trap.sigblockall-fn]
pub unsafe fn sigblockall(oldmask: *mut sigset_t) {
    let mut mask: sigset_t = core::mem::zeroed();

    libc::sigfillset(&mut mask);
    libc::sigprocmask(libc::SIG_SETMASK, &mask, oldmask);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::siginbox::{SignalsBlocked, signals};

    /// **The mirror is the table, or it is a bug.** `onsig` cannot reach
    /// `sh.traps`, so the bit it reads instead has to say the same thing
    /// the slot does. Every direction of that is a pty case
    /// (`tests/harness/ptydiff.py`, the job-control block); this states
    /// the same property where a mutation of `TrapTable::set` fails it in
    /// milliseconds rather than in a terminal.
    #[test]
    fn mirror_follows_the_slot() {
        let _g = crate::testutil::lock();
        unsafe {
            let mut t = TrapTable::new();
            assert!(!signals().is_trapped(libc::SIGINT), "a new table has no traps");

            let b = SignalsBlocked::new();
            drop(t.set(&b, libc::SIGINT as usize, Some(BString::from("echo hi"))));
            assert!(signals().is_trapped(libc::SIGINT), "set an action, set the bit");

            drop(t.set(&b, libc::SIGINT as usize, None));
            assert!(!signals().is_trapped(libc::SIGINT), "clear the action, clear the bit");
            drop(b);
        }
    }

    /// **The predicate is `is_some()`, not "has an action".** The C's
    /// three states are `NULL`, `""` and an action, and `onsig` tests
    /// `trap[signo] != NULL` — so `trap '' INT`, which *ignores* the
    /// signal, still reads as trapped. A mirror keyed on emptiness passes
    /// every other test here and gets that one case backwards.
    #[test]
    fn ignored_signal_counts_as_trapped() {
        let _g = crate::testutil::lock();
        unsafe {
            let mut t = TrapTable::new();
            let b = SignalsBlocked::new();
            drop(t.set(&b, libc::SIGINT as usize, Some(BString::new(Vec::new()))));
            assert!(
                signals().is_trapped(libc::SIGINT),
                "`trap '' INT` is a trap as far as the handler is concerned"
            );
            drop(t.set(&b, libc::SIGINT as usize, None));
            drop(b);
        }
    }

    /// A fresh table starts the mirror fresh with it. This is also where
    /// `docs/api-design.md` 6's limit bites: the inbox is the process's,
    /// so a second `Shell` in one process resets the first one's bits.
    /// Stated as a test so it is a known property rather than a surprise.
    #[test]
    fn a_new_table_clears_the_mirror() {
        let _g = crate::testutil::lock();
        unsafe {
            let mut t = TrapTable::new();
            let b = SignalsBlocked::new();
            drop(t.set(&b, libc::SIGCHLD as usize, Some(BString::from("echo chld"))));
            drop(b);
            assert!(signals().is_trapped(libc::SIGCHLD));

            let _fresh = TrapTable::new();
            assert!(!signals().is_trapped(libc::SIGCHLD), "a new table, a clear mirror");
        }
    }

    /// **The guard blocks, and puts the mask back.** Without the `Drop`
    /// the shell runs on with every signal blocked — which no test above
    /// would notice, and which would make it stop answering anything.
    #[test]
    fn the_guard_blocks_and_restores() {
        let _g = crate::testutil::lock();
        unsafe {
            crate::system::sigclearmask();
            let mut before: sigset_t = core::mem::zeroed();
            libc::sigprocmask(libc::SIG_SETMASK, null(), &mut before);
            assert_eq!(libc::sigismember(&before, libc::SIGINT), 0, "start unblocked");

            {
                let _b = SignalsBlocked::new();
                let mut during: sigset_t = core::mem::zeroed();
                libc::sigprocmask(libc::SIG_SETMASK, null(), &mut during);
                assert_eq!(libc::sigismember(&during, libc::SIGINT), 1, "blocked inside");
                assert_eq!(libc::sigismember(&during, libc::SIGCHLD), 1, "all of them");
            }

            let mut after: sigset_t = core::mem::zeroed();
            libc::sigprocmask(libc::SIG_SETMASK, null(), &mut after);
            assert_eq!(libc::sigismember(&after, libc::SIGINT), 0, "restored on drop");
        }
    }
}
