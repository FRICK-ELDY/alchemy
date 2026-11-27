//! Trash — logical delete via MoveNode to [`TRASH_ENTITY_ID`] + count/TTL GC (x00e3).
//!
//! Guarantees: FIFO [`TrashStore`] records; [`TrashStore::gc`] returns ids that
//! exceed [`TRASH_MAX_ENTRIES`] (oldest first) or [`TRASH_TTL_TICKS`];
//! [`TRASH_ENTITY_ID`] mirrors low 32 bits of [`TRASH_UUID`] (Phase 1b key;
//! full StableUuid wiring → x00e4).
//! Non-guarantees: Save body `despawns` projection (host `pending_despawns`);
//! Spatial VCS schema; Undo Intent wire.

use std::collections::VecDeque;

use super::EntityId;

/// StableUuid trash sentinel (roadmap §6). Full 3-layer ID path is x00e4.
pub const TRASH_UUID: u128 = 0xFFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFE;

/// Phase 1b wire / sparse key for the virtual trash parent (`TRASH_UUID` low 32).
///
/// Not seeded into [`super::SceneStore`]; [`super::SceneStore::apply`] allows
/// MoveNode with this parent without `contains`. Must not equal `u32::MAX`
/// (sparse ABSENT).
pub const TRASH_ENTITY_ID: EntityId = (TRASH_UUID & 0xFFFF_FFFF) as EntityId;

/// Max nodes retained under trash before oldest are GC candidates.
pub const TRASH_MAX_ENTRIES: usize = 256;

/// Host ticks after which a trash entry is GC-eligible (~60s at 60Hz).
pub const TRASH_TTL_TICKS: u64 = 3600;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TrashEntry {
    entity_id: EntityId,
    trashed_at_tick: u64,
}

/// Session trash index (FIFO). Does not own Scene Store nodes — caller applies
/// hard remove (`remove_node_sparse`) for ids returned by [`Self::gc`].
#[derive(Clone, Debug, Default)]
pub struct TrashStore {
    entries: VecDeque<TrashEntry>,
}

impl TrashStore {
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[inline]
    pub fn contains(&self, entity_id: EntityId) -> bool {
        self.entries.iter().any(|e| e.entity_id == entity_id)
    }

    /// Record (or refresh) a logical delete. Re-note moves the id to the back.
    pub fn note(&mut self, entity_id: EntityId, now_tick: u64) {
        self.entries.retain(|e| e.entity_id != entity_id);
        self.entries.push_back(TrashEntry {
            entity_id,
            trashed_at_tick: now_tick,
        });
    }

    /// Drop the index row without GC (e.g. Undo restored the node from trash).
    pub fn forget(&mut self, entity_id: EntityId) {
        self.entries.retain(|e| e.entity_id != entity_id);
    }

    /// Ids that must be hard-removed: TTL expired, then count overflow (oldest).
    ///
    /// Removes those rows from the index. Caller must `remove_node_sparse` each
    /// returned id (and clear SoftLease / treat Undo as No-op).
    pub fn gc(&mut self, now_tick: u64) -> Vec<EntityId> {
        self.gc_with_limits(now_tick, TRASH_TTL_TICKS, TRASH_MAX_ENTRIES)
    }

    pub fn gc_with_limits(
        &mut self,
        now_tick: u64,
        ttl_ticks: u64,
        max_entries: usize,
    ) -> Vec<EntityId> {
        let mut doomed = Vec::new();
        while let Some(front) = self.entries.front() {
            let age = now_tick.saturating_sub(front.trashed_at_tick);
            if age < ttl_ticks {
                break;
            }
            doomed.push(self.entries.pop_front().expect("front checked").entity_id);
        }
        while self.entries.len() > max_entries {
            doomed.push(self.entries.pop_front().expect("len > max").entity_id);
        }
        doomed
    }
}

#[inline]
pub fn is_trash_entity_id(id: EntityId) -> bool {
    id == TRASH_ENTITY_ID
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trash_entity_id_matches_uuid_low_bits_and_not_absent() {
        assert_eq!(TRASH_ENTITY_ID, 0xFFFF_FFFE);
        assert_ne!(TRASH_ENTITY_ID, u32::MAX);
        assert_eq!(TRASH_ENTITY_ID, (TRASH_UUID as u32));
    }

    #[test]
    fn note_refresh_moves_to_back() {
        let mut trash = TrashStore::default();
        trash.note(1, 0);
        trash.note(2, 1);
        trash.note(1, 2);
        assert_eq!(trash.len(), 2);
        assert_eq!(trash.entries.front().unwrap().entity_id, 2);
        assert_eq!(trash.entries.back().unwrap().entity_id, 1);
        assert_eq!(trash.entries.back().unwrap().trashed_at_tick, 2);
    }

    #[test]
    fn gc_ttl_returns_expired_fifo() {
        let mut trash = TrashStore::default();
        trash.note(10, 0);
        trash.note(20, 5);
        // age(10)=9 < 10, age(20)=4 < 10 → none
        let doomed = trash.gc_with_limits(9, 10, 256);
        assert!(doomed.is_empty());
        // age(10)=10 >= 5, age(20)=5 >= 5 → both
        let doomed = trash.gc_with_limits(10, 5, 256);
        assert_eq!(doomed, vec![10, 20]);
        assert!(trash.is_empty());
    }

    #[test]
    fn gc_count_overflow_drops_oldest() {
        let mut trash = TrashStore::default();
        trash.note(1, 0);
        trash.note(2, 1);
        trash.note(3, 2);
        let doomed = trash.gc_with_limits(2, 1000, 2);
        assert_eq!(doomed, vec![1]);
        assert_eq!(trash.len(), 2);
        assert!(trash.contains(2));
        assert!(trash.contains(3));
    }

    #[test]
    fn forget_removes_without_gc() {
        let mut trash = TrashStore::default();
        trash.note(7, 0);
        trash.forget(7);
        assert!(!trash.contains(7));
        assert!(trash.gc_with_limits(10_000, 1, 0).is_empty());
    }
}
