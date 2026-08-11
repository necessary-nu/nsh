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
            libc::abort();
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
    sh_warnx(msg);

    crate::output::flushall();
    exraise(cond);
    /* NOTREACHED */
}

// [spec:dash:def:error.sh-error-fn]
// [spec:dash:sem:error.sh-error-fn]
pub unsafe fn sh_error(msg: &[u8]) -> ! {
    crate::eval::exitstatus = 2;

    exverror(EXERROR, msg);
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
