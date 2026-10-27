//! Room Token verification against world-server JWKS + auth-gated token fetch.
//!
//! Guarantees: RS256 verify + iss/aud/exp/room_id checks; Room Token issued only with auth JWT.
//! x0021: verify never does HTTP on the input path (SWR + background refresh).
//! Non-guarantees: auth UI, refresh-token rotation, session MAC (P2).

use anyhow::{anyhow, Context, Result};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

const ISSUER: &str = "alchemy-world";
const AUDIENCE: &str = "alchemy-room";
const DEFAULT_AUTH_URL: &str = "http://127.0.0.1:4002";
/// Soft TTL: after this, verify still uses cache and kicks background refresh (SWR).
const JWKS_SOFT_TTL: Duration = Duration::from_secs(60);
const JWKS_HTTP_TIMEOUT: Duration = Duration::from_secs(2);
const JWKS_BG_POLL: Duration = Duration::from_secs(30);
const UNKNOWN_KID_COOLDOWN: Duration = Duration::from_secs(1);

/// Debug-only seed credentials (x001e: absent from release binaries).
#[cfg(debug_assertions)]
const DEFAULT_AUTH_USER: &str = "alice";
#[cfg(debug_assertions)]
const DEFAULT_AUTH_PASSWORD: &str = "Password1";

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
    /// Auth user id (UUID). Host routes Intent/Grab per `sub` (x0014).
    pub sub: String,
    pub room_id: String,
    pub role: String,
    /// Room Token id — binds Zenoh liveliness Delete to a verified session (x0022).
    pub jti: String,
    /// Present for JWT exp validation / serde; not read by host logic.
    #[allow(dead_code)]
    pub exp: i64,
}

pub struct RoomTokenVerifier {
    jwks_url: String,
    expected_room_id: String,
    cache: RwLock<KeyCache>,
    refresh_tx: Mutex<Option<SyncSender<()>>>,
    bg_started: AtomicBool,
}

struct KeyCache {
    keys: HashMap<String, DecodingKey>,
    fetched_at: Instant,
    last_unknown_kid_kick: Option<Instant>,
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
                last_unknown_kid_kick: None,
            }),
            refresh_tx: Mutex::new(None),
            bg_started: AtomicBool::new(false),
        }
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
        Ok(())
    }

    /// Verify using cached keys only (no HTTP). Soft-stale cache still accepts (SWR).
    pub fn verify(&self, token: &str) -> Result<RoomClaims> {
        if token.is_empty() {
            return Err(anyhow!("empty room token"));
        }

        let header = decode_header(token).context("decode JWT header")?;
        let kid = header
            .kid
            .ok_or_else(|| anyhow!("JWT header missing kid"))?;

        {
            let cache = self.cache.read().expect("jwks cache lock");
            if cache.keys.is_empty() {
                return Err(anyhow!("JWKS cache empty; call refresh_jwks before verify"));
            }
            if cache.fetched_at.elapsed() > JWKS_SOFT_TTL {
                drop(cache);
                self.kick_refresh();
            }
        }

        if !self.has_kid(&kid) {
            self.kick_refresh_unknown_kid();
            return Err(anyhow!("unknown kid: {kid}"));
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

    /// Test-only: inject a DecodingKey so `verify` works without HTTP JWKS.
    #[cfg(test)]
    pub fn insert_decoding_key_for_test(&self, kid: &str, key: DecodingKey) {
        let mut cache = self.cache.write().expect("jwks cache lock");
        cache.keys.insert(kid.to_string(), key);
        cache.fetched_at = Instant::now();
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    token: String,
}

#[derive(Debug, Deserialize)]
struct AuthSession {
    access_token: String,
}

/// Login to auth-server (or use `ALCHEMY_AUTH_TOKEN`).
///
/// Release (x001e): requires `ALCHEMY_AUTH_TOKEN` or both `AUTH_USER` + `AUTH_PASSWORD`.
/// Debug: may fall back to seed `alice` / `Password1` when env is unset.
pub fn fetch_auth_token(auth_url: &str) -> Result<String> {
    if let Ok(token) = std::env::var("ALCHEMY_AUTH_TOKEN") {
        if !token.is_empty() {
            return Ok(token);
        }
    }

    let (user, password) = auth_login_credentials()?;
    let base = auth_url.trim_end_matches('/');
    let login_url = format!("{base}/api/v1/auth/login");

    let resp: AuthSession = ureq::post(&login_url)
        .set("content-type", "application/json")
        .send_json(serde_json::json!({
            "identifier": user,
            "password": password
        }))
        .context("auth login failed (is auth-server up?)")?
        .into_json()
        .context("parse auth session")?;
    Ok(resp.access_token)
}

fn auth_login_credentials() -> Result<(String, String)> {
    resolve_auth_login_credentials(
        std::env::var("AUTH_USER").ok().filter(|s| !s.is_empty()),
        std::env::var("AUTH_PASSWORD")
            .ok()
            .filter(|s| !s.is_empty()),
        cfg!(debug_assertions),
    )
}

/// Resolve login credentials.
///
/// - `allow_seed=false`: missing user/password → error (release / fail-closed path).
/// - `allow_seed=true`: debug-only seed fallback; release builds always error if seed is requested.
pub(crate) fn resolve_auth_login_credentials(
    user: Option<String>,
    password: Option<String>,
    allow_seed: bool,
) -> Result<(String, String)> {
    if allow_seed {
        return resolve_with_optional_seed(user, password);
    }
    let user = user.ok_or_else(|| {
        anyhow!(
            "AUTH_USER is required in release (or set ALCHEMY_AUTH_TOKEN); seed defaults are debug-only"
        )
    })?;
    let password = password.ok_or_else(|| {
        anyhow!(
            "AUTH_PASSWORD is required in release (or set ALCHEMY_AUTH_TOKEN); seed defaults are debug-only"
        )
    })?;
    Ok((user, password))
}

#[cfg(debug_assertions)]
fn resolve_with_optional_seed(
    user: Option<String>,
    password: Option<String>,
) -> Result<(String, String)> {
    Ok((
        user.unwrap_or_else(|| DEFAULT_AUTH_USER.to_string()),
        password.unwrap_or_else(|| DEFAULT_AUTH_PASSWORD.to_string()),
    ))
}

#[cfg(not(debug_assertions))]
fn resolve_with_optional_seed(
    _user: Option<String>,
    _password: Option<String>,
) -> Result<(String, String)> {
    Err(anyhow!(
        "seed credentials are debug-only; set AUTH_USER and AUTH_PASSWORD (or ALCHEMY_AUTH_TOKEN)"
    ))
}

/// Create room as owner (if needed) and return a host Room Token (requires auth Bearer).
///
/// x0021: only `POST /rooms` assigns `owner_sub`. Guest clients must not create.
pub fn fetch_host_token(world_url: &str, room_id: &str, auth_token: &str) -> Result<String> {
    if let Ok(token) = std::env::var("ALCHEMY_ROOM_TOKEN") {
        if !token.is_empty() {
            return Ok(token);
        }
    }

    let base = world_url.trim_end_matches('/');
    let authz = format!("Bearer {auth_token}");
    let create_url = format!("{base}/api/v1/rooms");
    match ureq::post(&create_url)
        .set("content-type", "application/json")
        .set("authorization", &authz)
        .send_json(serde_json::json!({ "room_id": room_id }))
    {
        Ok(_) => log::info!("created room={room_id} as owner"),
        Err(ureq::Error::Status(409, _)) => {
            log::info!("room={room_id} already exists; requesting host Token as owner");
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            return Err(anyhow!(
                "create room failed status={code} body={body}; host must own the room"
            ));
        }
        Err(err) => {
            return Err(anyhow!("create room request failed: {err:#}"));
        }
    }

    let token_url = format!("{base}/api/v1/rooms/{room_id}/tokens");
    match ureq::post(&token_url)
        .set("content-type", "application/json")
        .set("authorization", &authz)
        .send_json(serde_json::json!({ "role": "host" }))
    {
        Ok(resp) => {
            let parsed: TokenResponse = resp.into_json().context("parse token response")?;
            Ok(parsed.token)
        }
        Err(ureq::Error::Status(403, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(anyhow!(
                "host Room Token forbidden (not room owner — another sub may have created the room first): {body}"
            ))
        }
        Err(ureq::Error::Status(404, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(anyhow!(
                "host Room Token: room not found after create ({body})"
            ))
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(anyhow!(
                "issue host Room Token failed status={code} body={body}"
            ))
        }
        Err(err) => Err(anyhow!("issue host Room Token request failed: {err:#}")),
    }
}

pub fn default_auth_url() -> String {
    std::env::var("AUTH_URL").unwrap_or_else(|_| DEFAULT_AUTH_URL.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_rejects_empty_token() {
        let verifier = RoomTokenVerifier::new("http://127.0.0.1:4003", "local");
        assert!(verifier.verify("").is_err());
    }

    #[test]
    fn verify_does_not_http_when_cache_empty() {
        let verifier = RoomTokenVerifier::new("http://127.0.0.1:9", "local");
        let err = verifier
            .verify("eyJhbGciOiJSUzI1NiIsImtpZCI6InQiLCJ0eXAiOiJKV1QifQ.e30.e30")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("JWKS cache empty"),
            "verify must not reach network; got: {err}"
        );
    }

    #[test]
    fn release_credentials_fail_closed_without_env() {
        let err = resolve_auth_login_credentials(None, None, false).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("AUTH_USER"));
        assert!(!msg.contains("Password1"));
        assert!(!msg.contains("alice"));
    }

    #[test]
    fn release_credentials_require_password_when_user_set() {
        let err = resolve_auth_login_credentials(Some("bob".into()), None, false).unwrap_err();
        assert!(err.to_string().contains("AUTH_PASSWORD"));
        assert!(!err.to_string().contains("Password1"));
    }

    #[test]
    #[cfg(debug_assertions)]
    fn debug_seed_allowed_when_flag_set() {
        let (user, password) =
            resolve_auth_login_credentials(None, None, true).expect("seed ok in debug");
        assert_eq!(user, "alice");
        assert_eq!(password, "Password1");
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn release_build_rejects_allow_seed_flag() {
        let err = resolve_auth_login_credentials(None, None, true).unwrap_err();
        assert!(!err.to_string().contains("Password1"));
        assert!(!err.to_string().contains("alice"));
    }
}
