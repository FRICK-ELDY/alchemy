//! Total-order op timestamp + lamport drift gate (x00de).

/// Max lamport advance beyond the store's observed max accepted in one apply.
///
/// Contract (roadmap §5.3): a far-future client clock must not permanently win
/// Property-level LWW. Host still owns realtime order via `(host_epoch, room_seq)`;
/// this bound only gates the **CRDT merge key** embedded in the op.
///
/// Guarantees: [`SceneStore`](super::SceneStore) **rejects** (No-op) when
/// `lamport > observed_max + MAX_LAMPORT_DRIFT` — silent clamp-and-apply is
/// avoided because it would ratchet the max by `DRIFT` on every forged tick.
/// Non-guarantees: NTP sync, per-actor clocks, JWT `sub` → `actor_id` (host gate).
pub const MAX_LAMPORT_DRIFT: u64 = 1_048_576; // 2^20

/// Total order across peers for LWW / tree parent merge.
///
/// Ordering: `(lamport, actor_id)` lexicographic — higher lamport wins; on tie,
/// higher `actor_id` wins. Host must force `actor_id` from Room Token `sub`
/// (not implemented here).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OpTimestamp {
    pub lamport: u64,
    pub actor_id: u64,
}

impl OpTimestamp {
    #[inline]
    pub const fn new(lamport: u64, actor_id: u64) -> Self {
        Self { lamport, actor_id }
    }

    /// `true` when `lamport` is within [`MAX_LAMPORT_DRIFT`] of `observed_max`.
    #[inline]
    pub fn within_drift(self, observed_max: u64) -> bool {
        self.lamport <= observed_max.saturating_add(MAX_LAMPORT_DRIFT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ord_is_lamport_then_actor() {
        let a = OpTimestamp::new(1, 99);
        let b = OpTimestamp::new(2, 0);
        let c = OpTimestamp::new(2, 1);
        assert!(a < b);
        assert!(b < c);
        assert!(a < c);
    }

    #[test]
    fn within_drift_bounds_far_future() {
        assert!(OpTimestamp::new(10 + MAX_LAMPORT_DRIFT, 1).within_drift(10));
        assert!(!OpTimestamp::new(10 + MAX_LAMPORT_DRIFT + 1, 1).within_drift(10));
        assert!(OpTimestamp::new(11, 1).within_drift(10));
        assert!(!OpTimestamp::new(u64::MAX, 7).within_drift(10));
    }
}
