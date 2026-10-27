//! Room Token verification against world-server JWKS + auth-gated token fetch.
//!
//! Guarantees: RS256 verify + iss/aud/exp/room_id checks; Room Token issued only with auth JWT.
//! x0021: verify never does HTTP on the input path (SWR + background refresh).
//! Long-running host: `HostCredentialChain` (auth → Room Token → HostAttestation) plus
//! `ManifestSealCache` (body blake3 → world seal); refresh failure is fail-closed (x0025 / x0026).
//! Credential HTTP uses a shared connect/read timeout; when multiple layers are due on the same
//! physics tick, requests run serially (bounded by that timeout each).
//! Non-guarantees: auth UI, session MAC (P2); async / background credential refresh.

use anyhow::{anyhow, Context, Result};
use base64::Engine;
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
/// Auth / Room Token / HostAttestation HTTP (physics loop must not hang forever).
const CREDENTIAL_HTTP_TIMEOUT: Duration = Duration::from_secs(5);
const JWKS_BG_POLL: Duration = Duration::from_secs(30);
const UNKNOWN_KID_COOLDOWN: Duration = Duration::from_secs(1);

fn credential_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(CREDENTIAL_HTTP_TIMEOUT)
        .timeout_read(CREDENTIAL_HTTP_TIMEOUT)
        .build()
}

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
    #[serde(default)]
    expires_in: u64,
}

/// Auth session for long-running host (x0025 three-tier refresh).
#[derive(Debug, Clone)]
pub struct HostAuthSession {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// 0 = no scheduled auth refresh (`ALCHEMY_AUTH_TOKEN` env). That path assumes a
    /// **long-lived** access JWT; a short-lived static token will fail when host Token
    /// is re-issued and is **not** auto-refreshed.
    pub expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct AuthSessionJson {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: u64,
}

/// Soft refresh fraction of token TTL (match client x0021).
pub const TOKEN_REFRESH_FRACTION: f64 = 0.8;

/// Hint TTL when Room Token bypass omits `expires_in` (debug).
const BYPASS_ROOM_TOKEN_TTL_HINT: u64 = 300;

/// Instant at which a token with `expires_in` should be refreshed, or `None` if TTL is 0.
pub fn refresh_deadline(expires_in: u64) -> Option<Instant> {
    if expires_in == 0 {
        return None;
    }
    let secs = ((expires_in as f64) * TOKEN_REFRESH_FRACTION).floor() as u64;
    Some(Instant::now() + Duration::from_secs(secs.max(1)))
}

/// Login to auth-server (or use `ALCHEMY_AUTH_TOKEN`).
///
/// Release (x001e): requires `ALCHEMY_AUTH_TOKEN` or both `AUTH_USER` + `AUTH_PASSWORD`.
/// Debug: may fall back to seed `alice` / `Password1` when env is unset.
///
/// `ALCHEMY_AUTH_TOKEN`: treated as long-lived (`expires_in=0`, no timed auth refresh).
/// Prefer `AUTH_USER`/`AUTH_PASSWORD` (+ `remember_me` refresh) for production hosts.
pub fn fetch_auth_session(auth_url: &str) -> Result<HostAuthSession> {
    if let Ok(token) = std::env::var("ALCHEMY_AUTH_TOKEN") {
        if !token.is_empty() {
            return Ok(HostAuthSession {
                access_token: token,
                refresh_token: None,
                expires_in: 0,
            });
        }
    }

    let (user, password) = auth_login_credentials()?;
    let base = auth_url.trim_end_matches('/');
    let login_url = format!("{base}/api/v1/auth/login");

    let resp: AuthSessionJson = credential_agent()
        .post(&login_url)
        .set("content-type", "application/json")
        // remember_me: long-running host needs refresh_token for three-tier refresh (x0025).
        .send_json(serde_json::json!({
            "identifier": user,
            "password": password,
            "remember_me": true
        }))
        .context("auth login failed (is auth-server up?)")?
        .into_json()
        .context("parse auth session")?;
    if resp.access_token.is_empty() {
        return Err(anyhow!("auth login missing access_token"));
    }
    Ok(HostAuthSession {
        access_token: resp.access_token,
        refresh_token: resp.refresh_token.filter(|s| !s.is_empty()),
        expires_in: resp.expires_in,
    })
}

/// Refresh auth access token (rotating refresh_token when returned).
pub fn refresh_auth_session(auth_url: &str, refresh_token: &str) -> Result<HostAuthSession> {
    let base = auth_url.trim_end_matches('/');
    let url = format!("{base}/api/v1/auth/refresh");
    let resp: AuthSessionJson = credential_agent()
        .post(&url)
        .set("content-type", "application/json")
        .send_json(serde_json::json!({ "refresh_token": refresh_token }))
        .context("auth refresh failed")?
        .into_json()
        .context("parse auth refresh session")?;
    if resp.access_token.is_empty() {
        return Err(anyhow!("auth refresh missing access_token"));
    }
    Ok(HostAuthSession {
        access_token: resp.access_token,
        refresh_token: resp
            .refresh_token
            .filter(|s| !s.is_empty())
            .or_else(|| Some(refresh_token.to_string())),
        expires_in: resp.expires_in,
    })
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

/// Create room as owner (if needed) and return host Room Token + TTL (requires auth Bearer).
///
/// x0021: only `POST /rooms` assigns `owner_sub`. Guest clients must not create.
/// x0025: `ALCHEMY_ROOM_TOKEN` bypass is **debug-only** (release fail-closed).
pub fn fetch_host_token(world_url: &str, room_id: &str, auth_token: &str) -> Result<(String, u64)> {
    if let Some(token) = room_token_env_bypass() {
        return Ok((token, BYPASS_ROOM_TOKEN_TTL_HINT));
    }

    let base = world_url.trim_end_matches('/');
    let authz = format!("Bearer {auth_token}");
    let agent = credential_agent();
    let create_url = format!("{base}/api/v1/rooms");
    match agent
        .post(&create_url)
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
    match agent
        .post(&token_url)
        .set("content-type", "application/json")
        .set("authorization", &authz)
        .send_json(serde_json::json!({ "role": "host" }))
    {
        Ok(resp) => {
            let parsed: TokenResponse = resp.into_json().context("parse token response")?;
            let ttl = if parsed.expires_in == 0 {
                BYPASS_ROOM_TOKEN_TTL_HINT
            } else {
                parsed.expires_in
            };
            Ok((parsed.token, ttl))
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

/// Which credential layers to refresh at `now` (pure; unit-tested).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CredentialRefreshPlan {
    pub refresh_auth: bool,
    /// Auth due implies host re-issue (new Bearer for tokens API).
    pub refresh_host: bool,
    /// Attestation only when its own soft deadline is due (not merely because host/auth refreshed).
    pub refresh_attest: bool,
}

/// Decide which layers of the credential chain are due.
pub fn plan_credential_refresh(
    now: Instant,
    auth_refresh_at: Option<Instant>,
    host_refresh_at: Instant,
    attest_refresh_at: Instant,
) -> Option<CredentialRefreshPlan> {
    let need_auth = auth_refresh_at.is_some_and(|t| now >= t);
    let need_host = now >= host_refresh_at;
    let need_attest = now >= attest_refresh_at;
    if !need_auth && !need_host && !need_attest {
        return None;
    }
    Some(CredentialRefreshPlan {
        refresh_auth: need_auth,
        refresh_host: need_auth || need_host,
        refresh_attest: need_attest,
    })
}

/// Long-running host credentials: auth JWT → host Room Token → HostAttestation (x0025).
pub struct HostCredentialChain {
    pub auth: HostAuthSession,
    pub auth_refresh_at: Option<Instant>,
    pub host_token: String,
    pub host_refresh_at: Instant,
    pub host_attestation: String,
    pub attest_refresh_at: Instant,
}

impl HostCredentialChain {
    pub fn bootstrap(
        auth_url: &str,
        world_url: &str,
        room_id: &str,
        host_pub: &[u8],
        verifier: &RoomTokenVerifier,
    ) -> Result<Self> {
        let auth = fetch_auth_session(auth_url).context("fetch auth session")?;
        let auth_refresh_at = refresh_deadline(auth.expires_in);
        let (host_token, host_ttl) = fetch_host_token(world_url, room_id, &auth.access_token)
            .context("fetch host Room Token")?;
        let claims = verifier
            .verify(&host_token)
            .context("host Room Token self-check")?;
        if claims.role != "host" {
            return Err(anyhow!(
                "host Room Token must have role=host (got {})",
                claims.role
            ));
        }
        let (host_attestation, attest_ttl) =
            fetch_host_attestation(world_url, room_id, &host_token, host_pub)
                .context("fetch HostAttestation")?;
        Ok(Self {
            auth,
            auth_refresh_at,
            host_token,
            host_refresh_at: refresh_deadline(host_ttl).unwrap_or_else(|| {
                Instant::now() + Duration::from_secs(BYPASS_ROOM_TOKEN_TTL_HINT)
            }),
            host_attestation,
            attest_refresh_at: refresh_deadline(attest_ttl).unwrap_or_else(|| {
                Instant::now() + Duration::from_secs(BYPASS_ROOM_TOKEN_TTL_HINT)
            }),
        })
    }

    /// Refresh auth / Room Token / attestation as needed.
    /// Returns `true` when `host_attestation` rotated (caller should republish Manifest).
    /// Fail-closed: any error → caller must stop publishing.
    pub fn refresh_if_due(
        &mut self,
        auth_url: &str,
        world_url: &str,
        room_id: &str,
        host_pub: &[u8],
        verifier: &RoomTokenVerifier,
    ) -> Result<bool> {
        let Some(plan) = plan_credential_refresh(
            Instant::now(),
            self.auth_refresh_at,
            self.host_refresh_at,
            self.attest_refresh_at,
        ) else {
            return Ok(false);
        };

        if plan.refresh_auth {
            self.refresh_auth(auth_url)?;
        }
        if plan.refresh_host {
            self.refresh_host_token(world_url, room_id, verifier)?;
        }
        if plan.refresh_attest {
            self.refresh_attestation(world_url, room_id, host_pub)?;
            return Ok(true);
        }
        Ok(false)
    }

    fn refresh_auth(&mut self, auth_url: &str) -> Result<()> {
        if let Some(rt) = self.auth.refresh_token.clone() {
            match refresh_auth_session(auth_url, &rt) {
                Ok(sess) => {
                    self.auth_refresh_at = refresh_deadline(sess.expires_in);
                    self.auth = sess;
                    log::info!("refreshed auth JWT via refresh_token");
                    return Ok(());
                }
                Err(err) => log::warn!("auth refresh_token failed ({err:#}); re-login"),
            }
        }
        let sess = fetch_auth_session(auth_url).context("re-login auth session")?;
        self.auth_refresh_at = refresh_deadline(sess.expires_in);
        self.auth = sess;
        log::info!("refreshed auth JWT via login / ALCHEMY_AUTH_TOKEN");
        Ok(())
    }

    fn refresh_host_token(
        &mut self,
        world_url: &str,
        room_id: &str,
        verifier: &RoomTokenVerifier,
    ) -> Result<()> {
        let (token, ttl) = fetch_host_token(world_url, room_id, &self.auth.access_token)
            .context("refresh host Room Token")?;
        let claims = verifier
            .verify(&token)
            .context("refreshed host Room Token verify")?;
        if claims.role != "host" {
            return Err(anyhow!(
                "refreshed host Room Token must have role=host (got {})",
                claims.role
            ));
        }
        self.host_token = token;
        self.host_refresh_at = refresh_deadline(ttl)
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(BYPASS_ROOM_TOKEN_TTL_HINT));
        log::info!("refreshed host Room Token (ttl={ttl}s)");
        Ok(())
    }

    fn refresh_attestation(
        &mut self,
        world_url: &str,
        room_id: &str,
        host_pub: &[u8],
    ) -> Result<()> {
        let (attestation, ttl) =
            fetch_host_attestation(world_url, room_id, &self.host_token, host_pub)
                .context("refresh HostAttestation")?;
        self.host_attestation = attestation;
        self.attest_refresh_at = refresh_deadline(ttl)
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(BYPASS_ROOM_TOKEN_TTL_HINT));
        log::info!("refreshed HostAttestation (ttl={ttl}s)");
        Ok(())
    }
}

/// World-authored scene definition for a room (x0027).
/// Built from JSON DTOs — prost types are not serde-deserializable.
#[derive(Debug, Clone)]
pub struct RoomScene {
    pub revision: u64,
    pub entries: Vec<alchemy_protocol::SceneEntry>,
    pub locomotion: Option<alchemy_protocol::LocomotionConfig>,
}

#[derive(Debug, Deserialize)]
struct SceneEntryJson {
    entity_id: u32,
    #[serde(default)]
    blake3: String,
    #[serde(default)]
    x: f64,
    #[serde(default)]
    y: f64,
    #[serde(default)]
    z: f64,
    #[serde(default)]
    half_extent: f32,
    #[serde(default)]
    flags: u32,
    #[serde(default)]
    r: f32,
    #[serde(default)]
    g: f32,
    #[serde(default)]
    b: f32,
    #[serde(default)]
    a: f32,
    #[serde(default)]
    qx: f64,
    #[serde(default)]
    qy: f64,
    #[serde(default)]
    qz: f64,
    #[serde(default)]
    qw: f64,
    #[serde(default)]
    sx: f64,
    #[serde(default)]
    sy: f64,
    #[serde(default)]
    sz: f64,
}

#[derive(Debug, Deserialize)]
struct LocomotionJson {
    #[serde(default)]
    mode: String,
    #[serde(default)]
    move_speed: f32,
}

#[derive(Debug, Deserialize)]
struct SceneResponseJson {
    revision: u64,
    entries: Vec<SceneEntryJson>,
    locomotion: Option<LocomotionJson>,
}

fn loco_mode_from_str(mode: &str) -> alchemy_protocol::LocomotionMode {
    match mode.to_ascii_lowercase().as_str() {
        "fly" => alchemy_protocol::LocomotionMode::Fly,
        "walk" => alchemy_protocol::LocomotionMode::Walk,
        "noclip" | "" => alchemy_protocol::LocomotionMode::Noclip,
        other => {
            log::warn!("unknown locomotion mode {other:?}; falling back to Noclip");
            alchemy_protocol::LocomotionMode::Noclip
        }
    }
}

/// Fetch world DemoScene for this room (host Room Token).
pub fn fetch_room_scene(
    world_url: &str,
    room_id: &str,
    host_room_token: &str,
) -> Result<RoomScene> {
    let base = world_url.trim_end_matches('/');
    let url = format!("{base}/api/v1/rooms/{room_id}/scene");
    let authz = format!("Bearer {host_room_token}");
    match credential_agent()
        .get(&url)
        .set("authorization", &authz)
        .call()
    {
        Ok(resp) => {
            let parsed: SceneResponseJson = resp.into_json().context("parse scene response")?;
            let entries = parsed
                .entries
                .into_iter()
                .map(|e| {
                    let mut entry = alchemy_protocol::SceneEntry {
                        entity_id: e.entity_id,
                        blake3: e.blake3,
                        x: e.x,
                        y: e.y,
                        z: e.z,
                        half_extent: e.half_extent,
                        flags: e.flags,
                        r: e.r,
                        g: e.g,
                        b: e.b,
                        a: e.a,
                        qx: e.qx,
                        qy: e.qy,
                        qz: e.qz,
                        qw: e.qw,
                        sx: e.sx,
                        sy: e.sy,
                        sz: e.sz,
                    };
                    alchemy_protocol::normalize_scene_entry_trs(&mut entry);
                    entry
                })
                .collect();
            let locomotion = parsed
                .locomotion
                .map(|l| alchemy_protocol::LocomotionConfig {
                    mode: loco_mode_from_str(&l.mode).into(),
                    move_speed: l.move_speed,
                });
            Ok(RoomScene {
                revision: parsed.revision,
                entries,
                locomotion,
            })
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(anyhow!("scene fetch failed status={code} body={body}"))
        }
        Err(err) => Err(anyhow!("scene fetch request failed: {err:#}")),
    }
}

/// Ask world to seal a Manifest body hash; returns ManifestSeal JWT + TTL (x0026).
pub fn fetch_manifest_seal(
    world_url: &str,
    room_id: &str,
    host_room_token: &str,
    revision: u64,
    manifest_blake3_hex: &str,
) -> Result<(String, u64)> {
    if !is_blake3_hex(manifest_blake3_hex) {
        return Err(anyhow!(
            "manifest_blake3 must be 64 lowercase/uppercase hex chars (got len={})",
            manifest_blake3_hex.len()
        ));
    }
    let base = world_url.trim_end_matches('/');
    let url = format!("{base}/api/v1/rooms/{room_id}/manifest_seal");
    let authz = format!("Bearer {host_room_token}");
    #[derive(Debug, Deserialize)]
    struct SealResponse {
        seal: String,
        expires_in: u64,
    }
    match credential_agent()
        .post(&url)
        .set("content-type", "application/json")
        .set("authorization", &authz)
        .send_json(serde_json::json!({
            "revision": revision,
            "manifest_blake3": manifest_blake3_hex,
        })) {
        Ok(resp) => {
            let parsed: SealResponse = resp.into_json().context("parse manifest_seal response")?;
            if parsed.seal.is_empty() || parsed.expires_in == 0 {
                return Err(anyhow!("manifest_seal response missing seal/expires_in"));
            }
            Ok((parsed.seal, parsed.expires_in))
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(anyhow!("manifest_seal failed status={code} body={body}"))
        }
        Err(err) => Err(anyhow!("manifest_seal request failed: {err:#}")),
    }
}

/// Cached world ManifestSeal for unchanged body republish (x0026).
pub struct ManifestSealCache {
    pub seal: String,
    pub body_blake3: String,
    pub revision: u64,
    pub refresh_at: Instant,
}

impl ManifestSealCache {
    /// Ensure seal matches `(revision, body_blake3)`; refresh when due or body changed.
    /// Returns `true` when `seal` string changed.
    pub fn ensure(
        &mut self,
        world_url: &str,
        room_id: &str,
        host_room_token: &str,
        revision: u64,
        body_blake3: &str,
    ) -> Result<bool> {
        let body_changed =
            self.body_blake3 != body_blake3 || self.revision != revision || self.seal.is_empty();
        let due = Instant::now() >= self.refresh_at;
        if !body_changed && !due {
            return Ok(false);
        }
        let (seal, ttl) =
            fetch_manifest_seal(world_url, room_id, host_room_token, revision, body_blake3)
                .context("fetch ManifestSeal")?;
        let changed = seal != self.seal;
        self.seal = seal;
        self.body_blake3 = body_blake3.to_string();
        self.revision = revision;
        self.refresh_at = refresh_deadline(ttl)
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(BYPASS_ROOM_TOKEN_TTL_HINT));
        log::info!("refreshed ManifestSeal (ttl={ttl}s revision={revision})");
        Ok(changed)
    }

    pub fn empty() -> Self {
        Self {
            seal: String::new(),
            body_blake3: String::new(),
            revision: 0,
            refresh_at: Instant::now(),
        }
    }
}

fn is_blake3_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Register host Ed25519 pub with world; returns HostAttestation JWT + TTL (x0025).
pub fn fetch_host_attestation(
    world_url: &str,
    room_id: &str,
    host_room_token: &str,
    host_pub: &[u8],
) -> Result<(String, u64)> {
    if host_pub.len() != 32 {
        return Err(anyhow!("host_pub length {} (want 32)", host_pub.len()));
    }
    let host_pub_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(host_pub);
    let base = world_url.trim_end_matches('/');
    let url = format!("{base}/api/v1/rooms/{room_id}/host_keys");
    let authz = format!("Bearer {host_room_token}");
    #[derive(Debug, Deserialize)]
    struct AttestResponse {
        attestation: String,
        expires_in: u64,
    }
    match credential_agent()
        .post(&url)
        .set("content-type", "application/json")
        .set("authorization", &authz)
        .send_json(serde_json::json!({ "host_pub": host_pub_b64 }))
    {
        Ok(resp) => {
            let parsed: AttestResponse = resp.into_json().context("parse host_keys response")?;
            if parsed.attestation.is_empty() || parsed.expires_in == 0 {
                return Err(anyhow!("host_keys response missing attestation/expires_in"));
            }
            Ok((parsed.attestation, parsed.expires_in))
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(anyhow!("host_keys failed status={code} body={body}"))
        }
        Err(err) => Err(anyhow!("host_keys request failed: {err:#}")),
    }
}

/// Debug-only Room Token env bypass (x0025). Release always returns `None`.
fn room_token_env_bypass() -> Option<String> {
    room_token_env_bypass_from(
        std::env::var("ALCHEMY_ROOM_TOKEN").ok(),
        cfg!(debug_assertions),
    )
}

pub(crate) fn room_token_env_bypass_from(
    env_value: Option<String>,
    allow_debug_bypass: bool,
) -> Option<String> {
    if !allow_debug_bypass {
        return None;
    }
    env_value.filter(|t| !t.is_empty())
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

    #[test]
    fn room_token_env_bypass_release_closed() {
        assert!(room_token_env_bypass_from(Some("tok".into()), false).is_none());
        assert_eq!(
            room_token_env_bypass_from(Some("tok".into()), true).as_deref(),
            Some("tok")
        );
        assert!(room_token_env_bypass_from(Some("".into()), true).is_none());
    }

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
    fn plan_refresh_none_when_nothing_due() {
        let now = Instant::now();
        assert!(plan_credential_refresh(
            now,
            Some(now + Duration::from_secs(10)),
            now + Duration::from_secs(10),
            now + Duration::from_secs(10),
        )
        .is_none());
        assert!(plan_credential_refresh(
            now,
            None,
            now + Duration::from_secs(10),
            now + Duration::from_secs(10),
        )
        .is_none());
    }

    #[test]
    fn plan_refresh_auth_due_implies_host_not_attest() {
        let now = Instant::now();
        let plan = plan_credential_refresh(
            now,
            Some(now - Duration::from_secs(1)),
            now + Duration::from_secs(60),
            now + Duration::from_secs(60),
        )
        .expect("due");
        assert_eq!(
            plan,
            CredentialRefreshPlan {
                refresh_auth: true,
                refresh_host: true,
                refresh_attest: false,
            }
        );
    }

    #[test]
    fn plan_refresh_host_only_skips_attest() {
        let now = Instant::now();
        let plan = plan_credential_refresh(
            now,
            Some(now + Duration::from_secs(60)),
            now - Duration::from_secs(1),
            now + Duration::from_secs(60),
        )
        .expect("due");
        assert_eq!(
            plan,
            CredentialRefreshPlan {
                refresh_auth: false,
                refresh_host: true,
                refresh_attest: false,
            }
        );
    }

    #[test]
    fn plan_refresh_attest_only() {
        let now = Instant::now();
        let plan = plan_credential_refresh(
            now,
            None,
            now + Duration::from_secs(60),
            now - Duration::from_secs(1),
        )
        .expect("due");
        assert_eq!(
            plan,
            CredentialRefreshPlan {
                refresh_auth: false,
                refresh_host: false,
                refresh_attest: true,
            }
        );
    }

    #[test]
    fn plan_refresh_all_layers_when_all_due() {
        let now = Instant::now();
        let plan = plan_credential_refresh(now, Some(now), now, now).expect("due");
        assert_eq!(
            plan,
            CredentialRefreshPlan {
                refresh_auth: true,
                refresh_host: true,
                refresh_attest: true,
            }
        );
    }

    #[test]
    fn blake3_hex_accepts_64_hex_rejects_charset() {
        assert!(is_blake3_hex(&"ab".repeat(32)));
        assert!(is_blake3_hex(&"AB".repeat(32)));
        assert!(!is_blake3_hex("short"));
        assert!(!is_blake3_hex(&"zz".repeat(32)));
        assert!(!is_blake3_hex(&format!("{}!", "a".repeat(63))));
    }
}
