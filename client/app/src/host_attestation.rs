//! Verify world-issued HostAttestation JWTs (x0025).
//!
//! Guarantees: RS256 via world JWKS; typ/aud/room_id/host_pub match.
//! Non-guarantees: Manifest body integrity; offline / cached JWKS freshness beyond soft TTL.

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

const ISSUER: &str = "alchemy-world";
const AUDIENCE: &str = "alchemy-host-attestation";
const TYP: &str = "host_attestation";
const JWKS_SOFT_TTL: Duration = Duration::from_secs(60);
const JWKS_HTTP_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Deserialize)]
pub struct HostAttestationClaims {
    pub typ: String,
    pub sub: String,
    pub room_id: String,
    /// base64url (no pad) of 32-byte Ed25519 pub.
    pub host_pub: String,
    pub exp: i64,
}

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

/// World JWKS cache for HostAttestation verification.
pub struct HostAttestationVerifier {
    jwks_url: String,
    expected_room_id: String,
    cache: RwLock<KeyCache>,
}

impl HostAttestationVerifier {
    pub fn new(world_url: &str, room_id: &str) -> Self {
        let base = world_url.trim_end_matches('/');
        Self {
            jwks_url: format!("{base}/.well-known/jwks.json"),
            expected_room_id: room_id.to_string(),
            cache: RwLock::new(KeyCache {
                keys: HashMap::new(),
                fetched_at: Instant::now() - Duration::from_secs(3600),
            }),
        }
    }

    pub fn refresh_jwks(&self) -> Result<()> {
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

    fn ensure_jwks(&self) -> Result<()> {
        let stale = {
            let cache = self.cache.read().expect("jwks cache lock");
            cache.keys.is_empty() || cache.fetched_at.elapsed() > JWKS_SOFT_TTL
        };
        if stale {
            self.refresh_jwks()?;
        }
        Ok(())
    }

    /// Verify JWT and return raw 32-byte `host_pub` if it matches `manifest_host_pub`.
    pub fn verify_and_host_pub(
        &self,
        attestation: &str,
        manifest_host_pub: &[u8],
    ) -> Result<Vec<u8>> {
        if attestation.is_empty() {
            return Err(anyhow!("empty host_attestation"));
        }
        self.ensure_jwks()?;
        let header = decode_header(attestation).context("attestation header")?;
        let kid = header
            .kid
            .ok_or_else(|| anyhow!("attestation missing kid"))?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[ISSUER]);
        validation.set_audience(&[AUDIENCE]);
        validation.validate_exp = true;

        // Soft TTL 内でも unknown kid なら一回限り JWKS 再取得（鍵ローテ直後）。
        if !self.has_kid(&kid) {
            self.refresh_jwks()
                .context("JWKS refresh after unknown kid")?;
        }
        let claims = self
            .decode_with_cached_kid(attestation, &kid, &validation)
            .with_context(|| format!("verify HostAttestation (kid={kid})"))?;

        if claims.typ != TYP {
            return Err(anyhow!("attestation typ mismatch: {}", claims.typ));
        }
        if claims.room_id != self.expected_room_id {
            return Err(anyhow!(
                "attestation room_id mismatch: {} expected {}",
                claims.room_id,
                self.expected_room_id
            ));
        }
        let host_pub = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(claims.host_pub.as_bytes())
            .context("decode attestation host_pub")?;
        if host_pub.len() != 32 {
            return Err(anyhow!(
                "attestation host_pub length {} (want 32)",
                host_pub.len()
            ));
        }
        if host_pub.as_slice() != manifest_host_pub {
            return Err(anyhow!(
                "attestation host_pub does not match Manifest.host_pub"
            ));
        }
        let _ = claims.sub;
        let _ = claims.exp;
        Ok(host_pub)
    }

    fn has_kid(&self, kid: &str) -> bool {
        let cache = self.cache.read().expect("jwks cache lock");
        cache.keys.contains_key(kid)
    }

    fn decode_with_cached_kid(
        &self,
        attestation: &str,
        kid: &str,
        validation: &Validation,
    ) -> Result<HostAttestationClaims> {
        let cache = self.cache.read().expect("jwks cache lock");
        let key = cache
            .keys
            .get(kid)
            .ok_or_else(|| anyhow!("unknown kid: {kid}"))?;
        Ok(
            decode::<HostAttestationClaims>(attestation, key, validation)
                .context("RS256 verify")?
                .claims,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_attestation_rejected() {
        let v = HostAttestationVerifier::new("http://127.0.0.1:4003", "local");
        assert!(v.verify_and_host_pub("", &[0u8; 32]).is_err());
    }
}
