# 🧠 AION LIFT — World Model IR

**The persistent, versioned knowledge graph that replaces the bounded text context window of long-horizon autonomous agents.**

Part of [AION-Runtime](https://github.com/AION-Runtime) — the AI Operating System for long-horizon agents.

## What this is

`aion-lift` defines the **World Model format** for the AION runtime:

- **Node types** — `Entity` (things the agent knows), `Action` (things the agent did), `Observation` (things the agent learned).
- **Typed relations** — `HasProperty`, `Produces`, `CausedBy`, `DependsOn`, `Contradicts`, `Supports`, `Refutes` — the causal edges the Consistency Checker reasons over.
- **Versioned snapshots** — every state is snapshotable and restorable: the primitive behind crash recovery and rollback.
- **Serialization** — human-readable JSON and compact self-describing binary framing.
- **Query API** — `Query`, `neighborhood`, `conflicts`, `properties_of`, `causal_chain`.

## Reuse of LIFT

The graph/serialization substrate is **reused from [LIFT](https://github.com/rustnew/Litf-IR)** (SSA IR foundation, MIT):

- `vendor/lift-core` — vendored, unmodified `lift-core` crate (slotmap arenas, interning, serde).
- `crates/lift-world` — the AION World Model layer built on top.

## Workspace layout

```
aion-lift/
├── vendor/
│   └── lift-core/        # vendored LIFT IR substrate
└── crates/
    └── lift-world/       # the World Model IR (nodes, relations, versioning, query, verify)
```

## Quick start

```rust
use lift_world::{WorldModel, RelationType};

let mut wm = WorldModel::new();
let contract = wm.add_entity("contract.pdf");
let action = wm.add_action("extract_revenue");
let obs = wm.add_observation("revenue EUR 4,200");
wm.relate(action, obs, RelationType::Produces)?;
wm.relate(contract, obs, RelationType::HasProperty)?;
let v1 = wm.snapshot();                 // versioned checkpoint
let bytes = wm.to_binary()?;            // persist
let restored = WorldModel::from_binary(&bytes)?; // exact resume
```

## Test

```sh
cargo test
```

## License

Apache-2.0 (vendored `lift-core` remains MIT, see its Cargo.toml).
