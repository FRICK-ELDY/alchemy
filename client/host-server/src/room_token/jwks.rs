//! JWKS cache, fetch, background refresh, and soft/hard TTL kick logic for [`RoomTokenVerifier`].
//!
//! [`RoomTokenVerifier`] is defined in `verify` (claims + `verify` / `new`). This module owns the
//! cache fields' HTTP/SWR path: `refresh_jwks`, background spawn, soft/hard kick helpers.
//!
//! Guarantees: x0021 verify never does HTTP (HTTP only in `refresh_jwks` / background thread);
//! x0040 soft TTL (SWR) + hard stale TTL (verify fails closed until refresh); unknown-kid /
//! hard-stale kicks are rate-limited by [`UNKNOWN_KID_COOLDOWN`].
//! Non-guarantees: JWT claim validation (see `verify`).

use super::verify::RoomTokenVerifier;
use anyhow::{anyhow, Context, Result};
use jsonwebtoken::DecodingKey;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Soft TTL: after this, verify still uses cache and kicks background refresh (SWR).
pub(super) const JWKS_SOFT_TTL: Duration = Duration::from_secs(60);
/// Hard stale: beyond this, Room Token verify fails until refresh (x0040 / P1-5).
pub(super) const JWKS_HARD_TTL: Duration = Duration::from_secs(300);
const JWKS_HTTP_TIMEOUT: Duration = Duration::from_secs(2);
const JWKS_BG_POLL: Duration = Duration::from_secs(30);
const UNKNOWN_KID_COOLDOWN: Duration = Duration::from_secs(1);

#[derive(Debug, Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

#[derive(Debug, Deserialize)]
struct Jwk {
    kid: Option<String>,
    kty: String,
    n: Option<String>,
    e: Option<String>,
}

pub(super) struct KeyCache {
    pub(super) keys: HashMap<String, DecodingKey>,
    pub(super) fetched_at: Instant,
    pub(super) last_unknown_kid_kick: Option<Instant>,
    pub(super) last_hard_stale_kick: Option<Instant>,
}

impl RoomTokenVerifier {
    /// Attach host metrics registry (x0049). Call once before verify traffic.
    pub fn set_metrics(&self, metrics: Arc<crate::metrics::HostMetrics>) {
        if let Ok(mut slot) = self.metrics.lock() {
            *slot = Some(metrics);
        }
        self.publish_jwks_age();
    }

    pub(super) fn with_metrics(&self, f: impl FnOnce(&crate::metrics::HostMetrics)) {
        if let Ok(slot) = self.metrics.lock() {
            if let Some(m) = slot.as_ref() {
                f(m);
            }
        }
    }

    pub(super) fn publish_jwks_age(&self) {
        let fetched_at = self.cache.read().expect("jwks cache lock").fetched_at;
        self.with_metrics(|m| m.set_jwks_fetched_at(fetched_at));
    }

    /// Poll JWKS off the input path. Safe to call once after initial [`Self::refresh_jwks`].
    pub fn spawn_background_refresh(self: &Arc<Self>) {
        if self
            .bg_started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        let (tx, rx) = mpsc::sync_channel::<()>(1);
        if let Ok(mut slot) = self.refresh_tx.lock() {
            *slot = Some(tx);
        }
        let this = Arc::clone(self);
        thread::Builder::new()
            .name("jwks-refresh".into())
            .spawn(move || {
                while let Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) =
                    rx.recv_timeout(JWKS_BG_POLL)
                {
                    if let Err(err) = this.refresh_jwks() {
                        log::warn!("JWKS background refresh failed (keeping cache): {err:#}");
                    }
                    this.publish_jwks_age();
                }
            })
            .expect("spawn jwks-refresh");
    }

    /// Fetch JWKS over HTTP (startup / background only — not from `verify`).
    pub fn refresh_jwks(&self) -> Result<()> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(JWKS_HTTP_TIMEOUT)
            .timeout_read(JWKS_HTTP_TIMEOUT)
            .build();
        let body: Jwks = agent
            .get(&self.jwks_url)
            .call()
            .context("fetch JWKS")?
            .into_json()
            .context("parse JWKS")?;

        let mut map = HashMap::new();
        for jwk in body.keys {
            if jwk.kty != "RSA" {
                continue;
            }
            let kid = jwk.kid.unwrap_or_default();
            let n = jwk.n.ok_or_else(|| anyhow!("JWK missing n"))?;
            let e = jwk.e.ok_or_else(|| anyhow!("JWK missing e"))?;
            let key = DecodingKey::from_rsa_components(&n, &e)
                .context("DecodingKey from RSA components")?;
            if !kid.is_empty() {
                map.insert(kid, key);
            }
        }
        if map.is_empty() {
            return Err(anyhow!("JWKS contained no usable RSA keys"));
        }
        let mut cache = self.cache.write().expect("jwks cache lock");
        cache.keys = map;
        cache.fetched_at = Instant::now();
        drop(cache);
        self.publish_jwks_age();
        Ok(())
    }

    pub(super) fn has_kid(&self, kid: &str) -> bool {
        let cache = self.cache.read().expect("jwks cache lock");
        cache.keys.contains_key(kid)
    }

    pub(super) fn kick_refresh(&self) {
        if let Ok(guard) = self.refresh_tx.lock() {
            if let Some(tx) = guard.as_ref() {
                let _ = tx.try_send(());
            }
        }
    }

    pub(super) fn kick_refresh_unknown_kid(&self) {
        {
            let mut cache = self.cache.write().expect("jwks cache lock");
            if let Some(last) = cache.last_unknown_kid_kick {
                if last.elapsed() < UNKNOWN_KID_COOLDOWN {
                    return;
                }
            }
            cache.last_unknown_kid_kick = Some(Instant::now());
        }
        self.kick_refresh();
    }

    pub(super) fn kick_refresh_hard_stale(&self) {
        {
            let mut cache = self.cache.write().expect("jwks cache lock");
            if let Some(last) = cache.last_hard_stale_kick {
                if last.elapsed() < UNKNOWN_KID_COOLDOWN {
                    return;
                }
            }
            cache.last_hard_stale_kick = Some(Instant::now());
        }
        self.kick_refresh();
    }
}
