//! Query API over the World Model graph.
//!
//! The World Model replaces the bounded text context: the agent *queries* its
//! history instead of re-reading a context window. This module provides the
//! graph queries used by the agent loop and the Consistency Checker.

use crate::node::{Node, NodeId, NodeKind};
use crate::relation::{Relation, RelationType};
use crate::world::WorldModel;

/// A node together with its direct neighborhood (used by traversal helpers).
#[derive(Debug, Clone, Default)]
pub struct Neighborhood {
    /// Incoming relations: `other -> node`.
    pub incoming: Vec<(RelationType, NodeId)>,
    /// Outgoing relations: `node -> other`.
    pub outgoing: Vec<(RelationType, NodeId)>,
}

impl Neighborhood {
    /// Number of incident relations.
    pub fn degree(&self) -> usize {
        self.incoming.len() + self.outgoing.len()
    }

    /// Neighbor ids (both directions), deduplicated.
    pub fn neighbors(&self) -> Vec<NodeId> {
        let mut v: Vec<NodeId> = self
            .incoming
            .iter()
            .map(|(_, n)| *n)
            .chain(self.outgoing.iter().map(|(_, n)| *n))
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }
}

/// A builder-style query API over the World Model.
#[derive(Debug, Clone)]
pub struct Query<'a> {
    wm: &'a WorldModel,
    kind: Option<NodeKind>,
    label_contains: Option<String>,
    data_path: Option<(String, serde_json::Value)>,
}

impl<'a> Query<'a> {
    /// Starts a query over the given World Model.
    pub fn new(wm: &'a WorldModel) -> Self {
        Self {
            wm,
            kind: None,
            label_contains: None,
            data_path: None,
        }
    }

    /// Restricts to nodes of a given kind.
    pub fn kind(mut self, kind: NodeKind) -> Self {
        self.kind = Some(kind);
        self
    }

    /// Restricts to nodes whose label contains the given substring.
    pub fn label_contains(mut self, needle: impl Into<String>) -> Self {
        self.label_contains = Some(needle.into());
        self
    }

    /// Restricts to nodes whose data payload has a value at the JSON path
    /// (e.g. `"revenue"` or `"meta.currency"`).
    pub fn data(mut self, path: impl Into<String>, value: serde_json::Value) -> Self {
        self.data_path = Some((path.into(), value));
        self
    }

    /// Executes the query, returning all matching nodes (sorted by id).
    pub fn collect(self) -> Vec<Node> {
        let mut out: Vec<Node> = self
            .wm
            .nodes_sorted()
            .into_iter()
            .filter(|n| self.kind.map(|k| n.kind == k).unwrap_or(true))
            .filter(|n| {
                self.label_contains
                    .as_ref()
                    .map(|needle| n.label.contains(needle.as_str()))
                    .unwrap_or(true)
            })
            .filter(|n| {
                self.data_path
                    .as_ref()
                    .map(|(path, expected)| {
                        n.data
                            .pointer(&json_pointer(path))
                            .map(|v| v == expected)
                            .unwrap_or(false)
                    })
                    .unwrap_or(true)
            })
            .collect();
        out.sort_by_key(|n| n.id.0);
        out
    }
}

/// Converts a dot-path (`"a.b.c"`) into a JSON pointer (`"/a/b/c"`).
fn json_pointer(path: &str) -> String {
    let mut out = String::from("/");
    out.push_str(&path.replace('.', "/"));
    out
}

impl WorldModel {
    /// Returns the neighborhood (incoming + outgoing relations) of a node.
    pub fn neighborhood(&self, key: crate::world::NodeKey) -> Option<Neighborhood> {
        let node = self.nodes.get(key)?;
        let mut nb = Neighborhood::default();
        for (_, r) in self.rels.iter() {
            if r.to == node.id {
                nb.incoming.push((r.kind, r.from));
            } else if r.from == node.id {
                nb.outgoing.push((r.kind, r.to));
            }
        }
        nb.incoming.sort_by_key(|(_, n)| *n);
        nb.outgoing.sort_by_key(|(_, n)| *n);
        Some(nb)
    }

    /// Returns all relations incident to a node.
    pub fn incident_relations(&self, key: crate::world::NodeKey) -> Vec<Relation> {
        let Some(node) = self.nodes.get(key) else {
            return Vec::new();
        };
        self.rels
            .values()
            .filter(|r| r.from == node.id || r.to == node.id)
            .cloned()
            .collect()
    }

    /// Returns the node with the given id, or `None`.
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.find_node(id)
    }

    /// Finds all `Contradicts` / `Refutes` relations in the graph — the
    /// primitive used by the Consistency Checker to detect drift.
    pub fn conflicts(&self) -> Vec<Relation> {
        self.rels
            .values()
            .filter(|r| r.kind.is_conflict())
            .cloned()
            .collect()
    }

    /// All observations attached to an entity (direct `HasProperty` edges).
    pub fn properties_of(&self, key: crate::world::NodeKey) -> Vec<Node> {
        let Some(node) = self.nodes.get(key) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (_, r) in self.rels.iter() {
            if r.from == node.id && r.kind == RelationType::HasProperty {
                if let Some(target) = self.find_node(r.to) {
                    out.push(target.clone());
                }
            }
        }
        out.sort_by_key(|n| n.id.0);
        out
    }

    /// The complete causal chain of actions that produced an observation,
    /// walking `Produces` edges backwards from the observation.
    pub fn causal_chain(&self, key: crate::world::NodeKey) -> Vec<Node> {
        let Some(node) = self.nodes.get(key) else {
            return Vec::new();
        };
        let mut chain = Vec::new();
        let mut current = node.id;
        let mut guard = 0;
        while guard < 1024 {
            guard += 1;
            let mut next = None;
            for (_, r) in self.rels.iter() {
                if r.to == current && r.kind == RelationType::Produces {
                    next = Some(r.from);
                    break;
                }
            }
            match next {
                Some(nid) => {
                    if let Some(n) = self.find_node(nid) {
                        chain.push(n.clone());
                        current = nid;
                    } else {
                        break;
                    }
                }
                None => break,
            }
        }
        chain
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RelationType, WorldModel};

    fn build_sample() -> (WorldModel, crate::world::NodeKey, crate::world::NodeKey) {
        let mut wm = WorldModel::new();
        let contract = wm.add_entity("contract.pdf");
        let action = wm.add_action("extract_revenue");
        let o1 = wm.add_observation("revenue EUR 4,200");
        let o2 = wm.add_observation("revenue EUR 5,000");
        wm.relate(contract, o1, RelationType::HasProperty).unwrap();
        wm.relate(action, o1, RelationType::Produces).unwrap();
        wm.relate(o1, o2, RelationType::Contradicts).unwrap();
        (wm, contract, action)
    }

    #[test]
    fn neighborhood_and_degree() {
        let (wm, contract, _) = build_sample();
        let nb = wm.neighborhood(contract).unwrap();
        assert_eq!(nb.degree(), 1);
        assert_eq!(nb.outgoing.len(), 1);
        assert_eq!(nb.incoming.len(), 0);
        assert_eq!(nb.neighbors().len(), 1);
    }

    #[test]
    fn query_by_kind_and_label() {
        let (wm, _, _) = build_sample();
        let observations = Query::new(&wm).kind(NodeKind::Observation).collect();
        assert_eq!(observations.len(), 2);

        // "revenue" matches both observations and the "extract_revenue" action.
        let revenues = Query::new(&wm).label_contains("revenue").collect();
        assert_eq!(revenues.len(), 3);

        let contracts = Query::new(&wm)
            .kind(NodeKind::Entity)
            .label_contains("contract")
            .collect();
        assert_eq!(contracts.len(), 1);
        assert_eq!(contracts[0].label, "contract.pdf");

        // Precise match on the observation text.
        let precise = Query::new(&wm).label_contains("EUR 4,200").collect();
        assert_eq!(precise.len(), 1);
    }

    #[test]
    fn query_by_data_payload() {
        let mut wm = WorldModel::new();
        let e = wm.add_entity("invoice");
        let o = wm.add_observation("amount");
        let okey = o;
        if let Some(n) = wm.node_mut(okey) {
            *n = n
                .clone()
                .with_data(serde_json::json!({"amount": 4200, "meta": {"currency": "EUR"}}))
                .unwrap();
        }
        let _ = e;
        let hits = Query::new(&wm)
            .data("meta.currency", serde_json::json!("EUR"))
            .collect();
        assert_eq!(hits.len(), 1);
        let misses = Query::new(&wm)
            .data("amount", serde_json::json!(999))
            .collect();
        assert!(misses.is_empty());
    }

    #[test]
    fn conflicts_detected() {
        let (wm, _, _) = build_sample();
        let conflicts = wm.conflicts();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kind, RelationType::Contradicts);
    }

    #[test]
    fn causal_chain_from_observation() {
        // Build: action --produces--> o1 ; contract --has_property--> o1.
        let mut wm = WorldModel::new();
        let action = wm.add_action("extract_revenue");
        let o1 = wm.add_observation("revenue EUR 4,200");
        wm.relate(action, o1, RelationType::Produces).unwrap();

        // Walking Produces edges backwards from o1 yields the action.
        let chain = wm.causal_chain(o1);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].label, "extract_revenue");

        // An action with no incoming Produces edge yields an empty chain.
        let chain2 = wm.causal_chain(action);
        assert!(chain2.is_empty());
    }

    #[test]
    fn properties_of_entity() {
        let (wm, contract, _) = build_sample();
        let props = wm.properties_of(contract);
        assert_eq!(props.len(), 1);
        assert_eq!(props[0].label, "revenue EUR 4,200");
    }

    #[test]
    fn json_pointer_conversion() {
        assert_eq!(json_pointer("meta.currency"), "/meta/currency");
        assert_eq!(json_pointer("a"), "/a");
    }
}
