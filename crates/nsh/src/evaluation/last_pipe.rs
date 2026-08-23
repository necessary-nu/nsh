//! `shopt -s lastpipe`: the last stage of a pipeline runs in this shell.
//!
//! Every other stage is a forked child, and the whole point of the
//! option is that this one is not: `seq 2 | mapfile m` has to leave `m`
//! behind, which a child cannot do.
//!
//! What that costs is the standard-input frame. `read` and `mapfile`
//! reach logical descriptor 0 through it, so pointing that descriptor at
//! the pipe means lending the frame to a source it knows nothing about
//! -- and everything the frame knows is about the shell's own input: the
//! bytes it has read ahead of the parser, whether that source can be
//! rewound, and the scratch pipe it peeks through. So the frame is
//! handed over the way a forked child receives it, and taken back the
//! same way afterwards.

use crate::context::Shell;
use crate::descriptors::LogicalDescriptor;
use crate::error::Error;
use crate::jobs::JobId;
use crate::nodes::{Node, Pipeline};
use crate::options::{BashShopt, Dialect, ShellOption};
use nsh_platform::Descriptor;

use super::{EvaluationContext, Flow};

/// Whether the option applies to this pipeline.
///
/// Bash's own three conditions, and each is load-bearing: an
/// asynchronous pipeline has no parent to run in, job control needs
/// every member in the job's own process group, and a pipeline of one
/// has nothing to fork in the first place.
// [spec:nsh:req:compat.bash.builtins-special-variables]
pub(super) fn applies(shell: &Shell, pipeline: &Pipeline) -> bool {
    shell.options.dialect() == Dialect::Bash
        && shell.options.shopt(BashShopt::LastPipe)
        && !pipeline.background
        && !shell.options.enabled(ShellOption::Monitor)
        && pipeline.commands.len() > 1
}

/// Run the last stage here, then collect the stages that were forked.
///
/// It runs before the wait rather than after it: the earlier stages are
/// already writing into a pipe with a finite buffer, and a shell that
/// waited first would deadlock against its own child.
// [spec:nsh:req:compat.bash.builtins-special-variables]
pub(super) fn run(
    shell: &mut Shell,
    command: &Node,
    input: Option<Descriptor>,
    job_id: JobId,
    context: EvaluationContext,
) -> Result<Flow, Error> {
    let restored = shell.descriptors.get(LogicalDescriptor::STDIN);
    lend_standard_input(shell);
    if let Some(input) = input {
        shell
            .descriptors
            .install_owned(LogicalDescriptor::STDIN, input)
            .map_err(|error| {
                crate::redirection::descriptor_error(shell, LogicalDescriptor::STDIN, error)
            })?;
    }

    let outcome = super::evaluate_tree(shell, Some(command), context.without_exit());

    /* Give the borrowed source back what was read ahead of it *before*
     * the shell's own descriptor returns, or the rewind would land on
     * the wrong one. */
    lend_standard_input(shell);
    shell
        .descriptors
        .replace(LogicalDescriptor::STDIN, restored);

    let flow = outcome?;
    let Flow::Done(status) = flow else {
        return Ok(flow);
    };
    crate::jobs::wait_for_job(shell, Some(job_id))?;
    // [spec:nsh:req:compat.bash.builtins-special-variables]
    crate::variables::special::append_pipeline_status(shell, status);
    shell.status = status;
    Ok(Flow::Done(status))
}

/// Hand the standard-input frame over to whichever descriptor is
/// installed on it now.
///
/// `reset_input` is what a forked child already does here: it returns
/// the read-ahead to that descriptor and clears the end of input the
/// frame latched. The peek pipe has to go with it -- bytes left in that
/// scratch describe one descriptor, and reading them back after the swap
/// would feed them to the parser as the shell's own script.
fn lend_standard_input(shell: &mut Shell) {
    crate::input::reset_input(shell);
    shell.input.standard_input_state.pending = None;
    drop(shell.input.standard_input_state.pipe.take());
}
