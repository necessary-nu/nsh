//! Typed identities and states owned by the job table.

use bstr::BString;
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

pub(crate) struct ProcessRecord {
    pub(crate) process_id: ProcessId,
    pub(crate) status: Option<ChildStatus>,
    pub(crate) command_text: BString,
}

pub(crate) struct Job {
    pub(crate) processes: Vec<ProcessRecord>,
    pub(crate) stop_status: Option<ChildStatus>,
    state: JobState,
    pub(crate) interrupted: bool,
    pub(crate) job_control: bool,
    pub(crate) waited: bool,
    pub(crate) changed: bool,
    pub(crate) terminal_settings: Option<nsh_platform::TerminalSettings>,
}

impl Job {
    pub(super) const fn new() -> Self {
        Self {
            processes: Vec::new(),
            stop_status: None,
            state: JobState::Running,
            interrupted: false,
            job_control: false,
            waited: false,
            changed: false,
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

    #[test]
    fn slots_track_job_liveness() {
        let mut table = JobTable::new();
        table.slots.resize_with(2, || None);
        table.occupy_current(JobId(1), Job::new());

        assert_eq!(table.current(), Some(JobId(1)));
        assert!(table.slots[0].is_none());
        assert!(table.slots[1].is_some());

        drop(table.remove(JobId(1)));
        assert_eq!(table.current(), None);
        assert!(table.slots[1].is_none());
    }

    #[test]
    fn ordering_tracks_job_state() {
        let mut table = JobTable::new();
        table.slots.resize_with(2, || None);
        table.occupy_current(JobId(0), Job::new());
        table.occupy_current(JobId(1), Job::new());

        table[JobId(0)].transition_to(JobState::Stopped);
        table.position_stopped(JobId(0));
        table.position_running(JobId(1));

        assert_eq!(table.current(), Some(JobId(0)));
        assert_eq!(table.previous(), Some(JobId(1)));
        assert_eq!(table.order_snapshot(), vec![JobId(0), JobId(1)]);
    }
}

/// The grace period around a stopped-job exit warning.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum JobWarning {
    #[default]
    Ready,
    Reported,
    Grace,
}

impl JobWarning {
    pub(crate) fn advance(self) -> Self {
        match self {
            Self::Reported => Self::Grace,
            Self::Ready | Self::Grace => Self::Ready,
        }
    }
}

/// The shell's jobs and the terminal state needed for job control.
// [spec:nsh:req:idiom.job-storage]
pub(crate) struct JobTable {
    pub(super) slots: Vec<Option<Job>>,
    order: Vec<JobId>,
    pub(crate) job_control: bool,
    pub(crate) initial_process_group: Option<ProcessGroupState>,
    pub(crate) terminal: Option<Descriptor>,
    pub(crate) shell_terminal_settings: Option<nsh_platform::TerminalSettings>,
    pub(crate) job_warning: JobWarning,
}

// [spec:dash:sem:jobs.set-curjob-fn]
impl JobTable {
    pub(crate) const fn new() -> Self {
        Self {
            slots: Vec::new(),
            order: Vec::new(),
            job_control: false,
            initial_process_group: None,
            terminal: None,
            shell_terminal_settings: None,
            job_warning: JobWarning::Ready,
        }
    }

    pub(crate) fn current(&self) -> Option<JobId> {
        match self.order.as_slice() {
            [current, ..] => Some(*current),
            [] => None,
        }
    }

    pub(crate) fn previous(&self) -> Option<JobId> {
        match self.order.as_slice() {
            [_, previous, ..] => Some(*previous),
            _ => None,
        }
    }

    pub(crate) fn order_snapshot(&self) -> Vec<JobId> {
        let mut snapshot = Vec::with_capacity(self.order.len());
        snapshot.extend(self.order.iter().copied());
        snapshot
    }

    pub(super) fn occupy_current(&mut self, id: JobId, job: Job) {
        let slot = self
            .slots
            .get_mut(id.0)
            .expect("a job slot must be allocated before occupation");
        assert!(slot.is_none(), "a live job cannot be overwritten");
        *slot = Some(job);
        self.order.retain(|candidate| *candidate != id);
        self.order.insert(0, id);
    }

    pub(crate) fn position_running(&mut self, id: JobId) {
        assert!(
            self.slots.get(id.0).is_some_and(Option::is_some),
            "only a live job can be reordered"
        );
        self.order.retain(|candidate| *candidate != id);
        let position = self
            .order
            .iter()
            .position(|candidate| !self[*candidate].is_stopped())
            .unwrap_or(self.order.len());
        self.order.insert(position, id);
    }

    pub(super) fn position_stopped(&mut self, id: JobId) {
        assert!(
            self.slots.get(id.0).is_some_and(Option::is_some),
            "only a live job can be reordered"
        );
        self.order.retain(|candidate| *candidate != id);
        self.order.insert(0, id);
    }

    pub(super) fn remove(&mut self, id: JobId) -> Job {
        self.order.retain(|candidate| *candidate != id);
        self.slots
            .get_mut(id.0)
            .and_then(Option::take)
            .expect("only a live job can be removed")
    }
}

impl Index<JobId> for JobTable {
    type Output = Job;

    fn index(&self, id: JobId) -> &Self::Output {
        self.slots
            .get(id.0)
            .and_then(Option::as_ref)
            .expect("a JobId must name a live job")
    }
}

impl IndexMut<JobId> for JobTable {
    fn index_mut(&mut self, id: JobId) -> &mut Self::Output {
        self.slots
            .get_mut(id.0)
            .and_then(Option::as_mut)
            .expect("a JobId must name a live job")
    }
}
