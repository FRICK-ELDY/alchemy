//! Credential TTL arithmetic (pure; no I/O).
//!
//! Guarantees: soft refresh at 80% of TTL, hard expiry at full TTL; `expires_in=0` => no deadline.
//! Non-guarantees: HTTP, token verification, worker scheduling.

use std::time::{Duration, Instant};

/// Soft refresh fraction of token TTL (match client x0021).
pub const TOKEN_REFRESH_FRACTION: f64 = 0.8;

/// Hint TTL when Room Token bypass omits `expires_in` (debug).
pub(super) const BYPASS_ROOM_TOKEN_TTL_HINT: u64 = 300;

/// Instant at which a token with `expires_in` should be refreshed, or `None` if TTL is 0.
pub fn refresh_deadline(expires_in: u64) -> Option<Instant> {
    if expires_in == 0 {
        return None;
    }
    let secs = ((expires_in as f64) * TOKEN_REFRESH_FRACTION).floor() as u64;
    Some(Instant::now() + Duration::from_secs(secs.max(1)))
}

/// Instant at which a token with `expires_in` is fully expired (hard), or `None` if TTL is 0.
pub fn hard_expire_deadline(expires_in: u64) -> Option<Instant> {
    if expires_in == 0 {
        return None;
    }
    Some(Instant::now() + Duration::from_secs(expires_in))
}

/// Earliest hard-expiry among candidates (`None` entries ignored).
pub fn earliest_hard_expire(candidates: &[Option<Instant>]) -> Option<Instant> {
    candidates.iter().flatten().copied().min()
}

/// Publish gate: once hard-expired, publishing stops until a successful refresh re-enables it.
pub fn publish_allowed_at(
    now: Instant,
    hard_expire_at: Option<Instant>,
    currently_allowed: bool,
) -> bool {
    currently_allowed && !matches!(hard_expire_at, Some(at) if now >= at)
}

pub(super) fn hard_expire_or_hint(expires_in: u64, hint_secs: u64) -> Instant {
    hard_expire_deadline(expires_in)
        .unwrap_or_else(|| Instant::now() + Duration::from_secs(hint_secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_deadline_none_when_ttl_zero() {
        assert!(refresh_deadline(0).is_none());
    }

    #[test]
    fn refresh_deadline_at_80_percent() {
        let start = Instant::now();
        let at = refresh_deadline(100).expect("ttl>0");
        let delta = at.duration_since(start);
        assert!(delta >= Duration::from_secs(79) && delta <= Duration::from_secs(81));
    }

    #[test]
    fn hard_expire_deadline_full_ttl() {
        let start = Instant::now();
        let at = hard_expire_deadline(100).expect("ttl>0");
        let delta = at.duration_since(start);
        assert!(delta >= Duration::from_secs(99) && delta <= Duration::from_secs(101));
        assert!(hard_expire_deadline(0).is_none());
    }

    #[test]
    fn earliest_hard_expire_picks_min() {
        let a = Instant::now() + Duration::from_secs(10);
        let b = Instant::now() + Duration::from_secs(5);
        assert_eq!(earliest_hard_expire(&[Some(a), None, Some(b)]), Some(b));
        assert!(earliest_hard_expire(&[None, None]).is_none());
    }

    #[test]
    fn publish_allowed_clears_after_hard_expiry() {
        let now = Instant::now();
        assert!(publish_allowed_at(
            now,
            Some(now + Duration::from_secs(1)),
            true
        ));
        assert!(!publish_allowed_at(
            now,
            Some(now - Duration::from_millis(1)),
            true
        ));
        assert!(!publish_allowed_at(
            now,
            Some(now + Duration::from_secs(60)),
            false
        ));
        assert!(publish_allowed_at(now, None, true));
    }
}
