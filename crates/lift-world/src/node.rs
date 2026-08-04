//! World Model node types.
//!
//! The World Model is a persistent, versioned knowledge graph that replaces
//! the bounded text context window of long-horizon agents. Nodes represent
//! everything an agent has observed (`Observation`), acted upon (`Action`) or
//! knows about the world (`Entity`).

use serde::{Deserialize, Serialize};

/// Identifier of a node in the World Model graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodeId(pub u64);

/// The kind of a World Model node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NodeKind {
    /// A real-world or abstract thing the agent knows about
    /// (document, contract clause, person, company, law…).
    Entity,
    /// Something the agent has done (a tool call, an LLM inference, a query).
    Action,
    /// Something the agent has observed or learned (a fact, a result).
    Observation,
}

impl NodeKind {
    /// Human-readable label for the kind.
    pub fn as_str(&self) -> &'static str {
        match self {
            NodeKind::Entity => "entity",
            NodeKind::Action => "action",
            NodeKind::Observation => "observation",
        }
    }
}

impl std::fmt::Display for NodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A node in the World Model.
///
/// Nodes carry a stable `id`, a `kind`, a human-readable `label`, and an
/// arbitrary typed `data` payload (serialized as JSON). Every mutation bumps
/// the `revision` so that concurrent readers can detect stale views.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    /// Stable identifier, unique within the World Model.
    pub id: NodeId,
    /// What this node represents.
    pub kind: NodeKind,
    /// Short human-readable name.
    pub label: String,
    /// Typed payload. The schema is free-form JSON so any dialect can attach
    /// structured data (e.g. a tensor from `lift-core`, a tool result, …).
    #[serde(default)]
    pub data: serde_json::Value,
    /// Monotonic revision counter, bumped on every edit.
    pub revision: u64,
    /// Monotonic wall-clock of creation (epoch millis). Not time-travel safe,
    /// it is a display aid; ordering must rely on `revision` / `created_seq`.
    #[serde(default)]
    pub created_at_ms: u64,
}

impl Node {
    /// Creates a new node. The caller supplies a unique `id`.
    pub fn new(id: NodeId, kind: NodeKind, label: impl Into<String>) -> Self {
        Self {
            id,
            kind,
            label: label.into(),
            data: serde_json::Value::Object(Default::default()),
            revision: 0,
            created_at_ms: 0,
        }
    }

    /// Convenience constructor for an `Entity` node.
    pub fn entity(id: NodeId, label: impl Into<String>) -> Self {
        Self::new(id, NodeKind::Entity, label)
    }

    /// Convenience constructor for an `Action` node.
    pub fn action(id: NodeId, label: impl Into<String>) -> Self {
        Self::new(id, NodeKind::Action, label)
    }

    /// Convenience constructor for an `Observation` node.
    pub fn observation(id: NodeId, label: impl Into<String>) -> Self {
        Self::new(id, NodeKind::Observation, label)
    }

    /// Attaches typed data to the node, bumping its revision.
    pub fn with_data(mut self, data: impl Serialize) -> Result<Self, serde_json::Error> {
        self.data = serde_json::to_value(data)?;
        self.revision += 1;
        Ok(self)
    }

    /// Returns true when the node carries no data payload.
    pub fn is_bare(&self) -> bool {
        matches!(&self.data, serde_json::Value::Object(o) if o.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_construction_kinds() {
        let e = Node::entity(NodeId(1), "contract.pdf");
        assert_eq!(e.kind, NodeKind::Entity);
        assert_eq!(e.label, "contract.pdf");
        assert!(e.is_bare());

        let a = Node::action(NodeId(2), "extract_clause");
        assert_eq!(a.kind, NodeKind::Action);

        let o = Node::observation(NodeId(3), "clause 4.2 found");
        assert_eq!(o.kind, NodeKind::Observation);
    }

    #[test]
    fn node_data_attachment_bumps_revision() {
        let mut n = Node::entity(NodeId(1), "invoice");
        assert_eq!(n.revision, 0);
        n = n
            .with_data(serde_json::json!({"amount": 4200, "currency": "EUR"}))
            .expect("valid json");
        assert_eq!(n.revision, 1);
        assert!(!n.is_bare());
        assert_eq!(n.data["amount"], serde_json::json!(4200));
    }

    #[test]
    fn node_serde_roundtrip() {
        let n = Node::entity(NodeId(7), "company")
            .with_data(serde_json::json!({"legal_name": "Acme GmbH"}))
            .unwrap();
        let json = serde_json::to_string(&n).unwrap();
        let back: Node = serde_json::from_str(&json).unwrap();
        assert_eq!(n, back);
    }

    #[test]
    fn node_kind_display() {
        assert_eq!(NodeKind::Entity.to_string(), "entity");
        assert_eq!(NodeKind::Action.to_string(), "action");
        assert_eq!(NodeKind::Observation.to_string(), "observation");
    }
}
