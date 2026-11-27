//! Scene Store — SparseSet + Host-Sequenced LWW / cycle reject (x00dd / x00de).
//!
//! Guarantees: [`SparseSet<T>`] with `sparse` / `dense_slots` / `dense_values`;
//! sparse key = [`EntityId`] (`u32`); dense walk of live entries; holes in the
//! sparse table are allowed (gaps between entity ids);
//! [`SceneStore`] Property-level LWW + cyclic [`CrdtOp::MoveNode`] Reject
//! (No-op); [`OpTimestamp`] with lamport drift reject gate.
//! Non-guarantees: LiveEditLedger facade / Zenoh edit / Save (→ x00e0);
//! TRASH_UUID / SoftLease / 3-layer IDs; `SimPatch` → Arena (→ x00df);
//! JWT `sub` → `actor_id` (host gate).
//!
//! # Live re-Spawn
//!
//! [`CrdtOp::SpawnNode`] on an already-live id is **not** node regenerate:
//! equal/older ts → [`ApplyOutcome::RejectedStale`]; newer ts → parent LWW
//! only ([`ApplyOutcome::AppliedAsReparent`]), props kept.
//!
//! # Sparse vs dense
//!
//! - **sparse key** = `entity_id` (Phase 1b; StableUuid is Phase 1c).
//! - **dense index** = packed live-entry walk index — not `entity_id` itself.
//!
//! # Id aliases
//!
//! [`EntityId`] and [`crate::sim_arena::LocalSlotId`] are both plain `u32`
//! aliases in Phase 1b — the type system will not catch mix-ups. Newtype
//! separation is deferred until Scene Store ↔ Arena wiring mixes them more
//! (x00df+). `u32::MAX` is **not** a valid [`EntityId`] (sparse ABSENT).

#![forbid(unsafe_code)]

mod op;
mod sparse_set;
mod store;
mod timestamp;

pub use op::{CrdtOp, PropValue};
pub use sparse_set::SparseSet;
pub use store::{ApplyOutcome, SceneStore, MAX_PARENT_HOPS};
pub use timestamp::{OpTimestamp, MAX_LAMPORT_DRIFT};

/// Phase 1b Scene Store key (wire / Manifest `entity_id`).
///
/// Not a dense row index and not [`crate::sim_arena::LocalSlotId`].
/// Must be host/quarantine-bounded; **must not** be `u32::MAX` (sparse ABSENT).
pub type EntityId = u32;
