//! Allocate subject throughput — epoch fixed windows on `sub` + peer IP (x0088 / P1-5).
//!
//! Guarantees: after JWT verify, short+long epoch windows (`unix_ms / period_ms`, same shape
//! as Elixir `AlchemyRateLimit`) for verified `sub` and peer IP; excess returns retry-after
//! before OpenSSL. Rejecting axis increments (throttle still consumes that axis); earlier axes
//! roll back if a later axis rejects. Per-window map capped (LRU; idle keys evicted first).
//! Single-process only.
//! Non-guarantees: distributed counters; `sub`/IP as metric labels; token-bucket smoothness;
//! `X-Forwarded-For` (loopback/tunnel peer is often `127.0.0.1` → IP axis ≈ process-wide).

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::time::Duration;

/// Cap unique keys per fixed window (aligned with `MAX_UNKNOWN_KID_ENTRIES`).
pub const MAX_WINDOW_KEYS: usize = 256;

/// Which axis tripped the limit (for low-cardinality metrics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThroughputAxis {
    Sub,
    Ip,
}

/// Outcome of [`SubjectThroughput::hit`] when wrapped by the agent lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThroughputError {
    Limited {
        axis: ThroughputAxis,
        retry_after_secs: u64,
    },
    LockPoisoned,
}

impl ThroughputAxis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sub => "sub",
            Self::Ip => "ip",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ThroughputLimits {
    pub short_window: Duration,
    pub long_window: Duration,
    pub sub_short: u32,
    pub sub_long: u32,
    pub ip_short: u32,
    pub ip_long: u32,
}

impl Default for ThroughputLimits {
    fn default() -> Self {
        Self {
            short_window: Duration::from_secs(10),
            long_window: Duration::from_secs(60),
            sub_short: 3,
            sub_long: 10,
            ip_short: 8,
            ip_long: 30,
        }
    }
}

impl ThroughputLimits {
    /// Env overrides (empty / invalid → keep default field), then [`sanitize`].
    pub fn from_env() -> Self {
        let mut lim = Self::default();
        if let Ok(v) = std::env::var("RELAY_ALLOCATE_SHORT_WINDOW_SECS") {
            if let Ok(secs) = v.parse::<u64>() {
                lim.short_window = Duration::from_secs(secs.max(1));
            }
        }
        if let Ok(v) = std::env::var("RELAY_ALLOCATE_LONG_WINDOW_SECS") {
            if let Ok(secs) = v.parse::<u64>() {
                lim.long_window = Duration::from_secs(secs.max(1));
            }
        }
        if let Ok(v) = std::env::var("RELAY_ALLOCATE_SUB_SHORT_LIMIT") {
            if let Ok(n) = v.parse::<u32>() {
                lim.sub_short = n.max(1);
            }
        }
        if let Ok(v) = std::env::var("RELAY_ALLOCATE_SUB_LONG_LIMIT") {
            if let Ok(n) = v.parse::<u32>() {
                lim.sub_long = n.max(1);
            }
        }
        if let Ok(v) = std::env::var("RELAY_ALLOCATE_IP_SHORT_LIMIT") {
            if let Ok(n) = v.parse::<u32>() {
                lim.ip_short = n.max(1);
            }
        }
        if let Ok(v) = std::env::var("RELAY_ALLOCATE_IP_LONG_LIMIT") {
            if let Ok(n) = v.parse::<u32>() {
                lim.ip_long = n.max(1);
            }
        }
        lim.sanitize()
    }

    /// Clamp inverted window/limit pairs; log when env values were adjusted.
    /// On limit inversion, **lower short** (safer under DoS) rather than raising long.
    pub fn sanitize(mut self) -> Self {
        if self.short_window > self.long_window {
            log::warn!(
                "RELAY_ALLOCATE_* windows inverted (short={:?} > long={:?}); swapping",
                self.short_window,
                self.long_window
            );
            std::mem::swap(&mut self.short_window, &mut self.long_window);
        }
        if self.sub_short > self.sub_long {
            log::warn!(
                "RELAY_ALLOCATE_SUB_* limits inverted (short={} > long={}); lowering short",
                self.sub_short,
                self.sub_long
            );
            self.sub_short = self.sub_long.max(1);
        }
        if self.ip_short > self.ip_long {
            log::warn!(
                "RELAY_ALLOCATE_IP_* limits inverted (short={} > long={}); lowering short",
                self.ip_short,
                self.ip_long
            );
            self.ip_short = self.ip_long.max(1);
        }
        self
    }

    pub fn log_effective(&self) {
        log::info!(
            "allocate throughput limits short={:?} long={:?} sub={}/{} ip={}/{} (max_keys={MAX_WINDOW_KEYS})",
            self.short_window,
            self.long_window,
            self.sub_short,
            self.sub_long,
            self.ip_short,
            self.ip_long
        );
    }

    fn short_period_ms(self) -> u64 {
        self.short_window.as_millis().max(1) as u64
    }

    fn long_period_ms(self) -> u64 {
        self.long_window.as_millis().max(1) as u64
    }
}

#[derive(Debug, Clone, Copy)]
struct WindowEntry {
    epoch: u64,
    count: u32,
}

#[derive(Debug, Default)]
struct FixedWindow {
    /// key → (epoch, count)
    entries: HashMap<String, WindowEntry>,
    /// LRU order: front = idle (evict first), back = recently hit.
    order: VecDeque<String>,
}

impl FixedWindow {
    /// Elixir-shaped hit: `count += 1` then reject if `count > limit` (throttle consumes).
    fn hit(&mut self, key: &str, now_ms: u64, period_ms: u64, limit: u32) -> Result<(), u64> {
        let epoch = now_ms / period_ms.max(1);
        self.purge(epoch);
        self.ensure_entry(key, epoch);
        let count = {
            let Some(entry) = self.entries.get_mut(key) else {
                // Should be unreachable after ensure_entry; fail closed without panicking the accept thread.
                log::error!(
                    "throughput window entry missing after ensure key_len={}",
                    key.len()
                );
                return Err(1);
            };
            if entry.epoch != epoch {
                *entry = WindowEntry { epoch, count: 0 };
            }
            entry.count = entry.count.saturating_add(1);
            entry.count
        };
        self.touch(key);
        if count > limit {
            let next_ms = (epoch + 1).saturating_mul(period_ms.max(1));
            let retry = next_ms.saturating_sub(now_ms).div_ceil(1000).max(1);
            return Err(retry);
        }
        Ok(())
    }

    fn unhit(&mut self, key: &str, now_ms: u64, period_ms: u64) {
        let epoch = now_ms / period_ms.max(1);
        if let Some(entry) = self.entries.get_mut(key) {
            if entry.epoch == epoch && entry.count > 0 {
                entry.count -= 1;
            }
        }
    }

    fn ensure_entry(&mut self, key: &str, epoch: u64) {
        if self.entries.contains_key(key) {
            return;
        }
        while self.entries.len() >= MAX_WINDOW_KEYS {
            if let Some(evict) = self.order.pop_front() {
                self.entries.remove(&evict);
            } else {
                break;
            }
        }
        let owned = key.to_string();
        self.entries
            .insert(owned.clone(), WindowEntry { epoch, count: 0 });
        self.order.push_back(owned);
    }

    /// Move `key` to the MRU end so active subjects survive cap pressure.
    fn touch(&mut self, key: &str) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            if let Some(owned) = self.order.remove(pos) {
                self.order.push_back(owned);
            }
        }
    }

    fn purge(&mut self, current_epoch: u64) {
        // Drop entries two epochs behind (AlchemyRateLimit cleanup: cutoff = now/period - 2).
        let cutoff = current_epoch.saturating_sub(2);
        self.entries.retain(|_, e| e.epoch >= cutoff);
        self.order.retain(|k| self.entries.contains_key(k));
    }
}

/// Four fixed windows: sub short/long + IP short/long.
#[derive(Debug)]
pub struct SubjectThroughput {
    limits: ThroughputLimits,
    sub_short: FixedWindow,
    sub_long: FixedWindow,
    ip_short: FixedWindow,
    ip_long: FixedWindow,
}

impl SubjectThroughput {
    pub fn new(limits: ThroughputLimits) -> Self {
        Self {
            limits: limits.sanitize(),
            sub_short: FixedWindow::default(),
            sub_long: FixedWindow::default(),
            ip_short: FixedWindow::default(),
            ip_long: FixedWindow::default(),
        }
    }

    /// Test helper: skip sanitize so inverted short/long pairs can exercise rollback paths.
    #[cfg(test)]
    fn new_unchecked(limits: ThroughputLimits) -> Self {
        Self {
            limits,
            sub_short: FixedWindow::default(),
            sub_long: FixedWindow::default(),
            ip_short: FixedWindow::default(),
            ip_long: FixedWindow::default(),
        }
    }

    pub fn from_env() -> Self {
        let limits = ThroughputLimits::from_env();
        limits.log_effective();
        Self::new(limits)
    }

    #[cfg(test)]
    fn sub_short_count(&self, key: &str) -> u32 {
        self.sub_short
            .entries
            .get(key)
            .map(|e| e.count)
            .unwrap_or(0)
    }

    /// Consume one hit on all axes. Rejecting axis keeps its increment; earlier axes roll back.
    pub fn hit(&mut self, sub: &str, ip: IpAddr, now_ms: u64) -> Result<(), (ThroughputAxis, u64)> {
        let ip_key = ip.to_string();
        let short_ms = self.limits.short_period_ms();
        let long_ms = self.limits.long_period_ms();

        self.sub_short
            .hit(sub, now_ms, short_ms, self.limits.sub_short)
            .map_err(|r| (ThroughputAxis::Sub, r))?;

        if let Err(r) = self
            .sub_long
            .hit(sub, now_ms, long_ms, self.limits.sub_long)
        {
            self.sub_short.unhit(sub, now_ms, short_ms);
            return Err((ThroughputAxis::Sub, r));
        }

        if let Err(r) = self
            .ip_short
            .hit(&ip_key, now_ms, short_ms, self.limits.ip_short)
        {
            self.sub_long.unhit(sub, now_ms, long_ms);
            self.sub_short.unhit(sub, now_ms, short_ms);
            return Err((ThroughputAxis::Ip, r));
        }

        if let Err(r) = self
            .ip_long
            .hit(&ip_key, now_ms, long_ms, self.limits.ip_long)
        {
            self.ip_short.unhit(&ip_key, now_ms, short_ms);
            self.sub_long.unhit(sub, now_ms, long_ms);
            self.sub_short.unhit(sub, now_ms, short_ms);
            return Err((ThroughputAxis::Ip, r));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn sub_short_window_throttles() {
        let mut t = SubjectThroughput::new(ThroughputLimits {
            short_window: Duration::from_secs(10),
            long_window: Duration::from_secs(60),
            sub_short: 2,
            sub_long: 100,
            ip_short: 100,
            ip_long: 100,
        });
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let now = 1_000_000u64;
        assert!(t.hit("alice", ip, now).is_ok());
        assert!(t.hit("alice", ip, now).is_ok());
        let err = t.hit("alice", ip, now).unwrap_err();
        assert_eq!(err.0, ThroughputAxis::Sub);
        assert!(err.1 >= 1);
        assert!(t.hit("bob", ip, now).is_ok());
    }

    #[test]
    fn ip_short_window_throttles_across_subs() {
        let mut t = SubjectThroughput::new(ThroughputLimits {
            short_window: Duration::from_secs(10),
            long_window: Duration::from_secs(60),
            sub_short: 100,
            sub_long: 100,
            ip_short: 2,
            ip_long: 100,
        });
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let now = 1_000_000u64;
        assert!(t.hit("a", ip, now).is_ok());
        assert!(t.hit("b", ip, now).is_ok());
        let err = t.hit("c", ip, now).unwrap_err();
        assert_eq!(err.0, ThroughputAxis::Ip);
    }

    #[test]
    fn window_resets_on_next_epoch() {
        let mut t = SubjectThroughput::new(ThroughputLimits {
            short_window: Duration::from_millis(50),
            long_window: Duration::from_secs(60),
            sub_short: 1,
            sub_long: 100,
            ip_short: 100,
            ip_long: 100,
        });
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let t0 = 100u64;
        assert!(t.hit("alice", ip, t0).is_ok());
        assert!(t.hit("alice", ip, t0).is_err());
        // Next 50ms epoch.
        assert!(t.hit("alice", ip, t0 + 50).is_ok());
    }

    #[test]
    fn later_axis_reject_rolls_back_earlier() {
        let mut t = SubjectThroughput::new(ThroughputLimits {
            short_window: Duration::from_secs(10),
            long_window: Duration::from_secs(60),
            sub_short: 100,
            sub_long: 100,
            ip_short: 1,
            ip_long: 100,
        });
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let now = 1_000_000u64;
        assert!(t.hit("alice", ip, now).is_ok());
        // IP full → 429; sub short must not be eaten by retries.
        let err = t.hit("alice", ip, now).unwrap_err();
        assert_eq!(err.0, ThroughputAxis::Ip);
        assert!(t.hit("alice", ip, now).is_err());
        assert_eq!(t.sub_short_count("alice"), 1);
    }

    #[test]
    fn sub_long_reject_rolls_back_sub_short() {
        // Unchecked: short>long is invalid for production sanitize, but exercises rollback order.
        let mut t = SubjectThroughput::new_unchecked(ThroughputLimits {
            short_window: Duration::from_secs(10),
            long_window: Duration::from_secs(60),
            sub_short: 5,
            sub_long: 1,
            ip_short: 100,
            ip_long: 100,
        });
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let now = 1_000_000u64;
        assert!(t.hit("alice", ip, now).is_ok());
        let err = t.hit("alice", ip, now).unwrap_err();
        assert_eq!(err.0, ThroughputAxis::Sub);
        assert!(t.hit("alice", ip, now).is_err());
        assert_eq!(t.sub_short_count("alice"), 1);
    }

    #[test]
    fn sanitize_inverts_windows_and_limits() {
        let lim = ThroughputLimits {
            short_window: Duration::from_secs(60),
            long_window: Duration::from_secs(10),
            sub_short: 20,
            sub_long: 5,
            ip_short: 40,
            ip_long: 10,
        }
        .sanitize();
        assert_eq!(lim.short_window, Duration::from_secs(10));
        assert_eq!(lim.long_window, Duration::from_secs(60));
        assert_eq!(lim.sub_short, 5);
        assert_eq!(lim.sub_long, 5);
        assert_eq!(lim.ip_short, 10);
        assert_eq!(lim.ip_long, 10);
    }

    #[test]
    fn window_key_cap_evicts_lru_idle() {
        let mut w = FixedWindow::default();
        w.hit("keep", 1_000, 10_000, 100).unwrap();
        for i in 0..MAX_WINDOW_KEYS {
            let key = format!("k{i}");
            w.hit(&key, 1_000, 10_000, 100).unwrap();
            // Re-touch active key so it stays MRU while idle keys fill the cap.
            w.hit("keep", 1_000, 10_000, u32::MAX).unwrap();
        }
        assert!(w.entries.len() <= MAX_WINDOW_KEYS);
        assert!(w.entries.contains_key("keep"));
        assert!(!w.entries.contains_key("k0"));
    }
}
