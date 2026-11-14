//! Pre-allocated ring-buffer page arena (x00a3 / Phase 1 DoD).
//!
//! Guarantees: fixed pool; `fork_page` CoW; `commit_transfer` / `commit_share` retain
//! new pages then release overwritten slots; dead [`PageId`] rejected; `release_page`
//! never underflows; tick page-alloc counter.
//! Non-guarantees: host/app wiring; XOR delta; XPBD contact (see [`super::adapter`]).

use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};

use super::page::{HotPhysicsPage, BITMASK_WORDS, HISTORY_TICKS};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PageId(pub u16);

#[derive(Clone, Debug)]
pub struct WorldSnapshot {
    pub tick: u64,
    pub hot_pages: Vec<PageId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArenaError {
    /// `PageId` out of range or refcount is already 0.
    DeadPage(PageId),
    /// Refcount would exceed `u16::MAX`.
    RefcountOverflow(PageId),
    /// `hot_pages` lists the same [`PageId`] more than once.
    DuplicatePage(PageId),
    /// [`MvccPageArena::commit_transfer`] on a page already listed in history.
    ///
    /// Phase 1 intentionally uses presence-in-history only (not rc / slot-excluding
    /// counts): even after `commit_share` leaves a caller ref, transfer is rejected.
    /// CoW path is `fork_page` → `commit_transfer`. Relax later only if dual-owned
    /// transfer is required.
    NotCallerOwned(PageId),
}

/// Fixed pool of [`HotPhysicsPage`] with refcounts (no hot-path page heap growth).
pub struct MvccPageArena {
    pool: Box<[HotPhysicsPage]>,
    ref_counts: Vec<AtomicU16>,
    history_ring: Vec<Option<WorldSnapshot>>,
    alloc_cursor: usize,
    latest_tick: AtomicU64,
    /// Page allocs (`alloc_page` / `fork_page`) since last [`Self::begin_tick`].
    tick_page_allocs: u32,
    /// Lifetime page allocs (never reset; DoD counters).
    total_page_allocs: u64,
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
            tick_page_allocs: 0,
            total_page_allocs: 0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.pool.len()
    }

    pub fn latest_tick(&self) -> u64 {
        self.latest_tick.load(Ordering::Acquire)
    }

    pub fn tick_page_allocs(&self) -> u32 {
        self.tick_page_allocs
    }

    pub fn total_page_allocs(&self) -> u64 {
        self.total_page_allocs
    }

    /// Reset per-tick page-alloc counter (call at the start of each sim tick).
    pub fn begin_tick(&mut self) {
        self.tick_page_allocs = 0;
    }

    pub fn refcount(&self, id: PageId) -> u16 {
        let idx = id.0 as usize;
        if idx >= self.ref_counts.len() {
            return 0;
        }
        self.ref_counts[idx].load(Ordering::Acquire)
    }

    /// Allocate a free page (refcount 0 → 1). Returns `None` if pool exhausted.
    pub fn alloc_page(&mut self) -> Option<PageId> {
        let id = self.alloc_free_slot()?;
        self.pool[id.0 as usize] = HotPhysicsPage::zeroed_page();
        Some(id)
    }

    fn alloc_free_slot(&mut self) -> Option<PageId> {
        let n = self.pool.len();
        for _ in 0..n {
            let idx = self.alloc_cursor % n;
            self.alloc_cursor = idx + 1;
            if self.ref_counts[idx]
                .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                self.tick_page_allocs = self.tick_page_allocs.saturating_add(1);
                self.total_page_allocs = self.total_page_allocs.saturating_add(1);
                return Some(PageId(idx as u16));
            }
        }
        None
    }

    /// Copy-on-Write fork: new page with cleared dirty mask.
    pub fn fork_page(&mut self, src: PageId) -> Option<PageId> {
        let src_idx = src.0 as usize;
        if src_idx >= self.pool.len() || self.ref_counts[src_idx].load(Ordering::Acquire) == 0 {
            return None;
        }
        let dst = self.alloc_free_slot()?;
        let dst_idx = dst.0 as usize;
        self.pool[dst_idx] = self.pool[src_idx];
        self.pool[dst_idx].dirty_entity_mask = [0u64; BITMASK_WORDS];
        Some(dst)
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

    /// Bump one ref. Fails if the page is dead or at `u16::MAX`.
    pub fn retain_page(&self, id: PageId) -> Result<(), ArenaError> {
        let idx = id.0 as usize;
        if idx >= self.ref_counts.len() {
            return Err(ArenaError::DeadPage(id));
        }
        let mut err = None;
        let updated =
            self.ref_counts[idx].try_update(Ordering::AcqRel, Ordering::Acquire, |c| match c {
                0 => {
                    err = Some(ArenaError::DeadPage(id));
                    None
                }
                u16::MAX => {
                    err = Some(ArenaError::RefcountOverflow(id));
                    None
                }
                n => Some(n + 1),
            });
        if updated.is_ok() {
            Ok(())
        } else {
            Err(err.unwrap_or(ArenaError::DeadPage(id)))
        }
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

    /// True if `id` appears in any live history snapshot (unique list entries).
    pub fn page_in_history(&self, id: PageId) -> bool {
        self.history_ring
            .iter()
            .flatten()
            .any(|s| s.hot_pages.contains(&id))
    }

    fn validate_hot_pages_unique_live(&self, hot_pages: &[PageId]) -> Result<(), ArenaError> {
        for (i, &pid) in hot_pages.iter().enumerate() {
            if self.refcount(pid) == 0 {
                return Err(ArenaError::DeadPage(pid));
            }
            if hot_pages[..i].contains(&pid) {
                return Err(ArenaError::DuplicatePage(pid));
            }
        }
        Ok(())
    }

    fn commit_with(
        &mut self,
        tick: u64,
        hot_pages: Vec<PageId>,
        drop_caller_refs: bool,
    ) -> Result<(), ArenaError> {
        self.validate_hot_pages_unique_live(&hot_pages)?;
        if drop_caller_refs {
            // Reject history-listed pages: transfer would drop the only remaining ref
            // after retain/release and leave other slots dangling.
            for &pid in &hot_pages {
                if self.page_in_history(pid) {
                    return Err(ArenaError::NotCallerOwned(pid));
                }
            }
        }
        let slot = (tick as usize) % HISTORY_TICKS;
        let old = self.history_ring[slot].take();
        for (retained, &pid) in hot_pages.iter().enumerate() {
            if let Err(e) = self.retain_page(pid) {
                for &rollback in hot_pages.iter().take(retained) {
                    self.release_page(rollback);
                }
                if let Some(old) = old {
                    self.history_ring[slot] = Some(old);
                }
                return Err(e);
            }
        }
        if let Some(old) = old {
            for pid in old.hot_pages {
                self.release_page(pid);
            }
        }
        if drop_caller_refs {
            for &pid in &hot_pages {
                self.release_page(pid);
            }
        }
        self.history_ring[slot] = Some(WorldSnapshot { tick, hot_pages });
        self.latest_tick.store(tick, Ordering::Release);
        Ok(())
    }

    /// Transfer caller ownership of `hot_pages` into history (CoW tick path).
    ///
    /// Retains each page, releases the overwritten slot, then drops one caller ref
    /// per listed page so alloc/fork cannot leak. Each [`PageId`] must appear once,
    /// be live, and **not** already listed in history (`alloc_page` / `fork_page`
    /// only). Re-listing → [`Self::commit_share`]. Presence-in-history gate is
    /// intentionally stricter than “caller still holds a ref” (see
    /// [`ArenaError::NotCallerOwned`]).
    pub fn commit_transfer(&mut self, tick: u64, hot_pages: Vec<PageId>) -> Result<(), ArenaError> {
        self.commit_with(tick, hot_pages, true)
    }

    /// Share live pages into history **without** consuming a caller ref.
    ///
    /// Use when re-listing a [`PageId`] already owned by history (no fork).
    /// `hot_pages` must be unique; duplicates → [`ArenaError::DuplicatePage`].
    pub fn commit_share(&mut self, tick: u64, hot_pages: Vec<PageId>) -> Result<(), ArenaError> {
        self.commit_with(tick, hot_pages, false)
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
    fn arena_alloc_commit_transfer() {
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
        arena.commit_transfer(1, vec![id]).unwrap();
        assert_eq!(arena.latest_tick(), 1);
        assert!(arena.snapshot_at(1).is_some());
        assert_eq!(arena.refcount(id), 1);
        let id2 = arena.alloc_page().expect("page2");
        arena
            .commit_transfer(1 + HISTORY_TICKS as u64, vec![id2])
            .unwrap();
        assert_eq!(arena.refcount(id), 0);
        assert_eq!(arena.refcount(id2), 1);
    }

    #[test]
    fn commit_transfer_rejects_dead_page() {
        let mut arena = MvccPageArena::new(16);
        let dead = PageId(0);
        assert_eq!(
            arena.commit_transfer(1, vec![dead]),
            Err(ArenaError::DeadPage(dead))
        );
        assert!(arena.snapshot_at(1).is_none());
    }

    #[test]
    fn commit_transfer_rejects_history_only_page() {
        let mut arena = MvccPageArena::new(16);
        let id = arena.alloc_page().expect("page");
        arena.commit_transfer(1, vec![id]).unwrap();
        assert!(arena.page_in_history(id));
        // Mistaken second transfer (should be commit_share) must fail and not dangle.
        assert_eq!(
            arena.commit_transfer(2, vec![id]),
            Err(ArenaError::NotCallerOwned(id))
        );
        assert_eq!(arena.refcount(id), 1);
        assert!(arena.page(id).is_some());
        assert!(arena.snapshot_at(1).is_some());
        assert!(arena.snapshot_at(2).is_none());
    }

    #[test]
    fn commit_rejects_duplicate_page_ids() {
        let mut arena = MvccPageArena::new(16);
        let id = arena.alloc_page().expect("page");
        assert_eq!(
            arena.commit_transfer(1, vec![id, id]),
            Err(ArenaError::DuplicatePage(id))
        );
        assert_eq!(arena.refcount(id), 1);
        assert!(arena.snapshot_at(1).is_none());
        arena.commit_transfer(1, vec![id]).unwrap();
        assert_eq!(
            arena.commit_share(2, vec![id, id]),
            Err(ArenaError::DuplicatePage(id))
        );
        assert_eq!(arena.refcount(id), 1);
    }

    #[test]
    fn recommit_same_page_without_fork_stays_live() {
        let mut arena = MvccPageArena::new(16);
        let id = arena.alloc_page().expect("page");
        arena.commit_transfer(1, vec![id]).unwrap();
        assert_eq!(arena.refcount(id), 1);
        // Same PageId into a different history slot (no fork / no caller ref).
        arena.commit_share(2, vec![id]).unwrap();
        assert_eq!(arena.refcount(id), 2);
        assert!(arena.page(id).is_some());
        let filler = arena.alloc_page().unwrap();
        arena
            .commit_transfer(1 + HISTORY_TICKS as u64, vec![filler])
            .unwrap();
        assert_eq!(arena.refcount(id), 1);
        assert_eq!(arena.refcount(filler), 1);
        assert!(arena.snapshot_at(2).is_some());
        assert!(arena.page(id).is_some());
    }

    #[test]
    fn fork_page_copies_and_clears_dirty() {
        let mut arena = MvccPageArena::new(16);
        let src = arena.alloc_page().expect("src");
        {
            let page = arena.page_mut(src).expect("mut");
            page.set_active(0, true);
            assert!(page.try_set_kinematic_pose(
                0,
                KinematicPoseF32::xyz_quat(2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0)
            ));
            assert_ne!(page.dirty_entity_mask[0] & 1, 0);
        }
        let dst = arena.fork_page(src).expect("fork");
        let src_page = arena.page(src).expect("src");
        let dst_page = arena.page(dst).expect("dst");
        assert_eq!(dst_page.soa.pos_x[0], 2.0);
        assert_eq!(dst_page.dirty_entity_mask, [0u64; BITMASK_WORDS]);
        assert_ne!(src_page.dirty_entity_mask[0] & 1, 0);
        assert_eq!(arena.tick_page_allocs(), 2);
    }
}
