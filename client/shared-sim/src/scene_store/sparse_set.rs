//! Typed SparseSet: entity_id → dense pack (x00dd).

use super::EntityId;
use crate::ids::SLOT_ABSENT;

/// Sentinel in [`SparseSet::sparse`]: no live dense entry for that entity id.
///
/// Same value as [`crate::ids::SLOT_ABSENT`] (`u32::MAX`).
const ABSENT: u32 = SLOT_ABSENT;

/// Typed sparse set: sparse key → dense slot, plus packed values.
///
/// Guarantees: O(1) insert / remove / contains by [`EntityId`]; dense iteration
/// visits only live entries in insert-stable order except after swap-remove;
/// sparse table may contain holes (unused ids between used ones).
/// Non-guarantees: parent hierarchy / LWW (see [`crate::scene_store::SceneStore`]);
/// Arena `LocalSlotId` map.
///
/// # Caller contract (id bounds)
///
/// - Sparse grows to `max(entity_id)+1`. Ids must be **host/quarantine-bounded**
///   (Manifest / edit gate). Huge or near-`u32::MAX` ids can OOM via sparse grow.
/// - [`EntityId`] **must not** be [`crate::ids::SLOT_ABSENT`] (`u32::MAX`) —
///   reserved as the sparse absent sentinel ([`ABSENT`]). Insert of that id is
///   rejected (no-op, returns `None`).
#[derive(Clone, Debug)]
pub struct SparseSet<T> {
    /// `entity_id as usize` → dense index, or [`ABSENT`]. Length ≥ max id + 1.
    sparse: Vec<u32>,
    /// Packed live entity ids (same order as [`Self::dense_values`]).
    dense_slots: Vec<EntityId>,
    dense_values: Vec<T>,
}

impl<T> Default for SparseSet<T> {
    fn default() -> Self {
        Self {
            sparse: Vec::new(),
            dense_slots: Vec::new(),
            dense_values: Vec::new(),
        }
    }
}

impl<T> SparseSet<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of live (dense) entries.
    #[inline]
    pub fn len(&self) -> usize {
        self.dense_slots.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.dense_slots.is_empty()
    }

    /// Sparse table length (may exceed live count when ids have holes).
    #[inline]
    pub fn sparse_len(&self) -> usize {
        self.sparse.len()
    }

    #[inline]
    pub fn contains(&self, entity_id: EntityId) -> bool {
        self.dense_index(entity_id).is_some()
    }

    #[inline]
    pub fn get(&self, entity_id: EntityId) -> Option<&T> {
        self.dense_index(entity_id).map(|di| &self.dense_values[di])
    }

    #[inline]
    pub fn get_mut(&mut self, entity_id: EntityId) -> Option<&mut T> {
        self.dense_index(entity_id)
            .map(|di| &mut self.dense_values[di])
    }

    /// Packed live entity ids (dense walk order).
    #[inline]
    pub fn dense_slots(&self) -> &[EntityId] {
        &self.dense_slots
    }

    /// Packed live values (same order as [`Self::dense_slots`]).
    #[inline]
    pub fn dense_values(&self) -> &[T] {
        &self.dense_values
    }

    /// Dense `(entity_id, &value)` pairs in packed order.
    pub fn iter(&self) -> impl Iterator<Item = (EntityId, &T)> {
        self.dense_slots
            .iter()
            .copied()
            .zip(self.dense_values.iter())
    }

    /// Insert or replace. Returns the previous value if the id was already live.
    ///
    /// Rejects `entity_id == u32::MAX` (absent sentinel): returns `None` without
    /// mutating the set (callers must not use this id).
    ///
    /// `None` is **not** proof of a successful new insert — it also means “no
    /// previous value” on first insert, and the same `None` on ABSENT reject.
    /// Reject vs store: pre-check the id, or verify with [`Self::contains`] after.
    pub fn insert(&mut self, entity_id: EntityId, value: T) -> Option<T> {
        if entity_id == ABSENT {
            return None;
        }
        if let Some(di) = self.dense_index(entity_id) {
            return Some(std::mem::replace(&mut self.dense_values[di], value));
        }
        let dense = self.dense_slots.len() as u32;
        self.ensure_sparse(entity_id);
        self.sparse[entity_id as usize] = dense;
        self.dense_slots.push(entity_id);
        self.dense_values.push(value);
        None
    }

    /// Remove by entity id (swap-remove on dense). Returns the removed value.
    pub fn remove(&mut self, entity_id: EntityId) -> Option<T> {
        let di = self.dense_index(entity_id)?;
        debug_assert_eq!(
            self.dense_slots.len(),
            self.dense_values.len(),
            "dense_slots / dense_values length invariant"
        );
        let last = self.dense_slots.len() - 1;
        self.sparse[entity_id as usize] = ABSENT;

        if di != last {
            let moved_id = self.dense_slots[last];
            self.dense_slots.swap(di, last);
            self.dense_values.swap(di, last);
            self.sparse[moved_id as usize] = di as u32;
            debug_assert_eq!(self.dense_slots[di], moved_id);
        }

        self.dense_slots.pop();
        debug_assert_eq!(
            self.dense_values.len(),
            self.dense_slots.len() + 1,
            "dense_values must still hold the removed tail before pop"
        );
        self.dense_values.pop()
    }

    pub fn clear(&mut self) {
        for &id in &self.dense_slots {
            self.sparse[id as usize] = ABSENT;
        }
        self.dense_slots.clear();
        self.dense_values.clear();
    }

    /// Resolve sparse → dense index with reverse-lookup checks (debug).
    #[inline]
    fn dense_index(&self, entity_id: EntityId) -> Option<usize> {
        if entity_id == ABSENT {
            return None;
        }
        let i = entity_id as usize;
        if i >= self.sparse.len() {
            return None;
        }
        let di = self.sparse[i];
        if di == ABSENT {
            return None;
        }
        let di = di as usize;
        debug_assert!(
            di < self.dense_slots.len(),
            "sparse→dense OOB: entity_id={entity_id} di={di} dense_len={}",
            self.dense_slots.len()
        );
        debug_assert_eq!(
            self.dense_slots[di], entity_id,
            "sparse↔dense reverse mismatch at di={di}"
        );
        debug_assert_eq!(
            self.dense_values.len(),
            self.dense_slots.len(),
            "dense_slots / dense_values length invariant"
        );
        Some(di)
    }

    /// Grow sparse to cover `entity_id` (holes filled with [`ABSENT`]).
    ///
    /// Precondition: `entity_id != ABSENT` (enforced by [`Self::insert`]).
    fn ensure_sparse(&mut self, entity_id: EntityId) {
        debug_assert_ne!(entity_id, ABSENT);
        let need = entity_id as usize + 1;
        if self.sparse.len() < need {
            self.sparse.resize(need, ABSENT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_get_and_dense_walk() {
        let mut set = SparseSet::new();
        assert!(set.is_empty());
        assert!(set.insert(2, "b").is_none());
        assert!(set.insert(0, "a").is_none());
        assert!(set.insert(5, "c").is_none());

        assert_eq!(set.len(), 3);
        assert_eq!(set.get(0), Some(&"a"));
        assert_eq!(set.get(2), Some(&"b"));
        assert_eq!(set.get(5), Some(&"c"));
        assert!(!set.contains(1));
        assert!(!set.contains(3));

        // Holes in sparse: ids 1,3,4 unused; table covers max id 5.
        assert!(set.sparse_len() > set.len());
        assert_eq!(set.sparse_len(), 6);

        let collected: Vec<_> = set.iter().map(|(id, v)| (id, *v)).collect();
        assert_eq!(collected, vec![(2, "b"), (0, "a"), (5, "c")]);
        assert_eq!(set.dense_slots(), &[2, 0, 5]);
        assert_eq!(set.dense_values(), &["b", "a", "c"]);
    }

    #[test]
    fn insert_replaces_existing() {
        let mut set = SparseSet::new();
        set.insert(7, 1);
        assert_eq!(set.insert(7, 2), Some(1));
        assert_eq!(set.len(), 1);
        assert_eq!(set.get(7), Some(&2));
        assert_eq!(set.dense_slots(), &[7]);
    }

    #[test]
    fn insert_rejects_absent_sentinel_id() {
        let mut set = SparseSet::new();
        assert!(set.insert(ABSENT, "nope").is_none());
        assert!(set.is_empty());
        assert_eq!(set.sparse_len(), 0);
        assert!(!set.contains(ABSENT));
        assert!(set.get(ABSENT).is_none());
        assert!(set.remove(ABSENT).is_none());
    }

    #[test]
    fn remove_swap_updates_moved_sparse_entry() {
        let mut set = SparseSet::new();
        set.insert(1, 'a');
        set.insert(3, 'b');
        set.insert(8, 'c');
        set.insert(4, 'd');
        assert_eq!(set.dense_slots(), &[1, 3, 8, 4]);
        assert_eq!(set.dense_values(), &['a', 'b', 'c', 'd']);
        assert_eq!(set.sparse_len(), 9);

        // Remove dense index 1 (id=3): last (id=4) swaps into that slot.
        assert_eq!(set.remove(3), Some('b'));
        assert_eq!(set.dense_slots(), &[1, 4, 8]);
        assert_eq!(set.dense_values(), &['a', 'd', 'c']);
        assert!(!set.contains(3));
        assert_eq!(set.sparse_len(), 9);

        // sparse[4] must now resolve to dense index 1 (not the old last index).
        assert_eq!(set.get(4), Some(&'d'));
        assert_eq!(set.dense_slots()[1], 4);
        assert_eq!(set.get(1), Some(&'a'));
        assert_eq!(set.get(8), Some(&'c'));
        assert!(!set.contains(2));
        assert!(!set.contains(5));

        assert_eq!(set.remove(1), Some('a'));
        // last (8) swaps into index 0
        assert_eq!(set.dense_slots(), &[8, 4]);
        assert_eq!(set.dense_values(), &['c', 'd']);
        assert_eq!(set.get(8), Some(&'c'));
        assert_eq!(set.get(4), Some(&'d'));

        assert_eq!(set.remove(8), Some('c'));
        assert_eq!(set.dense_slots(), &[4]);
        assert_eq!(set.remove(4), Some('d'));
        assert!(set.is_empty());
        assert!(set.remove(4).is_none());
        assert_eq!(set.sparse_len(), 9);
    }

    #[test]
    fn dense_walk_skips_removed_and_allows_reinsert() {
        let mut set = SparseSet::new();
        for id in [10u32, 20, 30, 40] {
            set.insert(id, id * 10);
        }
        set.remove(20);
        set.remove(40);

        let walked: Vec<_> = set.iter().map(|(id, v)| (id, *v)).collect();
        assert_eq!(walked.len(), 2);
        assert!(walked.iter().all(|(id, _)| *id == 10 || *id == 30));
        assert!(!set.contains(20));

        set.insert(20, 999);
        assert_eq!(set.get(20), Some(&999));
        assert_eq!(set.len(), 3);
    }

    #[test]
    fn clear_keeps_sparse_table_absent() {
        let mut set = SparseSet::new();
        set.insert(0, 1);
        set.insert(100, 2);
        set.clear();
        assert!(set.is_empty());
        assert!(!set.contains(0));
        assert!(!set.contains(100));
        assert_eq!(set.sparse_len(), 101);
        set.insert(100, 3);
        assert_eq!(set.get(100), Some(&3));
    }

    #[test]
    fn get_mut_updates_dense_value() {
        let mut set = SparseSet::new();
        set.insert(42, 0);
        *set.get_mut(42).expect("present") += 5;
        assert_eq!(set.dense_values(), &[5]);
    }
}
