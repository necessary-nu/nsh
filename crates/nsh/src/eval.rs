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

use crate::context::Shell;
use crate::error::Error;
use bstr::{BStr, BString};
use core::ptr::{addr_of, addr_of_mut, null, null_mut};
use libc::{c_char, c_int};
use std::ffi::{CStr, CString};
use std::io::Write as _;

use crate::builtins::{BUILTIN_ASSIGN, BUILTIN_REGULAR, BUILTIN_SPECIAL, Builtin, builtincmd};
use crate::error::{FORCEINTON, INTOFF, INTON};
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

/// Where the evaluator is: what it is skipping, how deep it is, and the
/// two buffers it must not re-enter.
///
/// These are independent scalars rather than one structure, which is why
/// the fields are `pub(crate)` where `AliasTable` and `ShellOptions`
/// keep theirs private: there is no container invariant for a method to
/// protect, and twelve one-line accessors would be noise.
///
/// **One pairing is real and is not enforced here.** `skipcount` only
/// means anything while `evalskip` is `SKIPBREAK` or `SKIPCONT` — it is
/// the number of loop levels still to unwind. Anything that sets one
/// must set the other, as `break`/`continue` do. Making that a single
/// `skip(kind, count)` setter would be an improvement and is *not* this
/// commit, which moves state and changes no behaviour.
pub struct EvalState {
    /// set if we are skipping commands
    pub(crate) evalskip: c_int,
    /// number of levels to skip — see the note above
    pub(crate) skipcount: c_int,
    /// current loop nesting level (MKINIT)
    pub(crate) loopnest: c_int,
    /// starting line number of current function, or 0
    ///
    /// Private: `eval.rs` is the only module that names it.
    funcline: c_int,
    /// Prevent PS4 nesting. (MKINIT)
    pub(crate) inps4: c_int,
    /// MKINIT `int tpip[2] = { -1 }`
    pub(crate) tpip: [c_int; 2],
    /// exit status of backquoted command
    pub(crate) back_exitstatus: c_int,
    /// exit status of the last command outside traps, or -1 when no trap
    /// is running. `dotrap` seeds it and `exitreset` restores from it.
    pub(crate) savestatus: c_int,
}

impl EvalState {
    /// What the eight statics were declared with.
    pub(crate) const fn new() -> Self {
        EvalState {
            evalskip: 0,
            skipcount: 0,
            loopnest: 0,
            funcline: 0,
            inps4: 0,
            tpip: [-1, 0],
            back_exitstatus: 0,
            savestatus: -1,
        }
    }
}

/// The name the running builtin was invoked by, for the error prefix.
///
/// dash points this at `argv[0]` and relies on the word outliving the
/// call. Owning the bytes states that lifetime instead of assuming it,
/// which is what lets `dotcmd` stop keeping its resolved path alive in a
/// static of its own.
///
/// **Still a `static mut`, and deliberately not in `EvalState`.**
/// `docs/api-design.md` §5 groups it here and it does belong to the
/// shell, but the only thing that reads it is `error.rs`'s `sh_warnx`,
/// which builds the diagnostic prefix — so moving it threads the whole
/// diagnostic spine (`sh_warnx` -> `report` -> `sh_error_value`, which
/// the parser and expansion call from everywhere). That is its own
/// slice and a larger one than this. See the node log.
pub static mut commandname: Option<BString> = None;
/* int exitstatus;      exit status of last command      -> Shell::status
 * int back_exitstatus; exit status of backquoted command -> EvalState
 * int savestatus;      status of last command outside traps -> EvalState */

// ---------------------------------------------------------------------
// control flow, which is not error
// ---------------------------------------------------------------------

/// What an evaluation hands back when it did not fail.
///
/// `[dec:nsh:errors-are-values]` and `docs/api-design.md` §3.1 divide
/// `error.rs`'s four exception codes three ways: `EXERROR` is the only one
/// that is an error and it is `Err(Error)`; `EXINT` is the interrupt; and
/// `EXEND` and `EXEXIT` are *control flow*, which the decision requires to
/// sit in the `Ok` position rather than the `Err` one. This is that
/// position.
///
/// **What the audit for this commit found, which `docs/api-design.md`
/// §10.2 asked for before `Flow` was written.** `error::exception` is read
/// in exactly three places in the crate: `evalcommand`'s test for a
/// built-in's `EXERROR` (`eval.rs`), `main`'s handler (`shellmain.rs`),
/// and `init::exitreset` (`init.rs:73`). Only the last one tells `EXEND`
/// from `EXEXIT`, and all it does with the difference is decide whether to
/// restore `savestatus` into `exitstatus`. `main`'s handler tests the two
/// together and does the same thing for both. So the two codes differ in
/// exactly one place and in exactly one bit, which is [`Flow::Exit`]'s
/// `by_exitcmd` — and §3.5's "if the conversion finds a second difference,
/// `Exit` grows a field" does not apply, because there is no second
/// difference.
///
/// `evalskip`'s `break` / `continue` / `return` are **not** here. §3.5
/// proposes collapsing them into this type as well, and they should be;
/// but they never travelled by longjmp — they are a global the evaluation
/// loops already poll — so converting them is idiomatisation riding on
/// this node rather than part of replacing the exception mechanism.
/// `docs/idiomatization.md` §2.2 is the rule that says not to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "an ignored Flow is an `exit` the shell does not perform"]
pub enum Flow {
    /// Evaluation finished. The value is the status, exactly what these
    /// functions returned before there was anything else to say.
    Done(c_int),
    /// The shell is exiting: the C's `EXEND` and `EXEXIT`.
    ///
    /// `by_exitcmd` is `EXEXIT` — `exit` ran and left what it was asked
    /// for in [`savestatus`], which `init::exitreset` restores. `false` is
    /// `EXEND`: `set -e`, an `EV_EXIT` evaluation, or an `exec` that could
    /// not happen, none of which name a status.
    Exit { by_exitcmd: bool },
}

impl Flow {
    /// The `EXEND` exit: the shell is ending without a status having been
    /// named.
    pub const END: Flow = Flow::Exit { by_exitcmd: false };
    /// The `EXEXIT` exit: `exit` ran.
    pub const EXIT: Flow = Flow::Exit { by_exitcmd: true };
}

/// `?` for [`Flow`]: take the status, or return the exit to the caller.
///
/// Every `evaltree(n, f)?` in the C was a call that could not come back at
/// all once the shell had decided to exit, because the decision travelled
/// by `longjmp` straight past this frame. `flow!(evaltree(n, f))` is that
/// same "does not come back" written as a return, and the `?` inside it
/// keeps propagating the diagnostics.
///
/// It is a macro rather than a method because the `return` has to happen
/// in the *caller's* frame, which is the whole point.
macro_rules! flow {
    ($e:expr) => {
        match $e? {
            $crate::eval::Flow::Done(status) => status,
            exit @ $crate::eval::Flow::Exit { .. } => return Ok(exit),
        }
    };
}
pub(crate) use flow;

/* src/options.h: `#define nflag optlist[5]` and friends. */
#[inline]
unsafe fn nflag(sh: &crate::context::Shell) -> c_int {
    sh.options.flag(crate::options::nflag) as c_int
}
#[inline]
unsafe fn eflag(sh: &crate::context::Shell) -> c_int {
    sh.options.flag(crate::options::eflag) as c_int
}
#[inline]
unsafe fn xflag(sh: &crate::context::Shell) -> c_int {
    sh.options.flag(crate::options::xflag) as c_int
}
#[inline]
unsafe fn iflag(sh: &crate::context::Shell) -> c_int {
    sh.options.flag(crate::options::iflag) as c_int
}

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
pub unsafe fn evalstring(sh: &mut Shell, s: *mut c_char, flags: c_int) -> Result<Flow, Error> {
    let mut status: c_int;
    /* `sstrdup(s)` and the `stunalloc(s)` at the bottom are one thing:
     * `setinputstring` keeps the pointer rather than copying, so the text
     * has to outlive every `popstackmark` the parse below performs — which
     * is why the copy is taken *before* the mark is set and released by
     * hand afterwards.  Owning it says both halves at once, and says them
     * on the unwind path too, where the C's `stunalloc` never runs. */
    let owned: Vec<u8> = CStr::from_ptr(s).to_bytes_with_nul().to_vec();
    let s: *mut c_char = owned.as_ptr() as *mut c_char;

    crate::input::setinputstring(sh, s);
    status = 0;
    loop {
        let n: Option<Node> = match crate::parser::parsecmd(sh, 0)? {
            crate::parser::ParseResult::Eof => break,
            crate::parser::ParseResult::Tree(n) => n,
        };
        {
            let i: c_int;

            /* The C's `longjmp` past this frame skipped the `popfile`
             * below, and so does a `Flow::Exit` returned through it: the
             * input stack is unwound to a mark by whoever catches, not by
             * the frame that was passed through. */
            i = flow!(evaltree(sh, 
                n.as_ref(),
                flags
                    & !(if crate::parser::parser_eof(sh) != 0 {
                        0
                    } else {
                        EV_EXIT
                    }),
            ));
            if n.is_some() {
                status = i;
            }

            if sh.eval.evalskip != 0 {
                break;
            }
        }
        /* `popstackmark(&smark)` — one per parsed command, and one on the
         * way out. */
    }
    crate::input::popfile(sh);
    drop(owned);

    Ok(Flow::Done(status))
}

/*
 * Evaluate a parse tree.  The value is left in the global variable
 * exitstatus.
 */

// [spec:dash:def:eval.evaltree-fn]
// [spec:dash:sem:eval.evaltree-fn]
pub unsafe fn evaltree(sh: &mut Shell, n: Option<&Node>, flags: c_int) -> Result<Flow, Error> {
    let mut checkexit: c_int = 0;
    /* C leaves `evalfn` uninitialised; every path that reaches
     * `calleval` assigns it first. Seeded here only so that Rust's
     * definite-initialisation analysis is trivially satisfied — any of
     * the six is as good, and `evaltree` itself no longer fits the type,
     * because the leaf evaluators all dereference their node. */
    let mut evalfn: unsafe fn(&mut Shell, &Node, c_int) -> Result<Flow, Error> =
        evalcommand;
    let isor: libc::c_uint;
    let mut status: c_int = 0;

    'out_lbl: {
        if nflag(sh) != 0 {
            break 'out_lbl;
        }

        let n: &Node = match n {
            Some(n) => n,
            None => {
                /* TRACE(("evaltree(NULL) called\n")); */
                break 'out_lbl;
            }
        };

        flow!(crate::trap::dotrap(sh));

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
                                sh.vars.lineno = r.linno;
                                if sh.eval.funcline != 0 {
                                    sh.vars.lineno -= sh.eval.funcline - 1;
                                }
                                expredir(sh, &r.redirect)?;
                                crate::redir::pushredir(sh, &r.redirect);
                                /* The C is `status = redirectsafe(..)`,
                                 * whose value is `setjmp(..) * 2`. The
                                 * error is dropped here because dash drops
                                 * it: the diagnostic is already written,
                                 * the body is skipped, and the compound
                                 * command's status is the 2 the failure
                                 * took (docs/api-design.md §3.3). */
                                match crate::redir::redirectsafe(sh, &r.redirect, REDIR_PUSH) {
                                    /* An interrupt is not a redirection
                                     * error and is not swallowed with
                                     * one. */
                                    Err(e) if e.is_interrupt() => return Err(e),
                                    Err(e) => {
                                        debug_assert_eq!(
                                            e.status(),
                                            2,
                                            "a redirection error takes status 2"
                                        );
                                        /* From the value, not hardcoded:
                                         * the assert above already claimed
                                         * the two agree, and taking it
                                         * from the error is what makes the
                                         * claim structural. */
                                        status = e.status();
                                        checkexit = EV_TESTED;
                                    }
                                    Ok(()) => {
                                        status =
                                            flow!(evaltree(sh, r.n.as_deref(), flags & EV_TESTED));
                                    }
                                }
                                if !r.redirect.is_empty() {
                                    crate::redir::popredir(sh, 0);
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
                                status = flow!(evaltree(sh, 
                                    b.ch1.as_deref(),
                                    (flags | (((isor >> 1).wrapping_sub(1)) as c_int)) & EV_TESTED,
                                ));
                                if ((status == 0) as libc::c_uint) == isor || sh.eval.evalskip != 0 {
                                    break 'sw;
                                }
                                nnext = b.ch2.as_deref();
                                break 'evaln;
                            }
                            NIF => {
                                let f = n.nif();
                                status = flow!(evaltree(sh, f.test.as_deref(), EV_TESTED));
                                if sh.eval.evalskip != 0 {
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
                                crate::exec::defun(sh, n);
                                break 'sw;
                            }
                            /* `default:` has no body outside DEBUG, so an
                             * unrecognised node type falls straight through
                             * into `case NNOT:`. No other node type reaches
                             * `evaltree`, so with a tagged union there is
                             * nothing left for the fallthrough to reinterpret. */
                            _ /* default, NNOT */ => {
                                status = flow!(evaltree(sh, n.nnot().com.as_deref(), EV_TESTED));
                                if sh.eval.evalskip == 0 {
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
                status = flow!(evaltree(sh, nnext, flags));
                break 'sw;
            }
            // calleval:
            status = flow!(evalfn(sh, n, flags));
        }

        sh.status = status;
    }
    // out:
    flow!(crate::trap::dotrap(sh));

    'exexit: {
        if eflag(sh) != 0 && (!flags & checkexit) != 0 && status != 0 {
            break 'exexit;
        }

        if (flags & EV_EXIT) != 0 {
            break 'exexit;
        }

        return Ok(Flow::Done(sh.status));
    }
    // exexit:
    /* `exraise(EXEND)`, which is the `set -e` abort and the end of an
     * `EV_EXIT` evaluation. Neither names a status -- `exitstatus` already
     * holds it -- so this is the `by_exitcmd: false` half of `Flow::Exit`,
     * and it is returned rather than jumped with. Note what is *not* here:
     * the C raises after the `popstackmark` that the normal return runs
     * before, and 2.3 warned that a naive rewrite would release the region
     * on a path the C never does. `delete-memalloc` removed both marks, so
     * there is nothing left to place. 8.5 is closed. */
    Ok(Flow::END)
}

// [spec:dash:def:eval.evaltreenr-fn]
// [spec:dash:sem:eval.evaltreenr-fn]
//
// `evaltree` declared `noreturn`. Where the C compiler supports
// `__attribute__((alias))` it is literally the same function; the
// portable fallback — reproduced here — calls `evaltree` and aborts if
// it ever comes back.
pub unsafe fn evaltreenr(sh: &mut Shell, n: Option<&Node>, flags: c_int) -> Result<Flow, Error> {
    /* The C's `noreturn` was true because every caller passes `EV_EXIT`,
     * and `evaltree`'s tail raises `EXEND` unconditionally under that
     * flag. It still cannot come back with a status -- that is what the
     * assertion says -- but "cannot come back" is now a `Flow::Exit`
     * travelling out through the caller rather than a jump past it. Each
     * of the three call sites is in a freshly forked child, whose copy of
     * every frame between here and `main` is its own, so returning
     * through them reaches the same `exit:` the longjmp reached. */
    let flow = evaltree(sh, n, flags)?;
    debug_assert!(
        matches!(flow, Flow::Exit { .. }),
        "evaltreenr's caller passed EV_EXIT, so evaltree cannot finish normally"
    );
    Ok(flow)
}

// [spec:dash:def:eval.skiploop-fn]
// [spec:dash:sem:eval.skiploop-fn]
unsafe fn skiploop(sh: &mut crate::context::Shell) -> c_int {
    let mut skip: c_int = sh.eval.evalskip;

    match skip {
        0 => {}

        SKIPBREAK | SKIPCONT => {
            sh.eval.skipcount -= 1;
            if sh.eval.skipcount <= 0 {
                sh.eval.evalskip = 0;
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
unsafe fn evalloop(sh: &mut Shell, n: &Node, flags: c_int) -> Result<Flow, Error> {
    let mut skip: c_int;
    let mut status: c_int;
    let mut flags: c_int = flags;

    sh.eval.loopnest += 1;
    status = 0;
    flags &= EV_TESTED;
    loop {
        {
            let mut i: c_int;

            i = flow!(evaltree(sh, n.nbinary().ch1.as_deref(), EV_TESTED));
            skip = skiploop(sh);
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
                status = flow!(evaltree(sh, n.nbinary().ch2.as_deref(), flags));
                skip = skiploop(sh);
            }
        }
        if (skip & !SKIPCONT) != 0 {
            break;
        }
    }
    sh.eval.loopnest -= 1;

    Ok(Flow::Done(status))
}

// [spec:dash:def:eval.evalfor-fn]
// [spec:dash:sem:eval.evalfor-fn]
unsafe fn evalfor(sh: &mut Shell, n: &Node, flags: c_int) -> Result<Flow, Error> {
    let mut arglist: arglist = arglist::new();
    let mut status: c_int;
    let mut flags: c_int = flags;

    let f = n.nfor();
    crate::error::errlinno = f.linno;
    sh.vars.lineno = f.linno;
    if sh.eval.funcline != 0 {
        sh.vars.lineno -= sh.eval.funcline - 1;
    }

    for argp in &f.args {
        crate::expand::expandarg(sh, argp, Some(&mut arglist), EXP_FULL | EXP_TILDE)?;
    }

    status = 0;
    sh.eval.loopnest += 1;
    flags &= EV_TESTED;
    for sp in &arglist.list {
        crate::var::setvar(sh, f.var.as_ptr(), sp.textp(), 0)?;
        status = flow!(evaltree(sh, f.body.as_deref(), flags));
        if (skiploop(sh) & !SKIPCONT) != 0 {
            break;
        }
    }
    sh.eval.loopnest -= 1;

    Ok(Flow::Done(status))
}

// [spec:dash:def:eval.evalcase-fn]
// [spec:dash:sem:eval.evalcase-fn]
unsafe fn evalcase(sh: &mut Shell, n: &Node, flags: c_int) -> Result<Flow, Error> {
    let mut arglist: arglist = arglist::new();
    let mut status: c_int = 0;

    let c = n.ncase();
    crate::error::errlinno = c.linno;
    sh.vars.lineno = c.linno;
    if sh.eval.funcline != 0 {
        sh.vars.lineno -= sh.eval.funcline - 1;
    }

    crate::expand::expandarg(sh, 
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
            if sh.eval.evalskip != 0 {
                break;
            }
            for patp in &cp.nclist().pattern {
                if crate::expand::casematch(sh, patp, arglist.list[0].textp())? != 0 {
                    /* Ensure body is non-empty as otherwise
                     * EV_EXIT may prevent us from setting the
                     * exit status.
                     */
                    if sh.eval.evalskip == 0 && cp.nclist().body.is_some() {
                        status = flow!(evaltree(sh, cp.nclist().body.as_deref(), flags));
                    }
                    break 'out_lbl;
                }
            }
        }
    }
    // out:
    Ok(Flow::Done(status))
}

/*
 * Kick off a subshell to evaluate a tree.
 */

// [spec:dash:def:eval.evalsubshell-fn]
// [spec:dash:sem:eval.evalsubshell-fn]
unsafe fn evalsubshell(sh: &mut Shell, n: &Node, flags: c_int) -> Result<Flow, Error> {
    let jp: usize;
    let backgnd: c_int = (n.node_type() == NBACKGND) as c_int;
    let mut status: c_int;
    let mut flags: c_int = flags;

    let r = n.nredir();
    crate::error::errlinno = r.linno;
    sh.vars.lineno = r.linno;
    if sh.eval.funcline != 0 {
        sh.vars.lineno -= sh.eval.funcline - 1;
    }

    expredir(sh, &r.redirect)?;
    INTOFF();
    /* Whether the tail below runs in a child of this process or in this
     * process. The C does not need to know, because its `evaltreenr`
     * leaves by longjmp either way; a return has to know, and this is the
     * difference. */
    let forked: bool;
    'nofork: {
        if backgnd == 0 && (flags & EV_EXIT) != 0 && crate::trap::have_traps() == 0 {
            crate::init::forkreset(sh, None);
            forked = false;
            break 'nofork;
        }
        jp = crate::jobs::makejob(sh, 1);
        if crate::jobs::forkshell(sh, Some(jp), r.n.as_deref(), backgnd)? == 0 {
            flags |= EV_EXIT;
            if backgnd != 0 {
                flags &= !EV_TESTED;
            }
            forked = true;
            break 'nofork;
        }
        /* the parent tail of the C function; the child path below
         * never returns, so it is reached only from here */
        status = 0;
        if backgnd == 0 {
            status = crate::jobs::waitforjob(sh, Some(jp))?;
        }
        INTON();
        return Ok(Flow::Done(status));
    }
    // nofork:
    INTON();
    let outcome = (|| -> Result<Flow, Error> {
        crate::redir::redirect(sh, &r.redirect, 0)?;
        evaltreenr(sh, r.n.as_deref(), flags)
    })();

    if forked {
        /* A child may **not** hand this back. The frames between here and
         * `main` are the parent's, copied by `fork`, and the parent was in
         * the middle of using them: returning through them resumes the
         * parent's work in the child. The case that says so is
         * `aud_exception_paths`'s
         *
         *     trap '( trap "echo inner" EXIT; exit 2 ); echo $?' EXIT
         *
         * where the copied frames include `exitshell`, already past its
         * `trap[0].take()`. Returning the exit re-entered that frame and
         * the child skipped its own EXIT trap: dash prints `inner` then
         * `2`, and the port printed only `2`. The C never had the choice,
         * because a longjmp to `main_handler` lands at `exit:` and calls a
         * *fresh* `exitshell`. That is what this does.
         *
         * The same trap in a different clothing as `shellmain.rs`'s note
         * about `exit:` living inside the loop -- a subshell in an EXIT
         * trap, which the corpus has now caught twice. */
        crate::shellmain::exit_from_child(sh, outcome);
    }
    /* Not forked: `forkreset` pointed `handler` at `main_handler` and this
     * is still the same process, so the frames this returns through are
     * its own and `main`'s handler is the right destination. */
    outcome
}

/*
 * Compute the names of the files in a redirection list.
 */

// [spec:dash:def:eval.expredir-fn]
// [spec:dash:sem:eval.expredir-fn]
unsafe fn expredir(sh: &mut Shell, n: &[Node]) -> Result<(), Error> {
    for redir in n {
        let mut fnl: arglist = arglist::new();
        match redir.node_type() {
            NFROMTO | NFROM | NTO | NCLOBBER | NAPPEND => {
                crate::expand::expandarg(sh, 
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
                            crate::expand::expandarg(sh, v, Some(&mut fnl), EXP_TILDE | EXP_REDIR)?;
                            true
                        }
                    }
                };
                if expand {
                    debug_assert_eq!(fnl.list.len(), 1, "an unsplit expansion is one field");
                    let word = fnl.list[0].textp();
                    crate::parser::fixredir(sh, redir, word, 1)?;
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
unsafe fn evalpipe(sh: &mut Shell, n: &Node, flags: c_int) -> Result<Flow, Error> {
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
    jp = crate::jobs::makejob(sh, pipelen);
    prevfd = -1;
    for (i, cmd) in p.cmdlist.iter().enumerate() {
        let has_next = i + 1 < p.cmdlist.len();
        match prehash(sh, cmd)? {
            Flow::Done(_) => {}
            exit @ Flow::Exit { .. } => return Ok(exit),
        }
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
        if crate::jobs::forkshell(sh, Some(jp), Some(cmd), p.backgnd)? == 0 {
            INTON();
            if pip[1] >= 0 {
                libc::close(pip[0]);
            }
            if prevfd > 0 {
                crate::input::reset_input(sh);
                libc::dup2(prevfd, 0);
                libc::close(prevfd);
            }
            if pip[1] > 1 {
                libc::dup2(pip[1], 1);
                libc::close(pip[1]);
            }
            /* In a forked child, which may not return through the
             * parent's frames; see `evalsubshell`. */
            let outcome = evaltreenr(sh, Some(cmd), flags);
            crate::shellmain::exit_from_child(sh, outcome);
        }
        if prevfd >= 0 {
            libc::close(prevfd);
        }
        prevfd = pip[0];
        libc::close(pip[1]);
    }
    if p.backgnd == 0 {
        status = crate::jobs::waitforjob(sh, Some(jp))?;
        /* TRACE(("evalpipe:  job done exit status %d\n", status)); */
    }
    INTON();

    Ok(Flow::Done(status))
}

/*
 * Execute a command inside back quotes.  If it's a builtin command, we
 * want to save its output in a block obtained from malloc.  Otherwise
 * we fork off a subprocess and get the output of the command via a pipe.
 * Should be called with interrupts off.
 */

// [spec:dash:def:eval.evalbackcmd-fn]
// [spec:dash:sem:eval.evalbackcmd-fn]
pub unsafe fn evalbackcmd(
    sh: &mut Shell,
    n: Option<&Node>,
    result: *mut backcmd,
) -> Result<(), Error> {
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
        sh.eval.tpip[0] = pip[0];
        sh.eval.tpip[1] = pip[1];
        jp = crate::jobs::makejob(sh, 1);
        pid = crate::jobs::forkshell(sh, Some(jp), n, FORK_NOJOB)?;
        sh.eval.tpip[0] = -1;
        if pid == 0 {
            FORCEINTON();
            libc::close(pip[0]);
            if pip[1] != 1 {
                libc::dup2(pip[1], 1);
                libc::close(pip[1]);
            }
            crate::expand::ifsfree();
            /* The one forked child that cannot hand its `Flow` back: it
             * sits under the whole expansion chain, which has no business
             * carrying control flow that only ever exists on the far side
             * of a `fork`. So it performs the ending here instead.
             *
             * That is exact rather than approximate, and the reason is
             * `forkchild`'s `shlvl += 1` (`jobs.rs:877`): `main`'s handler
             * tests `... || shlvl != 0`, so in *any* forked child every
             * outcome -- an exit, a `set -e` abort, a diagnostic -- takes
             * `goto exit` and nothing else. `exit_from_child` is those two
             * lines, and it is why the sibling children in `evalsubshell`
             * and `evalpipe` may return their `Flow` instead: they reach
             * the same place by the longer road. */
            let outcome = evaltreenr(sh, n, EV_EXIT);
            crate::shellmain::exit_from_child(sh, outcome);
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
    sh: &mut Shell,
    arglist: &mut arglist,
    argpp: &mut &'a [Node],
) -> Result<Option<usize>, Error> {
    let lastp: usize = arglist.list.len();

    loop {
        let Some((argp, rest)) = argpp.split_first() else {
            break;
        };
        crate::expand::expandarg(sh, argp, Some(arglist), EXP_FULL | EXP_TILDE)?;
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
    sh: &mut Shell,
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
            match fill_arglist(sh, arglist, argpp)? {
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
            if sp + 1 >= arglist.list.len() && fill_arglist(sh, arglist, argpp)?.is_none() {
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
unsafe fn evalcommand(sh: &mut Shell, cmd: &Node, flags: c_int) -> Result<Flow, Error> {
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
    sh.vars.lineno = c.linno;
    if sh.eval.funcline != 0 {
        sh.vars.lineno -= sh.eval.funcline - 1;
    }

    /* First expand the arguments. */
    /* TRACE(("evalcommand(0x%lx, %d) called\n", (long)cmd, flags)); */
    file_stop = crate::input::cur_mark(sh);
    sh.eval.back_exitstatus = 0;

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
    osp = fill_arglist(sh, &mut arglist, &mut argp)?;
    if osp.is_some() {
        let mut pseudovarflag: c_int = 0;

        loop {
            /* `find_command` can run a `%func` PATH file, which is shell
              * code and can `exit`; the C's longjmp took that past this
              * frame and so does this. */
            /* `pathval` and the call both take the shell, so the read is
             * hoisted out of the argument list rather than nested in it.
             * Arguments evaluate left to right and nothing before it here
             * has an effect, so it is read at the same point as before.
             * Do not re-inline it. */
            let regpath = crate::var::pathval(sh);
            match find_command(
                sh,
                arglist.list[head].textp(),
                &mut cmdentry,
                cmd_flag | DO_REGBLTIN,
                regpath,
            )? {
                Flow::Done(_) => {}
                exit @ Flow::Exit { .. } => return Ok(exit),
            }

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

            cmd_flag = parse_command_args(sh, &mut arglist, &mut argp, &mut path, &mut head)?;
            if cmd_flag == 0 {
                break;
            }
        }

        for a in argp {
            crate::expand::expandarg(sh, 
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

    localvar_stop = crate::var::pushlocalvars(sh, vlocal);

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
    if iflag(sh) != 0 && sh.eval.funcline == 0 && argc > 0 {
        lastarg = *nargv.offset(-1);
    }

    (*crate::output::previous_stderr()).fd = crate::streams::streams().stderr;
    expredir(sh, &c.redirect)?;
    redir_stop = crate::redir::pushredir(sh, &c.redirect);
    /* `status = redirectsafe(..)`, which the C computes as `setjmp(..) *
     * 2`. The value is kept as well as the status, because `bail:` below
     * re-raises it when the command is a special built-in — that is the
     * one place a redirection error is *not* swallowed, and an `int`
     * cannot be re-raised. */
    let mut redir_err: Option<Error> = None;
    match crate::redir::redirectsafe(sh, &c.redirect, REDIR_PUSH | REDIR_SAVEFD2) {
        /* Same as the `NREDIR` arm: an interrupt leaves rather than
         * becoming this command's status. */
        Err(e) if e.is_interrupt() => return Err(e),
        Err(e) => {
            /* From the value; see the `NREDIR` arm. Read before the move
             * into `redir_err`, which is where it is re-raised from. */
            status = e.status();
            redir_err = Some(e);
        }
        Ok(()) => status = 0,
    }

    'out_lbl: {
        'bail: {
            if status != 0 {
                break 'bail;
            }

            for a in &c.assign {
                let spp: usize;

                spp = varlist.list.len();
                crate::expand::expandarg(sh, a, Some(&mut varlist), EXP_VARTILDE)?;
                /* `(*spp)->text` with no null check: EXP_VARTILDE has no
                 * EXP_FULL, so `expandarg` appended exactly one entry. */
                debug_assert_eq!(
                    varlist.list.len(),
                    spp + 1,
                    "an unsplit expansion is one field"
                );

                if vlocal != 0 {
                    crate::var::mklocal(sh, varlist.list[spp].textp(), VEXPORT)?;
                } else {
                    crate::var::setvareq(sh, varlist.list[spp].textp(), vflags)?;
                }
            }

            /* Print the command if xflag is set. */
            if xflag(sh) != 0 && sh.eval.inps4 == 0 {
                let out: *mut Output;
                let mut sep: c_int;

                out = crate::output::previous_stderr();
                sh.eval.inps4 = 1;
                /* Hoisted out of `expandstr`'s argument list; see the
                 * note in `evalcommand`. */
                let ps4 = crate::var::ps4val(sh);
                let prompt = crate::parser::expandstr(sh, ps4)?;
                let _ = (&mut *out).write_all(CStr::from_ptr(prompt).to_bytes());
                sh.eval.inps4 = 0;
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
                    crate::var::pathval(sh)
                };
                match find_command(sh, *argv.offset(0), &mut cmdentry, cmd_flag | DO_ERR, path)? {
                    Flow::Done(_) => {}
                    exit @ Flow::Exit { .. } => return Ok(exit),
                }
            }

            jp = None;

            /* Execute the command. */
            match cmdentry.cmdtype {
                CMDUNKNOWN => {
                    status = 127;
                    break 'bail;
                }

                CMDBUILTIN => {
                    /* `if (evalbltin(..) && !(exception == EXERROR && spclbltin <= 0))
                     *      goto raise;`
                     *
                     * The C asks two questions of one integer and a global:
                     * did the builtin leave by the exception mechanism, and
                     * was it the one kind of exception this frame is allowed
                     * to swallow. Both are answered by the type now. A
                     * diagnostic is `Err`, and swallowing it -- reporting it
                     * and carrying on with its status -- is POSIX's rule that
                     * only a *special* builtin's error ends a non-interactive
                     * shell, which is `docs/api-design.md` 3.3's contract and
                     * the mechanism that decides which errors an embedder
                     * ever sees. Anything else leaves as it arrived. */
                    match evalbltin(sh, cmdentry.u.cmd, &args, flags) {
                        Ok(Flow::Done(_)) => {}
                        Ok(exit @ Flow::Exit { .. }) => return Ok(exit),
                        Err(e) => {
                            /* The C's `!(exception == EXERROR && spclbltin
                             * <= 0)`. An interrupt is not an EXERROR and
                             * was never swallowed here; now that it is a
                             * value, saying so is a test on the value. */
                            if spclbltin > 0 || e.is_interrupt() {
                                return Err(e);
                            }
                            /* Reported already, and `evalbltin`'s epilogue
                             * has run. The status it took travels in the
                             * error, so this frame -- the one that catches
                             * it -- is the one that writes it. It reaches
                             * `status` through `waitforjob(sh, None)`,
                             * which returns `exitstatus` when there is no
                             * job; `bail:` does not touch it on this path
                             * because the C reaches `out:` here. */
                            sh.status = e.status();
                            drop(e);
                        }
                    }
                }

                CMDFUNCTION => {
                    /* `if (evalfun(..)) goto raise;` -- a function body is
                     * not a builtin, so there is nothing to swallow: both an
                     * exit and a diagnostic leave through this frame. */
                    match evalfun(sh, cmdentry.u.func, argc, argv, flags)? {
                        Flow::Done(_) => {}
                        exit @ Flow::Exit { .. } => return Ok(exit),
                    }
                }

                _ => {
                    crate::input::flush_input(sh);

                    /* Fork off a child process if necessary. */
                    if (flags & EV_EXIT) == 0 || crate::trap::have_traps() != 0 {
                        INTOFF();
                        jp = Some(crate::jobs::vforkexec(sh, cmd, argv, path, cmdentry.u.index)?);
                    } else {
                        /* `shellexec` replaces the process image or fails;
                         * failing, it reports and is the C's EXEND. */
                        return shellexec(sh, argv, path, cmdentry.u.index);
                    }
                }
            }

            status = crate::jobs::waitforjob(sh, jp)?;
            FORCEINTON();
            break 'out_lbl;
        }
        // bail:
        sh.status = status;

        /* We have a redirection error. */
        if spclbltin > 0 {
            /* POSIX's "an error in a special built-in exits a
             * non-interactive shell", and the C's textless
             * `exraise(EXERROR)`: no diagnostic is written here because
             * whatever failed wrote its own.
             *
             * `redirectsafe` hands its error back, so the usual way in
             * carries the value. The other way in is `CMDUNKNOWN` with
             * status 127, where there is no value to carry: `find_command`
             * reported "not found" and returned normally, which is
             * `docs/api-design.md` 3.3's "reported and carried on past".
             * `Error::reported` is that case -- a value with no text,
             * because the text has already been written. */
            return Err(match redir_err.take() {
                Some(e) => {
                    debug_assert_eq!(e.status(), status, "a redirection error keeps its status");
                    e
                }
                None => crate::error::Error::reported(status),
            });
        }

        // goto out
    }
    // out:
    if !c.redirect.is_empty() {
        crate::redir::popredir(sh, execcmd);
    }
    crate::redir::unwindredir(sh, redir_stop);
    crate::input::unwindfiles(sh, file_stop);
    crate::var::unwindlocalvars(sh, localvar_stop);
    if !lastarg.is_null() {
        /* dsl: I think this is intended to be used to support
         * '_' in 'vi' command mode during line editing...
         * However I implemented that within libedit itself.
         */
        crate::var::setvar(sh, b"_\0".as_ptr() as *const c_char, lastarg, 0)?;
    }

    Ok(Flow::Done(status))
}

// [spec:dash:def:eval.evalbltin-fn]
// [spec:dash:sem:eval.evalbltin-fn]
unsafe fn evalbltin(
    sh: &mut Shell,
    cmd: *const builtincmd,
    args: &[&BStr],
    flags: c_int,
) -> Result<Flow, Error> {
    let savecmdname: Option<BString>; /* volatile */

    savecmdname = core::mem::take(&mut *addr_of_mut!(commandname));
    /* `commandname = argv[0]`, and NULL for the command that has no word
     * at all -- the assignment-only one `bltin` stands for. */
    commandname = args.first().map(|name| BString::from(<&BStr as AsRef<[u8]>>::as_ref(name)));

    let outcome = (|| -> Result<Flow, Error> {
        let mut status: c_int = match if cmd == crate::builtins::EVALCMD {
            crate::builtins::eval::evalcmd(sh, args, flags)?
        } else {
            let entry = (*cmd).builtin.expect("a builtin with no special entry");
            entry(sh, args)?
        } {
            Flow::Done(status) => status,
            exit @ Flow::Exit { .. } => return Ok(exit),
        };
        /* Every `?` and every `Flow::Exit` above skips the rest of this,
         * exactly as the C's `goto cmddone` skipped it. */
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
        sh.status = status;
        Ok(Flow::Done(status))
    })();

    // cmddone:
    /* The C's epilogue, and the reason it armed a handler at all: an
     * exception raised *beneath* a built-in had to run `freestdout` and
     * restore `commandname` on its way out rather than skip them. It runs
     * on every path here because there is only one way out now. `handler`
     * was the third thing it restored and there is no handler left. */
    crate::output::freestdout();
    commandname = savecmdname;

    outcome
}

// [spec:dash:def:eval.evalfun-fn]
// [spec:dash:sem:eval.evalfun-fn]
unsafe fn evalfun(
    sh: &mut Shell,
    func: *const funcnode,
    argc: c_int,
    argv: *mut *mut c_char,
    flags: c_int,
) -> Result<Flow, Error> {
    let saveparam: crate::options::shparam; /* volatile */
    let savefuncline: c_int;
    let saveloopnest: c_int;

    /* `saveparam = shellparam` plus the `shellparam.malloc = 0` that the C
     * puts inside the protected region so the epilogue's `freeparam` cannot
     * reach what the copy still points at. */
    saveparam = crate::options::takeparam(sh);
    savefuncline = sh.eval.funcline;
    saveloopnest = sh.eval.loopnest;

    INTOFF();
    /* `func->count++`: the second reference that keeps the body alive if
     * the function is redefined while it runs. */
    crate::nodes::reffunc(func);
    sh.eval.funcline = (*func).ndefun().linno;
    sh.eval.loopnest = 0;
    /* This `INTON` is *after* `reffunc`, and the epilogue's `freefunc` is
     * what balances it on both paths. docs/errors-are-values.md 2.6
     * records that a conversion reordering this prologue turns the
     * balance into a use-after-free that only shows when a function
     * redefines itself while running. Nothing here is reordered. */
    INTON();
    crate::options::borrowparam(sh, argv.add(1), argc - 1);

    let outcome = evaltree(sh, (*func).ndefun().body.as_deref(), flags & EV_TESTED);

    // funcdone:
    INTOFF();
    sh.eval.loopnest = saveloopnest;
    sh.eval.funcline = savefuncline;
    crate::nodes::freefunc(func);
    crate::options::restoreparam(sh, saveparam);
    INTON();
    sh.eval.evalskip &= !(SKIPFUNC | SKIPFUNCDEF);

    outcome
}

/*
 * Search for a command.  This is called before we fork so that the
 * location of the command will be available in the parent as well as
 * the child.  The check for "goodname" is an overly conservative
 * check that the name will not be subject to expansion.
 */

// [spec:dash:def:eval.prehash-fn]
// [spec:dash:sem:eval.prehash-fn]
unsafe fn prehash(sh: &mut Shell, n: &Node) -> Result<Flow, Error> {
    let mut entry: cmdentry = cmdentry {
        cmdtype: 0,
        u: param { index: 0 },
    };

    if n.node_type() == NCMD && !n.ncmd().args.is_empty() {
        let text = n.ncmd().args[0].narg().text.as_ptr();
        if crate::parser::goodname(text) != 0 {
            /* Hoisted out of the argument list; see the note in
             * `evalcommand`. */
            let path = crate::var::pathval(sh);
            return find_command(sh, text, &mut entry, 0, path);
        }
    }
    Ok(Flow::Done(0))
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

#[cfg(test)]
mod tests {
    //! `Flow`, and the propagation operator that carries it.
    //!
    //! What these pin is not the shape of the enum but the two claims the
    //! conversion rests on: that `flow!` *returns* rather than falling
    //! through, which is what makes it the literal stand-in for a longjmp
    //! past this frame; and that `by_exitcmd` is the single bit telling
    //! the C's EXEXIT from its EXEND. The behaviour is pinned end to end
    //! in `tests/errors_are_values.rs`.

    use super::*;

    /// `flow!` on a finished evaluation yields the status and carries on.
    // [spec:dash:sem:eval.evaltree-fn/test]
    #[test]
    fn flow_yields_a_status() {
        unsafe fn body(inner: Result<Flow, Error>) -> Result<Flow, Error> {
            let status = flow!(inner);
            Ok(Flow::Done(status + 100))
        }
        let got = unsafe { body(Ok(Flow::Done(7))) };
        assert_eq!(got.unwrap(), Flow::Done(107));
    }

    /// …and on an exit it returns, so nothing after it runs. That is the
    /// whole of what the C got from jumping past the frame, and getting
    /// it wrong would run epilogues the unwind skipped.
    // [spec:dash:sem:eval.evaltree-fn/test]
    #[test]
    fn flow_returns_an_exit() {
        unsafe fn body(inner: Result<Flow, Error>) -> Result<Flow, Error> {
            let _status = flow!(inner);
            panic!("flow! must not fall through on an exit");
        }
        let got = unsafe { body(Ok(Flow::EXIT)) };
        assert_eq!(got.unwrap(), Flow::Exit { by_exitcmd: true });
    }

    /// A diagnostic still propagates through it, because the `?` is
    /// inside: `flow!` adds an arm, it does not replace one.
    // [spec:dash:sem:eval.evaltree-fn/test]
    #[test]
    fn flow_still_propagates_an_error() {
        unsafe fn body(inner: Result<Flow, Error>) -> Result<Flow, Error> {
            let _status = flow!(inner);
            panic!("flow! must not fall through on an error");
        }
        let e = Error::Other {
            line: 3,
            status: 2,
            message: bstr::BString::from(&b"nope"[..]),
        };
        let got = unsafe { body(Err(e)) };
        assert_eq!(got.unwrap_err().message(), "nope");
    }

    /// The two named exits differ in exactly the bit `init::exitreset`
    /// reads, and in nothing else — which is the audit
    /// `docs/api-design.md` §10.2 asked for, asserted rather than
    /// described.
    // [spec:dash:sem:init.exitreset-fn/test]
    #[test]
    fn two_exits_differ_in_one_bit() {
        assert_eq!(Flow::EXIT, Flow::Exit { by_exitcmd: true });
        assert_eq!(Flow::END, Flow::Exit { by_exitcmd: false });
        assert_ne!(Flow::EXIT, Flow::END);
    }

    /// `exitreset` restores `savestatus` for what was EXEXIT and not for
    /// what was EXEND. This is the one place in the crate where the two
    /// C codes were ever told apart.
    // [spec:dash:sem:init.exitreset-fn/test]
    #[test]
    fn exitreset_takes_savestatus_for_an_exit() {
        let _guard = crate::testutil::lock();
        unsafe {
            /* A shell of this test's own, and nothing to save or
             * restore around it any more. Both statuses are fields now,
             * so the process has none to borrow and put back -- the last
             * of the save/restore this test was written around is gone
             * with them. */
            let mut owned = crate::context::Shell::new();
            let sh = &mut owned;

            sh.status = 1;
            sh.eval.savestatus = 9;
            crate::init::exitreset(sh, true);
            assert_eq!(sh.status, 9, "`exit 9` names the status the shell leaves with");
            assert_eq!(sh.eval.savestatus, -1, "and it is consumed");

            sh.status = 1;
            sh.eval.savestatus = 9;
            crate::init::exitreset(sh, false);
            assert_eq!(sh.status, 1, "a `set -e` abort names no status");
        }
    }
}
