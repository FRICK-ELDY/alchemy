//! Scene Store — SparseSet + Host-Sequenced LWW / cycle reject + SimPatch queue
//! + SoftLease (x00dd / x00de / x00df / x00e0 / x00e2).
//!
//! Guarantees: [`SparseSet<T>`] with `sparse` / `dense_slots` / `dense_values`;
//! sparse key = [`EntityId`] (`u32`); dense walk of live entries; holes in the
//! sparse table are allowed (gaps between entity ids);
//! [`SceneStore`] Property-level LWW + cyclic [`CrdtOp::MoveNode`] Reject
//! (No-op); [`OpTimestamp`] with lamport drift reject gate;
//! [`SimPatch`] queue (Spawn/Move auto-enqueue; dense transform via
//! [`SceneStore::enqueue_patch`]) for Arena Step 0 Drain;
//! host boot / physics-ok seed via [`SceneStore::seed_node`] (no patch);
//! host facade live ops via [`SceneStore::apply_sparse`] (LWW, no patch enqueue);
//! [`SceneStore::remove_node`] clears orphan parents + enqueues [`SimPatch::FreeSlot`];
//! facade despawn uses [`SceneStore::remove_node_sparse`] (no FreeSlot; host
//! `pending_despawns` + FrameBridge own dense free);
//! [`SoftLease`] / [`SoftLeaseTable`] structural edit lock (host gate acquires).
//! Non-guarantees: Zenoh edit / Save I/O / tip 採番 (host facade);
//! TRASH_UUID / 3-layer IDs; Dual-auth JWT path (host); physical grab SSoT;
//! SlotUndo Intent wire (library + host facade peel in x00e1).
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
//! - **Arena slot (Phase 1b)** = same numeric `u32` as `entity_id` when
//!   emitting [`SimPatch`] (no separate wire map yet).
//!
//! # Id aliases
//!
//! [`EntityId`] and [`crate::sim_arena::LocalSlotId`] are both plain `u32`
//! aliases in Phase 1b — the type system will not catch mix-ups. Newtype
//! separation lands with 3-layer IDs (→ x00e4). `u32::MAX` is **not** a valid
//! [`EntityId`] (sparse ABSENT).

#![forbid(unsafe_code)]

mod op;
mod patch;
mod soft_lease;
mod sparse_set;
mod store;
mod timestamp;

pub use op::{CrdtOp, PropValue};
pub use patch::SimPatch;
pub use soft_lease::{
    actor_id_from_sub, SoftLease, SoftLeaseConflict, SoftLeaseTable, SOFT_LEASE_TTL_TICKS,
};
pub use sparse_set::SparseSet;
pub use store::{ApplyOutcome, SceneStore, MAX_PARENT_HOPS};
pub use timestamp::{OpTimestamp, MAX_LAMPORT_DRIFT};

/// Phase 1b Scene Store key (wire / Manifest `entity_id`).
///
/// Not a dense row index and not [`crate::sim_arena::LocalSlotId`].
/// Must be host/quarantine-bounded; **must not** be `u32::MAX` (sparse ABSENT).
pub type EntityId = u32;
