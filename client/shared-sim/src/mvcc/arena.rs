//! Pre-allocated ring-buffer page arena (x00a3 scaffold).

use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};

use super::page::{HotPhysicsPage, HISTORY_TICKS};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PageId(pub u16);

#[derive(Clone, Debug)]
pub struct WorldSnapshot {
    pub tick: u64,
    pub hot_pages: Vec<PageId>,
}

/// Fixed pool of [`HotPhysicsPage`] with refcounts (no hot-path page alloc).
pub struct MvccPageArena {
    pool: Box<[HotPhysicsPage]>,
    ref_counts: Vec<AtomicU16>,
    history_ring: Vec<Option<WorldSnapshot>>,
    alloc_cursor: usize,
    latest_tick: AtomicU64,
}

impl MvccPageArena {
    pub fn new(pool_capacity_pages: usize) -> Self {
        assert!(
            (16..=u16::MAX as usize).contains(&pool_capacity_pages),
            "pool_capacity_pages out of range"
        );
        let pool = vec![HotPhysicsPage::zeroed_page(); pool_capacity_pages].into_boxed_slice();
        let ref_counts = (0..pool_capacity_pages)
            .map(|_| AtomicU16::new(0))
            .collect();
        let mut history_ring = Vec::with_capacity(HISTORY_TICKS);
        history_ring.resize_with(HISTORY_TICKS, || None);

        Self {
            pool,
            ref_counts,
            history_ring,
            alloc_cursor: 0,
            latest_tick: AtomicU64::new(0),
        }
    }

    pub fn capacity(&self) -> usize {
        self.pool.len()
    }

    pub fn latest_tick(&self) -> u64 {
        self.latest_tick.load(Ordering::Acquire)
    }

    /// Allocate a free page (refcount 0 → 1). Returns `None` if pool exhausted.
    pub fn alloc_page(&mut self) -> Option<PageId> {
        let n = self.pool.len();
        for _ in 0..n {
            let idx = self.alloc_cursor % n;
            self.alloc_cursor = idx + 1;
            if self.ref_counts[idx]
                .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                self.pool[idx] = HotPhysicsPage::zeroed_page();
                return Some(PageId(idx as u16));
            }
        }
        None
    }

    pub fn page_mut(&mut self, id: PageId) -> Option<&mut HotPhysicsPage> {
        let idx = id.0 as usize;
        if idx >= self.pool.len() || self.ref_counts[idx].load(Ordering::Acquire) == 0 {
            return None;
        }
        Some(&mut self.pool[idx])
    }

    pub fn page(&self, id: PageId) -> Option<&HotPhysicsPage> {
        let idx = id.0 as usize;
        if idx >= self.pool.len() || self.ref_counts[idx].load(Ordering::Acquire) == 0 {
            return None;
        }
        Some(&self.pool[idx])
    }

    /// Drop one ref. No-op if already zero (avoids u16 wrap / permanent occupancy).
    pub fn release_page(&self, id: PageId) {
        let idx = id.0 as usize;
        if idx >= self.ref_counts.len() {
            return;
        }
        let _ = self.ref_counts[idx].try_update(Ordering::AcqRel, Ordering::Acquire, |c| {
            if c == 0 {
                None
            } else {
                Some(c - 1)
            }
        });
    }

    /// Record a snapshot for the given tick (ring overwrite).
    ///
    /// Scaffold non-guarantee: does **not** bump page refcounts. Callers must keep
    /// listed pages alive for the history window or treat history as soft hints only.
    pub fn commit_snapshot(&mut self, tick: u64, hot_pages: Vec<PageId>) {
        let slot = (tick as usize) % HISTORY_TICKS;
        self.history_ring[slot] = Some(WorldSnapshot { tick, hot_pages });
        self.latest_tick.store(tick, Ordering::Release);
    }

    pub fn snapshot_at(&self, tick: u64) -> Option<&WorldSnapshot> {
        let slot = (tick as usize) % HISTORY_TICKS;
        self.history_ring[slot].as_ref().filter(|s| s.tick == tick)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mvcc::page::KinematicPoseF32;

    #[test]
    fn arena_alloc_commit_and_release() {
        let mut arena = MvccPageArena::new(16);
        let id = arena.alloc_page().expect("page");
        {
            let page = arena.page_mut(id).expect("mut");
            page.set_active(0, true);
            assert!(page.try_set_kinematic_pose(
                0,
                KinematicPoseF32::xyz_quat(1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0)
            ));
        }
        arena.commit_snapshot(1, vec![id]);
        assert_eq!(arena.latest_tick(), 1);
        assert!(arena.snapshot_at(1).is_some());
        arena.release_page(id);
    }
}
