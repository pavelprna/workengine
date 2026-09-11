use std::path::{Path, PathBuf};
use std::time::Duration;

use workengine_domain::{Outcome, OutcomeKind, Work, WorkEvent, WorkId, WorkStatus};

use crate::error::AppError;

/// One immutable event with a store-defined, resumable cursor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SequencedEvent {
    pub seq: u64,
    pub event: WorkEvent,
}

/// Read-only access to Work snapshots and their append-only history.
///
/// Observers implement only this port: reads must not recover, transition, or
/// otherwise mutate Work state.
pub trait WorkQuery {
    fn get(&self, id: &WorkId) -> Result<Option<Work>, AppError>;
    fn list(&self) -> Result<Vec<Work>, AppError>;
    fn events(&self, id: &WorkId) -> Result<Vec<WorkEvent>, AppError>;
    fn events_after(
        &self,
        after_seq: u64,
        work_id: Option<&WorkId>,
    ) -> Result<Vec<SequencedEvent>, AppError>;
}

/// Persists current Work status and the append-only event log.
pub trait WorkStore: WorkQuery {
    /// Status update and event append are one operation.
    fn put(&mut self, work: &Work, event: WorkEvent) -> Result<(), AppError>;
}

/// How to bind the isolated directory for one Work.
pub struct BindRequest<'a> {
    pub work_id: &'a WorkId,
    pub goal: &'a str,
    pub checkout: Option<&'a Path>,
}

/// Isolated directory for the life of one Work.
pub trait WorkspaceFactory {
    fn bind(&self, request: &BindRequest<'_>) -> Result<PathBuf, AppError>;
    fn read_artifact(&self, work_id: &WorkId) -> Result<Option<Vec<u8>>, AppError>;
    fn record_memory(
        &self,
        work_id: &WorkId,
        status: WorkStatus,
        outcome_kind: OutcomeKind,
    ) -> Result<(), AppError>;
}

/// Spawn, wait, record. Implementations own process groups and hang detection.
pub trait WorkerRunner {
    fn run(&self, request: &RunRequest<'_>) -> Result<Outcome, AppError>;
    fn decode(&self, bytes: &[u8]) -> Result<Outcome, AppError>;
}

pub struct RunRequest<'a> {
    pub work: &'a Work,
    pub workspace_root: &'a Path,
    pub budget: Duration,
}

/// Inputs for `start` that are not ports.
pub struct StartRequest<'a> {
    pub id: &'a WorkId,
    pub budget: Duration,
    /// Extra spawns after a retry-classified channel error. Snapshotted for this call.
    pub retry_limit: u32,
    pub checkout: Option<&'a Path>,
}
