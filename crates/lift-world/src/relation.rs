//! Typed relations between World Model nodes.
//!
//! Relations are the edges of the World Model graph. They are first-class,
//! versioned and queryable — the World Model is a *causal* graph, not a flat
//! list of facts.

use serde::{Deserialize, Serialize};

use crate::node::NodeId;

/// Identifier of a relation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RelationId(pub u64);

/// The type of a relation. Each type has a precise causal or semantic meaning
/// used by the Consistency Checker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RelationType {
    /// `Entity` has property `Value` (entity → observation/entity).
    HasProperty,
    /// `from` was caused by `to` (action → observation).
    CausedBy,
    /// `from` caused / produced `to` (action → observation).
    Produces,
    /// `from` depends on `to` (action → entity/observation).
    DependsOn,
    /// `from` contradicts `to` — the core drift signal.
    Contradicts,
    /// `from` supports / corroborates `to`.
    Supports,
    /// `from` refutes `to`.
    Refutes,
    /// Generic relation with a free-form `data` payload.
    RelatedTo,
}

impl RelationType {
    /// Stable machine name.
    pub fn as_str(&self) -> &'static str {
        match self {
            RelationType::HasProperty => "has_property",
            RelationType::CausedBy => "caused_by",
            RelationType::Produces => "produces",
            RelationType::DependsOn => "depends_on",
            RelationType::Contradicts => "contradicts",
            RelationType::Supports => "supports",
            RelationType::Refutes => "refutes",
            RelationType::RelatedTo => "related_to",
        }
    }

    /// True for relation types that are consistency-relevant (a drift signal).
    pub fn is_conflict(self) -> bool {
        matches!(self, RelationType::Contradicts | RelationType::Refutes)
    }
}

impl std::fmt::Display for RelationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A directed edge `from -> to` between two nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Relation {
    /// Stable identifier, unique within the World Model.
    pub id: RelationId,
    /// Source node.
    pub from: NodeId,
    /// Target node.
    pub to: NodeId,
    /// Edge type.
    pub kind: RelationType,
    /// Free-form payload (e.g. the conflicting facts for `Contradicts`).
    #[serde(default)]
    pub data: serde_json::Value,
    /// Monotonic revision counter, bumped on every edit.
    pub revision: u64,
}

impl Relation {
    /// Creates a new relation. The caller supplies a unique `id`.
    pub fn new(id: RelationId, from: NodeId, to: NodeId, kind: RelationType) -> Self {
        Self {
            id,
            from,
            to,
            kind,
            data: serde_json::Value::Object(Default::default()),
            revision: 0,
        }
    }

    /// Convenience constructor for a `Contradicts` edge with an explanatory
    /// payload describing the two conflicting facts.
    pub fn contradiction(id: RelationId, from: NodeId, to: NodeId, reason: impl Serialize) -> Result<Self, serde_json::Error> {
        let mut r = Self::new(id, from, to, RelationType::Contradicts);
        r.data = serde_json::to_value(reason)?;
        r.revision += 1;
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relation_construction() {
        let r = Relation::new(RelationId(1), NodeId(2), NodeId(3), RelationType::DependsOn);
        assert_eq!(r.from, NodeId(2));
        assert_eq!(r.to, NodeId(3));
        assert_eq!(r.kind, RelationType::DependsOn);
        assert!(!r.kind.is_conflict());
    }

    #[test]
    fn contradiction_carries_reason_and_is_conflict() {
        let r = Relation::contradiction(
            RelationId(5),
            NodeId(1),
            NodeId(2),
            serde_json::json!({"old": "EUR 4,200", "new": "EUR 5,000"}),
        )
        .unwrap();
        assert_eq!(r.kind, RelationType::Contradicts);
        assert!(r.kind.is_conflict());
        assert_eq!(r.data["old"], serde_json::json!("EUR 4,200"));
    }

    #[test]
    fn relation_serde_roundtrip() {
        let r = Relation::new(RelationId(9), NodeId(1), NodeId(2), RelationType::Produces);
        let json = serde_json::to_string(&r).unwrap();
        let back: Relation = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn relation_type_display() {
        assert_eq!(RelationType::Contradicts.to_string(), "contradicts");
        assert_eq!(RelationType::HasProperty.to_string(), "has_property");
        assert!(RelationType::Refutes.is_conflict());
        assert!(!RelationType::Supports.is_conflict());
    }
}
