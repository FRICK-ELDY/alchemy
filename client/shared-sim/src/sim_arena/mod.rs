//! Sim Arena — 3-face SoA buffer scaffold (x00da).
//!
//! Guarantees: `Prev` / `Published` / `Work` face roles rotate at tick end;
//! [`LocalSlotId`]; [`STATE_TELEPORT_THIS_TICK`]; minimal transform SoA columns
//! with equal lengths enforced by private columns + `resize`.
//! Non-guarantees: 2-frame dirty Carry-Over (x00db); FrameBridge / host wiring
//! (x00dc); scene_store / CRDT; physics; Depth Bucket; §6 `COMP_*` /
//! `STATE_IS_GRABBED` (intentional defer — grab SSoT stays on `mvcc` / Epoch).
//!
//! Ring3 readers must not touch Work — publish Prev/Published via existing
//! `EpochSnapshotPair` (wired in x00dc).
//!
//! # `STATE_TELEPORT_THIS_TICK` clear ownership
//!
//! - **Set**: Work writer / `SimPatch` apply (Spawn, SetParent, warp, enabled ON).
//! - **Observe**: Ring 2 Step 1 bitmask edge Observer *detects* the bit (and
//!   enabled edges); it does not own lifetime.
//! - **Rotate / recycle**: [`SimArena::rotate_faces`] moves faces only — the bit
//!   **remains** on the recycled buffer (including when that buffer becomes Work).
//!   Recycle alone is **not** a clear.
//! - **Clear (x00db)**: Step 0 / Carry-Over prep explicitly clears
//!   `STATE_TELEPORT_THIS_TICK` on **Work** after pose Carry-Over, so only this
//!   tick's writers can set it again. Published keeps the bit for Ring3 until
//!   the next rotate.
//! - **x00dc**: publishes Prev/Published as-is; must not invent a second clear
//!   path or leave sticky snap by skipping the x00db Step 0 clear.

#![forbid(unsafe_code)]

mod arena;

pub use arena::{FaceRole, SimArena, SimFace};

/// Internal Sim Arena / SparseSet dense-row index (wire map arrives in Phase 1c).
pub type LocalSlotId = u32;

/// Spawn / SetParent / warp: GPU lerp must snap to Published (roadmap §6).
///
/// Lifetime: set on Work; survives rotate into Published; recycle does not clear;
/// x00db Step 0 clears it on Work (not Observer). See module docs.
pub const STATE_TELEPORT_THIS_TICK: u64 = 1 << 11;
