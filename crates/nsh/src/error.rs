//! Literal port of `src/error.c` / `src/error.h`.
//! Rules: `docs/spec/port/src/error.md`.
//!
//! Deviations forced by Rust, all noted inline:
//!
//! * The C variadic diagnostic entry points take a complete byte message in
//!   Rust. Callers compose typed values before crossing this boundary, so
//!   diagnostics do not need a second formatting language or a `va_list`.
//! * There is no `setjmp`/`longjmp` and no `jmp_buf`. The C's exception
//!   mechanism is gone; a failure is a value and it is returned. See
//!   `[dec:nsh:errors-are-values]` and `docs/errors-are-values.md`.

use core::ptr::addr_of_mut;
use core::sync::atomic::{Ordering, compiler_fence};
use std::ffi::CStr;
use std::io::Write;

use bstr::{BStr, BString, ByteSlice};
use libc::{c_char, c_int};

use crate::shell::cstr;

/*
 * Types of operations (passed to the errmsg routine).
 */

pub const E_OPEN: c_int = 0o1; /* opening a file */
pub const E_CREAT: c_int = 0o2; /* creating a file */
pub const E_EXEC: c_int = 0o4; /* executing a program */

/*
 * `sig_atomic_t` is not re-exported by the `libc` crate; on every
 * platform dash targets it is `int`.
 */
pub type sig_atomic_t = c_int;

/* `jmp_buf`, `struct jmploc`, the four exception codes, `handler` and
 * `exception` were all here, along with the C's comment about saving
 * `handler` on entry to an inner scope and restoring it on exit, and an
 * `extern "C"` block for `setjmp`. All of it is gone with the mechanism:
 * there is no buffer, no handler, and no nesting discipline to observe.
 * A frame that wants to know whether what it called failed reads the
 * `Result` the call returned.
 *
 * What replaced each code is worth naming once, because the C's four
 * integers were three different things:
 *
 *   EXERROR  a diagnostic          -> `Err(Error)`
 *   EXINT    the user's interrupt  -> `Err(Error::Interrupted)`
 *   EXEND    the shell is ending   -> `Ok(Flow::END)`
 *   EXEXIT   `exit` ran            -> `Ok(Flow::EXIT)`
 *
 * `[dec:nsh:errors-are-values]` is the decision that says the middle
 * column is the right division and the last two belong in the `Ok`
 * position; `docs/api-design.md` 3.1 is where the three-way split is
 * written down. `handler` is gone because nothing arms one, and it is
 * what `[dec:nsh:no-ambient-state]` was waiting for: a pointer into a
 * live stack frame cannot be a field of a `Shell`, and there is no longer
 * a pointer. */

pub static mut suppressint: c_int = 0;
pub static mut intpending: sig_atomic_t = 0;
pub static mut errlinno: c_int = 0;

/*
 * These macros allow the user to suspend the handling of interrupt signals
 * over a period of time.  This is similar to SIGHOLD to or sigblock, but
 * much more efficient and portable.  (But hacking the kernel is so much
 * more fun than worrying about efficiency and portability. :-))
 */

/* `#define barrier() ({ __asm__ __volatile__ ("": : :"memory"); })` */
#[inline(always)]
pub fn barrier() {
    compiler_fence(Ordering::SeqCst);
}

/* `#define INTOFF ({ suppressint++; barrier(); 0; })` */
#[inline(always)]
pub unsafe fn INTOFF() -> c_int {
    suppressint += 1;
    barrier();
    0
}

/// `#define INTON ({ barrier(); if (--suppressint == 0 && intpending) onint(); 0; })`
///
/// The `onint()` is gone and the rest is unchanged. That is step F, and
/// it is the whole of the divergence `docs/divergences.md`'s
/// `error.interrupt-delivery-point` records: the C delivers a pending
/// interrupt at the instruction where the counter reaches zero, and this
/// leaves `intpending` set for the next poll site to take.
///
/// **`INTON` stays infallible, deliberately.** §4.3 measured what making
/// it fallible costs — 44 functions enter the fixpoint, and they are the
/// shell's teardown: `popredir`, `unwindredir`, `unwindfiles`,
/// `popallfiles`, `exitreset`, `freejob`, `ifsfree`. A design in which
/// cleanup can fail while handling a failure is the wrong shape, and
/// every call site would have to decide what to do with an error raised
/// while handling an error.
#[inline(always)]
pub unsafe fn INTON() -> c_int {
    barrier();
    suppressint -= 1;
    0
}

/// `#define FORCEINTON ({ barrier(); suppressint = 0; if (intpending) onint(); 0; })`
///
/// Same change, same reason. This one *resets* the counter rather than
/// balancing it (§2.4), which is what makes it the top level's way of
/// discarding a leak; discarding the leak and taking delivery were one
/// operation in the C and are two now.
#[inline(always)]
pub unsafe fn FORCEINTON() -> c_int {
    barrier();
    suppressint = 0;
    0
}

/* `#define CLEAR_PENDING_INT intpending = 0` */
#[inline(always)]
pub unsafe fn CLEAR_PENDING_INT() {
    core::ptr::write_volatile(addr_of_mut!(intpending), 0);
}

/// Take delivery of a pending interrupt, if one is due.
///
/// The question every poll site asks, in one place so that all of them
/// ask it the same way. "Due" is *pending* and *not suppressed*: an
/// `INTOFF` bracket still holds the interrupt off, exactly as it held off
/// the C's asynchronous delivery, because the bracket is what makes the
/// mutation inside it atomic against a signal.
///
/// There are five poll sites, and they are the places the shell reaches
/// on its own rather than the places a signal happens to arrive:
/// `trap::dotrap`, which `evaltree` calls before and after every command
/// and which is therefore the one that matters most; and the four `EINTR`
/// returns where a blocking syscall came back — `redir::sh_open`,
/// `input::preadfd`, `expand::expbackq`'s command-substitution read, and
/// `jobs::waitproc`'s `wait3`. `output.rs`'s `write` is deliberately not
/// one: dash collects output errors in `outerr` and checks them
/// separately rather than raising, and making the output path fallible is
/// the shape §4.3 argues against.
///
/// Returns `Some` at most once per interrupt: [`onint`] clears
/// `intpending` as it delivers.
#[inline]
pub unsafe fn poll_interrupt() -> Option<Error> {
    if suppressint == 0 && int_pending() != 0 {
        Some(onint())
    } else {
        None
    }
}

/// Put a taken interrupt back, for a frame that cannot carry it out.
///
/// [`poll_interrupt`] takes delivery, which means it *clears*
/// `intpending`; a frame that then drops the value has lost the
/// interrupt, and the shell stops answering `^C`. One frame is in that
/// position and cannot be moved out of it: `parser::getprompt` is a
/// callback the line editor calls through a function pointer, so it has
/// no `Result` to return and no caller of its own to return it to.
///
/// The C's answer there was to longjmp out of the line editor, through
/// frames a C library owns — the same shape as
/// `expand::opendir_interruptible` unwinding out of `glob`, and the same
/// reason it cannot survive `panic = "abort"`. This is the honest
/// alternative: the interrupt goes back in the inbox and the next poll
/// site takes it, which is one prompt-expansion later.
pub unsafe fn rearm_interrupt(e: Error) {
    debug_assert!(
        e.is_interrupt(),
        "only an interrupt may be put back; a diagnostic has already been written"
    );
    drop(e);
    core::ptr::write_volatile(addr_of_mut!(intpending), 1);
}

/* `#define int_pending() intpending` */
#[inline(always)]
pub unsafe fn int_pending() -> sig_atomic_t {
    core::ptr::read_volatile(addr_of_mut!(intpending))
}

/* `#define INTOFF` — macro spelling, for call sites that keep the C shape. */
#[macro_export]
macro_rules! INTOFF {
    () => {
        $crate::error::INTOFF()
    };
}

/* `#define INTON` — macro spelling. */
#[macro_export]
macro_rules! INTON {
    () => {
        $crate::error::INTON()
    };
}

/* `#define FORCEINTON` — macro spelling. */
#[macro_export]
macro_rules! FORCEINTON {
    () => {
        $crate::error::FORCEINTON()
    };
}

/* `#define SAVEINT(v) ((v) = suppressint)` */
#[macro_export]
macro_rules! SAVEINT {
    ($v:expr) => {
        $v = $crate::error::suppressint
    };
}

/*
 * ```c
 * #define RESTOREINT(v) \
 *	({ barrier(); if ((suppressint = (v)) == 0 && intpending) onint(); 0; })
 * ```
 */
#[macro_export]
macro_rules! RESTOREINT {
    ($v:expr) => {{
        /* The `if (... && intpending) onint()` is gone with the one in
         * `INTON`; see there. */
        $crate::error::barrier();
        $crate::error::suppressint = $v;
        0
    }};
}

/*
 * Called from trap.c when a SIGINT is received.  (If the user specifies
 * that SIGINT is to be trapped or ignored using the trap builtin, then
 * this routine is not called.)  Suppressint is nonzero when interrupts
 * are held using the INTOFF macro.  (The test for iflag is just
 * defensive programming.)
 */

/// Take delivery of a pending interrupt, as a value.
///
/// The C raises `EXINT` from here and never returns. This returns the
/// interrupt instead, and the change of shape is the whole of step F:
/// `onsig` no longer calls it from inside the signal handler, and `INTON`
/// no longer calls it when the counter reaches zero. It is called only
/// from a *poll site* — a place the shell reached on its own and that can
/// return a `Result`.
///
/// Clearing `intpending` is the delivery. After this returns, the
/// interrupt has been taken and the next poll site must not take it
/// again; that is why the poll sites call this rather than reading the
/// flag and building an `Error` themselves.
///
/// It still does not always return. When the shell is not an interactive
/// root shell it restores `SIG_DFL` and re-raises, so the process dies of
/// the signal, which is what a shell must do to report the right status
/// to its parent. That is a terminating operation in libc, not a
/// non-local jump, and `panic = "abort"` cannot break it.
/// `docs/api-design.md` §3.4 wants this half in `nsh-cli` eventually; it
/// is a frontend boundary question and not this node's.
// [spec:dash:def:error.onint-fn]
// [spec:dash:sem:error.onint-fn]
pub unsafe fn onint() -> Error {
    core::ptr::write_volatile(addr_of_mut!(intpending), 0);
    crate::system::sigclearmask();
    /* `#define rootshell (!shlvl)` (main.h); `#define iflag optlist[3]`. */
    let rootshell: bool = crate::shellmain::shlvl == 0;
    let iflag: c_char = crate::options::optlist[crate::options::iflag];
    if !(rootshell && iflag != 0) {
        libc::signal(libc::SIGINT as c_int, libc::SIG_DFL);
        libc::raise(libc::SIGINT);
    }
    crate::eval::exitstatus = libc::SIGINT + 128;
    Error::Interrupted {
        signal: libc::SIGINT,
    }
}

/* `exvwarning2` is not a separate function here. In the C it exists only
 * to accept the `va_list` that `sh_warnx`'s varargs collected, and the
 * two have the same body otherwise. A message is a `&[u8]` now, so there
 * is nothing for the inner one to accept and `sh_warnx` below carries
 * both rules. */

/// A shell diagnostic, as a value ([dec:nsh:errors-are-values]).
///
/// Every one of these is also *written* to the shell's stderr at the point
/// it happened, in dash's bytes and dash's order — see [`report`], which is
/// the only constructor that should reach a caller. That is not redundancy:
/// `tests/harness/dscase.sh` merges stdout and stderr and compares the
/// result, so where a diagnostic lands in the stream is under test in every
/// corpus case, and a design that returned the text instead of writing it
/// would emit every diagnostic at the end of the run.
///
/// Control flow is deliberately not here. `exit`, `return`, `break`,
/// `continue` and the `set -e` abort are not errors and must not sit in the
/// `Err` position; they keep the exception codes for now and become `Flow`
/// in the `Ok` position later in this node.
///
/// One variant so far. `docs/api-design.md` §3.4 names ten and says why the
/// conversion starts with `Other` alone: every raise site can be rewritten
/// mechanically and the interesting ones promoted afterwards, instead of
/// needing the final taxonomy before the first commit.
/// Constructing one of these is not the same as raising it, and dropping it
/// on the floor is how a diagnostic gets written while the shell carries on
/// past a failure it should have abandoned. That is a silent wrong answer
/// rather than a crash, which is the failure mode
/// `docs/errors-are-values.md` §6 names as the dangerous one for this whole
/// conversion -- and it happened: `redir::sh_open_fail` stopped diverging
/// when it started returning a value, and two `goto ecreate` sites fell
/// through instead of stopping, printing the diagnostic twice and then
/// redirecting to a descriptor that was never opened. The corpus caught it.
/// This attribute is what makes the compiler catch the next one.
#[must_use = "an Error that is built and not returned reports a failure the shell then ignores"]
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The user interrupted the shell.
    ///
    /// Kept apart from every other failure because a host has to tell
    /// "your script failed" from "the user pressed ^C"
    /// (`docs/api-design.md` §3.4), and because the frames that swallow a
    /// diagnostic must not swallow this one: `evalcommand` reports a
    /// non-special built-in's error and carries on, and an interrupt
    /// arriving the same way has to keep going.
    ///
    /// It is the first variant promoted out of `Other`, which is what
    /// §3.4 said the taxonomy would do: start with `Other` so every raise
    /// site converts mechanically, promote the interesting ones after.
    Interrupted {
        /// The signal that arrived. Always `SIGINT` today — it is the only
        /// one `onsig` delivers this way — and a number rather than a
        /// `Signal` newtype, which is `public-api`'s to introduce.
        signal: c_int,
    },
    /// A diagnostic with no more specific variant.
    Other {
        /// `errlinno` as it stood when the diagnostic was produced.
        line: c_int,
        /// The status the shell takes from it.
        status: c_int,
        /// dash's text, without the `sh: N: cd: ` prefix.
        message: BString,
    },
}

impl Error {
    /// A diagnostic with no more specific variant, at the current line and
    /// the status the shell has already taken.
    ///
    /// `sh_error` sets `exitstatus` to 2 before it reports, and `exerror`
    /// leaves whatever the site chose, so reading it here captures what
    /// each site meant without a second parameter.
    pub unsafe fn other(msg: &[u8]) -> Error {
        Error::Other {
            line: errlinno,
            status: crate::eval::exitstatus,
            message: BString::from(msg),
        }
    }

    /// A failure whose diagnostic has **already been written**, with no
    /// text of its own.
    ///
    /// The C raises `EXERROR` with no message where the thing that failed
    /// wrote its own diagnostic and then returned normally — `evalcommand`'s
    /// `bail:` after a `CMDUNKNOWN`, where `find_command` reported "not
    /// found" and came back. There is no value to carry there and dash
    /// carries none; this is that, as a value, so the frame can `return
    /// Err` instead of raising.
    ///
    /// Nothing writes it: [`report`] runs at construction and this is not
    /// constructed through it. An empty message is therefore never
    /// rendered, and it must stay that way — a caller that reports one of
    /// these would emit a bare prefix and a newline dash does not.
    pub unsafe fn reported(status: c_int) -> Error {
        Error::Other {
            line: errlinno,
            status,
            message: BString::default(),
        }
    }

    /// Is this the user's interrupt rather than a diagnostic?
    ///
    /// The question every frame that swallows an error has to ask. There
    /// are four of them — `evalcommand`'s built-in arm, `redirectsafe`,
    /// `expandstr` and `exitshell`'s EXIT trap — and each used to ask it
    /// of `error::exception` after a longjmp.
    pub fn is_interrupt(&self) -> bool {
        matches!(self, Error::Interrupted { .. })
    }

    /// The exit status the shell takes from this error.
    pub fn status(&self) -> c_int {
        match self {
            /* `onint` sets `exitstatus` to this before it returns, as the
             * C does before it raises. */
            Error::Interrupted { signal } => signal + 128,
            Error::Other { status, .. } => *status,
        }
    }

    /// dash's text for this error, byte for byte, **without** the
    /// `sh: 1: cd: ` prefix.
    ///
    /// The prefix is `$0`, `errlinno` and the running command's name, which
    /// are shell state and not error state, so an `Error` on its own cannot
    /// render them. [`sh_warnx`] adds them when it writes.
    pub fn message(&self) -> &BStr {
        match self {
            /* dash prints nothing for an interrupt. `main`'s handler
             * writes a bare newline and that is the whole of it. */
            Error::Interrupted { .. } => BStr::new(b""),
            Error::Other { message, .. } => message.as_bstr(),
        }
    }

    /// The line the error was reported at.
    pub fn line(&self) -> c_int {
        match self {
            /* No line: an interrupt did not happen *at* a line the way a
             * diagnostic did, and reading `errlinno` here would report
             * whichever line last failed. */
            Error::Interrupted { .. } => 0,
            Error::Other { line, .. } => *line,
        }
    }
}

/// Write a diagnostic where dash writes it, and hand it back as a value.
///
/// This is [`exverror`] with the raise removed, and it is the funnel every
/// diagnostic goes through: the bytes on the stream are rendered from the
/// same `Error` that is returned, so the two cannot drift.
///
/// Two details of dash's write are load-bearing and are preserved by doing
/// nothing more than the C does. `errout` is unbuffered, so the message is
/// three raw `write(2)`s and needs no flush of its own; and `flushall()`
/// runs *after* the message, so a built-in that filled the stdout buffer and
/// then failed produces its diagnostic before its own output in the merged
/// stream. Both are pinned by the corpus.
pub unsafe fn report(e: Error) -> Error {
    sh_warnx(e.message());

    crate::output::flushall();
    e
}

/// `sh_error`'s value half: take the status dash takes, write the
/// diagnostic where dash writes it, and **return** the error rather than
/// raising it.
///
/// This is what a converted raise site calls —
/// `return Err(sh_error_value(&msg))` — and it is the same three writes in
/// the same order as the diverging form below, because both are this
/// function. When the last caller of `sh_error` is gone this one takes its
/// name.
pub unsafe fn sh_error_value(msg: &[u8]) -> Error {
    crate::eval::exitstatus = 2;

    report(Error::other(msg))
}

/*
 * error/warning routines for external builtins
 */

// [spec:dash:def:error.sh-warnx-fn]
// [spec:dash:sem:error.sh-warnx-fn]
// [spec:dash:def:error.exvwarning2-fn]
// [spec:dash:sem:error.exvwarning2-fn]
pub unsafe fn sh_warnx(msg: &[u8]) {
    let name = if !crate::options::arg0.is_null() {
        crate::options::arg0
    } else {
        cstr(b"sh\0")
    };

    let mut prefix = Vec::new();
    prefix.extend_from_slice(CStr::from_ptr(name).to_bytes());
    prefix.extend_from_slice(b": ");
    let line = errlinno;
    write!(&mut prefix, "{line}").expect("writing to a Vec cannot fail");
    prefix.extend_from_slice(b": ");
    if let Some(name) = &*core::ptr::addr_of!(crate::eval::commandname) {
        prefix.extend_from_slice(name);
        prefix.extend_from_slice(b": ");
    }

    /* stderr is unbuffered. Keep the C's three output operations visible:
     * prefix, complete message body, then newline. */
    let errs = &mut *crate::output::stderr();
    let _ = errs.write_all(&prefix);
    let _ = errs.write_all(msg);
    let _ = errs.write_all(b"\n");
}

/*
 * Return a string describing an error.  The returned string may be a
 * pointer to a static buffer that will be overwritten on the next call.
 * Action describes the operation that got the error.
 */

// [spec:dash:def:error.errmsg-fn]
// [spec:dash:sem:error.errmsg-fn]
pub unsafe fn errmsg(e: c_int, action: c_int) -> *const c_char {
    if e != libc::ENOENT && e != libc::ENOTDIR {
        return libc::strerror(e);
    }

    if action & E_OPEN != 0 {
        cstr(b"No such file\0")
    } else if action & E_CREAT != 0 {
        cstr(b"Directory nonexistent\0")
    } else {
        cstr(b"not found\0")
    }
}

/*
 * `#ifdef REALLY_SMALL` — out-of-line body of INTON.  REALLY_SMALL is
 * not defined in the shipped build, so this is never called; it is kept
 * so the symbol has a home and stays in step with `INTON` above.
 */
// [spec:dash:def:error.inton-fn]
// [spec:dash:sem:error.inton-fn]
pub unsafe fn __inton() {
    /* In step with `INTON` above, including the `onint()` it no longer
     * makes. */
    suppressint -= 1;
}

/* There is no setjmp/longjmp here, no stand-in for one, and no FFI
 * declaration of either — and now no `catch_unwind` and no `panic_any`
 * either. The last of it went with `errors-are-values`.
 *
 * The port never used libc's `longjmp`: a `jmploc` armed by
 * `eval::setjmp_catch` was a `catch_unwind`, not a real jump buffer, and
 * handing one to `longjmp` is undefined and in practice segfaulted. That
 * was a real bug here on every fork and exit path. Reintroducing a shim
 * would make it easy to recreate, and there is nothing left that would
 * want one: a failure is a value, and it is returned.
 *
 * `panic = "abort"` is therefore sound for this crate, which is the
 * consequence `[dec:nsh:errors-are-values]` exists to deliver. */

#[cfg(test)]
mod tests {
    use super::*;

    /* The funnel itself. `docs/errors-are-values.md` §5 lists the error
     * value first among what the differential harness cannot see: it
     * compares bytes on a stream, and the value never reaches it. These
     * assert the half the corpus cannot, and they are deliberately about
     * `sh_error_value` rather than `sh_error`, because the diverging form
     * is now defined as the value form plus a jump. */

    #[test]
    fn reported_error_carries_its_status() {
        let _g = crate::testutil::lock();
        unsafe {
            crate::eval::exitstatus = 0;
            let e = sh_error_value(b"a diagnostic");

            /* `sh_error` sets 2 before it reports, and the value has to
             * carry what the site meant rather than what the global says
             * later, so that propagation through any number of `?` cannot
             * lose it. */
            assert_eq!(e.status(), 2);
            let took = crate::eval::exitstatus;
            assert_eq!(took, 2);
            assert_eq!(e.message().to_vec(), b"a diagnostic".to_vec());
        }
    }

    #[test]
    fn message_drops_the_prefix() {
        let _g = crate::testutil::lock();
        unsafe {
            let saved = errlinno;
            errlinno = 17;
            let e = report(Error::other(b"cd: bad directory"));
            errlinno = saved;

            /* The `sh: 17: ` prefix is `arg0`, `errlinno` and the running
             * command's name -- shell state, not error state -- so
             * `sh_warnx` adds it on the way out and the value does not
             * carry it. */
            assert_eq!(e.message().to_vec(), b"cd: bad directory".to_vec());
            assert_eq!(e.line(), 17);
        }
    }

    #[test]
    fn exend_keeps_its_own_status() {
        let _g = crate::testutil::lock();
        unsafe {
            /* `shellexec` reports its text and takes 127 or 126, then
             * raises EXEND. The status travels with the value even though
             * the code that goes with it does not. */
            crate::eval::exitstatus = 127;
            let e = report(Error::other(b"nosuchcmd: not found"));

            assert_eq!(e.status(), 127);
        }
    }

    /// Arrange for `onint` to be able to *return*.
    ///
    /// It restores `SIG_DFL` and re-raises unless the shell is an
    /// interactive root shell, which in a test process means the test
    /// dies of SIGINT. That branch is dash's and is deliberate; these
    /// cases are about the other one.
    unsafe fn as_interactive_root() -> c_char {
        let saved = crate::options::optlist[crate::options::iflag];
        crate::options::optlist[crate::options::iflag] = 1;
        /* Copied out: a shared reference to a mutable static is what the
         * lint forbids, and `assert_eq!` takes one. */
        let lvl = crate::shellmain::shlvl;
        assert_eq!(lvl, 0, "a test process is a root shell");
        saved
    }

    /// An interrupt is a value, it knows it is one, and it carries dash's
    /// status.
    // [spec:dash:sem:error.onint-fn/test]
    #[test]
    fn an_interrupt_is_a_value() {
        let _g = crate::testutil::lock();
        unsafe {
            let saved = as_interactive_root();
            let saved_status = crate::eval::exitstatus;
            CLEAR_PENDING_INT();

            let e = onint();

            assert!(e.is_interrupt());
            assert_eq!(e.status(), libc::SIGINT + 128);
            let status = crate::eval::exitstatus;
            assert_eq!(status, libc::SIGINT + 128);
            assert!(e.message().is_empty(), "dash prints nothing for a ^C");

            crate::eval::exitstatus = saved_status;
            crate::options::optlist[crate::options::iflag] = saved;
        }
    }

    /// `poll_interrupt` takes delivery once and only once: `onint` clears
    /// the flag as it hands the value over, so a second poll finds
    /// nothing. A frame that drops the value has lost the user's ^C,
    /// which is what `rearm_interrupt` exists for.
    // [spec:dash:sem:error.onint-fn/test]
    #[test]
    fn delivery_happens_once() {
        let _g = crate::testutil::lock();
        unsafe {
            let saved = as_interactive_root();
            let saved_status = crate::eval::exitstatus;
            suppressint = 0;
            core::ptr::write_volatile(addr_of_mut!(intpending), 1);

            assert!(poll_interrupt().is_some(), "one pending interrupt, one delivery");
            assert!(poll_interrupt().is_none(), "and not a second time");

            crate::eval::exitstatus = saved_status;
            crate::options::optlist[crate::options::iflag] = saved;
        }
    }

    /// **The INTOFF discipline, which the polling must not break.** An
    /// interrupt that arrives inside an `INTOFF` bracket is pending but
    /// not *due*, and no poll site may take it there -- the bracket is
    /// what makes the mutation inside it atomic against a signal. This is
    /// the one property that distinguishes "delivery moved to a poll
    /// site" from "delivery moved anywhere at all".
    // [spec:dash:sem:error.inton-fn/test]
    #[test]
    fn intoff_still_holds_it_off() {
        let _g = crate::testutil::lock();
        unsafe {
            let saved = as_interactive_root();
            suppressint = 0;
            core::ptr::write_volatile(addr_of_mut!(intpending), 1);

            INTOFF();
            assert!(poll_interrupt().is_none(), "suppressed: not due");
            /* And `INTON` does not deliver it either -- that is the
             * divergence, and it is why the counter reaching zero is no
             * longer a delivery point. */
            INTON();
            assert_eq!(int_pending(), 1, "still pending, waiting for a poll site");
            assert!(poll_interrupt().is_some(), "and due again once unsuppressed");

            crate::options::optlist[crate::options::iflag] = saved;
        }
    }

    /// A frame that cannot carry the interrupt out puts it back rather
    /// than losing it.
    // [spec:dash:sem:error.onint-fn/test]
    #[test]
    fn a_rearmed_interrupt_is_taken_later() {
        let _g = crate::testutil::lock();
        unsafe {
            let saved = as_interactive_root();
            suppressint = 0;
            CLEAR_PENDING_INT();

            rearm_interrupt(Error::Interrupted {
                signal: libc::SIGINT,
            });
            assert_eq!(int_pending(), 1);
            assert!(poll_interrupt().is_some(), "the next poll site takes it");

            crate::options::optlist[crate::options::iflag] = saved;
        }
    }
}
