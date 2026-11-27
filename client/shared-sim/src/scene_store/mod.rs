//! Scene Store — SparseSet + Host-Sequenced LWW / cycle reject + SimPatch queue
//! + SoftLease + Trash + 3-layer ID coexistence + op_chain / RoomSnapshot 枠
//!   (x00dd…x00e5).
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
//! logical delete = [`CrdtOp::MoveNode`] to [`TRASH_ENTITY_ID`] + [`TrashStore`]
//! count/TTL GC (hard remove via [`SceneStore::remove_node_sparse`]; host Save
//! still projects `pending_despawns`);
//! [`SoftLease`] / [`SoftLeaseTable`] structural edit lock (host gate acquires);
//! 3-layer ID types + [`crate::ids::WireToLocal`] skeleton (x00e4; live path
//! still keys on [`EntityId`]);
//! [`SequencedOp`] / [`fold_op_chain_hash`] / [`RoomSnapshot`] skeleton (x00e5).
//! Non-guarantees: Zenoh edit / Save I/O / tip 採番 (host facade);
//! PROTOCOL_PIN / edit `entity_id` cutover to StableUuid; Dual-auth JWT path
//! (host); physical grab SSoT; SlotUndo Intent wire (library + host facade peel);
//! wire carriage of `op_chain_hash`; `…/snapshot` / `…/sim_state` replacement;
//! host_loop auto-fold of the op chain.
//!
//! # Live re-Spawn
//!
//! [`CrdtOp::SpawnNode`] on an already-live id is **not** node regenerate:
//! equal/older ts → [`ApplyOutcome::RejectedStale`]; newer ts → parent LWW
//! only ([`ApplyOutcome::AppliedAsReparent`]), props kept.
//!
//! # Sparse vs dense vs 3-layer IDs
//!
//! - **sparse key** = [`EntityId`] (Phase 1b; remains through x00e4 coexistence).
//! - **dense index** = packed live-entry walk index — not `entity_id` itself.
//! - **Arena [`crate::ids::LocalSlotId`]** = Phase 1b still equals `entity_id`
//!   numerically when emitting [`SimPatch`]; remap via [`crate::ids::WireToLocal`]
//!   when wire ≠ local.
//! - **[`crate::ids::StableUuid`] / [`crate::ids::WireSlotId`]** = skeleton only;
//!   CrdtOp / PROTOCOL_PIN edit still use [`EntityId`].
//!
//! # Id aliases
//!
//! [`EntityId`], [`crate::ids::WireSlotId`], and [`crate::ids::LocalSlotId`] are
//! plain `u32` aliases — the type system will not catch mix-ups. Prefer
//! [`crate::ids`] helpers (`wire_from_entity_id` / `local_from_entity_id` /
//! `WireToLocal`) at new call sites. `u32::MAX` is **not** a valid [`EntityId`]
//! (sparse ABSENT). [`TRASH_ENTITY_ID`] is the virtual trash parent (not a
//! sparse row).

#![forbid(unsafe_code)]

mod op;
mod op_chain;
mod patch;
mod room_snapshot;
mod soft_lease;
mod sparse_set;
mod store;
mod timestamp;
mod trash;

pub use op::{CrdtOp, PropValue};
pub use op_chain::{
    digest_crdt_op, digest_crdt_op_into, fold_op_chain, fold_op_chain_hash, fold_op_chain_into,
    SequencedOp, OP_CHAIN_DOMAIN,
};
pub use patch::SimPatch;
pub use room_snapshot::{RoomSnapshot, RoomSnapshotNode, RoomSnapshotPose};
pub use soft_lease::{
    actor_id_from_sub, SoftLease, SoftLeaseConflict, SoftLeaseTable, SOFT_LEASE_TTL_TICKS,
};
pub use sparse_set::SparseSet;
pub use store::{ApplyOutcome, SceneStore, MAX_PARENT_HOPS};
pub use timestamp::{OpTimestamp, MAX_LAMPORT_DRIFT};
pub use trash::{
    is_trash_entity_id, TrashStore, TRASH_ENTITY_ID, TRASH_MAX_ENTRIES, TRASH_TTL_TICKS, TRASH_UUID,
};

/// Phase 1b Scene Store key (wire / Manifest `entity_id`).
///
/// Not a dense row index and not [`crate::ids::LocalSlotId`]. During x00e4
/// coexistence this remains the live edit / SparseSet key; see [`crate::ids`].
/// Must be host/quarantine-bounded; **must not** be `u32::MAX` (sparse ABSENT)
/// and **must not** be [`TRASH_ENTITY_ID`] as a spawnable node.
pub type EntityId = u32;
