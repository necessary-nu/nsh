//! Bash's `DEBUG`, `ERR` and `RETURN` pseudo-conditions.
//!
//! These are not signals: nothing raises them, no disposition is
//! installed for them, and the operating system never hears about them.
//! They are conditions the *evaluator* reaches, which is why they get a
//! table of their own rather than three more slots on the signal-indexed
//! one -- the arrays there are addressed by signal number and every loop
//! over them means "for each signal".
//!
//! Inheritance is the other half of the subset. A function body, a
//! subshell and a command substitution each start without these traps
//! unless `set -o functrace` (`DEBUG`, `RETURN`) or `set -o errtrace`
//! (`ERR`) says otherwise, so the suppression is a save-and-clear around
//! the nested evaluation rather than a test at the point of delivery.

use bstr::BStr;

use super::TrapAction;
use crate::context::Shell;
use crate::error::Error;
use crate::evaluation::{EvaluationContext, Flow};
use crate::options::{Dialect, ShellOption};

/// One Bash pseudo-condition.
// [spec:nsh:req:compat.bash.traps-introspection]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BashCondition {
    /// Before every simple command, `for`, `case`, `[[` and `((`.
    Debug,
    /// After a command whose failure `errexit` would act on.
    Err,
    /// When a function body or a dot script finishes.
    Return,
}

/// Every condition, in the order `trap` lists them.
pub(crate) const CONDITIONS: [BashCondition; 3] = [
    BashCondition::Debug,
    BashCondition::Err,
    BashCondition::Return,
];

impl BashCondition {
    const fn slot(self) -> usize {
        self as usize
    }

    /// The name `trap` accepts and prints.
    pub(crate) const fn name(self) -> &'static [u8] {
        match self {
            Self::Debug => b"DEBUG",
            Self::Err => b"ERR",
            Self::Return => b"RETURN",
        }
    }

    /// Which `set -o` option carries this condition into a nested
    /// evaluation.
    const fn inheritance_option(self) -> ShellOption {
        match self {
            Self::Err => ShellOption::Errtrace,
            Self::Debug | Self::Return => ShellOption::Functrace,
        }
    }
}

/// Resolve a `trap` operand naming a Bash pseudo-condition.
///
/// The comparison is case-insensitive for the same reason the signal
/// table's is: `trap ... err` is what a script that never capitalises
/// writes.
// [spec:nsh:req:compat.bash.traps-introspection]
pub(crate) fn decode(name: &BStr) -> Option<BashCondition> {
    CONDITIONS
        .into_iter()
        .find(|condition| name.eq_ignore_ascii_case(condition.name()))
}

/// The actions installed for the three pseudo-conditions.
///
/// `running` is not a re-entrancy convenience: an `ERR` action that fails
/// raises `ERR`, and a `DEBUG` action is itself a sequence of simple
/// commands. Bash suppresses the condition for the duration of its own
/// action, and without that this table is a non-terminating loop.
pub(crate) struct BashTraps {
    action: [TrapAction; 3],
    running: [bool; 3],
}

impl BashTraps {
    /// Whether one of the three actions is running right now.
    ///
    /// `$BASH_COMMAND` asks, because the reference's own name for what it
    /// holds is `the_printed_command_except_trap`: an action reads the
    /// command it was raised *for*, so the commands inside the action do
    /// not overwrite it.
    // [spec:nsh:req:compat.bash.names.ordinary-state]
    pub(crate) fn action_is_running(&self) -> bool {
        self.running.iter().any(|running| *running)
    }

    pub(crate) const fn new() -> Self {
        Self {
            action: [const { TrapAction::Default }; 3],
            running: [false; 3],
        }
    }

    /// The action installed for `condition`, which is what `trap` and
    /// `trap -p` report.
    pub(crate) fn listed_action(&self, condition: BashCondition) -> &TrapAction {
        &self.action[condition.slot()]
    }

    /// Whether any pseudo-condition carries an action.
    ///
    /// A subshell has to be forked for these exactly as it is for a
    /// signal trap, so `crate::trap::has_traps` asks this too.
    pub(crate) fn any_action(&self) -> bool {
        self.action
            .iter()
            .any(|action| matches!(action, TrapAction::Command(_)))
    }
}

/// Install `action` for `condition`.
///
/// There is no counter to keep beside this: `crate::trap::has_traps` asks
/// [`BashTraps::any_action`] directly, so the table is its own tally.
// [spec:nsh:req:compat.bash.traps-introspection]
pub(crate) fn set(shell: &mut Shell, condition: BashCondition, action: TrapAction) {
    shell.traps.bash.action[condition.slot()] = action;
}

/// The saved actions a nested evaluation must put back.
pub(crate) type SuppressedTraps = Option<[TrapAction; 3]>;

/// Clear the pseudo-traps a function body or forked child does not
/// inherit, handing back what to restore.
///
/// `None` means nothing was touched, which is every POSIX-dialect call:
/// the conditions do not exist there and the table cannot hold an action.
// [spec:nsh:req:compat.bash.traps-introspection]
pub(crate) fn suppress_uninherited(shell: &mut Shell) -> SuppressedTraps {
    if shell.options.dialect() != Dialect::Bash || !shell.traps.bash.any_action() {
        return None;
    }
    let saved = shell.traps.bash.action.clone();
    for condition in CONDITIONS {
        if !shell.options.enabled(condition.inheritance_option()) {
            shell.traps.bash.action[condition.slot()] = TrapAction::Default;
        }
    }
    Some(saved)
}

/// Put back what [`suppress_uninherited`] took.
pub(crate) fn restore(shell: &mut Shell, saved: SuppressedTraps) {
    if let Some(saved) = saved {
        shell.traps.bash.action = saved;
    }
}

/// Evaluate the action installed for `condition`.
///
/// The interrupted status is restored afterwards: none of the three
/// conditions is a command, so none of them may name the status the next
/// `$?` reports. An `exit` inside the action still travels out, and so
/// does the `Flow::Exit` an `errexit` abort inside it produces -- both of
/// those are decisions about the shell rather than about this status.
// [spec:nsh:req:compat.bash.traps-introspection]
fn dispatch(shell: &mut Shell, condition: BashCondition) -> Result<Flow, Error> {
    if shell.options.dialect() != Dialect::Bash {
        return Ok(Flow::Done(shell.status));
    }
    let slot = condition.slot();
    if shell.traps.bash.running[slot] {
        return Ok(Flow::Done(shell.status));
    }
    let TrapAction::Command(command) = shell.traps.bash.action[slot].clone() else {
        return Ok(Flow::Done(shell.status));
    };

    let status = shell.status;
    let line = shell.variables.line_number;
    let outer_trap_status = shell.evaluation.trap_default_exit_status.replace(status);
    shell.traps.bash.running[slot] = true;
    /* Not `evaluate_string`: the action's commands are numbered from the
     * line the condition was raised on, so `$LINENO` inside a one-line
     * action is that line and not `1`. */
    let outcome = crate::resource::with_resources(shell, |shell, _resources| {
        crate::input::set_input_string_at_line(shell, BStr::new(command.as_slice()), line);
        crate::evaluation::parse_and_execute(shell, EvaluationContext::DEFAULT)
    });
    shell.traps.bash.running[slot] = false;
    shell.variables.line_number = line;
    shell.evaluation.trap_default_exit_status = outer_trap_status;
    match outcome? {
        exit @ Flow::Exit { .. } => Ok(exit),
        /* `return` inside a `DEBUG` action is Bash's own no-op, and
         * `break`/`continue` cannot mean the interrupted loop either.
         * They end the action and nothing else. */
        _ => {
            shell.status = status;
            Ok(Flow::Done(status))
        }
    }
}

/// Run the `DEBUG` action before a command the condition covers.
pub(crate) fn run_debug(shell: &mut Shell) -> Result<Flow, Error> {
    dispatch(shell, BashCondition::Debug)
}

/// Run the `ERR` action for a command whose failure `errexit` would act on.
pub(crate) fn run_err(shell: &mut Shell) -> Result<Flow, Error> {
    dispatch(shell, BashCondition::Err)
}

/// Run the `RETURN` action as a function body or dot script finishes.
pub(crate) fn run_return(shell: &mut Shell) -> Result<Flow, Error> {
    dispatch(shell, BashCondition::Return)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bstr::BString;

    /// The three names resolve however they are spelled, and nothing
    /// else does.
    // [spec:nsh:req:compat.bash.traps-introspection/test]
    #[test]
    fn condition_names_round_trip() {
        assert_eq!(decode(BStr::new("DEBUG")), Some(BashCondition::Debug));
        assert_eq!(decode(BStr::new("err")), Some(BashCondition::Err));
        assert_eq!(decode(BStr::new("Return")), Some(BashCondition::Return));
        assert_eq!(decode(BStr::new("EXIT")), None);
        assert_eq!(decode(BStr::new("INT")), None);
        for condition in CONDITIONS {
            assert_eq!(decode(BStr::new(condition.name())), Some(condition));
        }
    }

    /// Each condition names the option that carries it into a nested
    /// evaluation, and they are not the same option.
    // [spec:nsh:req:compat.bash.traps-introspection/test]
    #[test]
    fn inheritance_options_are_split() {
        assert_eq!(
            BashCondition::Err.inheritance_option(),
            ShellOption::Errtrace
        );
        assert_eq!(
            BashCondition::Debug.inheritance_option(),
            ShellOption::Functrace
        );
        assert_eq!(
            BashCondition::Return.inheritance_option(),
            ShellOption::Functrace
        );
    }

    /// An empty table reports no action, an installed one does, and
    /// `Ignore` is not an action a subshell has to be forked for.
    // [spec:nsh:req:compat.bash.traps-introspection/test]
    #[test]
    fn the_table_is_its_own_tally() {
        let _g = crate::test_support::lock();
        let shell = &mut Shell::new(crate::streams::Streams::INHERIT);
        assert!(!shell.traps.bash.any_action());
        set(
            shell,
            BashCondition::Err,
            TrapAction::Command(BString::from("echo e")),
        );
        assert!(shell.traps.bash.any_action());
        set(shell, BashCondition::Err, TrapAction::Ignore);
        assert!(!shell.traps.bash.any_action());
        set(shell, BashCondition::Err, TrapAction::Default);
        assert!(!shell.traps.bash.any_action());
    }

    /// Suppression is Bash-only, and it keeps the conditions whose
    /// option is on.
    // [spec:nsh:req:compat.bash.traps-introspection/test]
    #[test]
    fn suppression_follows_the_options() {
        let _g = crate::test_support::lock();
        let shell = &mut Shell::new(crate::streams::Streams::INHERIT);
        set(
            shell,
            BashCondition::Debug,
            TrapAction::Command(BString::from("echo d")),
        );
        set(
            shell,
            BashCondition::Err,
            TrapAction::Command(BString::from("echo e")),
        );

        // POSIX mode never touches the table.
        assert!(suppress_uninherited(shell).is_none());

        shell.options.set(ShellOption::Bash, true);
        shell.options.set(ShellOption::Errtrace, true);
        let saved = suppress_uninherited(shell);
        assert!(saved.is_some());
        assert!(matches!(
            shell.traps.bash.listed_action(BashCondition::Debug),
            TrapAction::Default
        ));
        assert!(matches!(
            shell.traps.bash.listed_action(BashCondition::Err),
            TrapAction::Command(_)
        ));
        restore(shell, saved);
        assert!(matches!(
            shell.traps.bash.listed_action(BashCondition::Debug),
            TrapAction::Command(_)
        ));
    }
}
