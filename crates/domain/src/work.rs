use crate::error::DomainError;
use crate::outcome::Outcome;
use crate::status::WorkStatus;
use crate::work_id::WorkId;

/// Result of applying a domain operation. Idempotent repeats do not change status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Apply {
    Changed { from: WorkStatus, to: WorkStatus },
    Idempotent { status: WorkStatus },
}

/// Content of Work that is not lifecycle status.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkAttributes {
    goal: String,
    worker_profile: String,
}

impl WorkAttributes {
    pub fn new(
        goal: impl Into<String>,
        worker_profile: impl Into<String>,
    ) -> Result<Self, DomainError> {
        let goal = goal.into();
        let worker_profile = worker_profile.into();
        if goal.is_empty() || worker_profile.is_empty() {
            return Err(DomainError::InvalidAttributes);
        }
        Ok(Self {
            goal,
            worker_profile,
        })
    }

    pub fn goal(&self) -> &str {
        &self.goal
    }

    pub fn worker_profile(&self) -> &str {
        &self.worker_profile
    }
}

/// One Work. Workengine is the only writer of its status.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Work {
    id: WorkId,
    status: WorkStatus,
    attributes: WorkAttributes,
    workspace_root: Option<String>,
    created_at_unix_ms: u64,
}

impl Work {
    pub fn new(
        id: WorkId,
        attributes: WorkAttributes,
        created_at_unix_ms: u64,
    ) -> Result<Self, DomainError> {
        Ok(Self {
            id,
            status: WorkStatus::Ready,
            attributes,
            workspace_root: None,
            created_at_unix_ms,
        })
    }

    /// Load a persisted snapshot. Not a transition; the store is the writer.
    pub fn restore(
        id: WorkId,
        status: WorkStatus,
        attributes: WorkAttributes,
        workspace_root: Option<String>,
        created_at_unix_ms: u64,
    ) -> Self {
        Self {
            id,
            status,
            attributes,
            workspace_root,
            created_at_unix_ms,
        }
    }

    pub fn id(&self) -> &WorkId {
        &self.id
    }

    pub fn status(&self) -> WorkStatus {
        self.status
    }

    pub fn attributes(&self) -> &WorkAttributes {
        &self.attributes
    }

    pub fn workspace_root(&self) -> Option<&str> {
        self.workspace_root.as_deref()
    }

    pub fn created_at_unix_ms(&self) -> u64 {
        self.created_at_unix_ms
    }

    /// Bind the isolated workspace address. Allowed while `running` after `start`.
    pub fn bind_workspace(&mut self, root: impl Into<String>) {
        self.workspace_root = Some(root.into());
    }

    /// `ready` or `parked` → `running`. A leftover `running` is not start: recover parks first.
    pub fn start(&mut self) -> Result<Apply, DomainError> {
        match self.status {
            WorkStatus::Ready | WorkStatus::Parked => {
                let from = self.status;
                self.status = WorkStatus::Running;
                Ok(Apply::Changed {
                    from,
                    to: WorkStatus::Running,
                })
            }
            other => Err(DomainError::IllegalTransition {
                from: other,
                to: WorkStatus::Running,
                via: "start",
            }),
        }
    }

    /// `running` → succeeded or failed from a closed outcome. Repeat is idempotent.
    pub fn complete(&mut self, outcome: &Outcome) -> Result<Apply, DomainError> {
        let to = outcome.kind().to_status();
        match self.status {
            WorkStatus::Running => {
                let from = self.status;
                self.status = to;
                Ok(Apply::Changed { from, to })
            }
            WorkStatus::Succeeded if to == WorkStatus::Succeeded => Ok(Apply::Idempotent {
                status: WorkStatus::Succeeded,
            }),
            WorkStatus::Failed if to == WorkStatus::Failed => Ok(Apply::Idempotent {
                status: WorkStatus::Failed,
            }),
            WorkStatus::Succeeded | WorkStatus::Failed => {
                Err(DomainError::TerminalConflict(self.status))
            }
            other => Err(DomainError::IllegalTransition {
                from: other,
                to,
                via: "complete",
            }),
        }
    }

    /// `running` → `parked`. Already parked is idempotent. Does not kill a process group.
    pub fn park(&mut self) -> Result<Apply, DomainError> {
        match self.status {
            WorkStatus::Running => {
                let from = self.status;
                self.status = WorkStatus::Parked;
                Ok(Apply::Changed {
                    from,
                    to: WorkStatus::Parked,
                })
            }
            WorkStatus::Parked => Ok(Apply::Idempotent {
                status: WorkStatus::Parked,
            }),
            other => Err(DomainError::IllegalTransition {
                from: other,
                to: WorkStatus::Parked,
                via: "park",
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome::{Outcome, OutcomeKind};

    fn work() -> Work {
        Work::new(
            WorkId::parse("abc").unwrap(),
            WorkAttributes::new("goal", "stub").unwrap(),
            1,
        )
        .unwrap()
    }

    fn outcome(kind: OutcomeKind) -> Outcome {
        Outcome::new(1, kind, "stub").unwrap()
    }

    #[test]
    fn ready_start_running() {
        let mut w = work();
        assert_eq!(
            w.start().unwrap(),
            Apply::Changed {
                from: WorkStatus::Ready,
                to: WorkStatus::Running,
            }
        );
        assert_eq!(w.status(), WorkStatus::Running);
    }

    #[test]
    fn parked_start_running_same_id() {
        let mut w = work();
        w.start().unwrap();
        w.park().unwrap();
        let id = w.id().clone();
        w.start().unwrap();
        assert_eq!(w.status(), WorkStatus::Running);
        assert_eq!(w.id(), &id);
    }

    #[test]
    fn running_complete_succeeded() {
        let mut w = work();
        w.start().unwrap();
        w.complete(&outcome(OutcomeKind::Succeeded)).unwrap();
        assert_eq!(w.status(), WorkStatus::Succeeded);
    }

    #[test]
    fn timeout_completes_to_failed() {
        let mut w = work();
        w.start().unwrap();
        w.complete(&outcome(OutcomeKind::TimedOut)).unwrap();
        assert_eq!(w.status(), WorkStatus::Failed);
    }

    #[test]
    fn illegal_transition_is_a_domain_error() {
        let mut w = work();
        let err = w.complete(&outcome(OutcomeKind::Succeeded)).unwrap_err();
        assert!(matches!(
            err,
            DomainError::IllegalTransition {
                from: WorkStatus::Ready,
                via: "complete",
                ..
            }
        ));
        assert_eq!(w.status(), WorkStatus::Ready);
    }

    #[test]
    fn succeeded_cannot_start() {
        let mut w = work();
        w.start().unwrap();
        w.complete(&outcome(OutcomeKind::Succeeded)).unwrap();
        assert!(matches!(
            w.start(),
            Err(DomainError::IllegalTransition {
                from: WorkStatus::Succeeded,
                via: "start",
                ..
            })
        ));
    }

    #[test]
    fn complete_is_idempotent_for_the_same_terminal() {
        let mut w = work();
        w.start().unwrap();
        w.complete(&outcome(OutcomeKind::Succeeded)).unwrap();
        assert_eq!(
            w.complete(&outcome(OutcomeKind::Succeeded)).unwrap(),
            Apply::Idempotent {
                status: WorkStatus::Succeeded,
            }
        );
        assert_eq!(w.status(), WorkStatus::Succeeded);
    }

    #[test]
    fn complete_conflict_when_terminal_kind_differs() {
        let mut w = work();
        w.start().unwrap();
        w.complete(&outcome(OutcomeKind::Succeeded)).unwrap();
        assert!(matches!(
            w.complete(&outcome(OutcomeKind::Failed)),
            Err(DomainError::TerminalConflict(WorkStatus::Succeeded))
        ));
    }

    #[test]
    fn park_from_ready_is_illegal() {
        let mut w = work();
        assert!(matches!(
            w.park(),
            Err(DomainError::IllegalTransition {
                from: WorkStatus::Ready,
                via: "park",
                ..
            })
        ));
    }

    #[test]
    fn park_is_idempotent() {
        let mut w = work();
        w.start().unwrap();
        w.park().unwrap();
        assert_eq!(
            w.park().unwrap(),
            Apply::Idempotent {
                status: WorkStatus::Parked,
            }
        );
    }

    #[test]
    fn running_start_is_not_idempotent() {
        let mut w = work();
        w.start().unwrap();
        assert!(matches!(
            w.start(),
            Err(DomainError::IllegalTransition {
                from: WorkStatus::Running,
                via: "start",
                ..
            })
        ));
    }
}
