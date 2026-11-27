//! Scene Store — typed SparseSet skeleton (x00dd / Phase 1b start).
//!
//! Guarantees: [`SparseSet<T>`] with `sparse` / `dense_slots` / `dense_values`;
//! sparse key = [`EntityId`] (`u32`); dense walk of live entries; holes in the
//! sparse table are allowed (gaps between entity ids).
//! Non-guarantees: Host-Sequenced LWW / cycle reject (→ x00de); LiveEditLedger
//! facade / Zenoh edit / Save; TRASH_UUID / SoftLease / 3-layer IDs;
//! `SimPatch` → Arena (→ x00df).
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
//! (x00de+ / x00df). `u32::MAX` is **not** a valid [`EntityId`] (sparse ABSENT).

#![forbid(unsafe_code)]

mod sparse_set;

pub use sparse_set::SparseSet;

/// Phase 1b Scene Store key (wire / Manifest `entity_id`).
///
/// Not a dense row index and not [`crate::sim_arena::LocalSlotId`].
/// Must be host/quarantine-bounded; **must not** be `u32::MAX` (sparse ABSENT).
pub type EntityId = u32;
