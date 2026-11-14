//! Shared control-plane rate limit for edit + legacy inventory (x00b2).
//!
//! Guarantees: one 8/10s window across `…/edit` and dual-subscribe
//! `…/inventory/spawn|despawn` so a session cannot stack three independent budgets.
//! Non-guarantees: per-op fairness; Intent rate (separate).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MAX: u32 = 8;
const WINDOW: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub struct SharedControlRate {
    start: Instant,
    count: u32,
}

impl SharedControlRate {
    pub fn new_shared() -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self {
            start: Instant::now(),
            count: 0,
        }))
    }

    pub fn allow(&mut self) -> bool {
        if self.start.elapsed() >= WINDOW {
            self.start = Instant::now();
            self.count = 0;
        }
        if self.count >= MAX {
            return false;
        }
        self.count += 1;
        true
    }

    /// Undo one successful [`Self::allow`] when a later gate (e.g. `commit_control`) fails.
    pub fn refund(&mut self) {
        self.count = self.count.saturating_sub(1);
    }
}
