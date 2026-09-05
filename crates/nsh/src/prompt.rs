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

/// The name the hook is spelled with.
///
/// Bash's name, taken because a user's existing configuration already
/// spells it that way, and taken for the name alone: the rule says in as
/// many words that nothing here promises Bash's array form, its ordering
/// with `PS0`, or its `DEBUG` interactions.
const HOOK: &[u8] = b"PROMPT_COMMAND";

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
    shell.status = status;
    settle(shell, outcome)
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
