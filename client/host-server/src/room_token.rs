//! Room Token verification against world-server JWKS + auth-gated token fetch.
//!
//! Guarantees: RS256 verify + iss/aud/exp/room_id checks; Room Token issued only with auth JWT.
//! x0021: verify never does HTTP on the input path (SWR + background refresh).
//! x0040: JWKS soft TTL (SWR) + **hard stale TTL** — beyond hard, bind verify fails closed
//! until background refresh succeeds (revoked world keys cannot be used forever).
//! Long-running host: `HostCredentialChain` (auth → Room Token → HostAttestation) plus
//! `ManifestSealCache` (body blake3 → world seal).
//! x0039: seal request includes `host_attestation`; world rejects non-canonical blake3.
//! x003d: credential / seal HTTP runs on a dedicated worker; physics reads
//! [`HostPublishSnapshot`] only. Soft-deadline refresh failure retries with backoff and
//! keeps the last good snapshot. **Fail-closed** means stop publishing after hard expiry
//! (full TTL), not kill the process on a transient 502.
//! Non-guarantees: auth UI, session MAC (P2); Zenoh put path is x0042 network worker;
//! client `world_jwks` hard TTL (P2-3).

use alchemy_protocol::wire_crypto::manifest_seal_blake3_hex;
use alchemy_protocol::{
    LocomotionConfig, LocomotionMode, Message, SceneManifest, DEFAULT_MOVE_SPEED, PROTOCOL_PIN,
};
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
/// Hard stale: beyond this, Room Token verify fails until refresh (x0040 / P1-5).
const JWKS_HARD_TTL: Duration = Duration::from_secs(300);
const JWKS_HTTP_TIMEOUT: Duration = Duration::from_secs(2);
/// Auth / Room Token / HostAttestation / ManifestSeal HTTP (credential worker only; x003d).
const CREDENTIAL_HTTP_TIMEOUT: Duration = Duration::from_secs(5);
const JWKS_BG_POLL: Duration = Duration::from_secs(30);
const UNKNOWN_KID_COOLDOWN: Duration = Duration::from_secs(1);
/// Credential worker idle poll when nothing is soft-due.
const CREDENTIAL_WORKER_IDLE: Duration = Duration::from_millis(200);
const CREDENTIAL_BACKOFF_START: Duration = Duration::from_secs(1);
const CREDENTIAL_BACKOFF_MAX: Duration = Duration::from_secs(30);

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

/// Confirmation claim (RFC 7800 `cnf`) — Alchemy binds Intent session_pub (x003e).
#[derive(Debug, Clone, Deserialize)]
pub struct RoomCnf {
    /// base64url (no pad) of 32-byte Ed25519 session public key.
    pub spk: String,
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
    /// Intent session key binding (x003e). Required on bind; HTTP credential paths ignore.
    pub cnf: RoomCnf,
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
    last_hard_stale_kick: Option<Instant>,
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
                last_hard_stale_kick: None,
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

    /// Verify using cached keys only (no HTTP). Soft-stale cache still accepts (SWR);
    /// hard-stale cache rejects until background refresh (x0040).
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
            let age = cache.fetched_at.elapsed();
            if age > JWKS_HARD_TTL {
                drop(cache);
                self.kick_refresh_hard_stale();
                return Err(anyhow!(
                    "JWKS cache hard-stale (age > {}s); refresh required (x0040)",
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
        // x003e: reject malformed cnf.spk at verify (HTTP credential paths ignore the bytes).
        crate::session::decode_cnf_spk(&data.claims.cnf.spk)
            .context("Room Token cnf.spk invalid")?;
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

    /// Test-only: inject a DecodingKey so `verify` works without HTTP JWKS.
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

fn hard_expire_or_hint(expires_in: u64, hint_secs: u64) -> Instant {
    hard_expire_deadline(expires_in)
        .unwrap_or_else(|| Instant::now() + Duration::from_secs(hint_secs))
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

/// Keep in sync with `client` `access_level::AccessLevel` and world `World.Visibility`.
const ROOM_VISIBILITY_ALLOWED: &[&str] = &[
    "lan",
    "private",
    "friends",
    "friends_plus",
    "followers",
    "followers_plus",
    "public",
];

/// Allowlist parse; unknown → `None` (fail-closed at env boundary).
fn parse_room_visibility(raw: &str) -> Option<&'static str> {
    let v = raw.trim();
    ROOM_VISIBILITY_ALLOWED
        .iter()
        .copied()
        .find(|allowed| *allowed == v)
}

/// `ROOM_VISIBILITY` env → allowlisted wire value.
/// Unset → `public`. Set but invalid → error (avoid silently widening ACL).
fn room_visibility_from_env() -> Result<String> {
    match std::env::var("ROOM_VISIBILITY") {
        Ok(raw) => match parse_room_visibility(&raw) {
            Some(v) => Ok(v.to_string()),
            None => Err(anyhow!(
                "ROOM_VISIBILITY={raw:?} invalid; expected one of {ROOM_VISIBILITY_ALLOWED:?}"
            )),
        },
        Err(_) => Ok("public".into()),
    }
}

/// Create room as owner (if needed) and return host Room Token + TTL (requires auth Bearer).
///
/// x0021: `POST /rooms` assigns `owner_sub`. Prefer **host creates first** (launcher order).
/// x0037: client create panel may also `POST` / `PUT` when signed in as the **same** account;
/// if a different sub creates first, this host Token path returns 403.
/// x0025: `ALCHEMY_ROOM_TOKEN` bypass is **debug-only** (release fail-closed).
pub fn fetch_host_token(world_url: &str, room_id: &str, auth_token: &str) -> Result<(String, u64)> {
    if let Some(token) = room_token_env_bypass() {
        return Ok((token, BYPASS_ROOM_TOKEN_TTL_HINT));
    }

    let base = world_url.trim_end_matches('/');
    let authz = format!("Bearer {auth_token}");
    let agent = credential_agent();
    let create_url = format!("{base}/api/v1/rooms");
    // Optional initial ACL (x0037). Invalid env fails closed (no silent widen to public).
    let visibility = room_visibility_from_env()?;
    match agent
        .post(&create_url)
        .set("content-type", "application/json")
        .set("authorization", &authz)
        .send_json(serde_json::json!({
            "room_id": room_id,
            "visibility": visibility,
        })) {
        Ok(_) => log::info!("created room={room_id} as owner visibility={visibility}"),
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

    // x003e: API requires session_pub; HTTP credential paths do not check cnf.
    // Ephemeral key binds the JWT claim only (not used for Intent).
    let ephemeral = alchemy_protocol::wire_crypto::generate_signing_key();
    let session_pub_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(alchemy_protocol::wire_crypto::public_key_bytes(&ephemeral));
    let token_url = format!("{base}/api/v1/rooms/{room_id}/tokens");
    match agent
        .post(&token_url)
        .set("content-type", "application/json")
        .set("authorization", &authz)
        .send_json(serde_json::json!({
            "role": "host",
            "session_pub": session_pub_b64,
        })) {
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
    /// Full auth JWT expiry; `None` when `expires_in=0` (static `ALCHEMY_AUTH_TOKEN`).
    pub auth_hard_expire_at: Option<Instant>,
    pub host_token: String,
    pub host_refresh_at: Instant,
    pub host_hard_expire_at: Instant,
    pub host_attestation: String,
    /// World-assigned generation from last host_keys (x003a). Always >= 1 after bootstrap.
    pub host_generation: u64,
    pub attest_refresh_at: Instant,
    pub attest_hard_expire_at: Instant,
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
        let auth_hard_expire_at = hard_expire_deadline(auth.expires_in);
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
        let (host_attestation, host_generation, attest_ttl) =
            fetch_host_attestation(world_url, room_id, &host_token, host_pub)
                .context("fetch HostAttestation")?;
        Ok(Self {
            auth,
            auth_refresh_at,
            auth_hard_expire_at,
            host_token,
            host_refresh_at: refresh_deadline(host_ttl).unwrap_or_else(|| {
                Instant::now() + Duration::from_secs(BYPASS_ROOM_TOKEN_TTL_HINT)
            }),
            host_hard_expire_at: hard_expire_or_hint(host_ttl, BYPASS_ROOM_TOKEN_TTL_HINT),
            host_attestation,
            host_generation,
            attest_refresh_at: refresh_deadline(attest_ttl).unwrap_or_else(|| {
                Instant::now() + Duration::from_secs(BYPASS_ROOM_TOKEN_TTL_HINT)
            }),
            attest_hard_expire_at: hard_expire_or_hint(attest_ttl, BYPASS_ROOM_TOKEN_TTL_HINT),
        })
    }

    /// Earliest hard expiry across auth / host token / attestation.
    pub fn hard_expire_at(&self) -> Option<Instant> {
        earliest_hard_expire(&[
            self.auth_hard_expire_at,
            Some(self.host_hard_expire_at),
            Some(self.attest_hard_expire_at),
        ])
    }

    /// Apply a precomputed refresh plan (x003d: caller owns `plan_credential_refresh` timing).
    /// Returns `true` when `host_attestation` rotated (caller should rebuild sealed Manifest).
    pub fn apply_credential_plan(
        &mut self,
        plan: CredentialRefreshPlan,
        auth_url: &str,
        world_url: &str,
        room_id: &str,
        host_pub: &[u8],
        verifier: &RoomTokenVerifier,
    ) -> Result<bool> {
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
                    self.auth_hard_expire_at = hard_expire_deadline(sess.expires_in);
                    self.auth = sess;
                    log::info!("refreshed auth JWT via refresh_token");
                    return Ok(());
                }
                Err(err) => log::warn!("auth refresh_token failed ({err:#}); re-login"),
            }
        }
        let sess = fetch_auth_session(auth_url).context("re-login auth session")?;
        self.auth_refresh_at = refresh_deadline(sess.expires_in);
        self.auth_hard_expire_at = hard_expire_deadline(sess.expires_in);
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
        self.host_hard_expire_at = hard_expire_or_hint(ttl, BYPASS_ROOM_TOKEN_TTL_HINT);
        log::info!("refreshed host Room Token (ttl={ttl}s)");
        Ok(())
    }

    fn refresh_attestation(
        &mut self,
        world_url: &str,
        room_id: &str,
        host_pub: &[u8],
    ) -> Result<()> {
        let (attestation, generation, ttl) =
            fetch_host_attestation(world_url, room_id, &self.host_token, host_pub)
                .context("refresh HostAttestation")?;
        self.host_attestation = attestation;
        self.host_generation = generation;
        self.attest_refresh_at = refresh_deadline(ttl)
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(BYPASS_ROOM_TOKEN_TTL_HINT));
        self.attest_hard_expire_at = hard_expire_or_hint(ttl, BYPASS_ROOM_TOKEN_TTL_HINT);
        log::info!("refreshed HostAttestation (ttl={ttl}s generation={generation})");
        Ok(())
    }
}

/// Physics-facing credential / sealed-Manifest surface (x003d).
#[derive(Debug, Clone)]
pub struct HostPublishSnapshot {
    pub publish_allowed: bool,
    pub hard_expire_at: Option<Instant>,
    pub host_generation: u64,
    pub manifest_bytes: Vec<u8>,
    pub manifest_revision: u64,
    /// Bumped when the worker rebuilds a sealed Manifest (attest / seal rotate).
    pub manifest_epoch: u64,
}

impl HostPublishSnapshot {
    pub fn from_sealed(
        manifest: &SceneManifest,
        hard_expire_at: Option<Instant>,
        manifest_epoch: u64,
    ) -> Self {
        Self {
            publish_allowed: true,
            hard_expire_at,
            host_generation: manifest.host_generation,
            manifest_bytes: manifest.encode_to_vec(),
            manifest_revision: manifest.revision,
            manifest_epoch,
        }
    }
}

/// Inputs owned by the credential worker after bootstrap (x003d).
pub struct CredentialWorkerConfig {
    pub auth_url: String,
    pub world_url: String,
    pub room_id: String,
    pub host_pub: Vec<u8>,
    pub scene: RoomScene,
}

fn combined_hard_expire(
    creds: &HostCredentialChain,
    seal_cache: &ManifestSealCache,
) -> Option<Instant> {
    earliest_hard_expire(&[
        creds.auth_hard_expire_at,
        Some(creds.host_hard_expire_at),
        Some(creds.attest_hard_expire_at),
        if seal_cache.seal.is_empty() {
            None
        } else {
            Some(seal_cache.hard_expire_at)
        },
    ])
}

/// Apply a successful sealed Manifest into the shared snapshot (testable helper).
pub fn write_snapshot_on_success(
    snapshot: &mut HostPublishSnapshot,
    manifest: &SceneManifest,
    hard_expire_at: Option<Instant>,
) {
    let epoch = snapshot.manifest_epoch.wrapping_add(1);
    *snapshot = HostPublishSnapshot::from_sealed(manifest, hard_expire_at, epoch);
}

pub fn build_manifest_from_scene(
    room_id: &str,
    scene: &RoomScene,
    host_pub: Vec<u8>,
    host_attestation: String,
    world_seal: String,
    host_generation: u64,
) -> SceneManifest {
    let loco = scene.locomotion.unwrap_or(LocomotionConfig {
        mode: LocomotionMode::Noclip.into(),
        move_speed: DEFAULT_MOVE_SPEED,
    });
    SceneManifest {
        room_id: room_id.to_string(),
        revision: scene.revision,
        entries: scene.entries.clone(),
        locomotion: Some(loco),
        protocol_pin: PROTOCOL_PIN.to_string(),
        host_pub,
        host_attestation,
        world_seal,
        host_generation,
    }
}

/// Build Manifest body (empty seal), obtain/cache world seal, return publishable Manifest.
pub fn ensure_sealed_manifest(
    room_id: &str,
    scene: &RoomScene,
    host_pub: &[u8],
    creds: &HostCredentialChain,
    world_url: &str,
    seal_cache: &mut ManifestSealCache,
) -> Result<SceneManifest> {
    let mut manifest = build_manifest_from_scene(
        room_id,
        scene,
        host_pub.to_vec(),
        creds.host_attestation.clone(),
        String::new(),
        creds.host_generation,
    );
    let body_hex = manifest_seal_blake3_hex(&manifest);
    seal_cache
        .ensure(
            world_url,
            room_id,
            &creds.host_token,
            manifest.revision,
            &body_hex,
            &creds.host_attestation,
        )
        .context("ensure ManifestSeal")?;
    manifest.world_seal = seal_cache.seal.clone();
    Ok(manifest)
}

/// Mark seal soft-due so the next worker loop retries even if credential soft
/// deadlines already advanced (attest succeeded, seal HTTP failed).
pub fn force_seal_retry(seal_cache: &mut ManifestSealCache, pending_seal: &mut bool) {
    *pending_seal = true;
    seal_cache.refresh_at = Instant::now();
}

/// Spawn background auth → Room Token → attestation → seal refresh (x003d).
pub fn spawn_credential_worker(
    snapshot: Arc<RwLock<HostPublishSnapshot>>,
    mut creds: HostCredentialChain,
    mut seal_cache: ManifestSealCache,
    verifier: Arc<RoomTokenVerifier>,
    cfg: CredentialWorkerConfig,
) {
    thread::Builder::new()
        .name("credential-refresh".into())
        .spawn(move || {
            let mut backoff = CREDENTIAL_BACKOFF_START;
            let mut pending_seal = false;
            loop {
                let now = Instant::now();
                {
                    let Ok(mut snap) = snapshot.write() else {
                        log::error!("credential snapshot RwLock poisoned; stopping worker");
                        return;
                    };
                    if snap.publish_allowed
                        && !publish_allowed_at(now, snap.hard_expire_at, true)
                    {
                        log::error!(
                            "credential hard expiry reached; stopping Manifest/Frame/snapshot publish (host process stays up)"
                        );
                        snap.publish_allowed = false;
                    }
                }

                let plan = plan_credential_refresh(
                    now,
                    creds.auth_refresh_at,
                    creds.host_refresh_at,
                    creds.attest_refresh_at,
                );
                let seal_due = pending_seal || now >= seal_cache.refresh_at;
                if plan.is_none() && !seal_due {
                    thread::sleep(CREDENTIAL_WORKER_IDLE);
                    continue;
                }

                let refresh_result = (|| -> Result<bool> {
                    let mut attest_rotated = false;
                    if let Some(plan) = plan {
                        attest_rotated = creds.apply_credential_plan(
                            plan,
                            &cfg.auth_url,
                            &cfg.world_url,
                            &cfg.room_id,
                            &cfg.host_pub,
                            verifier.as_ref(),
                        )?;
                    }
                    if attest_rotated || seal_due || seal_cache.seal.is_empty() {
                        let manifest = ensure_sealed_manifest(
                            &cfg.room_id,
                            &cfg.scene,
                            &cfg.host_pub,
                            &creds,
                            &cfg.world_url,
                            &mut seal_cache,
                        )
                        .inspect_err(|_| {
                            force_seal_retry(&mut seal_cache, &mut pending_seal);
                        })?;
                        pending_seal = false;
                        let hard = combined_hard_expire(&creds, &seal_cache);
                        let Ok(mut snap) = snapshot.write() else {
                            return Err(anyhow!("credential snapshot RwLock poisoned"));
                        };
                        write_snapshot_on_success(&mut snap, &manifest, hard);
                        if attest_rotated {
                            log::info!(
                                "credential worker: sealed Manifest ready (HostAttestation rotated, epoch={})",
                                snap.manifest_epoch
                            );
                        } else {
                            log::info!(
                                "credential worker: sealed Manifest ready (ManifestSeal refresh, epoch={})",
                                snap.manifest_epoch
                            );
                        }
                    } else {
                        // Auth/host soft refresh without attest: extend hard expiry only.
                        let hard = combined_hard_expire(&creds, &seal_cache);
                        let Ok(mut snap) = snapshot.write() else {
                            return Err(anyhow!("credential snapshot RwLock poisoned"));
                        };
                        snap.hard_expire_at = hard;
                        snap.publish_allowed = true;
                        snap.host_generation = creds.host_generation;
                    }
                    Ok(attest_rotated)
                })();

                match refresh_result {
                    Ok(_) => {
                        backoff = CREDENTIAL_BACKOFF_START;
                        thread::sleep(CREDENTIAL_WORKER_IDLE);
                    }
                    Err(err) => {
                        log::warn!(
                            "credential refresh failed (retry with backoff; keeping last good snapshot): {err:#}"
                        );
                        thread::sleep(backoff);
                        backoff = backoff.saturating_mul(2).min(CREDENTIAL_BACKOFF_MAX);
                    }
                }
            }
        })
        .expect("spawn credential-refresh");
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

/// Ask world to seal a Manifest body hash; returns ManifestSeal JWT + TTL (x0026 / x0039).
///
/// World recomputes the canonical blake3 from DB scene + `host_attestation` and
/// rejects mismatch (`manifest_blake3_mismatch`) — signing oracle abolished.
pub fn fetch_manifest_seal(
    world_url: &str,
    room_id: &str,
    host_room_token: &str,
    revision: u64,
    manifest_blake3_hex: &str,
    host_attestation: &str,
) -> Result<(String, u64)> {
    if !is_blake3_hex(manifest_blake3_hex) {
        return Err(anyhow!(
            "manifest_blake3 must be 64 lowercase/uppercase hex chars (got len={})",
            manifest_blake3_hex.len()
        ));
    }
    if host_attestation.is_empty() {
        return Err(anyhow!(
            "host_attestation required for ManifestSeal (x0039)"
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
            "host_attestation": host_attestation,
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
    pub hard_expire_at: Instant,
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
        host_attestation: &str,
    ) -> Result<bool> {
        let body_changed =
            self.body_blake3 != body_blake3 || self.revision != revision || self.seal.is_empty();
        let due = Instant::now() >= self.refresh_at;
        if !body_changed && !due {
            return Ok(false);
        }
        let (seal, ttl) = fetch_manifest_seal(
            world_url,
            room_id,
            host_room_token,
            revision,
            body_blake3,
            host_attestation,
        )
        .context("fetch ManifestSeal")?;
        let changed = seal != self.seal;
        self.seal = seal;
        self.body_blake3 = body_blake3.to_string();
        self.revision = revision;
        self.refresh_at = refresh_deadline(ttl)
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(BYPASS_ROOM_TOKEN_TTL_HINT));
        self.hard_expire_at = hard_expire_or_hint(ttl, BYPASS_ROOM_TOKEN_TTL_HINT);
        log::info!("refreshed ManifestSeal (ttl={ttl}s revision={revision})");
        Ok(changed)
    }

    pub fn empty() -> Self {
        Self {
            seal: String::new(),
            body_blake3: String::new(),
            revision: 0,
            refresh_at: Instant::now(),
            hard_expire_at: Instant::now(),
        }
    }
}

fn is_blake3_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Register host Ed25519 pub with world; returns HostAttestation JWT + generation + TTL (x0025 / x003a).
pub fn fetch_host_attestation(
    world_url: &str,
    room_id: &str,
    host_room_token: &str,
    host_pub: &[u8],
) -> Result<(String, u64, u64)> {
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
        host_generation: u64,
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
            if parsed.host_generation == 0 {
                return Err(anyhow!("host_keys response missing host_generation (>0)"));
            }
            Ok((
                parsed.attestation,
                parsed.host_generation,
                parsed.expires_in,
            ))
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
    fn parse_room_visibility_allowlist() {
        assert_eq!(parse_room_visibility("friends_plus"), Some("friends_plus"));
        assert_eq!(parse_room_visibility("  lan "), Some("lan"));
        assert_eq!(parse_room_visibility("nope"), None);
        assert_eq!(parse_room_visibility(""), None);
        assert_eq!(parse_room_visibility("public"), Some("public"));
    }

    #[test]
    fn room_visibility_allowlist_matches_client_access_level_set() {
        // Drift guard vs client `access_level::AccessLevel::ALL` / world Visibility.
        // Order here is env allowlist (unsorted membership); set equality matters.
        let mut got: Vec<&str> = ROOM_VISIBILITY_ALLOWED.to_vec();
        got.sort_unstable();
        let mut want = vec![
            "followers",
            "followers_plus",
            "friends",
            "friends_plus",
            "lan",
            "private",
            "public",
        ];
        want.sort_unstable();
        assert_eq!(got, want);
    }

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

    #[test]
    fn write_snapshot_on_success_bumps_epoch_and_enables_publish() {
        let mut snap = HostPublishSnapshot {
            publish_allowed: false,
            hard_expire_at: Some(Instant::now() - Duration::from_secs(1)),
            host_generation: 1,
            manifest_bytes: vec![],
            manifest_revision: 0,
            manifest_epoch: 3,
        };
        let manifest = SceneManifest {
            room_id: "local".into(),
            revision: 7,
            entries: vec![],
            locomotion: None,
            protocol_pin: PROTOCOL_PIN.to_string(),
            host_pub: vec![0; 32],
            host_attestation: "attest".into(),
            world_seal: "seal".into(),
            host_generation: 2,
        };
        let hard = Instant::now() + Duration::from_secs(30);
        write_snapshot_on_success(&mut snap, &manifest, Some(hard));
        assert!(snap.publish_allowed);
        assert_eq!(snap.manifest_epoch, 4);
        assert_eq!(snap.manifest_revision, 7);
        assert_eq!(snap.host_generation, 2);
        assert!(!snap.manifest_bytes.is_empty());
    }

    #[test]
    fn force_seal_retry_marks_pending_and_soft_due() {
        let mut seal = ManifestSealCache::empty();
        seal.refresh_at = Instant::now() + Duration::from_secs(3600);
        let mut pending = false;
        force_seal_retry(&mut seal, &mut pending);
        assert!(pending);
        assert!(seal.refresh_at <= Instant::now());
    }

    #[test]
    fn seal_failure_path_does_not_write_snapshot() {
        // Contract: on ensure_sealed_manifest Err, worker calls force_seal_retry and
        // must not call write_snapshot_on_success (snapshot stays at previous epoch).
        let before = HostPublishSnapshot {
            publish_allowed: true,
            hard_expire_at: Some(Instant::now() + Duration::from_secs(10)),
            host_generation: 9,
            manifest_bytes: vec![1, 2, 3],
            manifest_revision: 1,
            manifest_epoch: 5,
        };
        let mut snap = before.clone();
        let mut pending = false;
        let mut seal = ManifestSealCache::empty();
        seal.refresh_at = Instant::now() + Duration::from_secs(60);
        // Simulate Err branch only:
        force_seal_retry(&mut seal, &mut pending);
        assert_eq!(snap.manifest_epoch, before.manifest_epoch);
        assert_eq!(snap.manifest_bytes, before.manifest_bytes);
        assert!(pending);
        // Success path would bump epoch:
        let manifest = SceneManifest {
            room_id: "local".into(),
            revision: 1,
            entries: vec![],
            locomotion: None,
            protocol_pin: PROTOCOL_PIN.to_string(),
            host_pub: vec![0; 32],
            host_attestation: "a".into(),
            world_seal: "s".into(),
            host_generation: 9,
        };
        write_snapshot_on_success(
            &mut snap,
            &manifest,
            Some(Instant::now() + Duration::from_secs(30)),
        );
        assert_eq!(snap.manifest_epoch, 6);
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

    fn test_rsa_jwt(room_id: &str) -> (RoomTokenVerifier, String) {
        use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
        use pkcs1::EncodeRsaPrivateKey;
        use pkcs8::EncodePublicKey;
        use rsa::RsaPrivateKey;
        use serde::Serialize;
        use std::time::{SystemTime, UNIX_EPOCH};

        let mut rng = rand::thread_rng();
        let private = RsaPrivateKey::new(&mut rng, 2048).expect("rsa");
        let priv_pem = private
            .to_pkcs1_pem(pkcs1::LineEnding::LF)
            .expect("priv pem");
        let pub_pem = private
            .to_public_key()
            .to_public_key_pem(pkcs8::LineEnding::LF)
            .expect("pub pem");
        let encoding = EncodingKey::from_rsa_pem(priv_pem.as_bytes()).expect("enc");
        let decoding = DecodingKey::from_rsa_pem(pub_pem.as_bytes()).expect("dec");
        let verifier = RoomTokenVerifier::new("http://127.0.0.1:9", room_id);
        verifier.insert_decoding_key_for_test("hard-kid", decoding);

        #[derive(Serialize)]
        struct Cnf {
            spk: String,
        }
        #[derive(Serialize)]
        struct Claims {
            sub: String,
            jti: String,
            room_id: String,
            role: String,
            exp: usize,
            iat: usize,
            iss: String,
            aud: String,
            cnf: Cnf,
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize;
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("hard-kid".into());
        let spk = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7u8; 32]);
        let token = encode(
            &header,
            &Claims {
                sub: "u-hard".into(),
                jti: "j-hard".into(),
                room_id: room_id.into(),
                role: "guest".into(),
                exp: now + 600,
                iat: now,
                iss: "alchemy-world".into(),
                aud: "alchemy-room".into(),
                cnf: Cnf { spk },
            },
            &encoding,
        )
        .expect("jwt");
        (verifier, token)
    }

    #[test]
    fn jwks_soft_accepts_hard_rejects_until_fresh() {
        // One RSA fixture covers soft SWR + hard fail-closed (avoid dual 2048 gen).
        let (verifier, token) = test_rsa_jwt("local");
        verifier.set_fetched_at_age_for_test(Duration::from_secs(61));
        assert!(
            verifier.verify(&token).is_ok(),
            "soft-stale must still accept (SWR)"
        );
        verifier.set_fetched_at_age_for_test(Duration::from_secs(301));
        let err = verifier.verify(&token).expect_err("hard-stale");
        assert!(err.to_string().contains("hard-stale"), "got: {err:#}");
        verifier.set_fetched_at_age_for_test(Duration::from_secs(0));
        assert!(verifier.verify(&token).is_ok());
    }
}
