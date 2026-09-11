use std::fmt;
use std::str::FromStr;

use crate::error::DomainError;

/// Opaque identity of one isolated project/repository queue.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProjectId(String);

impl ProjectId {
    pub fn parse(raw: impl AsRef<str>) -> Result<Self, DomainError> {
        let raw = raw.as_ref();
        if raw.is_empty()
            || raw.len() > 128
            || !raw
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(DomainError::InvalidProjectId);
        }
        Ok(Self(raw.to_owned()))
    }

    pub fn default_project() -> Self {
        Self("default".to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for ProjectId {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_identity_is_path_safe() {
        assert_eq!(ProjectId::default_project().as_str(), "default");
        assert!(ProjectId::parse("repo_1").is_ok());
        assert_eq!(
            ProjectId::parse("../repo"),
            Err(DomainError::InvalidProjectId)
        );
    }
}
