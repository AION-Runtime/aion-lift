//! AION LIFT — World Model IR.
//!
//! A persistent, versioned knowledge graph that replaces the bounded text
//! context window of long-horizon autonomous agents. Reuses `lift-core` as
//! the graph/serialization substrate (slotmap keys, interning, serde).
//!
//! ## Modules
//!
//! - [`node`] — node types: `Entity`, `Action`, `Observation`
//! - [`relation`] — typed relations between nodes
//! - [`world`] — the `WorldModel` graph with versioning
//! - [`ser`] — JSON and compact binary serialization
//! - [`query`] — query API over the graph
//! - [`verify`] — consistency/drift primitives

pub mod node;
pub mod query;
pub mod relation;
pub mod ser;
pub mod verify;
pub mod world;

pub use node::{Node, NodeId, NodeKind};
pub use query::{Neighborhood, Query};
pub use relation::{Relation, RelationId, RelationType};
pub use ser::{BinaryFormat, SerializationError};
pub use verify::{IssueSeverity, ProposedAction, VerificationIssue, VerificationOutcome, VerificationReport, verify_action};
pub use world::{MutationError, Snapshot, WorldModel};
