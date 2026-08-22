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
mod bash_arithmetic;
mod bash_arrays;
mod bash_conditional;

use crate::context::Shell;
use crate::descriptors::LogicalDescriptor;
use crate::error::Error;
use crate::status::ExitStatus;
use bstr::{BStr, BString, ByteSlice};
use nsh_platform::Descriptor;

use crate::builtins::{BuiltinHandler, BuiltinId, BuiltinSpec};
use crate::execution::{Command, CommandSearch, execute_external_command, find_command};
use crate::expand::{ExpandedField, ExpandedFields, ExpansionMode};
use crate::jobs::{ForkMode, JobId};
// [spec:nsh:def:idiom.job-control-model]
use crate::nodes::{
    BinaryCommand, CaseCommand, CompoundCommand, DescriptorTarget, ForCommand, FunctionDefinition,
    Node, Pipeline, Redirection, SimpleCommand,
};
use crate::options::ShellOption;
use crate::output::OutputDestination;
// [spec:nsh:def:idiom.shell-options]
use crate::redirection::{ExpandedRedirection, RedirectionMode};
use crate::variables::VariableAttributes;

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
pub(crate) struct EvaluationContext {
    exit: bool,
    tested: bool,
}

impl EvaluationContext {
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

pub struct CommandSubstitution {
    /* result of evalbackcmd */
    pub descriptor: Option<Descriptor>, /* descriptor to read from */
    pub job_id: Option<JobId>,          /* index of the job structure for command */
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
pub struct EvaluationState {
    /// Current loop nesting level.
    pub(crate) loop_depth: usize,
    /// starting line number of current function, or 0
    ///
    /// Private: `eval.rs` is the only module that names it.
    function_line: i32,
    /// Prevent PS4 nesting.
    pub(crate) expanding_trace_prompt: bool,
    /// exit status of backquoted command
    pub(crate) command_substitution_status: ExitStatus,
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
    pub(crate) diagnostic_line: i32,
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
    pub(crate) command_name: Option<BString>,
}

impl EvaluationState {
    /// What the eight statics were declared with.
    pub(crate) const fn new() -> Self {
        EvaluationState {
            loop_depth: 0,
            function_line: 0,
            expanding_trace_prompt: false,
            command_substitution_status: ExitStatus::SUCCESS,
            signal_trap_depth: 0,
            trap_default_exit_status: None,
            diagnostic_line: 0,
            command_name: None,
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
    ($error:expr) => {
        match $error? {
            $crate::evaluation::Flow::Done(status) => status,
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

// [spec:dash:sem:eval.evalstring-fn]
pub fn evaluate_string(
    shell: &mut Shell,
    text: &BStr,
    context: EvaluationContext,
) -> Result<Flow, Error> {
    /* `sstrdup(s)` and the `stunalloc(s)` at the bottom are one thing:
     * `setinputstring` keeps the pointer rather than copying, so the text
     * has to outlive every `popstackmark` the parse below performs — which
     * is why the copy is taken *before* the mark is set and released by
     * hand afterwards.  Owning it says both halves at once, and says them
     * on the unwind path too, where the C's `stunalloc` never runs. */
    crate::resource::with_resources(shell, |shell, _resources| {
        crate::input::set_input_string(shell, text);
        parse_and_execute(shell, context)
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
pub(crate) fn parse_and_execute(
    shell: &mut Shell,
    context: EvaluationContext,
) -> Result<Flow, Error> {
    let mut status = ExitStatus::SUCCESS;
    loop {
        let command: Option<Node> = match crate::parser::parse_command(shell, false)? {
            crate::parser::ParseResult::Eof => break,
            crate::parser::ParseResult::Tree(command) => command,
        };
        {
            let command_context = if crate::parser::parser_eof(shell) {
                context
            } else {
                context.without_exit()
            };
            let command_status =
                flow!(evaluate_top_level(shell, command.as_ref(), command_context));
            if command.is_some() {
                status = command_status;
            }
        }
        /* `popstackmark(&smark)` — one per parsed command, and one on the
         * way out. */
    }
    Ok(Flow::Done(status))
}

/// Evaluate one parsed top-level command, retaining the rest of an
/// interactive command list after a parameter-expansion failure.
///
/// The ordinary evaluator returns the error because a non-interactive shell
/// must terminate. An interactive root instead abandons the affected command,
/// restores its temporary state, and resumes at the next `;` command (or the
/// next parsed input record).
// [spec:nsh:req:compat.smoosh.error-contracts]
pub(crate) fn evaluate_top_level(
    shell: &mut Shell,
    node: Option<&Node>,
    context: EvaluationContext,
) -> Result<Flow, Error> {
    if !shell.options.enabled(ShellOption::Interactive) || shell.shell_level != 0 {
        return evaluate_tree(shell, node, context);
    }
    evaluate_interactive_sequence(shell, node, context)
}

/// Record the line the command about to run begins on.
///
/// dash reports `$LINENO` inside a function body relative to the
/// function's own first line; Bash reports the line in the file, which is
/// what `BASH_LINENO`, `caller`, and the `DEBUG` and `ERR` actions all
/// read. The subtraction is therefore the POSIX dialect's alone.
// [spec:nsh:req:compat.bash.traps-introspection]
fn record_command_line(shell: &mut Shell, line: i32) {
    shell.evaluation.diagnostic_line = line;
    shell.variables.line_number = line;
    if shell.evaluation.function_line != 0
        && shell.options.dialect() == crate::options::Dialect::Posix
    {
        shell.variables.line_number -= shell.evaluation.function_line - 1;
    }
}

/// Put `$LINENO` back where the call was written.
///
/// Bash's `$LINENO` after a function returns is the caller's line again,
/// which is what an `ERR` action raised *by the call* reports. dash never
/// restored it, so this is the Bash dialect's alone.
// [spec:nsh:req:compat.bash.traps-introspection]
pub(crate) fn restore_caller_line(shell: &mut Shell, line: i32) {
    if shell.options.dialect() == crate::options::Dialect::Bash {
        shell.variables.line_number = line;
    }
}

/// Re-record `line` and raise `DEBUG` for a command Bash announces more
/// than once: a `for` header once per iteration, a pipeline element once
/// per element, a `for ((;;))` header once per expression.
///
/// The dialect guard is here rather than at each site because the line is
/// re-recorded as well as the action raised, and re-recording is only
/// wanted where Bash's repetition is.
// [spec:nsh:req:compat.bash.traps-introspection]
fn repeat_debug_trap(shell: &mut Shell, line: i32) -> Result<Flow, Error> {
    if shell.options.dialect() != crate::options::Dialect::Bash {
        return Ok(Flow::Done(shell.status));
    }
    record_command_line(shell, line);
    crate::trap::bash::run_debug(shell)
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

fn builtin_error_is_fatal(shell: &Shell, special_builtin: bool, error: &Error) -> bool {
    error.is_interrupt() || (special_builtin && shell.evaluation.signal_trap_depth == 0)
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

fn evaluate_interactive_sequence(
    shell: &mut Shell,
    node: Option<&Node>,
    context: EvaluationContext,
) -> Result<Flow, Error> {
    if let Some(Node::Sequence(sequence)) = node {
        match evaluate_interactive_sequence(
            shell,
            Some(sequence.left.as_ref()),
            context.tested_only(),
        )? {
            Flow::Done(_) => {}
            control => return Ok(control),
        }
        return evaluate_interactive_sequence(shell, Some(sequence.right.as_ref()), context);
    }

    let outcome = crate::resource::with_resources(shell, |shell, _resources| {
        evaluate_tree(shell, node, context)
    });
    match outcome {
        Err(error) if error.is_expansion() => {
            let status = error.status();
            shell.status = status;
            drop(error);
            shell.clear_evaluation_resources();
            shell.unwind_local_variables();
            crate::error::clear_interrupt_deferral(&mut shell.interrupt_deferral);
            Ok(Flow::Done(status))
        }
        outcome => outcome,
    }
}

/*
 * Evaluate a parse tree.  The value is left in the global variable
 * exitstatus.
 */

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
pub fn evaluate_tree(
    shell: &mut Shell,
    node: Option<&Node>,
    context: EvaluationContext,
) -> Result<Flow, Error> {
    let mut check_exit = false;
    let mut status = ExitStatus::SUCCESS;

    if !shell.options.enabled(ShellOption::NoExec)
        && let Some(node) = node
    {
        flow!(crate::trap::run_pending_traps(shell));
        shell.display_history = true;
        // [spec:nsh:req:idiom.structural-ast]
        status = match node {
            Node::Redirect(redirection) => {
                record_command_line(shell, redirection.line);
                let expanded_redirections = expand_redirections(shell, &redirection.redirections)?;
                let outcome = crate::resource::with_resources(shell, |shell, resources| {
                    match resources.apply_redirections(shell, &expanded_redirections) {
                        Err(error) if error.is_interrupt() || error.is_expansion() => Err(error),
                        Err(error) => {
                            drop(error);
                            check_exit = true;
                            Ok(Flow::Done(ExitStatus::FAILURE))
                        }
                        Ok(()) => evaluate_tree(
                            shell,
                            Some(redirection.command.as_ref()),
                            context.tested_only(),
                        ),
                    }
                });
                match outcome? {
                    Flow::Done(status) => status,
                    control => return Ok(control),
                }
            }
            Node::Command(command) => {
                check_exit = true;
                /* A command with no `|` is a pipeline of one, and Bash's
                 * `${PIPESTATUS[@]}` says so. The forked forms publish
                 * from `wait_for_job`, which is the only place that can
                 * still see every member. */
                // [spec:nsh:req:compat.bash.special-variables]
                let status = flow!(evaluate_command(shell, command, context));
                crate::variables::special::set_pipeline_status(shell, &[status]);
                status
            }
            Node::For(command) => flow!(evaluate_for(shell, command, context)),
            Node::While(command) => flow!(evaluate_loop(shell, command, false, context)),
            Node::Until(command) => flow!(evaluate_loop(shell, command, true, context)),
            Node::Subshell(command) => {
                check_exit = true;
                flow!(evaluate_subshell(shell, command, false, context))
            }
            Node::Background(command) => {
                check_exit = true;
                flow!(evaluate_subshell(shell, command, true, context))
            }
            Node::Pipeline(pipeline) => {
                check_exit = true;
                flow!(evaluate_pipeline(shell, pipeline, context))
            }
            Node::Case(command) => flow!(evaluate_case(shell, command, context)),
            Node::And(command) => {
                let left = flow!(evaluate_tree(
                    shell,
                    Some(command.left.as_ref()),
                    EvaluationContext::TESTED
                ));
                if !left.success() {
                    left
                } else {
                    flow!(evaluate_tree(shell, Some(command.right.as_ref()), context))
                }
            }
            Node::Or(command) => {
                let left = flow!(evaluate_tree(
                    shell,
                    Some(command.left.as_ref()),
                    EvaluationContext::TESTED
                ));
                if left.success() {
                    left
                } else {
                    flow!(evaluate_tree(shell, Some(command.right.as_ref()), context))
                }
            }
            Node::Sequence(command) => {
                // A sequence's observable status is its right-hand command.
                flow!(evaluate_tree(
                    shell,
                    Some(command.left.as_ref()),
                    context.tested_only(),
                ));
                flow!(evaluate_tree(shell, Some(command.right.as_ref()), context))
            }
            Node::If(command) => {
                let condition = flow!(evaluate_tree(
                    shell,
                    Some(command.condition.as_ref()),
                    EvaluationContext::TESTED,
                ));
                if condition.success() {
                    flow!(evaluate_tree(
                        shell,
                        Some(command.then_branch.as_ref()),
                        context
                    ))
                } else if command.else_branch.is_some() {
                    flow!(evaluate_tree(
                        shell,
                        command.else_branch.as_deref(),
                        context
                    ))
                } else {
                    ExitStatus::SUCCESS
                }
            }
            Node::Function(definition) => {
                if shell.options.enabled(ShellOption::HashAll) {
                    flow!(prehash_tree(shell, Some(definition.body.as_ref())));
                }
                crate::execution::define_function(
                    &mut shell.interrupt_deferral,
                    &mut shell.commands,
                    definition,
                );
                // [spec:nsh:req:compat.bash.traps-introspection]
                crate::variables::call_stack::record_definition(shell, definition.name.as_bstr());
                ExitStatus::SUCCESS
            }
            Node::Bash(crate::nodes::BashNode::ArrayAssignment(assignment)) => {
                bash_arrays::assign(
                    shell,
                    assignment,
                    crate::variables::arrays::ReadOnlyGuard::Enforce,
                )?;
                ExitStatus::SUCCESS
            }
            // Both Bash spellings define the same kind of function; the
            // owned body is retained exactly as the POSIX form's is.
            // [spec:nsh:req:compat.bash.functions-scoping]
            Node::Bash(crate::nodes::BashNode::Function(function)) => {
                if shell.options.enabled(ShellOption::HashAll) {
                    flow!(prehash_tree(shell, Some(function.body.as_ref())));
                }
                let definition = FunctionDefinition {
                    line: function.line,
                    name: function.name.clone(),
                    body: function.body.clone(),
                };
                crate::execution::define_function(
                    &mut shell.interrupt_deferral,
                    &mut shell.commands,
                    &definition,
                );
                // [spec:nsh:req:compat.bash.traps-introspection]
                crate::variables::call_stack::record_definition(shell, definition.name.as_bstr());
                ExitStatus::SUCCESS
            }
            Node::Bash(crate::nodes::BashNode::Conditional(conditional)) => {
                check_exit = true;
                record_command_line(shell, conditional.line);
                // [spec:nsh:req:compat.bash.traps-introspection]
                flow!(crate::trap::bash::run_debug(shell));
                bash_conditional::evaluate(shell, conditional)?
            }
            Node::Bash(crate::nodes::BashNode::ArithmeticCommand(arithmetic)) => {
                check_exit = true;
                record_command_line(shell, arithmetic.line);
                // [spec:nsh:req:compat.bash.traps-introspection]
                flow!(crate::trap::bash::run_debug(shell));
                bash_arithmetic::command(shell, arithmetic)?
            }
            Node::Bash(crate::nodes::BashNode::ArithmeticFor(loop_command)) => {
                flow!(bash_arithmetic::for_loop(shell, loop_command, context))
            }
            Node::Bash(_) => {
                return Err(shell
                    .diagnostics()
                    .shell_error(b"Bash syntax is parsed but not executable yet"));
            }
            Node::Not(command) => {
                let status = flow!(evaluate_tree(
                    shell,
                    Some(command.command.as_ref()),
                    EvaluationContext::TESTED,
                ));
                if status.success() {
                    ExitStatus::FAILURE
                } else {
                    ExitStatus::SUCCESS
                }
            }
            Node::Word(_) => {
                return Err(shell
                    .diagnostics()
                    .shell_error(b"non-command syntax reached evaluation"));
            }
        };
        shell.status = status;
    }
    flow!(crate::trap::run_pending_traps(shell));

    /* Bash raises `ERR` exactly where `errexit` would act, which is not
     * a coincidence to be re-derived: both ask whether this command's
     * failure is the shell's to notice, and a status the surrounding
     * syntax consumes is neither's business. One predicate, read twice. */
    // [spec:nsh:req:compat.bash.traps-introspection]
    let acted_on_failure = check_exit && !context.is_tested() && !status.success();
    if acted_on_failure {
        flow!(crate::trap::bash::run_err(shell));
    }
    let abort_for_errexit = acted_on_failure && shell.options.enabled(ShellOption::Errexit);
    if !abort_for_errexit && !context.exits() {
        return Ok(Flow::Done(shell.status));
    }
    Ok(Flow::END)
}

// [spec:dash:sem:eval.evaltreenr-fn]
//
// Child-side callers require an exit flow rather than an ordinary status.
pub fn evaluate_tree_without_exit(
    shell: &mut Shell,
    node: Option<&Node>,
    context: EvaluationContext,
) -> Result<Flow, Error> {
    /* The C's `noreturn` was true because every caller passes `EV_EXIT`,
     * and `evaltree`'s tail raises `EXEND` unconditionally under that
     * flag. It still cannot come back with a status -- that is what the
     * assertion says -- but "cannot come back" is now a `Flow::Exit`
     * travelling out through the caller rather than a jump past it. Each
     * of the three call sites is in a freshly forked child, whose copy of
     * every frame between here and `main` is its own, so returning
     * through them reaches the same `exit:` the longjmp reached. */
    let flow = match evaluate_tree(shell, node, context)? {
        exit @ Flow::Exit { .. } => exit,
        control @ (Flow::Break { .. } | Flow::Continue { .. } | Flow::Return { .. }) => {
            shell.status = control
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

// [spec:dash:sem:eval.evalloop-fn]
// [spec:posix:req:cmd.while-execution]
// [spec:posix:req:cmd.while-exit-status]
// [spec:posix:req:cmd.until-execution]
// [spec:posix:req:cmd.until-exit-status]
fn evaluate_loop(
    shell: &mut Shell,
    command: &BinaryCommand,
    until: bool,
    context: EvaluationContext,
) -> Result<Flow, Error> {
    let context = context.tested_only();

    shell.evaluation.loop_depth += 1;
    let outcome = (|| {
        let mut status = ExitStatus::SUCCESS;
        loop {
            let mut condition = match catch_one_loop(evaluate_tree(
                shell,
                Some(command.left.as_ref()),
                EvaluationContext::TESTED,
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
            match catch_one_loop(evaluate_tree(shell, Some(command.right.as_ref()), context)?) {
                LoopStep::Value(body_status) => status = body_status,
                LoopStep::Break(break_status) => return Ok(Flow::Done(break_status)),
                LoopStep::Continue(next_status) => status = next_status,
                LoopStep::Propagate(control) => return Ok(control),
            }
        }
    })();
    shell.evaluation.loop_depth -= 1;
    outcome
}

// [spec:dash:sem:eval.evalfor-fn]
// [spec:posix:req:cmd.for-iteration]
// [spec:posix:req:cmd.for-omitted-in]
// [spec:posix:req:cmd.for-exit-status]
fn evaluate_for(
    shell: &mut Shell,
    command: &ForCommand,
    context: EvaluationContext,
) -> Result<Flow, Error> {
    let mut expanded_fields = ExpandedFields::new();
    let mut status: ExitStatus;
    let context = context.tested_only();

    record_command_line(shell, command.line);

    for argument in &command.words {
        crate::expand::expand_argument(
            shell,
            argument,
            Some(&mut expanded_fields),
            ExpansionMode::SPLIT | ExpansionMode::TILDE,
        )?;
    }

    status = ExitStatus::SUCCESS;
    shell.evaluation.loop_depth += 1;
    for field in &expanded_fields.fields {
        /* Bash raises `DEBUG` once per iteration for a `for` command,
         * not once for the whole loop. */
        match repeat_debug_trap(shell, command.line)? {
            Flow::Done(_) => {}
            control => {
                shell.evaluation.loop_depth -= 1;
                return Ok(control);
            }
        }
        crate::variables::set_bytes(
            shell,
            command.variable.as_bstr(),
            Some(field.as_bstr()),
            VariableAttributes::NONE,
        )?;
        match catch_one_loop(evaluate_tree(shell, Some(command.body.as_ref()), context)?) {
            LoopStep::Value(body_status) => status = body_status,
            LoopStep::Break(break_status) => {
                status = break_status;
                break;
            }
            LoopStep::Continue(next_status) => status = next_status,
            LoopStep::Propagate(control) => {
                shell.evaluation.loop_depth -= 1;
                return Ok(control);
            }
        }
    }
    shell.evaluation.loop_depth -= 1;

    Ok(Flow::Done(status))
}

// [spec:dash:sem:eval.evalcase-fn]
// [spec:posix:req:cmd.case-selection]
// [spec:posix:req:cmd.case-pattern-expansion]
// [spec:posix:req:cmd.case-multiple-pattern-order-unspecified]
// [spec:posix:req:cmd.case-exit-status]
// [spec:posix:req:cmd.case-clause-terminators]
fn evaluate_case(
    shell: &mut Shell,
    command: &CaseCommand,
    context: EvaluationContext,
) -> Result<Flow, Error> {
    let mut expanded_fields = ExpandedFields::new();
    let mut status = ExitStatus::SUCCESS;
    let mut fallthrough = false;

    record_command_line(shell, command.line);
    // [spec:nsh:req:compat.bash.traps-introspection]
    flow!(crate::trap::bash::run_debug(shell));

    crate::expand::expand_argument(
        shell,
        command.word.as_ref(),
        Some(&mut expanded_fields),
        ExpansionMode::TILDE | ExpansionMode::PRESERVE_MULTIBYTE,
    )?;
    /* The C reads `arglist.list->text` with no null check, and is right to:
     * `expandarg` without EXP_FULL takes its single-field arm, which appends
     * exactly one entry whatever the word expands to. */
    debug_assert_eq!(
        expanded_fields.fields.len(),
        1,
        "an unsplit expansion is one field"
    );
    'case_done: {
        for clause in &command.clauses {
            let mut selected = fallthrough;
            if !selected {
                for pattern in &clause.patterns {
                    if crate::expand::case_pattern_matches(
                        shell,
                        pattern,
                        expanded_fields.fields[0].as_bstr(),
                    )? {
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
                status = flow!(evaluate_tree(shell, clause.body.as_deref(), context));
            }
            if clause.fallthrough {
                fallthrough = true;
            } else {
                break 'case_done;
            }
        }
    }
    // out:
    Ok(Flow::Done(status))
}

/*
 * Kick off a subshell to evaluate a tree.
 */

// [spec:dash:sem:eval.evalsubshell-fn]
// [spec:posix:req:jobctl.list-splitting]
// [spec:posix:def:jobctl.background-job]
// [spec:posix:def:jobctl.foreground-job]
// [spec:posix:req:exit.subshell-error-exit]
// [spec:posix:req:cmd.group-subshell]
// [spec:posix:req:cmd.group-exit-status]
// [spec:posix:req:cmd.async-subshell-background]
// [spec:posix:req:cmd.async-exit-status]
fn evaluate_subshell(
    shell: &mut Shell,
    command: &CompoundCommand,
    background: bool,
    context: EvaluationContext,
) -> Result<Flow, Error> {
    let fork_mode = if background {
        ForkMode::Background
    } else {
        ForkMode::Foreground
    };
    let mut status = ExitStatus::SUCCESS;
    let mut context = context;

    record_command_line(shell, command.line);

    let expanded_redirections = expand_redirections(shell, &command.redirections)?;
    /* Whether the tail below runs in a child of this process or in this
     * process. The structured scope restores the caller's interrupt depth
     * before either tail continues. */
    let forked = crate::error::with_interrupts_deferred(shell, |shell| {
        if !background && context.exits() && !crate::trap::has_traps(shell) {
            shell.prepare_fork_child(None);
            return Ok(Some(false));
        }
        let job_id = crate::jobs::create_job(shell, 1);
        if matches!(
            crate::jobs::fork_shell(
                shell,
                Some(job_id),
                Some(command.command.as_ref()),
                fork_mode
            )?,
            nsh_platform::ForkResult::Child
        ) {
            context = context.with_exit();
            if background {
                context = context.without_tested();
            }
            return Ok(Some(true));
        }
        /* the parent tail of the C function; the child path below
         * never returns, so it is reached only from here */
        if !background {
            status = crate::jobs::wait_for_job(shell, Some(job_id))?;
        }
        Ok::<_, Error>(None)
    })?;
    let Some(forked) = forked else {
        return Ok(Flow::Done(status));
    };
    let outcome = (|| -> Result<Flow, Error> {
        crate::redirection::redirect(shell, &expanded_redirections, RedirectionMode::Apply)?;
        evaluate_tree_without_exit(shell, Some(command.command.as_ref()), context)
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
        crate::runtime::exit_from_child(shell, outcome);
    }
    /* Not forked: this is still the same process, so the frames this returns
     * through are its own. */
    outcome
}

/*
 * Compute the names of the files in a redirection list.
 */

// [spec:dash:sem:eval.expredir-fn]
// [spec:posix:req:redir.word-expansion]
// [spec:posix:req:redir.word-pathname-expansion]
// [spec:posix:req:grammar.redirection-filename]
// [spec:nsh:def:idiom.logical-descriptors]
fn expand_redirections<'a>(
    shell: &mut Shell,
    redirections: &'a [Redirection],
) -> Result<Vec<ExpandedRedirection<'a>>, Error> {
    let mut expanded = Vec::with_capacity(redirections.len());
    for redirection in redirections {
        let mut fnl = ExpandedFields::new();
        match redirection {
            Redirection::File(redirection) => {
                let target = Node::Word(redirection.target.clone());
                crate::expand::expand_argument(
                    shell,
                    &target,
                    Some(&mut fnl),
                    ExpansionMode::TILDE | ExpansionMode::REDIRECTION,
                )?;
                /* `fn.list->text` with no null check: no EXP_FULL means
                 * `expandarg` took its single-field arm. */
                debug_assert_eq!(fnl.fields.len(), 1, "an unsplit expansion is one field");
                let target = fnl.fields.remove(0).text;
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
                        crate::expand::expand_argument(
                            shell,
                            &word,
                            Some(&mut fnl),
                            ExpansionMode::TILDE | ExpansionMode::REDIRECTION,
                        )?;
                        debug_assert_eq!(fnl.fields.len(), 1, "an unsplit expansion is one field");
                        descriptor_source(shell, fnl.fields[0].as_bstr())?
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

fn descriptor_source(shell: &mut Shell, text: &BStr) -> Result<Option<LogicalDescriptor>, Error> {
    if text.len() == 1 && text[0].is_ascii_digit() {
        Ok(Some(
            LogicalDescriptor::from_digit(text[0])
                .expect("an ASCII digit names a logical descriptor"),
        ))
    } else if text == BStr::new(b"-") {
        Ok(None)
    } else {
        let mut message = b"Bad fd number: ".to_vec();
        message.extend_from_slice(text);
        Err(shell.diagnostics().shell_error(&message))
    }
}

/// Raise `DEBUG` for a pipeline's simple commands, in the shell that
/// forks them.
///
/// Bash's children do not inherit the trap, so a compound element -- a
/// brace group, a subshell -- contributes nothing and the elements that
/// do are announced by the parent before any of them starts. Recording
/// each element's line is not only for the action: it is also what leaves
/// `$LINENO` on the pipeline for an `ERR` action to read afterwards.
// [spec:nsh:req:compat.bash.traps-introspection]
fn run_pipeline_debug_traps(shell: &mut Shell, pipeline: &Pipeline) -> Result<Flow, Error> {
    for command in &pipeline.commands {
        if let Node::Command(simple) = command {
            flow!(repeat_debug_trap(shell, simple.line));
        }
    }
    Ok(Flow::Done(shell.status))
}

/*
 * Evaluate a pipeline.  All the processes in the pipeline are children
 * of the process creating the pipeline.  (This differs from some versions
 * of the shell, which make the last process in a pipeline the parent
 * of all the rest.)
 */

// [spec:dash:sem:eval.evalpipe-fn]
// [spec:posix:req:cmd.pipeline-connects-stdio]
// [spec:posix:req:cmd.pipeline-assignment-precedes-redirection]
// [spec:posix:req:cmd.pipeline-foreground-wait]
// [spec:posix:req:cmd.pipeline-exit-status]
// [spec:posix:req:cmd.pipeline-pipefail-setting-at-start]
// [spec:nsh:req:idiom.no-raw-fd-core]
fn evaluate_pipeline(
    shell: &mut Shell,
    pipeline: &Pipeline,
    context: EvaluationContext,
) -> Result<Flow, Error> {
    let context = context.with_exit();
    flow!(run_pipeline_debug_traps(shell, pipeline));

    enum PipelineStart<'a> {
        Parent(ExitStatus),
        Child {
            command: &'a Node,
            input: Option<Descriptor>,
            output: Option<Descriptor>,
        },
        Control(Flow),
    }

    let start = crate::error::with_interrupts_deferred(shell, |shell| {
        let job_id = crate::jobs::create_job(shell, pipeline.commands.len());
        let mut previous = None;
        for (index, command) in pipeline.commands.iter().enumerate() {
            let has_next = index + 1 < pipeline.commands.len();
            match prepare_command_hash(shell, command)? {
                Flow::Done(_) => {}
                control => return Ok(PipelineStart::Control(control)),
            }
            let mut pipe = if has_next {
                Some(crate::redirection::create_pipe(shell, false)?.0)
            } else {
                None
            };
            if matches!(
                crate::jobs::fork_shell(
                    shell,
                    Some(job_id),
                    Some(command),
                    if pipeline.background {
                        ForkMode::Background
                    } else {
                        ForkMode::Foreground
                    },
                )?,
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
            crate::jobs::wait_for_job(shell, Some(job_id))?
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
                crate::input::reset_input(shell);
                shell
                    .descriptors
                    .install_owned(LogicalDescriptor::STDIN, input)
                    .map_err(|error| {
                        crate::redirection::descriptor_error(shell, LogicalDescriptor::STDIN, error)
                    })?;
            }
            if let Some(output) = output {
                shell
                    .descriptors
                    .install_owned(LogicalDescriptor::STDOUT, output)
                    .map_err(|error| {
                        crate::redirection::descriptor_error(
                            shell,
                            LogicalDescriptor::STDOUT,
                            error,
                        )
                    })?;
            }
            /* In a forked child, which may not return through the
             * parent's frames; see `evalsubshell`. */
            let outcome = evaluate_tree_without_exit(shell, Some(command), context);
            crate::runtime::exit_from_child(shell, outcome);
        }
    }
}

/*
 * Execute a command inside back quotes.  If it's a builtin command, we
 * want to save its output in a block obtained from malloc.  Otherwise
 * we fork off a subprocess and get the output of the command via a pipe.
 * Should be called with interrupts off.
 */

// [spec:dash:sem:eval.evalbackcmd-fn]
pub fn evaluate_command_substitution(
    shell: &mut Shell,
    node: Option<&Node>,
    result: &mut CommandSubstitution,
) -> Result<(), Error> {
    let job_id: JobId;

    result.descriptor = None;
    result.job_id = None;
    'substitution_setup: {
        if node.is_none() {
            break 'substitution_setup;
        }

        let pipe = crate::redirection::create_pipe(shell, false)?.0;
        job_id = crate::jobs::create_job(shell, 1);
        if matches!(
            crate::jobs::fork_shell(shell, Some(job_id), node, ForkMode::WithoutJob)?,
            nsh_platform::ForkResult::Child
        ) {
            crate::error::clear_interrupt_deferral(&mut shell.interrupt_deferral);
            drop(pipe.read);
            shell
                .descriptors
                .install_owned(LogicalDescriptor::STDOUT, pipe.write)
                .map_err(|error| {
                    crate::redirection::descriptor_error(shell, LogicalDescriptor::STDOUT, error)
                })?;
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
            let outcome = evaluate_tree_without_exit(shell, node, EvaluationContext::EXITING);
            crate::runtime::exit_from_child(shell, outcome);
        }
        drop(pipe.write);
        result.descriptor = Some(pipe.read);
        result.job_id = Some(job_id);
    }
    // out:
    Ok(())
}

// [spec:dash:sem:eval.fill-arglist-fn]
//
// The C's `argpp` is a `union node **` cursor walking `narg.next`; the
// argument list is a slice now, so the cursor is the unconsumed tail of it.
// The return value is the C's `*lastp`: the first entry this call appended,
// or NULL if the argument list ran out without producing one. As an index it
// is the length the list had on entry, so the answer is `Some` exactly when
// the list grew.
fn append_expanded_arguments<'a>(
    shell: &mut Shell,
    expanded_fields: &mut ExpandedFields,
    remaining_argument_nodes: &mut &'a [Node],
    held: &mut Vec<bash_arrays::Declaration<'a>>,
) -> Result<Option<usize>, Error> {
    let initial_field_count = expanded_fields.fields.len();

    while let Some((argument, rest)) = remaining_argument_nodes.split_first() {
        bash_arrays::expand_command_argument(
            shell,
            argument,
            expanded_fields,
            ExpansionMode::SPLIT | ExpansionMode::TILDE,
            held,
        )?;
        *remaining_argument_nodes = rest;
        if expanded_fields.fields.len() != initial_field_count {
            break;
        }
    }

    if expanded_fields.fields.len() != initial_field_count {
        Ok(Some(initial_field_count))
    } else {
        Ok(None)
    }
}

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
fn parse_command_args<'a>(
    shell: &mut Shell,
    expanded_fields: &mut ExpandedFields,
    remaining_argument_nodes: &mut &'a [Node],
    path: &mut Option<BString>,
    standard_path: &BStr,
    head: &mut usize,
    held: &mut Vec<bash_arrays::Declaration<'a>>,
) -> Result<Option<CommandSearch>, Error> {
    let mut argument_index = *head;

    loop {
        /* `sp = sp->next ? sp->next : fill_arglist(arglist, argpp)` */
        argument_index = if argument_index + 1 < expanded_fields.fields.len() {
            argument_index + 1
        } else {
            match append_expanded_arguments(shell, expanded_fields, remaining_argument_nodes, held)?
            {
                Some(field_index) => field_index,
                None => return Ok(None),
            }
        };
        let word = expanded_fields.fields[argument_index].as_bstr();
        if word.first() != Some(&b'-') {
            break;
        }
        let options = &word[1..];
        if options.is_empty() {
            break;
        }
        if options == b"-" {
            if argument_index + 1 >= expanded_fields.fields.len()
                && append_expanded_arguments(
                    shell,
                    expanded_fields,
                    remaining_argument_nodes,
                    held,
                )?
                .is_none()
            {
                return Ok(None);
            }
            argument_index += 1;
            break;
        }
        for &option in options.as_bytes() {
            match option {
                b'p' => {
                    *path = Some(standard_path.to_owned());
                }
                _ => {
                    /* run 'typecmd' for other options */
                    return Ok(None);
                }
            }
        }
    }

    *head = argument_index;
    Ok(Some(CommandSearch::DEFAULT.skipping_functions()))
}

/*
 * Execute a simple command.
 */

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
fn evaluate_command(
    shell: &mut Shell,
    command: &SimpleCommand,
    context: EvaluationContext,
) -> Result<Flow, Error> {
    crate::resource::with_resources(shell, |shell, resources| {
        evaluate_command_in_scope(shell, command, context, resources)
    })
}

fn evaluate_command_in_scope(
    shell: &mut Shell,
    command: &SimpleCommand,
    context: EvaluationContext,
    resources: &mut crate::resource::ResourceScope,
) -> Result<Flow, Error> {
    let mut remaining_arguments: &[Node];
    /* A declaration built-in's `a=(1 2)` operand cannot be expanded into
     * a word: the value is structural, and its kind depends on the
     * attributes the built-in has yet to apply. The name goes into the
     * argument list and the assignment waits here. */
    let mut held_declarations = Vec::new();
    let mut expanded_fields = ExpandedFields::new();
    let mut assignment_fields = ExpandedFields::new();
    let mut argument_count: usize;
    /* The C's `arglist.list`, which `parse_command_args` moves past the
     * `command [-p]` words while `osp` keeps the original head for `set -x`. */
    let mut head: usize = 0;
    let mut resolved_command = Command::Builtin(&crate::builtins::EMPTY_BUILTIN);
    let job_id: Option<JobId>;
    let mut path: Option<BString> = None;
    let standard_path = crate::variables::default_path();
    let mut special_builtin: Option<bool>;
    let mut command_search: CommandSearch;
    let mut is_exec_builtin: bool;
    let mut status: ExitStatus;
    let mut variable_attributes: VariableAttributes;
    let mut use_local_variables: bool;
    let mut command_control: Option<Flow> = None;

    record_command_line(shell, command.line);
    // [spec:nsh:req:compat.bash.traps-introspection]
    flow!(crate::trap::bash::run_debug(shell));
    if command
        .assignments
        .iter()
        .chain(command.arguments.iter())
        .any(|node| {
            matches!(node, Node::Bash(inner)
                if !matches!(inner, crate::nodes::BashNode::ArrayAssignment(_)))
        })
    {
        return Err(shell
            .diagnostics()
            .shell_error(b"Bash syntax is parsed but not executable yet"));
    }

    /* First expand the arguments. */
    shell.evaluation.command_substitution_status = ExitStatus::SUCCESS;

    command_search = CommandSearch::DEFAULT;
    is_exec_builtin = false;
    special_builtin = None;
    variable_attributes = VariableAttributes::NONE;
    use_local_variables = false;
    argument_count = 0;
    remaining_arguments = command.arguments.as_slice();
    let original_fields_start = append_expanded_arguments(
        shell,
        &mut expanded_fields,
        &mut remaining_arguments,
        &mut held_declarations,
    )?;
    if original_fields_start.is_some() {
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
            let active_path = crate::variables::path_value(shell);
            match find_command(
                shell,
                expanded_fields.fields[head].as_bstr(),
                &mut resolved_command,
                command_search.regular_builtins_only(),
                BStr::new(active_path.as_slice()),
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
            is_exec_builtin = builtin.id() == BuiltinId::Exec;
            if builtin.id() != BuiltinId::Command {
                break;
            }

            let Some(next_search) = parse_command_args(
                shell,
                &mut expanded_fields,
                &mut remaining_arguments,
                &mut path,
                standard_path.as_slice().as_bstr(),
                &mut head,
                &mut held_declarations,
            )?
            else {
                break;
            };
            command_search = next_search;
        }

        for argument in remaining_arguments {
            let mode = if assignments_are_arguments
                && matches!(
                    argument,
                    Node::Word(word) if word.word.is_assignment(&shell.locale)
                ) {
                ExpansionMode::ASSIGNMENT_TILDE
            } else {
                ExpansionMode::SPLIT | ExpansionMode::TILDE
            };
            bash_arrays::expand_command_argument(
                shell,
                argument,
                &mut expanded_fields,
                mode,
                &mut held_declarations,
            )?;
        }

        argument_count = expanded_fields.fields.len() - head;

        if is_exec_builtin && argument_count > 1 {
            variable_attributes = VariableAttributes::EXPORTED;
        }
    }

    resources.begin_local_variables(shell, use_local_variables);

    let last_argument_index = if shell.options.enabled(ShellOption::Interactive)
        && shell.evaluation.function_line == 0
        && argument_count > 0
    {
        Some(expanded_fields.fields.len() - 1)
    } else {
        None
    };

    let stderr = shell.descriptors.slot(LogicalDescriptor::STDERR);
    shell.io.previous_stderr().set_destination(stderr);
    let expanded_redirections = expand_redirections(shell, &command.redirections)?;
    /* `status = redirectsafe(..)`, which the C computes as `setjmp(..) *
     * 2`. The value is kept as well as the status, because `bail:` below
     * re-raises it when the command is a special built-in — that is the
     * one place a redirection error is *not* swallowed, and an `int`
     * cannot be re-raised. */
    let mut redirection_error: Option<Error> = None;
    match resources.apply_redirections(shell, &expanded_redirections) {
        /* Same as compound-redirection evaluation: an interrupt leaves rather than
         * becoming this command's status. */
        Err(error) if error.is_interrupt() || error.is_expansion() => return Err(error),
        Err(error) => {
            /* From the value; see the compound-redirection arm. Read before the move
             * into `redir_err`, which is where it is re-raised from. */
            status = error.status();
            redirection_error = Some(error);
        }
        Ok(()) => status = ExitStatus::SUCCESS,
    }

    'command_done: {
        'abort_command: {
            if !status.success() {
                break 'abort_command;
            }

            for assignment in &command.assignments {
                // A structural array assignment is applied whole; it has
                // no single expanded field to hand the scalar path.
                if let Node::Bash(crate::nodes::BashNode::ArrayAssignment(array)) = assignment {
                    bash_arrays::assign(
                        shell,
                        array,
                        crate::variables::arrays::ReadOnlyGuard::Enforce,
                    )?;
                    continue;
                }
                let assignment_index = assignment_fields.fields.len();
                crate::expand::expand_argument(
                    shell,
                    assignment,
                    Some(&mut assignment_fields),
                    ExpansionMode::ASSIGNMENT_TILDE,
                )?;
                /* `(*spp)->text` with no null check: EXP_VARTILDE has no
                 * EXP_FULL, so `expandarg` appended exactly one entry. */
                debug_assert_eq!(
                    assignment_fields.fields.len(),
                    assignment_index + 1,
                    "an unsplit expansion is one field"
                );

                if use_local_variables {
                    crate::variables::make_local_bytes(
                        shell,
                        assignment_fields.fields[assignment_index].as_bstr(),
                        VariableAttributes::EXPORTED,
                    )?;
                } else {
                    crate::variables::set_assignment_bytes(
                        shell,
                        assignment_fields.fields[assignment_index].as_bstr(),
                        variable_attributes,
                    )?;
                }
            }

            /* Print the command if xflag is set. */
            if shell.options.enabled(ShellOption::Xtrace)
                && !shell.evaluation.expanding_trace_prompt
            {
                let mut already_printed: bool;

                /* This block is why `Dest` exists. It used to open with
                 * `out = previous_stderr()` and then hold that pointer
                 * across `ps4val(sh)`, `expandstr(sh, ..)` and two
                 * `eprintlist` calls — five reborrows of the shell with a
                 * raw pointer into its I/O still live. Sound while the
                 * pointer came from a static; undefined the moment it
                 * comes from `&mut sh.io`. Naming the destination defers
                 * the resolution to each write, so nothing spans a call. */
                let dest = OutputDestination::PreviousStderr;
                shell.evaluation.expanding_trace_prompt = true;
                /* Hoisted out of `expandstr`'s argument list; see the
                 * note in `evalcommand`. */
                let ps4 = crate::variables::trace_prompt_value(shell);
                let prompt = crate::parser::expand_string(shell, BStr::new(ps4.as_slice()))?;
                shell.write_output(dest, &prompt)?;
                shell.evaluation.expanding_trace_prompt = false;
                already_printed = false;
                already_printed =
                    write_trace_fields(shell, dest, &assignment_fields.fields, already_printed)?;
                /* `eprintlist(sh, out, osp, sep)` prints from the *original*
                 * head, so `command -p foo` traces as it was written and not
                 * as `parse_command_args` left it.  A NULL `osp` prints
                 * nothing, which is the empty slice. */
                write_trace_fields(
                    shell,
                    dest,
                    &expanded_fields.fields
                        [original_fields_start.unwrap_or(expanded_fields.fields.len())..],
                    already_printed,
                )?;
                shell.write_output(dest, b"\n")?;
            }

            /* Now locate the command. */
            if !matches!(
                &resolved_command,
                Command::Builtin(builtin) if builtin.attributes().is_regular()
            ) {
                if path.is_none() {
                    path = Some(crate::variables::path_value(shell));
                }
                let search_path =
                    BStr::new(path.as_ref().expect("command lookup has a PATH").as_slice());
                let command_name = expanded_fields.fields[head].as_bstr();
                match find_command(
                    shell,
                    command_name,
                    &mut resolved_command,
                    command_search.reporting_errors(),
                    search_path,
                )? {
                    Flow::Done(_) => {}
                    exit @ Flow::Exit { .. } => return Ok(exit),
                    control => {
                        command_control = Some(control);
                        break 'command_done;
                    }
                }
            }

            job_id = None;

            /* Execute the command. */
            match resolved_command {
                Command::Unknown => {
                    status = ExitStatus::NOT_FOUND;
                    break 'abort_command;
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
                    match evaluate_builtin(
                        shell,
                        builtin,
                        &mut expanded_fields.fields[head..],
                        context,
                    ) {
                        Ok(flow) => {
                            if let Err(exit) = capture_local_control(flow, &mut command_control) {
                                return Ok(exit);
                            }
                        }
                        Err(error) => {
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
                            if builtin_error_is_fatal(
                                shell,
                                special_builtin.unwrap_or(false),
                                &error,
                            ) {
                                return Err(error);
                            }
                            /* Reported already, and `evalbltin`'s epilogue
                             * has run. The status it took travels in the
                             * error, so this frame -- the one that catches
                             * it -- is the one that writes it. It reaches
                             * `status` through `waitforjob(sh, None)`,
                             * which returns `exitstatus` when there is no
                             * job; `bail:` does not touch it on this path
                             * because the C reaches `out:` here. */
                            shell.status = error.status();
                            drop(error);
                        }
                    }
                }

                Command::Function(function) => {
                    /* `if (evalfun(..)) goto raise;` -- a function body is
                     * not a builtin, so there is nothing to swallow: both an
                     * exit and a diagnostic leave through this frame. */
                    let args = crate::builtins::args(&expanded_fields.fields[head..]);
                    if let Err(exit) = capture_local_control(
                        evaluate_function(shell, &function, &args, context)?,
                        &mut command_control,
                    ) {
                        return Ok(exit);
                    }
                }

                Command::External { path_index } => {
                    shell.flush_input();
                    let args = crate::builtins::args(&expanded_fields.fields[head..]);

                    /* Fork off a child process if necessary. */
                    if !context.exits() || crate::trap::has_traps(shell) {
                        let syntax = Node::Command(command.clone());
                        status = crate::error::with_interrupts_deferred(shell, |shell| {
                            let job = crate::jobs::fork_and_execute(
                                shell,
                                &syntax,
                                &args,
                                BStr::new(
                                    path.as_ref()
                                        .expect("external command has a PATH")
                                        .as_slice(),
                                ),
                                path_index,
                            )?;
                            crate::jobs::wait_for_job(shell, Some(job))
                        })?;
                        crate::error::clear_interrupt_deferral(&mut shell.interrupt_deferral);
                        break 'command_done;
                    } else {
                        /* `shellexec` replaces the process image or fails;
                         * failing, it reports and is the C's EXEND. */
                        return execute_external_command(
                            shell,
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

            /* The attributes exist now, so the structural values the
             * declaration was written with can finally land. A
             * declaration that failed -- an unknown option, a name it
             * may not touch -- stores nothing, and has already said so. */
            if shell.status.success() {
                bash_arrays::apply_declarations(shell, &held_declarations)?;
            }
            status = crate::jobs::wait_for_job(shell, job_id)?;
            crate::error::clear_interrupt_deferral(&mut shell.interrupt_deferral);
            break 'command_done;
        }
        // bail:
        /* A redirection-only command has no builtin entry whose specialness
         * can classify the failure. The adopted Smoosh contract uses the
         * shell-error status 1 for that path; this is the foreground half of
         * the parsed `exec 9&<-` case. */
        // [spec:nsh:req:compat.smoosh.error-contracts]
        status = redirection_only_status(
            status,
            redirection_error.as_ref(),
            original_fields_start.is_some(),
        );
        shell.status = status;

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
            let error = match redirection_error.take() {
                Some(error) => {
                    debug_assert_eq!(
                        error.status(),
                        status,
                        "a redirection error keeps its status"
                    );
                    error
                }
                None => crate::error::Error::reported(shell.evaluation.diagnostic_line, status),
            };
            debug_assert!(
                !error.is_expansion(),
                "expansion errors bypass redirection status"
            );
            // Smoosh's adopted POSIX closure profile assigns status 1 to a
            // redirection failure on a directly invoked special builtin.
            // Its diagnostic was already written by the redirection layer.
            // [spec:nsh:req:compat.smoosh.error-contracts]
            shell.status = ExitStatus::FAILURE;
            return Err(crate::error::Error::reported(
                shell.evaluation.diagnostic_line,
                1,
            ));
        }

        // goto out
    }
    // out:
    if is_exec_builtin {
        resources.retain_redirections(shell);
    }
    resources.restore(shell);
    if let Some(last_argument_index) = last_argument_index {
        /* dsl: I think this is intended to be used to support
         * '_' in 'vi' command mode during line editing...
         * However I implemented that within libedit itself.
         */
        crate::variables::set_bytes(
            shell,
            BStr::new(b"_"),
            Some(expanded_fields.fields[last_argument_index].as_bstr()),
            VariableAttributes::NONE,
        )?;
    }

    Ok(command_control
        .unwrap_or(Flow::Done(status))
        .with_status(status))
}

// [spec:dash:sem:eval.evalbltin-fn]
fn evaluate_builtin(
    shell: &mut Shell,
    builtin: &'static BuiltinSpec,
    fields: &mut [ExpandedField],
    context: EvaluationContext,
) -> Result<Flow, Error> {
    let saved_command_name = core::mem::take(&mut shell.evaluation.command_name);
    /* `commandname = argv[0]`, and NULL for the command that has no word
     * at all -- the assignment-only one `bltin` stands for. */
    shell.evaluation.command_name = fields.first().map(|field| BString::from(field.as_bstr()));

    let outcome = (|| -> Result<Flow, Error> {
        let command_flow = match builtin.handler() {
            BuiltinHandler::History => crate::builtins::fc::run_fields(shell, fields)?,
            BuiltinHandler::Eval => {
                let args = crate::builtins::args(fields);
                crate::builtins::eval::evaluate_arguments(shell, &args, context)?
            }
            BuiltinHandler::Standard(entry) => {
                let args = crate::builtins::args(fields);
                entry(shell, &args)?
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
        if shell.io.flush_all().is_err() {
            // [spec:nsh:req:compat.smoosh.error-contracts]
            shell.diagnostics().command_warning(b"I/O error");
            status = ExitStatus::ERROR;
        }
        shell.status = status;
        Ok(command_flow.with_status(status))
    })();

    // cmddone:
    /* The C's epilogue, and the reason it armed a handler at all: an
     * exception raised *beneath* a built-in had to run `freestdout` and
     * restore `commandname` on its way out rather than skip them. It runs
     * on every path here because there is only one way out now. `handler`
     * was the third thing it restored and there is no handler left. */
    shell.io.reset_stdout();
    shell.evaluation.command_name = saved_command_name;

    outcome
}

// [spec:dash:sem:eval.evalfun-fn]
// [spec:posix:req:cmd.function-invocation-positional-parameters]
// [spec:posix:req:cmd.function-return]
// [spec:posix:req:cmd.function-exit-status]
// [spec:posix:req:cmd.function-syntax-error-properties]
fn evaluate_function(
    shell: &mut Shell,
    function: &FunctionDefinition,
    args: &[&BStr],
    context: EvaluationContext,
) -> Result<Flow, Error> {
    /* `saveparam = shellparam` plus the `shellparam.malloc = 0` that the C
     * puts inside the protected region so the epilogue's `freeparam` cannot
     * reach what the copy still points at. */
    let saved_parameters = crate::options::take_positional_parameters(shell);
    let saved_function_line = shell.evaluation.function_line;
    let saved_loop_depth = shell.evaluation.loop_depth;

    crate::error::with_interrupts_deferred(shell, |shell| {
        /* Command lookup cloned the owned body, so redefining this function
         * while it runs cannot pull the body out from under this call. */
        shell.evaluation.function_line = function.line;
        // [spec:nsh:req:compat.smoosh.nonlexical-control]
        // Ordinarily only loops lexically inside the function are visible.
        // The explicit extension preserves the caller's dynamic loop depth so
        // break/continue can leave through this frame and be consumed there.
        if !shell.options.enabled(ShellOption::NonLexicalControl) {
            shell.evaluation.loop_depth = 0;
        }
    });
    crate::options::set_positional_parameters(shell, args.get(1..).unwrap_or_default());

    /* The call line is read before the body runs, because that is what
     * `BASH_LINENO[0]` names: where the call was written, not where the
     * body is. */
    // [spec:nsh:req:compat.bash.traps-introspection]
    let call_line = shell.variables.line_number;
    crate::variables::call_stack::push_function(shell, function.name.as_bstr(), call_line);
    let suppressed = crate::trap::bash::suppress_uninherited(shell);

    /* The frame `evalcommand` already pushed is the one a declaration in
     * this body must save into; a declaration built-in pushes another. */
    // [spec:nsh:req:compat.bash.functions-scoping]
    let outcome = crate::variables::nameref::with_function_scope(shell, |shell| {
        evaluate_tree(shell, Some(function.body.as_ref()), context.tested_only())
    });

    /* Bash's `RETURN` action belongs to the frame that is finishing, so
     * it runs while that frame is still innermost and while the body's
     * own view of the trap table is still installed -- which is what
     * makes `functrace` decide whether it runs at all. */
    // [spec:nsh:req:compat.bash.traps-introspection]
    let outcome = match outcome {
        Ok(flow) => crate::trap::bash::run_return(shell).map(|action| match action {
            Flow::Done(_) => flow,
            control => control,
        }),
        failed => failed,
    };
    crate::trap::bash::restore(shell, suppressed);
    crate::variables::call_stack::pop(shell);

    // funcdone:
    crate::error::with_interrupts_deferred(shell, |shell| {
        shell.evaluation.loop_depth = saved_loop_depth;
        shell.evaluation.function_line = saved_function_line;
        restore_caller_line(shell, call_line);
        crate::options::restore_positional_parameters(shell, saved_parameters);
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

// [spec:dash:sem:eval.prehash-fn]
fn prepare_command_hash(shell: &mut Shell, node: &Node) -> Result<Flow, Error> {
    let mut entry = Command::Unknown;

    if let Node::Command(command) = node
        && let Some(Node::Word(word)) = command.arguments.first()
        && crate::parser::is_valid_name(&shell.locale, word.word.as_bstr())
    {
        /* Hoisted out of the argument list; see the note in
         * `evalcommand`. */
        let path = crate::variables::path_value(shell);
        return find_command(
            shell,
            word.word.as_bstr(),
            &mut entry,
            CommandSearch::DEFAULT,
            BStr::new(path.as_slice()),
        );
    }
    Ok(Flow::Done((0).into()))
}

/// With `set -h`, remember literal command names while a function is
/// defined. This walks only command-bearing tree edges; words, redirection
/// operands and here-documents are not executed or expanded.
// [spec:nsh:req:compat.smoosh.hash-all]
fn prehash_tree(shell: &mut Shell, node: Option<&Node>) -> Result<Flow, Error> {
    let Some(node) = node else {
        return Ok(Flow::Done((0).into()));
    };

    match node {
        Node::Command(_) => return prepare_command_hash(shell, node),
        Node::Pipeline(pipeline) => {
            for command in &pipeline.commands {
                flow!(prehash_tree(shell, Some(command)));
            }
        }
        Node::Redirect(command) | Node::Background(command) | Node::Subshell(command) => {
            flow!(prehash_tree(shell, Some(command.command.as_ref())));
        }
        Node::And(binary)
        | Node::Or(binary)
        | Node::Sequence(binary)
        | Node::While(binary)
        | Node::Until(binary) => {
            flow!(prehash_tree(shell, Some(binary.left.as_ref())));
            flow!(prehash_tree(shell, Some(binary.right.as_ref())));
        }
        Node::If(conditional) => {
            flow!(prehash_tree(shell, Some(conditional.condition.as_ref())));
            flow!(prehash_tree(shell, Some(conditional.then_branch.as_ref())));
            flow!(prehash_tree(shell, conditional.else_branch.as_deref()));
        }
        Node::For(command) => {
            flow!(prehash_tree(shell, Some(command.body.as_ref())));
        }
        Node::Case(command) => {
            for clause in &command.clauses {
                flow!(prehash_tree(shell, clause.body.as_deref()));
            }
        }
        Node::Function(definition) => {
            flow!(prehash_tree(shell, Some(definition.body.as_ref())));
        }
        Node::Bash(crate::nodes::BashNode::Function(function)) => {
            flow!(prehash_tree(shell, Some(function.body.as_ref())));
        }
        Node::Not(command) => {
            flow!(prehash_tree(shell, Some(command.command.as_ref())));
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

// [spec:dash:sem:eval.eprintlist-fn]
fn write_trace_fields(
    shell: &mut Shell,
    dest: OutputDestination,
    list: &[ExpandedField],
    mut already_printed: bool,
) -> Result<bool, Error> {
    for field in list {
        let mut record = Vec::new();
        if already_printed {
            record.push(b' ');
        }
        record.extend_from_slice(field.as_bstr());
        already_printed = true;
        shell.write_output(dest, &record)?;
    }

    Ok(already_printed)
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
        let _guard = crate::test_support::lock();
        let mut shell = crate::context::Shell::builder().build().unwrap();
        crate::input::set_input_string(&mut shell, BStr::new(b": >\"$target\"\n"));
        let tree = match crate::parser::parse_command(&mut shell, false).unwrap() {
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

        crate::variables::set_bytes(
            &mut shell,
            BStr::new(b"target"),
            Some(BStr::new(b"one")),
            VariableAttributes::NONE,
        )
        .unwrap();
        let first = expand_redirections(&mut shell, &command.redirections).unwrap();
        assert!(matches!(
            &first[0],
            ExpandedRedirection::File { target, .. } if target == BStr::new(b"one")
        ));
        drop(first);

        crate::variables::set_bytes(
            &mut shell,
            BStr::new(b"target"),
            Some(BStr::new(b"two")),
            VariableAttributes::NONE,
        )
        .unwrap();
        let second = expand_redirections(&mut shell, &command.redirections).unwrap();
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
            Ok(Flow::Done(ExitStatus::from_code(
                i32::from(status.code()) + 100,
            )))
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
        let error = Error::Other {
            line: 3,
            status: ExitStatus::ERROR,
            message: bstr::BString::from(&b"nope"[..]),
        };
        let got = body(Err(error));
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
        let _guard = crate::test_support::lock();
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let shell = &mut owned;

        shell.status = ExitStatus::from_code(9);
        shell.evaluation.loop_depth = 3;
        shell.evaluation.expanding_trace_prompt = true;
        shell.clear_evaluation_resources();
        assert_eq!(shell.status, ExitStatus::from_code(9));
        assert_eq!(shell.evaluation.loop_depth, 0);
        assert!(!shell.evaluation.expanding_trace_prompt);
    }
}
