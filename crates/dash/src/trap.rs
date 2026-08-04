//! Literal port of `src/trap.c` / `src/trap.h`.
//! Rules: `docs/spec/port/src/trap.md`.
//!
//! Note `sigmode`/`gotsig` are indexed by `signo - 1` while `trap` is indexed
//! by `signo`, slot 0 being the `EXIT` trap.

use libc::{c_char, c_int, c_void, sigset_t};
use core::ptr::{addr_of, addr_of_mut, null, null_mut};

/// `sig_atomic_t` — `int` on every platform dash supports.
pub type sig_atomic_t = c_int;

use crate::error::{jmploc, INTOFF, INTON};
use crate::eval::{evalskip, exitstatus, savestatus, SKIPFUNC, SKIPFUNCDEF};
use crate::memalloc::{ckfree, savestr};
use crate::mystring::nullstr;
use crate::output::VaArg;
use crate::shell::cstr;
use crate::nodes::node;
use crate::options::{argptr, nextopt};

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
static mut trap: [*mut c_char; NSIG] = [null_mut(); NSIG];
/* traps have not been fully cleared */
static mut ptrap: c_int = 0;
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

// [spec:dash:def:trap.have-traps-fn]
// [spec:dash:sem:trap.have-traps-fn]
pub unsafe fn have_traps() -> c_int {
    trapcnt
}

/* mkinit INIT fragment from src/trap.c:94-97. */
pub unsafe fn mkinit_init() {
    sigmode[(libc::SIGCHLD - 1) as usize] = S_DFL;
    setsignal(libc::SIGCHLD);
}

/* mkinit FORKRESET fragment from src/trap.c:99-101. */
pub unsafe fn mkinit_forkreset(n: *mut node) {
    clear_traps(n);
}

/*
 * The trap builtin.
 */

// [spec:dash:def:trap.trapcmd-fn]
// [spec:dash:sem:trap.trapcmd-fn]
pub unsafe fn trapcmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    let mut action: *mut c_char;
    let mut ap: *mut *mut c_char;
    let mut signo: c_int;

    nextopt(addr_of!(nullstr) as *const c_char);
    ap = argptr;
    if (*ap).is_null() {
        signo = 0;
        while signo < NSIG as c_int {
            if !trap[signo as usize].is_null() {
                crate::output::out1fmt(
                    cstr(b"trap -- %s %s\n\0"),
                    &[
                        VaArg::Str(crate::mystring::single_quote(trap[signo as usize])),
                        VaArg::Str(crate::signames::signal_names[signo as usize].as_ptr()),
                    ],
                );
            }
            signo += 1;
        }
        return 0;
    }
    if ptrap != 0 {
        clear_traps(null_mut());
    }
    if (*ap.offset(1)).is_null() || decode_signum(*ap) >= 0 {
        action = null_mut();
    } else {
        action = *ap;
        ap = ap.add(1);
    }
    while !(*ap).is_null() {
        signo = decode_signal(*ap, 0);
        if signo < 0 {
            crate::output::outfmt(
                crate::output::out2,
                cstr(b"trap: %s: bad trap\n\0"),
                &[VaArg::Str(*ap)],
            );
            return 1;
        }
        INTOFF();
        if !action.is_null() {
            if *action.offset(0) == b'-' as c_char && *action.offset(1) == b'\0' as c_char {
                action = null_mut();
            } else {
                if *action != 0 {
                    trapcnt += 1;
                }
                action = savestr(action);
            }
        }
        if !trap[signo as usize].is_null() {
            if *trap[signo as usize] != 0 {
                trapcnt -= 1;
            }
            ckfree(trap[signo as usize] as *mut c_void);
        }
        trap[signo as usize] = action;
        if signo != 0 {
            setsignal(signo);
        }
        INTON();
        ap = ap.add(1);
    }
    0
}

/*
 * Clear traps on a fork.
 */

// [spec:dash:def:trap.clear-traps-fn]
// [spec:dash:sem:trap.clear-traps-fn]
pub unsafe fn clear_traps(n: *mut node) {
    let simplecmd: c_int;
    let mut tp: *mut *mut c_char;

    simplecmd = crate::parser::issimplecmd(n, crate::builtins::TRAPCMD.name.as_ptr());

    INTOFF();
    tp = addr_of_mut!(trap) as *mut *mut c_char;
    while tp < (addr_of_mut!(trap) as *mut *mut c_char).add(NSIG) {
        if !(*tp).is_null() && **tp != 0 {
            /* trap not NULL or SIG_IGN */
            let otp: *mut c_char = *tp;

            *tp = null_mut();
            if tp != addr_of_mut!(trap) as *mut *mut c_char {
                setsignal(
                    ((tp as usize - addr_of_mut!(trap) as usize)
                        / core::mem::size_of::<*mut c_char>()) as c_int,
                );
            }

            if simplecmd != 0 {
                *tp = otp;
            } else {
                /* NB: *tp has just been set to NULL, so this frees NULL and
                 * leaks `otp`.  Reproduced verbatim (src/trap.c:189). */
                ckfree(*tp as *mut c_void);
            }
        }
        tp = tp.add(1);
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
pub unsafe fn setsignal(signo: c_int) {
    let mut action: c_int;
    let lvforked: c_int;
    let mut t: *mut c_char;
    let mut tsig: c_char;
    let mut act: libc::sigaction = core::mem::zeroed();

    lvforked = crate::jobs::vforked;

    t = trap[signo as usize];
    if t.is_null() {
        action = S_DFL as c_int;
    } else if *t != b'\0' as c_char {
        action = S_CATCH as c_int;
    } else {
        action = S_IGN as c_int;
    }
    if crate::shellmain::rootshell() != 0 && action == S_DFL as c_int && lvforked == 0 {
        match signo {
            libc::SIGINT => {
                if crate::options::optlist[crate::options::iflag] != 0 || !crate::options::minusc.is_null() || crate::options::optlist[crate::options::sflag] == 0
                {
                    action = S_CATCH as c_int;
                }
            }
            libc::SIGQUIT => {
                /* #ifdef DEBUG: if (debug) break; */
                if crate::shell::DEBUG && crate::options::optlist[crate::options::debug] != 0 {
                    /* break */
                } else if crate::options::optlist[crate::options::iflag] != 0 {
                    action = S_IGN as c_int;
                }
            }
            libc::SIGTERM => {
                if crate::options::optlist[crate::options::iflag] != 0 {
                    action = S_IGN as c_int;
                }
            }
            /* #if JOBS */
            libc::SIGTSTP | libc::SIGTTOU => {
                if crate::options::optlist[crate::options::mflag] != 0 {
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
            if crate::options::optlist[crate::options::mflag] != 0
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
pub unsafe extern "C" fn onsig(signo: c_int) {
    if crate::jobs::vforked != 0 && libc::getpid() != crate::jobs::vforked {
        return;
    }

    if signo == libc::SIGCHLD {
        gotsigchld = 1;
        if trap[libc::SIGCHLD as usize].is_null() {
            return;
        }
    }

    gotsig[(signo - 1) as usize] = 1;
    pending_sig = signo;

    if signo == libc::SIGINT && trap[libc::SIGINT as usize].is_null() {
        if crate::error::suppressint == 0 {
            crate::error::onint();
        }
        crate::error::intpending = 1;
    }
}

/*
 * Called to execute a trap.  Perhaps we should avoid entering new trap
 * handlers while we are executing a trap handler.
 */

// [spec:dash:def:trap.dotrap-fn]
// [spec:dash:sem:trap.dotrap-fn]
pub unsafe fn dotrap() {
    let mut p: *mut c_char;
    let mut q: *mut c_char;
    let mut i: c_int;
    let mut status: c_int;
    let last_status: c_int;

    if pending_sig == 0 {
        return;
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

        p = trap[(i + 1) as usize];
        if p.is_null() {
            i += 1;
            q = q.add(1);
            continue;
        }
        crate::eval::evalstring(p, 0);
        if evalskip != SKIPFUNC {
            exitstatus = status;
        }
        i += 1;
        q = q.add(1);
    }

    savestatus = last_status;
}

/*
 * Controls whether the shell is interactive or not.
 */

// [spec:dash:def:trap.setinteractive-fn]
// [spec:dash:sem:trap.setinteractive-fn]
pub unsafe fn setinteractive(on: c_int) {
    static mut is_interactive: c_int = 0;

    let on = on + 1;
    if on == is_interactive {
        return;
    }
    is_interactive = on;
    setsignal(libc::SIGINT);
    setsignal(libc::SIGQUIT);
    setsignal(libc::SIGTERM);
}

/*
 * Called to exit the shell.
 */

// [spec:dash:def:trap.exitshell-fn]
// [spec:dash:sem:trap.exitshell-fn]
pub unsafe fn exitshell() -> ! {
    let mut loc: jmploc = jmploc::new();
    let locp: *mut jmploc = addr_of_mut!(loc);

    savestatus = exitstatus;
    crate::TRACE!("pid %d, exitshell(%d)\n", libc::getpid(), savestatus);
    /* `if (setjmp(loc.loc)) goto out;` — the body below is the fall-through. */
    crate::eval::setjmp_catch(locp, || {
        let p: *mut c_char;

        crate::error::handler = locp;
        'out: {
            p = trap[0];
            if !p.is_null() {
                trap[0] = null_mut();
                if ptrap != 0 {
                    break 'out;
                }
                evalskip = 0;
                crate::eval::evalstring(p, 0);
                evalskip = SKIPFUNCDEF;
            }
        }
    });
    /* out: */
    crate::init::exitreset();
    crate::init::postexitreset();
    /*
     * Disable job control so that whoever had the foreground before we
     * started can get it back.
     */
    crate::eval::setjmp_catch(locp, || {
        crate::jobs::setjobctl(0);
    });
    crate::output::flushall();
    libc::_exit(exitstatus);
    /* NOTREACHED */
}

// [spec:dash:def:trap.decode-signum-fn]
// [spec:dash:sem:trap.decode-signum-fn]
unsafe fn decode_signum(string: *const c_char) -> c_int {
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
        if libc::strcasecmp(string, crate::signames::signal_names[signo as usize].as_ptr()) == 0 {
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
