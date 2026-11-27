//! Three-layer IDs — `StableUuid` / `WireSlotId` / `LocalSlotId` + `wire_to_local` (x00e4).
//!
//! Guarantees: type aliases matching roadmap §6; [`WireToLocal`] O(1) lookup by
//! dense `WireSlotId` index; Phase 1b [`crate::scene_store::EntityId`] coexistence
//! helpers (identity / low-32 Stable embed); reserved [`ROOT_UUID`] /
//! [`TRASH_UUID`] / [`TRASH_ENTITY_ID`] / [`SLOT_ABSENT`]; bind rejects reserved
//! ids; forward/reverse inconsistency is never auto-repaired.
//! Non-guarantees: PROTOCOL_PIN / EditProposal `entity_id: u32` rewrite; Save /
//! Spatial VCS schema switch to StableUuid; SparseSet key migration off
//! `EntityId`; host / SceneStore ownership of [`WireToLocal`] on the live path
//! (skeleton only — wire when WireSlotId is distributed); freelist compaction;
//! cross-host wire reuse policy.
//!
//! # Caller contract (id bounds)
//!
//! [`WireToLocal`] grows `by_wire` / `by_local` to `max(id)+1` (same shape as
//! [`crate::scene_store::SparseSet`]). Ids must be **host/quarantine-bounded**.
//! Huge or near-[`SLOT_ABSENT`] ids can OOM via Vec grow — this module does not
//! invent a numeric ceiling; the live gate must.
//!
//! # Coexistence (1b `entity_id` ↔ 1c layers)
//!
//! | Layer | Role now | Role later |
//! |:---|:---|:---|
//! | [`crate::scene_store::EntityId`] | SparseSet / edit / Manifest key | retires as sparse key once StableUuid path lands |
//! | [`WireSlotId`] | session 30Hz index (may equal entity id) | host-distributed; distinct from persist id |
//! | [`LocalSlotId`] | Arena / SimPatch row index | may diverge from wire after remap |
//! | [`StableUuid`] | persist / VCS / undo world id (skeleton) | SparseSet / CrdtOp node key |
//!
//! During this slice, live edit and PROTOCOL_PIN keep `entity_id: u32`. Bind
//! tables via [`WireToLocal::bind_identity`] for 1b nodes; do not bump pin.
//! Host/SceneStore do **not** hold a [`WireToLocal`] yet (additive skeleton).

#![forbid(unsafe_code)]

use std::collections::HashMap;

/// External / Scene Store / Spatial VCS / Undo world-stable id (roadmap §6).
pub type StableUuid = u128;

/// Host-epoch session index for high-frequency wire (30Hz dirty, 4-byte).
pub type WireSlotId = u32;

/// Per-peer Sim Arena / dense execution row index (`wire_to_local` target).
///
/// Owned here so Scene Store patches and Arena share one definition (breaks the
/// Phase 1b `u32` alias doc cycle).
pub type LocalSlotId = u32;

/// World-root StableUuid (not a sparse Scene Store row).
pub const ROOT_UUID: StableUuid = 0;

/// Virtual trash StableUuid (roadmap §6). Logical delete = MoveNode here.
pub const TRASH_UUID: StableUuid = 0xFFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFE;

/// Phase 1b wire / sparse key for the virtual trash parent (`TRASH_UUID` low 32).
///
/// Not a SparseSet row and **not** a valid [`WireToLocal`] bind target.
pub const TRASH_ENTITY_ID: u32 = (TRASH_UUID & 0xFFFF_FFFF) as u32;

/// Sparse / wire absent sentinel — never a valid bind target.
///
/// Shared with [`crate::scene_store::SparseSet`] absent encoding (`u32::MAX`).
pub const SLOT_ABSENT: u32 = u32::MAX;

/// Phase 1b embed: `entity_id` in the low 32 bits, high bits zero.
///
/// Not used for [`TRASH_UUID`] (full sentinel). Callers that need trash must
/// compare against `TRASH_UUID` directly.
#[inline]
pub const fn stable_from_entity_id(entity_id: u32) -> StableUuid {
    entity_id as StableUuid
}

/// Inverse of [`stable_from_entity_id`] when high 96 bits are zero.
///
/// Returns `None` for non-embed uuids (including [`TRASH_UUID`]) and for
/// [`SLOT_ABSENT`]-shaped low bits (`u32::MAX`).
#[inline]
pub const fn entity_id_from_stable_embed(stable: StableUuid) -> Option<u32> {
    if stable > u32::MAX as StableUuid {
        return None;
    }
    let id = stable as u32;
    if id == SLOT_ABSENT {
        None
    } else {
        Some(id)
    }
}

/// 1b coexistence: wire slot equals Manifest / edit `entity_id`.
#[inline]
pub const fn wire_from_entity_id(entity_id: u32) -> WireSlotId {
    entity_id
}

/// 1b coexistence: Arena local equals `entity_id` (Scene Store → SimPatch path).
#[inline]
pub const fn local_from_entity_id(entity_id: u32) -> LocalSlotId {
    entity_id
}

#[inline]
pub const fn entity_id_from_wire(wire: WireSlotId) -> Option<u32> {
    if wire == SLOT_ABSENT || wire == TRASH_ENTITY_ID {
        None
    } else {
        Some(wire)
    }
}

#[inline]
const fn is_reserved_slot(id: u32) -> bool {
    id == SLOT_ABSENT || id == TRASH_ENTITY_ID
}

/// O(1) `WireSlotId → LocalSlotId` table (dense index by wire raw value).
///
/// Guarantees: [`Self::get`] is O(1); unbound / out-of-range → `None`;
/// [`SLOT_ABSENT`] / [`TRASH_ENTITY_ID`] cannot be bound; forward/reverse
/// disagreement → [`IdMapError::Inconsistent`] (no silent repair). Optional
/// reverse [`Self::wire_of`] for Arena→net. StableUuid side is a separate
/// HashMap (persist path; not 30Hz).
/// Non-guarantees: numeric id ceiling (caller must quarantine); freelist
/// compaction; cross-host wire reuse; live host ownership (see module docs).
#[derive(Clone, Debug, Default)]
pub struct WireToLocal {
    /// Indexed by `WireSlotId as usize`; [`SLOT_ABSENT`] = unbound.
    by_wire: Vec<LocalSlotId>,
    /// Indexed by `LocalSlotId as usize`; [`SLOT_ABSENT`] = unbound.
    by_local: Vec<WireSlotId>,
    /// Persist / VCS prep — not on the 30Hz dirty path.
    stable_to_wire: HashMap<StableUuid, WireSlotId>,
    wire_to_stable: HashMap<WireSlotId, StableUuid>,
}

/// Bind / unbind errors for [`WireToLocal`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdMapError {
    /// `wire` or `local` is [`SLOT_ABSENT`].
    AbsentSentinel,
    /// `wire` or `local` is [`TRASH_ENTITY_ID`] (virtual parent, not an Arena row).
    ReservedId,
    /// `wire` already mapped to a different local.
    WireTaken,
    /// `local` already mapped to a different wire.
    LocalTaken,
    /// `stable` already mapped to a different wire.
    StableTaken,
    /// `wire` already has a different StableUuid.
    StableConflict,
    /// Forward and reverse maps disagree (corrupt / partial state).
    Inconsistent,
}

impl WireToLocal {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// O(1) wire → local. Missing / unbound → `None`.
    #[inline]
    pub fn get(&self, wire: WireSlotId) -> Option<LocalSlotId> {
        let idx = wire as usize;
        let slot = self.by_wire.get(idx).copied()?;
        if slot == SLOT_ABSENT {
            None
        } else {
            Some(slot)
        }
    }

    /// O(1) local → wire (reverse).
    #[inline]
    pub fn wire_of(&self, local: LocalSlotId) -> Option<WireSlotId> {
        let idx = local as usize;
        let wire = self.by_local.get(idx).copied()?;
        if wire == SLOT_ABSENT {
            None
        } else {
            Some(wire)
        }
    }

    /// Persist-side lookup (HashMap; not the 30Hz hot path).
    #[inline]
    pub fn wire_of_stable(&self, stable: StableUuid) -> Option<WireSlotId> {
        self.stable_to_wire.get(&stable).copied()
    }

    #[inline]
    pub fn stable_of_wire(&self, wire: WireSlotId) -> Option<StableUuid> {
        self.wire_to_stable.get(&wire).copied()
    }

    /// Resolve wire → local in one hop (API name from roadmap §3.1 / §6).
    #[inline]
    pub fn wire_to_local(&self, wire: WireSlotId) -> Option<LocalSlotId> {
        self.get(wire)
    }

    /// Bind wire ↔ local. Idempotent when the same pair is already consistent.
    ///
    /// Never auto-repairs a broken reverse/forward half — returns
    /// [`IdMapError::Inconsistent`] instead.
    pub fn bind(&mut self, wire: WireSlotId, local: LocalSlotId) -> Result<(), IdMapError> {
        Self::reject_reserved(wire, local)?;

        let forward = self.get(wire);
        let reverse = self.wire_of(local);

        match (forward, reverse) {
            (Some(existing_local), Some(existing_wire)) => {
                if existing_local == local && existing_wire == wire {
                    return Ok(());
                }
                if existing_local != local {
                    return Err(IdMapError::WireTaken);
                }
                // forward matches requested local, reverse points elsewhere
                Err(IdMapError::Inconsistent)
            }
            (Some(existing_local), None) => {
                if existing_local != local {
                    return Err(IdMapError::WireTaken);
                }
                // forward ok, reverse missing
                Err(IdMapError::Inconsistent)
            }
            (None, Some(existing_wire)) => {
                if existing_wire != wire {
                    return Err(IdMapError::LocalTaken);
                }
                // reverse claims this wire, forward missing
                Err(IdMapError::Inconsistent)
            }
            (None, None) => {
                self.ensure_wire(wire);
                self.ensure_local(local);
                self.by_wire[wire as usize] = local;
                self.by_local[local as usize] = wire;
                Ok(())
            }
        }
    }

    /// 1b bridge: `wire == local == entity_id` (numeric identity).
    pub fn bind_identity(&mut self, entity_id: u32) -> Result<(), IdMapError> {
        self.bind(
            wire_from_entity_id(entity_id),
            local_from_entity_id(entity_id),
        )
    }

    /// Bind wire ↔ local and attach a StableUuid (persist ledger prep).
    ///
    /// Rejects [`ROOT_UUID`] / [`TRASH_UUID`] — both are virtual (world root /
    /// trash), not sparse/persist node identities.
    pub fn bind_with_stable(
        &mut self,
        wire: WireSlotId,
        local: LocalSlotId,
        stable: StableUuid,
    ) -> Result<(), IdMapError> {
        Self::reject_reserved(wire, local)?;
        if stable == ROOT_UUID || stable == TRASH_UUID {
            return Err(IdMapError::ReservedId);
        }
        if let Some(existing) = self.wire_of_stable(stable) {
            if existing != wire {
                return Err(IdMapError::StableTaken);
            }
        }
        if let Some(existing) = self.stable_of_wire(wire) {
            if existing != stable {
                return Err(IdMapError::StableConflict);
            }
        }
        self.bind(wire, local)?;
        self.stable_to_wire.insert(stable, wire);
        self.wire_to_stable.insert(wire, stable);
        Ok(())
    }

    /// Remove wire ↔ local (and any stable link on that wire).
    pub fn unbind(&mut self, wire: WireSlotId) -> Option<LocalSlotId> {
        let local = self.get(wire)?;
        self.by_wire[wire as usize] = SLOT_ABSENT;
        if let Some(slot) = self.by_local.get_mut(local as usize) {
            if *slot == wire {
                *slot = SLOT_ABSENT;
            }
        }
        if let Some(stable) = self.wire_to_stable.remove(&wire) {
            self.stable_to_wire.remove(&stable);
        }
        Some(local)
    }

    #[inline]
    pub fn len_wire_capacity(&self) -> usize {
        self.by_wire.len()
    }

    #[inline]
    fn reject_reserved(wire: WireSlotId, local: LocalSlotId) -> Result<(), IdMapError> {
        if wire == SLOT_ABSENT || local == SLOT_ABSENT {
            return Err(IdMapError::AbsentSentinel);
        }
        if is_reserved_slot(wire) || is_reserved_slot(local) {
            return Err(IdMapError::ReservedId);
        }
        Ok(())
    }

    /// Grow wire table. Caller must quarantine ids (see module "Caller contract").
    fn ensure_wire(&mut self, wire: WireSlotId) {
        let need = wire as usize + 1;
        if self.by_wire.len() < need {
            self.by_wire.resize(need, SLOT_ABSENT);
        }
    }

    /// Grow local table. Caller must quarantine ids (see module "Caller contract").
    fn ensure_local(&mut self, local: LocalSlotId) {
        let need = local as usize + 1;
        if self.by_local.len() < need {
            self.by_local.resize(need, SLOT_ABSENT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_to_local_o1_bind_get_unbind() {
        let mut map = WireToLocal::new();
        assert_eq!(map.wire_to_local(7), None);
        map.bind(7, 3).expect("bind");
        assert_eq!(map.wire_to_local(7), Some(3));
        assert_eq!(map.wire_of(3), Some(7));
        assert_eq!(map.unbind(7), Some(3));
        assert_eq!(map.wire_to_local(7), None);
        assert_eq!(map.wire_of(3), None);
    }

    #[test]
    fn bind_rejects_absent_trash_and_conflicts() {
        let mut map = WireToLocal::new();
        assert_eq!(map.bind(SLOT_ABSENT, 1), Err(IdMapError::AbsentSentinel));
        assert_eq!(map.bind(1, SLOT_ABSENT), Err(IdMapError::AbsentSentinel));
        assert_eq!(map.bind(TRASH_ENTITY_ID, 1), Err(IdMapError::ReservedId));
        assert_eq!(map.bind(1, TRASH_ENTITY_ID), Err(IdMapError::ReservedId));
        assert_eq!(
            map.bind_identity(TRASH_ENTITY_ID),
            Err(IdMapError::ReservedId)
        );
        map.bind(1, 10).unwrap();
        assert_eq!(map.bind(1, 11), Err(IdMapError::WireTaken));
        assert_eq!(map.bind(2, 10), Err(IdMapError::LocalTaken));
        assert!(map.bind(1, 10).is_ok());
    }

    #[test]
    fn bind_rejects_inconsistent_forward_or_reverse() {
        let mut map = WireToLocal::new();
        map.bind(1, 10).unwrap();
        // Corrupt reverse only: clear reverse, leave forward.
        map.by_local[10] = SLOT_ABSENT;
        assert_eq!(map.bind(1, 10), Err(IdMapError::Inconsistent));

        let mut map = WireToLocal::new();
        map.bind(1, 10).unwrap();
        // Corrupt forward only: clear forward, leave reverse.
        map.by_wire[1] = SLOT_ABSENT;
        assert_eq!(map.bind(1, 10), Err(IdMapError::Inconsistent));
    }

    #[test]
    fn identity_coexistence_with_entity_id() {
        let mut map = WireToLocal::new();
        map.bind_identity(42).unwrap();
        assert_eq!(map.wire_to_local(wire_from_entity_id(42)), Some(42));
        assert_eq!(entity_id_from_wire(42), Some(42));
        assert_eq!(entity_id_from_wire(TRASH_ENTITY_ID), None);
        assert_eq!(local_from_entity_id(42), 42);
    }

    #[test]
    fn stable_embed_roundtrip_and_trash_not_embed() {
        assert_eq!(stable_from_entity_id(99), 99);
        assert_eq!(entity_id_from_stable_embed(99), Some(99));
        assert_eq!(entity_id_from_stable_embed(ROOT_UUID), Some(0));
        assert_eq!(entity_id_from_stable_embed(TRASH_UUID), None);
        assert_eq!(
            entity_id_from_stable_embed(stable_from_entity_id(SLOT_ABSENT)),
            None
        );
        assert_eq!(TRASH_ENTITY_ID, (TRASH_UUID & 0xFFFF_FFFF) as u32);
        assert_ne!(stable_from_entity_id(TRASH_ENTITY_ID), TRASH_UUID);
    }

    #[test]
    fn bind_with_stable_and_lookup() {
        let mut map = WireToLocal::new();
        let stable = stable_from_entity_id(5);
        map.bind_with_stable(5, 5, stable).unwrap();
        assert_eq!(map.wire_of_stable(stable), Some(5));
        assert_eq!(map.stable_of_wire(5), Some(stable));
        assert_eq!(map.wire_to_local(5), Some(5));
        map.unbind(5);
        assert_eq!(map.wire_of_stable(stable), None);
        assert_eq!(map.stable_of_wire(5), None);
    }

    #[test]
    fn stable_taken_and_stable_conflict_are_distinct() {
        let mut map = WireToLocal::new();
        let s1 = stable_from_entity_id(1);
        let s2 = stable_from_entity_id(2);
        map.bind_with_stable(1, 1, s1).unwrap();
        // same stable → different wire
        assert_eq!(map.bind_with_stable(2, 2, s1), Err(IdMapError::StableTaken));
        // same wire → different stable
        assert_eq!(
            map.bind_with_stable(1, 1, s2),
            Err(IdMapError::StableConflict)
        );
        assert_eq!(
            map.bind_with_stable(3, 3, TRASH_UUID),
            Err(IdMapError::ReservedId)
        );
        assert_eq!(
            map.bind_with_stable(3, 3, ROOT_UUID),
            Err(IdMapError::ReservedId)
        );
    }

    #[test]
    fn migration_path_1b_entity_still_resolves_via_identity_map() {
        // Documents coexistence: existing edit/Manifest entity_id path keeps
        // working while WireToLocal is populated with identity bindings.
        let entity_ids: &[u32] = &[1, 2, 10, 100];
        let mut map = WireToLocal::new();
        for &id in entity_ids {
            map.bind_identity(id).unwrap();
            map.bind_with_stable(
                wire_from_entity_id(id),
                local_from_entity_id(id),
                stable_from_entity_id(id),
            )
            .unwrap();
        }
        for &id in entity_ids {
            let wire = wire_from_entity_id(id);
            let local = map.wire_to_local(wire).expect("mapped");
            assert_eq!(local, local_from_entity_id(id));
            assert_eq!(map.stable_of_wire(wire), Some(stable_from_entity_id(id)));
        }
        // PROTOCOL_PIN / edit contract: entity_id remains the live key — map is
        // additive, not a wire schema break.
        assert_eq!(entity_id_from_wire(10), Some(10));
    }
}
