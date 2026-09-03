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
//! * **The child is nobody's job.** There is no job record for `jobs` to
//!   print, in either shell. Its pid is still the shell's to keep: `wait`
//!   with no operands blocks on the most recent substitution, which is why
//!   [`crate::context::Shell::last_process_substitution`] holds it apart
//!   from the here-document writers and command substitutions the generic
//!   reaper collects it beside. Bash also names it in `$!`, which this
//!   shell does not -- `name-a-process-substitution-in-the-bang-parameter`.
//!
//! # The child can outlive the shell, and in a container that loses its output
//!
//! Neither shell waits for a `>(list)` child at exit — measured 2026-09-02,
//! `seq 3 > >(sleep 1; tac)` returns in 4 ms under the pinned Bash 5.3.15 and
//! under this shell alike — so the child writes to a standard output whose
//! reader may already be tearing down. Under the survey's containment that is
//! a PID namespace whose init dies with the shell, and the child is killed
//! mid-write. `process-sub.test.sh:1` is that race and both shells lose it:
//! interleaved, 100 harness runs each at load 87, the pinned Bash lost it 11
//! times and this shell 9.
//!
//! What the window is made of, from `strace -f -tt` on an idle machine. For
//! `seq 3 > >(tac)` the child needs about 5 ms after its `execve` — almost
//! all of it dynamic linking and locale opening, none of it under the shell's
//! control — and the shell exits 2.3 ms before it finishes. For
//! `{ echo 1; echo 2; echo 3; } > >(tac)` the child is forked later and needs
//! about 22 ms, which is why that case is lost far more often by both shells.
//!
//! One structural difference is the shell's, and it is small. Bash expands a
//! forked command's *redirection* words in the child, so for `seq 3 > >(tac)`
//! the pipe is created inside the `seq` process and Bash's shell never holds
//! the write end at all: the child sees end-of-file the instant `seq` exits,
//! 883 µs before the shell exits. Here the redirection is expanded in the
//! parent, so the shell holds the write end until this module's [`NameScope`]
//! drops — after the command has been reaped — and the child sees
//! end-of-file only 237 µs before the shell exits. That is a real handicap of
//! roughly 0.6 ms, and it is about 3% of the smaller window and 0.3% of the
//! larger, which is why it does not show up in the rates above. Closing the
//! shell's end sooner would mean deciding, per node, that every process which
//! could open the name has already been forked — which is false for a builtin,
//! a loop body and `exec 3> >(list)` — so it is not worth the invariant.

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

        /* No job: the shell must not hold a job record that `jobs` would
         * print. The pid is kept all the same, because `wait` with no
         * operands does block on the most recent one, and the job table
         * is exactly where it cannot be kept. */
        // [spec:nsh:req:compat.bash.process-substitution]
        let forked = crate::jobs::fork_shell(shell, None, None, crate::jobs::ForkMode::WithoutJob)?;
        if let nsh_platform::ForkResult::Parent(child) = forked {
            shell.last_process_substitution = Some(child);
        }
        if matches!(forked, nsh_platform::ForkResult::Child) {
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
