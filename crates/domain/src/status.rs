use std::fmt;
use std::str::FromStr;

use crate::error::DomainError;

/// Closed lifecycle statuses for the first runtime slice.
///
/// These are not a team's delivery-phase names.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum WorkStatus {
    Ready,
    Running,
    Succeeded,
    Failed,
    Parked,
}

impl WorkStatus {
    pub const ALL: [WorkStatus; 5] = [
        WorkStatus::Ready,
        WorkStatus::Running,
        WorkStatus::Succeeded,
        WorkStatus::Failed,
        WorkStatus::Parked,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            WorkStatus::Ready => "ready",
            WorkStatus::Running => "running",
            WorkStatus::Succeeded => "succeeded",
            WorkStatus::Failed => "failed",
            WorkStatus::Parked => "parked",
        }
    }
}

impl fmt::Display for WorkStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for WorkStatus {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ready" => Ok(WorkStatus::Ready),
            "running" => Ok(WorkStatus::Running),
            "succeeded" => Ok(WorkStatus::Succeeded),
            "failed" => Ok(WorkStatus::Failed),
            "parked" => Ok(WorkStatus::Parked),
            other => Err(DomainError::UnknownStatus(other.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_slice_statuses_are_exactly_the_five() {
        let names: Vec<&str> = WorkStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            names,
            vec!["ready", "running", "succeeded", "failed", "parked"]
        );
    }

    #[test]
    fn unknown_status_is_a_schema_error() {
        assert!(matches!(
            "planning".parse::<WorkStatus>(),
            Err(DomainError::UnknownStatus(_))
        ));
    }
}
