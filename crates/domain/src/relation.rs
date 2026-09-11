use std::str::FromStr;

use crate::{DomainError, WorkId};

/// Closed relation vocabulary. Direction is always `from` -> `to`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RelationKind {
    ParentChild,
    Blocks,
    Follows,
}

impl RelationKind {
    pub const ALL: [Self; 3] = [Self::ParentChild, Self::Blocks, Self::Follows];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ParentChild => "parent_child",
            Self::Blocks => "blocks",
            Self::Follows => "follows",
        }
    }
}

impl FromStr for RelationKind {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "parent_child" => Ok(Self::ParentChild),
            "blocks" => Ok(Self::Blocks),
            "follows" => Ok(Self::Follows),
            other => Err(DomainError::UnknownRelationKind(other.to_owned())),
        }
    }
}

/// Relation data persisted by the control plane, never an implicit Worker chat.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkRelation {
    from: WorkId,
    to: WorkId,
    kind: RelationKind,
}

impl WorkRelation {
    pub fn new(from: WorkId, to: WorkId, kind: RelationKind) -> Result<Self, DomainError> {
        if from == to {
            return Err(DomainError::InvalidRelation);
        }
        Ok(Self { from, to, kind })
    }

    pub fn from(&self) -> &WorkId {
        &self.from
    }

    pub fn to(&self) -> &WorkId {
        &self.to
    }

    pub fn kind(&self) -> RelationKind {
        self.kind
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relations_are_closed_directional_data() {
        let parent = WorkId::parse("parent").unwrap();
        let child = WorkId::parse("child").unwrap();
        let relation =
            WorkRelation::new(parent.clone(), child.clone(), RelationKind::ParentChild).unwrap();
        assert_eq!(relation.from(), &parent);
        assert_eq!(relation.to(), &child);
        assert_eq!(
            RelationKind::ALL.map(RelationKind::as_str),
            ["parent_child", "blocks", "follows"]
        );
        assert!(WorkRelation::new(parent.clone(), parent, RelationKind::Blocks).is_err());
    }
}
