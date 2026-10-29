//! Alchemy Relay Agent MVP (x004e).
//!
//! Guarantees: `POST /v1/allocate` verifies Allocation Token (aud=alchemy-relay) via world JWKS,
//! then issues a short-lived mTLS client cert with CN `alchemy-host` or `alchemy-guest`.
//! Non-guarantees: CCU/QoS, multi-relay discovery, L7 Zenoh proxy, hot ACL reload.

use anyhow::{anyhow, Context, Result};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const ISSUER: &str = "alchemy-world";
const AUDIENCE: &str = "alchemy-relay";
const JWKS_SOFT_TTL: Duration = Duration::from_secs(60);
/// Refresh SLA returned to clients (`expires_in`). Re-allocate before this.
/// openssl NotAfter uses `-days 1` (tooling floor); session reopen is not required within that day
/// if the process re-allocates on Room Token refresh and presents a new cert on next open.
const CERT_TTL_SECS: u64 = 300;
const OPENSSL_CERT_DAYS: &str = "1";
const CN_HOST: &str = "alchemy-host";
const CN_GUEST: &str = "alchemy-guest";

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

#[derive(Debug, Deserialize)]
struct AllocClaims {
    #[allow(dead_code)]
    sub: String,
    room_id: String,
    role: String,
    wire_cap: String,
    relay_id: String,
    #[allow(dead_code)]
    jti: String,
    #[allow(dead_code)]
    exp: i64,
}

#[derive(Debug, Deserialize)]
struct AllocateRequest {
    allocation_token: String,
}

#[derive(Debug, Serialize)]
struct AllocateResponse {
    role: String,
    room_id: String,
    wire_cap: String,
    cert_pem: String,
    key_pem: String,
    ca_pem: String,
    expires_in: u64,
    cn: String,
}

struct KeyCache {
    keys: HashMap<String, DecodingKey>,
    fetched_at: Instant,
}

struct AgentState {
    jwks_url: String,
    relay_id: String,
    client_ca_cert: PathBuf,
    client_ca_key: PathBuf,
    work_dir: PathBuf,
    cache: RwLock<KeyCache>,
    openssl: PathBuf,
    /// Serializes openssl CA signing (shared serial / key use).
    cert_issue_lock: Mutex<()>,
}

fn main() -> Result<()> {
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
    });

    if !state.client_ca_cert.is_file() || !state.client_ca_key.is_file() {
        anyhow::bail!(
            "missing client CA at {:?} / {:?} (run gen-wan-dev-certs.sh)",
            state.client_ca_cert,
            state.client_ca_key
        );
    }

    state.refresh_jwks().context("initial JWKS fetch")?;
    let listener = TcpListener::bind(&listen).with_context(|| format!("bind {listen}"))?;
    log::info!(
        "relay-agent x004e listening on http://{listen} relay_id={} jwks={} cert_refresh_secs={CERT_TTL_SECS} openssl_days={OPENSSL_CERT_DAYS}",
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
        thread::spawn(move || {
            if let Err(e) = handle_client(stream, &state) {
                log::warn!("request failed: {e:#}");
            }
        });
    }
    Ok(())
}

impl AgentState {
    fn refresh_jwks(&self) -> Result<()> {
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
        let mut cache = self.cache.write().expect("jwks lock");
        cache.keys = map;
        cache.fetched_at = Instant::now();
        Ok(())
    }

    fn verify_allocation(&self, token: &str) -> Result<AllocClaims> {
        {
            let cache = self.cache.read().expect("jwks lock");
            if cache.fetched_at.elapsed() > JWKS_SOFT_TTL {
                drop(cache);
                let _ = self.refresh_jwks();
            }
        }
        let header = decode_header(token).map_err(|e| anyhow!("jwt header: {e}"))?;
        let kid = header.kid.ok_or_else(|| anyhow!("missing kid"))?;
        let cache = self.cache.read().expect("jwks lock");
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
        Ok(data.claims)
    }

    fn issue_cert(&self, role: &str) -> Result<(String, String, String)> {
        let _guard = self
            .cert_issue_lock
            .lock()
            .map_err(|_| anyhow!("cert_issue_lock poisoned"))?;
        let cn = if role == "host" { CN_HOST } else { CN_GUEST };
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        // Unique serial avoids -CAcreateserial races on a shared .srl file (full nanos under lock).
        let serial = format!("{stamp:x}");
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
                key_path.to_str().unwrap(),
                "-out",
                csr_path.to_str().unwrap(),
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
                csr_path.to_str().unwrap(),
                "-CA",
                self.client_ca_cert.to_str().unwrap(),
                "-CAkey",
                self.client_ca_key.to_str().unwrap(),
                "-set_serial",
                &serial,
                "-out",
                cert_path.to_str().unwrap(),
                "-days",
                OPENSSL_CERT_DAYS,
                "-extfile",
                ext_path.to_str().unwrap(),
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

/// Minimal GET JSON without adding ureq as a hard dep name clash — use std + tiny HTTP.
fn ureq_get_json<T: for<'de> Deserialize<'de>>(url: &str) -> Result<T> {
    // Prefer curl for JWKS to avoid new HTTP crates (guardrails).
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

fn handle_client(mut stream: TcpStream, state: &AgentState) -> Result<()> {
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
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            // If Content-Length present, read remainder.
            if let Some(cl) = content_length(&buf) {
                if let Some(body_start) = find_body_start(&buf) {
                    while buf.len() < body_start + cl {
                        let n = stream.read(&mut chunk)?;
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                    }
                }
            }
            break;
        }
        if buf.len() > 64 * 1024 {
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

    let body = match find_body_start(&buf) {
        Some(i) => &buf[i..],
        None => return write_http(&mut stream, 400, "{\"error\":\"bad_request\"}"),
    };
    let req_json: AllocateRequest = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return write_http(&mut stream, 400, "{\"error\":\"invalid_json\"}"),
    };

    let claims = match state.verify_allocation(&req_json.allocation_token) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("allocate deny: {e:#}");
            return write_http(&mut stream, 401, "{\"error\":\"invalid_allocation_token\"}");
        }
    };

    let (cert_pem, key_pem, ca_pem) = match state.issue_cert(&claims.role) {
        Ok(v) => v,
        Err(e) => {
            log::error!("cert issue: {e:#}");
            return write_http(&mut stream, 500, "{\"error\":\"cert_issue_failed\"}");
        }
    };

    let cn = if claims.role == "host" {
        CN_HOST
    } else {
        CN_GUEST
    };
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
    let body = serde_json::to_string(&resp)?;
    write_http(&mut stream, 200, &body)
}

fn content_length(buf: &[u8]) -> Option<usize> {
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

fn find_body_start(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

fn write_http(stream: &mut TcpStream, status: u16, body: &str) -> Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        413 => "Payload Too Large",
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
