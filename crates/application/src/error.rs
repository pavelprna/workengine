use std::fmt;

use workengine_domain::{DomainError, WorkId};

/// Application and port errors. Domain errors stay distinguishable for CLI exit codes.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error("work not found: {0}")]
    NotFound(WorkId),
    #[error("store conflict: {0}")]
    Conflict(String),
    #[error("store: {0}")]
    Store(String),
    #[error("workspace: {0}")]
    Workspace(String),
    #[error("worker: {0}")]
    Worker(String),
    #[error("outcome schema: {0}")]
    OutcomeSchema(String),
}

impl AppError {
    pub fn store(err: impl fmt::Display) -> Self {
        Self::Store(err.to_string())
    }

    pub fn workspace(err: impl fmt::Display) -> Self {
        Self::Workspace(err.to_string())
    }

    pub fn worker(err: impl fmt::Display) -> Self {
        Self::Worker(err.to_string())
    }

    pub fn outcome_schema(err: impl fmt::Display) -> Self {
        Self::OutcomeSchema(err.to_string())
    }
}
