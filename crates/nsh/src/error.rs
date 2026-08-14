//! Literal port of `src/error.c` / `src/error.h`.
//! Rules: `docs/spec/port/src/error.md`.
//!
//! Deviations forced by Rust, all noted inline:
//!
//! * The C variadic diagnostic entry points take a complete byte message in
//!   Rust. Callers compose typed values before crossing this boundary, so
//!   diagnostics do not need a second formatting language or a `va_list`.
//! * `setjmp`/`longjmp` are used through FFI.  `jmp_buf` is an opaque,
//!   over-sized, 16-byte-aligned buffer so it fits any libc's layout.

use core::ptr::addr_of_mut;
use core::sync::atomic::{Ordering, compiler_fence};
use std::ffi::CStr;
use std::io::Write;

use bstr::{BStr, BString, ByteSlice};
use libc::{c_char, c_int};

use crate::shell::{DEBUG, cstr};

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

/*
 * Opaque stand-in for libc's `jmp_buf`.  512 bytes at 16-byte alignment
 * covers every layout dash is built against (glibc/x86-64 needs 200).
 */
#[repr(C, align(16))]
#[derive(Copy, Clone)]
pub struct jmp_buf(pub [u8; 512]);

unsafe extern "C" {
    /*
     * Calling `setjmp` through FFI is the only option available: Rust
     * has no intrinsic for it.  Its "returns twice" behaviour is why
     * the caller must keep the enclosing frame simple.
     */
}

/*
 * We enclose jmp_buf in a structure so that we can declare pointers to
 * jump locations.  The global variable handler contains the location to
 * jump to when an exception occurs, and the global variable exception
 * contains a code identifying the exeception.  To implement nested
 * exception handlers, the user should save the value of handler on entry
 * to an inner scope, set handler to point to a jmploc structure for the
 * inner scope, and restore handler on exit from the scope.
 */

// [spec:dash:def:error.jmploc]
#[repr(C)]
#[derive(Copy, Clone)]
pub struct jmploc {
    pub loc: jmp_buf,
}

impl jmploc {
    /* C declares `struct jmploc jmploc;` uninitialised; this is the
     * equivalent fresh, never-read buffer. */
    pub const fn new() -> jmploc {
        jmploc {
            loc: jmp_buf([0; 512]),
        }
    }
}

/* exceptions */
pub const EXINT: c_int = 0; /* SIGINT received */
pub const EXERROR: c_int = 1; /* a generic error */
pub const EXEND: c_int = 3; /* exit the shell */
pub const EXEXIT: c_int = 4; /* exit the shell via exitcmd */

/*
 * Code to handle exceptions in C.
 */

pub static mut handler: *mut jmploc = core::ptr::null_mut();
pub static mut exception: c_int = 0;
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

/* `#define INTON ({ barrier(); if (--suppressint == 0 && intpending) onint(); 0; })` */
#[inline(always)]
pub unsafe fn INTON() -> c_int {
    barrier();
    suppressint -= 1;
    if suppressint == 0 && core::ptr::read_volatile(addr_of_mut!(intpending)) != 0 {
        onint();
    }
    0
}

/* `#define FORCEINTON ({ barrier(); suppressint = 0; if (intpending) onint(); 0; })` */
#[inline(always)]
pub unsafe fn FORCEINTON() -> c_int {
    barrier();
    suppressint = 0;
    if core::ptr::read_volatile(addr_of_mut!(intpending)) != 0 {
        onint();
    }
    0
}

/* `#define CLEAR_PENDING_INT intpending = 0` */
#[inline(always)]
pub unsafe fn CLEAR_PENDING_INT() {
    core::ptr::write_volatile(addr_of_mut!(intpending), 0);
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
        $crate::error::barrier();
        $crate::error::suppressint = $v;
        if $crate::error::suppressint == 0 && $crate::error::int_pending() != 0 {
            $crate::error::onint();
        }
        0
    }};
}

/*
 * Called to raise an exception.  Since C doesn't include exceptions, we
 * just do a longjmp to the exception handler.  The type of exception is
 * stored in the global variable "exception".
 */

// [spec:dash:def:error.exraise-fn]
// [spec:dash:sem:error.exraise-fn]
pub unsafe fn exraise(e: c_int) -> ! {
    if DEBUG {
        if handler.is_null() {
            std::process::abort();
        }
    }

    if crate::jobs::vforked != 0 {
        crate::shell::flush_coverage();
        libc::_exit(crate::eval::exitstatus);
    }

    INTOFF();

    exception = e;
    /* The C is `longjmp(handler->loc, 1)`. Handlers in this port are
     * established by `eval::setjmp_catch` (catch_unwind over a typed
     * payload), not by a real `setjmp`, so `handler->loc` is never a
     * live jump buffer — calling the libc `longjmp` on it is undefined
     * and crashes. Raise the payload the handlers actually catch.
     * `setjmp_catch` compares `loc` and resumes any payload aimed at an
     * outer handler, so nesting behaves as the C's does. */
    raise_longjmp(handler, 1);
}

/* The unwind-based counterpart of `longjmp(loc, val)`. Every exception
 * path in the port funnels through here so exactly one mechanism is in
 * play; see `eval::setjmp_catch` for the catching half. */
pub unsafe fn raise_longjmp(loc: *mut jmploc, val: c_int) -> ! {
    std::panic::panic_any(Longjmp { loc, val })
}

/*
 * Called from trap.c when a SIGINT is received.  (If the user specifies
 * that SIGINT is to be trapped or ignored using the trap builtin, then
 * this routine is not called.)  Suppressint is nonzero when interrupts
 * are held using the INTOFF macro.  (The test for iflag is just
 * defensive programming.)
 */

// [spec:dash:def:error.onint-fn]
// [spec:dash:sem:error.onint-fn]
pub unsafe fn onint() -> ! {
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
    exraise(EXINT);
    /* NOTREACHED */
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

    /// The exit status the shell takes from this error.
    pub fn status(&self) -> c_int {
        match self {
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
            Error::Other { message, .. } => message.as_bstr(),
        }
    }

    /// The line the error was reported at.
    pub fn line(&self) -> c_int {
        match self {
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

/// Perform the legacy non-local jump for an error that has **already been
/// reported**.
///
/// This is the bridge that lets the conversion proceed one function at a
/// time. A caller that has not been converted yet writes
/// `f(..).unwrap_or_else(|e| raise_reported(EXERROR, e))` over a callee that
/// already returns `Result`, so the wavefront can move outward from the
/// raise sites towards the catch frames with the harness green at every
/// commit. It is deleted with the rest of the longjmp machinery at the end
/// of this node.
///
/// It writes nothing. The diagnostic went out when [`report`] built the
/// value, which is where dash writes it and where it has to stay.
///
/// `cond` is a parameter rather than a property of the `Error` because two
/// of the four exception codes are control flow and not error at all —
/// `shellexec` reports its text and raises `EXEND` — and folding those into
/// the value is precisely what [dec:nsh:errors-are-values] forbids. The
/// parameter disappears when `Flow` takes the control-flow codes out of the
/// exception mechanism entirely.
pub unsafe fn raise_reported(cond: c_int, e: Error) -> ! {
    /* The status the raise site chose travels with the value, so
     * propagation through any number of `?` cannot lose it. Everything
     * between `report` and here is `flushall`, which swallows its errors
     * and cannot touch `exitstatus`, so this assignment is a no-op today
     * and an invariant once the value does the travelling. */
    crate::eval::exitstatus = e.status();

    exraise(cond);
    /* NOTREACHED */
}

/*
 * Exverror is called to print a complete diagnostic message and raise the
 * error exception.
 */
// [spec:dash:def:error.exverror-fn]
// [spec:dash:sem:error.exverror-fn]
unsafe fn exverror(cond: c_int, msg: &[u8]) -> ! {
    /*
     * #ifdef DEBUG
     *	if (msg) { va_list aq; TRACE(("exverror(%d, \"", cond));
     *		   va_copy(aq, ap); TRACEV((msg, aq)); va_end(aq);
     *		   TRACE(("\") pid=%d\n", getpid())); }
     *	else TRACE(("exverror(%d, NULL) pid=%d\n", cond, getpid()));
     *	if (msg)
     * #endif
     *
     * Without DEBUG the `if (msg)` guard is compiled out along with the
     * tracing, so exvwarning runs unconditionally.
     */
    /* `exverror` is now exactly its two halves: build-and-write the value,
     * then jump with it. Callers that still expect a diverging raise keep
     * this function; the wavefront that replaces it with `Err(e)` starts at
     * the leaves and works outward. */
    raise_reported(cond, report(Error::other(msg)))
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

// [spec:dash:def:error.sh-error-fn]
// [spec:dash:sem:error.sh-error-fn]
pub unsafe fn sh_error(msg: &[u8]) -> ! {
    raise_reported(EXERROR, sh_error_value(msg))
    /* NOTREACHED */
}

// [spec:dash:def:error.exerror-fn]
// [spec:dash:sem:error.exerror-fn]
pub unsafe fn exerror(cond: c_int, msg: &[u8]) -> ! {
    exverror(cond, msg);
    /* NOTREACHED */
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
    suppressint -= 1;
    if suppressint == 0 && core::ptr::read_volatile(addr_of_mut!(intpending)) != 0 {
        onint();
    }
}

/* Payload for the catch_unwind-based stand-in for longjmp (see
 * `eval::setjmp_catch`). It carries the target so a payload aimed at an
 * outer handler is resumed rather than swallowed. */
pub struct Longjmp {
    pub loc: *mut jmploc,
    pub val: c_int,
}
unsafe impl Send for Longjmp {}

/* There is deliberately no wrapper around libc's setjmp/longjmp here, and
 * no call site anywhere in the port.
 *
 * Every `jmploc` in this port is armed by `eval::setjmp_catch`, which is a
 * `catch_unwind`, not a real `setjmp` — so a `jmploc.loc` is never a live
 * jump buffer. Handing one to libc's `longjmp` is undefined and in
 * practice segfaults; that was a real bug here, on every fork and exit
 * path, until `exraise` was changed to `raise_longjmp` (see above).
 *
 * Reintroducing a setjmp/longjmp shim would make that failure easy to
 * recreate, so the FFI declarations are gone too. Use `setjmp_catch` to
 * arm a handler and `raise_longjmp` to jump to one. */

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
}
