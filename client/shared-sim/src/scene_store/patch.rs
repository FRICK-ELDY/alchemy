//! Ring 1 → Ring 2 dense patch queue (x00df).
//!
//! Scene Store owns the sparse SSoT; dense execution diffs cross the barrier only
//! as [`SimPatch`] (no page swap). Arena drains the queue at Step 0 after
//! Carry-Over so same-slot consecutive patches resolve to the **last** write for
//! pos / rot / scale / parent / masks / box (Lost Update prevention).
//! [`SimPatch::SetLocalTransform::teleport`] is **sticky OR** on Drain (see field).
//!
//! # Module placement
//!
//! This module imports [`LocalSlotId`] from `sim_arena` while Arena Drain imports
//! [`SimPatch`] here (Phase 1b `u32` alias cycle). Newtype / shared id mod lands
//! with 3-layer IDs (→ x00e4).

use crate::sim_arena::LocalSlotId;

/// Dense diff from Scene Store (Ring 1) to Sim Arena Work (Ring 2).
///
/// Phase 1b: [`LocalSlotId`] equals Scene Store [`super::EntityId`] numerically
/// (wire id = slot). Dedicated newtypes arrive with 3-layer IDs (→ x00e4).
///
/// Guarantees: variants cover Allocate / Free / SetParent / transform / bit
/// masks / procedural box (roadmap §6). Non-guarantees: SoftLease; drag
/// volatile stream; Depth Bucket rebuild on SetParent (optional later).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SimPatch {
    AllocateSlot {
        slot: LocalSlotId,
        /// `None` = world root. `Some(u32::MAX)` is illegal (Arena coerces to root).
        parent: Option<LocalSlotId>,
    },
    FreeSlot {
        slot: LocalSlotId,
    },
    SetParent {
        slot: LocalSlotId,
        /// `None` = world root. `Some(u32::MAX)` is illegal (Arena coerces to root).
        parent: Option<LocalSlotId>,
    },
    SetLocalTransform {
        slot: LocalSlotId,
        pos: [f32; 3],
        rot: [f32; 4],
        scale: [f32; 3],
        /// Sticky OR on Drain: `true` sets [`crate::sim_arena::STATE_TELEPORT_THIS_TICK`];
        /// `false` does **not** clear a bit already set this tick (e.g. by
        /// Allocate / SetParent). Pose fields still last-write-wins.
        teleport: bool,
    },
    SetAttachedBits {
        slot: LocalSlotId,
        add: u64,
        remove: u64,
    },
    SetEnabledBits {
        slot: LocalSlotId,
        add: u64,
        remove: u64,
    },
    SetProceduralBox {
        slot: LocalSlotId,
        half_extents: [f32; 3],
        corner_radius: f32,
    },
}
