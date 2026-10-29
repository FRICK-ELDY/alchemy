//! Per-session Intent token buckets (x0040 / P1-5).
//!
//! Guarantees:
//! - Steady: rate after Ed25519 success and seq check, **before** `last_seq` commit
//!   (forged sig cannot drain another sub's bucket). Leave (wire) does not consume.
//! - Bind: host-global + per-`session_pub` **after** Ed25519, **before** JWT;
//!   both buckets must have a token or **neither** is consumed; per-pub row is
//!   inserted only on successful consume (global deny must not grow the map).
//!   Leave does **not** exempt bind (no JWT bypass via `leave=1`+token). Failed
//!   bind forgets unbound per-pub entries. Successful bind must not forget the
//!   newly bound pub's bucket.
//!
//! Non-guarantees: temporary ban / strike; Zenoh ACL; unlimited Ed25519 on unknown
//! `session_pub` steady floods (evaluate fails without a sub bucket).

use std::collections::HashMap;
use std::time::Instant;

/// Steady Intent: matches client `INPUT_HZ=30` with ~2s burst headroom.
pub const STEADY_CAPACITY: f64 = 60.0;
pub const STEADY_REFILL_PER_SEC: f64 = 30.0;

/// Per-`session_pub` bind (rebind / same-key spam).
pub const BIND_CAPACITY: f64 = 5.0;
pub const BIND_REFILL_PER_SEC: f64 = 2.0;

/// Host-wide bind attempts before JWT (unique-pub JWT CPU bound).
pub const BIND_GLOBAL_CAPACITY: f64 = 20.0;
pub const BIND_GLOBAL_REFILL_PER_SEC: f64 = 10.0;

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
    /// Per-pub map entry is inserted only when both buckets actually consume
    /// (global deny on a fresh pub must not grow `by_session_pub`).
    pub fn allow_bind(&mut self, session_pub: &[u8], now: Instant) -> bool {
        self.ensure_bind_global(now);
        let global_ok = self
            .bind_global
            .as_mut()
            .expect("bind_global ensured")
            .has_token(now);
        if !global_ok {
            return false;
        }

        let pub_ok = match self.by_session_pub.get_mut(session_pub) {
            Some(bucket) => bucket.has_token(now),
            None => true, // fresh pub would start at BIND_CAPACITY
        };
        if !pub_ok {
            return false;
        }

        self.bind_global
            .as_mut()
            .expect("bind_global ensured")
            .consume_one();
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

    fn ensure_bind_global(&mut self, now: Instant) {
        if self.bind_global.is_none() {
            self.bind_global = Some(TokenBucket::new(
                BIND_GLOBAL_CAPACITY,
                BIND_GLOBAL_REFILL_PER_SEC,
                now,
            ));
        }
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
    fn bind_global_limits_unique_pubs() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        let mut ok = 0u32;
        for i in 0..40u8 {
            let pub_bytes = vec![i; 32];
            if lim.allow_bind(&pub_bytes, t0) {
                ok += 1;
            }
        }
        assert_eq!(ok, BIND_GLOBAL_CAPACITY as u32);
    }

    #[test]
    fn bind_per_pub_exhaust_does_not_drain_global() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        let pub_bytes = vec![7u8; 32];
        for _ in 0..(BIND_CAPACITY as u32) {
            assert!(lim.allow_bind(&pub_bytes, t0));
        }
        let global_before = lim.bind_global_tokens(t0);
        assert!(!lim.allow_bind(&pub_bytes, t0));
        assert!(
            (lim.bind_global_tokens(t0) - global_before).abs() < f64::EPSILON,
            "per-pub deny must not consume global"
        );
    }

    #[test]
    fn global_deny_on_fresh_pub_does_not_grow_map() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        for i in 0..(BIND_GLOBAL_CAPACITY as u8) {
            assert!(lim.allow_bind(&[i; 32], t0));
        }
        let map_len = lim.by_session_pub.len();
        assert_eq!(map_len, BIND_GLOBAL_CAPACITY as usize);
        for i in 0..50u8 {
            let pub_bytes = vec![200u8.wrapping_add(i); 32];
            assert!(!lim.allow_bind(&pub_bytes, t0));
            assert!(
                !lim.by_session_pub.contains_key(&pub_bytes),
                "global deny must not insert fresh pub"
            );
        }
        assert_eq!(lim.by_session_pub.len(), map_len);
    }

    #[test]
    fn allow_bind_then_jwt_fail_forget_keeps_global_spent() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        let pub_bytes = vec![9u8; 32];
        let global_before = lim.bind_global_tokens(t0);
        assert!(lim.allow_bind(&pub_bytes, t0));
        assert!(lim.by_session_pub.contains_key(&pub_bytes));
        lim.forget_session_pub(&pub_bytes);
        assert!(!lim.by_session_pub.contains_key(&pub_bytes));
        assert!(
            (lim.bind_global_tokens(t0) - (global_before - 1.0)).abs() < f64::EPSILON,
            "JWT-fail forget drops per-pub but global stay consumed"
        );
    }

    #[test]
    fn successful_rebind_keeps_same_pub_bucket_when_not_forgotten() {
        let mut lim = IntentRateLimiter::new();
        let t0 = Instant::now();
        let pub_bytes = vec![3u8; 32];
        assert!(lim.allow_bind(&pub_bytes, t0));
        let after_one = lim.bind_pub_tokens(&pub_bytes, t0);
        // Simulate commit_bind GC that excludes the current pub.
        lim.forget_session_pubs(&[vec![99u8; 32]]);
        assert!(lim.by_session_pub.contains_key(&pub_bytes));
        assert!((lim.bind_pub_tokens(&pub_bytes, t0) - after_one).abs() < f64::EPSILON);
    }
}
