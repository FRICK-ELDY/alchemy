//! Published MVCC faces for app interpolation (x00a9 Phase 2 foundation).
//!
//! Guarantees: ArcSwap publish of [`PublishedFace`] prev/curr; when the single recycle
//! shell is unique, [`PublishedPair::publish_recycled`] overwrites it via [`Arc::get_mut`]
//! (no extra `Arc::new` for that publish).
//! Non-guarantees: multi-page Directory worlds (single tip page only); wgpu upload;
//! jitter buffer; owned prediction; host wiring.

use std::sync::Arc;

use arc_swap::ArcSwap;

use super::page::HotPhysicsPage;

/// One published sim face (single tip page + sim tick). Multi-page later.
#[derive(Clone)]
pub struct PublishedFace {
    pub sim_tick: u64,
    pub page: HotPhysicsPage,
}

impl PublishedFace {
    pub fn zeroed(sim_tick: u64) -> Self {
        Self {
            sim_tick,
            page: HotPhysicsPage::zeroed_page(),
        }
    }
}

/// Prev/Curr pair swapped for render interpolation (sim_tick basis).
///
/// Recycle pool is **one** unique Arc shell (enough for the prev→retire cadence).
pub struct PublishedPair {
    prev: ArcSwap<PublishedFace>,
    curr: ArcSwap<PublishedFace>,
    /// At most one unique Arc shell for the next publish.
    recycle: Option<Arc<PublishedFace>>,
}

impl PublishedPair {
    pub fn new() -> Self {
        Self {
            prev: ArcSwap::from_pointee(PublishedFace::zeroed(0)),
            curr: ArcSwap::from_pointee(PublishedFace::zeroed(0)),
            recycle: None,
        }
    }

    pub fn load_prev(&self) -> Arc<PublishedFace> {
        self.prev.load_full()
    }

    pub fn load_curr(&self) -> Arc<PublishedFace> {
        self.curr.load_full()
    }

    /// Publish a new curr; previous curr becomes prev.
    pub fn publish(&mut self, face: PublishedFace) {
        self.publish_recycled(face);
    }

    /// Same as [`Self::publish`]; name matches the recycle contract in module docs.
    pub fn publish_recycled(&mut self, face: PublishedFace) {
        let new_curr = match self.recycle.take() {
            Some(mut recycled) if Arc::strong_count(&recycled) == 1 => {
                *Arc::get_mut(&mut recycled).expect("strong_count==1") = face;
                recycled
            }
            Some(_still_shared) => Arc::new(face),
            None => Arc::new(face),
        };
        let old_curr = self.curr.swap(new_curr);
        let old_prev = self.prev.swap(old_curr);
        // Keep the Arc shell if unique — do **not** try_unwrap + Arc::new.
        self.recycle = if Arc::strong_count(&old_prev) == 1 {
            Some(old_prev)
        } else {
            None
        };
    }
}

impl Default for PublishedPair {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_advances_sim_tick_and_recycles() {
        let mut pair = PublishedPair::new();
        for tick in 1u64..=8 {
            let mut face = PublishedFace::zeroed(tick);
            face.page.set_active(0, true);
            face.page.soa.pos_x[0] = tick as f32;
            pair.publish_recycled(face);
            assert_eq!(pair.load_curr().sim_tick, tick);
            if tick > 1 {
                assert_eq!(pair.load_prev().sim_tick, tick - 1);
            }
        }
        assert_eq!(pair.load_curr().page.soa.pos_x[0], 8.0);
        assert_eq!(pair.load_prev().page.soa.pos_x[0], 7.0);
        assert!(pair.recycle.is_some());
    }
}
