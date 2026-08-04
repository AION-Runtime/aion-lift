//! Serialization of the World Model.
//!
//! Two formats are provided:
//! - **JSON** — human-readable, interoperable, used for configs and debugging.
//! - **Compact binary** — a simple length-prefixed framing for speed, used by
//!   the tiered memory backend (`aion-memory`).

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::world::{Snapshot, WorldModel};

/// Errors that can occur while (de)serializing the World Model.
#[derive(Debug, Error)]
pub enum SerializationError {
    #[error("json serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("binary payload is truncated or malformed: {0}")]
    Corrupt(String),
}

/// Compact binary framing of a serialized JSON payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BinaryFormat {
    /// Format magic, enables forward detection of wrong bytes.
    pub magic: [u8; 4],
    /// Format version, for future migrations.
    pub format_version: u8,
    /// The wrapped JSON payload (nodes + relations).
    pub payload: Vec<u8>,
}

impl BinaryFormat {
    /// Magic constant for AION World Model binary snapshots.
    pub const MAGIC: [u8; 4] = *b"AION";

    /// Wraps an already-serialized JSON payload.
    pub fn new(payload: Vec<u8>) -> Self {
        Self {
            magic: Self::MAGIC,
            format_version: 1,
            payload,
        }
    }

    /// Encodes the whole World Model into a self-describing binary snapshot.
    pub fn encode(model: &WorldModel) -> Result<Vec<u8>, SerializationError> {
        let snap = model.export()?;
        Self::encode_snapshot(&snap)
    }

    /// Encodes a snapshot into binary form.
    pub fn encode_snapshot(snap: &Snapshot) -> Result<Vec<u8>, SerializationError> {
        let payload = serde_json::to_vec(snap)?;
        let mut out = Vec::with_capacity(16 + payload.len());
        out.extend_from_slice(&Self::MAGIC);
        out.push(1); // format version
                     // 4-byte big-endian payload length
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(&payload);
        Ok(out)
    }

    /// Decodes a binary snapshot back into a `Snapshot`.
    pub fn decode_snapshot(bytes: &[u8]) -> Result<Snapshot, SerializationError> {
        if bytes.len() < 9 {
            return Err(SerializationError::Corrupt(
                "header shorter than 9 bytes".into(),
            ));
        }
        if bytes[0..4] != Self::MAGIC {
            return Err(SerializationError::Corrupt("bad magic".into()));
        }
        let _format_version = bytes[4];
        let len = u32::from_be_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]) as usize;
        if bytes.len() < 9 + len {
            return Err(SerializationError::Corrupt("payload truncated".into()));
        }
        let payload = &bytes[9..9 + len];
        let snap: Snapshot = serde_json::from_slice(payload)?;
        Ok(snap)
    }

    /// Encodes into binary format.
    pub fn encode_with(self) -> Result<Vec<u8>, SerializationError> {
        let mut out = Vec::with_capacity(16 + self.payload.len());
        out.extend_from_slice(&self.magic);
        out.push(self.format_version);
        out.extend_from_slice(&(self.payload.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.payload);
        Ok(out)
    }

    /// Parses binary bytes into a `BinaryFormat`.
    pub fn parse(bytes: &[u8]) -> Result<Self, SerializationError> {
        if bytes.len() < 9 {
            return Err(SerializationError::Corrupt(
                "header shorter than 9 bytes".into(),
            ));
        }
        if bytes[0..4] != Self::MAGIC {
            return Err(SerializationError::Corrupt("bad magic".into()));
        }
        let len = u32::from_be_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]) as usize;
        if bytes.len() < 9 + len {
            return Err(SerializationError::Corrupt("payload truncated".into()));
        }
        Ok(Self {
            magic: Self::MAGIC,
            format_version: bytes[4],
            payload: bytes[9..9 + len].to_vec(),
        })
    }
}

impl WorldModel {
    /// Exports the current state as a `Snapshot` without recording it in the
    /// history (useful for streaming / backups).
    pub fn export(&self) -> Result<Snapshot, SerializationError> {
        Ok(Snapshot {
            version: self.current_version() + 1,
            nodes: self.nodes_sorted(),
            relations: self.relations_sorted(),
            label: String::new(),
        })
    }

    /// Serializes the current state to JSON bytes.
    pub fn to_json(&self) -> Result<Vec<u8>, SerializationError> {
        Ok(serde_json::to_vec(&self.export()?)?)
    }

    /// Serializes the current state to compact binary bytes.
    pub fn to_binary(&self) -> Result<Vec<u8>, SerializationError> {
        BinaryFormat::encode(self)
    }

    /// Restores a World Model from JSON bytes (replaces all state).
    pub fn from_json(bytes: &[u8]) -> Result<Self, SerializationError> {
        let snap: Snapshot = serde_json::from_slice(bytes)?;
        Ok(Self::from_snapshot(snap))
    }

    /// Restores a World Model from compact binary bytes.
    pub fn from_binary(bytes: &[u8]) -> Result<Self, SerializationError> {
        let snap = BinaryFormat::decode_snapshot(bytes)?;
        Ok(Self::from_snapshot(snap))
    }

    /// Rebuilds a World Model from a snapshot. The rebuilt model starts with
    /// no history and a current version of 0.
    pub fn from_snapshot(snap: Snapshot) -> Self {
        let mut wm = WorldModel::new();
        for n in snap.nodes {
            let _ = wm.insert_node(n);
        }
        for r in snap.relations {
            let _ = wm.insert_relation(r);
        }
        wm
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RelationType, WorldModel};

    #[test]
    fn json_roundtrip_preserves_graph() {
        let mut wm = WorldModel::new();
        let e = wm.add_entity("contract");
        let a = wm.add_action("extract");
        let o = wm.add_observation("clause found");
        wm.relate(e, o, RelationType::HasProperty).unwrap();
        wm.relate(a, o, RelationType::Produces).unwrap();

        let bytes = wm.to_json().unwrap();
        let back = WorldModel::from_json(&bytes).unwrap();
        assert_eq!(back.node_count(), 3);
        assert_eq!(back.relation_count(), 2);
    }

    #[test]
    fn binary_roundtrip_preserves_graph() {
        let mut wm = WorldModel::new();
        let e = wm.add_entity("company");
        let o = wm.add_observation("revenue 4200");
        wm.relate(e, o, RelationType::HasProperty).unwrap();
        wm.snapshot();

        let bytes = wm.to_binary().unwrap();
        assert!(bytes.starts_with(&BinaryFormat::MAGIC));
        let back = WorldModel::from_binary(&bytes).unwrap();
        assert_eq!(back.node_count(), 2);
        assert_eq!(back.relation_count(), 1);
        assert_eq!(back.current_version(), 0);
    }

    #[test]
    fn snapshot_binary_roundtrip() {
        let mut wm = WorldModel::new();
        wm.add_entity("a");
        let v = wm.snapshot();
        let snap = wm.history().iter().find(|s| s.version == v).unwrap();
        let bytes = BinaryFormat::encode_snapshot(snap).unwrap();
        let back = BinaryFormat::decode_snapshot(&bytes).unwrap();
        assert_eq!(&back, snap);
    }

    #[test]
    fn corrupt_binary_is_rejected() {
        let bad = vec![0u8, 1, 2, 3, 4, 5, 6, 7, 8];
        assert!(matches!(
            BinaryFormat::decode_snapshot(&bad),
            Err(SerializationError::Corrupt(_))
        ));

        // Truncated payload
        let mut wm = WorldModel::new();
        wm.add_entity("x");
        let bytes = wm.to_binary().unwrap();
        let truncated = &bytes[..bytes.len() - 5];
        assert!(matches!(
            BinaryFormat::decode_snapshot(truncated),
            Err(SerializationError::Corrupt(_))
        ));
    }
}
