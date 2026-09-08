use std::fmt;
use std::str::FromStr;

use crate::error::DomainError;

/// Stable identifier for one Work. Opaque; not a product or tracker key.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WorkId(String);

impl WorkId {
    /// Parse a Work id. Rejects empty values and path separators so a
    /// workspace directory can be derived from the id without escaping the
    /// assigned root.
    pub fn parse(raw: impl AsRef<str>) -> Result<Self, DomainError> {
        let raw = raw.as_ref();
        if raw.is_empty() || raw.len() > 128 {
            return Err(DomainError::InvalidWorkId);
        }
        if !raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(DomainError::InvalidWorkId);
        }
        Ok(Self(raw.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WorkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for WorkId {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_uuid_shape() {
        let id = WorkId::parse("550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert_eq!(id.as_str(), "550e8400-e29b-41d4-a716-446655440000");
    }

    #[test]
    fn rejects_path_escape() {
        assert_eq!(WorkId::parse("../etc"), Err(DomainError::InvalidWorkId));
        assert_eq!(WorkId::parse("a/b"), Err(DomainError::InvalidWorkId));
        assert_eq!(WorkId::parse(""), Err(DomainError::InvalidWorkId));
    }
}
