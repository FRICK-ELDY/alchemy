//! Shared world-server JWKS cache for RS256 JWT verification (x0025 / x0026).
//!
//! Guarantees: soft TTL refresh; unknown kid triggers one immediate refresh.
//! Non-guarantees: offline freshness beyond soft TTL; per-audience claim checks.

use anyhow::{anyhow, Context, Result};
use jsonwebtoken::{decode, decode_header, DecodingKey, Validation};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

const JWKS_SOFT_TTL: Duration = Duration::from_secs(60);
const JWKS_HTTP_TIMEOUT: Duration = Duration::from_secs(2);

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
}

/// Cached world JWKS shared by HostAttestation and ManifestSeal verifiers.
pub struct WorldJwks {
    jwks_url: String,
    cache: RwLock<KeyCache>,
}

impl WorldJwks {
    pub fn new(world_url: &str) -> Self {
        let base = world_url.trim_end_matches('/');
        Self {
            jwks_url: format!("{base}/.well-known/jwks.json"),
            cache: RwLock::new(KeyCache {
                keys: HashMap::new(),
                fetched_at: Instant::now() - Duration::from_secs(3600),
            }),
        }
    }

    pub fn refresh(&self) -> Result<()> {
        let agent = ureq::AgentBuilder::new().timeout(JWKS_HTTP_TIMEOUT).build();
        let jwks: Jwks = agent
            .get(&self.jwks_url)
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
        Ok(())
    }

    pub fn ensure(&self) -> Result<()> {
        let stale = {
            let cache = self.cache.read().expect("jwks cache lock");
            cache.keys.is_empty() || cache.fetched_at.elapsed() > JWKS_SOFT_TTL
        };
        if stale {
            self.refresh()?;
        }
        Ok(())
    }

    /// Decode RS256 JWT claims; unknown kid → one JWKS refresh then retry.
    pub fn decode_claims<T: DeserializeOwned>(
        &self,
        token: &str,
        validation: &Validation,
    ) -> Result<T> {
        self.ensure()?;
        let header = decode_header(token).context("JWT header")?;
        let kid = header.kid.ok_or_else(|| anyhow!("JWT missing kid"))?;
        if !self.has_kid(&kid) {
            self.refresh().context("JWKS refresh after unknown kid")?;
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

    /// Test-only: inject a key so verify works without HTTP.
    #[cfg(test)]
    pub fn insert_decoding_key_for_test(&self, kid: &str, key: DecodingKey) {
        let mut cache = self.cache.write().expect("jwks cache lock");
        cache.keys.insert(kid.to_string(), key);
        cache.fetched_at = Instant::now();
    }
}
