//! Per-session Intent token buckets (x0040 / P1-5; bind source split x0077;
//! session_pub-keyed global + refund x009c / P2-14).
//!
//! Guarantees:
//! - Steady: rate after Ed25519 success and seq check, **before** `last_seq` commit
//!   (forged sig cannot drain another sub's bucket). Leave (wire) does not consume.
//! - Bind: host soft-global + per-`session_pub` **after** Ed25519, **before** JWT
//!   (Leave **not** exempt). Both buckets must have a token or **neither** is consumed;
//!   per-pub rows insert only on successful consume. Failed JWT forgets per-pub
//!   **only if SessionTable has no binding at cleanup** (nested `sessions→rates`
//!   critical section: `is_bound` + `after_bind_jwt_fail`); already-bound /
//!   concurrent-commit keep the spent per-pub bucket. Successful JWT **refunds**
//!   one global token (legitimate joins do not permanently starve others; failed
//!   JWT keeps the global spend → JWT CPU ceiling).
//! - Key is Ed25519-verified `session_pub` (not self-declared ZenohId).
//! - `by_session_pub.len() ≤ MAX_BIND_PUBS`. Fresh pub at capacity GC's rows idle
//!   since last **successful** consume (`BIND_PUB_IDLE`); deny-only traffic does
//!   **not** refresh idle. If still full after GC, deny without insert.
//!
//! Non-guarantees: temporary ban / strike; Zenoh ACL; unlimited Ed25519 on unknown
//! `session_pub` steady floods (evaluate fails without a sub bucket); join denial
//! while global is drained by failed-JWT flood; fairness among rotating keys that
//! all fail JWT; PoW on first `session_pub` appearance; mTLS cert–keyed split.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Steady Intent: matches client `INPUT_HZ=30` with ~2s burst headroom.
pub const STEADY_CAPACITY: f64 = 60.0;
pub const STEADY_REFILL_PER_SEC: f64 = 30.0;

/// Per-`session_pub` bind (rebind / same-key spam).
pub const BIND_CAPACITY: f64 = 5.0;
pub const BIND_REFILL_PER_SEC: f64 = 2.0;

/// Host-wide JWT-attempt budget before verify (x0040 / x009c).
pub const BIND_GLOBAL_CAPACITY: f64 = 20.0;
pub const BIND_GLOBAL_REFILL_PER_SEC: f64 = 10.0;

/// Hard cap on distinct bind-pub rows (memory bound under key rotate).
pub const MAX_BIND_PUBS: usize = 256;

/// Idle pub rows may be GC'd when inserting at [`MAX_BIND_PUBS`].
/// Measured from last **successful** bind consume, not refill/`has_token`.
pub const BIND_PUB_IDLE: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
struct TokenBucket {
    tokens: f64,
    capacity: f64,
    refill_per_sec: f64,
    last: Instant,
}

impl TokenBucket {
    fn new(capacity: f64, refill_per_sec: f64, now: Instant) -> Self {
        Self {
            tokens: capacity,
            capacity,
            refill_per_sec,
            last: now,
        }
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        if elapsed > 0.0 {
            self.tokens = (self.tokens + elapsed * self.refill_per_sec).min(self.capacity);
            self.last = now;
        }
    }

    fn has_token(&mut self, now: Instant) -> bool {
        self.refill(now);
        self.tokens >= 1.0
    }

    /// Caller must have checked [`has_token`] at the same `now` (or accept underflow).
    fn consume_one(&mut self) {
        self.tokens -= 1.0;
    }

    fn refund_one(&mut self, now: Instant) {
        self.refill(now);
        self.tokens = (self.tokens + 1.0).min(self.capacity);
    }

    fn try_take(&mut self, now: Instant) -> bool {
        if self.has_token(now) {
            self.consume_one();
            true
        } else {
            false
        }
    }
}

/// Per-pub row: refill clock ≠ idle clock (deny must not pin the slot).
#[derive(Debug, Clone)]
struct BindPubEntry {
    bucket: TokenBucket,
    /// Last successful `allow_bind` consume for this pub (idle GC).
    last_consume: Instant,
}

/// Limiter shared via `Mutex` with Presence reclaim cleanup.
#[derive(Debug)]
pub struct IntentRateLimiter {
    by_sub: HashMap<String, TokenBucket>,
    by_session_pub: HashMap<Vec<u8>, BindPubEntry>,
    bind_global: Option<TokenBucket>,
}

impl Default for IntentRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl IntentRateLimiter {
    pub fn new() -> Self {
        Self {
            by_sub: HashMap::new(),
            by_session_pub: HashMap::new(),
            bind_global: None,
        }
    }

    /// After Ed25519, before JWT. **Not** Leave-exempt. Global + per-pub atomic.
    /// Map entries are inserted only when both buckets actually consume
    /// (deny on a fresh key must not grow the map). Fresh pubs at
    /// [`MAX_BIND_PUBS`] GC idle rows first, then deny if still full.
    pub fn allow_bind(&mut self, session_pub: &[u8], now: Instant) -> bool {
        if !self.by_session_pub.contains_key(session_pub) && !self.ensure_bind_pub_slot(now) {
            return false;
        }

        let global_ok = match self.bind_global.as_mut() {
            Some(bucket) => bucket.has_token(now),
            None => true, // fresh global would start at BIND_GLOBAL_CAPACITY
        };
        if !global_ok {
            return false;
        }

        let pub_ok = match self.by_session_pub.get_mut(session_pub) {
            Some(entry) => entry.bucket.has_token(now),
            None => true, // fresh pub would start at BIND_CAPACITY
        };
        if !pub_ok {
            return false;
        }

        match self.bind_global.as_mut() {
            Some(bucket) => bucket.consume_one(),
            None => {
                let mut bucket =
                    TokenBucket::new(BIND_GLOBAL_CAPACITY, BIND_GLOBAL_REFILL_PER_SEC, now);
                bucket.consume_one();
                self.bind_global = Some(bucket);
            }
        }
        match self.by_session_pub.get_mut(session_pub) {
            Some(entry) => {
                entry.bucket.consume_one();
                entry.last_consume = now;
            }
            None => {
                let mut bucket = TokenBucket::new(BIND_CAPACITY, BIND_REFILL_PER_SEC, now);
                bucket.consume_one();
                self.by_session_pub.insert(
                    session_pub.to_vec(),
                    BindPubEntry {
                        bucket,
                        last_consume: now,
                    },
                );
            }
        }
        true
    }

    /// After successful JWT bind: return one soft-global token (capped).
    /// Failed JWT must **not** call this — attacker keeps the spend.
    /// No-op if `allow_bind` never created the global bucket (mis-ordered call).
    pub fn refund_bind_global(&mut self, now: Instant) {
        if let Some(bucket) = self.bind_global.as_mut() {
            bucket.refund_one(now);
        }
    }

    /// True if a fresh pub may be inserted (`len < MAX` after idle GC).
    fn ensure_bind_pub_slot(&mut self, now: Instant) -> bool {
        if self.by_session_pub.len() < MAX_BIND_PUBS {
            return true;
        }
        self.gc_idle_bind_pubs(now);
        self.by_session_pub.len() < MAX_BIND_PUBS
    }

    fn gc_idle_bind_pubs(&mut self, now: Instant) {
        self.by_session_pub
            .retain(|_, entry| now.saturating_duration_since(entry.last_consume) < BIND_PUB_IDLE);
    }

    /// After Ed25519 + steady evaluate (seq ok), before `last_seq` commit. Leave exempt.
    pub fn allow_steady(&mut self, wire_leave: bool, sub: &str, now: Instant) -> bool {
        if wire_leave {
            return true;
        }
        self.take_sub(sub, now)
    }

    fn take_sub(&mut self, sub: &str, now: Instant) -> bool {
        if let Some(bucket) = self.by_sub.get_mut(sub) {
            return bucket.try_take(now);
        }
        let mut bucket = TokenBucket::new(STEADY_CAPACITY, STEADY_REFILL_PER_SEC, now);
        let ok = bucket.try_take(now);
        self.by_sub.insert(sub.to_string(), bucket);
        ok
    }

    pub fn forget_sub(&mut self, sub: &str) {
        self.by_sub.remove(sub);
    }

    pub fn forget_session_pubs(&mut self, pubs: &[Vec<u8>]) {
        for p in pubs {
            self.by_session_pub.remove(p);
        }
    }

    /// Drop per-pub bind bucket. Prefer [`after_bind_jwt_fail`] so bound pubs
    /// keep their spent budget.
    pub fn forget_session_pub(&mut self, session_pub: &[u8]) {
        self.by_session_pub.remove(session_pub);
    }

    /// x009c policy: forget unbound map rows after JWT fail; keep spent per-pub
    /// when `session_pub` is already Session-bound (rebind fail / concurrent commit).
    #[inline]
    pub fn should_forget_after_bind_fail(is_bound: bool) -> bool {
        !is_bound
    }

    /// After failed JWT: forget per-pub only when unbound (`is_bound == false`).
    pub fn after_bind_jwt_fail(&mut self, session_pub: &[u8], is_bound: bool) {
        if Self::should_forget_after_bind_fail(is_bound) {
            self.forget_session_pub(session_pub);
        }
    }

    /// Test helper: remaining steady tokens (approx after refill at `now`).
    #[cfg(test)]
    pub fn steady_tokens(&mut self, sub: &str, now: Instant) -> f64 {
        match self.by_sub.get_mut(sub) {
            Some(b) => {
                b.refill(now);
                b.tokens
            }
            None => STEADY_CAPACITY,
        }
    }

    #[cfg(test)]
    fn bind_global_tokens(&mut self, now: Instant) -> f64 {
        match self.bind_global.as_mut() {
            Some(b) => {
                b.refill(now);
                b.tokens
            }
            None => BIND_GLOBAL_CAPACITY,
        }
    }

    #[cfg(test)]
    pub(crate) fn test_has_bind_pub(&self, session_pub: &[u8]) -> bool {
        self.by_session_pub.contains_key(session_pub)
    }

    #[cfg(test)]
    pub(crate) fn test_bind_pub_tokens(&mut self, session_pub: &[u8], now: Instant) -> f64 {
        self.bind_pub_tokens(session_pub, now)
    }

    #[cfg(test)]
    fn bind_pub_tokens(&mut self, session_pub: &[u8], now: Instant) -> f64 {
        match self.by_session_pub.get_mut(session_pub) {
            Some(e) => {
                e.bucket.refill(now);
                e.bucket.tokens
            }
            None => BIND_CAPACITY,
        }
    }

    #[cfg(test)]
    fn bind_pub_last_consume(&self, session_pub: &[u8]) -> Option<Instant> {
        self.by_session_pub.get(session_pub).map(|e| e.last_consume)
    }

    #[cfg(test)]
    fn bind_pub_refill_last(&self, session_pub: &[u8]) -> Option<Instant> {
        self.by_session_pub.get(session_pub).map(|e| e.bucket.last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pub_n(n: u8) -> Vec<u8> {
        vec![n; 32]
    }

    fn pub_i(i: usize, tag: u8) -> Vec<u8> {
        let mut bytes = [0u8; 32];
        bytes[0] = (i & 0xff) as u8;
        bytes[1] = ((i >> 8) & 0xff) as u8;
        bytes[2] = tag;
        bytes.to_vec()
    }

    #[test]
    fn leave_always_allowed_without_consuming() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        for _ in 0..(STEADY_CAPACITY as u32) {
            assert!(lim.allow_steady(false, "u1", t0));
        }
        assert!(!lim.allow_steady(false, "u1", t0));
        assert!(lim.allow_steady(true, "u1", t0));
    }

    #[test]
    fn steady_burst_then_refill() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        for _ in 0..(STEADY_CAPACITY as u32) {
            assert!(lim.allow_steady(false, "u2", t0));
        }
        assert!(!lim.allow_steady(false, "u2", t0));
        let t1 = t0 + Duration::from_secs(1);
        let mut ok = 0u32;
        for _ in 0..40 {
            if lim.allow_steady(false, "u2", t1) {
                ok += 1;
            }
        }
        assert!(
            (28..=32).contains(&ok),
            "expected ~30 after 1s refill, got {ok}"
        );
    }

    #[test]
    fn bind_global_limits_unique_pubs() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        let mut ok = 0u32;
        for i in 0..40u8 {
            if lim.allow_bind(&pub_n(i), t0) {
                ok += 1;
            }
        }
        assert_eq!(ok, BIND_GLOBAL_CAPACITY as u32);
    }

    #[test]
    fn successful_bind_refund_frees_global_for_other_pub() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        for i in 0..(BIND_GLOBAL_CAPACITY as u8) {
            assert!(lim.allow_bind(&pub_n(i), t0));
            lim.refund_bind_global(t0);
        }
        // Net-zero global after refunds: a fresh pub still gets through.
        assert!(
            lim.allow_bind(&pub_n(200), t0),
            "refunded successful binds must not permanently drain global"
        );
    }

    #[test]
    fn jwt_fail_keeps_global_spent() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        let before = lim.bind_global_tokens(t0);
        assert!(lim.allow_bind(&pub_n(1), t0));
        lim.forget_session_pub(&pub_n(1));
        assert!(
            (lim.bind_global_tokens(t0) - (before - 1.0)).abs() < f64::EPSILON,
            "JWT-fail forget drops per-pub but global stay consumed"
        );
    }

    #[test]
    fn bind_per_pub_exhaust_does_not_drain_global() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        let pub_bytes = pub_n(7);
        for _ in 0..(BIND_CAPACITY as u32) {
            assert!(lim.allow_bind(&pub_bytes, t0));
            lim.refund_bind_global(t0); // simulate success so global stays full
        }
        let global_before = lim.bind_global_tokens(t0);
        assert!(!lim.allow_bind(&pub_bytes, t0));
        assert!(
            (lim.bind_global_tokens(t0) - global_before).abs() < f64::EPSILON,
            "per-pub deny must not consume global"
        );
    }

    #[test]
    fn fresh_pub_with_exhausted_global_does_not_grow_pub_map() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        for i in 0..(BIND_GLOBAL_CAPACITY as u8) {
            assert!(lim.allow_bind(&pub_n(i), t0));
            // no refund → global empty
        }
        let pub_len = lim.by_session_pub.len();
        assert_eq!(pub_len, BIND_GLOBAL_CAPACITY as usize);
        let fresh = pub_n(200);
        assert!(!lim.allow_bind(&fresh, t0));
        assert!(
            !lim.by_session_pub.contains_key(&fresh),
            "global deny must not insert a fresh pub row"
        );
        assert_eq!(lim.by_session_pub.len(), pub_len);
    }

    #[test]
    fn pub_deny_on_fresh_does_not_grow_maps_when_global_ok() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        let pub_bytes = pub_n(7);
        for _ in 0..(BIND_CAPACITY as u32) {
            assert!(lim.allow_bind(&pub_bytes, t0));
            lim.refund_bind_global(t0);
        }
        let pub_len = lim.by_session_pub.len();
        let global_before = lim.bind_global_tokens(t0);
        assert!(!lim.allow_bind(&pub_bytes, t0));
        assert_eq!(lim.by_session_pub.len(), pub_len);
        assert!(
            (lim.bind_global_tokens(t0) - global_before).abs() < f64::EPSILON,
            "pub deny must not consume global"
        );
    }

    #[test]
    fn successful_rebind_keeps_same_pub_bucket_when_not_forgotten() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        let pub_bytes = pub_n(3);
        assert!(lim.allow_bind(&pub_bytes, t0));
        lim.refund_bind_global(t0);
        let after_one = lim.bind_pub_tokens(&pub_bytes, t0);
        lim.forget_session_pubs(&[pub_n(99)]);
        assert!(lim.by_session_pub.contains_key(&pub_bytes));
        assert!((lim.bind_pub_tokens(&pub_bytes, t0) - after_one).abs() < f64::EPSILON);
    }

    #[test]
    fn bind_pub_map_capped_without_idle_gc() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        for i in 0..MAX_BIND_PUBS {
            let key = pub_i(i, 1);
            assert!(lim.allow_bind(&key, t0), "fill slot {i}");
            lim.refund_bind_global(t0); // keep global from blocking the fill
        }
        assert_eq!(lim.by_session_pub.len(), MAX_BIND_PUBS);
        let overflow = pub_n(42);
        assert!(!lim.allow_bind(&overflow, t0));
        assert!(!lim.by_session_pub.contains_key(&overflow));
        assert_eq!(lim.by_session_pub.len(), MAX_BIND_PUBS);
    }

    #[test]
    fn idle_bind_pubs_gc_before_insert_at_cap() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        for i in 0..MAX_BIND_PUBS {
            let key = pub_i(i, 2);
            assert!(lim.allow_bind(&key, t0));
            lim.refund_bind_global(t0);
        }
        assert_eq!(lim.by_session_pub.len(), MAX_BIND_PUBS);
        let t1 = t0 + BIND_PUB_IDLE + Duration::from_millis(1);
        let fresh = pub_n(99);
        assert!(
            lim.allow_bind(&fresh, t1),
            "idle GC must free a slot for a new pub"
        );
        assert!(lim.by_session_pub.contains_key(&fresh));
        assert!(lim.by_session_pub.len() <= MAX_BIND_PUBS);
    }

    #[test]
    fn deny_path_does_not_refresh_bind_pub_idle() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        // Fill map with unique pubs; exhaust per-pub while refunding global.
        let mut keys = Vec::with_capacity(MAX_BIND_PUBS);
        for i in 0..MAX_BIND_PUBS {
            let key = pub_i(i, 3);
            for _ in 0..(BIND_CAPACITY as u32) {
                assert!(lim.allow_bind(&key, t0));
                lim.refund_bind_global(t0);
            }
            assert_eq!(lim.bind_pub_last_consume(&key), Some(t0));
            keys.push(key);
        }
        assert_eq!(lim.by_session_pub.len(), MAX_BIND_PUBS);

        // Short dt: pub still empty (< 1 refilled token) but global.has_token refills.
        let t_mid = t0 + Duration::from_millis(10);
        for key in &keys {
            assert!(!lim.allow_bind(key, t_mid));
            assert_eq!(
                lim.bind_pub_last_consume(key),
                Some(t0),
                "pub deny must not move last_consume"
            );
            let refill_last = lim.bind_pub_refill_last(key).expect("pub row");
            assert!(
                refill_last > t0,
                "deny still refills the token clock (separate from idle)"
            );
        }

        let t_idle = t0 + BIND_PUB_IDLE + Duration::from_millis(1);
        let fresh = pub_n(77);
        assert!(
            lim.allow_bind(&fresh, t_idle),
            "deny-only traffic must not prevent idle GC after BIND_PUB_IDLE from last consume"
        );
        assert!(lim.by_session_pub.contains_key(&fresh));
    }

    #[test]
    fn refund_bind_global_noop_when_never_consumed() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        lim.refund_bind_global(t0);
        assert!(
            lim.bind_global.is_none(),
            "mis-ordered refund must not mint a full global bucket"
        );
        assert!((lim.bind_global_tokens(t0) - BIND_GLOBAL_CAPACITY).abs() < f64::EPSILON);
    }

    #[test]
    fn allow_bind_takes_only_session_pub_no_zenoh_source() {
        // P2-14: bind budget is keyed solely by session_pub; there is no Sample-ID
        // parameter that a publisher could rotate to reset the bucket.
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        let pub_bytes = pub_n(9);
        for _ in 0..(BIND_CAPACITY as u32) {
            assert!(lim.allow_bind(&pub_bytes, t0));
            lim.refund_bind_global(t0);
        }
        assert!(
            !lim.allow_bind(&pub_bytes, t0),
            "same session_pub must share one per-pub budget across attempts"
        );
    }

    #[test]
    fn should_forget_after_bind_fail_only_when_unbound() {
        assert!(IntentRateLimiter::should_forget_after_bind_fail(false));
        assert!(!IntentRateLimiter::should_forget_after_bind_fail(true));
    }

    #[test]
    fn after_bind_jwt_fail_forgets_unbound_keeps_bound() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        let unbound = pub_n(11);
        let bound = pub_n(12);
        assert!(lim.allow_bind(&unbound, t0));
        assert!(lim.allow_bind(&bound, t0));
        let bound_tokens = lim.bind_pub_tokens(&bound, t0);

        lim.after_bind_jwt_fail(&bound, true);
        assert!(
            lim.by_session_pub.contains_key(&bound),
            "already-bound JWT fail must keep per-pub row"
        );
        assert!((lim.bind_pub_tokens(&bound, t0) - bound_tokens).abs() < f64::EPSILON);

        lim.after_bind_jwt_fail(&unbound, false);
        assert!(
            !lim.by_session_pub.contains_key(&unbound),
            "unbound JWT fail must forget per-pub row"
        );
        // Global spend from both allow_bind calls remains (no refund on fail path).
        assert!((lim.bind_global_tokens(t0) - (BIND_GLOBAL_CAPACITY - 2.0)).abs() < f64::EPSILON);
    }
}
