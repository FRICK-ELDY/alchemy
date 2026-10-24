//! Room Token verification against world-server JWKS.
//!
//! Guarantees: RS256 verify + iss/aud/exp/room_id checks.
//! Non-guarantees: auth-server user JWT, revocation lists.

use anyhow::{anyhow, Context, Result};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

const ISSUER: &str = "alchemy-world";
const AUDIENCE: &str = "alchemy-room";

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

#[derive(Debug, Clone, Deserialize)]
pub struct RoomClaims {
    #[allow(dead_code)]
    pub sub: String,
    pub room_id: String,
    pub role: String,
    /// Present for JWT exp validation / serde; not read by host logic.
    #[allow(dead_code)]
    pub exp: i64,
}

pub struct RoomTokenVerifier {
    jwks_url: String,
    expected_room_id: String,
    cache: RwLock<KeyCache>,
}

struct KeyCache {
    keys: HashMap<String, DecodingKey>,
    fetched_at: Instant,
}

impl RoomTokenVerifier {
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
        let body: Jwks = ureq::get(&self.jwks_url)
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
        Ok(())
    }

    pub fn verify(&self, token: &str) -> Result<RoomClaims> {
        if token.is_empty() {
            return Err(anyhow!("empty room token"));
        }
        self.ensure_fresh_jwks()?;

        let header = decode_header(token).context("decode JWT header")?;
        let kid = header
            .kid
            .ok_or_else(|| anyhow!("JWT header missing kid"))?;

        if !self.has_kid(&kid) {
            self.refresh_jwks()?;
        }

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[ISSUER]);
        validation.set_audience(&[AUDIENCE]);
        validation.validate_exp = true;

        let cache = self.cache.read().expect("jwks cache lock");
        let key = cache
            .keys
            .get(&kid)
            .ok_or_else(|| anyhow!("unknown kid: {kid}"))?;
        let data = decode::<RoomClaims>(token, key, &validation).context("verify Room Token")?;
        if data.claims.room_id != self.expected_room_id {
            return Err(anyhow!(
                "room_id mismatch: token={} expected={}",
                data.claims.room_id,
                self.expected_room_id
            ));
        }
        if data.claims.role != "host" && data.claims.role != "guest" {
            return Err(anyhow!("invalid role: {}", data.claims.role));
        }
        Ok(data.claims)
    }

    fn has_kid(&self, kid: &str) -> bool {
        let cache = self.cache.read().expect("jwks cache lock");
        cache.keys.contains_key(kid)
    }

    fn ensure_fresh_jwks(&self) -> Result<()> {
        let stale = {
            let cache = self.cache.read().expect("jwks cache lock");
            cache.keys.is_empty() || cache.fetched_at.elapsed() > Duration::from_secs(60)
        };
        if stale {
            self.refresh_jwks()?;
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    token: String,
}

/// Ensure room exists and return a host Room Token from world-server.
pub fn fetch_host_token(world_url: &str, room_id: &str) -> Result<String> {
    if let Ok(token) = std::env::var("ALCHEMY_ROOM_TOKEN") {
        if !token.is_empty() {
            return Ok(token);
        }
    }

    let base = world_url.trim_end_matches('/');
    let create_url = format!("{base}/api/v1/rooms");
    let _ = ureq::post(&create_url)
        .set("content-type", "application/json")
        .send_json(serde_json::json!({ "room_id": room_id }));

    let token_url = format!("{base}/api/v1/rooms/{room_id}/tokens");
    let resp: TokenResponse = ureq::post(&token_url)
        .set("content-type", "application/json")
        .send_json(serde_json::json!({ "role": "host", "sub": "host-server" }))
        .context("issue host Room Token")?
        .into_json()
        .context("parse token response")?;
    Ok(resp.token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_rejects_empty_token() {
        let verifier = RoomTokenVerifier::new("http://127.0.0.1:4003", "local");
        assert!(verifier.verify("").is_err());
    }
}
