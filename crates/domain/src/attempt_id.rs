use std::fmt;
use std::str::FromStr;

use crate::error::DomainError;
use crate::execution_id::parse_opaque_id;

/// Identity of one Worker process attempt within an execution.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AttemptId(String);

impl AttemptId {
    pub fn parse(raw: impl AsRef<str>) -> Result<Self, DomainError> {
        parse_opaque_id(raw.as_ref())
            .map(Self)
            .ok_or(DomainError::InvalidAttemptId)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AttemptId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for AttemptId {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_opaque_safe_id() {
        let id = AttemptId::parse("attempt-01").unwrap();
        assert_eq!(id.as_str(), "attempt-01");
    }

    #[test]
    fn rejects_path_escape() {
        assert_eq!(
            AttemptId::parse("attempt/01"),
            Err(DomainError::InvalidAttemptId)
        );
    }
}
