//! Literal port of `src/memalloc.c` / `src/memalloc.h`.
//! Rules: `docs/spec/port/src/memalloc.md`.
//!
//! Pointer arithmetic, `static mut` globals and manual size accounting
//! are all deliberate: this is the shell's hand-rolled LIFO arena and
//! its rebasing behaviour is load-bearing for every string the shell
//! builds.  Arithmetic that can overflow uses the `wrapping_*` forms so
//! the C's overflow checks (`if (len < blocksize)`, `if (newlen <
//! stacknleft)`) still see the wrapped value in a debug build.

use core::mem::size_of;
use core::ptr::addr_of_mut;

use libc::{c_char, c_double, c_int, c_void, size_t};

use crate::error::{INTOFF, INTON};
use crate::shell::{cstr, DEBUG};

/*
 * From `src/machdep.h`.  Most machines require the value returned from
 * malloc to be aligned in some way; `SHELL_ALIGN` gets this right on
 * many machines.  machdep.h carries no port rules of its own, so these
 * two live here, next to their only heavy user.
 */
#[repr(C)]
union shell_size_union {
    i: c_int,
    cp: *mut c_char,
    d: c_double,
}

pub const SHELL_SIZE: usize = size_of::<shell_size_union>() - 1;

/*
 * It appears that grabstackstr() will barf with such alignments
 * because stalloc() will return a string allocated in a new stackblock.
 */
#[inline(always)]
pub const fn SHELL_ALIGN(nbytes: usize) -> usize {
    nbytes.wrapping_add(SHELL_SIZE) & !SHELL_SIZE
}

// [spec:dash:def:memalloc.outofspace-fn]
// [spec:dash:sem:memalloc.outofspace-fn]
#[inline(always)]
pub unsafe fn outofspace() {
    crate::error::sh_error(cstr(b"Out of space\0"), &[]);
}

// [spec:dash:def:memalloc.checknull-fn]
// [spec:dash:sem:memalloc.checknull-fn]
unsafe fn checknull(p: *mut c_void) -> *mut c_void {
    if p.is_null() {
        outofspace();
    }
    p
}

/*
 * Like malloc, but returns an error when out of space.
 */

// [spec:dash:def:memalloc.ckmalloc-fn]
// [spec:dash:sem:memalloc.ckmalloc-fn]
#[inline(never)]
pub unsafe fn ckmalloc(nbytes: size_t) -> *mut c_void {
    let p: *mut c_void;

    p = libc::malloc(nbytes);
    checknull(p)
}

/*
 * Same for realloc.
 */

// [spec:dash:def:memalloc.ckrealloc-fn]
// [spec:dash:sem:memalloc.ckrealloc-fn]
#[inline(never)]
pub unsafe fn ckrealloc(p: *mut c_void, nbytes: size_t) -> *mut c_void {
    let p = libc::realloc(p, nbytes);
    checknull(p)
}

/*
 * Make a copy of a string in safe storage.
 */

// [spec:dash:def:memalloc.savestr-fn]
// [spec:dash:sem:memalloc.savestr-fn]
pub unsafe fn savestr(s: *const c_char) -> *mut c_char {
    checknull(libc::strdup(s) as *mut c_void) as *mut c_char
}

/*
 * Parse trees for commands are allocated in lifo order, so we use a stack
 * to make this more efficient, and also to avoid all sorts of exception
 * handling code to handle interrupts in the middle of a parse.
 *
 * The size 504 was chosen because the Ultrix malloc handles that size
 * well.
 */

/* minimum size of a block */
pub const MINSIZE: usize = SHELL_ALIGN(504);

// [spec:dash:def:memalloc.stack-block]
#[repr(C)]
pub struct stack_block {
    pub prev: *mut stack_block,
    pub space: [c_char; MINSIZE],
}

// [spec:dash:def:memalloc.stackmark]
#[repr(C)]
pub struct stackmark {
    pub stackp: *mut stack_block,
    pub stacknxt: *mut c_char,
    pub stacknleft: size_t,
}

impl stackmark {
    /* C writes `struct stackmark smark;` and fills it in with
     * setstackmark(); this is the equivalent uninitialised value. */
    pub const fn new() -> stackmark {
        stackmark {
            stackp: core::ptr::null_mut(),
            stacknxt: core::ptr::null_mut(),
            stacknleft: 0,
        }
    }
}

pub static mut stackbase: stack_block = stack_block {
    prev: core::ptr::null_mut(),
    space: [0; MINSIZE],
};
pub static mut stackp: *mut stack_block = addr_of_mut!(stackbase);
pub static mut stacknxt: *mut c_char = unsafe { addr_of_mut!(stackbase.space) as *mut c_char };
pub static mut stacknleft: size_t = MINSIZE;
pub static mut sstrend: *mut c_char =
    unsafe { (addr_of_mut!(stackbase.space) as *mut c_char).add(MINSIZE) };

// [spec:dash:def:memalloc.stalloc-fn]
// [spec:dash:sem:memalloc.stalloc-fn]
pub unsafe fn stalloc(nbytes: size_t) -> *mut c_void {
    let p: *mut c_char;
    let aligned: size_t;

    aligned = SHELL_ALIGN(nbytes);
    if aligned > stacknleft {
        let len: size_t;
        let mut blocksize: size_t;
        let sp: *mut stack_block;

        blocksize = aligned;
        if blocksize < MINSIZE {
            blocksize = MINSIZE;
        }
        len = (size_of::<stack_block>() - MINSIZE).wrapping_add(blocksize);
        if len < blocksize {
            outofspace();
        }
        INTOFF();
        sp = ckmalloc(len) as *mut stack_block;
        (*sp).prev = stackp;
        stacknxt = addr_of_mut!((*sp).space) as *mut c_char;
        stacknleft = blocksize;
        sstrend = stacknxt.add(blocksize);
        stackp = sp;
        INTON();
    }
    p = stacknxt;
    stacknxt = stacknxt.add(aligned);
    stacknleft -= aligned;
    p as *mut c_void
}

// [spec:dash:def:memalloc.stunalloc-fn]
// [spec:dash:sem:memalloc.stunalloc-fn]
pub unsafe fn stunalloc(p: *mut c_void) {
    if DEBUG {
        if p.is_null()
            || (stacknxt as usize) < (p as usize)
            || (p as usize) < (addr_of_mut!((*stackp).space) as usize)
        {
            let _ = libc::write(2, b"stunalloc\n".as_ptr() as *const c_void, 10);
            libc::abort();
        }
    }
    stacknleft += (stacknxt as usize).wrapping_sub(p as usize);
    stacknxt = p as *mut c_char;
}

// [spec:dash:def:memalloc.pushstackmark-fn]
// [spec:dash:sem:memalloc.pushstackmark-fn]
#[inline(never)]
pub unsafe fn pushstackmark(mark: *mut stackmark, len: size_t) {
    (*mark).stackp = stackp;
    (*mark).stacknxt = stacknxt;
    (*mark).stacknleft = stacknleft;
    grabstackblock(len);
}

// [spec:dash:def:memalloc.setstackmark-fn]
// [spec:dash:sem:memalloc.setstackmark-fn]
pub unsafe fn setstackmark(mark: *mut stackmark) {
    pushstackmark(
        mark,
        (stacknxt == addr_of_mut!((*stackp).space) as *mut c_char
            && stackp != addr_of_mut!(stackbase)) as size_t,
    );
}

// [spec:dash:def:memalloc.popstackmark-fn]
// [spec:dash:sem:memalloc.popstackmark-fn]
pub unsafe fn popstackmark(mark: *mut stackmark) {
    let mut sp: *mut stack_block;

    INTOFF();
    while stackp != (*mark).stackp {
        sp = stackp;
        stackp = (*sp).prev;
        crate::ckfree!(sp);
    }
    stacknxt = (*mark).stacknxt;
    stacknleft = (*mark).stacknleft;
    sstrend = (*mark).stacknxt.add((*mark).stacknleft);
    INTON();
}

/*
 * When the parser reads in a string, it wants to stick the string on the
 * stack and only adjust the stack pointer when it knows how big the
 * string is.  Stackblock (defined in stack.h) returns a pointer to a block
 * of space on top of the stack and stackblocklen returns the length of
 * this block.  Growstackblock will grow this space by at least one byte,
 * possibly moving it (like realloc).  Grabstackblock actually allocates the
 * part of the block that has been used.
 */

// [spec:dash:def:memalloc.growstackblock-fn]
// [spec:dash:sem:memalloc.growstackblock-fn]
unsafe fn growstackblock(min: size_t) -> *mut c_char {
    let mut newlen: size_t;
    let p: *mut c_char;
    let mut min = min;

    newlen = stacknleft.wrapping_mul(2);
    if newlen < stacknleft {
        outofspace();
    }
    min = SHELL_ALIGN(min | 128);
    if newlen < min {
        newlen = newlen.wrapping_add(min);
    }

    if stacknxt == addr_of_mut!((*stackp).space) as *mut c_char && stackp != addr_of_mut!(stackbase)
    {
        let mut sp: *mut stack_block;
        let prevstackp: *mut stack_block;
        let grosslen: size_t;

        INTOFF();
        sp = stackp;
        prevstackp = (*sp).prev;
        grosslen = newlen
            .wrapping_add(size_of::<stack_block>())
            .wrapping_sub(MINSIZE);
        sp = ckrealloc(sp as *mut c_void, grosslen) as *mut stack_block;
        (*sp).prev = prevstackp;
        stackp = sp;
        stacknxt = addr_of_mut!((*sp).space) as *mut c_char;
        p = stacknxt;
        stacknleft = newlen;
        sstrend = (addr_of_mut!((*sp).space) as *mut c_char).add(newlen);
        INTON();
    } else {
        let oldspace: *mut c_char = stacknxt;
        /*
         * BUG (faithfully reproduced, src/memalloc.c:236): `oldlen` is
         * an `int` while `stacknleft` is a `size_t`, so a scratch block
         * larger than INT_MAX is truncated here and the memcpy below
         * copies the wrong length.
         */
        let oldlen: c_int = stacknleft as c_int;

        p = stalloc(newlen) as *mut c_char;

        /* free the space we just allocated */
        libc::memcpy(p as *mut c_void, oldspace as *const c_void, oldlen as size_t);
        stacknxt = p;
        stacknleft += newlen;
    }

    p
}

/*
 * The following routines are somewhat easier to use than the above.
 * The user declares a variable of type STACKSTR, which may be declared
 * to be a register.  The macro STARTSTACKSTR initializes things.  Then
 * the user uses the macro STPUTC to add characters to the string.  In
 * effect, STPUTC(c, p) is the same as *p++ = c except that the stack is
 * grown as necessary.  When the user is done, she can just leave the
 * string there and refer to it using stackblock().  Or she can allocate
 * the space for it using grabstackstr().  If it is necessary to allow
 * someone else to use the stack temporarily and then continue to grow
 * the string, the user should use grabstack to allocate the space, and
 * then call ungrabstr(p) to return to the previous mode of operation.
 *
 * USTPUTC is like STPUTC except that it doesn't check for overflow.
 * CHECKSTACKSPACE can be called before USTPUTC to ensure that there
 * is space for at least one character.
 */

// [spec:dash:def:memalloc.growstackstr-fn]
// [spec:dash:sem:memalloc.growstackstr-fn]
pub unsafe fn growstackstr() -> *mut c_void {
    let len: size_t = stackblocksize();

    growstackblock(0).add(len) as *mut c_void
}

// [spec:dash:def:memalloc.growstackto-fn]
// [spec:dash:sem:memalloc.growstackto-fn]
#[inline(never)]
pub unsafe fn growstackto(len: size_t) -> *mut c_char {
    if stackblocksize() < len {
        return growstackblock(len);
    }
    stackblock() as *mut c_char
}

/*
 * Called from CHECKSTRSPACE.
 */

// [spec:dash:def:memalloc.makestrspace-fn]
// [spec:dash:sem:memalloc.makestrspace-fn]
#[inline(never)]
pub unsafe fn makestrspace(newlen: size_t, p: *mut c_char) -> *mut c_char {
    let len: size_t = (p as usize).wrapping_sub(stacknxt as usize);

    growstackto(len + newlen).add(len)
}

// [spec:dash:def:memalloc.stnputs-fn]
// [spec:dash:sem:memalloc.stnputs-fn]
#[inline(never)]
pub unsafe fn stnputs(s: *const c_char, n: size_t, p: *mut c_char) -> *mut c_char {
    let mut p = p;
    p = makestrspace(n, p);
    p = crate::system::mempcpy(p as *mut c_void, s as *const c_void, n) as *mut c_char;
    p
}

// [spec:dash:def:memalloc.stputs-fn]
// [spec:dash:sem:memalloc.stputs-fn]
pub unsafe fn stputs(s: *const c_char, p: *mut c_char) -> *mut c_char {
    stnputs(s, libc::strlen(s), p)
}

/* ------------------------------------------------------------------ */
/* src/memalloc.h                                                     */
/* ------------------------------------------------------------------ */

// [spec:dash:def:memalloc.grabstackblock-fn]
// [spec:dash:sem:memalloc.grabstackblock-fn]
#[inline]
pub unsafe fn grabstackblock(len: size_t) {
    stalloc(len);
}

// [spec:dash:def:memalloc.stputc-fn]
// [spec:dash:sem:memalloc.stputc-fn]
#[inline]
pub unsafe fn _STPUTC(c: c_int, p: *mut c_char) -> *mut c_char {
    let mut p = p;
    if p == sstrend {
        p = growstackstr() as *mut c_char;
    }
    *p = c as c_char;
    p = p.add(1);
    p
}

/*
 * `#define stackblock() ((void *)stacknxt)`
 */
#[inline(always)]
pub unsafe fn stackblock() -> *mut c_void {
    stacknxt as *mut c_void
}

/*
 * `#define stackblocksize() stacknleft`
 */
#[inline(always)]
pub unsafe fn stackblocksize() -> size_t {
    stacknleft
}

/*
 * `#define stackstrend() ((void *)sstrend)`
 */
#[inline(always)]
pub unsafe fn stackstrend() -> *mut c_void {
    sstrend as *mut c_void
}

/*
 * `#define grabstackstr(p) stalloc((char *)(p) - (char *)stackblock())`
 * — generic over the pointee only so that the C macro's `(char *)` cast
 * has an equivalent.
 */
#[inline(always)]
pub unsafe fn grabstackstr<T>(p: *mut T) -> *mut c_void {
    stalloc((p as usize).wrapping_sub(stackblock() as usize))
}

/*
 * `#define ungrabstackstr(s, p) stunalloc((s))`
 */
#[inline(always)]
pub unsafe fn ungrabstackstr<T, U>(s: *mut T, _p: *mut U) {
    stunalloc(s as *mut c_void);
}

/*
 * `#define ckfree(p) free((void *)(p))` — function form.  Generic over
 * the pointee only so that the C macro's implicit `(void *)` cast has an
 * equivalent; there is no other abstraction here.
 */
#[inline(always)]
pub unsafe fn ckfree<T>(p: *mut T) {
    libc::free(p as *mut c_void);
}

/*
 * `#define ckfree(p) free((void *)(p))` — macro form so call sites keep
 * the C spelling and the implicit `(void *)` cast.
 */
#[macro_export]
macro_rules! ckfree {
    ($p:expr) => {
        ::libc::free($p as *mut ::libc::c_void)
    };
}

/*
 * `#define STARTSTACKSTR(p) ((p) = stackblock())`
 */
#[macro_export]
macro_rules! STARTSTACKSTR {
    ($p:expr) => {
        $p = $crate::memalloc::stackblock() as *mut _
    };
}

/*
 * `#define STPUTC(c, p) ((p) = _STPUTC((c), (p)))`
 */
#[macro_export]
macro_rules! STPUTC {
    ($c:expr, $p:expr) => {
        $p = $crate::memalloc::_STPUTC($c as ::libc::c_int, $p)
    };
}

/*
 * ```c
 * #define CHECKSTRSPACE(n, p) \
 *	({ char *_q = (p); size_t _l = (n); size_t _m = sstrend - _q; \
 *	   if (_l > _m) (p) = makestrspace(_l, _q); 0; })
 * ```
 */
#[macro_export]
macro_rules! CHECKSTRSPACE {
    ($n:expr, $p:expr) => {{
        let _q: *mut ::libc::c_char = $p;
        let _l: usize = $n as usize;
        let _m: usize = ($crate::memalloc::sstrend as usize).wrapping_sub(_q as usize);
        if _l > _m {
            $p = $crate::memalloc::makestrspace(_l, _q);
        }
        0
    }};
}

/*
 * `#define USTPUTC(c, p) (*p++ = (c))`
 */
#[macro_export]
macro_rules! USTPUTC {
    ($c:expr, $p:expr) => {{
        let _c = $c as ::libc::c_char;
        *$p = _c;
        $p = $p.add(1);
        _c
    }};
}

/*
 * ```c
 * #define STACKSTRNUL(p) \
 *	((p) == sstrend? (p = growstackstr(), *p = '\0') : (*p = '\0'))
 * ```
 */
#[macro_export]
macro_rules! STACKSTRNUL {
    ($p:expr) => {{
        if $p == $crate::memalloc::sstrend {
            $p = $crate::memalloc::growstackstr() as *mut ::libc::c_char;
            *$p = 0;
        } else {
            *$p = 0;
        }
    }};
}

/*
 * `#define STUNPUTC(p) (--p)`
 */
#[macro_export]
macro_rules! STUNPUTC {
    ($p:expr) => {
        $p = $p.sub(1)
    };
}

/*
 * `#define STTOPC(p) p[-1]`
 */
#[macro_export]
macro_rules! STTOPC {
    ($p:expr) => {
        *$p.offset(-1)
    };
}

/*
 * `#define STADJUST(amount, p) (p += (amount))`
 */
#[macro_export]
macro_rules! STADJUST {
    ($amount:expr, $p:expr) => {
        $p = $p.offset($amount as isize)
    };
}
