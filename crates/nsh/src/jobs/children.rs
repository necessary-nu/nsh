//! The children this shell forked, and nobody else's.
//!
//! This sits beside the job table rather than inside it, because a job is
//! not where every fork is recorded. `record_forked_child` is given no job
//! at all for a command substitution or a `ForkMode::WithoutJob` child, so
//! a set derived from the job table would be missing exactly the pids that
//! belong to the shell's own most private work. The one thing every fork
//! has in common is that `fork_shell` or `fork_and_execute` performed it,
//! and those are the two places this is written.
//!
//! A `Vec` and not a set: the shell's live children are a handful -- a
//! pipeline's stages and whatever is in the background -- so a linear scan
//! is cheaper than a tree, and insertion order is the order the shell
//! forked them in, which is the order it is least surprised to see them
//! reported in.

use nsh_platform::ProcessId;

/// Every process this shell forked and has not yet reaped.
// [spec:nsh:req:embedding-safety.host-children-are-not-reaped]
#[derive(Debug, Default)]
pub(crate) struct ForkedChildren(Vec<ProcessId>);

impl ForkedChildren {
    pub(crate) const fn new() -> Self {
        Self(Vec::new())
    }

    /// Take ownership of a child this shell has just forked.
    pub(crate) fn record(&mut self, process: ProcessId) {
        debug_assert!(
            !self.0.contains(&process),
            "a live child cannot be forked twice"
        );
        self.0.push(process);
    }

    /// Give up a child that has been reaped, or that the operating system
    /// says is no longer ours to reap.
    pub(crate) fn release(&mut self, process: ProcessId) {
        self.0.retain(|candidate| *candidate != process);
    }

    /// Forget every child, for a forked shell that inherited this set by
    /// copy and has none of them as children of its own.
    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }

    /// The pids to try, as a value: reaping one mutates this set, so the
    /// scan cannot hold a borrow of it.
    pub(crate) fn snapshot(&self) -> Vec<ProcessId> {
        self.0.clone()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The child to block on when no status is ready and one must be
    /// waited for. `preferred` is a process of the job the shell was
    /// asked about; it is used when it is still one of ours, so that a
    /// blocking wait cannot come to rest on an unrelated background job
    /// that outlives the command being waited for.
    pub(crate) fn blocking_target(&self, preferred: Option<ProcessId>) -> Option<ProcessId> {
        match preferred {
            Some(process) if self.0.contains(&process) => Some(process),
            _ => self.0.first().copied(),
        }
    }
}
