//! Literal port of `src/error.c` / `src/error.h`.
//! Rules: `docs/spec/port/src/error.md`.
//!
//! Deviations forced by Rust, all noted inline:
//!
//! * C variadic functions cannot be *defined* in stable Rust, so
//!   `sh_error`, `exerror` and `sh_warnx` take `&[VaArg]` (see
//!   `crate::output::VaArg`) in place of `...`, and `va_list` is that
//!   same slice.  `va_start`/`va_end` disappear; `va_copy` is a copy of
//!   the slice reference.
//! * `setjmp`/`longjmp` are used through FFI.  `jmp_buf` is an opaque,
//!   over-sized, 16-byte-aligned buffer so it fits any libc's layout.

use core::ptr::addr_of_mut;
use core::sync::atomic::{compiler_fence, Ordering};

use libc::{c_char, c_int, c_void};

use crate::output::VaArg;
use crate::shell::{cstr, DEBUG};

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

extern "C" {
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
            libc::abort();
        }
    }

    if crate::jobs::vforked != 0 {
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

// [spec:dash:def:error.exvwarning2-fn]
// [spec:dash:sem:error.exvwarning2-fn]
unsafe fn exvwarning2(msg: *const c_char, ap: &[VaArg]) {
    let errs: *mut crate::output::output;
    let name: *const c_char;
    let fmt: *const c_char;

    errs = crate::output::out2;
    name = if !crate::options::arg0.is_null() {
        crate::options::arg0
    } else {
        cstr(b"sh\0")
    };
    if crate::eval::commandname.is_null() {
        fmt = cstr(b"%s: %d: \0");
    } else {
        fmt = cstr(b"%s: %d: %s: \0");
    }
    crate::output::outfmt(
        errs,
        fmt,
        &[
            VaArg::Str(name),
            VaArg::Int(errlinno),
            VaArg::Str(crate::eval::commandname),
        ],
    );
    crate::output::doformat(errs, msg, ap);
    /* FLUSHERR is not defined in the shipped build, so: outcslow. */
    crate::output::outcslow('\n' as c_int, errs);
}

/* `#define exvwarning(a, b, c) exvwarning2(b, c)` */
macro_rules! exvwarning {
    ($a:expr, $b:expr, $c:expr) => {
        exvwarning2($b, $c)
    };
}

/*
 * Exverror is called to raise the error exception.  If the second argument
 * is not NULL then error prints an error message using printf style
 * formatting.  It then raises the error exception.
 */
// [spec:dash:def:error.exverror-fn]
// [spec:dash:sem:error.exverror-fn]
unsafe fn exverror(cond: c_int, msg: *const c_char, ap: &[VaArg]) -> ! {
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
    exvwarning!(-1, msg, ap);

    crate::output::flushall();
    exraise(cond);
    /* NOTREACHED */
}

// [spec:dash:def:error.sh-error-fn]
// [spec:dash:sem:error.sh-error-fn]
pub unsafe fn sh_error(msg: *const c_char, ap: &[VaArg]) -> ! {
    crate::eval::exitstatus = 2;

    exverror(EXERROR, msg, ap);
    /* NOTREACHED */
}

// [spec:dash:def:error.exerror-fn]
// [spec:dash:sem:error.exerror-fn]
pub unsafe fn exerror(cond: c_int, msg: *const c_char, ap: &[VaArg]) -> ! {
    exverror(cond, msg, ap);
    /* NOTREACHED */
}

/*
 * error/warning routines for external builtins
 */

// [spec:dash:def:error.sh-warnx-fn]
// [spec:dash:sem:error.sh-warnx-fn]
pub unsafe fn sh_warnx(fmt: *const c_char, ap: &[VaArg]) {
    exvwarning!(-1, fmt, ap);
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

// ---------------------------------------------------------------------
// Variadic compatibility layer — see the note in `output.rs`. These
// restore the C call shape for the error entry points; `sh_error` and
// `exerror` diverge, so the macros are usable in expression position
// where the C uses them as statements that do not return.
// ---------------------------------------------------------------------

#[macro_export]
macro_rules! sh_error {
    ($fmt:expr $(, $arg:expr)* $(,)?) => {
        $crate::error::sh_error($fmt, &[$($crate::output::VaArg::from($arg)),*])
    };
}

#[macro_export]
macro_rules! sh_warnx {
    ($fmt:expr $(, $arg:expr)* $(,)?) => {
        $crate::error::sh_warnx($fmt, &[$($crate::output::VaArg::from($arg)),*])
    };
}

#[macro_export]
macro_rules! exerror {
    ($cond:expr, $fmt:expr $(, $arg:expr)* $(,)?) => {
        $crate::error::exerror($cond, $fmt, &[$($crate::output::VaArg::from($arg)),*])
    };
}

pub use crate::{exerror, sh_error, sh_warnx};

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
