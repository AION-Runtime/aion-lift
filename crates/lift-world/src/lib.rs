//! AION LIFT — World Model IR.
//!
//! A persistent, versioned knowledge graph that replaces the bounded text
//! context window of long-horizon autonomous agents. Reuses `lift-core` as
//! the graph/serialization substrate (slotmap keys, interning, serde).

pub mod node;
pub mod relation;

pub use node::{Node, NodeId, NodeKind};
pub use relation::{Relation, RelationId, RelationType};
