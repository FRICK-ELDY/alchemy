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
//!
//! Non-guarantees: temporary ban / strike; Zenoh ACL; unlimited Ed25519 on unknown
//! `session_pub` steady floods (evaluate fails without a sub bucket); JWT CPU under
//! forged rotating timestamp IDs; fairness among timestamp-less (`unknown`) peers;
//! mTLS certificate–keyed split (not visible on Sample).

use std::collections::HashMap;
use std::time::Instant;

/// Steady Intent: matches client `INPUT_HZ=30` with ~2s burst headroom.
pub const STEADY_CAPACITY: f64 = 60.0;
pub const STEADY_REFILL_PER_SEC: f64 = 30.0;

/// Per-`session_pub` bind (rebind / same-key spam).
pub const BIND_CAPACITY: f64 = 5.0;
pub const BIND_REFILL_PER_SEC: f64 = 2.0;

/// Per bind-source (Zenoh timestamp ID) before JWT (unique-pub JWT CPU bound).
pub const BIND_SOURCE_CAPACITY: f64 = 20.0;
pub const BIND_SOURCE_REFILL_PER_SEC: f64 = 10.0;

/// Shared key when Sample has no timestamp (legacy / unstamped puts).
pub const BIND_SOURCE_UNKNOWN: &[u8] = b"\0alchemy-bind-unknown";

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

/// Limiter shared via `Mutex` with Presence reclaim cleanup.
#[derive(Debug)]
pub struct IntentRateLimiter {
    by_sub: HashMap<String, TokenBucket>,
    by_session_pub: HashMap<Vec<u8>, TokenBucket>,
    by_bind_source: HashMap<Vec<u8>, TokenBucket>,
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
    /// (deny on a fresh key must not grow the maps).
    pub fn allow_bind(&mut self, source: &[u8], session_pub: &[u8], now: Instant) -> bool {
        let source_ok = match self.by_bind_source.get_mut(source) {
            Some(bucket) => bucket.has_token(now),
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
            Some(bucket) => bucket.consume_one(),
            None => {
                let mut bucket =
                    TokenBucket::new(BIND_SOURCE_CAPACITY, BIND_SOURCE_REFILL_PER_SEC, now);
                bucket.consume_one();
                self.by_bind_source.insert(source.to_vec(), bucket);
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
    fn bind_source_tokens(&mut self, source: &[u8], now: Instant) -> f64 {
        match self.by_bind_source.get_mut(source) {
            Some(b) => {
                b.refill(now);
                b.tokens
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

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
        let source = b"peer-a";
        let mut ok = 0u32;
        for i in 0..40u8 {
            let pub_bytes = vec![i; 32];
            if lim.allow_bind(source, &pub_bytes, t0) {
                ok += 1;
            }
        }
        assert_eq!(ok, BIND_SOURCE_CAPACITY as u32);
    }

    #[test]
    fn bind_other_source_unaffected_when_one_exhausted() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        let attacker = b"attacker";
        let victim = b"victim";
        for i in 0..(BIND_SOURCE_CAPACITY as u8) {
            assert!(lim.allow_bind(attacker, &[i; 32], t0));
        }
        assert!(!lim.allow_bind(attacker, &[200u8; 32], t0));
        assert!(
            lim.allow_bind(victim, &[1u8; 32], t0),
            "exhausted attacker source must not block another source"
        );
    }

    #[test]
    fn bind_per_pub_exhaust_does_not_drain_source() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        let source = b"peer-b";
        let pub_bytes = vec![7u8; 32];
        for _ in 0..(BIND_CAPACITY as u32) {
            assert!(lim.allow_bind(source, &pub_bytes, t0));
        }
        let source_before = lim.bind_source_tokens(source, t0);
        assert!(!lim.allow_bind(source, &pub_bytes, t0));
        assert!(
            (lim.bind_source_tokens(source, t0) - source_before).abs() < f64::EPSILON,
            "per-pub deny must not consume source"
        );
    }

    #[test]
    fn source_deny_on_fresh_pub_does_not_grow_maps() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        let source = b"peer-c";
        for i in 0..(BIND_SOURCE_CAPACITY as u8) {
            assert!(lim.allow_bind(source, &[i; 32], t0));
        }
        let source_len = lim.by_bind_source.len();
        let pub_len = lim.by_session_pub.len();
        assert_eq!(source_len, 1);
        assert_eq!(pub_len, BIND_SOURCE_CAPACITY as usize);
        for i in 0..50u8 {
            let pub_bytes = vec![200u8.wrapping_add(i); 32];
            assert!(!lim.allow_bind(source, &pub_bytes, t0));
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
        let source = b"peer-d";
        let pub_bytes = vec![9u8; 32];
        let source_before = lim.bind_source_tokens(source, t0);
        assert!(lim.allow_bind(source, &pub_bytes, t0));
        assert!(lim.by_session_pub.contains_key(&pub_bytes));
        lim.forget_session_pub(&pub_bytes);
        assert!(!lim.by_session_pub.contains_key(&pub_bytes));
        assert!(
            (lim.bind_source_tokens(source, t0) - (source_before - 1.0)).abs() < f64::EPSILON,
            "JWT-fail forget drops per-pub but source stay consumed"
        );
    }

    #[test]
    fn successful_rebind_keeps_same_pub_bucket_when_not_forgotten() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        let source = b"peer-e";
        let pub_bytes = vec![3u8; 32];
        assert!(lim.allow_bind(source, &pub_bytes, t0));
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
            assert!(lim.allow_bind(BIND_SOURCE_UNKNOWN, &[i; 32], t0));
        }
        assert!(!lim.allow_bind(BIND_SOURCE_UNKNOWN, &[255u8; 32], t0));
    }
}
