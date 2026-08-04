//! Literal port of `src/init.h` (and of the `init.c` that `mkinit`
//! generates from it).
//! Rules: `docs/spec/port/src/init.md`.
//!
//! These five functions have no hand-written C body: `mkinit` scans every
//! `.c` file for `INIT` / `EXITRESET` / `FORKRESET` / `POSTEXITRESET` /
//! `RESET` blocks and concatenates the bodies **in the order the files are
//! listed in `dash_CFILES`** (src/Makefile.am:18-22):
//!
//! ```text
//! alias arith_yacc arith_yylex cd error eval exec expand histedit input
//! jobs mail main memalloc miscbltin mystring options parser redir show
//! trap output bltin/printf system bltin/test bltin/times var
//! ```
//!
//! Per the spec, the port satisfies the contract by running each fragment in
//! that order rather than by reproducing the generator.  Fragments whose
//! bodies touch state private to a module are delegated to a `mkinit_*`
//! routine in that module; the rest are inlined here, which is what
//! `init.c` itself looks like.
//!
//! Fragment order is load bearing and was verified against the `init.c`
//! that `mkinit` actually emits: `init` runs input, trap, output, var;
//! `forkreset` runs input, main, redir, trap.  That is what
//! `docs/spec/port/src/init.md` states and what this file implements — an
//! earlier note here claimed the two disagreed, which was wrong.

use libc::c_void;
use core::ptr::addr_of_mut;

use crate::nodes::node;

/*
 * Initialization code.
 */

// [spec:dash:def:init.init-fn]
// [spec:dash:sem:init.init-fn]
pub unsafe fn init() {
    /* from input.c: */
    crate::input::mkinit_init();

    /* from trap.c: */
    crate::trap::mkinit_init();

    /* from output.c: */
    {
        /* #ifdef USE_GLIBC_STDIO — not defined, so `initstreams()` is not
         * called in the shipped build. */
    }

    /* from var.c: */
    crate::var::mkinit_init();
}

/*
 * This routine is called when an error or an interrupt occurs in an
 * interactive shell and control is returned to the main command loop
 * but prior to exitshell.
 */

// [spec:dash:def:init.exitreset-fn]
// [spec:dash:sem:init.exitreset-fn]
pub unsafe fn exitreset() {
    /* from eval.c: */
    {
        if crate::eval::savestatus >= 0 {
            if crate::error::exception == crate::error::EXEXIT
                || crate::eval::evalskip == crate::eval::SKIPFUNCDEF
            {
                crate::eval::exitstatus = crate::eval::savestatus;
            }
            crate::eval::savestatus = -1;
        }
        crate::eval::evalskip = 0;
        crate::eval::loopnest = 0;
        crate::eval::inps4 = 0;

        if crate::eval::tpip[0] >= 0 {
            libc::close(crate::eval::tpip[0]);
            libc::close(crate::eval::tpip[1]);
        }
    }

    /* from expand.c: */
    {
        crate::expand::ifsfree();
    }

    /* from redir.c: */
    crate::redir::mkinit_exitreset();
}

/*
 * This routine is called when we enter a subshell.
 */

// [spec:dash:def:init.forkreset-fn]
// [spec:dash:sem:init.forkreset-fn]
pub unsafe fn forkreset(n: *mut node) {
    /* from input.c: */
    crate::input::mkinit_forkreset();

    /* from main.c: */
    {
        crate::error::handler = addr_of_mut!(crate::shellmain::main_handler);
    }

    /* from redir.c: */
    crate::redir::mkinit_forkreset();

    /* from trap.c: */
    crate::trap::mkinit_forkreset(n);
}

/*
 * This routine is called in exitshell.
 */

// [spec:dash:def:init.postexitreset-fn]
// [spec:dash:sem:init.postexitreset-fn]
pub unsafe fn postexitreset() {
    /* from input.c: */
    crate::input::mkinit_postexitreset();
}

/*
 * This routine is called when an error or an interrupt occurs in an
 * interactive shell and control is returned to the main command loop.
 */

// [spec:dash:def:init.reset-fn]
// [spec:dash:sem:init.reset-fn]
pub unsafe fn reset() {
    /* from input.c: */
    crate::input::mkinit_reset();

    /* from output.c: */
    {
        /* #ifdef notyet — the memout teardown is not compiled. */
        let _: *mut c_void = core::ptr::null_mut();
    }

    /* from var.c: */
    crate::var::mkinit_reset();
}
