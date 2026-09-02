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
mod command;
mod compound;
use command::evaluate_command;
use compound::{evaluate_case, evaluate_for, evaluate_loop};
mod bash_conditional;
pub(crate) mod bash_process_substitution;
mod last_pipe;
mod select;
mod timed;
mod trace;

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
    /// Whether the child this is running in was forked *for this command*,
    /// so the command is the child rather than something the child runs.
    ///
    /// Bash forks inside the code that runs an asynchronous simple command
    /// and a pipeline member, and the child then becomes the command --
    /// `execute_command_internal` never reaches the foot where `ERR` and
    /// `-e` read the status, and its `pipe_in == NO_PIPE && pipe_out ==
    /// NO_PIPE` guard says the same thing for a member that did not exec.
    /// There is no shell left above such a command to notice its failure.
    ///
    /// It describes one node and stops there: a function body, a group, a
    /// nested subshell are all commands the child *runs*, and each restores
    /// the fact rather than inheriting it. That is why it is not `tested`,
    /// which travels down into everything the syntax consumes.
    // [spec:nsh:req:compat.bash.traps-introspection]
    forked_as_this_command: bool,
}

impl EvaluationContext {
    pub(crate) const DEFAULT: Self = Self {
        exit: false,
        tested: false,
        forked_as_this_command: false,
    };
    pub(crate) const EXITING: Self = Self {
        exit: true,
        tested: false,
        forked_as_this_command: false,
    };
    pub(crate) const TESTED: Self = Self {
        exit: false,
        tested: true,
        forked_as_this_command: false,
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

    pub(crate) const fn tested_only(self) -> Self {
        Self {
            exit: false,
            tested: self.tested,
            forked_as_this_command: false,
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
    /// Names a declaration built-in refused in the command now running.
    ///
    /// `declare` applies each operand on its own: `declare a[ a[2]=3 ]=Y`
    /// stores the middle one and reports the other two. The structural
    /// operands land after the built-in returns, so which of them it
    /// refused has to survive the return.
    // [spec:nsh:req:compat.bash.arrays-declarations]
    pub(crate) refused_declarations: Vec<BString>,
    /// The array kind `export -a` or `readonly -A` asked for, waiting
    /// for the compound operand it applies to.
    ///
    /// Those two built-ins take `-a` and `-A` only to say how a compound
    /// value is to be read, and Bash consults the letter *only* then:
    /// `readonly -A m` with no value leaves `m` a plain read-only name.
    /// `declare` has no need of this because it applies its attributes
    /// to the operand it was written with; these two are handed the bare
    /// name and the value arrives after they have returned.
    // [spec:nsh:req:compat.bash.arrays-declarations]
    pub(crate) declared_kind: Option<crate::variables::value::VariableKind>,
    /// How many string re-entries into the evaluator are active.
    ///
    /// `eval`, a trap action and `fc -e` all parse a string and run it on
    /// top of the frame that asked for it, which is a stack frame per
    /// level exactly as a call is. The call stack counts calls and dot
    /// scripts; this counts the rest, and
    /// [`crate::variables::call_stack::evaluation_depth`] adds the two.
    // [spec:nsh:req:idiom.bounded-recursion]
    pub(crate) nested_evaluations: usize,
    /// Bytes of script text the live string re-entries are between them
    /// evaluating, which is the resource a depth alone cannot see.
    ///
    /// Depth counts levels; this counts what the levels are carrying.
    /// `eval eval ... echo hi` is 512 legitimate levels each re-parsing
    /// the text of the one below it, so the work is the product and only
    /// one of its factors is bounded. It sits beside
    /// [`Self::nested_evaluations`] because it is charged and released at
    /// the same two lines, by the same re-entry.
    // [spec:nsh:req:idiom.bounded-recursion]
    pub(crate) live_evaluation_bytes: usize,
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
            refused_declarations: Vec::new(),
            declared_kind: None,
            nested_evaluations: 0,
            live_evaluation_bytes: 0,
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
    /* `eval eval eval ... :` recursed here once per word and overflowed
     * the stack at about 3,500 levels, which dash and Bash both survive.
     * It is the same resource a call spends and it shares the same
     * ceiling; only the diagnostic differs, because there is no function
     * here to name. */
    // [spec:nsh:req:idiom.bounded-recursion]
    if crate::variables::call_stack::evaluation_depth(shell) >= MAX_EVALUATION_DEPTH {
        let mut message = b"Maximum recursion depth (".to_vec();
        message.extend_from_slice(MAX_EVALUATION_DEPTH.to_string().as_bytes());
        message.extend_from_slice(b") reached");
        return Err(shell.diagnostics().shell_error(&message));
    }
    /* The depth ceiling above stops the recursion and does not stop the
     * work: each of its 512 levels re-parses the text of the level below,
     * so `eval` repeated N times costs 512N and was killed for memory at
     * N = 100,000 long before any of it was refused. What every re-entry
     * has in hand, and no ancestor can free while it runs, is the text it
     * was asked to evaluate; the sum of those is the work the depth
     * cannot see, and it is what is bounded here. */
    // [spec:nsh:req:idiom.bounded-recursion]
    if shell.evaluation.live_evaluation_bytes + text.len() > MAX_EVALUATION_WORK {
        let mut message = b"Maximum evaluation size (".to_vec();
        message.extend_from_slice(MAX_EVALUATION_WORK.to_string().as_bytes());
        message.extend_from_slice(b" bytes) reached");
        return Err(shell.diagnostics().shell_error(&message));
    }
    shell.evaluation.nested_evaluations += 1;
    shell.evaluation.live_evaluation_bytes += text.len();
    let outcome = crate::resource::with_resources(shell, |shell, _resources| {
        crate::input::set_input_string(shell, text);
        parse_and_execute(shell, context)
    });
    shell.evaluation.live_evaluation_bytes -= text.len();
    shell.evaluation.nested_evaluations -= 1;
    outcome
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
            let command_status = flow!(evaluate_record(
                shell,
                command.as_ref(),
                command_context,
                true
            ));
            if command.is_some() {
                status = command_status;
            }
        }
        /* `popstackmark(&smark)` — one per parsed command, and one on the
         * way out. */
    }
    Ok(Flow::Done(status))
}

/// Evaluate one parsed input record, and resume at the next one after a
/// failure whose boundary is the record rather than the shell.
///
/// This is the resumption point for [`Error::Abandoned`], and it is the
/// right one because it is where Bash's is: `parse_and_execute` reads one
/// record, runs it, and its `setjmp` catches the `DISCARD` an expansion or
/// assignment failure raises, so the rest of *that* record is abandoned and
/// the next is read. Every caller is such a loop -- the interactive command
/// loop, a script or `-c` string, and the `eval`, dot and `source` frames
/// that parse newly supplied text. Bash recovers at all of them, and
/// observably: `readonly r=1; r=2; echo x` prints nothing while the same
/// three commands on three lines print `x`, and an `eval` whose text fails
/// leaves the caller's locals and its enclosing loop intact.
///
/// Nothing is unwound here on purpose. Every temporary the abandoned record
/// held is already released by the frame that owns it as the error passes
/// through: [`crate::resource::ResourceScope::restore`] runs on the error
/// path and takes redirections, input frames and local scopes with it,
/// `evaluate_loop` decrements its own depth, and `with_interrupts_deferred`
/// restores the previous deferral. Clearing those wholesale -- as the
/// interactive arm does, because a POSIX interactive shell really is
/// returning to its outermost loop -- would destroy the locals of a
/// function that merely called `eval`.
///
/// `errexit` overrides the recovery, and that is Bash's rule rather than an
/// interpretation of it: `set -e` makes Bash's `report_error` end the shell
/// where it stands. A script that asked to stop at the first error gets to
/// stop at this one, and the recovery cannot be the thing that swallows it.
///
/// `outermost` is false for a record of a `.` or `source` operand, which
/// recovers the same way but does not take the interactive arm -- that one
/// belongs to the loop a person is typing at. It is a different rule: a
/// POSIX expansion failure abandons the affected command and resumes at the
/// next `;` command, which is finer than a record and is what dash does.
///
/// No dialect test: [`Error::Abandoned`] is built only in Bash mode.
// [spec:nsh:req:compat.smoosh.error-contracts]
// [spec:nsh:req:compat.bash.error-boundary]
pub(crate) fn evaluate_record(
    shell: &mut Shell,
    node: Option<&Node>,
    context: EvaluationContext,
    outermost: bool,
) -> Result<Flow, Error> {
    let interactive_root =
        outermost && shell.options.enabled(ShellOption::Interactive) && shell.shell_level == 0;
    let outcome = if interactive_root {
        evaluate_interactive_sequence(shell, node, context)
    } else {
        evaluate_tree(shell, node, context)
    };
    match outcome {
        Err(error) if error.is_abandoned() && !shell.options.enabled(ShellOption::Errexit) => {
            let status = error.status();
            drop(error);
            shell.status = status;
            Ok(Flow::Done(status))
        }
        outcome => outcome,
    }
}

/// Record the line the command about to run is reported at.
///
/// dash reports `$LINENO` inside a function body relative to the
/// function's own first line; Bash reports the line in the file, which is
/// what `BASH_LINENO`, `caller`, and the `DEBUG` and `ERR` actions all
/// read. The subtraction is therefore the POSIX dialect's alone.
///
/// WHICH LINE A COMPOUND COMMAND PASSES IS THE PARSER'S ANSWER, not this
/// function's, and it is not the same line for every form. A subshell and
/// `(( ))` record the line their closing token is on, because that is
/// where Bash builds those nodes; `for`, `case`, `select`, `for ((;;))`
/// and `[[ ]]` record a line from inside themselves, because Bash hands
/// those nodes one. `bash_compound_command_line` is the measured table.
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

/// Whether a built-in's failure ends the shell rather than becoming the
/// command's status.
///
/// POSIX makes a special built-in's error fatal to a non-interactive shell.
/// Bash does not, outside its own POSIX mode, and a failure the dialect has
/// already classed as survivable must not be escalated by the frame that
/// catches it -- `readonly r=1; unset r; echo next` prints `next` there.
///
/// Specialness is the whole of what the dialect withdraws, and it is
/// withdrawn for the class rather than for the sites that thought to build
/// an [`Error::Abandoned`]: `unset -v 'a['`, `local x=1` outside a function
/// and a bad option to any of them are all ordinary command failures under
/// Bash, and the next command of the same list runs.
///
/// Three things are not that and stay fatal in Bash mode too. An expansion
/// error only crossed this frame on its way out of `eval` or `.` --
/// `eval ': ${x:?boom}'` ends both shells -- and the shell's own input
/// failing to read is unrecoverable by POSIX wherever it is noticed.
/// `break` and `continue` are the third, and they are Bash's own rule
/// rather than an exception to it: their count goes through
/// `get_numeric_arg`'s fatal flag, which ends the shell instead of
/// returning, so `while true; do break oops; done` stops there in Bash as
/// well. A status in place of that refusal would leave the loop that asked
/// to be left still running.
// [spec:nsh:req:compat.bash.error-boundary]
fn builtin_error_is_fatal(
    shell: &Shell,
    builtin: BuiltinId,
    special_builtin: bool,
    error: &Error,
) -> bool {
    if error.is_abandoned() {
        return false;
    }
    if error.is_interrupt() {
        return true;
    }
    if !special_builtin || shell.evaluation.signal_trap_depth != 0 {
        return false;
    }
    shell.options.dialect() != crate::options::Dialect::Bash
        || error.is_expansion()
        || error.is_unrecoverable_read()
        || matches!(builtin, BuiltinId::Break | BuiltinId::Continue)
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
    /* Every `<(list)` name this node's words open dies with the node. Bash
     * keeps them until the outermost command finishes, which is why a loop
     * body there opens one pipe per iteration and why an unrelated program
     * run in between inherits a descriptor that is none of its business.
     * `[dec:nsh:safety-trumps-compatibility]`: the narrower lifetime is the
     * safe one, and it is still wide enough for the node that built the name
     * -- a redirected group, a `for` list, a command -- to have finished
     * using it. The guard owns the stack rather than borrowing the shell, so
     * the body below still has the shell to itself. */
    // [spec:nsh:req:compat.bash.process-substitution]
    let _substitution_names = bash_process_substitution::scope(shell);
    let mut check_exit = false;
    let mut status = ExitStatus::SUCCESS;

    if !shell.options.enabled(ShellOption::NoExec)
        && let Some(node) = node
    {
        flow!(crate::trap::run_pending_traps(shell));
        shell.display_history = true;
        // [spec:nsh:req:idiom.structural-ast]
        status = match node {
            Node::Redirect(redirection) | Node::Group(redirection) => {
                record_command_line(shell, redirection.line.get());
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
                // [spec:nsh:req:compat.bash.builtins-special-variables]
                let status = flow!(evaluate_command(shell, command, context));
                crate::variables::special::set_pipeline_status(shell, &[status]);
                status
            }
            Node::For(command) => flow!(evaluate_for(shell, command, context)),
            Node::Select(command) => flow!(select::evaluate_select(shell, command, context)),
            Node::Timed(command) => flow!(timed::evaluate_timed(shell, command, context)),
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
            Node::And(_) | Node::Or(_) | Node::Sequence(_) => {
                flow!(evaluate_list(shell, node, context))
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
                    /* The same function under the other spelling, so it
                     * keeps the run it was actually written as. */
                    // [spec:nsh:def:idiom.token-stream]
                    tokens: function.tokens.clone(),
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
                record_command_line(shell, conditional.line.get());
                // [spec:nsh:req:compat.bash.traps-introspection]
                flow!(crate::trap::bash::run_debug(shell));
                bash_conditional::evaluate(shell, conditional)?
            }
            Node::Bash(crate::nodes::BashNode::ArithmeticCommand(arithmetic)) => {
                check_exit = true;
                record_command_line(shell, arithmetic.line.get());
                // [spec:nsh:req:compat.bash.traps-introspection]
                flow!(crate::trap::bash::run_debug(shell));
                bash_arithmetic::command(shell, arithmetic)?
            }
            Node::Bash(crate::nodes::BashNode::ArithmeticFor(loop_command)) => {
                flow!(bash_arithmetic::for_loop(shell, loop_command, context))
            }
            /* A process substitution is a word, not a command: it reaches
             * the evaluator through the expansion of the word that carries
             * it, in `expand::typed`. Arriving here means a tree the parser
             * does not build, so it is a shell error rather than a gap. */
            // [spec:nsh:req:compat.bash.process-substitution]
            Node::Bash(crate::nodes::BashNode::ProcessSubstitution(_)) => {
                return Err(shell
                    .diagnostics()
                    .shell_error(b"process substitution is not a command"));
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
     * syntax consumes is neither's business. One predicate, read twice.
     *
     * A command the shell forked a child *for* is neither's business
     * either, and for the same reason rather than a second one: Bash
     * replaces that process with the command, so nothing is left above it
     * to read the status back. */
    // [spec:nsh:req:compat.bash.traps-introspection]
    let acted_on_failure =
        check_exit && !context.is_tested() && !context.forked_as_this_command && !status.success();
    if acted_on_failure {
        flow!(crate::trap::bash::run_err(shell));
    }
    let abort_for_errexit = acted_on_failure && shell.options.enabled(ShellOption::Errexit);
    if !abort_for_errexit && !context.exits() {
        return Ok(Flow::Done(shell.status));
    }
    Ok(Flow::END)
}

/// Which operator joined an element of a list to the ones before it.
#[derive(Clone, Copy)]
enum ListJoin {
    And,
    Or,
    Sequence,
}

/* The parser reads a list one element at a time and hangs each new one off
 * what it already has, so `a; b; c` arrives as a chain leaning left that is
 * as deep as the line is long. Nothing bounds that depth: a nesting ceiling
 * charges constructs that nest, and a list nests nothing. A walk that spent
 * a frame per element would therefore overflow the stack on a list both
 * reference shells run, so the spine is collected and then run in the one
 * frame -- iteratively, the way the parser built it.
 *
 * Each element's context is settled on the way down with it, because it is
 * the joins to an element's *right* that say whether the shell may act on
 * its failure: `&&` and `||` consume their left side's status, a `;` passes
 * down whatever its own caller said, and the last element answers for the
 * list. `-e` reads that distinction, so it is carried rather than recomputed.
 */
// [spec:posix:req:cmd.and-or-precedence]
fn evaluate_list(
    shell: &mut Shell,
    node: &Node,
    context: EvaluationContext,
) -> Result<Flow, Error> {
    let mut joins = Vec::new();
    let mut element = node;
    let mut element_context = context;
    /* The join a list node makes, and the two sides it makes it between.
     * Anything else is the element the spine ends at. */
    while let Some((join, binary)) = match element {
        Node::And(binary) => Some((ListJoin::And, binary)),
        Node::Or(binary) => Some((ListJoin::Or, binary)),
        Node::Sequence(binary) => Some((ListJoin::Sequence, binary)),
        _ => None,
    } {
        joins.push((join, binary.right.as_ref(), element_context));
        element_context = match join {
            ListJoin::Sequence => element_context.tested_only(),
            ListJoin::And | ListJoin::Or => EvaluationContext::TESTED,
        };
        element = binary.left.as_ref();
    }

    let mut status = flow!(evaluate_tree(shell, Some(element), element_context));
    for (join, element, element_context) in joins.into_iter().rev() {
        let short_circuited = match join {
            ListJoin::And => !status.success(),
            ListJoin::Or => status.success(),
            // A sequence's observable status is its right-hand command.
            ListJoin::Sequence => false,
        };
        if !short_circuited {
            status = flow!(evaluate_tree(shell, Some(element), element_context));
        }
    }
    Ok(Flow::Done(status))
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

/*
 * Kick off a subshell to evaluate a tree.
 */

// [spec:dash:sem:eval.evalsubshell-fn]
// [spec:posix:req:jobctl.list-splitting]
/// The context a subshell's child runs the body under.
///
/// The child runs the body rather than the wrapper, so "forked as this
/// command" is restored here, and then set again only where Bash's own
/// fork leaves the body with no shell above it to read a status back. A
/// simple command and a pipeline are not control structures, so Bash
/// forks an asynchronous one from inside the code that runs it and the
/// child becomes the command. A subshell is the same story and not only
/// when asynchronous: entering one, Bash runs its *body*, so the subshell
/// node is never a command anything noticed the failure of. Everything
/// else -- a group, a loop, a `case`, `(( ))`, `[[ ]]` -- is a control
/// structure Bash runs inside the child it forked first, where the
/// failure is noticed as usual: `(( 0 )) &` raises `ERR` where `false &`
/// does not.
// [spec:nsh:req:compat.bash.traps-introspection]
fn subshell_child_context(
    context: EvaluationContext,
    body: &Node,
    background: bool,
) -> EvaluationContext {
    EvaluationContext {
        exit: context.exit,
        /* An asynchronous command's status is the child's own exit
         * status, not something the surrounding syntax consumed. */
        tested: context.tested && !background,
        forked_as_this_command: match body {
            Node::Subshell(_) => true,
            Node::Command(_) | Node::Pipeline(_) => background,
            _ => false,
        },
    }
}

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

    record_command_line(shell, command.line.get());

    let expanded_redirections = expand_redirections(shell, &command.redirections)?;
    /* Whether the tail below runs in a child of this process or in this
     * process. The structured scope restores the caller's interrupt depth
     * before either tail continues. */
    let forked = crate::error::with_interrupts_deferred(shell, |shell| {
        if !background && context.exits() && !crate::trap::has_traps(shell) {
            shell.prepare_fork_child(None);
            context = subshell_child_context(context, command.command.as_ref(), background);
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
            context =
                subshell_child_context(context.with_exit(), command.command.as_ref(), background);
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
                expanded.push(crate::redirection::expand_file_target(shell, redirection)?);
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
                    descriptor: redirection.descriptor.clone(),
                    source,
                });
            }
            Redirection::HereDocument(document) => {
                expanded.push(ExpandedRedirection::HereDocument(document));
            }
            /* `<<< word` expands once, without splitting or pathname
             * expansion, and the descriptor reads the result plus one
             * newline. */
            // [spec:nsh:req:compat.bash.expansion-globbing]
            Redirection::HereString(here) => {
                let word = Node::Word(here.word.clone());
                crate::expand::expand_argument(shell, &word, Some(&mut fnl), ExpansionMode::TILDE)?;
                debug_assert_eq!(fnl.fields.len(), 1, "an unsplit expansion is one field");
                let mut content = fnl.fields.remove(0).text;
                content.push(b'\n');
                expanded.push(ExpandedRedirection::HereString {
                    descriptor: here.descriptor.clone(),
                    content,
                });
            }
        }
    }
    Ok(expanded)
}

fn descriptor_source(shell: &mut Shell, text: &BStr) -> Result<Option<LogicalDescriptor>, Error> {
    // [spec:posix:req:redir.dup-output]
    if let Some(number) = LogicalDescriptor::from_digits(text) {
        Ok(Some(number))
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
            flow!(repeat_debug_trap(shell, simple.line.get()));
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
    /* A member with a pipe on either side is a command Bash forked a child
     * for, and its `pipe_in == NO_PIPE && pipe_out == NO_PIPE` guard keeps
     * `ERR` and `-e` off that member whether or not the child went on to
     * exec. The pipeline itself is still noticed, which is why `false |
     * false` raises once and `false | cat` not at all. A pipeline of one
     * has no pipe and is an ordinary command. */
    // [spec:nsh:req:compat.bash.traps-introspection]
    let context = if pipeline.commands.len() > 1 {
        EvaluationContext {
            forked_as_this_command: true,
            ..context
        }
    } else {
        context
    };
    let caller_context = context;
    let context = context.with_exit();
    flow!(run_pipeline_debug_traps(shell, pipeline));

    enum PipelineStart<'a> {
        Parent(ExitStatus),
        Child {
            command: &'a Node,
            input: Option<Descriptor>,
            output: Option<Descriptor>,
        },
        /// `shopt -s lastpipe`: the final stage stays in this shell.
        LastStage {
            command: &'a Node,
            input: Option<Descriptor>,
            job_id: JobId,
        },
        Control(Flow),
    }

    let keep_last_stage = last_pipe::applies(shell, pipeline);
    let start = crate::error::with_interrupts_deferred(shell, |shell| {
        let job_id = crate::jobs::create_job(shell, pipeline.commands.len());
        let mut previous = None;
        for (index, command) in pipeline.commands.iter().enumerate() {
            let has_next = index + 1 < pipeline.commands.len();
            if !has_next && keep_last_stage {
                return Ok(PipelineStart::LastStage {
                    command,
                    input: previous.take(),
                    job_id,
                });
            }
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
        PipelineStart::LastStage {
            command,
            input,
            job_id,
        } => last_pipe::run(shell, command, input, job_id, caller_context),
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
/// How deeply the evaluator may re-enter itself before it refuses.
///
/// Bash leaves this unbounded unless `FUNCNEST` is set and segfaults on
/// `f() { f; }; f`; dash refuses at 1,000 and says so. Bash's crash is
/// the named unsafety in [dec:nsh:safety-trumps-compatibility] rather
/// than a behaviour to reproduce, so the shape here is dash's -- refuse,
/// with its wording, so a script that hits it reads the same diagnostic.
///
/// One ceiling for calls, dot scripts and `eval` together rather than
/// one each, because they compose: `f() { eval f; }` spends one of each
/// per turn, and separate ceilings would let it reach a depth neither
/// names. [`crate::variables::call_stack::evaluation_depth`] is what they
/// are all measured against.
///
/// The number is not dash's, and the arithmetic is why. A call costs
/// 1,952 bytes of stack in a release build and an `eval` 2,351, both
/// measured; dash's 1,000 calls would be 1.86 MiB, which does not fit
/// the 2 MiB a spawned Rust thread gets by default with anything to
/// spare. 512 is 0.95 MiB of calls, half the budget, or 1.15 MiB if
/// every level is an `eval` -- the same rule the parser's bound is set
/// by, which is to size it against the smallest stack the shell can
/// plausibly be asked to run on rather than the largest. A debug build
/// costs 13,952 bytes a call, so the full depth wants 7 MiB there and
/// the tests say so where it matters.
///
/// The limit is observable, since the diagnostic names it, but it is not
/// a compatibility surface: no rule fixes it and it moves if the
/// per-level cost does.
// [spec:nsh:req:idiom.bounded-recursion]
pub(crate) const MAX_EVALUATION_DEPTH: usize = 512;

/// How much script text the live string re-entries may be carrying between
/// them, in bytes.
///
/// [`MAX_EVALUATION_DEPTH`] bounds one factor of a product. `eval` repeated
/// N times reaches the ceiling at 512 levels and each of them re-parses the
/// O(N)-word command that carried it, so the work is 512N and only the 512
/// is bounded: measured here, N = 8,000 refused correctly after 1.9 GB of
/// resident memory and N = 100,000 after 25.1 GB, which on a machine with
/// less to spare is the OOM kill this was filed for. Both reference shells
/// are worse on the same input -- neither bounds the depth at all, so both
/// pay N squared and both pass 8 GiB at N = 100,000 -- but a shell that
/// refuses must not spend more doing it than one that succeeds, so the
/// other factor is bounded too.
///
/// The charge is the text a re-entry is asked to evaluate, summed over the
/// re-entries that are live, and it is against *re-entry* rather than
/// against size on purpose. A generated script of a hundred thousand words
/// run once is ordinary and is not charged at all: a file is read one
/// command at a time, so nothing accumulates. The same text reached
/// through four hundred nested evaluations is four hundred copies alive at
/// once, and that is what this sees.
///
/// Set from measurement rather than from taste. Over 41,189 real script
/// cases -- every case of the Oils spec suites, the Smoosh suite and this
/// repository's own corpus -- the peak of this sum is 0 bytes at the
/// median, 7 bytes at the 99th centile and 20,004 bytes at its maximum,
/// which is itself an adversarial case in `aud_foundation_e.txt`. 8 MiB is
/// 419 times that maximum, and is far past any real `eval` payload: the
/// shell-initialisation blobs that motivate a large one are kilobytes.
///
/// What it buys is a ceiling on memory, because the retained parse is
/// about a hundred times the text it came from -- 61 MB to parse one
/// 500 KB command, measured. Eight mebibytes of live text is therefore
/// about 0.85 GB whatever N is, against 25.1 GB before, and below the
/// 1.52 GB the pinned Bash spends *succeeding* on the same input at
/// N = 4,000. Lowering it would buy proportionally less memory and cost
/// the same headroom; the hundredfold is the parse's own constant and is
/// a separate question from this bound.
///
/// Fixed, with no option and no variable, for the reason
/// [`MAX_EVALUATION_DEPTH`] is: a limit a script can raise is a control for
/// turning the safety off.
// [spec:nsh:req:idiom.bounded-recursion]
pub(crate) const MAX_EVALUATION_WORK: usize = 8 << 20;

fn evaluate_function(
    shell: &mut Shell,
    function: &FunctionDefinition,
    args: &[&BStr],
    context: EvaluationContext,
) -> Result<Flow, Error> {
    /* `saveparam = shellparam` plus the `shellparam.malloc = 0` that the C
     * puts inside the protected region so the epilogue's `freeparam` cannot
     * reach what the copy still points at. */
    /* A call spends a stack frame and a script may recurse without
     * meaning to, so the depth is bounded rather than trusted: `f() { f;
     * }; f` otherwise overflows the stack, which Bash does too and dash
     * does not. Bash's crash is not a behaviour to reproduce -- unbounded
     * resource consumption is the named unsafety in
     * [dec:nsh:safety-trumps-compatibility] -- so this follows dash and
     * refuses, at the same depth and with the same words. Every kind of
     * frame counts, because `.` inside a function nests the evaluator
     * exactly as another call does. */
    // [spec:nsh:req:idiom.bounded-recursion]
    if crate::variables::call_stack::evaluation_depth(shell) >= MAX_EVALUATION_DEPTH {
        let mut message = b"Maximum function recursion depth (".to_vec();
        message.extend_from_slice(MAX_EVALUATION_DEPTH.to_string().as_bytes());
        message.extend_from_slice(b") reached");
        return Err(shell.diagnostics().shell_error(&message));
    }
    let saved_parameters = crate::options::take_positional_parameters(shell);
    let saved_function_line = shell.evaluation.function_line;
    let saved_loop_depth = shell.evaluation.loop_depth;

    crate::error::with_interrupts_deferred(shell, |shell| {
        /* Command lookup took a handle on the body, so redefining this
         * function while it runs cannot pull the body out from under this
         * call: the table gets a new handle and this frame keeps the old
         * one. */
        shell.evaluation.function_line = function.line.get();
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
        Node::Redirect(command)
        | Node::Background(command)
        | Node::Subshell(command)
        | Node::Group(command) => {
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
        Node::For(command) | Node::Select(command) => {
            flow!(prehash_tree(shell, Some(command.body.as_ref())));
        }
        Node::Timed(command) => {
            flow!(prehash_tree(shell, command.command.as_deref()));
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

#[cfg(test)]
mod tests;
