//! Shared world-server JWKS cache for RS256 JWT verification (x0025 / x0026 / **x0044**).
//!
//! Guarantees: soft TTL SWR + hard stale fail-closed; unknown kid kicks bg refresh with
//! cooldown; **decode path never does HTTP** (parity with host `room_token` / P2-3).
//! Non-guarantees: offline freshness beyond hard TTL; per-audience claim checks (callers);
//! **empty cache does not kick bg refresh** (startup must call [`WorldJwks::refresh`] first —
//! same as host; a never-populated cache stays empty until an explicit refresh).

use anyhow::{anyhow, Context, Result};
use jsonwebtoken::{decode, decode_header, DecodingKey, Validation};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

const JWKS_SOFT_TTL: Duration = Duration::from_secs(60);
/// Hard stale: beyond this, decode fails until background refresh (x0044 / P2-3).
const JWKS_HARD_TTL: Duration = Duration::from_secs(300);
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

struct KeyCache {
    keys: HashMap<String, DecodingKey>,
    fetched_at: Instant,
    last_unknown_kid_kick: Option<Instant>,
    last_hard_stale_kick: Option<Instant>,
}

/// Cached world JWKS shared by HostAttestation and ManifestSeal verifiers.
pub struct WorldJwks {
    jwks_url: RwLock<String>,
    cache: RwLock<KeyCache>,
    refresh_tx: Mutex<Option<SyncSender<()>>>,
    bg_started: AtomicBool,
    /// Optional x0049 client metrics (JWKS age / hard-stale).
    metrics: Mutex<Option<Arc<crate::metrics::ClientMetrics>>>,
}

impl WorldJwks {
    pub fn new(world_url: &str) -> Self {
        let base = world_url.trim_end_matches('/');
        Self {
            jwks_url: RwLock::new(format!("{base}/.well-known/jwks.json")),
            cache: RwLock::new(KeyCache {
                keys: HashMap::new(),
                fetched_at: Instant::now() - Duration::from_secs(3600),
                last_unknown_kid_kick: None,
                last_hard_stale_kick: None,
            }),
            refresh_tx: Mutex::new(None),
            bg_started: AtomicBool::new(false),
            metrics: Mutex::new(None),
        }
    }

    /// Point at a new world base URL and invalidate cache (x0066 Connect Apply, unjoined).
    pub fn retarget_world(&self, world_url: &str) {
        let base = world_url.trim_end_matches('/');
        let url = format!("{base}/.well-known/jwks.json");
        *self.jwks_url.write().expect("jwks_url lock") = url;
        let mut cache = self.cache.write().expect("jwks cache lock");
        cache.keys.clear();
        cache.fetched_at = Instant::now() - Duration::from_secs(3600);
        cache.last_unknown_kid_kick = None;
        cache.last_hard_stale_kick = None;
    }

    /// Attach client metrics registry (x0049). Call once before verify traffic.
    pub fn set_metrics(&self, metrics: Arc<crate::metrics::ClientMetrics>) {
        if let Ok(mut slot) = self.metrics.lock() {
            *slot = Some(metrics);
        }
        self.publish_jwks_age();
    }

    fn with_metrics(&self, f: impl FnOnce(&crate::metrics::ClientMetrics)) {
        if let Ok(slot) = self.metrics.lock() {
            if let Some(m) = slot.as_ref() {
                f(m);
            }
        }
    }

    fn publish_jwks_age(&self) {
        let fetched_at = self.cache.read().expect("jwks cache lock").fetched_at;
        self.with_metrics(|m| m.set_jwks_fetched_at(fetched_at));
    }

    /// Poll JWKS off the Manifest receive path. Call once after initial [`Self::refresh`].
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
            .name("world-jwks-refresh".into())
            .spawn(move || {
                while let Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) =
                    rx.recv_timeout(JWKS_BG_POLL)
                {
                    if let Err(err) = this.refresh() {
                        log::warn!("world JWKS background refresh failed (keeping cache): {err:#}");
                    }
                    this.publish_jwks_age();
                }
            })
            .expect("spawn world-jwks-refresh");
    }

    /// Fetch JWKS over HTTP (startup / background only — not from `decode_claims`).
    pub fn refresh(&self) -> Result<()> {
        let url = self.jwks_url.read().expect("jwks_url lock").clone();
        let agent = ureq::AgentBuilder::new().timeout(JWKS_HTTP_TIMEOUT).build();
        let jwks: Jwks = agent
            .get(&url)
            .call()
            .context("fetch world JWKS")?
            .into_json()
            .context("parse world JWKS")?;
        let mut keys = HashMap::new();
        for jwk in jwks.keys {
            if jwk.kty != "RSA" {
                continue;
            }
            let (Some(kid), Some(n), Some(e)) = (jwk.kid, jwk.n, jwk.e) else {
                continue;
            };
            let key = DecodingKey::from_rsa_components(&n, &e).context("JWKS RSA components")?;
            keys.insert(kid, key);
        }
        if keys.is_empty() {
            return Err(anyhow!("world JWKS empty"));
        }
        let mut cache = self.cache.write().expect("jwks cache lock");
        cache.keys = keys;
        cache.fetched_at = Instant::now();
        drop(cache);
        self.publish_jwks_age();
        Ok(())
    }

    /// Decode RS256 JWT claims from cache only (no HTTP). Soft-stale still accepts (SWR);
    /// hard-stale / unknown kid reject and kick background refresh (x0044).
    pub fn decode_claims<T: DeserializeOwned>(
        &self,
        token: &str,
        validation: &Validation,
    ) -> Result<T> {
        let header = decode_header(token).context("JWT header")?;
        let kid = header.kid.ok_or_else(|| anyhow!("JWT missing kid"))?;

        {
            let cache = self.cache.read().expect("jwks cache lock");
            // Intentional: no kick_refresh on empty (host parity). Startup refresh is required;
            // otherwise decode fails closed until an explicit `refresh` / bg poll after seed.
            if cache.keys.is_empty() {
                return Err(anyhow!("JWKS cache empty; call refresh before decode"));
            }
            let age = cache.fetched_at.elapsed();
            if age > JWKS_HARD_TTL {
                drop(cache);
                self.with_metrics(|m| m.inc_jwks_hard_stale());
                self.kick_refresh_hard_stale();
                return Err(anyhow!(
                    "JWKS cache hard-stale (age > {}s); refresh required (x0044)",
                    JWKS_HARD_TTL.as_secs()
                ));
            }
            if age > JWKS_SOFT_TTL {
                drop(cache);
                self.kick_refresh();
            }
        }

        if !self.has_kid(&kid) {
            self.kick_refresh_unknown_kid();
            return Err(anyhow!("unknown kid: {kid}"));
        }

        self.decode_with_cached_kid(token, &kid, validation)
            .with_context(|| format!("RS256 verify (kid={kid})"))
    }

    fn has_kid(&self, kid: &str) -> bool {
        let cache = self.cache.read().expect("jwks cache lock");
        cache.keys.contains_key(kid)
    }

    fn decode_with_cached_kid<T: DeserializeOwned>(
        &self,
        token: &str,
        kid: &str,
        validation: &Validation,
    ) -> Result<T> {
        let cache = self.cache.read().expect("jwks cache lock");
        let key = cache
            .keys
            .get(kid)
            .ok_or_else(|| anyhow!("unknown kid: {kid}"))?;
        Ok(decode::<T>(token, key, validation)
            .context("RS256 verify")?
            .claims)
    }

    fn kick_refresh(&self) {
        if let Ok(guard) = self.refresh_tx.lock() {
            if let Some(tx) = guard.as_ref() {
                let _ = tx.try_send(());
            }
        }
    }

    fn kick_refresh_unknown_kid(&self) {
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

    fn kick_refresh_hard_stale(&self) {
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

    /// Test-only: inject a key so verify works without HTTP.
    #[cfg(test)]
    pub fn insert_decoding_key_for_test(&self, kid: &str, key: DecodingKey) {
        let mut cache = self.cache.write().expect("jwks cache lock");
        cache.keys.insert(kid.to_string(), key);
        cache.fetched_at = Instant::now();
    }

    /// Test-only: age the JWKS cache (for soft/hard stale tests).
    #[cfg(test)]
    pub fn set_fetched_at_age_for_test(&self, age: Duration) {
        let mut cache = self.cache.write().expect("jwks cache lock");
        cache.fetched_at = Instant::now()
            .checked_sub(age)
            .expect("fetched_at age within Instant range");
    }

    /// Test-only: unknown-kid kick timestamp (cooldown assertions).
    #[cfg(test)]
    fn last_unknown_kid_kick_for_test(&self) -> Option<Instant> {
        self.cache
            .read()
            .expect("jwks cache lock")
            .last_unknown_kid_kick
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use pkcs1::EncodeRsaPrivateKey;
    use pkcs8::EncodePublicKey;
    use rsa::RsaPrivateKey;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize)]
    struct TinyClaims {
        sub: String,
        exp: i64,
    }

    fn test_rsa_jwt() -> (WorldJwks, String, EncodingKey) {
        let mut rng = rand::thread_rng();
        let private = RsaPrivateKey::new(&mut rng, 2048).expect("rsa");
        let priv_pem = private
            .to_pkcs1_pem(pkcs1::LineEnding::LF)
            .expect("priv pem");
        let pub_pem = private
            .to_public_key()
            .to_public_key_pem(pkcs8::LineEnding::LF)
            .expect("pub pem");
        let encoding = EncodingKey::from_rsa_pem(priv_pem.as_bytes()).expect("encoding");
        let decoding = DecodingKey::from_rsa_pem(pub_pem.as_bytes()).expect("decoding");

        let jwks = WorldJwks::new("http://127.0.0.1:9");
        jwks.insert_decoding_key_for_test("soft-kid", decoding);

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("soft-kid".into());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let token = encode(
            &header,
            &TinyClaims {
                sub: "t".into(),
                exp: now + 600,
            },
            &encoding,
        )
        .expect("jwt");
        (jwks, token, encoding)
    }

    #[test]
    fn jwks_soft_accepts_hard_rejects_until_fresh() {
        let (jwks, token, _) = test_rsa_jwt();
        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_aud = false;
        validation.set_required_spec_claims(&["exp", "sub"]);

        jwks.set_fetched_at_age_for_test(Duration::from_secs(61));
        jwks.decode_claims::<TinyClaims>(&token, &validation)
            .expect("soft-stale must still accept (SWR)");

        jwks.set_fetched_at_age_for_test(Duration::from_secs(301));
        let err = jwks
            .decode_claims::<TinyClaims>(&token, &validation)
            .expect_err("hard-stale");
        assert!(err.to_string().contains("hard-stale"), "got: {err:#}");

        jwks.set_fetched_at_age_for_test(Duration::from_secs(0));
        assert!(jwks
            .decode_claims::<TinyClaims>(&token, &validation)
            .is_ok());
    }

    #[test]
    fn hard_stale_increments_client_metrics() {
        let (jwks, token, _) = test_rsa_jwt();
        let metrics = Arc::new(crate::metrics::ClientMetrics::default());
        jwks.set_metrics(Arc::clone(&metrics));
        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_aud = false;
        validation.set_required_spec_claims(&["exp", "sub"]);

        jwks.set_fetched_at_age_for_test(Duration::from_secs(301));
        let _ = jwks
            .decode_claims::<TinyClaims>(&token, &validation)
            .expect_err("hard-stale");
        assert_eq!(metrics.jwks_hard_stale_total(), 1);
    }

    #[test]
    fn unknown_kid_rejects_without_http_and_cools_down() {
        let (jwks, _, encoding) = test_rsa_jwt();
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("missing-kid".into());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let token = encode(
            &header,
            &TinyClaims {
                sub: "t".into(),
                exp: now + 600,
            },
            &encoding,
        )
        .expect("jwt");

        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_aud = false;
        validation.set_required_spec_claims(&["exp", "sub"]);

        let err = jwks
            .decode_claims::<TinyClaims>(&token, &validation)
            .expect_err("unknown kid");
        assert!(err.to_string().contains("unknown kid"), "got: {err:#}");
        let first_kick = jwks
            .last_unknown_kid_kick_for_test()
            .expect("kick recorded");

        let err2 = jwks
            .decode_claims::<TinyClaims>(&token, &validation)
            .expect_err("still unknown");
        assert!(err2.to_string().contains("unknown kid"));
        let second_kick = jwks
            .last_unknown_kid_kick_for_test()
            .expect("kick still set");
        assert_eq!(
            first_kick, second_kick,
            "cooldown must not re-stamp kick within 1s"
        );
    }

    #[test]
    fn empty_cache_rejects_without_http() {
        let jwks = WorldJwks::new("http://127.0.0.1:9");
        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_aud = false;
        let err = jwks
            .decode_claims::<TinyClaims>("eyJhbGciOiJSUzI1NiIsImtpZCI6InQifQ.e30.e30", &validation)
            .expect_err("empty");
        assert!(
            err.to_string().contains("JWKS cache empty") || err.to_string().contains("JWT header"),
            "got: {err:#}"
        );
    }
}
