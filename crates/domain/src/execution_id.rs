use std::fmt;
use std::str::FromStr;

use crate::error::DomainError;

/// Stable identity of one logical execution of a Work.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ExecutionId(String);

impl ExecutionId {
    pub fn parse(raw: impl AsRef<str>) -> Result<Self, DomainError> {
        parse_opaque_id(raw.as_ref())
            .map(Self)
            .ok_or(DomainError::InvalidExecutionId)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExecutionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for ExecutionId {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

pub(crate) fn parse_opaque_id(raw: &str) -> Option<String> {
    if raw.is_empty() || raw.len() > 128 {
        return None;
    }
    raw.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        .then(|| raw.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_opaque_safe_id() {
        let id = ExecutionId::parse("exec_550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert_eq!(id.as_str(), "exec_550e8400-e29b-41d4-a716-446655440000");
    }

    #[test]
    fn rejects_path_escape() {
        assert_eq!(
            ExecutionId::parse("../execution"),
            Err(DomainError::InvalidExecutionId)
        );
    }
}
