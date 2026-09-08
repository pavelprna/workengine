use std::str::FromStr;

use crate::error::DomainError;
use crate::status::WorkStatus;

/// Schema version of the Worker outcome artifact for this slice.
pub const OUTCOME_SCHEMA_VERSION: u32 = 1;

/// Closed set of Worker outcome kinds. Unknown kinds are a schema error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutcomeKind {
    Succeeded,
    Failed,
    TimedOut,
    BudgetExceeded,
    ChannelError,
}

impl OutcomeKind {
    pub const ALL: [OutcomeKind; 5] = [
        OutcomeKind::Succeeded,
        OutcomeKind::Failed,
        OutcomeKind::TimedOut,
        OutcomeKind::BudgetExceeded,
        OutcomeKind::ChannelError,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            OutcomeKind::Succeeded => "succeeded",
            OutcomeKind::Failed => "failed",
            OutcomeKind::TimedOut => "timed_out",
            OutcomeKind::BudgetExceeded => "budget_exceeded",
            OutcomeKind::ChannelError => "channel_error",
        }
    }

    /// Maps a closed outcome onto a Work status. The Worker does not choose this.
    pub fn to_status(self) -> WorkStatus {
        match self {
            OutcomeKind::Succeeded => WorkStatus::Succeeded,
            OutcomeKind::Failed
            | OutcomeKind::TimedOut
            | OutcomeKind::BudgetExceeded
            | OutcomeKind::ChannelError => WorkStatus::Failed,
        }
    }
}

impl FromStr for OutcomeKind {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "succeeded" => Ok(OutcomeKind::Succeeded),
            "failed" => Ok(OutcomeKind::Failed),
            "timed_out" => Ok(OutcomeKind::TimedOut),
            "budget_exceeded" => Ok(OutcomeKind::BudgetExceeded),
            "channel_error" => Ok(OutcomeKind::ChannelError),
            other => Err(DomainError::UnknownOutcomeKind(other.to_owned())),
        }
    }
}

/// Versioned Worker outcome. Authorship is the Worker profile that produced it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Outcome {
    schema_version: u32,
    kind: OutcomeKind,
    worker_profile: String,
}

impl Outcome {
    pub fn new(
        schema_version: u32,
        kind: OutcomeKind,
        worker_profile: impl Into<String>,
    ) -> Result<Self, DomainError> {
        if schema_version != OUTCOME_SCHEMA_VERSION {
            return Err(DomainError::UnsupportedSchemaVersion(schema_version));
        }
        let worker_profile = worker_profile.into();
        if worker_profile.is_empty() {
            return Err(DomainError::InvalidAttributes);
        }
        Ok(Self {
            schema_version,
            kind,
            worker_profile,
        })
    }

    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn kind(&self) -> OutcomeKind {
        self.kind
    }

    pub fn worker_profile(&self) -> &str {
        &self.worker_profile
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_kind_is_a_schema_error() {
        assert!(matches!(
            "needs_review".parse::<OutcomeKind>(),
            Err(DomainError::UnknownOutcomeKind(_))
        ));
    }

    #[test]
    fn unknown_schema_version_is_rejected() {
        let err = Outcome::new(99, OutcomeKind::Succeeded, "stub").unwrap_err();
        assert_eq!(err, DomainError::UnsupportedSchemaVersion(99));
    }

    #[test]
    fn fail_closed_kinds_map_to_failed_status() {
        for kind in [
            OutcomeKind::Failed,
            OutcomeKind::TimedOut,
            OutcomeKind::BudgetExceeded,
            OutcomeKind::ChannelError,
        ] {
            assert_eq!(kind.to_status(), WorkStatus::Failed);
        }
        assert_eq!(OutcomeKind::Succeeded.to_status(), WorkStatus::Succeeded);
    }

    #[test]
    fn first_slice_kinds_are_exactly_the_five() {
        let names: Vec<&str> = OutcomeKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "succeeded",
                "failed",
                "timed_out",
                "budget_exceeded",
                "channel_error"
            ]
        );
    }
}
