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

use crate::error::Error;
use bstr::{BStr, BString};
use core::ptr::{addr_of, addr_of_mut, null, null_mut};
use libc::{c_char, c_int};
use std::ffi::{CStr, CString};
use std::io::Write as _;

use crate::builtins::{BUILTIN_ASSIGN, BUILTIN_REGULAR, BUILTIN_SPECIAL, Builtin, builtincmd};
use crate::error::{FORCEINTON, INTOFF, INTON, jmploc};
use crate::exec::{CMDBUILTIN, CMDFUNCTION, CMDNORMAL, CMDUNKNOWN, DO_ERR, DO_NOFUNC, DO_REGBLTIN};
use crate::exec::{cmdentry, find_command, param, shellexec};
use crate::expand::{EXP_FULL, EXP_MBCHAR, EXP_REDIR, EXP_TILDE, EXP_VARTILDE};
use crate::expand::{arglist, strlist};
use crate::jobs::FORK_NOJOB;
use crate::nodes::{
    NAND, NAPPEND, NBACKGND, NCASE, NCLOBBER, NCMD, NDEFUN, NFOR, NFROM, NFROMFD, NFROMTO, NIF,
    NNOT, NOR, NPIPE, NREDIR, NSEMI, NSUBSHELL, NTO, NTOFD, NUNTIL, NWHILE,
};
use crate::nodes::{Node, funcnode};
use crate::output::Output;
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
    pub fd: c_int,         /* file descriptor to read from */
    pub buf: *mut c_char,  /* buffer */
    pub nleft: c_int,      /* number of chars in buffer */
    pub jp: Option<usize>, /* index of the job structure for command */
}

// ---------------------------------------------------------------------
// module globals
// ---------------------------------------------------------------------

pub static mut evalskip: c_int = 0; /* set if we are skipping commands */
pub(crate) static mut skipcount: c_int = 0; /* number of levels to skip */
pub static mut loopnest: c_int = 0; /* current loop nesting level (MKINIT) */
static mut funcline: c_int = 0; /* starting line number of current function, or 0 */

/// The name the running builtin was invoked by, for the error prefix.
///
/// dash points this at `argv[0]` and relies on the word outliving the
/// call. Owning the bytes states that lifetime instead of assuming it,
/// which is what lets `dotcmd` stop keeping its resolved path alive in a
/// static of its own.
pub static mut commandname: Option<BString> = None;
pub static mut exitstatus: c_int = 0; /* exit status of last command */
pub static mut back_exitstatus: c_int = 0; /* exit status of backquoted command */
pub static mut savestatus: c_int = -1; /* exit status of last command outside traps */

/* Prevent PS4 nesting. */
pub static mut inps4: c_int = 0; /* MKINIT */

pub static mut tpip: [c_int; 2] = [-1, 0]; /* MKINIT int tpip[2] = { -1 } */

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

/*
 * Execute a command or commands contained in a string.
 */

// [spec:dash:def:eval.evalstring-fn]
// [spec:dash:sem:eval.evalstring-fn]
pub unsafe fn evalstring(s: *mut c_char, flags: c_int) -> Result<c_int, Error> {
    let mut status: c_int;
    /* `sstrdup(s)` and the `stunalloc(s)` at the bottom are one thing:
     * `setinputstring` keeps the pointer rather than copying, so the text
     * has to outlive every `popstackmark` the parse below performs — which
     * is why the copy is taken *before* the mark is set and released by
     * hand afterwards.  Owning it says both halves at once, and says them
     * on the unwind path too, where the C's `stunalloc` never runs. */
    let owned: Vec<u8> = CStr::from_ptr(s).to_bytes_with_nul().to_vec();
    let s: *mut c_char = owned.as_ptr() as *mut c_char;

    crate::input::setinputstring(s);
    status = 0;
    loop {
        let n: Option<Node> = match crate::parser::parsecmd(0) {
            crate::parser::ParseResult::Eof => break,
            crate::parser::ParseResult::Tree(n) => n,
        };
        {
            let i: c_int;

            i = evaltree(
                n.as_ref(),
                flags
                    & !(if crate::parser::parser_eof() != 0 {
                        0
                    } else {
                        EV_EXIT
                    }),
            )?;
            if n.is_some() {
                status = i;
            }

            if evalskip != 0 {
                break;
            }
        }
        /* `popstackmark(&smark)` — one per parsed command, and one on the
         * way out. */
    }
    crate::input::popfile();
    drop(owned);

    Ok(status)
}

/*
 * Evaluate a parse tree.  The value is left in the global variable
 * exitstatus.
 */

// [spec:dash:def:eval.evaltree-fn]
// [spec:dash:sem:eval.evaltree-fn]
pub unsafe fn evaltree(n: Option<&Node>, flags: c_int) -> Result<c_int, Error> {
    let mut checkexit: c_int = 0;
    /* C leaves `evalfn` uninitialised; every path that reaches
     * `calleval` assigns it first. Seeded here only so that Rust's
     * definite-initialisation analysis is trivially satisfied — any of
     * the six is as good, and `evaltree` itself no longer fits the type,
     * because the leaf evaluators all dereference their node. */
    let mut evalfn: unsafe fn(&Node, c_int) -> Result<c_int, Error> = evalcommand;
    let isor: libc::c_uint;
    let mut status: c_int = 0;

    'out_lbl: {
        if nflag() != 0 {
            break 'out_lbl;
        }

        let n: &Node = match n {
            Some(n) => n,
            None => {
                /* TRACE(("evaltree(NULL) called\n")); */
                break 'out_lbl;
            }
        };

        crate::trap::dotrap();

        /* #ifndef SMALL: show history substitutions done with fc */
        crate::histedit::displayhist = 1;

        /* TRACE(("pid %d, evaltree(%p: %d, %d) called\n", ...)); */
        /* The C's `goto evaln` reassigns `n` and jumps; the node it jumps
         * with travels here instead, because `n` is a borrow. */
        let mut nnext: Option<&Node> = None;
        'sw: {
            'calleval: {
                'evaln: {
                    'checkexit_lbl: {
                        match n.node_type() {
                            NREDIR => {
                                let r = n.nredir();
                                crate::error::errlinno = r.linno;
                                crate::var::lineno = r.linno;
                                if funcline != 0 {
                                    crate::var::lineno -= funcline - 1;
                                }
                                expredir(&r.redirect)?;
                                crate::redir::pushredir(&r.redirect);
                                status = crate::redir::redirectsafe(&r.redirect, REDIR_PUSH);
                                if status != 0 {
                                    checkexit = EV_TESTED;
                                } else {
                                    status = evaltree(r.n.as_deref(), flags & EV_TESTED)?;
                                }
                                if !r.redirect.is_empty() {
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
                                isor = (n.node_type() - NAND) as libc::c_uint;
                                let b = n.nbinary();
                                status = evaltree(
                                    b.ch1.as_deref(),
                                    (flags | (((isor >> 1).wrapping_sub(1)) as c_int)) & EV_TESTED,
                                )?;
                                if ((status == 0) as libc::c_uint) == isor || evalskip != 0 {
                                    break 'sw;
                                }
                                nnext = b.ch2.as_deref();
                                break 'evaln;
                            }
                            NIF => {
                                let f = n.nif();
                                status = evaltree(f.test.as_deref(), EV_TESTED)?;
                                if evalskip != 0 {
                                    break 'sw;
                                }
                                if status == 0 {
                                    nnext = f.ifpart.as_deref();
                                    break 'evaln;
                                } else if f.elsepart.is_some() {
                                    nnext = f.elsepart.as_deref();
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
                             * into `case NNOT:`. No other node type reaches
                             * `evaltree`, so with a tagged union there is
                             * nothing left for the fallthrough to reinterpret. */
                            _ /* default, NNOT */ => {
                                status = evaltree(n.nnot().com.as_deref(), EV_TESTED)?;
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
                // evaln: the C sets `evalfn = evaltree` and falls into
                // `calleval:`, which with the reassigned node is this call.
                status = evaltree(nnext, flags)?;
                break 'sw;
            }
            // calleval:
            status = evalfn(n, flags)?;
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

        return Ok(exitstatus);
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
pub unsafe fn evaltreenr(n: Option<&Node>, flags: c_int) -> ! {
    /* `evaltree` is fallible now and this one cannot be: it is the
     * `noreturn` alias, called where the caller has already committed to
     * not coming back. The bridge performs the jump the raise performed. */
    evaltree(n, flags)
        .unwrap_or_else(|e| crate::error::raise_reported(crate::error::EXERROR, e));
    std::process::abort();
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
unsafe fn evalloop(n: &Node, flags: c_int) -> Result<c_int, Error> {
    let mut skip: c_int;
    let mut status: c_int;
    let mut flags: c_int = flags;

    loopnest += 1;
    status = 0;
    flags &= EV_TESTED;
    loop {
        {
            let mut i: c_int;

            i = evaltree(n.nbinary().ch1.as_deref(), EV_TESTED)?;
            skip = skiploop();
            if skip == SKIPFUNC {
                status = i;
            }
            if skip != 0 {
                /* `continue` in the C do/while: re-test the condition */
            } else {
                if n.node_type() != NWHILE {
                    i = (i == 0) as c_int;
                }
                if i != 0 {
                    break;
                }
                status = evaltree(n.nbinary().ch2.as_deref(), flags)?;
                skip = skiploop();
            }
        }
        if (skip & !SKIPCONT) != 0 {
            break;
        }
    }
    loopnest -= 1;

    Ok(status)
}

// [spec:dash:def:eval.evalfor-fn]
// [spec:dash:sem:eval.evalfor-fn]
unsafe fn evalfor(n: &Node, flags: c_int) -> Result<c_int, Error> {
    let mut arglist: arglist = arglist::new();
    let mut status: c_int;
    let mut flags: c_int = flags;

    let f = n.nfor();
    crate::error::errlinno = f.linno;
    crate::var::lineno = f.linno;
    if funcline != 0 {
        crate::var::lineno -= funcline - 1;
    }

    for argp in &f.args {
        crate::expand::expandarg(argp, Some(&mut arglist), EXP_FULL | EXP_TILDE)?;
    }

    status = 0;
    loopnest += 1;
    flags &= EV_TESTED;
    for sp in &arglist.list {
        /* The evaluator is not converted yet, so the bridge performs the
         * jump `setvar`'s `sh_error` performed; it retires when this frame
         * returns `Result`. */
        crate::var::setvar(f.var.as_ptr(), sp.textp(), 0)
            .unwrap_or_else(|e| crate::error::raise_reported(crate::error::EXERROR, e));
        status = evaltree(f.body.as_deref(), flags)?;
        if (skiploop() & !SKIPCONT) != 0 {
            break;
        }
    }
    loopnest -= 1;

    Ok(status)
}

// [spec:dash:def:eval.evalcase-fn]
// [spec:dash:sem:eval.evalcase-fn]
unsafe fn evalcase(n: &Node, flags: c_int) -> Result<c_int, Error> {
    let mut arglist: arglist = arglist::new();
    let mut status: c_int = 0;

    let c = n.ncase();
    crate::error::errlinno = c.linno;
    crate::var::lineno = c.linno;
    if funcline != 0 {
        crate::var::lineno -= funcline - 1;
    }

    crate::expand::expandarg(
        c.expr.as_deref().unwrap(),
        Some(&mut arglist),
        if crate::mystring::FNMATCH_IS_ENABLED != 0 {
            EXP_TILDE
        } else {
            EXP_TILDE | EXP_MBCHAR
        },
    )?;
    /* The C reads `arglist.list->text` with no null check, and is right to:
     * `expandarg` without EXP_FULL takes its single-field arm, which appends
     * exactly one entry whatever the word expands to. */
    debug_assert_eq!(arglist.list.len(), 1, "an unsplit expansion is one field");
    'out_lbl: {
        for cp in &c.cases {
            if evalskip != 0 {
                break;
            }
            for patp in &cp.nclist().pattern {
                if crate::expand::casematch(patp, arglist.list[0].textp())? != 0 {
                    /* Ensure body is non-empty as otherwise
                     * EV_EXIT may prevent us from setting the
                     * exit status.
                     */
                    if evalskip == 0 && cp.nclist().body.is_some() {
                        status = evaltree(cp.nclist().body.as_deref(), flags)?;
                    }
                    break 'out_lbl;
                }
            }
        }
    }
    // out:
    Ok(status)
}

/*
 * Kick off a subshell to evaluate a tree.
 */

// [spec:dash:def:eval.evalsubshell-fn]
// [spec:dash:sem:eval.evalsubshell-fn]
unsafe fn evalsubshell(n: &Node, flags: c_int) -> Result<c_int, Error> {
    let jp: usize;
    let backgnd: c_int = (n.node_type() == NBACKGND) as c_int;
    let mut status: c_int;
    let mut flags: c_int = flags;

    let r = n.nredir();
    crate::error::errlinno = r.linno;
    crate::var::lineno = r.linno;
    if funcline != 0 {
        crate::var::lineno -= funcline - 1;
    }

    expredir(&r.redirect)?;
    INTOFF();
    'nofork: {
        if backgnd == 0 && (flags & EV_EXIT) != 0 && crate::trap::have_traps() == 0 {
            crate::init::forkreset(None);
            break 'nofork;
        }
        jp = crate::jobs::makejob(1);
        if crate::jobs::forkshell(Some(jp), r.n.as_deref(), backgnd)? == 0 {
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
            status = crate::jobs::waitforjob(Some(jp))?;
        }
        INTON();
        return Ok(status);
    }
    // nofork:
    INTON();
    crate::redir::redirect(&r.redirect, 0)?;
    evaltreenr(r.n.as_deref(), flags)
    /* never returns */
}

/*
 * Compute the names of the files in a redirection list.
 */

// [spec:dash:def:eval.expredir-fn]
// [spec:dash:sem:eval.expredir-fn]
unsafe fn expredir(n: &[Node]) -> Result<(), Error> {
    for redir in n {
        let mut fnl: arglist = arglist::new();
        match redir.node_type() {
            NFROMTO | NFROM | NTO | NCLOBBER | NAPPEND => {
                crate::expand::expandarg(
                    redir.nfile().fname.as_deref().unwrap(),
                    Some(&mut fnl),
                    EXP_TILDE | EXP_REDIR,
                )?;
                /* `fn.list->text` with no null check: no EXP_FULL means
                 * `expandarg` took its single-field arm. */
                debug_assert_eq!(fnl.list.len(), 1, "an unsplit expansion is one field");
                /* `redir->nfile.expfname = fn.list->text` — the C hands the
                 * node a pointer into the region and relies on this
                 * function's caller not popping its mark before `redirect`
                 * has run. The node owns the bytes instead; `fnl` is a
                 * per-iteration local and its list would be gone one
                 * statement later.  Now that the field owns them too this
                 * is the C's assignment exactly: a move, not a copy. */
                *redir.nfile().expfname.borrow_mut() = Some(fnl.list.remove(0).text);
            }
            NFROMFD | NTOFD => {
                /* The borrow of `vname` ends before `fixredir`, which writes
                 * `dupfd` on this same node. */
                let expand = {
                    let vname = redir.ndup().vname.borrow();
                    match vname.as_deref() {
                        None => false,
                        Some(v) => {
                            crate::expand::expandarg(v, Some(&mut fnl), EXP_TILDE | EXP_REDIR)?;
                            true
                        }
                    }
                };
                if expand {
                    debug_assert_eq!(fnl.list.len(), 1, "an unsplit expansion is one field");
                    crate::parser::fixredir(redir, fnl.list[0].textp(), 1);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/*
 * Evaluate a pipeline.  All the processes in the pipeline are children
 * of the process creating the pipeline.  (This differs from some versions
 * of the shell, which make the last process in a pipeline the parent
 * of all the rest.)
 */

// [spec:dash:def:eval.evalpipe-fn]
// [spec:dash:sem:eval.evalpipe-fn]
unsafe fn evalpipe(n: &Node, flags: c_int) -> Result<c_int, Error> {
    let jp: usize;
    let pipelen: c_int;
    let mut prevfd: c_int;
    let mut pip: [c_int; 2] = [0; 2];
    let mut status: c_int = 0;
    let mut flags: c_int = flags;

    /* TRACE(("evalpipe(0x%lx) called\n", (long)n)); */
    let p = n.npipe();
    pipelen = p.cmdlist.len() as c_int;
    flags |= EV_EXIT;
    INTOFF();
    jp = crate::jobs::makejob(pipelen);
    prevfd = -1;
    for (i, cmd) in p.cmdlist.iter().enumerate() {
        let has_next = i + 1 < p.cmdlist.len();
        prehash(cmd)?;
        pip[1] = -1;
        if has_next {
            if libc::pipe(pip.as_mut_ptr()) < 0 {
                libc::close(prevfd);
                /* Between this frame's `INTOFF` and its `INTON`, exactly
                 * where the longjmp was: the jump skipped the same `INTON`
                 * and left the counter raised. Pairing them with a guard
                 * would move the instruction a pending SIGINT is delivered
                 * at, which `docs/errors-are-values.md` §2.4 forbids. */
                return Err(crate::error::sh_error_value(b"Pipe call failed"));
            }
        }
        if crate::jobs::forkshell(Some(jp), Some(cmd), p.backgnd)? == 0 {
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
            evaltreenr(Some(cmd), flags);
            /* never returns */
        }
        if prevfd >= 0 {
            libc::close(prevfd);
        }
        prevfd = pip[0];
        libc::close(pip[1]);
    }
    if p.backgnd == 0 {
        status = crate::jobs::waitforjob(Some(jp))?;
        /* TRACE(("evalpipe:  job done exit status %d\n", status)); */
    }
    INTON();

    Ok(status)
}

/*
 * Execute a command inside back quotes.  If it's a builtin command, we
 * want to save its output in a block obtained from malloc.  Otherwise
 * we fork off a subprocess and get the output of the command via a pipe.
 * Should be called with interrupts off.
 */

// [spec:dash:def:eval.evalbackcmd-fn]
// [spec:dash:sem:eval.evalbackcmd-fn]
pub unsafe fn evalbackcmd(n: Option<&Node>, result: *mut backcmd) -> Result<(), Error> {
    let jp: usize;
    let mut pip: [c_int; 2] = [0; 2];
    let pid: c_int;

    (*result).fd = -1;
    (*result).buf = null_mut();
    (*result).nleft = 0;
    (*result).jp = None;
    'out_lbl: {
        if n.is_none() {
            break 'out_lbl;
        }

        crate::redir::sh_pipe(pip.as_mut_ptr(), 0)?;
        tpip[0] = pip[0];
        tpip[1] = pip[1];
        jp = crate::jobs::makejob(1);
        pid = crate::jobs::forkshell(Some(jp), n, FORK_NOJOB)?;
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
        (*result).jp = Some(jp);
    }
    // out:
    /* TRACE(("evalbackcmd done: fd=%d buf=0x%x nleft=%d jp=0x%x\n", ...)); */
    Ok(())
}

// [spec:dash:def:eval.fill-arglist-fn]
// [spec:dash:sem:eval.fill-arglist-fn]
//
// The C's `argpp` is a `union node **` cursor walking `narg.next`; the
// argument list is a slice now, so the cursor is the unconsumed tail of it.
// The return value is the C's `*lastp`: the first entry this call appended,
// or NULL if the argument list ran out without producing one. As an index it
// is the length the list had on entry, so the answer is `Some` exactly when
// the list grew.
unsafe fn fill_arglist<'a>(
    arglist: &mut arglist,
    argpp: &mut &'a [Node],
) -> Result<Option<usize>, Error> {
    let lastp: usize = arglist.list.len();

    loop {
        let Some((argp, rest)) = argpp.split_first() else {
            break;
        };
        crate::expand::expandarg(argp, Some(arglist), EXP_FULL | EXP_TILDE)?;
        *argpp = rest;
        if arglist.list.len() != lastp {
            break;
        }
    }

    if arglist.list.len() != lastp {
        Ok(Some(lastp))
    } else {
        Ok(None)
    }
}

// [spec:dash:def:eval.parse-command-args-fn]
// [spec:dash:sem:eval.parse-command-args-fn]
// `head` is the C's `arglist->list`, which this function reassigns to skip
// the `command [-p]` words it consumed. A `Vec`'s start does not move, so the
// head is an index the caller keeps; see [`crate::expand::arglist`].
unsafe fn parse_command_args(
    arglist: &mut arglist,
    argpp: &mut &[Node],
    path: *mut *const c_char,
    head: &mut usize,
) -> Result<c_int, Error> {
    let mut sp: usize = *head;
    let mut cp: *mut c_char;
    let mut c: c_char;

    loop {
        /* `sp = sp->next ? sp->next : fill_arglist(arglist, argpp)` */
        sp = if sp + 1 < arglist.list.len() {
            sp + 1
        } else {
            match fill_arglist(arglist, argpp)? {
                Some(i) => i,
                None => return Ok(0),
            }
        };
        cp = arglist.list[sp].textp();
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
            if sp + 1 >= arglist.list.len() && fill_arglist(arglist, argpp)?.is_none() {
                return Ok(0);
            }
            sp += 1;
            break;
        }
        loop {
            match c as u8 {
                b'p' => {
                    *path = crate::var::defpath();
                }
                _ => {
                    /* run 'typecmd' for other options */
                    return Ok(0);
                }
            }
            c = *cp;
            cp = cp.add(1);
            if c == 0 {
                break;
            }
        }
    }

    *head = sp;
    Ok(DO_NOFUNC)
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
unsafe fn evalcommand(cmd: &Node, flags: c_int) -> Result<c_int, Error> {
    let localvar_stop: usize;
    let file_stop: usize;
    let redir_stop: usize;
    let mut argp: &[Node];
    let mut arglist: arglist = arglist::new();
    let mut varlist: arglist = arglist::new();
    let argv: *mut *mut c_char;
    let mut argc: c_int;
    let osp: Option<usize>;
    /* The C's `arglist.list`, which `parse_command_args` moves past the
     * `command [-p]` words while `osp` keeps the original head for `set -x`. */
    let mut head: usize = 0;
    let mut cmdentry: cmdentry = cmdentry {
        cmdtype: 0,
        u: param { index: 0 },
    };
    let mut jp: Option<usize>;
    let mut lastarg: *mut c_char;
    let mut path: *const c_char;
    let mut spclbltin: c_int;
    let mut cmd_flag: c_int;
    let mut execcmd: c_int;
    let mut status: c_int;
    let mut nargv: *mut *mut c_char;
    let mut vflags: c_int;
    let mut vlocal: c_int;

    let c = cmd.ncmd();
    crate::error::errlinno = c.linno;
    crate::var::lineno = c.linno;
    if funcline != 0 {
        crate::var::lineno -= funcline - 1;
    }

    /* First expand the arguments. */
    /* TRACE(("evalcommand(0x%lx, %d) called\n", (long)cmd, flags)); */
    file_stop = crate::input::cur_mark();
    back_exitstatus = 0;

    cmdentry.cmdtype = CMDBUILTIN;
    cmdentry.u.cmd = addr_of_mut!(crate::builtins::bltin);

    cmd_flag = 0;
    execcmd = 0;
    spclbltin = -1;
    vflags = 0;
    vlocal = 0;
    path = null();

    argc = 0;
    argp = c.args.as_slice();
    osp = fill_arglist(&mut arglist, &mut argp)?;
    if osp.is_some() {
        let mut pseudovarflag: c_int = 0;

        loop {
            find_command(
                arglist.list[head].textp(),
                &mut cmdentry,
                cmd_flag | DO_REGBLTIN,
                crate::var::pathval(),
            )?;

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

            cmd_flag = parse_command_args(&mut arglist, &mut argp, &mut path, &mut head)?;
            if cmd_flag == 0 {
                break;
            }
        }

        for a in argp {
            crate::expand::expandarg(
                a,
                Some(&mut arglist),
                if pseudovarflag != 0 && crate::parser::isassignment(a.narg().text.as_ptr()) != 0 {
                    EXP_VARTILDE
                } else {
                    EXP_FULL | EXP_TILDE
                },
            )?;
        }

        argc = (arglist.list.len() - head) as c_int;

        if execcmd != 0 && argc > 1 {
            vflags = VEXPORT;
        }
    }

    localvar_stop = crate::var::pushlocalvars(vlocal);

    /* Reserve one extra spot at the front for shellexec.
     *
     * The C `stalloc`s `argc + 2` pointers and hands out `+ 1`, so
     * `shellexec` can write `argv[-1]`; the block lives until this
     * function's `popstackmark`.  A `Vec` of the same length owned by
     * this frame is the same lifetime, and covers the unwind out of a
     * builtin that the C's mark covers only because the handler pops
     * it. */
    let mut argvbuf: Vec<*mut c_char> = vec![null_mut(); argc as usize + 2];
    let argvend: *mut *mut c_char = argvbuf.as_mut_ptr().add(argc as usize + 2);
    nargv = argvbuf.as_mut_ptr().add(1);
    argv = nargv;
    for sp in &arglist.list[head..] {
        /* TRACE(("evalcommand arg: %s\n", sp->text)); */
        *nargv = sp.textp();
        nargv = nargv.add(1);
    }
    *nargv = null_mut();
    /* `argc` was counted off the same list a few lines above, so the
     * terminator lands at `argvbuf[argc + 1]` and the last slot is spare.
     * A `stalloc`'d block that is one short overruns into whatever the
     * region hands out next; a `Vec` that is one short is a heap
     * overflow, so the count is asserted rather than assumed. */
    debug_assert!(nargv < argvend);

    /* The same words as `argv`, in the shape a builtin takes them: no
     * terminator, no array, and borrowed from `arglist` -- which this
     * frame owns, so a builtin that re-enters evaluation is not holding
     * anything the shell might move underneath it. */
    let args: Vec<&BStr> = crate::builtins::args(&arglist.list[head..]);

    lastarg = null_mut();
    if iflag() != 0 && funcline == 0 && argc > 0 {
        lastarg = *nargv.offset(-1);
    }

    (*crate::output::previous_stderr()).fd = crate::streams::streams().stderr;
    expredir(&c.redirect)?;
    redir_stop = crate::redir::pushredir(&c.redirect);
    status = crate::redir::redirectsafe(&c.redirect, REDIR_PUSH | REDIR_SAVEFD2);

    'out_lbl: {
        'bail: {
            if status != 0 {
                break 'bail;
            }

            for a in &c.assign {
                let spp: usize;

                spp = varlist.list.len();
                crate::expand::expandarg(a, Some(&mut varlist), EXP_VARTILDE)?;
                /* `(*spp)->text` with no null check: EXP_VARTILDE has no
                 * EXP_FULL, so `expandarg` appended exactly one entry. */
                debug_assert_eq!(
                    varlist.list.len(),
                    spp + 1,
                    "an unsplit expansion is one field"
                );

                /* Bridges until `evalcommand` returns `Result`. */
                let raise =
                    |e| crate::error::raise_reported(crate::error::EXERROR, e);
                if vlocal != 0 {
                    crate::var::mklocal(varlist.list[spp].textp(), VEXPORT)
                        .unwrap_or_else(raise);
                } else {
                    crate::var::setvareq(varlist.list[spp].textp(), vflags)
                        .unwrap_or_else(|e| {
                            crate::error::raise_reported(crate::error::EXERROR, e)
                        });
                }
            }

            /* Print the command if xflag is set. */
            if xflag() != 0 && inps4 == 0 {
                let out: *mut Output;
                let mut sep: c_int;

                out = crate::output::previous_stderr();
                inps4 = 1;
                let prompt = crate::parser::expandstr(crate::var::ps4val());
                let _ = (&mut *out).write_all(CStr::from_ptr(prompt).to_bytes());
                inps4 = 0;
                sep = 0;
                sep = eprintlist(out, &varlist.list, sep);
                /* `eprintlist(out, osp, sep)` prints from the *original*
                 * head, so `command -p foo` traces as it was written and not
                 * as `parse_command_args` left it.  A NULL `osp` prints
                 * nothing, which is the empty slice. */
                eprintlist(out, &arglist.list[osp.unwrap_or(arglist.list.len())..], sep);
                let _ = (&mut *out).write_all(b"\n");
            }

            /* Now locate the command. */
            if cmdentry.cmdtype != CMDBUILTIN || ((*cmdentry.u.cmd).flags & BUILTIN_REGULAR) == 0 {
                path = if !path.is_null() {
                    path
                } else {
                    crate::var::pathval()
                };
                find_command(*argv.offset(0), &mut cmdentry, cmd_flag | DO_ERR, path)?;
            }

            jp = None;

            /* Execute the command. */
            match cmdentry.cmdtype {
                CMDUNKNOWN => {
                    status = 127;
                    break 'bail;
                }

                CMDBUILTIN => {
                    if evalbltin(cmdentry.u.cmd, &args, flags) != 0
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
                        jp = Some(crate::jobs::vforkexec(cmd, argv, path, cmdentry.u.index)?);
                    } else {
                        shellexec(argv, path, cmdentry.u.index);
                        /* NOTREACHED */
                    }
                }
            }

            status = crate::jobs::waitforjob(jp)?;
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
    if !c.redirect.is_empty() {
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
        crate::var::setvar(b"_\0".as_ptr() as *const c_char, lastarg, 0)
            .unwrap_or_else(|e| crate::error::raise_reported(crate::error::EXERROR, e));
    }

    Ok(status)
}

// [spec:dash:def:eval.evalbltin-fn]
// [spec:dash:sem:eval.evalbltin-fn]
unsafe fn evalbltin(cmd: *const builtincmd, args: &[&BStr], flags: c_int) -> c_int {
    let savecmdname: Option<BString>; /* volatile */
    let savehandler: *mut jmploc; /* volatile */
    let mut jmploc_: jmploc = jmploc::new();
    let i: c_int;

    savecmdname = core::mem::take(&mut *addr_of_mut!(commandname));
    savehandler = crate::error::handler;
    let jl: *mut jmploc = &mut jmploc_;
    i = setjmp_catch(jl, || unsafe {
        let mut status: c_int;

        crate::error::handler = jl;
        /* `commandname = argv[0]`, and NULL for the command that has no
         * word at all -- the assignment-only one `bltin` stands for. */
        commandname = args.first().map(|name| BString::from(<&BStr as AsRef<[u8]>>::as_ref(name)));
        /* A builtin hands its diagnostic back now instead of jumping out
         * with it, and this frame is the handler that jump was aimed at.
         * The bridge raises it again right here, which skips the `flushall`
         * and the `outerr` check below exactly as the longjmp did, and
         * lands in this closure's own `setjmp_catch` with `exception` set
         * to EXERROR for `evalcommand` to inspect. `evalbltin` itself stops
         * needing this when the catch frames convert. */
        let raise = |e| crate::error::raise_reported(crate::error::EXERROR, e);
        if cmd == crate::builtins::EVALCMD {
            status = crate::builtins::eval::evalcmd(args, flags).unwrap_or_else(raise);
        } else {
            let entry = (*cmd).builtin.expect("a builtin with no special entry");
            status = entry(args).unwrap_or_else(raise);
        }
        crate::output::flushall();
        if crate::output::outerr(crate::output::stdout()) != 0 {
            let mut message = Vec::new();
            if let Some(name) = &*addr_of!(commandname) {
                message.extend_from_slice(name);
            }
            message.extend_from_slice(b": I/O error");
            crate::error::sh_warnx(&message);
        }
        status |= crate::output::outerr(crate::output::stdout());
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
unsafe fn evalfun(
    func: *const funcnode,
    argc: c_int,
    argv: *mut *mut c_char,
    flags: c_int,
) -> c_int {
    let saveparam: crate::options::shparam; /* volatile */
    let savehandler: *mut jmploc; /* volatile */
    let mut jmploc_: jmploc = jmploc::new();
    let e: c_int;
    let savefuncline: c_int;
    let saveloopnest: c_int;

    /* `saveparam = shellparam` plus the `shellparam.malloc = 0` that the C
     * puts inside the protected region so the epilogue's `freeparam` cannot
     * reach what the copy still points at. */
    saveparam = crate::options::takeparam();
    savefuncline = funcline;
    saveloopnest = loopnest;
    savehandler = crate::error::handler;
    let jl: *mut jmploc = &mut jmploc_;
    e = setjmp_catch(jl, || unsafe {
        INTOFF();
        crate::error::handler = jl;
        /* `func->count++`: the second reference that keeps the body alive if
         * the function is redefined while it runs. */
        crate::nodes::reffunc(func);
        funcline = (*func).ndefun().linno;
        loopnest = 0;
        INTON();
        crate::options::borrowparam(argv.add(1), argc - 1);
        /* Inside `evalfun`'s `setjmp_catch`; the bridge retires with that
         * frame at step D. */
        evaltree((*func).ndefun().body.as_deref(), flags & EV_TESTED)
            .unwrap_or_else(|e| crate::error::raise_reported(crate::error::EXERROR, e));
    });
    // funcdone:
    INTOFF();
    loopnest = saveloopnest;
    funcline = savefuncline;
    crate::nodes::freefunc(func);
    crate::options::restoreparam(saveparam);
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
unsafe fn prehash(n: &Node) -> Result<(), Error> {
    let mut entry: cmdentry = cmdentry {
        cmdtype: 0,
        u: param { index: 0 },
    };

    if n.node_type() == NCMD && !n.ncmd().args.is_empty() {
        let text = n.ncmd().args[0].narg().text.as_ptr();
        if crate::parser::goodname(text) != 0 {
            find_command(text, &mut entry, 0, crate::var::pathval())?;
        }
    }
    Ok(())
}

/*
 * Builtin commands.  Builtin commands whose functions are closely
 * tied to evaluation are implemented here.
 */

/*
 * No command given.
 */

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

/*
 * The return command.
 */

// [spec:dash:def:eval.eprintlist-fn]
// [spec:dash:sem:eval.eprintlist-fn]
unsafe fn eprintlist(out: *mut Output, list: &[strlist], sep: c_int) -> c_int {
    let mut sep: c_int = sep;

    for sp in list {
        let mut record = Vec::new();
        if sep != 0 {
            record.push(b' ');
        }
        record.extend_from_slice(CStr::from_ptr(sp.textp()).to_bytes());
        sep |= 1;
        let _ = (&mut *out).write_all(&record);
    }

    sep
}
