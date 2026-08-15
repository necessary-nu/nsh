//! Literal port of `src/trap.c` / `src/trap.h`.
//! Rules: `docs/spec/port/src/trap.md`.
//!
//! Note `sigmode`/`gotsig` are indexed by `signo - 1` while `trap` is indexed
//! by `signo`, slot 0 being the `EXIT` trap.

use bstr::{BStr, BString};
use core::ptr::{addr_of, addr_of_mut, null, null_mut};
use libc::{c_char, c_int, sigset_t};
use std::ffi::CStr;
use std::io::Write;

/// `sig_atomic_t` — `int` on every platform dash supports.
pub type sig_atomic_t = c_int;

use crate::error::{INTOFF, INTON};
use crate::error::Error;
use crate::eval::{Flow, SKIPFUNC, SKIPFUNCDEF, evalskip, exitstatus, savestatus};
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

/* trap handler commands */
/// The C's three states are `NULL` (no trap), `""` (the signal is ignored)
/// and an action; `None` and an empty `BString` keep them apart.
///
/// `onsig` reads this array from a kernel-delivered signal frame, so nothing
/// here may do more than the C's pointer load did: `Option<BString>` carries
/// its emptiness in the vector's pointer word, and `is_none()` is that load.
pub(crate) static mut trap: [Option<BString>; NSIG] = [const { None }; NSIG];
/* traps have not been fully cleared */
pub(crate) static mut ptrap: c_int = 0;
/* number of non-null traps */
pub static mut trapcnt: c_int = 0;
/* current value of signal */
pub static mut sigmode: [c_char; NSIG - 1] = [0; NSIG - 1];
/* indicates specified signal received */
static mut gotsig: [c_char; NSIG - 1] = [0; NSIG - 1];
/* last pending signal */
pub static mut pending_sig: sig_atomic_t = 0;
/* received SIGCHLD */
pub static mut gotsigchld: sig_atomic_t = 0;

#[inline]
pub(crate) unsafe fn trap_mut() -> &'static mut [Option<BString>; NSIG] {
    &mut *addr_of_mut!(trap)
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
pub unsafe fn have_traps() -> c_int {
    trapcnt
}

/* mkinit INIT fragment from src/trap.c:94-97. */
pub unsafe fn mkinit_init(sh: &mut crate::context::Shell) {
    sigmode[(libc::SIGCHLD - 1) as usize] = S_DFL;
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

// [spec:dash:def:trap.clear-traps-fn]
// [spec:dash:sem:trap.clear-traps-fn]
pub unsafe fn clear_traps(sh: &mut crate::context::Shell, n: Option<&Node>) {
    let simplecmd: c_int;

    simplecmd = crate::parser::issimplecmd(n, crate::builtins::TRAPCMD.name.as_ptr());

    INTOFF();
    for signo in 0..NSIG {
        /* trap not NULL or SIG_IGN */
        match &(*addr_of!(trap))[signo] {
            Some(t) if !t.is_empty() => {}
            _ => continue,
        }
        let otp = trap_mut()[signo].take();
        if signo != 0 {
            setsignal(sh, signo as c_int);
        }

        if simplecmd != 0 {
            trap_mut()[signo] = otp;
        }
        /* The C's else arm is `ckfree(*tp)` after `*tp = NULL`, so it frees
         * NULL and leaks `otp` (src/trap.c:189).  Dropping `otp` here frees
         * it instead, which no reader can tell apart: `dotrap` and
         * `exitshell` are the only readers of an action and both take a
         * copy before running it. */
    }
    trapcnt = 0;
    ptrap = simplecmd;
    INTON();
}

/*
 * Set the signal handler for the specified signal.  The routine figures
 * out what it should be set to.
 */

// [spec:dash:def:trap.setsignal-fn]
// [spec:dash:sem:trap.setsignal-fn]
pub unsafe fn setsignal(sh: &mut crate::context::Shell, signo: c_int) {
    let mut action: c_int;
    let lvforked: c_int;
    let mut tsig: c_char;
    let mut act: libc::sigaction = core::mem::zeroed();

    lvforked = crate::jobs::vforked;

    action = match &(*addr_of!(trap))[signo as usize] {
        None => S_DFL as c_int,
        Some(t) if !t.is_empty() => S_CATCH as c_int,
        Some(_) => S_IGN as c_int,
    };
    if crate::shellmain::rootshell() != 0 && action == S_DFL as c_int && lvforked == 0 {
        match signo {
            libc::SIGINT => {
                if sh.options.flag(crate::options::iflag) != 0
                    || !crate::options::minusc.is_null()
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

    let tp: *mut c_char = addr_of_mut!(sigmode[(signo - 1) as usize]);
    tsig = *tp;
    if tsig == 0 {
        /*
         * current setting unknown
         */
        if libc::sigaction(signo, null(), &mut act) == -1 {
            /*
             * Pretend it worked; maybe we should give a warning
             * here, but other shells don't. We don't alter
             * sigmode, so that we retry every time.
             */
            return;
        }
        if act.sa_sigaction == libc::SIG_IGN {
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
    match action {
        x if x == S_CATCH as c_int => {
            act.sa_sigaction = onsig as *const () as usize;
        }
        x if x == S_IGN as c_int => {
            act.sa_sigaction = libc::SIG_IGN;
        }
        _ => {
            act.sa_sigaction = libc::SIG_DFL;
        }
    }
    if lvforked == 0 {
        *tp = action as c_char;
    }
    act.sa_flags = 0;
    libc::sigfillset(&mut act.sa_mask);
    libc::sigaction(signo, &act, null_mut());
}

/*
 * Ignore a signal.
 */

// [spec:dash:def:trap.ignoresig-fn]
// [spec:dash:sem:trap.ignoresig-fn]
pub unsafe fn ignoresig(signo: c_int) {
    if sigmode[(signo - 1) as usize] == S_IGN || sigmode[(signo - 1) as usize] == S_HARD_IGN {
        return;
    }
    libc::signal(signo, libc::SIG_IGN);
    if crate::jobs::vforked == 0 {
        sigmode[(signo - 1) as usize] = S_IGN;
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
    if crate::jobs::vforked != 0 && libc::getpid() != crate::jobs::vforked {
        return;
    }

    if signo == libc::SIGCHLD {
        gotsigchld = 1;
        if (*addr_of!(trap))[libc::SIGCHLD as usize].is_none() {
            return;
        }
    }

    gotsig[(signo - 1) as usize] = 1;
    pending_sig = signo;

    if signo == libc::SIGINT && (*addr_of!(trap))[libc::SIGINT as usize].is_none() {
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

    status = savestatus;
    last_status = status;
    if status < 0 {
        status = exitstatus;
        savestatus = status;
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

        if evalskip != 0 {
            pending_sig = i + 1;
            break;
        }

        *q = 0;

        /* The action is copied out because `evalstring` parses from the
         * buffer it is handed and the action it runs may `trap` over this
         * very slot; the C passes the slot's own pointer and keeps reading
         * it after `trapcmd` has freed it. */
        let mut p = match &(*addr_of!(trap))[(i + 1) as usize] {
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
        if evalskip != SKIPFUNC {
            exitstatus = status;
        }
        i += 1;
        q = q.add(1);
    }

    savestatus = last_status;
    Ok(Flow::Done(exitstatus))
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
pub unsafe fn exitshell(sh: &mut crate::context::Shell) -> ! {
    savestatus = exitstatus;
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
        let p = (*addr_of_mut!(trap))[0].take();
        if let Some(p) = p {
            if ptrap != 0 {
                break 'out;
            }
            evalskip = 0;
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
                    drop(e);
                    break 'out;
                }
            }
            evalskip = SKIPFUNCDEF;
        }
    }
    /* out: */
    crate::init::exitreset(sh, by_exitcmd);
    crate::init::postexitreset();
    /*
     * Disable job control so that whoever had the foreground before we
     * started can get it back.
     */
    /* The C wraps this in a second `setjmp(loc.loc)` for one reason: a
     * raise inside the job-control teardown must not prevent the `_exit`
     * below. Dropping the diagnostic is that frame, exactly -- it caught
     * and went on -- and it is why the frame itself can go. */
    drop(crate::jobs::setjobctl(sh, 0));
    crate::output::flushall();
    crate::shell::flush_coverage();
    libc::_exit(exitstatus);
    /* NOTREACHED */
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
