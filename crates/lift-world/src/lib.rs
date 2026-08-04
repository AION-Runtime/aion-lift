//! AION LIFT — World Model IR.
//!
//! A persistent, versioned knowledge graph that replaces the bounded text
//! context window of long-horizon autonomous agents. Reuses `lift-core` as
//! the graph/serialization substrate (slotmap keys, interning, serde).

pub mod node;
pub mod query;
pub mod relation;
pub mod ser;
pub mod world;

pub use node::{Node, NodeId, NodeKind};
pub use query::{Neighborhood, Query};
pub use relation::{Relation, RelationId, RelationType};
pub use ser::{BinaryFormat, SerializationError};
pub use world::{MutationError, Snapshot, WorldModel};
