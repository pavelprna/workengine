use std::path::{Path, PathBuf};
use std::time::Duration;

use workengine_domain::{Outcome, Work, WorkEvent, WorkId};

use crate::error::AppError;

/// Persists current Work status and the append-only event log.
pub trait WorkStore {
    fn get(&self, id: &WorkId) -> Result<Option<Work>, AppError>;
    fn list(&self) -> Result<Vec<Work>, AppError>;
    fn events(&self, id: &WorkId) -> Result<Vec<WorkEvent>, AppError>;
    /// Status update and event append are one operation.
    fn put(&mut self, work: &Work, event: WorkEvent) -> Result<(), AppError>;
}

/// Isolated directory for the life of one Work.
pub trait WorkspaceFactory {
    fn bind(&self, work_id: &WorkId) -> Result<PathBuf, AppError>;
    fn read_artifact(&self, work_id: &WorkId) -> Result<Option<Vec<u8>>, AppError>;
    fn record_memory(&self, work_id: &WorkId, entry: &str) -> Result<(), AppError>;
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
