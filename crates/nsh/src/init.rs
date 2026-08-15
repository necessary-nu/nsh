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

use crate::context::Shell;
use libc::c_void;

use crate::nodes::Node;

/*
 * Initialization code.
 */

// [spec:dash:def:init.init-fn]
// [spec:dash:sem:init.init-fn]
pub unsafe fn init(sh: &mut crate::context::Shell) -> Result<(), crate::error::Error> {
    /* from input.c: */
    crate::input::mkinit_init(sh);

    /* from trap.c: */
    crate::trap::mkinit_init(sh);

    /* from output.c: */
    {
        /* #ifdef USE_GLIBC_STDIO — not defined, so `initstreams()` is not
         * called in the shipped build. */
    }

    /* from var.c: */
    crate::var::mkinit_init(sh)
}

/*
 * This routine is called when an error or an interrupt occurs in an
 * interactive shell and control is returned to the main command loop
 * but prior to exitshell.
 */

// [spec:dash:def:init.exitreset-fn]
// [spec:dash:sem:init.exitreset-fn]
///
/// `by_exitcmd` is the C's `exception == EXEXIT`, passed in rather than
/// read off a global. It is the *only* thing that ever told `EXEXIT` from
/// `EXEND` — see `eval::Flow`, whose doc comment records the audit
/// `docs/api-design.md` 10.2 asked for — so it is the whole of what the
/// two callers still have to say about which one arrived.
pub unsafe fn exitreset(sh: &mut crate::context::Shell, by_exitcmd: bool) {
    /* from eval.c: */
    {
        if sh.eval.savestatus >= 0 {
            if by_exitcmd || sh.eval.evalskip == crate::eval::SKIPFUNCDEF {
                sh.status = sh.eval.savestatus;
            }
            sh.eval.savestatus = -1;
        }
        sh.eval.evalskip = 0;
        sh.eval.loopnest = 0;
        sh.eval.inps4 = 0;

        if sh.eval.tpip[0] >= 0 {
            libc::close(sh.eval.tpip[0]);
            libc::close(sh.eval.tpip[1]);
        }
    }

    /* from expand.c: */
    {
        crate::expand::ifsfree();
    }

    /* from redir.c: */
    crate::redir::mkinit_exitreset(sh);
}

/*
 * This routine is called when we enter a subshell.
 */

// [spec:dash:def:init.forkreset-fn]
// [spec:dash:sem:init.forkreset-fn]
pub unsafe fn forkreset(sh: &mut crate::context::Shell, n: Option<&Node>) {
    /* from input.c: */
    crate::input::mkinit_forkreset(sh);

    /* from main.c: `handler = &main_handler`, which pointed the child at
     * `main`'s catch frame so an exception raised in a subshell landed at
     * `exit:`. There are no handlers; a forked child reaches the same
     * place by returning, or by `shellmain::exit_from_child` where it
     * cannot return. */

    /* from redir.c: */
    crate::redir::mkinit_forkreset(sh);

    /* from trap.c: */
    crate::trap::mkinit_forkreset(sh, n);
}

/*
 * This routine is called in exitshell.
 */

// [spec:dash:def:init.postexitreset-fn]
// [spec:dash:sem:init.postexitreset-fn]
pub unsafe fn postexitreset(sh: &mut Shell) {
    /* from input.c: */
    crate::input::mkinit_postexitreset(sh);
}

/*
 * This routine is called when an error or an interrupt occurs in an
 * interactive shell and control is returned to the main command loop.
 */

// [spec:dash:def:init.reset-fn]
// [spec:dash:sem:init.reset-fn]
pub unsafe fn reset(sh: &mut crate::context::Shell) {
    /* from input.c: */
    crate::input::mkinit_reset(sh);

    /* from output.c: */
    {
        /* #ifdef notyet — the memout teardown is not compiled. */
        let _: *mut c_void = core::ptr::null_mut();
    }

    /* from var.c: */
    crate::var::mkinit_reset(sh);
}
