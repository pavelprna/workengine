use crate::status::WorkStatus;

/// Errors that originate in the domain. None of these perform I/O.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum DomainError {
    #[error("illegal transition from {from} to {to} via {via}")]
    IllegalTransition {
        from: WorkStatus,
        to: WorkStatus,
        via: &'static str,
    },
    #[error("unknown work status {0:?}")]
    UnknownStatus(String),
    #[error("unknown outcome kind {0:?}")]
    UnknownOutcomeKind(String),
    #[error("unknown event kind {0:?}")]
    UnknownEventKind(String),
    #[error("invalid work id")]
    InvalidWorkId,
    #[error("invalid project id")]
    InvalidProjectId,
    #[error("unknown Work relation kind {0:?}")]
    UnknownRelationKind(String),
    #[error("invalid Work relation")]
    InvalidRelation,
    #[error("invalid execution id")]
    InvalidExecutionId,
    #[error("invalid attempt id")]
    InvalidAttemptId,
    #[error("invalid execution spec")]
    InvalidExecutionSpec,
    #[error("invalid confirmed outcome")]
    InvalidConfirmedOutcome,
    #[error("invalid work attributes")]
    InvalidAttributes,
    #[error("unsupported schema version {0}")]
    UnsupportedSchemaVersion(u32),
    #[error("cannot replay an empty event list")]
    EmptyReplay,
    #[error("complete conflicts with terminal status {0}")]
    TerminalConflict(WorkStatus),
    #[error("cannot bind workspace in status {0}")]
    CannotBindWorkspace(WorkStatus),
}
