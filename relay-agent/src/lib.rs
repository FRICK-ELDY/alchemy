//! Alchemy Relay Agent (x004e / x0068 / x007a).
//!
//! Guarantees: `POST /v1/allocate` verifies Allocation Token (`aud=alchemy-relay`) via world JWKS
//! (soft refresh + hard TTL), then issues a client cert with CN `alchemy-host` / `alchemy-guest`
//! whose **NotAfter ≈ `expires_in` (300s)**. Body size, in-flight concurrency, and `jti` reuse are
//! bounded. Optional loopback `/metrics` + `room_jti` log correlation (x007a).
//! Non-guarantees: CCU/QoS, multi-relay discovery, L7 Zenoh proxy, CSR-only allocate (P0-4).

mod metrics;

use anyhow::{anyhow, Context, Result};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use metrics::RelayMetrics;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn relay_metrics() -> &'static Arc<RelayMetrics> {
    static M: OnceLock<Arc<RelayMetrics>> = OnceLock::new();
    M.get_or_init(|| Arc::new(RelayMetrics::default()))
}

pub const ISSUER: &str = "alchemy-world";
pub const AUDIENCE: &str = "alchemy-relay";
pub const CERT_TTL_SECS: u64 = 300;
pub const JWKS_SOFT_TTL: Duration = Duration::from_secs(60);
pub const JWKS_HARD_TTL: Duration = Duration::from_secs(600);
pub const MAX_BODY_BYTES: usize = 16 * 1024;
pub const MAX_HEADER_BYTES: usize = 8 * 1024;
pub const MAX_IN_FLIGHT: usize = 8;
/// Cap on concurrent accepted connections (all methods; mitigates slowloris thread growth).
pub const MAX_CONNECTIONS: usize = 32;
pub const UNKNOWN_KID_COOLDOWN: Duration = Duration::from_secs(5);
pub const MAX_UNKNOWN_KID_ENTRIES: usize = 256;
pub const JTI_DEDUP_TTL: Duration = Duration::from_secs(300);
pub const CN_HOST: &str = "alchemy-host";
pub const CN_GUEST: &str = "alchemy-guest";

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
pub struct AllocClaims {
    #[allow(dead_code)]
    pub sub: String,
    pub room_id: String,
    pub role: String,
    pub wire_cap: String,
    pub relay_id: String,
    pub jti: String,
    /// Room Token `jti` for cross-service log correlation (x007a).
    pub room_jti: String,
    #[allow(dead_code)]
    pub exp: i64,
}

#[derive(Debug, Deserialize)]
struct AllocateRequest {
    allocation_token: String,
}

#[derive(Debug, Serialize)]
pub struct AllocateResponse {
    pub role: String,
    pub room_id: String,
    pub wire_cap: String,
    pub cert_pem: String,
    pub key_pem: String,
    pub ca_pem: String,
    pub expires_in: u64,
    pub cn: String,
}

struct KeyCache {
    keys: HashMap<String, DecodingKey>,
    fetched_at: Instant,
}

/// Short-lived jti set (FIFO eviction by insert time).
pub struct JtiDedupe {
    entries: VecDeque<(String, Instant)>,
}

impl JtiDedupe {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
        }
    }

    fn purge(&mut self, now: Instant) {
        while let Some((_, at)) = self.entries.front() {
            if now.duration_since(*at) > JTI_DEDUP_TTL {
                self.entries.pop_front();
            } else {
                break;
            }
        }
    }

    /// Returns true if `jti` is new and recorded; false if already seen.
    pub fn try_insert(&mut self, jti: &str, now: Instant) -> bool {
        self.purge(now);
        if self.entries.iter().any(|(j, _)| j == jti) {
            return false;
        }
        self.entries.push_back((jti.to_string(), now));
        true
    }

    pub fn remove(&mut self, jti: &str) {
        self.entries.retain(|(j, _)| j != jti);
    }
}

impl Default for JtiDedupe {
    fn default() -> Self {
        Self::new()
    }
}

pub struct AgentState {
    pub jwks_url: String,
    pub relay_id: String,
    pub client_ca_cert: PathBuf,
    pub client_ca_key: PathBuf,
    pub work_dir: PathBuf,
    cache: RwLock<KeyCache>,
    pub openssl: PathBuf,
    cert_issue_lock: Mutex<()>,
    /// Arc so [`CounterGuard`] can outlive borrows into `thread::spawn`.
    in_flight: Arc<Mutex<usize>>,
    connections: Arc<Mutex<usize>>,
    jti_seen: Mutex<JtiDedupe>,
    /// Per-kid cooldown after unknown kid (avoids process-wide allocate DoS).
    unknown_kid_until: Mutex<HashMap<String, Instant>>,
}

struct CounterGuard {
    slot: Arc<Mutex<usize>>,
}

impl CounterGuard {
    fn try_acquire(slot: &Arc<Mutex<usize>>, max: usize) -> Option<Self> {
        let mut n = slot.lock().ok()?;
        if *n >= max {
            return None;
        }
        *n += 1;
        Some(Self {
            slot: Arc::clone(slot),
        })
    }
}

impl Drop for CounterGuard {
    fn drop(&mut self) {
        if let Ok(mut n) = self.slot.lock() {
            *n = n.saturating_sub(1);
        }
    }
}

/// UTC ASN.1 time `YYYYMMDDHHMMSSZ` for openssl `-not_after` / `-not_before`.
pub fn asn1_utc_from_unix(secs: u64) -> String {
    // Manual calendar (UTC) — avoid chrono dependency (guardrails: no new crates).
    let days = (secs / 86_400) as i64;
    let tod = secs % 86_400;
    let hour = tod / 3600;
    let min = (tod % 3600) / 60;
    let sec = tod % 60;

    // Civil from days since 1970-01-01 (Howard Hinnant algorithm).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}{m:02}{d:02}{hour:02}{min:02}{sec:02}Z")
}

pub fn content_length(buf: &[u8]) -> Option<usize> {
    let s = std::str::from_utf8(buf).ok()?;
    for line in s.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            return v.trim().parse().ok();
        }
        if line.is_empty() {
            break;
        }
    }
    None
}

pub fn find_body_start(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

/// Reject when Content-Length exceeds cap (call as soon as headers are complete).
pub fn content_length_over_limit(buf: &[u8], limit: usize) -> bool {
    content_length(buf).is_some_and(|cl| cl > limit)
}

pub fn run() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let listen = std::env::var("RELAY_AGENT_LISTEN").unwrap_or_else(|_| "127.0.0.1:7450".into());
    if !listen.starts_with("127.0.0.1")
        && !listen.starts_with("localhost")
        && !listen.starts_with("[::1]")
    {
        log::warn!(
            "RELAY_AGENT_LISTEN={listen} is not loopback — allocate responses include private keys over cleartext HTTP"
        );
    }
    let jwks_url = std::env::var("WORLD_JWKS_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:4003/.well-known/jwks.json".into());
    let relay_id = std::env::var("RELAY_ID").unwrap_or_else(|_| "alchemy-official".into());
    let tls_dir =
        PathBuf::from(std::env::var("RELAY_TLS_DIR").unwrap_or_else(|_| "/etc/alchemy/tls".into()));
    let work_dir = PathBuf::from(
        std::env::var("RELAY_AGENT_WORK_DIR").unwrap_or_else(|_| "/tmp/alchemy-relay-agent".into()),
    );
    fs::create_dir_all(&work_dir).context("create work dir")?;

    let state = Arc::new(AgentState {
        jwks_url,
        relay_id,
        client_ca_cert: tls_dir.join("client-ca-cert.pem"),
        client_ca_key: tls_dir.join("client-ca-key.pem"),
        work_dir,
        cache: RwLock::new(KeyCache {
            keys: HashMap::new(),
            fetched_at: Instant::now() - Duration::from_secs(3600),
        }),
        openssl: PathBuf::from(std::env::var("OPENSSL_BIN").unwrap_or_else(|_| "openssl".into())),
        cert_issue_lock: Mutex::new(()),
        in_flight: Arc::new(Mutex::new(0)),
        connections: Arc::new(Mutex::new(0)),
        jti_seen: Mutex::new(JtiDedupe::new()),
        unknown_kid_until: Mutex::new(HashMap::new()),
    });

    if !state.client_ca_cert.is_file() || !state.client_ca_key.is_file() {
        anyhow::bail!(
            "missing client CA at {:?} / {:?} (run gen-wan-dev-certs.sh)",
            state.client_ca_cert,
            state.client_ca_key
        );
    }

    state.refresh_jwks().context("initial JWKS fetch")?;
    metrics::maybe_spawn_from_env(Arc::clone(relay_metrics()));
    let listener = TcpListener::bind(&listen).with_context(|| format!("bind {listen}"))?;
    log::info!(
        "relay-agent x0068 listening on http://{listen} relay_id={} jwks={} cert_ttl_secs={CERT_TTL_SECS} max_body={MAX_BODY_BYTES} max_in_flight={MAX_IN_FLIGHT} max_connections={MAX_CONNECTIONS}",
        state.relay_id,
        state.jwks_url
    );

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                log::warn!("accept: {e}");
                continue;
            }
        };
        let state = Arc::clone(&state);
        let Some(conn_guard) = CounterGuard::try_acquire(&state.connections, MAX_CONNECTIONS)
        else {
            let mut s = stream;
            let _ = write_http(&mut s, 503, "{\"error\":\"too_many_connections\"}");
            continue;
        };
        thread::spawn(move || {
            let _conn = conn_guard;
            if let Err(e) = handle_client(stream, &state) {
                log::warn!("request failed: {e:#}");
            }
        });
    }
    Ok(())
}

impl AgentState {
    pub fn refresh_jwks(&self) -> Result<()> {
        let body: Jwks = ureq_get_json(&self.jwks_url)?;
        let mut map = HashMap::new();
        for jwk in body.keys {
            if jwk.kty != "RSA" {
                continue;
            }
            let (Some(kid), Some(n), Some(e)) = (jwk.kid, jwk.n, jwk.e) else {
                continue;
            };
            let key =
                DecodingKey::from_rsa_components(&n, &e).map_err(|e| anyhow!("jwk decode: {e}"))?;
            map.insert(kid, key);
        }
        if map.is_empty() {
            return Err(anyhow!("JWKS had no RSA keys"));
        }
        let mut cache = self
            .cache
            .write()
            .map_err(|_| anyhow!("jwks lock poisoned"))?;
        cache.keys = map;
        cache.fetched_at = Instant::now();
        Ok(())
    }

    /// Install JWKS keys for tests (bypasses HTTP).
    pub fn install_jwks_for_test(&self, keys: HashMap<String, DecodingKey>, fetched_at: Instant) {
        let mut cache = self.cache.write().expect("jwks lock");
        cache.keys = keys;
        cache.fetched_at = fetched_at;
    }

    pub fn jwks_age(&self) -> Duration {
        self.cache.read().expect("jwks lock").fetched_at.elapsed()
    }

    pub fn verify_allocation(&self, token: &str) -> Result<AllocClaims> {
        {
            let cache = self
                .cache
                .read()
                .map_err(|_| anyhow!("jwks lock poisoned"))?;
            if cache.fetched_at.elapsed() > JWKS_SOFT_TTL {
                drop(cache);
                if let Err(e) = self.refresh_jwks() {
                    let age = self.jwks_age();
                    if age > JWKS_HARD_TTL {
                        return Err(anyhow!("JWKS hard stale ({age:?}): {e:#}"));
                    }
                    log::warn!("JWKS soft refresh failed (age={age:?}): {e:#}");
                }
            }
        }

        let header = decode_header(token).map_err(|e| anyhow!("jwt header: {e}"))?;
        let kid = header.kid.ok_or_else(|| anyhow!("missing kid"))?;

        {
            let mut map = self
                .unknown_kid_until
                .lock()
                .map_err(|_| anyhow!("kid cooldown lock poisoned"))?;
            let now = Instant::now();
            map.retain(|_, until| *until > now);
            if map.get(&kid).is_some_and(|until| *until > now) {
                return Err(anyhow!("unknown kid cooldown kid={kid}"));
            }
        }

        let key_present = {
            let cache = self
                .cache
                .read()
                .map_err(|_| anyhow!("jwks lock poisoned"))?;
            cache.keys.contains_key(&kid)
        };
        if !key_present {
            let _ = self.refresh_jwks();
            let still_missing = {
                let cache = self
                    .cache
                    .read()
                    .map_err(|_| anyhow!("jwks lock poisoned"))?;
                !cache.keys.contains_key(&kid)
            };
            if still_missing {
                let mut map = self
                    .unknown_kid_until
                    .lock()
                    .map_err(|_| anyhow!("kid cooldown lock poisoned"))?;
                let now = Instant::now();
                map.retain(|_, until| *until > now);
                // Cap: FIFO by earliest `until` (equal TTL ⇒ oldest insert). Evict then insert
                // so the same unknown kid still gets a cooldown window.
                if map.len() >= MAX_UNKNOWN_KID_ENTRIES && !map.contains_key(&kid) {
                    if let Some(evict) = map
                        .iter()
                        .min_by_key(|(_, until)| *until)
                        .map(|(k, _)| k.clone())
                    {
                        map.remove(&evict);
                        log::warn!(
                            "unknown-kid cooldown map full; evicted earliest entry kid={evict}"
                        );
                    }
                }
                map.insert(kid.clone(), now + UNKNOWN_KID_COOLDOWN);
                return Err(anyhow!("unknown kid={kid}"));
            }
        }

        // Hard TTL: refuse if cache is too old even after soft attempts.
        {
            let age = self.jwks_age();
            if age > JWKS_HARD_TTL {
                if let Err(e) = self.refresh_jwks() {
                    return Err(anyhow!("JWKS hard stale ({age:?}): {e:#}"));
                }
                if self.jwks_age() > JWKS_HARD_TTL {
                    return Err(anyhow!("JWKS hard stale after refresh"));
                }
            }
        }

        let cache = self
            .cache
            .read()
            .map_err(|_| anyhow!("jwks lock poisoned"))?;
        let key = cache
            .keys
            .get(&kid)
            .ok_or_else(|| anyhow!("unknown kid={kid}"))?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[ISSUER]);
        validation.set_audience(&[AUDIENCE]);
        let data = decode::<AllocClaims>(token, key, &validation)
            .map_err(|e| anyhow!("allocation verify: {e}"))?;
        if data.claims.relay_id != self.relay_id {
            return Err(anyhow!(
                "relay_id mismatch: token={} expected={}",
                data.claims.relay_id,
                self.relay_id
            ));
        }
        if data.claims.role != "host" && data.claims.role != "guest" {
            return Err(anyhow!("invalid role"));
        }
        if data.claims.room_jti.is_empty() || data.claims.room_jti.contains('/') {
            return Err(anyhow!("room_jti missing or contains '/'"));
        }
        Ok(data.claims)
    }

    /// Reserve `jti` before openssl; call [`release_jti`] if issue fails so retries work.
    pub fn try_reserve_jti(&self, jti: &str) -> Result<()> {
        let mut guard = self
            .jti_seen
            .lock()
            .map_err(|_| anyhow!("jti lock poisoned"))?;
        if !guard.try_insert(jti, Instant::now()) {
            return Err(anyhow!("jti reused"));
        }
        Ok(())
    }

    pub fn release_jti(&self, jti: &str) {
        if let Ok(mut guard) = self.jti_seen.lock() {
            guard.remove(jti);
        }
    }

    pub fn issue_cert(&self, role: &str) -> Result<(String, String, String)> {
        let _guard = self
            .cert_issue_lock
            .lock()
            .map_err(|_| anyhow!("cert_issue_lock poisoned"))?;
        let cn = if role == "host" { CN_HOST } else { CN_GUEST };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let not_before = asn1_utc_from_unix(now.saturating_sub(60));
        let not_after = asn1_utc_from_unix(now + CERT_TTL_SECS);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        // OpenSSL -set_serial parses decimal unless 0x-prefixed (full nanos hex).
        let serial = format!("0x{stamp:x}");
        let prefix = self.work_dir.join(format!("ephemeral-{stamp}"));
        let key_path = PathBuf::from(format!("{}.key.pem", prefix.display()));
        let csr_path = PathBuf::from(format!("{}.csr.pem", prefix.display()));
        let cert_path = PathBuf::from(format!("{}.cert.pem", prefix.display()));
        let ext_path = PathBuf::from(format!("{}.ext", prefix.display()));

        run_openssl(
            &self.openssl,
            &[
                "req",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-keyout",
                key_path.to_str().ok_or_else(|| anyhow!("key path utf8"))?,
                "-out",
                csr_path.to_str().ok_or_else(|| anyhow!("csr path utf8"))?,
                "-subj",
                &format!("/CN={cn}"),
            ],
        )?;

        fs::write(
            &ext_path,
            "basicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=clientAuth\n",
        )?;

        run_openssl(
            &self.openssl,
            &[
                "x509",
                "-req",
                "-in",
                csr_path.to_str().ok_or_else(|| anyhow!("csr path utf8"))?,
                "-CA",
                self.client_ca_cert
                    .to_str()
                    .ok_or_else(|| anyhow!("ca cert path utf8"))?,
                "-CAkey",
                self.client_ca_key
                    .to_str()
                    .ok_or_else(|| anyhow!("ca key path utf8"))?,
                "-set_serial",
                &serial,
                "-out",
                cert_path
                    .to_str()
                    .ok_or_else(|| anyhow!("cert path utf8"))?,
                "-not_before",
                &not_before,
                "-not_after",
                &not_after,
                "-extfile",
                ext_path.to_str().ok_or_else(|| anyhow!("ext path utf8"))?,
            ],
        )?;

        let cert_pem = fs::read_to_string(&cert_path)?;
        let key_pem = fs::read_to_string(&key_path)?;
        let ca_pem = fs::read_to_string(&self.client_ca_cert)?;
        let _ = fs::remove_file(&key_path);
        let _ = fs::remove_file(&csr_path);
        let _ = fs::remove_file(&cert_path);
        let _ = fs::remove_file(&ext_path);
        Ok((cert_pem, key_pem, ca_pem))
    }
}

fn run_openssl(bin: &Path, args: &[&str]) -> Result<()> {
    let out = Command::new(bin)
        .args(args)
        .output()
        .with_context(|| format!("spawn openssl {:?}", args))?;
    if !out.status.success() {
        return Err(anyhow!(
            "openssl failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

fn ureq_get_json<T: for<'de> Deserialize<'de>>(url: &str) -> Result<T> {
    let out = Command::new("curl")
        .args(["-fsSL", "--max-time", "5", url])
        .output()
        .context("curl JWKS (install curl or set PATH)")?;
    if !out.status.success() {
        return Err(anyhow!(
            "curl JWKS failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    serde_json::from_slice(&out.stdout).context("parse JWKS JSON")
}

pub fn handle_client(mut stream: TcpStream, state: &AgentState) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_HEADER_BYTES + MAX_BODY_BYTES {
            return write_http(&mut stream, 413, "{\"error\":\"too_large\"}");
        }
        if let Some(body_start) = find_body_start(&buf) {
            if body_start > MAX_HEADER_BYTES {
                return write_http(&mut stream, 413, "{\"error\":\"too_large\"}");
            }
            if content_length_over_limit(&buf, MAX_BODY_BYTES) {
                return write_http(&mut stream, 413, "{\"error\":\"too_large\"}");
            }
            if let Some(cl) = content_length(&buf) {
                while buf.len() < body_start + cl {
                    let n = stream.read(&mut chunk)?;
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if buf.len() > body_start + MAX_BODY_BYTES {
                        return write_http(&mut stream, 413, "{\"error\":\"too_large\"}");
                    }
                }
            } else {
                // No Content-Length: reject POST bodies larger than cap once headers done.
                if buf.len().saturating_sub(body_start) > MAX_BODY_BYTES {
                    return write_http(&mut stream, 413, "{\"error\":\"too_large\"}");
                }
            }
            break;
        }
        if buf.len() > MAX_HEADER_BYTES {
            return write_http(&mut stream, 413, "{\"error\":\"too_large\"}");
        }
    }

    let req = String::from_utf8_lossy(&buf);
    let first = req.lines().next().unwrap_or("");
    if first.starts_with("GET /health") {
        return write_http(&mut stream, 200, "{\"ok\":true}");
    }
    if !first.starts_with("POST /v1/allocate") {
        return write_http(&mut stream, 404, "{\"error\":\"not_found\"}");
    }

    let _inflight = match CounterGuard::try_acquire(&state.in_flight, MAX_IN_FLIGHT) {
        Some(g) => g,
        None => return allocate_deny(&mut stream, 503, "{\"error\":\"too_many_requests\"}"),
    };

    let body = match find_body_start(&buf) {
        Some(i) => &buf[i..],
        None => return allocate_deny(&mut stream, 400, "{\"error\":\"bad_request\"}"),
    };
    if body.len() > MAX_BODY_BYTES {
        return allocate_deny(&mut stream, 413, "{\"error\":\"too_large\"}");
    }
    let req_json: AllocateRequest = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return allocate_deny(&mut stream, 400, "{\"error\":\"invalid_json\"}"),
    };

    let claims = match state.verify_allocation(&req_json.allocation_token) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("allocate deny: {e:#}");
            return allocate_deny(&mut stream, 401, "{\"error\":\"invalid_allocation_token\"}");
        }
    };

    // Reserve jti before openssl so parallel same-jti cannot both issue; release on fail.
    if let Err(e) = state.try_reserve_jti(&claims.jti) {
        log::warn!("allocate deny: {e:#}");
        return allocate_deny(&mut stream, 401, "{\"error\":\"jti_reused\"}");
    }

    let (cert_pem, key_pem, ca_pem) = match state.issue_cert(&claims.role) {
        Ok(v) => v,
        Err(e) => {
            state.release_jti(&claims.jti);
            log::error!("cert issue: {e:#}");
            return allocate_deny(&mut stream, 500, "{\"error\":\"cert_issue_failed\"}");
        }
    };

    let cn = if claims.role == "host" {
        CN_HOST
    } else {
        CN_GUEST
    };
    let room_jti = claims.room_jti.clone();
    let alloc_jti = claims.jti.clone();
    let resp = AllocateResponse {
        role: claims.role,
        room_id: claims.room_id,
        wire_cap: claims.wire_cap,
        cert_pem,
        key_pem,
        ca_pem,
        expires_in: CERT_TTL_SECS,
        cn: cn.to_string(),
    };
    // Client never saw the body → release so the same jti can retry.
    let body = match serde_json::to_string(&resp) {
        Ok(b) => b,
        Err(e) => {
            state.release_jti(&claims.jti);
            relay_metrics().inc_allocate_err();
            return Err(e.into());
        }
    };
    match write_http(&mut stream, 200, &body) {
        Ok(()) => {
            relay_metrics().inc_allocate_ok();
            log::info!(
                "allocate ok room_jti={room_jti} jti={alloc_jti} room_id={} role={}",
                resp.room_id,
                resp.role
            );
            Ok(())
        }
        Err(e) => {
            state.release_jti(&claims.jti);
            relay_metrics().inc_allocate_err();
            Err(e)
        }
    }
}

fn allocate_deny(stream: &mut TcpStream, status: u16, body: &str) -> Result<()> {
    relay_metrics().inc_allocate_err();
    write_http(stream, status, body)
}

pub fn write_http(stream: &mut TcpStream, status: u16, body: &str) -> Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        413 => "Payload Too Large",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn openssl_bin() -> PathBuf {
        if let Ok(p) = std::env::var("OPENSSL_BIN") {
            return PathBuf::from(p);
        }
        for cand in [
            "openssl",
            r"C:\Program Files\Git\usr\bin\openssl.exe",
            r"C:\Program Files\OpenSSL-Win64\bin\openssl.exe",
        ] {
            if Command::new(cand)
                .arg("version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                return PathBuf::from(cand);
            }
        }
        panic!("openssl not found; set OPENSSL_BIN");
    }

    #[test]
    fn asn1_utc_epoch() {
        assert_eq!(asn1_utc_from_unix(0), "19700101000000Z");
        // 2026-11-01 00:00:00 UTC
        assert_eq!(asn1_utc_from_unix(1_793_491_200), "20261101000000Z");
    }

    #[test]
    fn content_length_over_limit_detects() {
        let headers = b"POST /v1/allocate HTTP/1.1\r\nContent-Length: 20000\r\n\r\n";
        assert!(content_length_over_limit(headers, MAX_BODY_BYTES));
        let ok = b"POST /v1/allocate HTTP/1.1\r\nContent-Length: 100\r\n\r\n";
        assert!(!content_length_over_limit(ok, MAX_BODY_BYTES));
    }

    #[test]
    fn jti_dedupe_rejects_reuse() {
        let mut d = JtiDedupe::new();
        let now = Instant::now();
        assert!(d.try_insert("a", now));
        assert!(!d.try_insert("a", now));
        assert!(d.try_insert("b", now));
    }

    #[test]
    fn jwks_hard_stale_rejects_when_refresh_fails() {
        let openssl = openssl_bin();
        let work = std::env::temp_dir().join(format!(
            "alchemy-relay-jwks-hard-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&work).unwrap();
        // Empty CA paths unused for this test.
        let state = AgentState {
            jwks_url: "http://127.0.0.1:9/.well-known/jwks.json".into(),
            relay_id: "alchemy-official".into(),
            client_ca_cert: work.join("missing-ca.pem"),
            client_ca_key: work.join("missing-key.pem"),
            work_dir: work.join("w"),
            cache: RwLock::new(KeyCache {
                keys: HashMap::new(),
                fetched_at: Instant::now() - JWKS_HARD_TTL - Duration::from_secs(1),
            }),
            openssl,
            cert_issue_lock: Mutex::new(()),
            in_flight: Arc::new(Mutex::new(0)),
            connections: Arc::new(Mutex::new(0)),
            jti_seen: Mutex::new(JtiDedupe::new()),
            unknown_kid_until: Mutex::new(HashMap::new()),
        };
        // Minimal unsigned JWT header.kid present — verify fails on hard stale before decode.
        let token = "eyJhbGciOiJSUzI1NiIsImtpZCI6InRlc3QifQ.e30.e30";
        let err = state.verify_allocation(token).unwrap_err().to_string();
        assert!(
            err.contains("hard stale") || err.contains("JWKS"),
            "unexpected err={err}"
        );
        let _ = fs::remove_dir_all(&work);
    }

    #[test]
    fn http_health_ok_without_inflight_slot() {
        let state = test_state_minimal();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let st = Arc::new(state);
        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_client(stream, &st).unwrap();
        });
        let mut client = TcpStream::connect(addr).unwrap();
        client
            .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut resp = String::new();
        client.read_to_string(&mut resp).unwrap();
        assert!(resp.starts_with("HTTP/1.1 200"), "{resp}");
    }

    #[test]
    fn http_oversize_content_length_is_413() {
        let state = test_state_minimal();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let st = Arc::new(state);
        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let _ = handle_client(stream, &st);
        });
        let mut client = TcpStream::connect(addr).unwrap();
        let req = format!(
            "POST /v1/allocate HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY_BYTES + 1
        );
        client.write_all(req.as_bytes()).unwrap();
        let mut resp = String::new();
        client.read_to_string(&mut resp).unwrap();
        assert!(resp.starts_with("HTTP/1.1 413"), "{resp}");
    }

    fn test_state_minimal() -> AgentState {
        let openssl = openssl_bin();
        let work = std::env::temp_dir().join(format!(
            "alchemy-relay-http-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&work).unwrap();
        AgentState {
            jwks_url: "http://127.0.0.1:9/.well-known/jwks.json".into(),
            relay_id: "alchemy-official".into(),
            client_ca_cert: work.join("missing-ca.pem"),
            client_ca_key: work.join("missing-key.pem"),
            work_dir: work.join("w"),
            cache: RwLock::new(KeyCache {
                keys: HashMap::new(),
                fetched_at: Instant::now(),
            }),
            openssl,
            cert_issue_lock: Mutex::new(()),
            in_flight: Arc::new(Mutex::new(0)),
            connections: Arc::new(Mutex::new(0)),
            jti_seen: Mutex::new(JtiDedupe::new()),
            unknown_kid_until: Mutex::new(HashMap::new()),
        }
    }

    #[test]
    fn http_allocate_503_when_inflight_full() {
        let state = test_state_minimal();
        *state.in_flight.lock().unwrap() = MAX_IN_FLIGHT;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let st = Arc::new(state);
        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let _ = handle_client(stream, &st);
        });
        let mut client = TcpStream::connect(addr).unwrap();
        let body = br#"{"allocation_token":"x"}"#;
        let req = format!(
            "POST /v1/allocate HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        client.write_all(req.as_bytes()).unwrap();
        client.write_all(body).unwrap();
        let mut resp = String::new();
        client.read_to_string(&mut resp).unwrap();
        assert!(resp.starts_with("HTTP/1.1 503"), "{resp}");
        assert!(resp.contains("too_many_requests"), "{resp}");
    }

    #[test]
    fn jti_reserve_release_roundtrip() {
        let state = test_state_minimal();
        state.try_reserve_jti("j1").unwrap();
        assert!(state.try_reserve_jti("j1").is_err());
        state.release_jti("j1");
        state.try_reserve_jti("j1").unwrap();
    }

    #[test]
    fn room_jti_empty_rejected_after_decode() {
        use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
        use serde::Serialize;

        #[derive(Serialize)]
        struct Claims<'a> {
            sub: &'a str,
            room_id: &'a str,
            role: &'a str,
            wire_cap: &'a str,
            relay_id: &'a str,
            jti: &'a str,
            room_jti: &'a str,
            exp: i64,
            iss: &'a str,
            aud: &'a str,
        }

        let openssl = openssl_bin();
        let work = std::env::temp_dir().join(format!(
            "alchemy-relay-room-jti-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&work).unwrap();
        let priv_pem = work.join("jwt.key.pem");
        let pub_pem = work.join("jwt.pub.pem");
        run_openssl(
            &openssl,
            &["genrsa", "-out", priv_pem.to_str().unwrap(), "2048"],
        )
        .unwrap();
        run_openssl(
            &openssl,
            &[
                "rsa",
                "-in",
                priv_pem.to_str().unwrap(),
                "-pubout",
                "-out",
                pub_pem.to_str().unwrap(),
            ],
        )
        .unwrap();

        let mut state = test_state_minimal();
        state.openssl = openssl;
        let decoding =
            DecodingKey::from_rsa_pem(&fs::read(&pub_pem).unwrap()).expect("DecodingKey");
        let mut keys = HashMap::new();
        keys.insert("test-kid".into(), decoding);
        state.install_jwks_for_test(keys, Instant::now());

        let encoding =
            EncodingKey::from_rsa_pem(&fs::read(&priv_pem).unwrap()).expect("EncodingKey");
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-kid".into());
        let exp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 600;
        let token = encode(
            &header,
            &Claims {
                sub: "u1",
                room_id: "r1",
                role: "guest",
                wire_cap: "wire-a",
                relay_id: "alchemy-official",
                jti: "alloc-1",
                room_jti: "",
                exp,
                iss: ISSUER,
                aud: AUDIENCE,
            },
            &encoding,
        )
        .unwrap();

        let err = state.verify_allocation(&token).unwrap_err();
        assert!(
            err.to_string().contains("room_jti"),
            "expected room_jti error, got {err:#}"
        );
        let _ = fs::remove_dir_all(&work);
    }

    #[test]
    fn http_allocate_401_jti_reused() {
        use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
        use serde::Serialize;

        #[derive(Serialize)]
        struct Claims<'a> {
            sub: &'a str,
            room_id: &'a str,
            role: &'a str,
            wire_cap: &'a str,
            relay_id: &'a str,
            jti: &'a str,
            room_jti: &'a str,
            exp: i64,
            iss: &'a str,
            aud: &'a str,
        }

        let openssl = openssl_bin();
        let work = std::env::temp_dir().join(format!(
            "alchemy-relay-jti-http-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&work).unwrap();
        let priv_pem = work.join("jwt.key.pem");
        let pub_pem = work.join("jwt.pub.pem");
        run_openssl(
            &openssl,
            &["genrsa", "-out", priv_pem.to_str().unwrap(), "2048"],
        )
        .unwrap();
        run_openssl(
            &openssl,
            &[
                "rsa",
                "-in",
                priv_pem.to_str().unwrap(),
                "-pubout",
                "-out",
                pub_pem.to_str().unwrap(),
            ],
        )
        .unwrap();

        let mut state = test_state_minimal();
        state.openssl = openssl;
        let decoding =
            DecodingKey::from_rsa_pem(&fs::read(&pub_pem).unwrap()).expect("DecodingKey");
        let mut keys = HashMap::new();
        keys.insert("test-kid".into(), decoding);
        state.install_jwks_for_test(keys, Instant::now());

        let encoding =
            EncodingKey::from_rsa_pem(&fs::read(&priv_pem).unwrap()).expect("EncodingKey");
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-kid".into());
        let exp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 600;
        let token = encode(
            &header,
            &Claims {
                sub: "u1",
                room_id: "r1",
                role: "guest",
                wire_cap: "wire-a",
                relay_id: "alchemy-official",
                jti: "reuse-me",
                room_jti: "room-jti-1",
                exp,
                iss: ISSUER,
                aud: AUDIENCE,
            },
            &encoding,
        )
        .unwrap();

        state.try_reserve_jti("reuse-me").unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let st = Arc::new(state);
        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let _ = handle_client(stream, &st);
        });
        let body = serde_json::json!({ "allocation_token": token }).to_string();
        let mut client = TcpStream::connect(addr).unwrap();
        let req = format!(
            "POST /v1/allocate HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        client.write_all(req.as_bytes()).unwrap();
        let mut resp = String::new();
        client.read_to_string(&mut resp).unwrap();
        assert!(resp.starts_with("HTTP/1.1 401"), "{resp}");
        assert!(resp.contains("jti_reused"), "{resp}");
        let _ = fs::remove_dir_all(&work);
    }

    #[test]
    fn issue_cert_not_after_matches_ttl() {
        let openssl = openssl_bin();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let work = std::env::temp_dir().join(format!("alchemy-relay-agent-test-{stamp}"));
        fs::create_dir_all(&work).unwrap();
        let ca_key = work.join("ca.key.pem");
        let ca_cert = work.join("ca.cert.pem");
        run_openssl(
            &openssl,
            &[
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-keyout",
                ca_key.to_str().unwrap(),
                "-out",
                ca_cert.to_str().unwrap(),
                "-days",
                "1",
                "-subj",
                "/CN=alchemy-test-client-ca",
            ],
        )
        .unwrap();

        let state = AgentState {
            jwks_url: "http://127.0.0.1:9/.well-known/jwks.json".into(),
            relay_id: "alchemy-official".into(),
            client_ca_cert: ca_cert,
            client_ca_key: ca_key,
            work_dir: work.join("issue"),
            cache: RwLock::new(KeyCache {
                keys: HashMap::new(),
                fetched_at: Instant::now(),
            }),
            openssl: openssl.clone(),
            cert_issue_lock: Mutex::new(()),
            in_flight: Arc::new(Mutex::new(0)),
            connections: Arc::new(Mutex::new(0)),
            jti_seen: Mutex::new(JtiDedupe::new()),
            unknown_kid_until: Mutex::new(HashMap::new()),
        };
        fs::create_dir_all(&state.work_dir).unwrap();

        let before = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let (cert_pem, _key, _ca) = state.issue_cert("guest").expect("issue");
        let after = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let cert_path = work.join("issued.pem");
        fs::write(&cert_path, &cert_pem).unwrap();
        let out = Command::new(&openssl)
            .args([
                "x509",
                "-in",
                cert_path.to_str().unwrap(),
                "-noout",
                "-enddate",
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let enddate = String::from_utf8_lossy(&out.stdout);
        // enddate=Nov  1 14:45:03 2026 GMT
        let parsed = parse_openssl_enddate(enddate.trim());
        let lo = before + CERT_TTL_SECS - 30;
        let hi = after + CERT_TTL_SECS + 30;
        assert!(
            parsed >= lo && parsed <= hi,
            "NotAfter unix={parsed} expected in [{lo},{hi}]; raw={enddate}"
        );

        let _ = fs::remove_dir_all(&work);
    }

    fn parse_openssl_enddate(line: &str) -> u64 {
        // "notAfter=Nov  1 14:45:03 2026 GMT"
        let s = line
            .strip_prefix("notAfter=")
            .unwrap_or(line)
            .trim()
            .trim_end_matches(" GMT")
            .trim_end_matches(" UTC");
        // Use openssl itself to convert to unix when available is hard; parse month.
        let parts: Vec<&str> = s.split_whitespace().collect();
        assert!(parts.len() >= 4, "bad enddate {s}");
        let mon = match parts[0] {
            "Jan" => 1,
            "Feb" => 2,
            "Mar" => 3,
            "Apr" => 4,
            "May" => 5,
            "Jun" => 6,
            "Jul" => 7,
            "Aug" => 8,
            "Sep" => 9,
            "Oct" => 10,
            "Nov" => 11,
            "Dec" => 12,
            other => panic!("month {other}"),
        };
        let day: u64 = parts[1].parse().unwrap();
        let time = parts[2];
        let year: i32 = parts[3].parse().unwrap();
        let hm: Vec<u64> = time.split(':').map(|x| x.parse().unwrap()).collect();
        civil_to_unix(year, mon, day, hm[0], hm[1], hm[2])
    }

    fn civil_to_unix(y: i32, m: u64, d: u64, hh: u64, mm: u64, ss: u64) -> u64 {
        // Inverse of asn1 path via days since epoch (Hinnant).
        let y = if m <= 2 { y - 1 } else { y };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = (y - era * 400) as u64;
        let mp = if m > 2 { m - 3 } else { m + 9 };
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = (era as i64) * 146_097 + (doe as i64) - 719_468;
        (days as u64) * 86_400 + hh * 3600 + mm * 60 + ss
    }
}
