//! Execution of Bash `<(list)` and `>(list)` process substitutions.
//!
//! A process substitution is a word, not a command: the list runs in its own
//! child connected to a pipe, and what lands in the surrounding word is a
//! *name* for the shell's end of that pipe. Everything hard about it is
//! therefore ownership rather than evaluation.
//!
//! Three rules hold the design together.
//!
//! * **The shell owns its end for exactly as long as the name means
//!   anything.** The descriptor lives in [`SubstitutionStack`] until the
//!   syntax node that produced it finishes — the command, loop or redirected
//!   group whose word carried the substitution. Nothing may close it sooner,
//!   because the consuming program has not opened the name yet when the word
//!   is built; and nothing may keep it longer, because a `>(list)` reader
//!   sees end-of-file only once the shell's write end is gone, and a loop
//!   that never released one would open a pipe per iteration.
//! * **It is close-on-exec, and published once.** Every other descriptor
//!   this shell owns is hidden from the programs it runs. This one has to be
//!   visible to exactly one image, so the flag is cleared at the process
//!   terminus in [`publish_before_exec`], after the last fork this process
//!   will ever make.
//! * **The child is nobody's job.** Bash does not wait for a process
//!   substitution and does not set `$!` to it, so there is no job record for
//!   `jobs` to print or `wait` to block on; the generic reaper collects it
//!   alongside the here-document writers.

use std::sync::{Arc, Mutex, MutexGuard};

use bstr::BString;
use nsh_platform::{Descriptor, NativeStrExt as _};

use crate::context::Shell;
use crate::descriptors::LogicalDescriptor;
use crate::error::Error;
use crate::nodes::{BashProcessDirection, BashProcessSubstitution, Node};

/// Every process substitution whose name is still open.
///
/// A stack rather than a set: substitutions are opened while a syntax node
/// runs and closed when it finishes, so the live ones are always a suffix and
/// one mark names them all. The descriptor *is* the value — dropping it
/// closes the shell's end, which is the entire release mechanism.
///
/// Shared ownership for the same reason [`crate::descriptors::DescriptorSlot`]
/// has it: a scope guard that borrowed the shell would stop the scope's own
/// body from using it, so the guard holds the stack instead.
#[derive(Clone, Default)]
pub(crate) struct SubstitutionStack(Arc<Mutex<Vec<Descriptor>>>);

impl SubstitutionStack {
    fn open(&self) -> MutexGuard<'_, Vec<Descriptor>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The substitution names one syntax node may open.
///
/// Dropping this closes them, on the error path as readily as on the ordinary
/// one — which is where a `>(list)` child gets its end-of-file.
pub(crate) struct NameScope {
    stack: SubstitutionStack,
    mark: usize,
}

impl Drop for NameScope {
    fn drop(&mut self) {
        self.stack.open().truncate(self.mark);
    }
}

/// Begin the scope in which one syntax node's substitution names are valid.
// [spec:nsh:req:compat.bash.process-substitution]
pub(crate) fn scope(shell: &Shell) -> NameScope {
    let stack = shell.process_substitutions.clone();
    let mark = stack.open().len();
    NameScope { stack, mark }
}

/// Make every live substitution name resolve in the image about to run.
///
/// Called from the process terminus, immediately before the descriptor table
/// is materialized and `exec` replaces this image. A substituted `/dev/fd/N`
/// is only a promise until this runs, and it is kept for one program at a
/// time rather than for every child the shell happens to fork.
// [spec:nsh:req:idiom.descriptor-materialization]
pub(crate) fn publish_before_exec(shell: &Shell) -> std::io::Result<()> {
    for descriptor in shell.process_substitutions.open().iter() {
        nsh_platform::publish_descriptor_across_exec(descriptor)?;
    }
    Ok(())
}

/// Run one `<(list)` or `>(list)` and answer with the name of its pipe.
// [spec:nsh:req:compat.bash.process-substitution]
pub(crate) fn substitute(
    shell: &mut Shell,
    substitution: &BashProcessSubstitution,
) -> Result<BString, Error> {
    let body = substitution.body.as_deref();
    let direction = substitution.direction;

    crate::error::with_interrupts_deferred(shell, |shell| {
        let (pipe, _memory_backed) = crate::redirection::create_pipe(shell, false)?;
        /* Which end the shell keeps is the whole difference between the two
         * directions: `<(list)` publishes a name to read the list's output
         * from, `>(list)` a name to write the list's input to. */
        let (shell_end, child_end, child_slot) = match direction {
            BashProcessDirection::Input => (pipe.read, pipe.write, LogicalDescriptor::STDOUT),
            BashProcessDirection::Output => (pipe.write, pipe.read, LogicalDescriptor::STDIN),
        };
        let Some(name) = nsh_platform::descriptor_name(&shell_end) else {
            return Err(unnameable_descriptor_error(shell));
        };
        let name = BString::from(name.to_shell_bytes());

        /* No job: the shell neither waits for this child nor reports it, so
         * it must not hold a job record that `jobs` would print, `wait`
         * would block on, or `$!` would name. */
        if matches!(
            crate::jobs::fork_shell(shell, None, None, crate::jobs::ForkMode::WithoutJob)?,
            nsh_platform::ForkResult::Child
        ) {
            crate::error::clear_interrupt_deferral(&mut shell.interrupt_deferral);
            drop(shell_end);
            /* The names its parent is holding belong to the command that
             * built them, not to this child. A write end kept alive here
             * would deny end-of-file to an unrelated reader. */
            shell.process_substitutions.open().clear();
            let outcome = run_substitution_body(shell, body, child_slot, child_end);
            crate::runtime::exit_from_child(shell, outcome);
        }
        drop(child_end);
        shell.process_substitutions.open().push(shell_end);
        Ok(name)
    })
}

/// The child half: connect the pipe to the list and run it to its terminus.
fn run_substitution_body(
    shell: &mut Shell,
    body: Option<&Node>,
    slot: LogicalDescriptor,
    end: Descriptor,
) -> Result<crate::evaluation::Flow, Error> {
    shell
        .descriptors
        .install_owned(slot, end)
        .map_err(|error| crate::redirection::descriptor_error(shell, slot, error))?;
    crate::evaluation::evaluate_tree_without_exit(
        shell,
        body,
        crate::evaluation::EvaluationContext::EXITING,
    )
}

/// The diagnostic for a system that cannot name an open descriptor.
///
/// `[dec:nsh:safety-trumps-compatibility]`: the alternative is a temporary
/// FIFO, and a careless one is the shape of CVE-2000-1134. Saying so is
/// better than substituting a path that will not open.
fn unnameable_descriptor_error(shell: &mut Shell) -> Error {
    shell
        .diagnostics()
        .shell_error(b"process substitution is not supported on this system")
}
