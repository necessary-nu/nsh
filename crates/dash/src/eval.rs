//! Literal port of `src/eval.c` / `src/eval.h`.
//! Rules: `docs/spec/port/src/eval.md`.
//!
//! ## `setjmp`/`longjmp`
//!
//! Rust has no `setjmp`. The port replaces it with `catch_unwind` over a
//! *typed* panic payload (`crate::error::Longjmp`) carrying the address
//! of the target `jmploc` and the value `setjmp` should appear to
//! return. `setjmp_catch(loc, body)` is the literal stand-in for
//!
//! ```c
//! if ((i = setjmp(loc))) goto label;
//! body
//! label:
//! ```
//!
//! — the *body* is exactly the C text between the `setjmp` and the
//! label, and the code after the call is exactly the label's body, so
//! every save/restore of `handler`, `commandname`, `shellparam`,
//! `loopnest`, `funcline` and the interrupt counter stays at the same
//! point in the same order as the C.
//!
//! Divergences are listed in the port report; the important ones are
//! that unwinding *runs `Drop`* (C's `longjmp` does not), and that a
//! `longjmp` cannot cross a non-Rust frame.
//!
//! Other translation notes: `TRACE(...)` compiles to nothing without
//! `DEBUG`, so the calls are dropped; C `goto`s are reproduced with
//! labelled blocks whose nesting mirrors the order of the C labels.

use core::ptr::{addr_of_mut, null, null_mut};
use libc::{c_char, c_int, c_void, size_t};

use crate::builtins::{builtincmd, BUILTIN_ASSIGN, BUILTIN_REGULAR, BUILTIN_SPECIAL};
use crate::error::{jmploc, FORCEINTON, INTOFF, INTON};
use crate::exec::{cmdentry, find_command, param, shellexec};
use crate::exec::{CMDBUILTIN, CMDFUNCTION, CMDNORMAL, CMDUNKNOWN, DO_ERR, DO_NOFUNC, DO_REGBLTIN};
use crate::expand::{arglist, strlist};
use crate::expand::{EXP_FULL, EXP_MBCHAR, EXP_REDIR, EXP_TILDE, EXP_VARTILDE};
use crate::jobs::{job, FORK_NOJOB};
use crate::memalloc::{popstackmark, setstackmark, stackblock, stackmark, stalloc, stunalloc};
use crate::nodes::{funcnode, Node};
use crate::nodes::{
    NAND, NAPPEND, NBACKGND, NCASE, NCLOBBER, NCMD, NDEFUN, NFOR, NFROM, NFROMFD, NFROMTO, NIF,
    NNOT, NOR, NPIPE, NREDIR, NSEMI, NSUBSHELL, NTO, NTOFD, NUNTIL, NWHILE,
};
use crate::output::{out1, output};
use crate::redir::{REDIR_PUSH, REDIR_SAVEFD2};
use crate::var::VEXPORT;

// ---------------------------------------------------------------------
// src/eval.h
// ---------------------------------------------------------------------

/* flags in argument to evaltree */
pub const EV_EXIT: c_int = 0o1; /* exit after evaluating tree */
pub const EV_TESTED: c_int = 0o2; /* exit status is checked; ignore -e flag */

/* reasons for skipping commands (see comment on breakcmd routine) */
pub const SKIPBREAK: c_int = 1 << 0;
pub const SKIPCONT: c_int = 1 << 1;
pub const SKIPFUNC: c_int = 1 << 2;
pub const SKIPFUNCDEF: c_int = 1 << 3;

// [spec:dash:def:eval.backcmd]
#[repr(C)]
pub struct backcmd {
    /* result of evalbackcmd */
    pub fd: c_int,        /* file descriptor to read from */
    pub buf: *mut c_char, /* buffer */
    pub nleft: c_int,     /* number of chars in buffer */
    pub jp: *mut job,     /* job structure for command */
}

// ---------------------------------------------------------------------
// module globals
// ---------------------------------------------------------------------

pub static mut evalskip: c_int = 0; /* set if we are skipping commands */
static mut skipcount: c_int = 0; /* number of levels to skip */
pub static mut loopnest: c_int = 0; /* current loop nesting level (MKINIT) */
static mut funcline: c_int = 0; /* starting line number of current function, or 0 */

pub static mut commandname: *mut c_char = null_mut();
pub static mut exitstatus: c_int = 0; /* exit status of last command */
pub static mut back_exitstatus: c_int = 0; /* exit status of backquoted command */
pub static mut savestatus: c_int = -1; /* exit status of last command outside traps */

/* Prevent PS4 nesting. */
pub static mut inps4: c_int = 0; /* MKINIT */

pub static mut tpip: [c_int; 2] = [-1, 0]; /* MKINIT int tpip[2] = { -1 } */

/* C: `.name = nullstr`. Rust cannot take the address of another
 * module's static in a const initialiser, so this carries its own
 * copy of the empty string; the field is never read for `bltin`. */
static BLTIN_NULLSTR: [c_char; 1] = [0];

static mut bltin: builtincmd = builtincmd {
    name: c"",
    builtin: Some(bltincmd),
    flags: BUILTIN_REGULAR,
};

/* src/options.h: `#define nflag optlist[5]` and friends. */
#[inline]
unsafe fn nflag() -> c_int {
    crate::options::optlist[crate::options::nflag] as c_int
}
#[inline]
unsafe fn eflag() -> c_int {
    crate::options::optlist[crate::options::eflag] as c_int
}
#[inline]
unsafe fn xflag() -> c_int {
    crate::options::optlist[crate::options::xflag] as c_int
}
#[inline]
unsafe fn iflag() -> c_int {
    crate::options::optlist[crate::options::iflag] as c_int
}

// ---------------------------------------------------------------------
// setjmp/longjmp stand-in (see the module comment)
// ---------------------------------------------------------------------

/// Literal stand-in for `if ((v = setjmp(loc))) goto label; body label:`.
///
/// Runs `body` with `loc` armed as a `longjmp` target. Returns 0 if the
/// body ran to completion, or the value passed to `longjmp` if one
/// unwound to exactly this `loc`. A `longjmp` aimed at some *other*
/// `jmploc`, and any panic that is not a `longjmp` at all, is re-raised
/// unchanged so it keeps propagating outwards.
pub(crate) unsafe fn setjmp_catch<F: FnOnce()>(loc: *mut jmploc, body: F) -> c_int {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(()) => 0,
        Err(payload) => match payload.downcast::<crate::error::Longjmp>() {
            Ok(lj) => {
                if lj.loc == loc {
                    lj.val
                } else {
                    std::panic::resume_unwind(lj as Box<dyn std::any::Any + Send>)
                }
            }
            Err(other) => std::panic::resume_unwind(other),
        },
    }
}

// ---------------------------------------------------------------------

/*
 * Called to reset things after an exception.
 *
 * The `EXITRESET` block at the top of src/eval.c is `#ifdef mkinit`
 * material: it is collected by the generator into `init.c`, which the
 * port keeps in `crate::init`. No manifest symbol lives here.
 */

/*
 * The eval commmand.
 */

// [spec:dash:def:eval.evalcmd-fn]
// [spec:dash:sem:eval.evalcmd-fn]
unsafe fn evalcmd(argc: c_int, argv: *mut *mut c_char, flags: c_int) -> c_int {
    let mut p: *mut c_char;
    let mut concat: *mut c_char;
    let mut ap: *mut *mut c_char;

    if argc > 1 {
        p = *argv.offset(1);
        if argc > 2 {
            /* STARTSTACKSTR(concat) */
            concat = stackblock() as *mut c_char;
            ap = argv.offset(2);
            loop {
                concat = crate::memalloc::stputs(p, concat);
                p = *ap;
                ap = ap.add(1);
                if p.is_null() {
                    break;
                }
                /* STPUTC(' ', concat) */
                concat = crate::memalloc::_STPUTC(' ' as c_int, concat);
            }
            /* STPUTC('\0', concat) */
            concat = crate::memalloc::_STPUTC('\0' as c_int, concat);
            /* grabstackstr(concat) */
            p = stalloc(concat as usize - stackblock() as usize) as *mut c_char;
        }
        return evalstring(p, flags & EV_TESTED);
    }
    0
}

/*
 * Execute a command or commands contained in a string.
 */

// [spec:dash:def:eval.evalstring-fn]
// [spec:dash:sem:eval.evalstring-fn]
pub unsafe fn evalstring(s: *mut c_char, flags: c_int) -> c_int {
    let mut n: *mut Node;
    let mut smark: stackmark = core::mem::zeroed();
    let mut status: c_int;
    let s: *mut c_char = crate::mystring::sstrdup(s);

    crate::input::setinputstring(s);
    setstackmark(&mut smark);

    status = 0;
    loop {
        n = crate::parser::parsecmd(0);
        if n == crate::parser::NEOF() {
            break;
        }
        {
            let i: c_int;

            i = evaltree(
                n,
                flags
                    & !(if crate::parser::parser_eof() != 0 {
                        0
                    } else {
                        EV_EXIT
                    }),
            );
            if !n.is_null() {
                status = i;
            }

            if evalskip != 0 {
                break;
            }
        }
        popstackmark(&mut smark);
    }
    popstackmark(&mut smark);
    crate::input::popfile();
    stunalloc(s as *mut c_void);

    status
}

/*
 * Evaluate a parse tree.  The value is left in the global variable
 * exitstatus.
 */

// [spec:dash:def:eval.evaltree-fn]
// [spec:dash:sem:eval.evaltree-fn]
pub unsafe fn evaltree(n: *mut Node, flags: c_int) -> c_int {
    let mut n: *mut Node = n;
    let mut checkexit: c_int = 0;
    /* C leaves `evalfn` uninitialised; every path that reaches
     * `calleval` assigns it first. Seeded here only so that Rust's
     * definite-initialisation analysis is trivially satisfied. */
    let mut evalfn: unsafe fn(*mut Node, c_int) -> c_int = evaltree;
    let mut smark: stackmark = core::mem::zeroed();
    let isor: libc::c_uint;
    let mut status: c_int = 0;

    setstackmark(&mut smark);

    'out_lbl: {
        if nflag() != 0 {
            break 'out_lbl;
        }

        if n.is_null() {
            /* TRACE(("evaltree(NULL) called\n")); */
            break 'out_lbl;
        }

        crate::trap::dotrap();

        /* #ifndef SMALL: show history substitutions done with fc */
        crate::histedit::displayhist = 1;

        /* TRACE(("pid %d, evaltree(%p: %d, %d) called\n", ...)); */
        'sw: {
            'calleval: {
                'evaln: {
                    'checkexit_lbl: {
                        match (*n).r#type {
                            NREDIR => {
                                crate::error::errlinno = (*n).nredir.linno;
                                crate::var::lineno = (*n).nredir.linno;
                                if funcline != 0 {
                                    crate::var::lineno -= funcline - 1;
                                }
                                expredir((*n).nredir.redirect);
                                crate::redir::pushredir((*n).nredir.redirect);
                                status = crate::redir::redirectsafe(
                                    (*n).nredir.redirect,
                                    REDIR_PUSH,
                                );
                                if status != 0 {
                                    checkexit = EV_TESTED;
                                } else {
                                    status = evaltree((*n).nredir.n, flags & EV_TESTED);
                                }
                                if !(*n).nredir.redirect.is_null() {
                                    crate::redir::popredir(0);
                                }
                                break 'sw;
                            }
                            NCMD => {
                                evalfn = evalcommand;
                                /* falls through into `checkexit:` */
                            }
                            NFOR => {
                                evalfn = evalfor;
                                break 'calleval;
                            }
                            NWHILE | NUNTIL => {
                                evalfn = evalloop;
                                break 'calleval;
                            }
                            NSUBSHELL | NBACKGND => {
                                evalfn = evalsubshell;
                                break 'checkexit_lbl;
                            }
                            NPIPE => {
                                evalfn = evalpipe;
                                break 'checkexit_lbl;
                            }
                            NCASE => {
                                evalfn = evalcase;
                                break 'calleval;
                            }
                            NAND | NOR | NSEMI => {
                                /* #if NAND + 1 != NOR / NOR + 1 != NSEMI */
                                isor = ((*n).r#type - NAND) as libc::c_uint;
                                status = evaltree(
                                    (*n).nbinary.ch1,
                                    (flags | (((isor >> 1).wrapping_sub(1)) as c_int)) & EV_TESTED,
                                );
                                if ((status == 0) as libc::c_uint) == isor || evalskip != 0 {
                                    break 'sw;
                                }
                                n = (*n).nbinary.ch2;
                                break 'evaln;
                            }
                            NIF => {
                                status = evaltree((*n).nif.test, EV_TESTED);
                                if evalskip != 0 {
                                    break 'sw;
                                }
                                if status == 0 {
                                    n = (*n).nif.ifpart;
                                    break 'evaln;
                                } else if !(*n).nif.elsepart.is_null() {
                                    n = (*n).nif.elsepart;
                                    break 'evaln;
                                }
                                status = 0;
                                break 'sw;
                            }
                            NDEFUN => {
                                crate::exec::defun(n);
                                break 'sw;
                            }
                            /* `default:` has no body outside DEBUG, so an
                             * unrecognised node type falls straight through
                             * into `case NNOT:`. Reproduced bug-for-bug. */
                            _ /* default, NNOT */ => {
                                status = evaltree((*n).nnot.com, EV_TESTED);
                                if evalskip == 0 {
                                    status = (status == 0) as c_int;
                                }
                                break 'sw;
                            }
                        }
                    }
                    // checkexit:
                    checkexit = EV_TESTED;
                    break 'calleval;
                }
                // evaln:
                evalfn = evaltree;
            }
            // calleval:
            status = evalfn(n, flags);
        }

        exitstatus = status;
    }
    // out:
    crate::trap::dotrap();

    'exexit: {
        if eflag() != 0 && (!flags & checkexit) != 0 && status != 0 {
            break 'exexit;
        }

        if (flags & EV_EXIT) != 0 {
            break 'exexit;
        }

        popstackmark(&mut smark);

        return exitstatus;
    }
    // exexit:
    crate::error::exraise(crate::error::EXEND);
}

// [spec:dash:def:eval.evaltreenr-fn]
// [spec:dash:sem:eval.evaltreenr-fn]
//
// `evaltree` declared `noreturn`. Where the C compiler supports
// `__attribute__((alias))` it is literally the same function; the
// portable fallback — reproduced here — calls `evaltree` and aborts if
// it ever comes back.
pub unsafe fn evaltreenr(n: *mut Node, flags: c_int) -> ! {
    evaltree(n, flags);
    libc::abort();
}

// [spec:dash:def:eval.skiploop-fn]
// [spec:dash:sem:eval.skiploop-fn]
unsafe fn skiploop() -> c_int {
    let mut skip: c_int = evalskip;

    match skip {
        0 => {}

        SKIPBREAK | SKIPCONT => {
            skipcount -= 1;
            if skipcount <= 0 {
                evalskip = 0;
            } else {
                skip = SKIPBREAK;
            }
        }

        _ => {}
    }

    skip
}

// [spec:dash:def:eval.evalloop-fn]
// [spec:dash:sem:eval.evalloop-fn]
unsafe fn evalloop(n: *mut Node, flags: c_int) -> c_int {
    let mut skip: c_int;
    let mut status: c_int;
    let mut flags: c_int = flags;

    loopnest += 1;
    status = 0;
    flags &= EV_TESTED;
    loop {
        {
            let mut i: c_int;

            i = evaltree((*n).nbinary.ch1, EV_TESTED);
            skip = skiploop();
            if skip == SKIPFUNC {
                status = i;
            }
            if skip != 0 {
                /* `continue` in the C do/while: re-test the condition */
            } else {
                if (*n).r#type != NWHILE {
                    i = (i == 0) as c_int;
                }
                if i != 0 {
                    break;
                }
                status = evaltree((*n).nbinary.ch2, flags);
                skip = skiploop();
            }
        }
        if (skip & !SKIPCONT) != 0 {
            break;
        }
    }
    loopnest -= 1;

    status
}

// [spec:dash:def:eval.evalfor-fn]
// [spec:dash:sem:eval.evalfor-fn]
unsafe fn evalfor(n: *mut Node, flags: c_int) -> c_int {
    let mut arglist: arglist = core::mem::zeroed();
    let mut argp: *mut Node;
    let mut sp: *mut strlist;
    let mut status: c_int;
    let mut flags: c_int = flags;

    crate::error::errlinno = (*n).nfor.linno;
    crate::var::lineno = (*n).nfor.linno;
    if funcline != 0 {
        crate::var::lineno -= funcline - 1;
    }

    arglist.lastp = &mut arglist.list;
    argp = (*n).nfor.args;
    while !argp.is_null() {
        crate::expand::expandarg(argp, &mut arglist, EXP_FULL | EXP_TILDE);
        argp = (*argp).narg.next;
    }
    *arglist.lastp = null_mut();

    status = 0;
    loopnest += 1;
    flags &= EV_TESTED;
    sp = arglist.list;
    while !sp.is_null() {
        crate::var::setvar((*n).nfor.var, (*sp).text, 0);
        status = evaltree((*n).nfor.body, flags);
        if (skiploop() & !SKIPCONT) != 0 {
            break;
        }
        sp = (*sp).next;
    }
    loopnest -= 1;

    status
}

// [spec:dash:def:eval.evalcase-fn]
// [spec:dash:sem:eval.evalcase-fn]
unsafe fn evalcase(n: *mut Node, flags: c_int) -> c_int {
    let mut cp: *mut Node;
    let mut patp: *mut Node;
    let mut arglist: arglist = core::mem::zeroed();
    let mut status: c_int = 0;

    crate::error::errlinno = (*n).ncase.linno;
    crate::var::lineno = (*n).ncase.linno;
    if funcline != 0 {
        crate::var::lineno -= funcline - 1;
    }

    arglist.lastp = &mut arglist.list;
    crate::expand::expandarg(
        (*n).ncase.expr,
        &mut arglist,
        if crate::mystring::FNMATCH_IS_ENABLED != 0 {
            EXP_TILDE
        } else {
            EXP_TILDE | EXP_MBCHAR
        },
    );
    'out_lbl: {
        cp = (*n).ncase.cases;
        while !cp.is_null() && evalskip == 0 {
            patp = (*cp).nclist.pattern;
            while !patp.is_null() {
                if crate::expand::casematch(patp, (*arglist.list).text) != 0 {
                    /* Ensure body is non-empty as otherwise
                     * EV_EXIT may prevent us from setting the
                     * exit status.
                     */
                    if evalskip == 0 && !(*cp).nclist.body.is_null() {
                        status = evaltree((*cp).nclist.body, flags);
                    }
                    break 'out_lbl;
                }
                patp = (*patp).narg.next;
            }
            cp = (*cp).nclist.next;
        }
    }
    // out:
    status
}

/*
 * Kick off a subshell to evaluate a tree.
 */

// [spec:dash:def:eval.evalsubshell-fn]
// [spec:dash:sem:eval.evalsubshell-fn]
unsafe fn evalsubshell(n: *mut Node, flags: c_int) -> c_int {
    let jp: *mut job;
    let backgnd: c_int = ((*n).r#type == NBACKGND) as c_int;
    let mut status: c_int;
    let mut flags: c_int = flags;

    crate::error::errlinno = (*n).nredir.linno;
    crate::var::lineno = (*n).nredir.linno;
    if funcline != 0 {
        crate::var::lineno -= funcline - 1;
    }

    expredir((*n).nredir.redirect);
    INTOFF();
    'nofork: {
        if backgnd == 0 && (flags & EV_EXIT) != 0 && crate::trap::have_traps() == 0 {
            crate::init::forkreset(null_mut());
            break 'nofork;
        }
        jp = crate::jobs::makejob(1);
        if crate::jobs::forkshell(jp, (*n).nredir.n, backgnd) == 0 {
            flags |= EV_EXIT;
            if backgnd != 0 {
                flags &= !EV_TESTED;
            }
            break 'nofork;
        }
        /* the parent tail of the C function; the child path below
         * never returns, so it is reached only from here */
        status = 0;
        if backgnd == 0 {
            status = crate::jobs::waitforjob(jp);
        }
        INTON();
        return status;
    }
    // nofork:
    INTON();
    crate::redir::redirect((*n).nredir.redirect, 0);
    evaltreenr((*n).nredir.n, flags)
    /* never returns */
}

/*
 * Compute the names of the files in a redirection list.
 */

// [spec:dash:def:eval.expredir-fn]
// [spec:dash:sem:eval.expredir-fn]
unsafe fn expredir(n: *mut Node) {
    let mut redir: *mut Node;

    redir = n;
    while !redir.is_null() {
        let mut fnl: arglist = core::mem::zeroed();
        fnl.lastp = &mut fnl.list;
        match (*redir).r#type {
            NFROMTO | NFROM | NTO | NCLOBBER | NAPPEND => {
                crate::expand::expandarg((*redir).nfile.fname, &mut fnl, EXP_TILDE | EXP_REDIR);
                (*redir).nfile.expfname = (*fnl.list).text;
            }
            NFROMFD | NTOFD => {
                if !(*redir).ndup.vname.is_null() {
                    crate::expand::expandarg((*redir).ndup.vname, &mut fnl, EXP_TILDE | EXP_REDIR);
                    crate::parser::fixredir(redir, (*fnl.list).text, 1);
                }
            }
            _ => {}
        }
        redir = (*redir).nfile.next;
    }
}

/*
 * Evaluate a pipeline.  All the processes in the pipeline are children
 * of the process creating the pipeline.  (This differs from some versions
 * of the shell, which make the last process in a pipeline the parent
 * of all the rest.)
 */

// [spec:dash:def:eval.evalpipe-fn]
// [spec:dash:sem:eval.evalpipe-fn]
unsafe fn evalpipe(n: *mut Node, flags: c_int) -> c_int {
    let jp: *mut job;
    let mut lp: *mut crate::nodes::nodelist;
    let mut pipelen: c_int;
    let mut prevfd: c_int;
    let mut pip: [c_int; 2] = [0; 2];
    let mut status: c_int = 0;
    let mut flags: c_int = flags;

    /* TRACE(("evalpipe(0x%lx) called\n", (long)n)); */
    pipelen = 0;
    lp = (*n).npipe.cmdlist;
    while !lp.is_null() {
        pipelen += 1;
        lp = (*lp).next;
    }
    flags |= EV_EXIT;
    INTOFF();
    jp = crate::jobs::makejob(pipelen);
    prevfd = -1;
    lp = (*n).npipe.cmdlist;
    while !lp.is_null() {
        prehash((*lp).n);
        pip[1] = -1;
        if !(*lp).next.is_null() {
            if libc::pipe(pip.as_mut_ptr()) < 0 {
                libc::close(prevfd);
                crate::sh_error!(b"Pipe call failed\0".as_ptr() as *const c_char);
            }
        }
        if crate::jobs::forkshell(jp, (*lp).n, (*n).npipe.backgnd) == 0 {
            INTON();
            if pip[1] >= 0 {
                libc::close(pip[0]);
            }
            if prevfd > 0 {
                crate::input::reset_input();
                libc::dup2(prevfd, 0);
                libc::close(prevfd);
            }
            if pip[1] > 1 {
                libc::dup2(pip[1], 1);
                libc::close(pip[1]);
            }
            evaltreenr((*lp).n, flags);
            /* never returns */
        }
        if prevfd >= 0 {
            libc::close(prevfd);
        }
        prevfd = pip[0];
        libc::close(pip[1]);
        lp = (*lp).next;
    }
    if (*n).npipe.backgnd == 0 {
        status = crate::jobs::waitforjob(jp);
        /* TRACE(("evalpipe:  job done exit status %d\n", status)); */
    }
    INTON();

    status
}

/*
 * Execute a command inside back quotes.  If it's a builtin command, we
 * want to save its output in a block obtained from malloc.  Otherwise
 * we fork off a subprocess and get the output of the command via a pipe.
 * Should be called with interrupts off.
 */

// [spec:dash:def:eval.evalbackcmd-fn]
// [spec:dash:sem:eval.evalbackcmd-fn]
pub unsafe fn evalbackcmd(n: *mut Node, result: *mut backcmd) {
    let jp: *mut job;
    let mut pip: [c_int; 2] = [0; 2];
    let pid: c_int;

    (*result).fd = -1;
    (*result).buf = null_mut();
    (*result).nleft = 0;
    (*result).jp = null_mut();
    'out_lbl: {
        if n.is_null() {
            break 'out_lbl;
        }

        crate::redir::sh_pipe(pip.as_mut_ptr(), 0);
        tpip[0] = pip[0];
        tpip[1] = pip[1];
        jp = crate::jobs::makejob(1);
        pid = crate::jobs::forkshell(jp, n, FORK_NOJOB);
        tpip[0] = -1;
        if pid == 0 {
            FORCEINTON();
            libc::close(pip[0]);
            if pip[1] != 1 {
                libc::dup2(pip[1], 1);
                libc::close(pip[1]);
            }
            crate::expand::ifsfree();
            evaltreenr(n, EV_EXIT);
            /* NOTREACHED */
        }
        libc::close(pip[1]);
        (*result).fd = pip[0];
        (*result).jp = jp;
    }
    // out:
    /* TRACE(("evalbackcmd done: fd=%d buf=0x%x nleft=%d jp=0x%x\n", ...)); */
}

// [spec:dash:def:eval.fill-arglist-fn]
// [spec:dash:sem:eval.fill-arglist-fn]
unsafe fn fill_arglist(arglist: *mut arglist, argpp: *mut *mut Node) -> *mut strlist {
    let lastp: *mut *mut strlist = (*arglist).lastp;
    let mut argp: *mut Node;

    loop {
        argp = *argpp;
        if argp.is_null() {
            break;
        }
        crate::expand::expandarg(argp, arglist, EXP_FULL | EXP_TILDE);
        *argpp = (*argp).narg.next;
        if !(*lastp).is_null() {
            break;
        }
    }

    *lastp
}

// [spec:dash:def:eval.parse-command-args-fn]
// [spec:dash:sem:eval.parse-command-args-fn]
unsafe fn parse_command_args(
    arglist: *mut arglist,
    argpp: *mut *mut Node,
    path: *mut *const c_char,
) -> c_int {
    let mut sp: *mut strlist = (*arglist).list;
    let mut cp: *mut c_char;
    let mut c: c_char;

    loop {
        sp = if !(*sp).next.is_null() {
            (*sp).next
        } else {
            fill_arglist(arglist, argpp)
        };
        if sp.is_null() {
            return 0;
        }
        cp = (*sp).text;
        let c0 = *cp;
        cp = cp.add(1);
        if c0 != b'-' as c_char {
            break;
        }
        c = *cp;
        cp = cp.add(1);
        if c == 0 {
            break;
        }
        if c == b'-' as c_char && *cp == 0 {
            if (*sp).next.is_null() && fill_arglist(arglist, argpp).is_null() {
                return 0;
            }
            sp = (*sp).next;
            break;
        }
        loop {
            match c as u8 {
                b'p' => {
                    *path = crate::var::defpath();
                }
                _ => {
                    /* run 'typecmd' for other options */
                    return 0;
                }
            }
            c = *cp;
            cp = cp.add(1);
            if c == 0 {
                break;
            }
        }
    }

    (*arglist).list = sp;
    DO_NOFUNC
}

/*
 * Execute a simple command.
 */

// [spec:dash:def:eval.evalcommand-fn]
// [spec:dash:sem:eval.evalcommand-fn]
//
// The `def` rule quotes the `#ifdef notyet` three-argument prototype;
// the compiled signature — ported here — is
// `STATIC int evalcommand(union node *cmd, int flags)`.
unsafe fn evalcommand(cmd: *mut Node, flags: c_int) -> c_int {
    let localvar_stop: *mut crate::var::localvar_list;
    let file_stop: *mut crate::input::parsefile;
    let redir_stop: *mut crate::redir::redirtab;
    let mut argp: *mut Node;
    let mut arglist: arglist = core::mem::zeroed();
    let mut varlist: arglist = core::mem::zeroed();
    let argv: *mut *mut c_char;
    let mut argc: c_int;
    let osp: *mut strlist;
    let mut sp: *mut strlist;
    let mut cmdentry: cmdentry = cmdentry {
        cmdtype: 0,
        u: param { index: 0 },
    };
    let mut jp: *mut job;
    let mut lastarg: *mut c_char;
    let mut path: *const c_char;
    let mut spclbltin: c_int;
    let mut cmd_flag: c_int;
    let mut execcmd: c_int;
    let mut status: c_int;
    let mut nargv: *mut *mut c_char;
    let mut vflags: c_int;
    let mut vlocal: c_int;

    crate::error::errlinno = (*cmd).ncmd.linno;
    crate::var::lineno = (*cmd).ncmd.linno;
    if funcline != 0 {
        crate::var::lineno -= funcline - 1;
    }

    /* First expand the arguments. */
    /* TRACE(("evalcommand(0x%lx, %d) called\n", (long)cmd, flags)); */
    file_stop = crate::input::parsefile;
    back_exitstatus = 0;

    cmdentry.cmdtype = CMDBUILTIN;
    cmdentry.u.cmd = addr_of_mut!(bltin);
    varlist.lastp = &mut varlist.list;
    *varlist.lastp = null_mut();
    arglist.lastp = &mut arglist.list;
    *arglist.lastp = null_mut();

    cmd_flag = 0;
    execcmd = 0;
    spclbltin = -1;
    vflags = 0;
    vlocal = 0;
    path = null();

    argc = 0;
    argp = (*cmd).ncmd.args;
    osp = fill_arglist(&mut arglist, &mut argp);
    if !osp.is_null() {
        let mut pseudovarflag: c_int = 0;

        loop {
            find_command(
                (*arglist.list).text,
                &mut cmdentry,
                cmd_flag | DO_REGBLTIN,
                crate::var::pathval(),
            );

            vlocal += 1;

            /* implement bltin and command here */
            if cmdentry.cmdtype != CMDBUILTIN {
                break;
            }

            pseudovarflag = ((*cmdentry.u.cmd).flags & BUILTIN_ASSIGN) as c_int;
            if spclbltin < 0 {
                spclbltin = ((*cmdentry.u.cmd).flags & BUILTIN_SPECIAL) as c_int;
                vlocal = spclbltin ^ (BUILTIN_SPECIAL as c_int);
            }
            execcmd = (cmdentry.u.cmd == crate::builtins::EXECCMD) as c_int;
            if cmdentry.u.cmd != crate::builtins::COMMANDCMD {
                break;
            }

            cmd_flag = parse_command_args(&mut arglist, &mut argp, &mut path);
            if cmd_flag == 0 {
                break;
            }
        }

        while !argp.is_null() {
            crate::expand::expandarg(
                argp,
                &mut arglist,
                if pseudovarflag != 0 && crate::parser::isassignment((*argp).narg.text) != 0 {
                    EXP_VARTILDE
                } else {
                    EXP_FULL | EXP_TILDE
                },
            );
            argp = (*argp).narg.next;
        }

        sp = arglist.list;
        while !sp.is_null() {
            argc += 1;
            sp = (*sp).next;
        }

        if execcmd != 0 && argc > 1 {
            vflags = VEXPORT;
        }
    }

    localvar_stop = crate::var::pushlocalvars(vlocal);

    /* Reserve one extra spot at the front for shellexec. */
    nargv = stalloc(core::mem::size_of::<*mut c_char>() * (argc as usize + 2)) as *mut *mut c_char;
    nargv = nargv.add(1);
    argv = nargv;
    sp = arglist.list;
    while !sp.is_null() {
        /* TRACE(("evalcommand arg: %s\n", sp->text)); */
        *nargv = (*sp).text;
        nargv = nargv.add(1);
        sp = (*sp).next;
    }
    *nargv = null_mut();

    lastarg = null_mut();
    if iflag() != 0 && funcline == 0 && argc > 0 {
        lastarg = *nargv.offset(-1);
    }

    crate::output::preverrout.fd = 2;
    expredir((*cmd).ncmd.redirect);
    redir_stop = crate::redir::pushredir((*cmd).ncmd.redirect);
    status = crate::redir::redirectsafe((*cmd).ncmd.redirect, REDIR_PUSH | REDIR_SAVEFD2);

    'out_lbl: {
        'bail: {
            if status != 0 {
                break 'bail;
            }

            argp = (*cmd).ncmd.assign;
            while !argp.is_null() {
                let spp: *mut *mut strlist;

                spp = varlist.lastp;
                crate::expand::expandarg(argp, &mut varlist, EXP_VARTILDE);

                if vlocal != 0 {
                    crate::var::mklocal((**spp).text, VEXPORT);
                } else {
                    crate::var::setvareq((**spp).text, vflags);
                }
                argp = (*argp).narg.next;
            }

            /* Print the command if xflag is set. */
            if xflag() != 0 && inps4 == 0 {
                let out: *mut output;
                let mut sep: c_int;

                out = addr_of_mut!(crate::output::preverrout);
                inps4 = 1;
                crate::output::outstr(crate::parser::expandstr(crate::var::ps4val()), out);
                inps4 = 0;
                sep = 0;
                sep = eprintlist(out, varlist.list, sep);
                eprintlist(out, osp, sep);
                crate::output::outcslow('\n' as c_int, out);
            }

            /* Now locate the command. */
            if cmdentry.cmdtype != CMDBUILTIN || ((*cmdentry.u.cmd).flags & BUILTIN_REGULAR) == 0 {
                path = if !path.is_null() {
                    path
                } else {
                    crate::var::pathval()
                };
                find_command(*argv.offset(0), &mut cmdentry, cmd_flag | DO_ERR, path);
            }

            jp = null_mut();

            /* Execute the command. */
            match cmdentry.cmdtype {
                CMDUNKNOWN => {
                    status = 127;
                    break 'bail;
                }

                CMDBUILTIN => {
                    if evalbltin(cmdentry.u.cmd, argc, argv, flags) != 0
                        && !(crate::error::exception == crate::error::EXERROR && spclbltin <= 0)
                    {
                        // raise:
                        crate::error::raise_longjmp(crate::error::handler, 1);
                    }
                }

                CMDFUNCTION => {
                    if evalfun(cmdentry.u.func, argc, argv, flags) != 0 {
                        // goto raise
                        crate::error::raise_longjmp(crate::error::handler, 1);
                    }
                }

                _ => {
                    crate::input::flush_input();

                    /* Fork off a child process if necessary. */
                    if (flags & EV_EXIT) == 0 || crate::trap::have_traps() != 0 {
                        INTOFF();
                        jp = crate::jobs::vforkexec(cmd, argv, path, cmdentry.u.index);
                    } else {
                        shellexec(argv, path, cmdentry.u.index);
                        /* NOTREACHED */
                    }
                }
            }

            status = crate::jobs::waitforjob(jp);
            FORCEINTON();
            break 'out_lbl;
        }
        // bail:
        exitstatus = status;

        /* We have a redirection error. */
        if spclbltin > 0 {
            crate::error::exraise(crate::error::EXERROR);
        }

        // goto out
    }
    // out:
    if !(*cmd).ncmd.redirect.is_null() {
        crate::redir::popredir(execcmd);
    }
    crate::redir::unwindredir(redir_stop);
    crate::input::unwindfiles(file_stop);
    crate::var::unwindlocalvars(localvar_stop);
    if !lastarg.is_null() {
        /* dsl: I think this is intended to be used to support
         * '_' in 'vi' command mode during line editing...
         * However I implemented that within libedit itself.
         */
        crate::var::setvar(b"_\0".as_ptr() as *const c_char, lastarg, 0);
    }

    status
}

// [spec:dash:def:eval.evalbltin-fn]
// [spec:dash:sem:eval.evalbltin-fn]
unsafe fn evalbltin(
    cmd: *const builtincmd,
    argc: c_int,
    argv: *mut *mut c_char,
    flags: c_int,
) -> c_int {
    let savecmdname: *mut c_char; /* volatile */
    let savehandler: *mut jmploc; /* volatile */
    let mut jmploc_: jmploc = jmploc::new();
    let i: c_int;

    savecmdname = commandname;
    savehandler = crate::error::handler;
    let jl: *mut jmploc = &mut jmploc_;
    i = setjmp_catch(jl, || unsafe {
        let mut status: c_int;

        crate::error::handler = jl;
        commandname = *argv.offset(0);
        crate::options::argptr = argv.add(1);
        crate::options::optptr = null_mut(); /* initialize nextopt */
        if cmd == crate::builtins::EVALCMD {
            status = evalcmd(argc, argv, flags);
        } else {
            status = ((*cmd).builtin.unwrap())(argc, argv);
        }
        crate::output::flushall();
        if crate::output::outerr(out1) != 0 {
            crate::sh_warnx!(b"%s: I/O error\0".as_ptr() as *const c_char, commandname);
        }
        status |= crate::output::outerr(out1);
        exitstatus = status;
    });
    // cmddone:
    crate::output::freestdout();
    commandname = savecmdname;
    crate::error::handler = savehandler;

    i
}

// [spec:dash:def:eval.evalfun-fn]
// [spec:dash:sem:eval.evalfun-fn]
unsafe fn evalfun(func: *mut funcnode, argc: c_int, argv: *mut *mut c_char, flags: c_int) -> c_int {
    let saveparam: crate::options::shparam; /* volatile */
    let savehandler: *mut jmploc; /* volatile */
    let mut jmploc_: jmploc = jmploc::new();
    let e: c_int;
    let savefuncline: c_int;
    let saveloopnest: c_int;

    saveparam = crate::options::shellparam;
    savefuncline = funcline;
    saveloopnest = loopnest;
    savehandler = crate::error::handler;
    let jl: *mut jmploc = &mut jmploc_;
    e = setjmp_catch(jl, || unsafe {
        INTOFF();
        crate::error::handler = jl;
        crate::options::shellparam.malloc = 0;
        (*func).count += 1;
        funcline = (*func).n.ndefun.linno;
        loopnest = 0;
        INTON();
        crate::options::shellparam.nparam = argc - 1;
        crate::options::shellparam.p = argv.add(1);
        crate::options::shellparam.optind = 1;
        crate::options::shellparam.optoff = -1;
        evaltree((*func).n.ndefun.body, flags & EV_TESTED);
    });
    // funcdone:
    INTOFF();
    loopnest = saveloopnest;
    funcline = savefuncline;
    crate::nodes::freefunc(func);
    crate::options::freeparam(addr_of_mut!(crate::options::shellparam));
    crate::options::shellparam = saveparam;
    crate::error::handler = savehandler;
    INTON();
    evalskip &= !(SKIPFUNC | SKIPFUNCDEF);
    e
}

/*
 * Search for a command.  This is called before we fork so that the
 * location of the command will be available in the parent as well as
 * the child.  The check for "goodname" is an overly conservative
 * check that the name will not be subject to expansion.
 */

// [spec:dash:def:eval.prehash-fn]
// [spec:dash:sem:eval.prehash-fn]
unsafe fn prehash(n: *mut Node) {
    let mut entry: cmdentry = cmdentry {
        cmdtype: 0,
        u: param { index: 0 },
    };

    if (*n).r#type == NCMD && !(*n).ncmd.args.is_null() {
        if crate::parser::goodname((*(*n).ncmd.args).narg.text) != 0 {
            find_command(
                (*(*n).ncmd.args).narg.text,
                &mut entry,
                0,
                crate::var::pathval(),
            );
        }
    }
}

/*
 * Builtin commands.  Builtin commands whose functions are closely
 * tied to evaluation are implemented here.
 */

/*
 * No command given.
 */

// [spec:dash:def:eval.bltincmd-fn]
// [spec:dash:sem:eval.bltincmd-fn]
unsafe fn bltincmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    /*
     * Preserve exitstatus of a previous possible redirection
     * as POSIX mandates
     */
    back_exitstatus
}

/*
 * Handle break and continue commands.  Break, continue, and return are
 * all handled by setting the evalskip flag.  The evaluation routines
 * above all check this flag, and if it is set they start skipping
 * commands rather than executing them.  The variable skipcount is
 * the number of loops to break/continue, or the number of function
 * levels to return.  (The latter is always 1.)  It should probably
 * be an error to break out of more loops than exist, but it isn't
 * in the standard shell so we don't make it one here.
 */

// [spec:dash:def:eval.breakcmd-fn]
// [spec:dash:sem:eval.breakcmd-fn]
pub unsafe fn breakcmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    let mut n: c_int = if argc > 1 {
        crate::mystring::number(*argv.offset(1))
    } else {
        1
    };

    if n <= 0 {
        crate::mystring::badnum(*argv.offset(1));
    }
    if n > loopnest {
        n = loopnest;
    }
    if n > 0 {
        evalskip = if **argv == b'c' as c_char {
            SKIPCONT
        } else {
            SKIPBREAK
        };
        skipcount = n;
    }
    0
}

/*
 * The return command.
 */

// [spec:dash:def:eval.returncmd-fn]
// [spec:dash:sem:eval.returncmd-fn]
pub unsafe fn returncmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    let skip: c_int;
    let status: c_int;

    /*
     * If called outside a function, do what ksh does;
     * skip the rest of the file.
     */
    if !(*argv.offset(1)).is_null() {
        skip = SKIPFUNC;
        status = crate::mystring::number(*argv.offset(1));
    } else {
        skip = SKIPFUNCDEF;
        status = exitstatus;
    }
    evalskip = skip;

    status
}

// [spec:dash:def:eval.falsecmd-fn]
// [spec:dash:sem:eval.falsecmd-fn]
pub unsafe fn falsecmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    1
}

// [spec:dash:def:eval.truecmd-fn]
// [spec:dash:sem:eval.truecmd-fn]
pub unsafe fn truecmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    0
}

// [spec:dash:def:eval.execcmd-fn]
// [spec:dash:sem:eval.execcmd-fn]
pub unsafe fn execcmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    if argc > 1 {
        crate::options::optlist[crate::options::iflag] = 0; /* exit on error */
        crate::options::optlist[crate::options::mflag] = 0;
        crate::options::optschanged();
        crate::input::flush_input();
        shellexec(argv.add(1), crate::var::pathval(), 0);
    }
    0
}

// [spec:dash:def:eval.eprintlist-fn]
// [spec:dash:sem:eval.eprintlist-fn]
unsafe fn eprintlist(out: *mut output, sp: *mut strlist, sep: c_int) -> c_int {
    let mut sp: *mut strlist = sp;
    let mut sep: c_int = sep;

    while !sp.is_null() {
        let mut p: *const c_char;

        p = b" %s\0".as_ptr() as *const c_char;
        p = p.offset((1 - sep) as isize);
        sep |= 1;
        crate::outfmt!(out, p, (*sp).text);
        sp = (*sp).next;
    }

    sep
}
