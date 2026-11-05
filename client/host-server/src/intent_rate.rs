//! Per-session Intent token buckets (x0040 / P1-5; bind source split x0077 / P2-5).
//!
//! Guarantees:
//! - Steady: rate after Ed25519 success and seq check, **before** `last_seq` commit
//!   (forged sig cannot drain another sub's bucket). Leave (wire) does not consume.
//! - Bind: per-ZenohId source + per-`session_pub` **after** Ed25519, **before** JWT;
//!   both buckets must have a token or **neither** is consumed; source / per-pub rows
//!   are inserted only on successful consume (deny must not grow the maps).
//!   Leave does **not** exempt bind (no JWT bypass via `leave=1`+token). Failed
//!   bind forgets unbound per-pub entries. Successful bind must not forget the
//!   newly bound pub's bucket. One source's bind spend does not reduce another
//!   source's budget.
//! - `by_bind_source.len() ≤ MAX_BIND_SOURCES`. Inserting a fresh source at capacity
//!   first GC's rows idle since **last successful consume** (`BIND_SOURCE_IDLE`);
//!   deny-only traffic (refill / `has_token`) does **not** refresh idle. If still
//!   full after GC, deny without insert (forged ID rotate cannot unbounded-grow RAM).
//!
//! Non-guarantees: temporary ban / strike; Zenoh ACL; unlimited Ed25519 on unknown
//! `session_pub` steady floods (evaluate fails without a sub bucket); host-wide JWT
//! soft-global (removed on purpose); JWT CPU / join denial while ≤ `MAX_BIND_SOURCES`
//! sources keep **successful** consumes within `BIND_SOURCE_IDLE` (map full + all hot);
//! fairness among timestamp-less (`unknown`) peers; mTLS cert–keyed split (not on Sample).

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Steady Intent: matches client `INPUT_HZ=30` with ~2s burst headroom.
pub const STEADY_CAPACITY: f64 = 60.0;
pub const STEADY_REFILL_PER_SEC: f64 = 30.0;

/// Per-`session_pub` bind (rebind / same-key spam).
pub const BIND_CAPACITY: f64 = 5.0;
pub const BIND_REFILL_PER_SEC: f64 = 2.0;

/// Per bind-source (Zenoh timestamp ID) JWT-attempt budget before verify.
pub const BIND_SOURCE_CAPACITY: f64 = 20.0;
pub const BIND_SOURCE_REFILL_PER_SEC: f64 = 10.0;

/// Zenoh / uhlc clock ID width (`ID::MAX_SIZE` / `to_le_bytes`).
pub const BIND_SOURCE_KEY_LEN: usize = 16;

/// Hard cap on distinct bind-source rows (memory bound under forged ID rotate).
pub const MAX_BIND_SOURCES: usize = 256;

/// Idle source rows may be GC'd when inserting at [`MAX_BIND_SOURCES`].
/// Measured from last **successful** bind consume, not refill/`has_token`.
pub const BIND_SOURCE_IDLE: Duration = Duration::from_secs(60);

/// Shared key when Sample has no timestamp (uhlc rejects all-zero IDs).
pub const BIND_SOURCE_UNKNOWN: [u8; BIND_SOURCE_KEY_LEN] = [0u8; BIND_SOURCE_KEY_LEN];

pub type BindSourceKey = [u8; BIND_SOURCE_KEY_LEN];

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

    fn try_take(&mut self, now: Instant) -> bool {
        if self.has_token(now) {
            self.consume_one();
            true
        } else {
            false
        }
    }
}

/// Bind-source row: refill clock ≠ idle clock (deny must not pin the slot).
#[derive(Debug, Clone)]
struct BindSourceEntry {
    bucket: TokenBucket,
    /// Last successful `allow_bind` consume for this source (idle GC).
    last_consume: Instant,
}

/// Limiter shared via `Mutex` with Presence reclaim cleanup.
#[derive(Debug)]
pub struct IntentRateLimiter {
    by_sub: HashMap<String, TokenBucket>,
    by_session_pub: HashMap<Vec<u8>, TokenBucket>,
    by_bind_source: HashMap<BindSourceKey, BindSourceEntry>,
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
            by_bind_source: HashMap::new(),
        }
    }

    /// After Ed25519, before JWT. **Not** Leave-exempt. Source + per-pub atomic.
    /// Map entries are inserted only when both buckets actually consume
    /// (deny on a fresh key must not grow the maps). Fresh sources at
    /// [`MAX_BIND_SOURCES`] GC idle rows first, then deny if still full.
    pub fn allow_bind(&mut self, source: &BindSourceKey, session_pub: &[u8], now: Instant) -> bool {
        if !self.by_bind_source.contains_key(source) && !self.ensure_bind_source_slot(now) {
            return false;
        }

        let source_ok = match self.by_bind_source.get_mut(source) {
            Some(entry) => entry.bucket.has_token(now),
            None => true, // fresh source would start at BIND_SOURCE_CAPACITY
        };
        if !source_ok {
            return false;
        }

        let pub_ok = match self.by_session_pub.get_mut(session_pub) {
            Some(bucket) => bucket.has_token(now),
            None => true, // fresh pub would start at BIND_CAPACITY
        };
        if !pub_ok {
            return false;
        }

        match self.by_bind_source.get_mut(source) {
            Some(entry) => {
                entry.bucket.consume_one();
                entry.last_consume = now;
            }
            None => {
                // `ensure_bind_source_slot` only checked capacity (+ idle GC); insert
                // happens here after both buckets accepted the consume.
                let mut bucket =
                    TokenBucket::new(BIND_SOURCE_CAPACITY, BIND_SOURCE_REFILL_PER_SEC, now);
                bucket.consume_one();
                self.by_bind_source.insert(
                    *source,
                    BindSourceEntry {
                        bucket,
                        last_consume: now,
                    },
                );
            }
        }
        match self.by_session_pub.get_mut(session_pub) {
            Some(bucket) => bucket.consume_one(),
            None => {
                let mut bucket = TokenBucket::new(BIND_CAPACITY, BIND_REFILL_PER_SEC, now);
                bucket.consume_one();
                self.by_session_pub.insert(session_pub.to_vec(), bucket);
            }
        }
        true
    }

    /// True if a fresh source may be inserted (`len < MAX` after idle GC).
    fn ensure_bind_source_slot(&mut self, now: Instant) -> bool {
        if self.by_bind_source.len() < MAX_BIND_SOURCES {
            return true;
        }
        self.gc_idle_bind_sources(now);
        self.by_bind_source.len() < MAX_BIND_SOURCES
    }

    fn gc_idle_bind_sources(&mut self, now: Instant) {
        self.by_bind_source.retain(|_, entry| {
            now.saturating_duration_since(entry.last_consume) < BIND_SOURCE_IDLE
        });
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

    /// Drop per-pub bind bucket after failed JWT / bind evaluate (avoid map growth).
    pub fn forget_session_pub(&mut self, session_pub: &[u8]) {
        self.by_session_pub.remove(session_pub);
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
    fn bind_source_tokens(&mut self, source: &BindSourceKey, now: Instant) -> f64 {
        match self.by_bind_source.get_mut(source) {
            Some(e) => {
                e.bucket.refill(now);
                e.bucket.tokens
            }
            None => BIND_SOURCE_CAPACITY,
        }
    }

    #[cfg(test)]
    fn bind_pub_tokens(&mut self, session_pub: &[u8], now: Instant) -> f64 {
        match self.by_session_pub.get_mut(session_pub) {
            Some(b) => {
                b.refill(now);
                b.tokens
            }
            None => BIND_CAPACITY,
        }
    }

    #[cfg(test)]
    fn bind_source_last_consume(&self, source: &BindSourceKey) -> Option<Instant> {
        self.by_bind_source.get(source).map(|e| e.last_consume)
    }

    #[cfg(test)]
    fn bind_source_refill_last(&self, source: &BindSourceKey) -> Option<Instant> {
        self.by_bind_source.get(source).map(|e| e.bucket.last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_n(n: u8) -> BindSourceKey {
        let mut key = [0u8; BIND_SOURCE_KEY_LEN];
        key[0] = n;
        // Non-zero so it never collides with UNKNOWN.
        if n == 0 {
            key[1] = 1;
        }
        key
    }

    fn source_i(i: usize, tag: u8) -> BindSourceKey {
        let mut key = [0u8; BIND_SOURCE_KEY_LEN];
        key[0] = (i & 0xff) as u8;
        key[1] = ((i >> 8) & 0xff) as u8;
        key[2] = tag; // non-zero vs UNKNOWN
        key
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
    fn bind_source_limits_unique_pubs() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        let source = source_n(1);
        let mut ok = 0u32;
        for i in 0..40u8 {
            let pub_bytes = vec![i; 32];
            if lim.allow_bind(&source, &pub_bytes, t0) {
                ok += 1;
            }
        }
        assert_eq!(ok, BIND_SOURCE_CAPACITY as u32);
    }

    #[test]
    fn bind_other_source_unaffected_when_one_exhausted() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        let attacker = source_n(2);
        let victim = source_n(3);
        for i in 0..(BIND_SOURCE_CAPACITY as u8) {
            assert!(lim.allow_bind(&attacker, &[i; 32], t0));
        }
        assert!(!lim.allow_bind(&attacker, &[200u8; 32], t0));
        assert!(
            lim.allow_bind(&victim, &[1u8; 32], t0),
            "exhausted attacker source must not block another source"
        );
    }

    #[test]
    fn bind_per_pub_exhaust_does_not_drain_source() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        let source = source_n(4);
        let pub_bytes = vec![7u8; 32];
        for _ in 0..(BIND_CAPACITY as u32) {
            assert!(lim.allow_bind(&source, &pub_bytes, t0));
        }
        let source_before = lim.bind_source_tokens(&source, t0);
        assert!(!lim.allow_bind(&source, &pub_bytes, t0));
        assert!(
            (lim.bind_source_tokens(&source, t0) - source_before).abs() < f64::EPSILON,
            "per-pub deny must not consume source"
        );
    }

    #[test]
    fn fresh_source_with_exhausted_pub_does_not_grow_source_map() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        let first = source_n(5);
        let pub_bytes = vec![7u8; 32];
        for _ in 0..(BIND_CAPACITY as u32) {
            assert!(lim.allow_bind(&first, &pub_bytes, t0));
        }
        let source_len = lim.by_bind_source.len();
        assert_eq!(source_len, 1);
        let fresh = source_n(6);
        assert!(!lim.allow_bind(&fresh, &pub_bytes, t0));
        assert!(
            !lim.by_bind_source.contains_key(&fresh),
            "pub deny must not insert a fresh source row"
        );
        assert_eq!(lim.by_bind_source.len(), source_len);
    }

    #[test]
    fn source_deny_on_fresh_pub_does_not_grow_maps() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        let source = source_n(7);
        for i in 0..(BIND_SOURCE_CAPACITY as u8) {
            assert!(lim.allow_bind(&source, &[i; 32], t0));
        }
        let source_len = lim.by_bind_source.len();
        let pub_len = lim.by_session_pub.len();
        assert_eq!(source_len, 1);
        assert_eq!(pub_len, BIND_SOURCE_CAPACITY as usize);
        for i in 0..50u8 {
            let pub_bytes = vec![200u8.wrapping_add(i); 32];
            assert!(!lim.allow_bind(&source, &pub_bytes, t0));
            assert!(
                !lim.by_session_pub.contains_key(&pub_bytes),
                "source deny must not insert fresh pub"
            );
        }
        assert_eq!(lim.by_bind_source.len(), source_len);
        assert_eq!(lim.by_session_pub.len(), pub_len);
    }

    #[test]
    fn allow_bind_then_jwt_fail_forget_keeps_source_spent() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        let source = source_n(8);
        let pub_bytes = vec![9u8; 32];
        let source_before = lim.bind_source_tokens(&source, t0);
        assert!(lim.allow_bind(&source, &pub_bytes, t0));
        assert!(lim.by_session_pub.contains_key(&pub_bytes));
        lim.forget_session_pub(&pub_bytes);
        assert!(!lim.by_session_pub.contains_key(&pub_bytes));
        assert!(
            (lim.bind_source_tokens(&source, t0) - (source_before - 1.0)).abs() < f64::EPSILON,
            "JWT-fail forget drops per-pub but source stay consumed"
        );
    }

    #[test]
    fn successful_rebind_keeps_same_pub_bucket_when_not_forgotten() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        let source = source_n(9);
        let pub_bytes = vec![3u8; 32];
        assert!(lim.allow_bind(&source, &pub_bytes, t0));
        let after_one = lim.bind_pub_tokens(&pub_bytes, t0);
        // Simulate commit_bind GC that excludes the current pub.
        lim.forget_session_pubs(&[vec![99u8; 32]]);
        assert!(lim.by_session_pub.contains_key(&pub_bytes));
        assert!((lim.bind_pub_tokens(&pub_bytes, t0) - after_one).abs() < f64::EPSILON);
    }

    #[test]
    fn unknown_source_key_is_shared() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        for i in 0..(BIND_SOURCE_CAPACITY as u8) {
            assert!(lim.allow_bind(&BIND_SOURCE_UNKNOWN, &[i; 32], t0));
        }
        assert!(!lim.allow_bind(&BIND_SOURCE_UNKNOWN, &[255u8; 32], t0));
    }

    #[test]
    fn bind_source_map_capped_without_idle_gc() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        for i in 0..MAX_BIND_SOURCES {
            let key = source_i(i, 1);
            assert!(
                lim.allow_bind(&key, &[(i % 250) as u8 + 1; 32], t0),
                "fill slot {i}"
            );
        }
        assert_eq!(lim.by_bind_source.len(), MAX_BIND_SOURCES);
        let overflow = source_n(200);
        assert!(!lim.allow_bind(&overflow, &[42u8; 32], t0));
        assert!(!lim.by_bind_source.contains_key(&overflow));
        assert_eq!(lim.by_bind_source.len(), MAX_BIND_SOURCES);
    }

    #[test]
    fn idle_bind_sources_gc_before_insert_at_cap() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        for i in 0..MAX_BIND_SOURCES {
            let key = source_i(i, 2);
            assert!(lim.allow_bind(&key, &[(i % 250) as u8 + 1; 32], t0));
        }
        assert_eq!(lim.by_bind_source.len(), MAX_BIND_SOURCES);
        let t1 = t0 + BIND_SOURCE_IDLE + Duration::from_millis(1);
        let fresh = source_n(201);
        assert!(
            lim.allow_bind(&fresh, &[99u8; 32], t1),
            "idle GC must free a slot for a new source"
        );
        assert!(lim.by_bind_source.contains_key(&fresh));
        assert!(lim.by_bind_source.len() <= MAX_BIND_SOURCES);
    }

    #[test]
    fn deny_path_does_not_refresh_bind_source_idle() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        // Unique pub per source; exhaust per-pub while source still has tokens.
        for i in 0..MAX_BIND_SOURCES {
            let key = source_i(i, 3);
            let mut pub_bytes = [0u8; 32];
            pub_bytes[0] = key[0];
            pub_bytes[1] = key[1];
            pub_bytes[2] = 3;
            for _ in 0..(BIND_CAPACITY as u32) {
                assert!(lim.allow_bind(&key, &pub_bytes, t0));
            }
            assert_eq!(lim.bind_source_last_consume(&key), Some(t0));
        }
        assert_eq!(lim.by_bind_source.len(), MAX_BIND_SOURCES);

        // Short dt: pub still empty (< 1 refilled token) but source.has_token refills.
        let t_mid = t0 + Duration::from_millis(10);
        for i in 0..MAX_BIND_SOURCES {
            let key = source_i(i, 3);
            let mut pub_bytes = [0u8; 32];
            pub_bytes[0] = key[0];
            pub_bytes[1] = key[1];
            pub_bytes[2] = 3;
            assert!(!lim.allow_bind(&key, &pub_bytes, t_mid));
            assert_eq!(
                lim.bind_source_last_consume(&key),
                Some(t0),
                "pub deny must not move last_consume"
            );
            let refill_last = lim.bind_source_refill_last(&key).expect("source row");
            assert!(
                refill_last > t0,
                "deny still refills the token clock (separate from idle)"
            );
        }

        let t_idle = t0 + BIND_SOURCE_IDLE + Duration::from_millis(1);
        let fresh = source_n(202);
        assert!(
            lim.allow_bind(&fresh, &[77u8; 32], t_idle),
            "deny-only traffic must not prevent idle GC after BIND_SOURCE_IDLE from last consume"
        );
        assert!(lim.by_bind_source.contains_key(&fresh));
    }
}
