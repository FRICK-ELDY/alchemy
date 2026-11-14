//! Pre-allocated ring-buffer page arena (x00a3 / x00a4 / x00a6 true CoW).
//!
//! Guarantees: fixed pool; `fork_page` CoW; [`Self::make_mut`] (history tip fork /
//! rc>1 fork+release); exclusive-only [`Self::page_mut`]; `commit_transfer_slice` /
//! `commit_share_slice` retain-then-release without an extra page-id `Vec`;
//! fixed [`SnapshotSlot`] + [`MAX_PAGES_PER_SNAPSHOT`] Err; tick page-alloc counter.
//! Non-guarantees: host/app wiring; XOR delta; XPBD contact (see [`super::adapter`]).

use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};

use super::page::{HotPhysicsPage, BITMASK_WORDS, HISTORY_TICKS, MAX_PAGES_PER_SNAPSHOT};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PageId(pub u16);

/// Fixed-length history entry (no per-commit `Vec` alloc on the hot path).
#[derive(Clone, Copy, Debug)]
pub struct SnapshotSlot {
    pub tick: u64,
    pub len: usize,
    pub pages: [PageId; MAX_PAGES_PER_SNAPSHOT],
}

impl SnapshotSlot {
    pub fn pages_slice(&self) -> &[PageId] {
        &self.pages[..self.len]
    }

    fn from_pages(tick: u64, hot_pages: &[PageId]) -> Result<Self, ArenaError> {
        if hot_pages.len() > MAX_PAGES_PER_SNAPSHOT {
            return Err(ArenaError::TooManyPages {
                len: hot_pages.len(),
                max: MAX_PAGES_PER_SNAPSHOT,
            });
        }
        let mut pages = [PageId(0); MAX_PAGES_PER_SNAPSHOT];
        pages[..hot_pages.len()].copy_from_slice(hot_pages);
        Ok(Self {
            tick,
            len: hot_pages.len(),
            pages,
        })
    }
}

/// Backward-compatible view name used by older tests / docs.
pub type WorldSnapshot = SnapshotSlot;

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
    /// CoW path is `fork_page` / `make_mut` → `commit_transfer`. Relax later only if
    /// dual-owned transfer is required.
    NotCallerOwned(PageId),
    /// Snapshot lists more pages than [`MAX_PAGES_PER_SNAPSHOT`].
    TooManyPages { len: usize, max: usize },
    /// Pool has no free slot for fork / alloc.
    PoolExhausted,
}

/// Fixed pool of [`HotPhysicsPage`] with refcounts (no hot-path page heap growth).
pub struct MvccPageArena {
    pool: Box<[HotPhysicsPage]>,
    ref_counts: Vec<AtomicU16>,
    history_ring: Box<[Option<SnapshotSlot>]>,
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
        let history_ring = vec![None; HISTORY_TICKS].into_boxed_slice();

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

    /// True CoW mutable access (Arc::make_mut–style).
    ///
    /// - Exclusive (`rc == 1` and not in history): mutate in place.
    /// - History tip only (`rc == 1` and in history): fork; **do not** release the tip
    ///   (primary tick path after [`Self::commit_transfer`]).
    /// - Shared (`rc > 1`): fork then [`Self::release_page`] the old id (caller's share).
    ///   Precondition: the caller must own one of those refs (e.g. after [`Self::commit_share`]
    ///   or an explicit [`Self::retain_page`]). Do not call on a bare history handle when
    ///   `rc > 1` from multiple history slots alone.
    pub fn make_mut(&mut self, id: &mut PageId) -> Result<&mut HotPhysicsPage, ArenaError> {
        let old = *id;
        let idx = old.0 as usize;
        if idx >= self.pool.len() {
            return Err(ArenaError::DeadPage(old));
        }
        let rc = self.refcount(old);
        if rc == 0 {
            return Err(ArenaError::DeadPage(old));
        }
        let in_history = self.page_in_history(old);
        if rc > 1 || in_history {
            let forked = self.fork_page(old).ok_or(ArenaError::PoolExhausted)?;
            if rc > 1 {
                self.release_page(old);
            }
            *id = forked;
        }
        let idx = id.0 as usize;
        Ok(&mut self.pool[idx])
    }

    /// Exclusive mutable access only. Returns `None` if dead, in history, or `refcount > 1`.
    /// Prefer [`Self::make_mut`] on the tick path.
    pub fn page_mut(&mut self, id: PageId) -> Option<&mut HotPhysicsPage> {
        let idx = id.0 as usize;
        if idx >= self.pool.len() || self.ref_counts[idx].load(Ordering::Acquire) == 0 {
            return None;
        }
        if self.refcount(id) > 1 || self.page_in_history(id) {
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
            .any(|s| s.pages_slice().contains(&id))
    }

    fn validate_hot_pages_unique_live(&self, hot_pages: &[PageId]) -> Result<(), ArenaError> {
        if hot_pages.len() > MAX_PAGES_PER_SNAPSHOT {
            return Err(ArenaError::TooManyPages {
                len: hot_pages.len(),
                max: MAX_PAGES_PER_SNAPSHOT,
            });
        }
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
        hot_pages: &[PageId],
        drop_caller_refs: bool,
    ) -> Result<(), ArenaError> {
        self.validate_hot_pages_unique_live(hot_pages)?;
        if drop_caller_refs {
            for &pid in hot_pages {
                if self.page_in_history(pid) {
                    return Err(ArenaError::NotCallerOwned(pid));
                }
            }
        }
        let slot = SnapshotSlot::from_pages(tick, hot_pages)?;
        let hist_i = (tick as usize) % HISTORY_TICKS;
        let old = self.history_ring[hist_i].take();
        for (retained, &pid) in hot_pages.iter().enumerate() {
            if let Err(e) = self.retain_page(pid) {
                for &rollback in hot_pages.iter().take(retained) {
                    self.release_page(rollback);
                }
                if let Some(old) = old {
                    self.history_ring[hist_i] = Some(old);
                }
                return Err(e);
            }
        }
        if let Some(old) = old {
            for &pid in old.pages_slice() {
                self.release_page(pid);
            }
        }
        if drop_caller_refs {
            for &pid in hot_pages {
                self.release_page(pid);
            }
        }
        self.history_ring[hist_i] = Some(slot);
        self.latest_tick.store(tick, Ordering::Release);
        Ok(())
    }

    /// Transfer caller ownership of `hot_pages` into history (CoW tick path).
    pub fn commit_transfer(&mut self, tick: u64, hot_pages: Vec<PageId>) -> Result<(), ArenaError> {
        self.commit_transfer_slice(tick, &hot_pages)
    }

    /// Zero-extra-Vec commit: transfer caller ownership into history.
    pub fn commit_transfer_slice(
        &mut self,
        tick: u64,
        hot_pages: &[PageId],
    ) -> Result<(), ArenaError> {
        self.commit_with(tick, hot_pages, true)
    }

    /// Share live pages into history **without** consuming a caller ref.
    pub fn commit_share(&mut self, tick: u64, hot_pages: Vec<PageId>) -> Result<(), ArenaError> {
        self.commit_share_slice(tick, &hot_pages)
    }

    /// Zero-extra-Vec share into history (not the CoW tick transfer path).
    pub fn commit_share_slice(
        &mut self,
        tick: u64,
        hot_pages: &[PageId],
    ) -> Result<(), ArenaError> {
        self.commit_with(tick, hot_pages, false)
    }

    /// Plan-book name for **share** semantics (does **not** drop caller refs).
    /// CoW tick path must use [`Self::commit_transfer`] / [`Self::commit_transfer_slice`].
    pub fn commit_snapshot(&mut self, tick: u64, pages: &[PageId]) -> Result<(), ArenaError> {
        self.commit_share_slice(tick, pages)
    }

    pub fn snapshot_at(&self, tick: u64) -> Option<&SnapshotSlot> {
        let slot = (tick as usize) % HISTORY_TICKS;
        self.history_ring[slot].as_ref().filter(|s| s.tick == tick)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mvcc::page::{KinematicPoseF32, NO_HOLDER};

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
        arena.commit_transfer_slice(1, &[id]).unwrap();
        assert_eq!(arena.latest_tick(), 1);
        assert!(arena.snapshot_at(1).is_some());
        assert_eq!(arena.refcount(id), 1);
        let id2 = arena.alloc_page().expect("page2");
        arena
            .commit_transfer_slice(1 + HISTORY_TICKS as u64, &[id2])
            .unwrap();
        assert_eq!(arena.refcount(id), 0);
        assert_eq!(arena.refcount(id2), 1);
    }

    #[test]
    fn commit_transfer_rejects_dead_page() {
        let mut arena = MvccPageArena::new(16);
        let dead = PageId(0);
        assert_eq!(
            arena.commit_transfer_slice(1, &[dead]),
            Err(ArenaError::DeadPage(dead))
        );
        assert!(arena.snapshot_at(1).is_none());
    }

    #[test]
    fn commit_transfer_rejects_history_only_page() {
        let mut arena = MvccPageArena::new(16);
        let id = arena.alloc_page().expect("page");
        arena.commit_transfer_slice(1, &[id]).unwrap();
        assert!(arena.page_in_history(id));
        assert_eq!(
            arena.commit_transfer_slice(2, &[id]),
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
            arena.commit_transfer_slice(1, &[id, id]),
            Err(ArenaError::DuplicatePage(id))
        );
        assert_eq!(arena.refcount(id), 1);
        assert!(arena.snapshot_at(1).is_none());
        arena.commit_transfer_slice(1, &[id]).unwrap();
        assert_eq!(
            arena.commit_share_slice(2, &[id, id]),
            Err(ArenaError::DuplicatePage(id))
        );
        assert_eq!(arena.refcount(id), 1);
    }

    #[test]
    fn recommit_same_page_without_fork_stays_live() {
        let mut arena = MvccPageArena::new(16);
        let id = arena.alloc_page().expect("page");
        arena.commit_transfer_slice(1, &[id]).unwrap();
        assert_eq!(arena.refcount(id), 1);
        arena.commit_share_slice(2, &[id]).unwrap();
        assert_eq!(arena.refcount(id), 2);
        assert!(arena.page(id).is_some());
        let filler = arena.alloc_page().unwrap();
        arena
            .commit_transfer_slice(1 + HISTORY_TICKS as u64, &[filler])
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

    #[test]
    fn make_mut_forks_history_page_and_preserves_history() {
        let mut arena = MvccPageArena::new(16);
        let mut id = arena.alloc_page().expect("page");
        {
            let page = arena.page_mut(id).expect("mut");
            page.set_active(0, true);
            page.soa.pos_x[0] = 3.0;
            page.soa.drot_x[0] = 0.5;
            assert!(page.set_holder(0, 1, 9));
        }
        arena.commit_transfer_slice(1, &[id]).unwrap();
        assert!(
            arena.page_mut(id).is_none(),
            "history page must not be exclusive"
        );

        {
            let page = arena.make_mut(&mut id).expect("make_mut");
            assert_eq!(page.soa.pos_x[0], 3.0);
            page.soa.pos_x[0] = 99.0;
            page.soa.drot_x[0] = 1.0;
        }
        let hist = arena.snapshot_at(1).expect("hist");
        let hist_page = arena.page(hist.pages_slice()[0]).expect("hist page");
        assert_eq!(hist_page.soa.pos_x[0], 3.0);
        assert_eq!(hist_page.soa.drot_x[0].to_bits(), 0.5f32.to_bits());
        assert_eq!(hist_page.holder_at(0), 1);
        assert_eq!(arena.page(id).expect("working").soa.pos_x[0], 99.0);
        assert_ne!(id, hist.pages_slice()[0]);
        // History tip path: rc was 1 — no release of tip.
        assert_eq!(arena.refcount(hist.pages_slice()[0]), 1);
    }

    #[test]
    fn make_mut_releases_caller_ref_when_rc_gt_one() {
        let mut arena = MvccPageArena::new(16);
        let mut id = arena.alloc_page().expect("page");
        {
            let page = arena.page_mut(id).expect("mut");
            page.set_active(0, true);
            page.soa.pos_x[0] = 1.0;
        }
        arena.commit_transfer_slice(1, &[id]).unwrap();
        // Caller retains a second ref (commit_share-style ownership).
        arena.retain_page(id).unwrap();
        assert_eq!(arena.refcount(id), 2);
        let old = id;
        {
            let page = arena.make_mut(&mut id).expect("make_mut");
            page.soa.pos_x[0] = 7.0;
        }
        assert_ne!(id, old);
        assert_eq!(
            arena.refcount(old),
            1,
            "history keeps one ref; caller share released"
        );
        assert_eq!(arena.refcount(id), 1);
        assert_eq!(arena.page(old).unwrap().soa.pos_x[0], 1.0);
        assert_eq!(arena.page(id).unwrap().soa.pos_x[0], 7.0);
    }

    #[test]
    fn commit_rejects_too_many_pages() {
        let mut arena = MvccPageArena::new(MAX_PAGES_PER_SNAPSHOT + 16);
        let mut ids = Vec::new();
        for _ in 0..=MAX_PAGES_PER_SNAPSHOT {
            ids.push(arena.alloc_page().expect("page"));
        }
        assert_eq!(
            arena.commit_transfer_slice(1, &ids),
            Err(ArenaError::TooManyPages {
                len: MAX_PAGES_PER_SNAPSHOT + 1,
                max: MAX_PAGES_PER_SNAPSHOT,
            })
        );
        assert!(arena.snapshot_at(1).is_none());
    }

    #[test]
    fn holder_default_is_no_holder() {
        let page = HotPhysicsPage::zeroed_page();
        assert_eq!(page.holder_at(0), NO_HOLDER);
    }
}
