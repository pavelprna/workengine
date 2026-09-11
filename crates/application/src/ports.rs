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

/// Durable operator request consumed by the matching active supervisor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlKind {
    Park,
    Abort,
}

impl ControlKind {
    pub const ALL: [Self; 2] = [Self::Park, Self::Abort];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Park => "park",
            Self::Abort => "abort",
        }
    }

    pub fn parse(value: &str) -> Result<Self, AppError> {
        match value {
            "park" => Ok(Self::Park),
            "abort" => Ok(Self::Abort),
            _ => Err(AppError::store(format!(
                "unknown persisted control kind {value}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControlDirective {
    pub request_id: i64,
    pub kind: ControlKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperatorInputKind {
    Answer,
    Consent,
}

impl OperatorInputKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Answer => "answer",
            Self::Consent => "consent",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorInput {
    pub id: i64,
    pub kind: OperatorInputKind,
    pub body: String,
    pub created_at_unix_ms: u64,
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
        _work_id: &WorkId,
        _execution_id: &ExecutionId,
        _attempt_id: &AttemptId,
        observed_at_unix_ms: u64,
    ) -> Result<(), AppError>;

    fn record_process_event(
        &mut self,
        _work_id: &WorkId,
        _execution_id: &ExecutionId,
        _attempt_id: &AttemptId,
        event: ProcessEvent,
        observed_at_unix_ms: u64,
    ) -> Result<(), AppError>;

    fn control_directive(
        &mut self,
        _work_id: &WorkId,
        _execution_id: &ExecutionId,
        _attempt_id: &AttemptId,
    ) -> Result<Option<ControlDirective>, AppError> {
        Ok(None)
    }

    fn record_checkpoint(
        &mut self,
        _work_id: &WorkId,
        _execution_id: &ExecutionId,
        _attempt_id: &AttemptId,
        _observed_at_unix_ms: u64,
    ) -> Result<(), AppError> {
        Ok(())
    }
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

    fn control_directive(
        &mut self,
        _work_id: &WorkId,
        _execution_id: &ExecutionId,
        _attempt_id: &AttemptId,
    ) -> Result<Option<ControlDirective>, AppError> {
        Ok(None)
    }

    fn record_checkpoint(
        &mut self,
        _work_id: &WorkId,
        _execution_id: &ExecutionId,
        _attempt_id: &AttemptId,
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

    fn confirm_attempt_with_reason(
        &mut self,
        work: &Work,
        event: WorkEvent,
        outcome: &ConfirmedOutcome,
        terminal_reason: &str,
        control_request_id: Option<i64>,
    ) -> Result<(), AppError> {
        let _ = (terminal_reason, control_request_id);
        self.confirm_attempt(work, event, outcome)
    }

    /// Atomically releases the matching lease into the parked queue.
    fn park_attempt(
        &mut self,
        work: &Work,
        event: WorkEvent,
        execution_id: &ExecutionId,
        attempt_id: &AttemptId,
    ) -> Result<(), AppError>;

    fn park_attempt_with_checkpoint(
        &mut self,
        work: &Work,
        event: WorkEvent,
        execution_id: &ExecutionId,
        attempt_id: &AttemptId,
        checkpoint_recorded: bool,
        control_request_id: Option<i64>,
    ) -> Result<(), AppError> {
        if !checkpoint_recorded {
            return Err(AppError::Conflict(
                "park requires a validated attempt checkpoint".to_owned(),
            ));
        }
        let _ = control_request_id;
        self.park_attempt(work, event, execution_id, attempt_id)
    }

    /// Persist an attempt-scoped operator request after validating the lease.
    fn request_control(
        &mut self,
        _work_id: &WorkId,
        kind: ControlKind,
        created_at_unix_ms: u64,
    ) -> Result<ControlDirective, AppError> {
        let _ = (kind, created_at_unix_ms);
        Err(AppError::Conflict(
            "store does not support live control".to_owned(),
        ))
    }

    /// Atomically release an abandoned active lease back into the queue.
    fn reclaim_attempt(
        &mut self,
        work: &Work,
        event: WorkEvent,
        execution_id: &ExecutionId,
        attempt_id: &AttemptId,
    ) -> Result<(), AppError> {
        let _ = (execution_id, attempt_id);
        self.put(work, event)
    }

    fn append_operator_input(
        &mut self,
        _work_id: &WorkId,
        kind: OperatorInputKind,
        body: &str,
        created_at_unix_ms: u64,
    ) -> Result<OperatorInput, AppError> {
        Ok(OperatorInput {
            id: 1,
            kind,
            body: body.to_owned(),
            created_at_unix_ms,
        })
    }
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
        _work_id: &WorkId,
        status: WorkStatus,
        outcome_kind: OutcomeKind,
    ) -> Result<(), AppError>;
    fn bind_attempt_control(
        &self,
        work_id: &WorkId,
        execution_id: &ExecutionId,
        attempt_id: &AttemptId,
    ) -> Result<PathBuf, AppError> {
        let _ = (work_id, execution_id, attempt_id);
        Ok(PathBuf::from("."))
    }
    fn record_operator_input(
        &self,
        _work_id: &WorkId,
        input: &OperatorInput,
    ) -> Result<(), AppError> {
        let _ = input;
        Ok(())
    }
}

/// Spawn, wait, record. Implementations own process groups and hang detection.
pub trait WorkerRunner {
    fn run(&self, request: &mut RunRequest<'_>) -> Result<WorkerExit, AppError>;
    fn decode(&self, bytes: &[u8]) -> Result<Outcome, AppError>;
}

pub enum WorkerExit {
    Completed(Outcome),
    Parked { control_request_id: Option<i64> },
    Aborted { control_request_id: i64 },
}

pub struct RunRequest<'a> {
    pub work: &'a Work,
    pub execution_id: &'a ExecutionId,
    pub attempt_id: &'a AttemptId,
    pub workspace_root: &'a Path,
    pub control_root: &'a Path,
    pub budget: Duration,
    pub recorder: &'a mut dyn AttemptRecorder,
}

/// Inputs for `start` that are not ports.
pub struct StartRequest<'a> {
    pub id: &'a WorkId,
    pub execution_spec: &'a ExecutionSpec,
    pub checkout: Option<&'a Path>,
}
