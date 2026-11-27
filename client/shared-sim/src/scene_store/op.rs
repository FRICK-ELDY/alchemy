//! Phase 1b Scene Store ops (`entity_id: u32`) — x00de.
//!
//! Contract sample in roadmap §6 uses `StableUuid` / `wire_slot` (Phase 1c shape).
//! Here `node` / `parent` are [`EntityId`]; `wire_slot` is omitted (wire id = entity).
//! TRASH_UUID deletion is **not** in this slice.

use super::{EntityId, OpTimestamp};

/// Typed property payload for [`CrdtOp::SetProperty`].
///
/// LWW decides by [`OpTimestamp`] only — payload equality is not consulted on
/// apply. `PartialEq` on `F32*` follows IEEE (NaN ≠ NaN); do not use value Eq
/// as a merge key without an explicit total-order policy.
#[derive(Clone, Debug, PartialEq)]
pub enum PropValue {
    Bool(bool),
    U32(u32),
    F32(f32),
    F32x3([f32; 3]),
    F32x4([f32; 4]),
    Bytes(Vec<u8>),
    Blake3([u8; 32]),
}

/// Host-Sequenced CRDT op (Phase 1b keys = [`EntityId`]).
///
/// Caller applies ops in `(host_epoch, room_seq)` order. [`OpTimestamp`] is the
/// property / parent merge key (LWW), not the realtime sequencer.
#[derive(Clone, Debug, PartialEq)]
pub enum CrdtOp {
    /// Insert `node` under `parent` (`None` = world root).
    ///
    /// **Not** a regenerate / reset. If `node` is already live:
    /// - `ts <=` stored parent ts → [`super::ApplyOutcome::RejectedStale`] (equal
    ///   included; not an idempotent `Applied`)
    /// - newer `ts` → parent LWW only ([`super::ApplyOutcome::AppliedAsReparent`]);
    ///   existing props are **kept** (same path as [`Self::MoveNode`])
    ///
    /// Fresh insert (absent id) → [`super::ApplyOutcome::Applied`].
    SpawnNode {
        ts: OpTimestamp,
        node: EntityId,
        parent: Option<EntityId>,
    },
    /// Reparent `node` under `new_parent` (`None` = world root).
    /// Cyclic moves are Reject / No-op ([`super::ApplyOutcome::RejectedCycle`]).
    MoveNode {
        ts: OpTimestamp,
        node: EntityId,
        new_parent: Option<EntityId>,
    },
    /// Property-level LWW write for `(node, prop_key)`.
    SetProperty {
        ts: OpTimestamp,
        node: EntityId,
        prop_key: u32,
        value: PropValue,
    },
}

impl CrdtOp {
    #[inline]
    pub fn timestamp(&self) -> OpTimestamp {
        match self {
            Self::SpawnNode { ts, .. }
            | Self::MoveNode { ts, .. }
            | Self::SetProperty { ts, .. } => *ts,
        }
    }
}
