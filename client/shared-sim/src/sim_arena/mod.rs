//! Sim Arena — 3-face SoA buffer + 2-frame dirty Carry-Over + SimPatch Drain
//! (x00da / x00db / x00df).
//!
//! Guarantees: `Prev` / `Published` / `Work` face roles rotate at tick end;
//! [`LocalSlotId`]; [`STATE_TELEPORT_THIS_TICK`]; minimal transform SoA columns
//! with equal lengths enforced by private columns + `resize`;
//! [`SimArena::carry_over`] copies **pos + rot + state_bits** for
//! `prev_dirty ∪ prev2_dirty` only (no full-face copy); column
//! [`SimArena::transform_dirty_epoch`] / [`SimArena::state_bits_dirty_epoch`];
//! Carry-Over clears `STATE_TELEPORT_THIS_TICK` on those **Work** rows;
//! [`SimArena::rotate_faces`] clears it on the face **leaving Published**
//! (lifetime = one Published epoch; prevents 3rd-face sticky snap);
//! [`SimArena::apply_patches`] / [`SimArena::step0_carry_and_apply_patches`]
//! drain [`crate::scene_store::SimPatch`] onto Work after Carry-Over (no page
//! swap; pos/rot/scale/parent/masks/box → last write wins; **teleport is sticky
//! OR** — `false` does not clear). Durable parent/scale/masks/box are
//! **Arena-internal** (not face-rotated; not Ring3-published with Published pose
//! — atomic co-publish is a later slice).
//! Non-guarantees: scene_store CRDT apply; physics; Depth Bucket rebuild;
//! §6 `COMP_*` / `STATE_IS_GRABBED` (intentional defer — grab SSoT stays on
//! `sim_pages` / Epoch); SoftLease / drag volatile stream; per-column dirty_epoch for
//! procedural-box (roadmap §4.1 low-freq columns — deferred); durable↔Published
//! atomic snapshot for Ring3.
//!
//! Ring3 readers must not touch Work — publish via
//! [`crate::bridge::publish_arena_then_rotate`] into `EpochSnapshotPair`
//! (x00dc; owned sheets, then rotate). Durable accessors
//! ([`SimArena::parent`] / [`SimArena::scale`] / …) are Work-update-side only
//! until a later publish barrier.
//!
//! # Tick order
//!
//! 1. Step 0: [`SimArena::carry_over`] then [`SimArena::apply_patches`]
//!    (or [`SimArena::step0_carry_and_apply_patches`])
//! 2. Work writers ([`SimArena::write_pos`] / [`SimArena::write_rot`] /
//!    [`SimArena::write_teleport`]; raw [`SimArena::work_mut`] **must**
//!    [`SimArena::mark_dirty`] — without it the slot misses Carry-Over **and**
//!    Published-leave TELEPORT clear)
//! 3. **host/bridge (x00dc):** [`crate::bridge::publish_arena_then_rotate`] —
//!    publish **caller-owned Epoch sheets** (Phase 1a Ring3 SSoT = sheet
//!    `start`/`curr`; not Arena face overlay), then rotate
//! 4. [`SimArena::rotate_faces`] (invoked inside step 3; role + dirty-ring shift +
//!    Published-leave TELEPORT clear)
//!
//! # `STATE_TELEPORT_THIS_TICK` clear ownership
//!
//! - **Set**: Work writer / `SimPatch` apply (Spawn, SetParent, warp, enabled ON).
//! - **Observe / Ring3 (x00eb)**: Arena Work → publish clone
//!   [`crate::bridge::stamp_teleport_flags_from_arena_work`] →
//!   [`crate::tape::WorldTransformf64::FLAG_TELEPORT`] → Epoch GPU snap.
//!   Ring 2 Step 1 bitmask edge Observer *detects* the bit; it does not own
//!   lifetime. Wire Frame teleport bit is still out of scope.
//! - **Work→Published**: bit **survives** rotate for one Arena Published epoch.
//! - **Published→Prev (rotate)**: [`SimArena::rotate_faces`] **clears** the bit on
//!   `prev_dirty_slots` of the face leaving Published (sparse, not O(n) full
//!   scan). Lifetime ends here — a later recycle must not republish TELEPORT
//!   without a new writer.
//! - **Clear on Work (carry_over)**: after pose/rot/bits row copy from Published,
//!   [`SimArena::carry_over`] clears TELEPORT on those **Work** rows so only this
//!   tick's writers can set it again. Published still holds the bit until the
//!   rotate that ends its epoch.
//! - **x00dc**: [`crate::bridge::publish_arena_then_rotate`] publishes **caller
//!   sheets** then rotate. Pure-Arena tests build sheets from faces first.
//!   Leave-clear is in-place on the face becoming Prev — do not hand Ring3 a
//!   live face across rotate, invent a second clear path, or skip x00db Work /
//!   leave clear. Work mutations on the host path should go through `write_*`
//!   (not raw `work_mut` without `mark_dirty`).

#![forbid(unsafe_code)]

mod arena;

pub use arena::{FaceRole, SimArena, SimFace};

/// Internal Sim Arena row index (definition owned by [`crate::ids`]; x00e4).
pub use crate::ids::LocalSlotId;

/// Spawn / SetParent / warp: GPU lerp must snap to Published (roadmap §6).
///
/// Lifetime: set on Work; survives into Published for one epoch; cleared when
/// that face leaves Published ([`SimArena::rotate_faces`]); [`SimArena::carry_over`]
/// also clears it on Work after the row copy (not Observer). See module docs.
pub const STATE_TELEPORT_THIS_TICK: u64 = 1 << 11;
