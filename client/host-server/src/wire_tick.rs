//! Wire tick policy for pure-delta `frame` + `snapshot` (x0024).
//!
//! Guarantees: published frame ticks are consecutive; snapshot cache tick never
//! advertises the *next* unpublished `net_tick` (that collides with DropStale).
//! Non-guarantees: u64 wrap handling on the client.
//!
//! [`snapshot_tick_on_empty_hold`] is an identity on `last_published` — kept as a
//! **testable contract hook** so the empty-hold rule stays named and unit-tested
//! instead of only living as an inline `if let Some(tick) = last_published_tick`.

/// Tick to stamp on the snapshot cache while skipping an empty delta.
///
/// Contract hook (x0024): identity on `last_published`.
/// `None` → do not refresh a ready snapshot yet (no successful publish).
/// `Some(t)` → refresh poses but keep tick = last published (not next `net_tick`).
pub fn snapshot_tick_on_empty_hold(last_published: Option<u64>) -> Option<u64> {
    last_published
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_hold_uses_last_published_not_next_net_tick() {
        assert_eq!(snapshot_tick_on_empty_hold(None), None);
        assert_eq!(snapshot_tick_on_empty_hold(Some(5)), Some(5));
        // Next publish would use net_tick=6; snapshot must stay at 5.
        let last_published = Some(5u64);
        let next_net_tick = 6u64;
        let stamped = snapshot_tick_on_empty_hold(last_published).unwrap();
        assert_ne!(stamped, next_net_tick);
        assert_eq!(stamped, 5);
    }
}
