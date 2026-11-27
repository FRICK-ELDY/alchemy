//! Sim Arena — 3-face SoA buffer + 2-frame dirty Carry-Over (x00da / x00db).
//!
//! Guarantees: `Prev` / `Published` / `Work` face roles rotate at tick end;
//! [`LocalSlotId`]; [`STATE_TELEPORT_THIS_TICK`]; minimal transform SoA columns
//! with equal lengths enforced by private columns + `resize`;
//! [`SimArena::carry_over`] copies **pos + rot + state_bits** for
//! `prev_dirty ∪ prev2_dirty` only (no full-face copy); column
//! [`SimArena::transform_dirty_epoch`] / [`SimArena::state_bits_dirty_epoch`];
//! Carry-Over clears `STATE_TELEPORT_THIS_TICK` on those **Work** rows;
//! [`SimArena::rotate_faces`] clears it on the face **leaving Published**
//! (lifetime = one Published epoch; prevents 3rd-face sticky snap).
//! Non-guarantees: FrameBridge / host wiring (x00dc); scene_store / CRDT;
//! physics; Depth Bucket; §6 `COMP_*` / `STATE_IS_GRABBED` (intentional defer —
//! grab SSoT stays on `mvcc` / Epoch).
//!
//! Ring3 readers must not touch Work — publish Prev/Published via existing
//! `EpochSnapshotPair` (wired in x00dc).
//!
//! # Tick order
//!
//! 1. [`SimArena::carry_over`] (Step 0)
//! 2. Work writers ([`SimArena::write_pos`] / [`SimArena::write_rot`] /
//!    [`SimArena::write_teleport`]; raw [`SimArena::work_mut`] **must**
//!    [`SimArena::mark_dirty`] — without it the slot misses Carry-Over **and**
//!    Published-leave TELEPORT clear)
//! 3. **host/bridge (x00dc):** publish Prev/Published snapshot **immediately
//!    before** rotate (library itself has no publish step)
//! 4. [`SimArena::rotate_faces`] (role + dirty-ring shift + Published-leave
//!    TELEPORT clear)
//!
//! # `STATE_TELEPORT_THIS_TICK` clear ownership
//!
//! - **Set**: Work writer / `SimPatch` apply (Spawn, SetParent, warp, enabled ON).
//! - **Observe**: Ring 2 Step 1 bitmask edge Observer *detects* the bit (and
//!   enabled edges); it does not own lifetime.
//! - **Work→Published**: bit **survives** rotate so Ring3 can snap for one epoch.
//! - **Published→Prev (rotate)**: [`SimArena::rotate_faces`] **clears** the bit on
//!   `prev_dirty_slots` of the face leaving Published (sparse, not O(n) full
//!   scan). Lifetime ends here — a later recycle must not republish TELEPORT
//!   without a new writer.
//! - **Clear on Work (carry_over)**: after pose/rot/bits row copy from Published,
//!   [`SimArena::carry_over`] clears TELEPORT on those **Work** rows so only this
//!   tick's writers can set it again. Published still holds the bit until the
//!   rotate that ends its epoch.
//! - **x00dc**: publish Prev/Published **as-is**, then rotate. Leave-clear is
//!   in-place on the face becoming Prev — do not hand Ring3 a live face across
//!   rotate, invent a second clear path, or skip x00db Work / leave clear.
//!   Work mutations on the host path should go through `write_*` (not raw
//!   `work_mut` without `mark_dirty`).

#![forbid(unsafe_code)]

mod arena;

pub use arena::{FaceRole, SimArena, SimFace};

/// Internal Sim Arena / SparseSet dense-row index (wire map arrives in Phase 1c).
pub type LocalSlotId = u32;

/// Spawn / SetParent / warp: GPU lerp must snap to Published (roadmap §6).
///
/// Lifetime: set on Work; survives into Published for one epoch; cleared when
/// that face leaves Published ([`SimArena::rotate_faces`]); [`SimArena::carry_over`]
/// also clears it on Work after the row copy (not Observer). See module docs.
pub const STATE_TELEPORT_THIS_TICK: u64 = 1 << 11;
