//! Structured ownership of temporary shell execution state.
//!
//! A scope is driven by a closure because holding a Rust borrow of the shell
//! in a `Drop` guard would prevent the body from using that same shell. The
//! closure may return any value, including `Result` or evaluator control flow;
//! restoration runs after that return value has been produced and before it is
//! handed back to the caller.

use crate::context::Shell;
use crate::error::Error;
use crate::redirection::ExpandedRedirection;

/// Snapshot of the temporary stacks that one shell operation may extend.
// [spec:nsh:req:idiom.resource-scopes]
pub(crate) struct ResourceScope {
    input_mark: usize,
    input_floor: usize,
    redirection_mark: Option<usize>,
    local_mark: usize,
    redirection_frame: bool,
    active: bool,
}

impl ResourceScope {
    fn enter(shell: &mut Shell) -> Self {
        Self {
            input_mark: shell.input.mark(),
            input_floor: shell.input.floor(),
            redirection_mark: None,
            local_mark: crate::variables::push_local_scope(shell, false),
            redirection_frame: false,
            active: true,
        }
    }

    /// Start a command-local variable frame when the command requires one.
    pub(crate) fn begin_local_variables(&mut self, shell: &mut Shell, enabled: bool) {
        let mark = crate::variables::push_local_scope(shell, enabled);
        debug_assert_eq!(
            mark, self.local_mark,
            "a resource scope owns its local frame"
        );
    }

    /// Save and apply a command's redirections under this scope.
    pub(crate) fn apply_redirections(
        &mut self,
        shell: &mut Shell,
        redirections: &[ExpandedRedirection<'_>],
    ) -> Result<(), Error> {
        debug_assert!(
            self.redirection_mark.is_none(),
            "one scope applies redirections once"
        );
        let mark = crate::redirection::push_redirections(shell, redirections);
        self.redirection_mark = Some(mark);
        self.redirection_frame = !redirections.is_empty();
        crate::redirection::redirect_safely(
            shell,
            redirections,
            crate::redirection::RedirectionMode::Push,
        )
    }

    /// Consume saved descriptor state while retaining the active mappings.
    ///
    /// This is the `exec`-without-a-command case: its redirections become the
    /// shell's current descriptors instead of being restored on scope exit.
    pub(crate) fn retain_redirections(&mut self, shell: &mut Shell) {
        if self.redirection_frame {
            crate::redirection::pop_redirection(shell, true);
            self.redirection_frame = false;
        }
    }

    /// Restore this scope now. Calling it again is a no-op.
    ///
    /// Most callers let [`with_resources`] do this. A caller whose ordinary
    /// epilogue must run after restoration may finish the scope explicitly.
    pub(crate) fn restore(&mut self, shell: &mut Shell) {
        if !self.active {
            return;
        }
        if let Some(mark) = self.redirection_mark.take() {
            crate::redirection::unwind_redirections(shell, mark);
        }
        crate::input::unwind_input_frames(shell, self.input_mark);
        shell.input.set_floor(self.input_floor);
        crate::variables::unwind_local_scopes(shell, self.local_mark);
        self.redirection_frame = false;
        self.active = false;
    }
}

/// Run an operation with structured temporary-resource restoration.
pub(crate) fn with_resources<T>(
    shell: &mut Shell,
    body: impl FnOnce(&mut Shell, &mut ResourceScope) -> T,
) -> T {
    let mut resources = ResourceScope::enter(shell);
    let outcome = body(shell, &mut resources);
    resources.restore(shell);
    outcome
}

#[cfg(test)]
mod tests {
    use bstr::BStr;

    use super::*;
    use crate::descriptors::LogicalDescriptor;
    use crate::variables::VariableAttributes;

    // [spec:nsh:req:idiom.resource-scopes/test]
    #[test]
    fn an_error_restores_every_temporary_resource() {
        let mut shell = Shell::builder().build().unwrap();
        crate::variables::set_bytes(
            &mut shell,
            BStr::new(b"scope_value"),
            Some(BStr::new(b"outer")),
            VariableAttributes::NONE,
        )
        .unwrap();
        let input_mark = shell.input.mark();
        let local_mark = crate::variables::push_local_scope(&mut shell, false);

        let outcome: Result<(), Error> = with_resources(&mut shell, |shell, resources| {
            crate::input::set_input_string(shell, BStr::new(b"temporary input"));
            resources.begin_local_variables(shell, true);
            crate::variables::make_local_bytes(
                shell,
                BStr::new(b"scope_value=inner"),
                VariableAttributes::NONE,
            )?;
            let redirections = [ExpandedRedirection::Descriptor {
                descriptor: LogicalDescriptor::STDOUT,
                source: None,
            }];
            resources.apply_redirections(shell, &redirections)?;
            assert!(!shell.descriptors.is_open(LogicalDescriptor::STDOUT));
            Err(Error::reported(0, 1))
        });

        assert!(outcome.is_err());
        assert_eq!(shell.input.mark(), input_mark);
        assert_eq!(
            crate::variables::push_local_scope(&mut shell, false),
            local_mark
        );
        assert!(shell.descriptors.is_open(LogicalDescriptor::STDOUT));
        assert_eq!(
            crate::variables::lookup_bytes(&mut shell, BStr::new(b"scope_value"))
                .as_ref()
                .map(|value| value.as_slice()),
            Some(b"outer".as_slice()),
        );
    }
}
