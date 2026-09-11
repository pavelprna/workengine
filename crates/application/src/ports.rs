use std::path::{Path, PathBuf};
use std::time::Duration;

use workengine_domain::{
    AttemptId, ConfirmedOutcome, ExecutionId, ExecutionSpec, Outcome, OutcomeKind, Work, WorkEvent,
    WorkId, WorkStatus,
};

use crate::error::AppError;

/// One immutable event with a store-defined, resumable cursor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SequencedEvent {
    pub seq: u64,
    pub event: WorkEvent,
}

/// Closed metadata-only process event set exposed to observers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessEvent {
    Spawned,
    Exited,
    Killed,
    ChildStdout,
    ChildStderr,
}

impl ProcessEvent {
    pub const ALL: [Self; 5] = [
        Self::Spawned,
        Self::Exited,
        Self::Killed,
        Self::ChildStdout,
        Self::ChildStderr,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Spawned => "spawned",
            Self::Exited => "exited",
            Self::Killed => "killed",
            Self::ChildStdout => "child_stdout",
            Self::ChildStderr => "child_stderr",
        }
    }

    pub fn parse(value: &str) -> Result<Self, AppError> {
        match value {
            "spawned" => Ok(Self::Spawned),
            "exited" => Ok(Self::Exited),
            "killed" => Ok(Self::Killed),
            "child_stdout" => Ok(Self::ChildStdout),
            "child_stderr" => Ok(Self::ChildStderr),
            _ => Err(AppError::store(format!(
                "unknown persisted process event {value}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptState {
    Active,
    Confirmed,
    Retried,
    Parked,
}

impl AttemptState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Confirmed => "confirmed",
            Self::Retried => "retried",
            Self::Parked => "parked",
        }
    }

    pub fn parse(value: &str) -> Result<Self, AppError> {
        match value {
            "active" => Ok(Self::Active),
            "confirmed" => Ok(Self::Confirmed),
            "retried" => Ok(Self::Retried),
            "parked" => Ok(Self::Parked),
            _ => Err(AppError::store(format!(
                "unknown persisted attempt state {value}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecretRefObservation {
    pub name: String,
    pub source: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionSpecObservation {
    pub schema_version: u32,
    pub worker_profile: String,
    pub worker_config_digest: String,
    pub runtime_kind: Option<String>,
    pub runtime_digest: String,
    pub wall_clock_budget_ms: u64,
    pub retry_limit: u32,
    pub channel_policy: String,
    pub secret_refs: Vec<SecretRefObservation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessRecordObservation {
    pub event: ProcessEvent,
    pub occurrences: u64,
    pub first_observed_at_unix_ms: u64,
    pub last_observed_at_unix_ms: u64,
    pub payload_redacted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfirmedOutcomeObservation {
    pub schema_version: u32,
    pub kind: OutcomeKind,
    pub worker_profile: String,
    pub confirmed_at_unix_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptObservation {
    pub attempt_id: AttemptId,
    pub state: AttemptState,
    pub retry_ordinal: u32,
    pub started_at_unix_ms: u64,
    pub last_heartbeat_at_unix_ms: u64,
    pub finished_at_unix_ms: Option<u64>,
    pub terminal_reason: Option<String>,
    pub checkpoint_recorded: bool,
    pub process_records: Vec<ProcessRecordObservation>,
    pub confirmed_outcome: Option<ConfirmedOutcomeObservation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionObservation {
    pub execution_id: ExecutionId,
    pub work_id: WorkId,
    pub created_at_unix_ms: u64,
    pub spec: ExecutionSpecObservation,
    pub attempts: Vec<AttemptObservation>,
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
    fn executions(&self, id: &WorkId) -> Result<Vec<ExecutionObservation>, AppError>;
}

/// Live, attempt-scoped observation written only for the matching lease.
pub trait AttemptRecorder {
    fn heartbeat_attempt(
        &mut self,
        work_id: &WorkId,
        execution_id: &ExecutionId,
        attempt_id: &AttemptId,
        observed_at_unix_ms: u64,
    ) -> Result<(), AppError>;

    fn record_process_event(
        &mut self,
        work_id: &WorkId,
        execution_id: &ExecutionId,
        attempt_id: &AttemptId,
        event: ProcessEvent,
        observed_at_unix_ms: u64,
    ) -> Result<(), AppError>;
}

/// Recorder for adapter tests and runners used outside a claimed execution.
pub struct DiscardAttemptRecorder;

impl AttemptRecorder for DiscardAttemptRecorder {
    fn heartbeat_attempt(
        &mut self,
        _work_id: &WorkId,
        _execution_id: &ExecutionId,
        _attempt_id: &AttemptId,
        _observed_at_unix_ms: u64,
    ) -> Result<(), AppError> {
        Ok(())
    }

    fn record_process_event(
        &mut self,
        _work_id: &WorkId,
        _execution_id: &ExecutionId,
        _attempt_id: &AttemptId,
        _event: ProcessEvent,
        _observed_at_unix_ms: u64,
    ) -> Result<(), AppError> {
        Ok(())
    }
}

/// Persists current Work status and the append-only event log.
pub trait WorkStore: WorkQuery + AttemptRecorder {
    /// Status update and event append are one operation.
    fn put(&mut self, work: &Work, event: WorkEvent) -> Result<(), AppError>;

    /// Atomically installs the one active-attempt lease and commits `running`.
    fn claim_attempt(&mut self, claim: &AttemptClaim<'_>) -> Result<(), AppError>;

    /// Atomically replaces a retrying attempt without changing Work status.
    fn retry_attempt(
        &mut self,
        execution_id: &ExecutionId,
        previous_attempt_id: &AttemptId,
        next_attempt_id: &AttemptId,
        started_at_unix_ms: u64,
    ) -> Result<(), AppError>;

    /// Atomically confirms the matching lease, terminal Work, event, and proof.
    fn confirm_attempt(
        &mut self,
        work: &Work,
        event: WorkEvent,
        outcome: &ConfirmedOutcome,
    ) -> Result<(), AppError>;

    /// Atomically releases the matching lease into the parked queue.
    fn park_attempt(
        &mut self,
        work: &Work,
        event: WorkEvent,
        execution_id: &ExecutionId,
        attempt_id: &AttemptId,
    ) -> Result<(), AppError>;
}

pub struct AttemptClaim<'a> {
    pub execution_id: &'a ExecutionId,
    pub attempt_id: &'a AttemptId,
    pub spec: &'a ExecutionSpec,
    pub work: &'a Work,
    pub event: WorkEvent,
    pub started_at_unix_ms: u64,
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
    fn run(&self, request: &mut RunRequest<'_>) -> Result<Outcome, AppError>;
    fn decode(&self, bytes: &[u8]) -> Result<Outcome, AppError>;
}

pub struct RunRequest<'a> {
    pub work: &'a Work,
    pub execution_id: &'a ExecutionId,
    pub attempt_id: &'a AttemptId,
    pub workspace_root: &'a Path,
    pub budget: Duration,
    pub recorder: &'a mut dyn AttemptRecorder,
}

/// Inputs for `start` that are not ports.
pub struct StartRequest<'a> {
    pub id: &'a WorkId,
    pub execution_spec: &'a ExecutionSpec,
    pub checkout: Option<&'a Path>,
}
