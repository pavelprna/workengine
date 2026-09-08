use std::str::FromStr;

use crate::error::DomainError;
use crate::outcome::{OUTCOME_SCHEMA_VERSION, Outcome, OutcomeKind};
use crate::status::WorkStatus;
use crate::work::{Apply, Work, WorkAttributes};
use crate::work_id::WorkId;

/// Schema version of a stored Work event for this slice.
pub const EVENT_SCHEMA_VERSION: u32 = 1;

/// Kinds of events in the append-only Work log.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventKind {
    Created,
    Started,
    Completed,
    Parked,
}

impl EventKind {
    pub const ALL: [EventKind; 4] = [
        EventKind::Created,
        EventKind::Started,
        EventKind::Completed,
        EventKind::Parked,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::Created => "created",
            EventKind::Started => "started",
            EventKind::Completed => "completed",
            EventKind::Parked => "parked",
        }
    }
}

impl FromStr for EventKind {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "created" => Ok(EventKind::Created),
            "started" => Ok(EventKind::Started),
            "completed" => Ok(EventKind::Completed),
            "parked" => Ok(EventKind::Parked),
            other => Err(DomainError::UnknownEventKind(other.to_owned())),
        }
    }
}

/// One immutable record in the Work history. Replay reconstructs status.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkEvent {
    schema_version: u32,
    work_id: WorkId,
    kind: EventKind,
    from: Option<WorkStatus>,
    to: WorkStatus,
    attributes: Option<WorkAttributes>,
    outcome_kind: Option<OutcomeKind>,
    workspace_root: Option<String>,
    created_at_unix_ms: u64,
}

impl WorkEvent {
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        schema_version: u32,
        work_id: WorkId,
        kind: EventKind,
        from: Option<WorkStatus>,
        to: WorkStatus,
        attributes: Option<WorkAttributes>,
        outcome_kind: Option<OutcomeKind>,
        workspace_root: Option<String>,
        created_at_unix_ms: u64,
    ) -> Result<Self, DomainError> {
        if schema_version != EVENT_SCHEMA_VERSION {
            return Err(DomainError::UnsupportedSchemaVersion(schema_version));
        }
        Ok(Self {
            schema_version,
            work_id,
            kind,
            from,
            to,
            attributes,
            outcome_kind,
            workspace_root,
            created_at_unix_ms,
        })
    }

    fn at(
        work: &Work,
        kind: EventKind,
        from: Option<WorkStatus>,
        to: WorkStatus,
        outcome_kind: Option<OutcomeKind>,
        unix_ms: u64,
    ) -> Self {
        Self {
            schema_version: EVENT_SCHEMA_VERSION,
            work_id: work.id().clone(),
            kind,
            from,
            to,
            attributes: match kind {
                EventKind::Created => Some(work.attributes().clone()),
                _ => None,
            },
            outcome_kind,
            workspace_root: match kind {
                EventKind::Created => None,
                _ => work.workspace_root().map(str::to_owned),
            },
            created_at_unix_ms: unix_ms,
        }
    }

    pub fn created(work: &Work) -> Self {
        Self::at(
            work,
            EventKind::Created,
            None,
            WorkStatus::Ready,
            None,
            work.created_at_unix_ms(),
        )
    }

    pub fn started(work: &Work, from: WorkStatus, unix_ms: u64) -> Self {
        Self::at(
            work,
            EventKind::Started,
            Some(from),
            WorkStatus::Running,
            None,
            unix_ms,
        )
    }

    pub fn completed(work: &Work, from: WorkStatus, kind: OutcomeKind, unix_ms: u64) -> Self {
        Self::at(
            work,
            EventKind::Completed,
            Some(from),
            work.status(),
            Some(kind),
            unix_ms,
        )
    }

    pub fn parked(work: &Work, from: WorkStatus, unix_ms: u64) -> Self {
        Self::at(
            work,
            EventKind::Parked,
            Some(from),
            WorkStatus::Parked,
            None,
            unix_ms,
        )
    }

    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn work_id(&self) -> &WorkId {
        &self.work_id
    }

    pub fn kind(&self) -> EventKind {
        self.kind
    }

    pub fn from(&self) -> Option<WorkStatus> {
        self.from
    }

    pub fn to(&self) -> WorkStatus {
        self.to
    }

    pub fn attributes(&self) -> Option<&WorkAttributes> {
        self.attributes.as_ref()
    }

    pub fn outcome_kind(&self) -> Option<OutcomeKind> {
        self.outcome_kind
    }

    pub fn workspace_root(&self) -> Option<&str> {
        self.workspace_root.as_deref()
    }

    pub fn created_at_unix_ms(&self) -> u64 {
        self.created_at_unix_ms
    }
}

/// Fold an append-only event list into current Work. Events are never edited.
pub fn replay(events: &[WorkEvent]) -> Result<Work, DomainError> {
    let Some(first) = events.first() else {
        return Err(DomainError::EmptyReplay);
    };
    if first.kind != EventKind::Created {
        return Err(DomainError::EmptyReplay);
    }
    let attributes = first
        .attributes
        .clone()
        .ok_or(DomainError::InvalidAttributes)?;
    let mut work = Work::new(first.work_id.clone(), attributes, first.created_at_unix_ms)?;
    for event in &events[1..] {
        if event.work_id != *work.id() {
            return Err(DomainError::InvalidWorkId);
        }
        if event.schema_version != EVENT_SCHEMA_VERSION {
            return Err(DomainError::UnsupportedSchemaVersion(event.schema_version));
        }
        match event.kind {
            EventKind::Created => return Err(DomainError::InvalidAttributes),
            EventKind::Started => match work.start()? {
                Apply::Changed { .. } => {
                    if let Some(root) = &event.workspace_root {
                        work.bind_workspace(root.clone())?;
                    }
                }
                Apply::Idempotent { .. } => {}
            },
            EventKind::Completed => {
                let kind = event.outcome_kind.ok_or(DomainError::InvalidAttributes)?;
                let profile = work.attributes().worker_profile().to_owned();
                let outcome = Outcome::new(OUTCOME_SCHEMA_VERSION, kind, profile)?;
                work.complete(&outcome)?;
            }
            EventKind::Parked => {
                work.park()?;
            }
        }
    }
    Ok(work)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work_id::WorkId;

    fn sample() -> Work {
        Work::new(
            WorkId::parse("work-1").unwrap(),
            WorkAttributes::new("do the thing", "stub").unwrap(),
            10,
        )
        .unwrap()
    }

    #[test]
    fn replay_reconstructs_parked_then_succeeded() {
        let mut work = sample();
        let created = WorkEvent::created(&work);
        let from_ready = work.status();
        work.start().unwrap();
        work.bind_workspace("/tmp/w").unwrap();
        let started = WorkEvent::started(&work, from_ready, 11);
        let from_running = work.status();
        work.park().unwrap();
        let parked = WorkEvent::parked(&work, from_running, 12);
        let from_parked = work.status();
        work.start().unwrap();
        let started_again = WorkEvent::started(&work, from_parked, 13);
        let outcome = Outcome::new(1, OutcomeKind::Succeeded, "stub").unwrap();
        let from_running = work.status();
        work.complete(&outcome).unwrap();
        let completed = WorkEvent::completed(&work, from_running, OutcomeKind::Succeeded, 14);

        let replayed = replay(&[created, started, parked, started_again, completed]).unwrap();
        assert_eq!(replayed.status(), WorkStatus::Succeeded);
        assert_eq!(replayed.workspace_root(), Some("/tmp/w"));
        assert_eq!(replayed.id().as_str(), "work-1");
    }

    #[test]
    fn replay_complete_from_parked_skips_a_second_start() {
        let mut work = sample();
        let created = WorkEvent::created(&work);
        let from_ready = work.status();
        work.start().unwrap();
        work.bind_workspace("/tmp/w").unwrap();
        let started = WorkEvent::started(&work, from_ready, 11);
        let from_running = work.status();
        work.park().unwrap();
        let parked = WorkEvent::parked(&work, from_running, 12);
        let outcome = Outcome::new(1, OutcomeKind::Succeeded, "stub").unwrap();
        let from_parked = work.status();
        work.complete(&outcome).unwrap();
        let completed = WorkEvent::completed(&work, from_parked, OutcomeKind::Succeeded, 13);

        let replayed = replay(&[created, started, parked, completed]).unwrap();
        assert_eq!(replayed.status(), WorkStatus::Succeeded);
        assert_eq!(replayed.workspace_root(), Some("/tmp/w"));
    }

    #[test]
    fn non_created_events_store_occurrence_time() {
        let mut work = sample();
        work.start().unwrap();
        let started = WorkEvent::started(&work, WorkStatus::Ready, 99);
        assert_eq!(started.created_at_unix_ms(), 99);
        assert_eq!(WorkEvent::created(&work).created_at_unix_ms(), 10);
    }
}
