//! Auth session -> host Room Token -> HostAttestation chain (x0025 / x0039 / x003a) + env gates.
//!
//! Guarantees: release builds fail closed without explicit credentials (x001e) and ignore the
//! `ALCHEMY_ROOM_TOKEN` bypass (x0025); invalid `ROOM_VISIBILITY` errors instead of widening ACL.
//! Non-guarantees: sealed Manifest / worker thread (see `credential_worker`).

use super::http::credential_agent;
use super::timing::{
    earliest_hard_expire, hard_expire_deadline, hard_expire_or_hint, refresh_deadline,
    BYPASS_ROOM_TOKEN_TTL_HINT,
};
use super::verify::RoomTokenVerifier;
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde::Deserialize;
use std::time::{Duration, Instant};

const DEFAULT_AUTH_URL: &str = "http://127.0.0.1:4002";

/// Debug-only seed credentials (x001e: absent from release binaries).
#[cfg(debug_assertions)]
const DEFAULT_AUTH_USER: &str = "alice";
#[cfg(debug_assertions)]
const DEFAULT_AUTH_PASSWORD: &str = "Password1";

#[derive(Debug, Deserialize)]
struct MtlsResponse {
    client_cert_pem: String,
    #[serde(default)]
    #[allow(dead_code)]
    ca_cert_pem: String,
    /// Soft TTL / NotAfter alignment (x0068).
    #[serde(default)]
    expires_at: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    token: String,
    #[serde(default)]
    expires_in: u64,
    /// Zenoh key channel secret (x004e). Empty on pre-x004e world → `require_wire_cap` / WIRE_CAP.
    #[serde(default)]
    wire_cap: String,
    /// Room AEAD key, URL-safe base64 of 32 bytes (x0069). Empty → fail closed except debug bypass.
    #[serde(default)]
    room_key: String,
    /// Room key epoch (x007e). Missing → 0.
    #[serde(default)]
    room_key_epoch: u64,
    /// Host-only: previous Room key while world `RoomKeyGrace` is live (x007e).
    #[serde(default)]
    room_key_prev: Option<String>,
    /// Host-only: previous epoch paired with `room_key_prev`.
    #[serde(default)]
    room_key_prev_epoch: Option<u64>,
    /// Relay Allocation Token (x004e); empty when Relay unused or world older than x004e.
    #[serde(default)]
    allocation_token: String,
    /// x0066 world-issued client cert.
    #[serde(default)]
    mtls: Option<MtlsResponse>,
}

/// World-issued ephemeral client certificate (x0066 / x0068).
#[derive(Debug, Clone)]
pub struct WorldMtls {
    pub client_cert_pem: String,
    pub expires_at: String,
}

/// Host Room Token bootstrap payload (x004e includes wire_cap + allocation; x0066 mtls).
#[derive(Debug, Clone)]
pub struct HostTokenBundle {
    pub token: String,
    pub expires_in: u64,
    pub wire_cap: String,
    /// Raw 32-byte Room AEAD key (x0069).
    pub room_key: [u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN],
    /// Room key epoch (x007e).
    pub room_key_epoch: u64,
    /// Optional prev from world grace (host Token only).
    pub room_key_prev: Option<([u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN], u64)>,
    pub allocation_token: String,
    pub mtls: Option<WorldMtls>,
}

fn parse_room_key_b64(b64: &str) -> Result<[u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN]> {
    alchemy_protocol::wire_aead::decode_room_key_b64(b64)
        .map_err(|e| anyhow!("room_key from token: {e}"))
}

fn room_key_from_env_or_local() -> [u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN] {
    if let Ok(b64) = std::env::var("ALCHEMY_ROOM_KEY") {
        if !b64.is_empty() {
            if let Ok(key) = parse_room_key_b64(&b64) {
                return key;
            }
            log::warn!("ALCHEMY_ROOM_KEY invalid; falling back to LOCAL_ROOM_KEY");
        }
    }
    alchemy_protocol::LOCAL_ROOM_KEY
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
/// x0066: optional `csr_pem` → `mtls` when world CA configured.
pub fn fetch_host_token(
    world_url: &str,
    room_id: &str,
    auth_token: &str,
    csr_pem: Option<&str>,
) -> Result<HostTokenBundle> {
    if let Some(token) = room_token_env_bypass() {
        let wire_cap = zenoh_connect::require_wire_cap(None)
            .unwrap_or_else(|_| alchemy_protocol::LOCAL_WIRE_CAP.to_string());
        return Ok(HostTokenBundle {
            token,
            expires_in: BYPASS_ROOM_TOKEN_TTL_HINT,
            wire_cap,
            room_key: room_key_from_env_or_local(),
            room_key_epoch: 0,
            room_key_prev: None,
            allocation_token: String::new(),
            mtls: None,
        });
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
    let mut body = serde_json::json!({
        "role": "host",
        "session_pub": session_pub_b64,
    });
    if let Some(csr) = csr_pem {
        if !csr.is_empty() {
            body["csr_pem"] = serde_json::Value::String(csr.to_string());
        }
    }
    match agent
        .post(&token_url)
        .set("content-type", "application/json")
        .set("authorization", &authz)
        .send_json(body)
    {
        Ok(resp) => {
            let parsed: TokenResponse = resp.into_json().context("parse token response")?;
            let ttl = if parsed.expires_in == 0 {
                BYPASS_ROOM_TOKEN_TTL_HINT
            } else {
                parsed.expires_in
            };
            let wire_cap = zenoh_connect::require_wire_cap(Some(parsed.wire_cap.as_str()))
                .context("host token wire_cap")?;
            let room_key = parse_room_key_b64(&parsed.room_key).context("host token room_key")?;
            let room_key_prev = match (parsed.room_key_prev.as_deref(), parsed.room_key_prev_epoch)
            {
                (Some(b64), Some(epoch)) if !b64.is_empty() => Some((
                    parse_room_key_b64(b64).context("host token room_key_prev")?,
                    epoch,
                )),
                _ => None,
            };
            let mtls = parsed.mtls.map(|m| WorldMtls {
                client_cert_pem: m.client_cert_pem,
                expires_at: m.expires_at,
            });
            Ok(HostTokenBundle {
                token: parsed.token,
                expires_in: ttl,
                wire_cap,
                room_key,
                room_key_epoch: parsed.room_key_epoch,
                room_key_prev,
                allocation_token: parsed.allocation_token,
                mtls,
            })
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
    /// Zenoh key channel secret (x004e); stable for room lifetime.
    pub wire_cap: String,
    /// Room AEAD key (x0069); rotates with `room_key_epoch` (x007e).
    pub room_key: [u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN],
    /// Room key epoch (x007e).
    pub room_key_epoch: u64,
    /// Optional prev from world `RoomKeyGrace` (host Token only · x007e).
    pub room_key_prev: Option<([u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN], u64)>,
    /// Latest Allocation Token for Relay mTLS refresh (x004e).
    pub allocation_token: String,
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
        let (csr_pem, eph_key) = if zenoh_connect::should_request_world_mtls() {
            let eph = zenoh_connect::EphemeralMtlsIdentity::generate()
                .context("ephemeral mTLS CSR (host)")?;
            (
                Some(eph.csr_pem.clone()),
                Some(eph.private_key_pem().to_string()),
            )
        } else {
            (None, None)
        };
        let bundle = fetch_host_token(world_url, room_id, &auth.access_token, csr_pem.as_deref())
            .context("fetch host Room Token")?;
        let claims = verifier
            .verify(&bundle.token)
            .context("host Room Token self-check")?;
        if claims.role != "host" {
            return Err(anyhow!(
                "host Room Token must have role=host (got {})",
                claims.role
            ));
        }
        if let Err(err) = zenoh_connect::apply_world_mtls_or_allocate(
            bundle.mtls.as_ref().map(|m| m.client_cert_pem.as_str()),
            eph_key.as_deref(),
            &bundle.wire_cap,
            "host",
            &bundle.allocation_token,
            bundle
                .mtls
                .as_ref()
                .map(|m| m.expires_at.as_str())
                .filter(|s| !s.is_empty()),
        ) {
            log::warn!("host bootstrap mTLS: {err:#}");
        }
        let (host_attestation, host_generation, attest_ttl) =
            fetch_host_attestation(world_url, room_id, &bundle.token, host_pub)
                .context("fetch HostAttestation")?;
        Ok(Self {
            auth,
            auth_refresh_at,
            auth_hard_expire_at,
            host_token: bundle.token,
            host_refresh_at: refresh_deadline(bundle.expires_in).unwrap_or_else(|| {
                Instant::now() + Duration::from_secs(BYPASS_ROOM_TOKEN_TTL_HINT)
            }),
            host_hard_expire_at: hard_expire_or_hint(bundle.expires_in, BYPASS_ROOM_TOKEN_TTL_HINT),
            host_attestation,
            host_generation,
            attest_refresh_at: refresh_deadline(attest_ttl).unwrap_or_else(|| {
                Instant::now() + Duration::from_secs(BYPASS_ROOM_TOKEN_TTL_HINT)
            }),
            attest_hard_expire_at: hard_expire_or_hint(attest_ttl, BYPASS_ROOM_TOKEN_TTL_HINT),
            wire_cap: bundle.wire_cap,
            room_key: bundle.room_key,
            room_key_epoch: bundle.room_key_epoch,
            room_key_prev: bundle.room_key_prev,
            allocation_token: bundle.allocation_token,
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
        let (csr_pem, eph_key) = if zenoh_connect::should_request_world_mtls() {
            match zenoh_connect::EphemeralMtlsIdentity::generate() {
                Ok(eph) => (
                    Some(eph.csr_pem.clone()),
                    Some(eph.private_key_pem().to_string()),
                ),
                Err(err) => {
                    log::warn!("host refresh CSR failed: {err:#}");
                    (None, None)
                }
            }
        } else {
            (None, None)
        };
        let bundle = fetch_host_token(
            world_url,
            room_id,
            &self.auth.access_token,
            csr_pem.as_deref(),
        )
        .context("refresh host Room Token")?;
        let claims = verifier
            .verify(&bundle.token)
            .context("refreshed host Room Token verify")?;
        if claims.role != "host" {
            return Err(anyhow!(
                "refreshed host Room Token must have role=host (got {})",
                claims.role
            ));
        }
        self.host_token = bundle.token;
        // wire_cap is room-lifetime stable (Zenoh keys fixed at boot).
        // room_key / epoch may rotate (x007e); caller syncs SharedRoomAead via apply_room_aead.
        if self.wire_cap.is_empty() {
            self.wire_cap = bundle.wire_cap.clone();
        } else if !bundle.wire_cap.is_empty() && bundle.wire_cap != self.wire_cap {
            log::warn!("host Token refresh wire_cap changed (ignored; keys stay on boot cap)");
        }
        self.room_key = bundle.room_key;
        self.room_key_epoch = bundle.room_key_epoch;
        self.room_key_prev = bundle.room_key_prev;
        self.allocation_token = bundle.allocation_token.clone();
        let ttl = bundle.expires_in;
        self.host_refresh_at = refresh_deadline(ttl)
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(BYPASS_ROOM_TOKEN_TTL_HINT));
        self.host_hard_expire_at = hard_expire_or_hint(ttl, BYPASS_ROOM_TOKEN_TTL_HINT);
        // Refresh mTLS PEM files (x0068 NotAfter ≈ soft TTL). Host does not mid-flight
        // reopen Zenoh; established TLS usually continues. Crash/restart or transport
        // reconnect uses these files — keep Token refresh healthy on WAN.
        match zenoh_connect::apply_world_mtls_or_allocate(
            bundle.mtls.as_ref().map(|m| m.client_cert_pem.as_str()),
            eph_key.as_deref(),
            &bundle.wire_cap,
            "host",
            &self.allocation_token,
            bundle
                .mtls
                .as_ref()
                .map(|m| m.expires_at.as_str())
                .filter(|s| !s.is_empty()),
        ) {
            Ok(Some(id)) => {
                log::info!(
                    "refreshed host mTLS cert files cn={} soft_ttl={}s (no mid-flight reopen; next open / reconnect)",
                    id.cn,
                    id.expires_in
                );
            }
            Ok(None) => {}
            Err(err) => log::warn!("host mTLS refresh failed: {err:#}"),
        }
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
}
