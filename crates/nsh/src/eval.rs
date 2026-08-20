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
use crate::context::Shell;
use crate::error::Error;
use crate::fd::LogicalDescriptor;
use crate::status::ExitStatus;
use bstr::{BStr, BString, ByteSlice};
use core::ffi::c_int;
use nsh_platform::Descriptor;
use std::io::Write as _;

use crate::builtins::{BuiltinHandler, BuiltinId, BuiltinSpec};
use crate::exec::{Command, DO_ERR, DO_NOFUNC, DO_REGBLTIN, find_command, shellexec};
use crate::expand::{ExpansionMode, arglist, strlist};
use crate::jobs::{FORK_NOJOB, JobId};
// [spec:nsh:def:idiom.job-control-model]
use crate::nodes::{
    BinaryCommand, CaseCommand, CompoundCommand, DescriptorTarget, ForCommand, FunctionDefinition,
    Node, Pipeline, Redirection, SimpleCommand,
};
use crate::options::ShellOption;
use crate::output::Dest;
// [spec:nsh:def:idiom.shell-options]
use crate::redir::{ExpandedRedirection, RedirectionMode};
use crate::var::VariableAttributes;

// ---------------------------------------------------------------------
// src/eval.h
// ---------------------------------------------------------------------

/// How the caller intends an evaluation to return.
///
/// `exit` is used only by child/process-terminus evaluations. `tested` says
/// that the command status is consumed by surrounding shell syntax, so `-e`
/// must not act on it. The two facts are independent, and callers narrow a
/// context explicitly instead of masking unrelated integer bits.
// [spec:nsh:req:idiom.operation-modes]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct EvalContext {
    exit: bool,
    tested: bool,
}

impl EvalContext {
    pub(crate) const DEFAULT: Self = Self {
        exit: false,
        tested: false,
    };
    pub(crate) const EXITING: Self = Self {
        exit: true,
        tested: false,
    };
    pub(crate) const TESTED: Self = Self {
        exit: false,
        tested: true,
    };

    const fn exits(self) -> bool {
        self.exit
    }

    const fn is_tested(self) -> bool {
        self.tested
    }

    const fn with_exit(self) -> Self {
        Self { exit: true, ..self }
    }

    const fn without_exit(self) -> Self {
        Self {
            exit: false,
            ..self
        }
    }

    const fn without_tested(self) -> Self {
        Self {
            tested: false,
            ..self
        }
    }

    pub(crate) const fn tested_only(self) -> Self {
        Self {
            exit: false,
            tested: self.tested,
        }
    }
}

// [spec:dash:def:eval.backcmd]
pub struct backcmd {
    /* result of evalbackcmd */
    pub fd: Option<Descriptor>, /* descriptor to read from */
    pub jp: Option<JobId>,      /* index of the job structure for command */
}

// ---------------------------------------------------------------------
// module globals
// ---------------------------------------------------------------------

/// Where the evaluator is: how deep it is and the two buffers it must not
/// re-enter.
///
/// These are independent scalars rather than one structure, which is why
/// the fields are `pub(crate)` where `AliasTable` and `ShellOptions`
/// keep theirs private: there is no container invariant for a method to
/// protect, and twelve one-line accessors would be noise.
///
pub struct EvalState {
    /// Current loop nesting level.
    pub(crate) loopnest: c_int,
    /// starting line number of current function, or 0
    ///
    /// Private: `eval.rs` is the only module that names it.
    funcline: c_int,
    /// Prevent PS4 nesting.
    pub(crate) inps4: c_int,
    /// exit status of backquoted command
    pub(crate) back_exitstatus: ExitStatus,
    /// Number of signal trap actions currently being evaluated.
    ///
    /// A special-builtin failure ordinarily terminates a non-interactive
    /// shell. A signal action is a catch boundary instead: the action's
    /// command status is discarded and the interrupted status is restored.
    /// Keeping that mode on the shell makes it survive functions and `eval`
    /// without adding a process-global trap flag.
    pub(crate) signal_trap_depth: usize,
    /// Status used by operand-less `exit` when that command directly ends
    /// the currently executing trap action.
    ///
    /// A fork clears this: `exit` in a subshell ends the subshell, not the
    /// parent's trap action. Signal actions temporarily replace the value
    /// with the status they interrupted, then restore the outer action.
    pub(crate) trap_default_exit_status: Option<ExitStatus>,
    /// The line a diagnostic reports — the `17` of `sh: 17: cd: ...`.
    ///
    /// `error.rs`'s `errlinno`. Six sites write it, five of them here
    /// from the node being evaluated and one in `parser.rs` from the
    /// line being parsed, and the only reader is the diagnostic prefix.
    /// It has no row of its own in `docs/api-design.md` §5; it lands
    /// beside `commandname` because they are written by the same frames
    /// and read by the same one function.
    pub(crate) errlinno: c_int,
    /// The name the running builtin was invoked by, for the error prefix.
    ///
    /// dash points this at `argv[0]` and relies on the word outliving the
    /// call. Owning the bytes states that lifetime instead of assuming
    /// it, which is what lets `dotcmd` stop keeping its resolved path
    /// alive in a static of its own.
    ///
    /// `docs/api-design.md` §5 groups it here, and `move-state`'s third
    /// correction confirmed that placement against §5.2's stale claim
    /// that it is a transient alias: it describes the C's `char *`, and
    /// the port owns the bytes.
    pub(crate) commandname: Option<BString>,
}

impl EvalState {
    /// What the eight statics were declared with.
    pub(crate) const fn new() -> Self {
        EvalState {
            loopnest: 0,
            funcline: 0,
            inps4: 0,
            back_exitstatus: ExitStatus::SUCCESS,
            signal_trap_depth: 0,
            trap_default_exit_status: None,
            errlinno: 0,
            commandname: None,
        }
    }
}

/* int exitstatus;      exit status of last command      -> Shell::status
 * int back_exitstatus; exit status of backquoted command -> EvalState
 * int savestatus;      replaced by local trap status and `Flow::Exit::status` */

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
/// `EXEND` carries no newly selected status: the command status already in
/// [`Shell::status`](crate::context::Shell::status) is the one to use.
/// `EXEXIT` carries the status selected by `exit`, including the then-current
/// status when no operand was supplied. Keeping that status in this value
/// avoids pairing control flow with a second ambient field and lets nested
/// traps carry independent exit decisions.
///
/// Loop and function control travel in the same value. This makes every
/// propagation and catch boundary explicit and leaves no ambient skip code
/// for unrelated evaluator frames to poll.
// [spec:nsh:req:idiom.evaluator-control-flow]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "an ignored Flow drops a shell control transfer"]
pub enum Flow {
    /// Evaluation finished. The value is the status, exactly what these
    /// functions returned before there was anything else to say.
    Done(ExitStatus),
    /// The shell is exiting: the C's `EXEND` and `EXEXIT`.
    ///
    /// `status` is `Some` when the `exit` builtin selected a status and
    /// `None` for `EXEND`: `set -e`, an `EV_EXIT` evaluation, or an `exec`
    /// that could not happen. The latter already left its status on the
    /// shell.
    Exit { status: Option<ExitStatus> },
    /// Leave `levels` lexically enclosing loops.
    Break { levels: usize, status: ExitStatus },
    /// Resume at the top of the `levels`th lexically enclosing loop.
    Continue { levels: usize, status: ExitStatus },
    /// Leave the nearest function or sourced-command boundary.
    Return { status: ExitStatus, explicit: bool },
}

impl Flow {
    /// The `EXEND` exit: the shell is ending without a status having been
    /// named.
    pub const END: Flow = Flow::Exit { status: None };

    /// The `EXEXIT` exit: `exit` ran and selected `status`.
    pub fn exit(status: impl Into<ExitStatus>) -> Flow {
        Flow::Exit {
            status: Some(status.into()),
        }
    }

    pub(crate) const fn status(self) -> Option<ExitStatus> {
        match self {
            Flow::Done(status)
            | Flow::Break { status, .. }
            | Flow::Continue { status, .. }
            | Flow::Return { status, .. } => Some(status),
            Flow::Exit { status } => status,
        }
    }

    pub(crate) const fn with_status(self, status: ExitStatus) -> Self {
        match self {
            Flow::Done(_) => Flow::Done(status),
            Flow::Break { levels, .. } => Flow::Break { levels, status },
            Flow::Continue { levels, .. } => Flow::Continue { levels, status },
            Flow::Return { explicit, .. } => Flow::Return { status, explicit },
            Flow::Exit { .. } => self,
        }
    }
}

enum LoopStep {
    Value(ExitStatus),
    Break(ExitStatus),
    Continue(ExitStatus),
    Propagate(Flow),
}

fn catch_one_loop(flow: Flow) -> LoopStep {
    match flow {
        Flow::Done(status) => LoopStep::Value(status),
        Flow::Break { levels: 1, status } => LoopStep::Break(status),
        Flow::Continue { levels: 1, status } => LoopStep::Continue(status),
        Flow::Break { levels, status } => {
            debug_assert!(levels > 1);
            LoopStep::Propagate(Flow::Break {
                levels: levels - 1,
                status,
            })
        }
        Flow::Continue { levels, status } => {
            debug_assert!(levels > 1);
            LoopStep::Propagate(Flow::Continue {
                levels: levels - 1,
                status,
            })
        }
        control => LoopStep::Propagate(control),
    }
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
            control => return Ok(control),
        }
    };
}
pub(crate) use flow;

/*
 * The eval commmand.
 */

/*
 * Execute a command or commands contained in a string.
 */

// [spec:dash:def:eval.evalstring-fn]
// [spec:dash:sem:eval.evalstring-fn]
pub fn evalstring(sh: &mut Shell, s: &BStr, context: EvalContext) -> Result<Flow, Error> {
    /* `sstrdup(s)` and the `stunalloc(s)` at the bottom are one thing:
     * `setinputstring` keeps the pointer rather than copying, so the text
     * has to outlive every `popstackmark` the parse below performs — which
     * is why the copy is taken *before* the mark is set and released by
     * hand afterwards.  Owning it says both halves at once, and says them
     * on the unwind path too, where the C's `stunalloc` never runs. */
    crate::resource::with_resources(sh, |sh, _resources| {
        crate::input::setinputstring(sh, s);
        parse_execute(sh, context)
    })
}

/// Parse and execute until the current input frame runs out.
///
/// The middle of [`evalstring`], and the whole of what
/// [`crate::context::Shell::run`] does with a byte source. It is a
/// function because the two differ only in what pushed the frame and what
/// unwinds it: `evalstring` pushes with `setinputstring` and pops one
/// frame, `run` pushes a [`crate::source::Source`] and unwinds to a mark.
/// Keeping one body is what stops `run` and `eval` drifting apart, which
/// they must not — `docs/api-design.md` §4.1's whole finding is that they
/// are the same primitive.
///
/// The caller pushes the frame and the caller takes it down. A
/// `Flow::Exit` returned through here skips both, which is deliberate and
/// is what the C's `longjmp` past this frame did: the input stack is
/// unwound to a mark by whoever catches, not by the frame that was passed
/// through.
// [spec:posix:req:token.incremental-execution]
// [spec:nsh:req:idiom.lexer-tokens]
pub(crate) fn parse_execute(sh: &mut Shell, context: EvalContext) -> Result<Flow, Error> {
    let mut status = ExitStatus::SUCCESS;
    loop {
        let n: Option<Node> = match crate::parser::parsecmd(sh, 0)? {
            crate::parser::ParseResult::Eof => break,
            crate::parser::ParseResult::Tree(n) => n,
        };
        {
            let i: ExitStatus;

            let command_context = if crate::parser::parser_eof(sh) {
                context
            } else {
                context.without_exit()
            };
            i = flow!(eval_top_level(sh, n.as_ref(), command_context));
            if n.is_some() {
                status = i;
            }
        }
        /* `popstackmark(&smark)` — one per parsed command, and one on the
         * way out. */
    }
    Ok(Flow::Done((status).into()))
}

/// Evaluate one parsed top-level command, retaining the rest of an
/// interactive command list after a parameter-expansion failure.
///
/// The ordinary evaluator returns the error because a non-interactive shell
/// must terminate. An interactive root instead abandons the affected command,
/// restores its temporary state, and resumes at the next `;` command (or the
/// next parsed input record).
// [spec:nsh:req:compat.smoosh.error-contracts]
pub(crate) fn eval_top_level(
    sh: &mut Shell,
    n: Option<&Node>,
    context: EvalContext,
) -> Result<Flow, Error> {
    if !sh.options.enabled(ShellOption::Interactive) || sh.shell_level != 0 {
        return evaltree(sh, n, context);
    }
    eval_interactive_sequence(sh, n, context)
}

fn redirection_only_status(
    status: ExitStatus,
    redirection_error: Option<&Error>,
    has_command: bool,
) -> ExitStatus {
    if redirection_error.is_some() && !has_command {
        ExitStatus::FAILURE
    } else {
        status
    }
}

fn builtin_error_is_fatal(sh: &Shell, special_builtin: bool, error: &Error) -> bool {
    error.is_interrupt() || (special_builtin && sh.eval.signal_trap_depth == 0)
}

fn capture_local_control(flow: Flow, slot: &mut Option<Flow>) -> Result<(), Flow> {
    match flow {
        Flow::Done(_) => Ok(()),
        exit @ Flow::Exit { .. } => Err(exit),
        control => {
            *slot = Some(control);
            Ok(())
        }
    }
}

fn eval_interactive_sequence(
    sh: &mut Shell,
    n: Option<&Node>,
    context: EvalContext,
) -> Result<Flow, Error> {
    if let Some(Node::Sequence(sequence)) = n {
        match eval_interactive_sequence(sh, Some(sequence.left.as_ref()), context.tested_only())? {
            Flow::Done(_) => {}
            control => return Ok(control),
        }
        return eval_interactive_sequence(sh, Some(sequence.right.as_ref()), context);
    }

    let outcome = crate::resource::with_resources(sh, |sh, _resources| evaltree(sh, n, context));
    match outcome {
        Err(error) if error.is_expansion() => {
            let status = error.status();
            sh.status = status;
            drop(error);
            sh.clear_evaluation_resources();
            sh.unwind_local_variables();
            crate::error::clear_interrupt_deferral(&mut sh.interrupt_deferral);
            Ok(Flow::Done((status).into()))
        }
        outcome => outcome,
    }
}

/*
 * Evaluate a parse tree.  The value is left in the global variable
 * exitstatus.
 */

// [spec:dash:def:eval.evaltree-fn]
// [spec:dash:sem:eval.evaltree-fn]
// [spec:posix:def:exit.command-status]
// [spec:posix:req:cmd.default-exit-status]
// [spec:posix:req:cmd.sequential-execution]
// [spec:posix:req:cmd.sequential-exit-status]
// [spec:posix:req:cmd.sequential-foreground-job]
// [spec:posix:req:cmd.and-list-execution]
// [spec:posix:req:cmd.and-list-exit-status]
// [spec:posix:req:cmd.or-list-execution]
// [spec:posix:req:cmd.or-list-exit-status]
// [spec:posix:req:cmd.compound-list-exit-status]
// [spec:posix:req:cmd.compound-redirection-scope]
// [spec:posix:sem:cmd.group-brace-current-environment]
// [spec:posix:req:cmd.if-execution]
// [spec:posix:req:cmd.if-exit-status]
pub fn evaltree(sh: &mut Shell, n: Option<&Node>, context: EvalContext) -> Result<Flow, Error> {
    let mut check_exit = false;
    let mut status = ExitStatus::SUCCESS;

    if !sh.options.enabled(ShellOption::NoExec)
        && let Some(node) = n
    {
        flow!(crate::trap::dotrap(sh));
        sh.displayhist = 1;
        // [spec:nsh:req:idiom.structural-ast]
        status = match node {
            Node::Redirect(redirection) => {
                sh.eval.errlinno = redirection.line;
                sh.vars.lineno = redirection.line;
                if sh.eval.funcline != 0 {
                    sh.vars.lineno -= sh.eval.funcline - 1;
                }
                let expanded_redirections = expredir(sh, &redirection.redirections)?;
                let outcome = crate::resource::with_resources(sh, |sh, resources| match resources
                    .apply_redirections(sh, &expanded_redirections)
                {
                    Err(error) if error.is_interrupt() || error.is_expansion() => Err(error),
                    Err(error) => {
                        drop(error);
                        check_exit = true;
                        Ok(Flow::Done(ExitStatus::FAILURE))
                    }
                    Ok(()) => evaltree(
                        sh,
                        Some(redirection.command.as_ref()),
                        context.tested_only(),
                    ),
                });
                match outcome? {
                    Flow::Done(status) => status,
                    control => return Ok(control),
                }
            }
            Node::Command(command) => {
                check_exit = true;
                flow!(evalcommand(sh, command, context))
            }
            Node::For(command) => flow!(evalfor(sh, command, context)),
            Node::While(command) => flow!(evalloop(sh, command, false, context)),
            Node::Until(command) => flow!(evalloop(sh, command, true, context)),
            Node::Subshell(command) => {
                check_exit = true;
                flow!(evalsubshell(sh, command, false, context))
            }
            Node::Background(command) => {
                check_exit = true;
                flow!(evalsubshell(sh, command, true, context))
            }
            Node::Pipeline(pipeline) => {
                check_exit = true;
                flow!(evalpipe(sh, pipeline, context))
            }
            Node::Case(command) => flow!(evalcase(sh, command, context)),
            Node::And(command) => {
                let left = flow!(evaltree(
                    sh,
                    Some(command.left.as_ref()),
                    EvalContext::TESTED
                ));
                if !left.success() {
                    left
                } else {
                    flow!(evaltree(sh, Some(command.right.as_ref()), context))
                }
            }
            Node::Or(command) => {
                let left = flow!(evaltree(
                    sh,
                    Some(command.left.as_ref()),
                    EvalContext::TESTED
                ));
                if left.success() {
                    left
                } else {
                    flow!(evaltree(sh, Some(command.right.as_ref()), context))
                }
            }
            Node::Sequence(command) => {
                let _ = flow!(evaltree(
                    sh,
                    Some(command.left.as_ref()),
                    context.tested_only(),
                ));
                flow!(evaltree(sh, Some(command.right.as_ref()), context))
            }
            Node::If(command) => {
                let condition = flow!(evaltree(
                    sh,
                    Some(command.condition.as_ref()),
                    EvalContext::TESTED,
                ));
                if condition.success() {
                    flow!(evaltree(sh, Some(command.then_branch.as_ref()), context))
                } else if command.else_branch.is_some() {
                    flow!(evaltree(sh, command.else_branch.as_deref(), context))
                } else {
                    ExitStatus::SUCCESS
                }
            }
            Node::Function(definition) => {
                if sh.options.enabled(ShellOption::HashAll) {
                    let _ = flow!(prehash_tree(sh, Some(definition.body.as_ref())));
                }
                crate::exec::defun(&mut sh.interrupt_deferral, &mut sh.commands, definition);
                ExitStatus::SUCCESS
            }
            Node::Bash(_) => {
                return Err(sh
                    .diagnostics()
                    .sh_error_value(b"Bash syntax is parsed but not executable yet"));
            }
            Node::Not(command) => {
                let status = flow!(evaltree(
                    sh,
                    Some(command.command.as_ref()),
                    EvalContext::TESTED,
                ));
                if status.success() {
                    ExitStatus::FAILURE
                } else {
                    ExitStatus::SUCCESS
                }
            }
            Node::Word(_) => {
                return Err(sh
                    .diagnostics()
                    .sh_error_value(b"non-command syntax reached evaluation"));
            }
        };
        sh.status = status;
    }
    flow!(crate::trap::dotrap(sh));

    let abort_for_errexit = sh.options.enabled(ShellOption::Errexit)
        && check_exit
        && !context.is_tested()
        && !status.success();
    if !abort_for_errexit && !context.exits() {
        return Ok(Flow::Done((sh.status).into()));
    }
    Ok(Flow::END)
}

// [spec:dash:def:eval.evaltreenr-fn]
// [spec:dash:sem:eval.evaltreenr-fn]
//
// `evaltree` declared `noreturn`. Where the C compiler supports
// `__attribute__((alias))` it is literally the same function; the
// portable fallback — reproduced here — calls `evaltree` and aborts if
// it ever comes back.
pub fn evaltreenr(sh: &mut Shell, n: Option<&Node>, context: EvalContext) -> Result<Flow, Error> {
    /* The C's `noreturn` was true because every caller passes `EV_EXIT`,
     * and `evaltree`'s tail raises `EXEND` unconditionally under that
     * flag. It still cannot come back with a status -- that is what the
     * assertion says -- but "cannot come back" is now a `Flow::Exit`
     * travelling out through the caller rather than a jump past it. Each
     * of the three call sites is in a freshly forked child, whose copy of
     * every frame between here and `main` is its own, so returning
     * through them reaches the same `exit:` the longjmp reached. */
    let flow = match evaltree(sh, n, context)? {
        exit @ Flow::Exit { .. } => exit,
        control @ (Flow::Break { .. } | Flow::Continue { .. } | Flow::Return { .. }) => {
            sh.status = control
                .status()
                .expect("local control at a process terminus carries a status");
            Flow::END
        }
        done @ Flow::Done(_) => done,
    };
    debug_assert!(
        matches!(flow, Flow::Exit { .. }),
        "evaltreenr's caller passed EV_EXIT, so evaltree cannot finish normally"
    );
    Ok(flow)
}

// [spec:dash:def:eval.evalloop-fn]
// [spec:dash:sem:eval.evalloop-fn]
// [spec:posix:req:cmd.while-execution]
// [spec:posix:req:cmd.while-exit-status]
// [spec:posix:req:cmd.until-execution]
// [spec:posix:req:cmd.until-exit-status]
fn evalloop(
    sh: &mut Shell,
    command: &BinaryCommand,
    until: bool,
    context: EvalContext,
) -> Result<Flow, Error> {
    let context = context.tested_only();

    sh.eval.loopnest += 1;
    let outcome = (|| {
        let mut status = ExitStatus::SUCCESS;
        loop {
            let mut condition = match catch_one_loop(evaltree(
                sh,
                Some(command.left.as_ref()),
                EvalContext::TESTED,
            )?) {
                LoopStep::Value(status) => status,
                LoopStep::Break(status) => return Ok(Flow::Done(status)),
                LoopStep::Continue(next_status) => {
                    status = next_status;
                    continue;
                }
                LoopStep::Propagate(control) => return Ok(control),
            };
            if until {
                condition = if condition.success() {
                    ExitStatus::FAILURE
                } else {
                    ExitStatus::SUCCESS
                };
            }
            if !condition.success() {
                return Ok(Flow::Done(status));
            }
            match catch_one_loop(evaltree(sh, Some(command.right.as_ref()), context)?) {
                LoopStep::Value(body_status) => status = body_status,
                LoopStep::Break(break_status) => return Ok(Flow::Done(break_status)),
                LoopStep::Continue(next_status) => status = next_status,
                LoopStep::Propagate(control) => return Ok(control),
            }
        }
    })();
    sh.eval.loopnest -= 1;
    outcome
}

// [spec:dash:def:eval.evalfor-fn]
// [spec:dash:sem:eval.evalfor-fn]
// [spec:posix:req:cmd.for-iteration]
// [spec:posix:req:cmd.for-omitted-in]
// [spec:posix:req:cmd.for-exit-status]
fn evalfor(sh: &mut Shell, command: &ForCommand, context: EvalContext) -> Result<Flow, Error> {
    let mut arglist: arglist = arglist::new();
    let mut status: ExitStatus;
    let context = context.tested_only();

    sh.eval.errlinno = command.line;
    sh.vars.lineno = command.line;
    if sh.eval.funcline != 0 {
        sh.vars.lineno -= sh.eval.funcline - 1;
    }

    for argp in &command.words {
        crate::expand::expandarg(
            sh,
            argp,
            Some(&mut arglist),
            ExpansionMode::SPLIT | ExpansionMode::TILDE,
        )?;
    }

    status = ExitStatus::SUCCESS;
    sh.eval.loopnest += 1;
    for sp in &arglist.list {
        crate::var::set_bytes(
            sh,
            command.variable.as_bstr(),
            Some(crate::mystring::cstr_prefix(&sp.text)),
            VariableAttributes::NONE,
        )?;
        match catch_one_loop(evaltree(sh, Some(command.body.as_ref()), context)?) {
            LoopStep::Value(body_status) => status = body_status,
            LoopStep::Break(break_status) => {
                status = break_status;
                break;
            }
            LoopStep::Continue(next_status) => status = next_status,
            LoopStep::Propagate(control) => {
                sh.eval.loopnest -= 1;
                return Ok(control);
            }
        }
    }
    sh.eval.loopnest -= 1;

    Ok(Flow::Done((status).into()))
}

// [spec:dash:def:eval.evalcase-fn]
// [spec:dash:sem:eval.evalcase-fn]
// [spec:posix:req:cmd.case-selection]
// [spec:posix:req:cmd.case-pattern-expansion]
// [spec:posix:req:cmd.case-multiple-pattern-order-unspecified]
// [spec:posix:req:cmd.case-exit-status]
// [spec:posix:req:cmd.case-clause-terminators]
fn evalcase(sh: &mut Shell, command: &CaseCommand, context: EvalContext) -> Result<Flow, Error> {
    let mut arglist: arglist = arglist::new();
    let mut status = ExitStatus::SUCCESS;
    let mut fallthrough = false;

    sh.eval.errlinno = command.line;
    sh.vars.lineno = command.line;
    if sh.eval.funcline != 0 {
        sh.vars.lineno -= sh.eval.funcline - 1;
    }

    crate::expand::expandarg(
        sh,
        command.word.as_ref(),
        Some(&mut arglist),
        ExpansionMode::TILDE | ExpansionMode::PRESERVE_MULTIBYTE,
    )?;
    /* The C reads `arglist.list->text` with no null check, and is right to:
     * `expandarg` without EXP_FULL takes its single-field arm, which appends
     * exactly one entry whatever the word expands to. */
    debug_assert_eq!(arglist.list.len(), 1, "an unsplit expansion is one field");
    'out_lbl: {
        for clause in &command.clauses {
            let mut selected = fallthrough;
            if !selected {
                for patp in &clause.patterns {
                    if crate::expand::casematch(
                        sh,
                        patp,
                        BStr::new(crate::mystring::cstr_prefix(&arglist.list[0].text)),
                    )? != 0
                    {
                        selected = true;
                        break;
                    }
                }
            }
            if !selected {
                continue;
            }
            /* Ensure body is non-empty as otherwise EV_EXIT may prevent us
             * from setting the exit status. */
            if clause.body.is_some() {
                status = flow!(evaltree(sh, clause.body.as_deref(), context));
            }
            if clause.fallthrough {
                fallthrough = true;
            } else {
                break 'out_lbl;
            }
        }
    }
    // out:
    Ok(Flow::Done((status).into()))
}

/*
 * Kick off a subshell to evaluate a tree.
 */

// [spec:dash:def:eval.evalsubshell-fn]
// [spec:dash:sem:eval.evalsubshell-fn]
// [spec:posix:req:jobctl.list-splitting]
// [spec:posix:def:jobctl.background-job]
// [spec:posix:def:jobctl.foreground-job]
// [spec:posix:req:exit.subshell-error-exit]
// [spec:posix:req:cmd.group-subshell]
// [spec:posix:req:cmd.group-exit-status]
// [spec:posix:req:cmd.async-subshell-background]
// [spec:posix:req:cmd.async-exit-status]
fn evalsubshell(
    sh: &mut Shell,
    command: &CompoundCommand,
    background: bool,
    context: EvalContext,
) -> Result<Flow, Error> {
    let backgnd: c_int = background as c_int;
    let mut status = ExitStatus::SUCCESS;
    let mut context = context;

    sh.eval.errlinno = command.line;
    sh.vars.lineno = command.line;
    if sh.eval.funcline != 0 {
        sh.vars.lineno -= sh.eval.funcline - 1;
    }

    let expanded_redirections = expredir(sh, &command.redirections)?;
    /* Whether the tail below runs in a child of this process or in this
     * process. The structured scope restores the caller's interrupt depth
     * before either tail continues. */
    let forked = crate::error::with_interrupts_deferred(sh, |sh| {
        if backgnd == 0 && context.exits() && crate::trap::have_traps(sh) == 0 {
            sh.prepare_fork_child(None);
            return Ok(Some(false));
        }
        let jp = crate::jobs::makejob(sh, 1);
        if matches!(
            crate::jobs::forkshell(sh, Some(jp), Some(command.command.as_ref()), backgnd)?,
            nsh_platform::ForkResult::Child
        ) {
            context = context.with_exit();
            if backgnd != 0 {
                context = context.without_tested();
            }
            return Ok(Some(true));
        }
        /* the parent tail of the C function; the child path below
         * never returns, so it is reached only from here */
        if backgnd == 0 {
            status = crate::jobs::waitforjob(sh, Some(jp))?;
        }
        Ok::<_, Error>(None)
    })?;
    let Some(forked) = forked else {
        return Ok(Flow::Done((status).into()));
    };
    let outcome = (|| -> Result<Flow, Error> {
        crate::redir::redirect(sh, &expanded_redirections, RedirectionMode::Apply)?;
        evaltreenr(sh, Some(command.command.as_ref()), context)
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
    /* Not forked: this is still the same process, so the frames this returns
     * through are its own. */
    outcome
}

/*
 * Compute the names of the files in a redirection list.
 */

// [spec:dash:def:eval.expredir-fn]
// [spec:dash:sem:eval.expredir-fn]
// [spec:posix:req:redir.word-expansion]
// [spec:posix:req:redir.word-pathname-expansion]
// [spec:posix:req:grammar.redirection-filename]
// [spec:nsh:def:idiom.logical-descriptors]
fn expredir<'a>(
    sh: &mut Shell,
    redirections: &'a [Redirection],
) -> Result<Vec<ExpandedRedirection<'a>>, Error> {
    let mut expanded = Vec::with_capacity(redirections.len());
    for redir in redirections {
        let mut fnl: arglist = arglist::new();
        match redir {
            Redirection::File(redirection) => {
                let target = Node::Word(redirection.target.clone());
                crate::expand::expandarg(
                    sh,
                    &target,
                    Some(&mut fnl),
                    ExpansionMode::TILDE | ExpansionMode::REDIRECTION,
                )?;
                /* `fn.list->text` with no null check: no EXP_FULL means
                 * `expandarg` took its single-field arm. */
                debug_assert_eq!(fnl.list.len(), 1, "an unsplit expansion is one field");
                let mut target = fnl.list.remove(0).text;
                debug_assert_eq!(target.last(), Some(&0), "expanded path is terminated");
                target.pop();
                expanded.push(ExpandedRedirection::File {
                    operator: redirection.operator,
                    descriptor: redirection.descriptor,
                    target,
                });
            }
            Redirection::Descriptor(redirection) => {
                let source = match &redirection.target {
                    DescriptorTarget::Number(number) => Some(*number),
                    DescriptorTarget::Close => None,
                    DescriptorTarget::Word(word) => {
                        let word = Node::Word(word.clone());
                        crate::expand::expandarg(
                            sh,
                            &word,
                            Some(&mut fnl),
                            ExpansionMode::TILDE | ExpansionMode::REDIRECTION,
                        )?;
                        debug_assert_eq!(fnl.list.len(), 1, "an unsplit expansion is one field");
                        descriptor_source(sh, crate::mystring::cstr_prefix(&fnl.list[0].text))?
                    }
                };
                expanded.push(ExpandedRedirection::Descriptor {
                    descriptor: redirection.descriptor,
                    source,
                });
            }
            Redirection::HereDocument(document) => {
                expanded.push(ExpandedRedirection::HereDocument(document));
            }
        }
    }
    Ok(expanded)
}

fn descriptor_source(sh: &mut Shell, text: &BStr) -> Result<Option<LogicalDescriptor>, Error> {
    if text.len() == 1 && crate::syntax::is_digit(text[0] as c_int) {
        Ok(Some(
            LogicalDescriptor::from_digit(text[0])
                .expect("an ASCII digit names a logical descriptor"),
        ))
    } else if text == BStr::new(b"-") {
        Ok(None)
    } else {
        let mut message = b"Bad fd number: ".to_vec();
        message.extend_from_slice(text);
        Err(sh.diagnostics().sh_error_value(&message))
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
// [spec:posix:req:cmd.pipeline-connects-stdio]
// [spec:posix:req:cmd.pipeline-assignment-precedes-redirection]
// [spec:posix:req:cmd.pipeline-foreground-wait]
// [spec:posix:req:cmd.pipeline-exit-status]
// [spec:posix:req:cmd.pipeline-pipefail-setting-at-start]
// [spec:nsh:req:idiom.no-raw-fd-core]
fn evalpipe(sh: &mut Shell, pipeline: &Pipeline, context: EvalContext) -> Result<Flow, Error> {
    let context = context.with_exit();

    enum PipelineStart<'a> {
        Parent(ExitStatus),
        Child {
            command: &'a Node,
            input: Option<Descriptor>,
            output: Option<Descriptor>,
        },
        Control(Flow),
    }

    let start = crate::error::with_interrupts_deferred(sh, |sh| {
        let jp = crate::jobs::makejob(sh, pipeline.commands.len() as c_int);
        let mut previous = None;
        for (index, command) in pipeline.commands.iter().enumerate() {
            let has_next = index + 1 < pipeline.commands.len();
            match prehash(sh, command)? {
                Flow::Done(_) => {}
                control => return Ok(PipelineStart::Control(control)),
            }
            let mut pipe = if has_next {
                Some(crate::redir::sh_pipe(sh, false)?.0)
            } else {
                None
            };
            if matches!(
                crate::jobs::forkshell(sh, Some(jp), Some(command), pipeline.background as c_int,)?,
                nsh_platform::ForkResult::Child
            ) {
                let output = pipe.take().map(|pipe| {
                    drop(pipe.read);
                    pipe.write
                });
                return Ok(PipelineStart::Child {
                    command,
                    input: previous.take(),
                    output,
                });
            }
            drop(previous.take());
            if let Some(pipe) = pipe {
                previous = Some(pipe.read);
                drop(pipe.write);
            }
        }
        let status = if pipeline.background {
            ExitStatus::SUCCESS
        } else {
            crate::jobs::waitforjob(sh, Some(jp))?
        };
        Ok::<_, Error>(PipelineStart::Parent(status))
    })?;

    match start {
        PipelineStart::Parent(status) => Ok(Flow::Done(status)),
        PipelineStart::Control(control) => Ok(control),
        PipelineStart::Child {
            command,
            input,
            output,
        } => {
            if let Some(input) = input {
                crate::input::reset_input(sh);
                sh.fds
                    .install_owned(LogicalDescriptor::STDIN, input)
                    .map_err(|error| {
                        crate::redir::descriptor_error(sh, LogicalDescriptor::STDIN, error)
                    })?;
            }
            if let Some(output) = output {
                sh.fds
                    .install_owned(LogicalDescriptor::STDOUT, output)
                    .map_err(|error| {
                        crate::redir::descriptor_error(sh, LogicalDescriptor::STDOUT, error)
                    })?;
            }
            /* In a forked child, which may not return through the
             * parent's frames; see `evalsubshell`. */
            let outcome = evaltreenr(sh, Some(command), context);
            crate::shellmain::exit_from_child(sh, outcome);
        }
    }
}

/*
 * Execute a command inside back quotes.  If it's a builtin command, we
 * want to save its output in a block obtained from malloc.  Otherwise
 * we fork off a subprocess and get the output of the command via a pipe.
 * Should be called with interrupts off.
 */

// [spec:dash:def:eval.evalbackcmd-fn]
// [spec:dash:sem:eval.evalbackcmd-fn]
pub fn evalbackcmd(sh: &mut Shell, n: Option<&Node>, result: &mut backcmd) -> Result<(), Error> {
    let jp: JobId;

    result.fd = None;
    result.jp = None;
    'out_lbl: {
        if n.is_none() {
            break 'out_lbl;
        }

        let pipe = crate::redir::sh_pipe(sh, false)?.0;
        jp = crate::jobs::makejob(sh, 1);
        if matches!(
            crate::jobs::forkshell(sh, Some(jp), n, FORK_NOJOB)?,
            nsh_platform::ForkResult::Child
        ) {
            crate::error::clear_interrupt_deferral(&mut sh.interrupt_deferral);
            drop(pipe.read);
            sh.fds
                .install_owned(LogicalDescriptor::STDOUT, pipe.write)
                .map_err(|error| {
                    crate::redir::descriptor_error(sh, LogicalDescriptor::STDOUT, error)
                })?;
            crate::expand::ifsfree(&mut sh.expand);
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
            let outcome = evaltreenr(sh, n, EvalContext::EXITING);
            crate::shellmain::exit_from_child(sh, outcome);
            /* NOTREACHED */
        }
        drop(pipe.write);
        result.fd = Some(pipe.read);
        result.jp = Some(jp);
    }
    // out:
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
fn fill_arglist<'a>(
    sh: &mut Shell,
    arglist: &mut arglist,
    argpp: &mut &'a [Node],
) -> Result<Option<usize>, Error> {
    let lastp: usize = arglist.list.len();

    loop {
        let Some((argp, rest)) = argpp.split_first() else {
            break;
        };
        crate::expand::expandarg(
            sh,
            argp,
            Some(arglist),
            ExpansionMode::SPLIT | ExpansionMode::TILDE,
        )?;
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
// [spec:posix:req:builtin.command.suppress-function-lookup]
// [spec:posix:req:builtin.command.special-builtin-properties-suppressed]
// [spec:posix:req:builtin.command.equivalent-to-omitting-command]
// [spec:posix:req:builtin.command.declaration-utility]
// [spec:posix:def:builtin.command.operands]
// [spec:posix:req:builtin.command.exit-status-invocation]
// [spec:posix:req:param.ps4]
// `head` is the C's `arglist->list`, which this function reassigns to skip
// the `command [-p]` words it consumed. A `Vec`'s start does not move, so the
// head is an index the caller keeps; see [`crate::expand::arglist`].
fn parse_command_args(
    sh: &mut Shell,
    arglist: &mut arglist,
    argpp: &mut &[Node],
    path: &mut Option<BString>,
    standard_path: &BStr,
    head: &mut usize,
) -> Result<c_int, Error> {
    let mut sp: usize = *head;

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
        let word = crate::mystring::cstr_prefix(&arglist.list[sp].text);
        if word.first() != Some(&b'-') {
            break;
        }
        let options = &word[1..];
        if options.is_empty() {
            break;
        }
        if options == b"-" {
            if sp + 1 >= arglist.list.len() && fill_arglist(sh, arglist, argpp)?.is_none() {
                return Ok(0);
            }
            sp += 1;
            break;
        }
        for &option in options.as_bytes() {
            match option {
                b'p' => {
                    *path = Some(standard_path.to_owned());
                }
                _ => {
                    /* run 'typecmd' for other options */
                    return Ok(0);
                }
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
// [spec:posix:req:builtin.special.error-may-abort-shell]
// [spec:posix:req:builtin.special.preceding-assignments-persist]
// [spec:posix:sem:shell.command-execution]
// [spec:posix:req:grammar.word-expansion-timing]
// [spec:posix:req:grammar.assignment-word-processing]
// [spec:posix:req:shenv.utility-does-not-change-shell-environment]
// [spec:posix:req:cmd.simple-processing-order]
// [spec:posix:req:cmd.simple-command-name-determination]
// [spec:posix:req:cmd.simple-declaration-utility-expansion]
// [spec:posix:req:cmd.simple-argument-expansion]
// [spec:posix:req:cmd.simple-redirections-performed]
// [spec:posix:req:cmd.simple-assignment-expansion]
// [spec:posix:req:cmd.simple-step-order-reversal]
// [spec:posix:req:cmd.declaration-utility-lexical-analysis]
// [spec:posix:req:cmd.assign-no-command-name]
// [spec:posix:req:cmd.assign-exported-to-command]
// [spec:posix:req:cmd.assign-standard-utility-as-function]
// [spec:posix:req:cmd.assign-special-builtin]
// [spec:posix:req:cmd.assign-function]
// [spec:posix:req:cmd.assign-readonly-error]
// [spec:posix:req:cmd.no-name-redirections-subshell]
// [spec:posix:req:cmd.no-name-redirection-failure]
// [spec:posix:req:cmd.no-name-exit-status]
// [spec:nsh:req:idiom.command-dispatch]
fn evalcommand(
    sh: &mut Shell,
    command: &SimpleCommand,
    context: EvalContext,
) -> Result<Flow, Error> {
    crate::resource::with_resources(sh, |sh, resources| {
        evalcommand_in_scope(sh, command, context, resources)
    })
}

fn evalcommand_in_scope(
    sh: &mut Shell,
    command: &SimpleCommand,
    context: EvalContext,
    resources: &mut crate::resource::ResourceScope,
) -> Result<Flow, Error> {
    let mut argp: &[Node];
    let mut arglist: arglist = arglist::new();
    let mut varlist: arglist = arglist::new();
    let mut argc: c_int;
    let osp: Option<usize>;
    /* The C's `arglist.list`, which `parse_command_args` moves past the
     * `command [-p]` words while `osp` keeps the original head for `set -x`. */
    let mut head: usize = 0;
    let mut resolved_command = Command::Builtin(&crate::builtins::EMPTY_BUILTIN);
    let jp: Option<JobId>;
    let lastarg: Option<usize>;
    let mut path: Option<BString> = None;
    let standard_path = crate::var::defpath();
    let mut special_builtin: Option<bool>;
    let mut cmd_flag: c_int;
    let mut exec_builtin: bool;
    let mut status: ExitStatus;
    let mut variable_attributes: VariableAttributes;
    let mut use_local_variables: bool;
    let mut command_control: Option<Flow> = None;

    sh.eval.errlinno = command.line;
    sh.vars.lineno = command.line;
    if sh.eval.funcline != 0 {
        sh.vars.lineno -= sh.eval.funcline - 1;
    }
    if command
        .assignments
        .iter()
        .chain(command.arguments.iter())
        .any(|node| matches!(node, Node::Bash(_)))
    {
        return Err(sh
            .diagnostics()
            .sh_error_value(b"Bash array syntax is not executable yet"));
    }

    /* First expand the arguments. */
    sh.eval.back_exitstatus = ExitStatus::SUCCESS;

    cmd_flag = 0;
    exec_builtin = false;
    special_builtin = None;
    variable_attributes = VariableAttributes::NONE;
    use_local_variables = false;
    argc = 0;
    argp = command.arguments.as_slice();
    osp = fill_arglist(sh, &mut arglist, &mut argp)?;
    if osp.is_some() {
        let mut assignments_are_arguments = false;

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
                crate::mystring::cstr_prefix(&arglist.list[head].text),
                &mut resolved_command,
                cmd_flag | DO_REGBLTIN,
                BStr::new(regpath.as_slice()),
            )? {
                Flow::Done(_) => {}
                control => return Ok(control),
            }

            use_local_variables = true;

            /* implement bltin and command here */
            let Command::Builtin(builtin) = &resolved_command else {
                break;
            };
            let builtin = *builtin;

            assignments_are_arguments = builtin.attributes().takes_assignments();
            if special_builtin.is_none() {
                let special = builtin.attributes().is_special();
                special_builtin = Some(special);
                use_local_variables = !special;
            }
            exec_builtin = builtin.id() == BuiltinId::Exec;
            if builtin.id() != BuiltinId::Command {
                break;
            }

            cmd_flag = parse_command_args(
                sh,
                &mut arglist,
                &mut argp,
                &mut path,
                standard_path.as_slice().as_bstr(),
                &mut head,
            )?;
            if cmd_flag == 0 {
                break;
            }
        }

        for a in argp {
            crate::expand::expandarg(
                sh,
                a,
                Some(&mut arglist),
                if assignments_are_arguments
                    && matches!(
                        a,
                        Node::Word(word)
                            if crate::parser::isassignment(
                                &sh.locale,
                                word.word.as_bstr(),
                            ) != 0
                    )
                {
                    ExpansionMode::ASSIGNMENT_TILDE
                } else {
                    ExpansionMode::SPLIT | ExpansionMode::TILDE
                },
            )?;
        }

        argc = (arglist.list.len() - head) as c_int;

        if exec_builtin && argc > 1 {
            variable_attributes = VariableAttributes::EXPORTED;
        }
    }

    resources.begin_local_variables(sh, use_local_variables);

    lastarg = if sh.options.enabled(ShellOption::Interactive) && sh.eval.funcline == 0 && argc > 0 {
        Some(arglist.list.len() - 1)
    } else {
        None
    };

    let stderr = sh.fds.slot(LogicalDescriptor::STDERR);
    sh.io.previous_stderr().set_destination(stderr);
    let expanded_redirections = expredir(sh, &command.redirections)?;
    /* `status = redirectsafe(..)`, which the C computes as `setjmp(..) *
     * 2`. The value is kept as well as the status, because `bail:` below
     * re-raises it when the command is a special built-in — that is the
     * one place a redirection error is *not* swallowed, and an `int`
     * cannot be re-raised. */
    let mut redir_err: Option<Error> = None;
    match resources.apply_redirections(sh, &expanded_redirections) {
        /* Same as compound-redirection evaluation: an interrupt leaves rather than
         * becoming this command's status. */
        Err(e) if e.is_interrupt() || e.is_expansion() => return Err(e),
        Err(e) => {
            /* From the value; see the compound-redirection arm. Read before the move
             * into `redir_err`, which is where it is re-raised from. */
            status = e.status();
            redir_err = Some(e);
        }
        Ok(()) => status = ExitStatus::SUCCESS,
    }

    'out_lbl: {
        'bail: {
            if !status.success() {
                break 'bail;
            }

            for a in &command.assignments {
                let spp: usize;

                spp = varlist.list.len();
                crate::expand::expandarg(
                    sh,
                    a,
                    Some(&mut varlist),
                    ExpansionMode::ASSIGNMENT_TILDE,
                )?;
                /* `(*spp)->text` with no null check: EXP_VARTILDE has no
                 * EXP_FULL, so `expandarg` appended exactly one entry. */
                debug_assert_eq!(
                    varlist.list.len(),
                    spp + 1,
                    "an unsplit expansion is one field"
                );

                if use_local_variables {
                    crate::var::make_local_bytes(
                        sh,
                        crate::mystring::cstr_prefix(&varlist.list[spp].text),
                        VariableAttributes::EXPORTED,
                    )?;
                } else {
                    crate::var::set_assignment_bytes(
                        sh,
                        crate::mystring::cstr_prefix(&varlist.list[spp].text),
                        variable_attributes,
                    )?;
                }
            }

            /* Print the command if xflag is set. */
            if sh.options.enabled(ShellOption::Xtrace) && sh.eval.inps4 == 0 {
                let mut sep: c_int;

                /* This block is why `Dest` exists. It used to open with
                 * `out = previous_stderr()` and then hold that pointer
                 * across `ps4val(sh)`, `expandstr(sh, ..)` and two
                 * `eprintlist` calls — five reborrows of the shell with a
                 * raw pointer into its I/O still live. Sound while the
                 * pointer came from a static; undefined the moment it
                 * comes from `&mut sh.io`. Naming the destination defers
                 * the resolution to each write, so nothing spans a call. */
                let dest = Dest::PreviousStderr;
                sh.eval.inps4 = 1;
                /* Hoisted out of `expandstr`'s argument list; see the
                 * note in `evalcommand`. */
                let ps4 = crate::var::ps4val(sh);
                let prompt = crate::parser::expandstr(sh, BStr::new(ps4.as_slice()))?;
                let _ = sh.io.get(dest).write_all(&prompt);
                sh.eval.inps4 = 0;
                sep = 0;
                sep = eprintlist(sh.io.get(dest), &varlist.list, sep);
                /* `eprintlist(sh, out, osp, sep)` prints from the *original*
                 * head, so `command -p foo` traces as it was written and not
                 * as `parse_command_args` left it.  A NULL `osp` prints
                 * nothing, which is the empty slice. */
                eprintlist(
                    sh.io.get(dest),
                    &arglist.list[osp.unwrap_or(arglist.list.len())..],
                    sep,
                );
                let _ = sh.io.get(dest).write_all(b"\n");
            }

            /* Now locate the command. */
            if !matches!(
                &resolved_command,
                Command::Builtin(builtin) if builtin.attributes().is_regular()
            ) {
                if path.is_none() {
                    path = Some(crate::var::pathval(sh));
                }
                let search_path =
                    BStr::new(path.as_ref().expect("command lookup has a PATH").as_slice());
                let command_name = crate::mystring::cstr_prefix(&arglist.list[head].text);
                match find_command(
                    sh,
                    command_name,
                    &mut resolved_command,
                    cmd_flag | DO_ERR,
                    search_path,
                )? {
                    Flow::Done(_) => {}
                    exit @ Flow::Exit { .. } => return Ok(exit),
                    control => {
                        command_control = Some(control);
                        break 'out_lbl;
                    }
                }
            }

            jp = None;

            /* Execute the command. */
            match resolved_command {
                Command::Unknown => {
                    status = ExitStatus::NOT_FOUND;
                    break 'bail;
                }

                Command::Builtin(builtin) => {
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
                    match evalbltin(sh, builtin, &mut arglist.list[head..], context) {
                        Ok(flow) => {
                            if let Err(exit) = capture_local_control(flow, &mut command_control) {
                                return Ok(exit);
                            }
                        }
                        Err(e) => {
                            /* The C's `!(exception == EXERROR && spclbltin
                             * <= 0)`. An interrupt is not an EXERROR and
                             * was never swallowed here; now that it is a
                             * value, saying so is a test on the value.
                             *
                             * A signal trap adds one catch boundary. Its
                             * command failures must reach `dotrap` as a
                             * status so the interrupted status can be
                             * restored; returning the typed special-builtin
                             * error here would instead abort the shell and
                             * skip this command's ordinary cleanup. */
                            // [spec:nsh:req:compat.smoosh.trap-status]
                            if builtin_error_is_fatal(sh, special_builtin.unwrap_or(false), &e) {
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

                Command::Function(function) => {
                    /* `if (evalfun(..)) goto raise;` -- a function body is
                     * not a builtin, so there is nothing to swallow: both an
                     * exit and a diagnostic leave through this frame. */
                    let args = crate::builtins::args(&arglist.list[head..]);
                    if let Err(exit) = capture_local_control(
                        evalfun(sh, &function, &args, context)?,
                        &mut command_control,
                    ) {
                        return Ok(exit);
                    }
                }

                Command::External { path_index } => {
                    sh.flush_input();
                    let args = crate::builtins::args(&arglist.list[head..]);

                    /* Fork off a child process if necessary. */
                    if !context.exits() || crate::trap::have_traps(sh) != 0 {
                        let syntax = Node::Command(command.clone());
                        status = crate::error::with_interrupts_deferred(sh, |sh| {
                            let job = crate::jobs::forkexec(
                                sh,
                                &syntax,
                                &args,
                                BStr::new(
                                    path.as_ref()
                                        .expect("external command has a PATH")
                                        .as_slice(),
                                ),
                                path_index,
                            )?;
                            crate::jobs::waitforjob(sh, Some(job))
                        })?;
                        crate::error::clear_interrupt_deferral(&mut sh.interrupt_deferral);
                        break 'out_lbl;
                    } else {
                        /* `shellexec` replaces the process image or fails;
                         * failing, it reports and is the C's EXEND. */
                        return shellexec(
                            sh,
                            &args,
                            BStr::new(
                                path.as_ref()
                                    .expect("external command has a PATH")
                                    .as_slice(),
                            ),
                            path_index,
                        );
                    }
                }
            }

            status = crate::jobs::waitforjob(sh, jp)?;
            crate::error::clear_interrupt_deferral(&mut sh.interrupt_deferral);
            break 'out_lbl;
        }
        // bail:
        /* A redirection-only command has no builtin entry whose specialness
         * can classify the failure. The adopted Smoosh contract uses the
         * shell-error status 1 for that path; this is the foreground half of
         * the parsed `exec 9&<-` case. */
        // [spec:nsh:req:compat.smoosh.error-contracts]
        status = redirection_only_status(status, redir_err.as_ref(), osp.is_some());
        sh.status = status;

        /* We have a redirection error. */
        if special_builtin == Some(true) {
            /* POSIX's "an error in a special built-in exits a
             * non-interactive shell", and the C's textless
             * `exraise(EXERROR)`: no diagnostic is written here because
             * whatever failed wrote its own.
             *
             * `redirectsafe` hands its error back, so the usual way in
             * carries the value. The other way in is an unknown command with
             * status 127, where there is no value to carry: `find_command`
             * reported "not found" and returned normally, which is
             * `docs/api-design.md` 3.3's "reported and carried on past".
             * `Error::reported` is that case -- a value with no text,
             * because the text has already been written. */
            let error = match redir_err.take() {
                Some(e) => {
                    debug_assert_eq!(e.status(), status, "a redirection error keeps its status");
                    e
                }
                None => crate::error::Error::reported(sh.eval.errlinno, status),
            };
            debug_assert!(
                !error.is_expansion(),
                "expansion errors bypass redirection status"
            );
            // Smoosh's adopted POSIX closure profile assigns status 1 to a
            // redirection failure on a directly invoked special builtin.
            // Its diagnostic was already written by the redirection layer.
            // [spec:nsh:req:compat.smoosh.error-contracts]
            sh.status = ExitStatus::FAILURE;
            return Err(crate::error::Error::reported(sh.eval.errlinno, 1));
        }

        // goto out
    }
    // out:
    if exec_builtin {
        resources.retain_redirections(sh);
    }
    resources.restore(sh);
    if let Some(lastarg) = lastarg {
        /* dsl: I think this is intended to be used to support
         * '_' in 'vi' command mode during line editing...
         * However I implemented that within libedit itself.
         */
        crate::var::set_bytes(
            sh,
            BStr::new(b"_"),
            Some(crate::mystring::cstr_prefix(&arglist.list[lastarg].text)),
            VariableAttributes::NONE,
        )?;
    }

    Ok(command_control
        .unwrap_or(Flow::Done(status))
        .with_status(status))
}

// [spec:dash:def:eval.evalbltin-fn]
// [spec:dash:sem:eval.evalbltin-fn]
fn evalbltin(
    sh: &mut Shell,
    cmd: &'static BuiltinSpec,
    fields: &mut [strlist],
    context: EvalContext,
) -> Result<Flow, Error> {
    let savecmdname: Option<BString>; /* volatile */

    savecmdname = core::mem::take(&mut sh.eval.commandname);
    /* `commandname = argv[0]`, and NULL for the command that has no word
     * at all -- the assignment-only one `bltin` stands for. */
    sh.eval.commandname = fields
        .first()
        .map(|field| BString::from(crate::mystring::cstr_prefix(&field.text)));

    let outcome = (|| -> Result<Flow, Error> {
        let command_flow = match cmd.handler() {
            BuiltinHandler::History => crate::builtins::fc::histcmd_fields(sh, fields)?,
            BuiltinHandler::Eval => {
                let args = crate::builtins::args(fields);
                crate::builtins::eval::evalcmd(sh, &args, context)?
            }
            BuiltinHandler::Standard(entry) => {
                let args = crate::builtins::args(fields);
                entry(sh, &args)?
            }
        };
        if matches!(command_flow, Flow::Exit { .. }) {
            return Ok(command_flow);
        }
        let mut status = command_flow
            .status()
            .expect("non-exit builtin control carries a command status");
        /* Every `?` and every `Flow::Exit` above skips the rest of this,
         * exactly as the C's `goto cmddone` skipped it. */
        if sh.io.flushall().is_err() {
            // [spec:nsh:req:compat.smoosh.error-contracts]
            sh.diagnostics().command_warnx(b"I/O error");
            status = ExitStatus::ERROR;
        }
        sh.status = status;
        Ok(command_flow.with_status(status))
    })();

    // cmddone:
    /* The C's epilogue, and the reason it armed a handler at all: an
     * exception raised *beneath* a built-in had to run `freestdout` and
     * restore `commandname` on its way out rather than skip them. It runs
     * on every path here because there is only one way out now. `handler`
     * was the third thing it restored and there is no handler left. */
    crate::output::freestdout(&mut sh.io);
    sh.eval.commandname = savecmdname;

    outcome
}

// [spec:dash:def:eval.evalfun-fn]
// [spec:dash:sem:eval.evalfun-fn]
// [spec:posix:req:cmd.function-invocation-positional-parameters]
// [spec:posix:req:cmd.function-return]
// [spec:posix:req:cmd.function-exit-status]
// [spec:posix:req:cmd.function-syntax-error-properties]
fn evalfun(
    sh: &mut Shell,
    function: &FunctionDefinition,
    args: &[&BStr],
    context: EvalContext,
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

    crate::error::with_interrupts_deferred(sh, |sh| {
        /* Command lookup cloned the owned body, so redefining this function
         * while it runs cannot pull the body out from under this call. */
        sh.eval.funcline = function.line;
        // [spec:nsh:req:compat.smoosh.nonlexical-control]
        // Ordinarily only loops lexically inside the function are visible.
        // The explicit extension preserves the caller's dynamic loop depth so
        // break/continue can leave through this frame and be consumed there.
        if !sh.options.enabled(ShellOption::NonLexicalControl) {
            sh.eval.loopnest = 0;
        }
    });
    crate::options::setparam(sh, args.get(1..).unwrap_or_default());

    let outcome = evaltree(sh, Some(function.body.as_ref()), context.tested_only());

    // funcdone:
    crate::error::with_interrupts_deferred(sh, |sh| {
        sh.eval.loopnest = saveloopnest;
        sh.eval.funcline = savefuncline;
        crate::options::restoreparam(sh, saveparam);
    });
    match outcome? {
        Flow::Return { status, .. } => Ok(Flow::Done(status)),
        control => Ok(control),
    }
}

/*
 * Search for a command.  This is called before we fork so that the
 * location of the command will be available in the parent as well as
 * the child.  The check for "goodname" is an overly conservative
 * check that the name will not be subject to expansion.
 */

// [spec:dash:def:eval.prehash-fn]
// [spec:dash:sem:eval.prehash-fn]
fn prehash(sh: &mut Shell, n: &Node) -> Result<Flow, Error> {
    let mut entry = Command::Unknown;

    if let Node::Command(command) = n
        && let Some(Node::Word(word)) = command.arguments.first()
        && crate::parser::goodname(&sh.locale, word.word.as_bstr()) != 0
    {
        /* Hoisted out of the argument list; see the note in
         * `evalcommand`. */
        let path = crate::var::pathval(sh);
        return find_command(
            sh,
            word.word.as_bstr(),
            &mut entry,
            0,
            BStr::new(path.as_slice()),
        );
    }
    Ok(Flow::Done((0).into()))
}

/// With `set -h`, remember literal command names while a function is
/// defined. This walks only command-bearing tree edges; words, redirection
/// operands and here-documents are not executed or expanded.
// [spec:nsh:req:compat.smoosh.hash-all]
fn prehash_tree(sh: &mut Shell, n: Option<&Node>) -> Result<Flow, Error> {
    let Some(n) = n else {
        return Ok(Flow::Done((0).into()));
    };

    match n {
        Node::Command(_) => return prehash(sh, n),
        Node::Pipeline(pipeline) => {
            for command in &pipeline.commands {
                let _ = flow!(prehash_tree(sh, Some(command)));
            }
        }
        Node::Redirect(command) | Node::Background(command) | Node::Subshell(command) => {
            let _ = flow!(prehash_tree(sh, Some(command.command.as_ref())));
        }
        Node::And(binary)
        | Node::Or(binary)
        | Node::Sequence(binary)
        | Node::While(binary)
        | Node::Until(binary) => {
            let _ = flow!(prehash_tree(sh, Some(binary.left.as_ref())));
            let _ = flow!(prehash_tree(sh, Some(binary.right.as_ref())));
        }
        Node::If(conditional) => {
            let _ = flow!(prehash_tree(sh, Some(conditional.condition.as_ref())));
            let _ = flow!(prehash_tree(sh, Some(conditional.then_branch.as_ref())));
            let _ = flow!(prehash_tree(sh, conditional.else_branch.as_deref()));
        }
        Node::For(command) => {
            let _ = flow!(prehash_tree(sh, Some(command.body.as_ref())));
        }
        Node::Case(command) => {
            for clause in &command.clauses {
                let _ = flow!(prehash_tree(sh, clause.body.as_deref()));
            }
        }
        Node::Function(definition) => {
            let _ = flow!(prehash_tree(sh, Some(definition.body.as_ref())));
        }
        Node::Not(command) => {
            let _ = flow!(prehash_tree(sh, Some(command.command.as_ref())));
        }
        Node::Word(_) | Node::Bash(_) => {}
    }

    Ok(Flow::Done((0).into()))
}

/*
 * Builtin commands.  Builtin commands whose functions are closely
 * tied to evaluation are implemented here.
 */

/*
 * No command given.
 */

/* Break, continue, and return are typed `Flow` values. Each loop or
 * function consumes its own level and propagates the rest. */

/*
 * The return command.
 */

// [spec:dash:def:eval.eprintlist-fn]
// [spec:dash:sem:eval.eprintlist-fn]
fn eprintlist(output: &mut crate::output::Output, list: &[strlist], sep: c_int) -> c_int {
    let mut sep: c_int = sep;

    for sp in list {
        let mut record = Vec::new();
        if sep != 0 {
            record.push(b' ');
        }
        record.extend_from_slice(crate::mystring::cstr_prefix(&sp.text));
        sep |= 1;
        let _ = output.write_all(&record);
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
    //! past this frame; and that an explicit exit carries its selected
    //! status while EXEND does not. The behaviour is pinned end to end in
    //! `tests/errors_are_values.rs`.

    use super::*;

    // [spec:nsh:req:idiom.immutable-ast/test]
    #[test]
    fn redirection_expansion_stays_evaluation_local() {
        let _guard = crate::testutil::lock();
        let mut sh = crate::context::Shell::builder().build().unwrap();
        crate::input::setinputstring(&mut sh, BStr::new(b": >\"$target\"\n"));
        let tree = match crate::parser::parsecmd(&mut sh, 0).unwrap() {
            crate::parser::ParseResult::Tree(Some(tree)) => tree,
            _ => panic!("expected a command"),
        };
        let Node::Command(command) = &tree else {
            panic!("expected a simple command");
        };
        let Redirection::File(parsed) = &command.redirections[0] else {
            panic!("expected a file redirection");
        };
        let parsed_spelling = parsed.target.word.as_bstr().to_owned();

        crate::var::set_bytes(
            &mut sh,
            BStr::new(b"target"),
            Some(BStr::new(b"one")),
            VariableAttributes::NONE,
        )
        .unwrap();
        let first = expredir(&mut sh, &command.redirections).unwrap();
        assert!(matches!(
            &first[0],
            ExpandedRedirection::File { target, .. } if target == BStr::new(b"one")
        ));
        drop(first);

        crate::var::set_bytes(
            &mut sh,
            BStr::new(b"target"),
            Some(BStr::new(b"two")),
            VariableAttributes::NONE,
        )
        .unwrap();
        let second = expredir(&mut sh, &command.redirections).unwrap();
        assert!(matches!(
            &second[0],
            ExpandedRedirection::File { target, .. } if target == BStr::new(b"two")
        ));
        assert_eq!(parsed.target.word.as_bstr(), parsed_spelling.as_bstr());
    }

    /// `flow!` on a finished evaluation yields the status and carries on.
    // [spec:dash:sem:eval.evaltree-fn/test]
    #[test]
    fn flow_yields_a_status() {
        fn body(inner: Result<Flow, Error>) -> Result<Flow, Error> {
            let status = flow!(inner);
            Ok(Flow::Done(
                (ExitStatus::from_code(i32::from(status.code()) + 100)).into(),
            ))
        }
        let got = body(Ok(Flow::Done((7).into())));
        assert_eq!(got.unwrap(), Flow::Done((107).into()));
    }

    /// …and on an exit it returns, so nothing after it runs. That is the
    /// whole of what the C got from jumping past the frame, and getting
    /// it wrong would run epilogues the unwind skipped.
    // [spec:dash:sem:eval.evaltree-fn/test]
    #[test]
    fn flow_returns_an_exit() {
        fn body(inner: Result<Flow, Error>) -> Result<Flow, Error> {
            let _status = flow!(inner);
            panic!("flow! must not fall through on an exit");
        }
        let got = body(Ok(Flow::exit(9)));
        assert_eq!(
            got.unwrap(),
            Flow::Exit {
                status: Some(ExitStatus::from_code(9))
            }
        );
    }

    /// A diagnostic still propagates through it, because the `?` is
    /// inside: `flow!` adds an arm, it does not replace one.
    // [spec:dash:sem:eval.evaltree-fn/test]
    #[test]
    fn flow_still_propagates_an_error() {
        fn body(inner: Result<Flow, Error>) -> Result<Flow, Error> {
            let _status = flow!(inner);
            panic!("flow! must not fall through on an error");
        }
        let e = Error::Other {
            line: 3,
            status: ExitStatus::ERROR,
            message: bstr::BString::from(&b"nope"[..]),
        };
        let got = body(Err(e));
        assert_eq!(got.unwrap_err().message(), "nope");
    }

    /// EXEXIT owns the selected status while EXEND uses the status already
    /// on the shell.
    // [spec:dash:sem:init.exitreset-fn/test]
    // [spec:nsh:req:compat.smoosh.trap-status/test]
    #[test]
    fn explicit_exit_carries_status() {
        assert_eq!(
            Flow::exit(9),
            Flow::Exit {
                status: Some(ExitStatus::from_code(9))
            }
        );
        assert_eq!(Flow::END, Flow::Exit { status: None });
        assert_ne!(Flow::exit(9), Flow::END);
    }

    /// The catch frame applies any selected status before cleanup. Reset
    /// therefore cannot overwrite the status chosen by either exit path.
    // [spec:dash:sem:init.exitreset-fn/test]
    // [spec:nsh:req:compat.smoosh.trap-status/test]
    #[test]
    fn exitreset_preserves_status() {
        let _guard = crate::testutil::lock();
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let sh = &mut owned;

        sh.status = ExitStatus::from_code(9);
        sh.eval.loopnest = 3;
        sh.eval.inps4 = 1;
        sh.clear_evaluation_resources();
        assert_eq!(sh.status, ExitStatus::from_code(9));
        assert_eq!(sh.eval.loopnest, 0);
        assert_eq!(sh.eval.inps4, 0);
    }
}
