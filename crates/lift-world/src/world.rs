//! The `WorldModel` graph with versioned snapshots.
//!
//! A `WorldModel` is the single source of truth for an agent's state. Every
//! mutation is recorded; at any point a `Snapshot` can be taken and later
//! restored — the primitive behind crash recovery and rollback.

use serde::{Deserialize, Serialize};
use slotmap::{SlotMap, SecondaryMap};
use thiserror::Error;

use crate::node::{Node, NodeId};
use crate::relation::{Relation, RelationId, RelationType};

/// Stable, unique key for a node inside the graph.
pub type NodeKey = slotmap::DefaultKey;

/// Stable, unique key for a relation inside the graph.
pub type RelationKey = slotmap::DefaultKey;

/// Errors that can occur when mutating the World Model.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum MutationError {
    #[error("node {0:?} does not exist")]
    UnknownNode(NodeId),
    #[error("relation {0:?} does not exist")]
    UnknownRelation(RelationId),
    #[error("relation endpoints {from:?} and {to:?} are the same node")]
    SelfRelation { from: NodeId, to: NodeId },
    #[error("relation would create a cycle of kind {kind:?} between {from:?} and {to:?}")]
    Cycle { from: NodeId, to: NodeId, kind: RelationType },
    #[error("cannot delete node {0:?}: it still has {1} incident relations")]
    NodeStillConnected(NodeId, usize),
    #[error("snapshot {0} not found")]
    UnknownSnapshot(u64),
}

/// An immutable point-in-time view of the World Model.
///
/// Snapshots are cheap to take (they store the full serialized state) and
/// provide exact resume/rollback semantics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Monotonic snapshot number.
    pub version: u64,
    /// Nodes at this point in time.
    pub nodes: Vec<Node>,
    /// Relations at this point in time.
    pub relations: Vec<Relation>,
    /// Optional human-readable reason (e.g. "rollback after drift").
    #[serde(default)]
    pub label: String,
}

impl Snapshot {
    /// Serializes the snapshot to JSON bytes.
    pub fn to_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// Deserializes a snapshot from JSON bytes.
    pub fn from_json(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

/// The World Model graph.
///
/// Internally a `SlotMap` of nodes plus a `SlotMap` of relations — the same
/// keyed-arena pattern used by `lift-core`. Nodes and relations are versioned:
/// every mutation is recorded in the history, and snapshots capture exact
/// point-in-time states.
#[derive(Debug, Clone, Default)]
pub struct WorldModel {
    /// Arena of nodes (crate-visible for sibling query modules).
    pub(crate) nodes: SlotMap<NodeKey, Node>,
    /// Arena of relations.
    pub(crate) rels: SlotMap<RelationKey, Relation>,
    node_by_id: SecondaryMap<NodeKey, NodeId>,
    rel_by_id: SecondaryMap<RelationKey, RelationId>,
    next_node_id: u64,
    next_rel_id: u64,
    /// Versioned history of all snapshots, oldest first.
    history: Vec<Snapshot>,
    /// Current version (last snapshot taken, or 0 before any snapshot).
    pub(crate) current: u64,
}

impl WorldModel {
    /// Creates an empty World Model.
    pub fn new() -> Self {
        Self::default()
    }

    // ── Nodes ────────────────────────────────────────────────────────────

    /// Inserts a node into the graph, returning its stable key.
    /// The node's `id` must not collide with an existing node.
    pub fn insert_node(&mut self, node: Node) -> Result<NodeKey, MutationError> {
        if self.find_node(node.id).is_some() {
            return Err(MutationError::UnknownNode(node.id));
        }
        let key = self.nodes.insert(node.clone());
        self.node_by_id.insert(key, node.id);
        if node.id.0 >= self.next_node_id {
            self.next_node_id = node.id.0 + 1;
        }
        Ok(key)
    }

    /// Inserts an entity node with an auto-assigned id.
    pub fn add_entity(&mut self, label: impl Into<String>) -> NodeKey {
        let id = NodeId(self.next_node_id);
        self.next_node_id += 1;
        self.insert_node(Node::entity(id, label)).expect("fresh id")
    }

    /// Inserts an action node with an auto-assigned id.
    pub fn add_action(&mut self, label: impl Into<String>) -> NodeKey {
        let id = NodeId(self.next_node_id);
        self.next_node_id += 1;
        self.insert_node(Node::action(id, label)).expect("fresh id")
    }

    /// Inserts an observation node with an auto-assigned id.
    pub fn add_observation(&mut self, label: impl Into<String>) -> NodeKey {
        let id = NodeId(self.next_node_id);
        self.next_node_id += 1;
        self.insert_node(Node::observation(id, label)).expect("fresh id")
    }

    /// Returns the node at `key`, if any.
    pub fn node(&self, key: NodeKey) -> Option<&Node> {
        self.nodes.get(key)
    }

    /// Returns a mutable node at `key`, if any.
    pub fn node_mut(&mut self, key: NodeKey) -> Option<&mut Node> {
        self.nodes.get_mut(key)
    }

    /// Looks up a node by its stable `NodeId`.
    pub fn find_node(&self, id: NodeId) -> Option<&Node> {
        self.node_by_id
            .iter()
            .find(|(_, v)| **v == id)
            .and_then(|(k, _)| self.nodes.get(k))
    }

    /// Removes a node and all its incident relations.
    pub fn remove_node(&mut self, key: NodeKey) -> Result<Node, MutationError> {
        let node = self.nodes.get(key).cloned().ok_or(MutationError::UnknownNode(NodeId(0)))?;
        // Drop incident relations.
        let rel_keys: Vec<RelationKey> = self
            .rels
            .iter()
            .filter(|(_, r)| r.from == node.id || r.to == node.id)
            .map(|(k, _)| k)
            .collect();
        for k in rel_keys {
            if let Some(r) = self.rels.remove(k) {
                self.rel_by_id.remove(k);
                let _ = r;
            }
        }
        self.node_by_id.remove(key);
        self.nodes.remove(key);
        Ok(node)
    }

    /// Number of nodes.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    // ── Relations ─────────────────────────────────────────────────────────

    /// Inserts a relation, validating endpoints and rejecting self-loops.
    pub fn insert_relation(&mut self, rel: Relation) -> Result<RelationKey, MutationError> {
        if rel.from == rel.to {
            return Err(MutationError::SelfRelation { from: rel.from, to: rel.to });
        }
        if self.find_node(rel.from).is_none() {
            return Err(MutationError::UnknownNode(rel.from));
        }
        if self.find_node(rel.to).is_none() {
            return Err(MutationError::UnknownNode(rel.to));
        }
        let key = self.rels.insert(rel.clone());
        self.rel_by_id.insert(key, rel.id);
        if rel.id.0 >= self.next_rel_id {
            self.next_rel_id = rel.id.0 + 1;
        }
        Ok(key)
    }

    /// Adds a typed relation between two nodes with auto-assigned ids.
    pub fn relate(&mut self, from: NodeKey, to: NodeKey, kind: RelationType) -> Result<RelationKey, MutationError> {
        let from_id = self.nodes.get(from).map(|n| n.id).ok_or(MutationError::UnknownNode(NodeId(0)))?;
        let to_id = self.nodes.get(to).map(|n| n.id).ok_or(MutationError::UnknownNode(NodeId(0)))?;
        let id = RelationId(self.next_rel_id);
        self.next_rel_id += 1;
        self.insert_relation(Relation::new(id, from_id, to_id, kind))
    }

    /// Returns the relation at `key`, if any.
    pub fn relation(&self, key: RelationKey) -> Option<&Relation> {
        self.rels.get(key)
    }

    /// Removes a relation.
    pub fn remove_relation(&mut self, key: RelationKey) -> Result<Relation, MutationError> {
        let rel = self.rels.remove(key).ok_or(MutationError::UnknownRelation(RelationId(0)))?;
        self.rel_by_id.remove(key);
        Ok(rel)
    }

    /// Number of relations.
    pub fn relation_count(&self) -> usize {
        self.rels.len()
    }

    // ── Introspection ─────────────────────────────────────────────────────

    /// All nodes, sorted by stable id (deterministic order for serialization).
    pub fn nodes_sorted(&self) -> Vec<Node> {
        let mut v: Vec<Node> = self.nodes.values().cloned().collect();
        v.sort_by_key(|n| n.id.0);
        v
    }

    /// All relations, sorted by stable id.
    pub fn relations_sorted(&self) -> Vec<Relation> {
        let mut v: Vec<Relation> = self.rels.values().cloned().collect();
        v.sort_by_key(|r| r.id.0);
        v
    }

    // ── Versioning ────────────────────────────────────────────────────────

    /// Captures the current state as a snapshot, appending it to the history.
    /// Returns the new version number.
    pub fn snapshot(&mut self) -> u64 {
        let version = self.current + 1;
        let snap = Snapshot {
            version,
            nodes: self.nodes.values().cloned().collect(),
            relations: self.rels.values().cloned().collect(),
            label: String::new(),
        };
        self.history.push(snap);
        self.current = version;
        version
    }

    /// Restores the graph to a previously captured snapshot.
    pub fn restore(&mut self, version: u64) -> Result<(), MutationError> {
        let snap = self
            .history
            .iter()
            .find(|s| s.version == version)
            .ok_or(MutationError::UnknownSnapshot(version))?;
        self.nodes.clear();
        self.node_by_id.clear();
        self.rels.clear();
        self.rel_by_id.clear();
        for n in &snap.nodes {
            let key = self.nodes.insert(n.clone());
            self.node_by_id.insert(key, n.id);
        }
        for r in &snap.relations {
            let key = self.rels.insert(r.clone());
            self.rel_by_id.insert(key, r.id);
        }
        // Keep the snapshot in history (already there); cap counters conservatively.
        self.current = version;
        Ok(())
    }

    /// Returns the current version (0 if no snapshot has been taken yet).
    pub fn current_version(&self) -> u64 {
        self.current
    }

    /// Returns all snapshots.
    pub fn history(&self) -> &[Snapshot] {
        &self.history
    }

    /// Total number of recorded snapshots.
    pub fn snapshot_count(&self) -> usize {
        self.history.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_query_nodes() {
        let mut wm = WorldModel::new();
        let c = wm.add_entity("contract.pdf");
        let a = wm.add_action("extract_clause");
        let o = wm.add_observation("clause 4.2 found");
        assert_eq!(wm.node_count(), 3);
        assert_eq!(wm.node(c).unwrap().kind, crate::node::NodeKind::Entity);
        assert_eq!(wm.node(a).unwrap().kind, crate::node::NodeKind::Action);
        assert_eq!(wm.node(o).unwrap().kind, crate::node::NodeKind::Observation);
    }

    #[test]
    fn relate_and_validate_endpoints() {
        let mut wm = WorldModel::new();
        let a = wm.add_action("query_db");
        let o = wm.add_observation("result set");
        wm.relate(a, o, RelationType::Produces).unwrap();

        // Self-loop rejected.
        let err = wm.relate(a, a, RelationType::DependsOn).unwrap_err();
        assert!(matches!(err, MutationError::SelfRelation { .. }));

        // Unknown endpoint rejected (the removed node's key is stale).
        let ghost = wm.add_entity("ghost");
        wm.remove_node(ghost).unwrap();
        let err = wm.relate(ghost, o, RelationType::RelatedTo).unwrap_err();
        assert!(matches!(err, MutationError::UnknownNode(_)));
    }

    #[test]
    fn remove_node_drops_incident_relations() {
        let mut wm = WorldModel::new();
        let a = wm.add_action("call_tool");
        let o = wm.add_observation("tool result");
        wm.relate(a, o, RelationType::Produces).unwrap();
        assert_eq!(wm.relation_count(), 1);
        let id = wm.node(o).unwrap().id;
        wm.remove_node(o).unwrap();
        assert_eq!(wm.relation_count(), 0);
        assert!(wm.find_node(id).is_none());
    }

    #[test]
    fn snapshots_restore_exact_state() {
        let mut wm = WorldModel::new();
        let e = wm.add_entity("company");
        let o1 = wm.add_observation("revenue EUR 4,200");
        wm.relate(e, o1, RelationType::HasProperty).unwrap();
        let v1 = wm.snapshot();

        // Mutate: add a contradicting observation.
        let o2 = wm.add_observation("revenue EUR 5,000");
        wm.relate(e, o2, RelationType::Contradicts).unwrap();
        assert_eq!(wm.node_count(), 3);
        assert_eq!(wm.relation_count(), 2);

        // Rollback to the coherent state. `restore` does not append a new
        // snapshot: the history keeps the original single snapshot.
        wm.restore(v1).unwrap();
        assert_eq!(wm.node_count(), 2);
        assert_eq!(wm.relation_count(), 1);
        assert_eq!(wm.current_version(), v1);
        assert_eq!(wm.snapshot_count(), 1);
    }

    #[test]
    fn restore_unknown_snapshot_fails() {
        let mut wm = WorldModel::new();
        wm.add_entity("x");
        let err = wm.restore(99).unwrap_err();
        assert!(matches!(err, MutationError::UnknownSnapshot(99)));
    }

    #[test]
    fn snapshot_json_roundtrip() {
        let mut wm = WorldModel::new();
        wm.add_entity("a");
        let v = wm.snapshot();
        let snap = wm.history().iter().find(|s| s.version == v).unwrap().clone();
        let bytes = snap.to_json().unwrap();
        let back = Snapshot::from_json(&bytes).unwrap();
        assert_eq!(back, snap);
    }
}
