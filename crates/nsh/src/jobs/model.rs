//! Typed identities and states owned by the job table.

use bstr::BString;
use core::ffi::c_int;
use nsh_platform::{ChildStatus, Descriptor, ProcessGroupState, ProcessId};
use std::ops::{Index, IndexMut};

/// Stable identity of a slot in one shell's job table.
// [spec:nsh:def:idiom.job-control-model]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct JobId(pub(super) usize);

/// Aggregate state derived from every process in a job.
// [spec:nsh:def:idiom.job-control-model]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum JobState {
    #[default]
    Running,
    Stopped,
    Done,
}

impl JobState {
    const fn can_transition_to(self, next: Self) -> bool {
        !matches!((self, next), (Self::Done, Self::Running | Self::Stopped))
    }
}

// [spec:dash:def:jobs.procstat]
pub(crate) struct ProcStat {
    pub(crate) pid: ProcessId,
    pub(crate) status: Option<ChildStatus>,
    pub(crate) cmd: BString,
}

// [spec:dash:def:jobs.job]
pub(crate) struct Job {
    pub(crate) ps: Vec<ProcStat>,
    pub(crate) stopstatus: Option<ChildStatus>,
    state: JobState,
    pub(crate) sigint: bool,
    pub(crate) jobctl: bool,
    pub(crate) waited: bool,
    pub(crate) used: bool,
    pub(crate) changed: bool,
    pub(crate) prev_job: Option<JobId>,
    pub(crate) terminal_settings: Option<nsh_platform::TerminalSettings>,
}

impl Job {
    pub(super) const fn new() -> Self {
        Self {
            ps: Vec::new(),
            stopstatus: None,
            state: JobState::Running,
            sigint: false,
            jobctl: false,
            waited: false,
            used: false,
            changed: false,
            prev_job: None,
            terminal_settings: None,
        }
    }

    pub(crate) const fn is_running(&self) -> bool {
        matches!(self.state, JobState::Running)
    }

    pub(crate) const fn is_stopped(&self) -> bool {
        matches!(self.state, JobState::Stopped)
    }

    pub(crate) const fn is_done(&self) -> bool {
        matches!(self.state, JobState::Done)
    }

    pub(super) fn transition_to(&mut self, next: JobState) -> bool {
        assert!(
            self.state.can_transition_to(next),
            "a completed job cannot become active again"
        );
        if self.state == next {
            return false;
        }
        self.state = next;
        true
    }

    pub(crate) fn restart(&mut self) -> bool {
        if self.state == JobState::Done {
            return false;
        }
        self.transition_to(JobState::Running);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn done_state_is_terminal() {
        assert!(!JobState::Done.can_transition_to(JobState::Running));
        assert!(!JobState::Done.can_transition_to(JobState::Stopped));
        assert!(JobState::Running.can_transition_to(JobState::Stopped));
        assert!(JobState::Stopped.can_transition_to(JobState::Running));
        assert!(JobState::Stopped.can_transition_to(JobState::Done));
    }
}

/// The shell's jobs and the terminal state needed for job control.
pub(crate) struct JobTable {
    pub(crate) tab: Vec<Job>,
    pub(crate) curjob: Option<JobId>,
    pub(crate) jobctl: bool,
    pub(crate) initialpgrp: Option<ProcessGroupState>,
    pub(crate) ttyfd: Option<Descriptor>,
    pub(crate) shell_terminal_settings: Option<nsh_platform::TerminalSettings>,
    pub(crate) job_warning: c_int,
}

impl JobTable {
    pub(crate) const fn new() -> Self {
        Self {
            tab: Vec::new(),
            curjob: None,
            jobctl: false,
            initialpgrp: None,
            ttyfd: None,
            shell_terminal_settings: None,
            job_warning: 0,
        }
    }
}

impl Index<JobId> for JobTable {
    type Output = Job;

    fn index(&self, id: JobId) -> &Self::Output {
        &self.tab[id.0]
    }
}

impl IndexMut<JobId> for JobTable {
    fn index_mut(&mut self, id: JobId) -> &mut Self::Output {
        &mut self.tab[id.0]
    }
}
