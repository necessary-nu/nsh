//! What the shell does between one command finishing and the next
//! prompt appearing.
//!
//! `docs/spec/nsh/interactive.md` §"A prompt a program generates" is the
//! whole of the design. The short version is that a prompt worth looking
//! at is computed from state only the shell holds, so the shell hands
//! that state to whatever computes it rather than making the computation
//! recover it from the outside.
//!
//! This sits at the command loop's boundary rather than at
//! [`crate::parser::render_prompt`], and that is not a convenience. The
//! renderer runs inside a parse, is reached for the continuation prompt
//! as well as the primary one, and is called again by `expandstr` while
//! it is expanding the prompt it just produced. A hook running there
//! would re-enter the parser with a half-read command unit still on the
//! input stack. The command loop's top is the one place a primary prompt
//! is about to be drawn and nothing else is in progress.

use bstr::{BStr, BString};

use crate::context::Shell;
use crate::error::Error;
use crate::evaluation::{EvaluationContext, Flow};
use crate::variables::VariableAttributes;

/// The name the hook is spelled with.
///
/// Bash's name, taken because a user's existing configuration already
/// spells it that way, and taken for the name alone: the rule says in as
/// many words that nothing here promises Bash's array form, its ordering
/// with `PS0`, or its `DEBUG` interactions.
const HOOK: &[u8] = b"PROMPT_COMMAND";

/// How many jobs the shell is tracking, as the hook reads it.
const JOBS: &[u8] = b"NSH_JOBS";

/// How long the last command record took, in milliseconds.
///
/// The unit is in the name because there is no convention to appeal to:
/// the reference has no name for this at all, and the shells that do
/// disagree (fish counts milliseconds, zsh's `$EPOCHREALTIME` arithmetic
/// counts seconds). A name that has to be looked up is worse than a long
/// one.
const DURATION: &[u8] = b"NSH_DURATION_MS";

/// What the shell measured around the last command, kept for the hook.
///
/// Beside [`crate::mail::MailState`] on the shell and for the same
/// reason: it is what the command loop remembers between one command and
/// the next prompt.
pub struct PromptState {
    /// Milliseconds the last *command* record took. A record that parsed
    /// to nothing does not replace it, which is the same distinction the
    /// command loop already draws for the status it carries forward.
    duration: u64,
}

impl PromptState {
    pub(crate) const fn new() -> Self {
        Self { duration: 0 }
    }
}

/// The shell's clock, read on both sides of a command record.
///
/// The rule requires the measurement to be taken *here*, by the shell,
/// around the command it reports -- and not by a hook sampling a clock at
/// each prompt. A hook runs after the fact and cannot see where the
/// pipeline began, so a shell built that way needs a second, pre-execution
/// hook as well, and still mis-times `slow | slow | fast`: the members of
/// a pipeline start together, so the moment any of them was reached says
/// nothing about when the pipeline did.
#[derive(Clone, Copy)]
pub(crate) struct Elapsed(f64);

impl Elapsed {
    /// The moment a command record is about to run, taken *after* the
    /// parse: the parse is where the shell waits for the person typing,
    /// and their thinking time is not the command's.
    pub(crate) fn started() -> Self {
        Self(nsh_platform::facts::monotonic_seconds())
    }
}

/// Charge the time since `started` to the record that has just finished.
///
/// Clamped at zero and saturated at the top, because the value crosses
/// into a shell variable and an integer that went backwards there would
/// be read as an enormous positive one.
pub(crate) fn record_duration(shell: &mut Shell, started: Elapsed) {
    let seconds = nsh_platform::facts::monotonic_seconds() - started.0;
    shell.prompt.duration = (seconds.max(0.0) * 1000.0) as u64;
}

/// Run the prompt hook, if there is one, before the primary prompt.
///
/// The result is a [`Flow`] so the command loop can propagate an explicit
/// `exit` from inside the hook. Everything else the hook can end with is
/// [`Flow::Done`]: a hook is not allowed to break the loop it runs at the
/// top of.
// [spec:nsh:req:interactive.prompt-hook]
pub(crate) fn run_hook(shell: &mut Shell) -> Result<Flow, Error> {
    let action = match crate::variables::lookup_bytes(shell, BStr::new(HOOK)) {
        Some(action) if !action.is_empty() => action,
        _ => return Ok(Flow::Done(shell.status)),
    };

    /* The status the last command left, which the hook reads as `$?` and
     * which the *next* command must still see. Nothing between here and
     * the restore below may leave a different one behind: a hook exists
     * to look at the status, and a shell whose prompt changed it would
     * make `cmd; echo $?` answer for the hook instead. That trap is why
     * starship's Bash integration needs a helper function at all --
     * Bash cannot assign `$?`, so the value has to be captured before
     * anything else in `PROMPT_COMMAND` runs. */
    let status = shell.status;

    /* Published for the length of the hook and taken away again, which is
     * what `[spec:nsh:req:interactive.prompt-state]` asks for and no more:
     * the state must be readable *by the hook*. Leaving the two names on
     * the table would put them in `declare -p`, and
     * `[spec:nsh:req:compat.bash.names.only-what-the-reference-has]`
     * forbids Bash mode publishing a name the reference does not have --
     * `bash_shell_facts::the_published_set_is_the_references_less_four`
     * is the check that would fail. */
    // [spec:nsh:req:interactive.prompt-state]
    let state = publish_state(shell);

    /* A diagnostic raised inside the hook is prefixed with the hook's
     * name, so a syntax error in `PROMPT_COMMAND` cannot be read as the
     * command the user just ran having failed. A command *within* the
     * hook still reports under its own name, because `evalcommand`
     * replaces this for the length of the call -- which is right: that
     * diagnostic is the inner command's own. */
    let outer_name = shell.evaluation.command_name.replace(BString::from(HOOK));

    /* In the shell's own execution environment, which is the entire
     * point: what the hook assigns is what the prompt expansion below it
     * then reads. A subshell would leave the assignment behind with the
     * child. */
    let outcome = crate::evaluation::evaluate_string(
        shell,
        BStr::new(action.as_slice()),
        EvaluationContext::DEFAULT,
    );

    shell.evaluation.command_name = outer_name;
    withdraw_state(shell, state);
    shell.status = status;
    settle(shell, outcome)
}

/// A name the shell lends the hook for the length of one run.
struct Lent {
    name: &'static [u8],
    /// What the name held before the loan and what it gets back.
    /// `None` means it held nothing and is unset again afterwards.
    previous: Option<BString>,
}

/// Give the hook the state a prompt generator needs, and say what has to
/// be put back.
///
/// The status is not among these: `$?` is already the answer to it, and
/// [`run_hook`] is what keeps that answer truthful across the call.
///
/// A read-only name is skipped rather than assigned through. Assigning
/// through one is refused *with a diagnostic*, and a session that printed
/// the same refusal above every prompt would be unusable; a name the user
/// has pinned is left holding what they pinned.
fn publish_state(shell: &mut Shell) -> Vec<Lent> {
    let jobs = shell.jobs.tracked().to_string();
    let duration = shell.prompt.duration.to_string();
    let mut lent = Vec::with_capacity(2);
    for (name, value) in [(JOBS, jobs), (DURATION, duration)] {
        if !writable(shell, BStr::new(name)) {
            continue;
        }
        let previous = crate::variables::lookup_bytes(shell, BStr::new(name));
        assign(shell, name, value.as_bytes());
        lent.push(Lent { name, previous });
    }
    lent
}

fn withdraw_state(shell: &mut Shell, lent: Vec<Lent>) {
    for Lent { name, previous } in lent {
        match previous {
            Some(value) => assign(shell, name, value.as_slice()),
            None => drop(crate::variables::unset_bytes(shell, BStr::new(name))),
        }
    }
}

/// Whether the shell may write through `name` without being refused.
fn writable(shell: &Shell, name: &BStr) -> bool {
    !crate::variables::variable_attributes(shell, name)
        .is_some_and(|attributes| attributes.read_only)
}

/// Land `value` on `name`, dropping a refusal the guard above did not
/// foresee -- a `local` declaration in the hook's own scope, say. The
/// hook then reads whatever the name does hold, which is the outcome the
/// prompt can live with.
fn assign(shell: &mut Shell, name: &[u8], value: &[u8]) {
    let assigned = crate::variables::set_bytes(
        shell,
        BStr::new(name),
        Some(BStr::new(value)),
        VariableAttributes::NONE,
    );
    drop(assigned);
}

/// What a finished hook leaves the command loop.
///
/// `exit 3` in a hook is a request, not a failure, and is honoured. A
/// *failure* -- a syntax error, a refused assignment, `set -e` acting on
/// a command inside the hook -- must not end the session, so it stops
/// here with its diagnostic already written. `Flow::END` is that class:
/// the `exit` builtin always names a status, so an unnamed one is
/// `errexit` or an `EV_EXIT` evaluation and never a deliberate `exit`.
///
/// An interrupt is the exception and propagates. It is the user's `^C`,
/// it belongs to the session rather than to the hook, and the loop above
/// recovers from one the same way it does anywhere else.
fn settle(shell: &Shell, outcome: Result<Flow, Error>) -> Result<Flow, Error> {
    match outcome {
        Ok(Flow::Exit {
            status: Some(status),
        }) => Ok(Flow::exit(status)),
        Ok(_) => Ok(Flow::Done(shell.status)),
        Err(error) if error.is_interrupt() => Err(error),
        Err(error) => {
            drop(error);
            Ok(Flow::Done(shell.status))
        }
    }
}
