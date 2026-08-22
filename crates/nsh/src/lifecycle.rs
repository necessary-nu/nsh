//! Ordered lifecycle transitions for an owned [`Shell`](crate::Shell).
//!
//! These are deliberate operations on one shell instance. Each transition
//! names its subsystem effects in execution order; there is no generated
//! fragment list or registration mechanism.

use crate::context::Shell;
use crate::nodes::Node;
use bstr::BString;

impl Shell {
    /// Establish the input, signal, locale, and variable state of a new shell.
    // [spec:dash:sem:init.init-fn]
    // [spec:nsh:req:idiom.owned-lifecycle]
    pub(crate) fn initialize_from(
        &mut self,
        environment: &[(BString, BString)],
    ) -> Result<(), crate::Error> {
        self.initialize_input_state();
        self.initialize_trap_state();
        self.initialize_variable_state(environment)
    }

    /// Release evaluator resources before either recovery or final shutdown.
    // [spec:dash:sem:init.exitreset-fn]
    pub(crate) fn clear_evaluation_resources(&mut self) {
        self.evaluation.loop_depth = 0;
        self.evaluation.expanding_trace_prompt = false;
        self.restore_saved_redirections();
    }

    /// Detach child execution state from the parent immediately after a fork.
    // [spec:dash:sem:init.forkreset-fn]
    pub(crate) fn prepare_fork_child(&mut self, command: Option<&Node>) {
        // [spec:nsh:req:compat.smoosh.control-boundaries]
        self.evaluation.loop_depth = 0;
        self.evaluation.trap_default_exit_status = None;
        self.detach_parent_input();
        // [spec:nsh:req:compat.bash.special-variables]
        crate::variables::special::fork_child(self);
        self.discard_saved_redirections();
        self.prepare_traps_for_child(command);
    }

    /// Restore the state required to continue at the interactive command loop.
    // [spec:dash:sem:init.reset-fn]
    pub(crate) fn recover_command_loop(&mut self) {
        self.discard_interrupted_input();
        self.unwind_local_variables();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bstr::{BStr, BString};

    // [spec:nsh:req:idiom.owned-lifecycle/test]
    #[test]
    fn initialization_populates_owned_state() {
        let _guard = crate::test_support::lock();
        let mut shell = Shell::new(crate::streams::Streams::INHERIT);
        let environment = [(BString::from("OWNED_LIFECYCLE"), BString::from("yes"))];

        shell.initialize_from(&environment).unwrap();

        assert_eq!(
            shell.var(BStr::new("OWNED_LIFECYCLE")),
            Some(BStr::new("yes"))
        );
        assert!(crate::input::current_input_frame(&mut shell.input).uses_stdin());
        shell.evaluation.loop_depth = 3;
        shell.evaluation.expanding_trace_prompt = true;
        shell.clear_evaluation_resources();
        assert_eq!(
            (
                shell.evaluation.loop_depth,
                shell.evaluation.expanding_trace_prompt
            ),
            (0, false)
        );
    }
}
