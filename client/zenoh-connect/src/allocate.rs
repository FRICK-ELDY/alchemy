//! Relay Agent allocate client (x004e / P0-4) + world-issued PEM install (x0066).
//!
//! Guarantees: POST Allocation Token + **client CSR** → Agent returns cert only (no `key_pem`);
//! local private key stays on the client; write cert/key PEM files under a work dir;
//! world `mtls` PEMs + in-memory key install the same way (short-lived owner-only files);
//! Unix `0600` / Windows user-only DACL (x0078); PEM bytes are written only after restrict;
//! replace + stale sweep of managed `{client,world-mtls}-*.{cert,key}.pem` only;
//! **both** paths set `ZENOH_TLS_CONNECT_CERT` / `KEY` (Agent 併存に必須).
//! Non-guarantees: agent discovery; soft TTL enforcement; zero disk for Zenoh paths;
//! DPAPI/TPM; create-with-ACL atomicity (empty placeholder may exist briefly before restrict);
//! exclusive locking of a shared work dir across processes.

use crate::ephemeral::EphemeralMtlsIdentity;
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const ALLOCATE_HTTP_TIMEOUT: Duration = Duration::from_secs(5);
/// Fallback soft TTL when `mtls.expires_at` is missing/unparseable (x0068).
pub const WORLD_MTLS_SOFT_TTL_SECS: u64 = 300;
/// Orphan managed PEM GC threshold: soft TTL + clock/skew grace (x0078).
const MTLS_TEMP_STALE_SECS: u64 = WORLD_MTLS_SOFT_TTL_SECS + 120;
/// Filename prefix for Relay Agent allocate PEMs (`{prefix}-{stamp}.key.pem`).
const MTLS_PEM_PREFIX_CLIENT: &str = "client";
/// Filename prefix for world-issued PEMs.
const MTLS_PEM_PREFIX_WORLD: &str = "world-mtls";
/// Sweep / replace only touch names under these prefixes (keep in sync with writers).
const MANAGED_MTLS_PEM_PREFIXES: &[&str] = &[MTLS_PEM_PREFIX_CLIENT, MTLS_PEM_PREFIX_WORLD];

/// Parse world `mtls.expires_at` (ISO-8601 / RFC3339 UTC) into remaining seconds.
pub fn soft_ttl_secs_from_expires_at(expires_at: &str) -> Option<u64> {
    let end = parse_rfc3339_unix(expires_at)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(end.saturating_sub(now).max(1))
}

fn parse_rfc3339_unix(s: &str) -> Option<u64> {
    // World `DateTime.to_iso8601` emits UTC with `Z` (second truncation). Only `Z` / `+00:00`
    // are accepted; other offsets fall through to WORLD_MTLS_SOFT_TTL_SECS.
    let s = s.trim();
    let s = s.strip_suffix('Z').or_else(|| s.strip_suffix("+00:00"))?;
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-');
    let y: i32 = d.next()?.parse().ok()?;
    let m: u64 = d.next()?.parse().ok()?;
    let day: u64 = d.next()?.parse().ok()?;
    let time = time.split('.').next()?;
    let mut t = time.split(':');
    let hh: u64 = t.next()?.parse().ok()?;
    let mm: u64 = t.next()?.parse().ok()?;
    let ss: u64 = t.next()?.parse().ok()?;
    Some(civil_to_unix(y, m, day, hh, mm, ss))
}

fn civil_to_unix(y: i32, m: u64, d: u64, hh: u64, mm: u64, ss: u64) -> u64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = (era as i64) * 146_097 + (doe as i64) - 719_468;
    (days as u64) * 86_400 + hh * 3600 + mm * 60 + ss
}

#[derive(Debug, Deserialize)]
pub struct AllocateResponse {
    pub role: String,
    #[allow(dead_code)]
    pub room_id: String,
    pub wire_cap: String,
    pub cert_pem: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub ca_pem: String,
    pub expires_in: u64,
    pub cn: String,
}

#[derive(Debug, Clone)]
pub struct AllocatedIdentity {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    pub wire_cap: String,
    pub role: String,
    pub cn: String,
    pub expires_in: u64,
}

/// True when `ZENOH_TLS_CONNECT_CERT` / `KEY` already point at an identity.
pub fn mtls_env_ready() -> bool {
    let cert = std::env::var("ZENOH_TLS_CONNECT_CERT").unwrap_or_default();
    let key = std::env::var("ZENOH_TLS_CONNECT_KEY").unwrap_or_default();
    !cert.is_empty() && !key.is_empty()
}

fn role_cn(role: &str) -> String {
    match role {
        "host" => "alchemy-host".into(),
        "guest" => "alchemy-guest".into(),
        other => other.to_string(),
    }
}

fn mtls_work_dir() -> PathBuf {
    PathBuf::from(std::env::var("RELAY_MTLS_WORK_DIR").unwrap_or_else(|_| {
        std::env::temp_dir()
            .join("alchemy-mtls")
            .display()
            .to_string()
    }))
}

/// True only for short-lived PEMs we wrote under the mtls work dir (never static cert paths).
fn is_managed_mtls_path(path: &Path) -> bool {
    let work = mtls_work_dir();
    if path.starts_with(&work) {
        return true;
    }
    match (work.canonicalize(), path.canonicalize()) {
        (Ok(work), Ok(canon)) => canon.starts_with(&work),
        _ => false,
    }
}

/// Point Zenoh env at `id`, removing previous **managed** short-lived PEM paths when replaced.
fn install_identity_env(id: &AllocatedIdentity) {
    let prev_cert = std::env::var("ZENOH_TLS_CONNECT_CERT").ok();
    let prev_key = std::env::var("ZENOH_TLS_CONNECT_KEY").ok();
    std::env::set_var("ZENOH_TLS_CONNECT_CERT", id.cert_path.display().to_string());
    std::env::set_var("ZENOH_TLS_CONNECT_KEY", id.key_path.display().to_string());
    for prev in [prev_cert, prev_key].into_iter().flatten() {
        let path = PathBuf::from(&prev);
        if path != id.cert_path && path != id.key_path && is_managed_mtls_path(&path) {
            let _ = fs::remove_file(&path);
        }
    }
}

/// Install world-issued cert + caller-held private key as short-lived PEM files and env.
///
/// Zenoh 1.x connect_certificate expects paths; files are owner-only under
/// `RELAY_MTLS_WORK_DIR` (or temp/`alchemy-mtls`). Not persisted as shipping assets.
pub fn install_world_mtls_pems(
    client_cert_pem: &str,
    private_key_pem: &str,
    wire_cap: &str,
    role: &str,
    soft_ttl_secs: Option<u64>,
) -> Result<AllocatedIdentity> {
    let ttl = soft_ttl_secs.unwrap_or(WORLD_MTLS_SOFT_TTL_SECS);
    let id = write_identity_pems(
        client_cert_pem,
        private_key_pem,
        wire_cap,
        role,
        MTLS_PEM_PREFIX_WORLD,
        &role_cn(role),
        ttl,
    )?;
    install_identity_env(&id);
    Ok(id)
}

/// Prefer world-issued cert + in-memory key; else Relay Agent allocate (x0066 / x004e).
///
/// Both branches install `ZENOH_TLS_CONNECT_*` when they produce an identity.
/// `mtls_expires_at` (ISO-8601) sets soft TTL when present (x0068).
pub fn apply_world_mtls_or_allocate(
    client_cert_pem: Option<&str>,
    private_key_pem: Option<&str>,
    wire_cap: &str,
    role: &str,
    allocation_token: &str,
    mtls_expires_at: Option<&str>,
) -> Result<Option<AllocatedIdentity>> {
    if let (Some(cert), Some(key)) = (client_cert_pem, private_key_pem) {
        if !cert.is_empty() && !key.is_empty() {
            let ttl = mtls_expires_at.and_then(soft_ttl_secs_from_expires_at);
            return install_world_mtls_pems(cert, key, wire_cap, role, ttl).map(Some);
        }
    }
    allocate_if_configured(allocation_token)
}

/// When `RELAY_ALLOCATE_URL` is set, exchange `allocation_token` for mTLS files + env.
///
/// Returns `Ok(None)` when the env is unset/empty (local tcp path).
/// Does **not** skip when `ZENOH_TLS_CONNECT_*` is already set — refresh must replace PEMs.
/// Prefer [`apply_world_mtls_or_allocate`] when world may have issued certs first.
pub fn allocate_if_configured(allocation_token: &str) -> Result<Option<AllocatedIdentity>> {
    let Ok(url) = std::env::var("RELAY_ALLOCATE_URL") else {
        return Ok(None);
    };
    if url.is_empty() {
        return Ok(None);
    }
    allocate(&url, allocation_token).map(Some)
}

/// Request a CSR from world when WAN TLS is in use (`ZENOH_CONNECT=tls/…` or `WORLD_MTLS=1`).
pub fn should_request_world_mtls() -> bool {
    match std::env::var("WORLD_MTLS") {
        Ok(v) if matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES") => return true,
        _ => {}
    }
    std::env::var("ZENOH_CONNECT")
        .unwrap_or_default()
        .starts_with("tls/")
}

pub fn allocate(allocate_url: &str, allocation_token: &str) -> Result<AllocatedIdentity> {
    let base = allocate_url.trim_end_matches('/');
    let url = format!("{base}/v1/allocate");
    // P0-4: key stays local; Agent signs CSR only (same shape as world mtls).
    let eph = EphemeralMtlsIdentity::generate().context("ephemeral CSR for relay allocate")?;
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(ALLOCATE_HTTP_TIMEOUT)
        .timeout_read(ALLOCATE_HTTP_TIMEOUT)
        .build();
    let resp: AllocateResponse = agent
        .post(&url)
        .set("content-type", "application/json")
        .send_json(serde_json::json!({
            "allocation_token": allocation_token,
            "csr_pem": eph.csr_pem,
        }))
        .context("relay allocate HTTP")?
        .into_json()
        .context("parse allocate response")?;

    let id = write_identity_pems(
        &resp.cert_pem,
        eph.private_key_pem(),
        &resp.wire_cap,
        &resp.role,
        MTLS_PEM_PREFIX_CLIENT,
        &resp.cn,
        resp.expires_in,
    )?;
    install_identity_env(&id);
    Ok(id)
}

fn write_identity_pems(
    cert_pem: &str,
    key_pem: &str,
    wire_cap: &str,
    role: &str,
    prefix: &str,
    cn: &str,
    expires_in: u64,
) -> Result<AllocatedIdentity> {
    let work = mtls_work_dir();
    fs::create_dir_all(&work).context("mtls work dir")?;
    let stamp = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let cert_path = work.join(format!("{prefix}-{stamp}.cert.pem"));
    let key_path = work.join(format!("{prefix}-{stamp}.key.pem"));
    write_secret(&cert_path, cert_pem)?;
    if let Err(err) = write_secret(&key_path, key_pem) {
        let _ = fs::remove_file(&cert_path);
        return Err(err);
    }
    // Best-effort: crash/orphan managed PEMs from prior runs (x0078). Keep the files we just wrote.
    sweep_stale_mtls_temps(&work, &[&cert_path, &key_path]);

    Ok(AllocatedIdentity {
        cert_path,
        key_path,
        wire_cap: wire_cap.to_string(),
        role: role.to_string(),
        cn: cn.to_string(),
        expires_in,
    })
}

/// Create empty → restrict → write PEM so secret bytes never sit on a default/loose ACL.
fn write_secret(path: &Path, pem: &str) -> Result<()> {
    fs::File::create(path).with_context(|| format!("create {}", path.display()))?;
    if let Err(err) = restrict_secret_permissions(path) {
        let _ = fs::remove_file(path);
        return Err(err).with_context(|| format!("restrict {}", path.display()));
    }
    if let Err(err) = fs::write(path, pem) {
        let _ = fs::remove_file(path);
        return Err(err).with_context(|| format!("write {}", path.display()));
    }
    Ok(())
}

/// Owner-only access: Unix `0600`; Windows current-user DACL via `icacls` (x0078).
fn restrict_secret_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)
            .with_context(|| format!("stat {}", path.display()))?
            .permissions();
        perms.set_mode(0o600);
        fs::set_permissions(path, perms)
            .with_context(|| format!("chmod 0600 {}", path.display()))?;
    }
    #[cfg(windows)]
    {
        restrict_secret_permissions_windows(path)?;
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        return Err(anyhow!(
            "mTLS PEM permission hardening is unsupported on this OS (fail-closed)"
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn restrict_secret_permissions_windows(path: &Path) -> Result<()> {
    use std::process::Command;

    let user = windows_acl_principal()?;
    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow!("mTLS PEM path is not valid UTF-8 for icacls"))?;
    let grant = format!("{user}:(R,W)");
    let output = Command::new("icacls")
        .args([path_str, "/inheritance:r", "/grant:r", &grant])
        .output()
        .context("spawn icacls")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(anyhow!(
            "icacls failed status={:?}: {}{}",
            output.status.code(),
            stdout.trim(),
            stderr.trim()
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn windows_acl_principal() -> Result<String> {
    let name = std::env::var("USERNAME").unwrap_or_default();
    let name = name.trim();
    if name.is_empty() {
        return Err(anyhow!(
            "USERNAME unset — cannot tighten mTLS PEM ACL on Windows"
        ));
    }
    let domain = std::env::var("USERDOMAIN").unwrap_or_default();
    let domain = domain.trim();
    if domain.is_empty() {
        Ok(name.to_string())
    } else {
        Ok(format!("{domain}\\{name}"))
    }
}

/// True for names we write: `{prefix}-*.{cert|key}.pem` under [`MANAGED_MTLS_PEM_PREFIXES`].
fn is_managed_mtls_pem_name(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    let managed_prefix = MANAGED_MTLS_PEM_PREFIXES.iter().any(|p| {
        lower
            .strip_prefix(p)
            .is_some_and(|rest| rest.starts_with('-'))
    });
    managed_prefix && (lower.ends_with(".cert.pem") || lower.ends_with(".key.pem"))
}

/// Remove managed identity PEMs under `work` older than [`MTLS_TEMP_STALE_SECS`], keeping `keep`.
/// Non-managed names (e.g. CA PEMs colocated by mistake) are never deleted.
fn sweep_stale_mtls_temps(work: &Path, keep: &[&Path]) {
    let Ok(entries) = fs::read_dir(work) else {
        return;
    };
    let now = SystemTime::now();
    let stale = Duration::from_secs(MTLS_TEMP_STALE_SECS);
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_managed_mtls_pem_name(&path) {
            continue;
        }
        if keep.contains(&path.as_path()) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        let aged_out = now
            .duration_since(modified)
            .map(|age| age >= stale)
            .unwrap_or(false);
        if aged_out {
            let _ = fs::remove_file(&path);
        }
    }
}

/// Resolve wire_cap: prefer token field, else `WIRE_CAP` env, else error.
pub fn require_wire_cap(from_token: Option<&str>) -> Result<String> {
    if let Some(cap) = from_token {
        if !cap.is_empty() {
            alchemy_protocol::validate_wire_cap(cap)
                .map_err(|e| anyhow!("wire_cap from token: {e}"))?;
            return Ok(cap.to_string());
        }
    }
    let env_cap = std::env::var("WIRE_CAP").unwrap_or_default();
    if !env_cap.is_empty() {
        alchemy_protocol::validate_wire_cap(&env_cap).map_err(|e| anyhow!("WIRE_CAP env: {e}"))?;
        return Ok(env_cap);
    }
    Err(anyhow!(
        "wire_cap missing (token response or WIRE_CAP env required; x004e)"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{ErrorKind, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    /// Env-mutating tests must not run in parallel with each other.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Restores env keys (and optional work dir cleanup) on drop — including panic paths.
    struct EnvRestore {
        entries: Vec<(&'static str, Option<String>)>,
        work_dir: Option<PathBuf>,
    }

    impl EnvRestore {
        fn capture(keys: &[&'static str]) -> Self {
            Self {
                entries: keys.iter().map(|k| (*k, std::env::var(k).ok())).collect(),
                work_dir: None,
            }
        }

        fn set_work_dir(&mut self, path: PathBuf) {
            std::env::set_var("RELAY_MTLS_WORK_DIR", path.display().to_string());
            self.work_dir = Some(path);
        }
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            for (key, prev) in &self.entries {
                match prev {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
            if let Some(work) = self.work_dir.take() {
                let _ = fs::remove_dir_all(work);
            }
        }
    }

    fn http_request_complete(buf: &[u8]) -> bool {
        let Some(start) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
            return false;
        };
        let Ok(headers) = std::str::from_utf8(&buf[..start]) else {
            return false;
        };
        match headers.lines().find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .and_then(|v| v.trim().parse::<usize>().ok())
        }) {
            Some(cl) => buf.len() >= start + 4 + cl,
            // No Content-Length: respond after headers to avoid client/server deadlock.
            None => true,
        }
    }

    /// P0-4: client must send `csr_pem`, ignore any legacy `key_pem`, and install the local key.
    #[test]
    fn allocate_sends_csr_and_installs_local_key_without_response_key_pem() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut env = EnvRestore::capture(&[
            "ZENOH_TLS_CONNECT_CERT",
            "ZENOH_TLS_CONNECT_KEY",
            "RELAY_MTLS_WORK_DIR",
        ]);
        let stamp = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let work = std::env::temp_dir().join(format!("alchemy-mtls-test-allocate-csr-{stamp}"));
        env.set_work_dir(work);

        let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let captured_req = Arc::clone(&captured);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
            let mut buf = Vec::new();
            let mut chunk = [0u8; 2048];
            loop {
                if buf.len() > 64 * 1024 {
                    break;
                }
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&chunk[..n]);
                        if http_request_complete(&buf) {
                            break;
                        }
                    }
                    Err(e)
                        if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut =>
                    {
                        break;
                    }
                    Err(_) => break,
                }
            }
            let req = String::from_utf8_lossy(&buf).into_owned();
            *captured_req.lock().unwrap() = Some(req);
            // Deliberately include legacy `key_pem` — client must not use it (P0-4).
            let body = serde_json::json!({
                "role": "guest",
                "room_id": "r1",
                "wire_cap": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "cert_pem": "-----BEGIN CERTIFICATE-----\nCSRONLY\n-----END CERTIFICATE-----\n",
                "key_pem": "-----BEGIN PRIVATE KEY-----\nLEGACY-SERVER-KEY\n-----END PRIVATE KEY-----\n",
                "ca_pem": "-----BEGIN CERTIFICATE-----\nCA\n-----END CERTIFICATE-----\n",
                "expires_in": 300,
                "cn": "alchemy-guest",
            })
            .to_string();
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(body.as_bytes());
        });

        let id = allocate(&format!("http://{addr}"), "alloc-token-1").expect("allocate");
        let req = captured.lock().unwrap().clone().expect("captured request");
        assert!(
            req.contains("\"csr_pem\""),
            "request must include csr_pem: {req}"
        );
        assert!(
            req.contains("BEGIN CERTIFICATE REQUEST"),
            "request csr must look like a CSR: {req}"
        );
        assert!(
            req.contains("\"allocation_token\":\"alloc-token-1\""),
            "token missing: {req}"
        );

        let cert = fs::read_to_string(&id.cert_path).unwrap();
        let key = fs::read_to_string(&id.key_path).unwrap();
        assert!(cert.contains("CSRONLY"), "installed cert={cert}");
        assert!(
            !key.contains("LEGACY-SERVER-KEY"),
            "must not install response key_pem: {key}"
        );
        assert!(
            key.contains("BEGIN PRIVATE KEY") || key.contains("BEGIN EC PRIVATE KEY"),
            "must install local ephemeral key: {key}"
        );
        assert_eq!(id.cn, "alchemy-guest");
        // `env` Drop restores TLS/work env and removes work dir (including panic paths).
        drop(env);
    }

    #[test]
    fn should_request_world_mtls_from_tls_connect() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev_c = std::env::var("ZENOH_CONNECT").ok();
        let prev_w = std::env::var("WORLD_MTLS").ok();
        std::env::remove_var("WORLD_MTLS");
        std::env::set_var("ZENOH_CONNECT", "tls/example.com:7447");
        assert!(should_request_world_mtls());
        std::env::set_var("ZENOH_CONNECT", "tcp/127.0.0.1:7447");
        assert!(!should_request_world_mtls());
        std::env::set_var("WORLD_MTLS", "1");
        assert!(should_request_world_mtls());
        match prev_c {
            Some(v) => std::env::set_var("ZENOH_CONNECT", v),
            None => std::env::remove_var("ZENOH_CONNECT"),
        }
        match prev_w {
            Some(v) => std::env::set_var("WORLD_MTLS", v),
            None => std::env::remove_var("WORLD_MTLS"),
        }
    }

    #[test]
    fn allocate_skipped_when_url_unset() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev_url = std::env::var("RELAY_ALLOCATE_URL").ok();
        let prev_cert = std::env::var("ZENOH_TLS_CONNECT_CERT").ok();
        let prev_key = std::env::var("ZENOH_TLS_CONNECT_KEY").ok();
        std::env::remove_var("RELAY_ALLOCATE_URL");
        std::env::remove_var("ZENOH_TLS_CONNECT_CERT");
        std::env::remove_var("ZENOH_TLS_CONNECT_KEY");
        let out = allocate_if_configured("tok").expect("none");
        assert!(out.is_none());
        match prev_url {
            Some(v) => std::env::set_var("RELAY_ALLOCATE_URL", v),
            None => std::env::remove_var("RELAY_ALLOCATE_URL"),
        }
        match prev_cert {
            Some(v) => std::env::set_var("ZENOH_TLS_CONNECT_CERT", v),
            None => std::env::remove_var("ZENOH_TLS_CONNECT_CERT"),
        }
        match prev_key {
            Some(v) => std::env::set_var("ZENOH_TLS_CONNECT_KEY", v),
            None => std::env::remove_var("ZENOH_TLS_CONNECT_KEY"),
        }
    }

    #[test]
    fn install_world_sets_env_and_cn() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev_cert = std::env::var("ZENOH_TLS_CONNECT_CERT").ok();
        let prev_key = std::env::var("ZENOH_TLS_CONNECT_KEY").ok();
        let prev_work = std::env::var("RELAY_MTLS_WORK_DIR").ok();
        let work = std::env::temp_dir().join(format!(
            "alchemy-mtls-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::env::set_var("RELAY_MTLS_WORK_DIR", work.display().to_string());

        let id = install_world_mtls_pems(
            "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n",
            "-----BEGIN PRIVATE KEY-----\nMIIB\n-----END PRIVATE KEY-----\n",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "guest",
            None,
        )
        .expect("install");
        assert_eq!(id.cn, "alchemy-guest");
        assert_eq!(id.expires_in, WORLD_MTLS_SOFT_TTL_SECS);
        assert_eq!(
            std::env::var("ZENOH_TLS_CONNECT_CERT").unwrap(),
            id.cert_path.display().to_string()
        );
        assert_eq!(
            std::env::var("ZENOH_TLS_CONNECT_KEY").unwrap(),
            id.key_path.display().to_string()
        );
        let _ = fs::remove_dir_all(&work);
        match prev_cert {
            Some(v) => std::env::set_var("ZENOH_TLS_CONNECT_CERT", v),
            None => std::env::remove_var("ZENOH_TLS_CONNECT_CERT"),
        }
        match prev_key {
            Some(v) => std::env::set_var("ZENOH_TLS_CONNECT_KEY", v),
            None => std::env::remove_var("ZENOH_TLS_CONNECT_KEY"),
        }
        match prev_work {
            Some(v) => std::env::set_var("RELAY_MTLS_WORK_DIR", v),
            None => std::env::remove_var("RELAY_MTLS_WORK_DIR"),
        }
    }

    #[test]
    fn install_does_not_delete_static_env_paths() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev_cert = std::env::var("ZENOH_TLS_CONNECT_CERT").ok();
        let prev_key = std::env::var("ZENOH_TLS_CONNECT_KEY").ok();
        let prev_work = std::env::var("RELAY_MTLS_WORK_DIR").ok();

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let static_dir = std::env::temp_dir().join(format!("alchemy-static-mtls-{stamp}"));
        fs::create_dir_all(&static_dir).unwrap();
        let static_cert = static_dir.join("static.cert.pem");
        let static_key = static_dir.join("static.key.pem");
        fs::write(&static_cert, "static-cert").unwrap();
        fs::write(&static_key, "static-key").unwrap();
        std::env::set_var("ZENOH_TLS_CONNECT_CERT", static_cert.display().to_string());
        std::env::set_var("ZENOH_TLS_CONNECT_KEY", static_key.display().to_string());

        let work = std::env::temp_dir().join(format!("alchemy-mtls-test-{stamp}"));
        std::env::set_var("RELAY_MTLS_WORK_DIR", work.display().to_string());

        let _ = install_world_mtls_pems(
            "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n",
            "-----BEGIN PRIVATE KEY-----\nMIIB\n-----END PRIVATE KEY-----\n",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "host",
            Some(120),
        )
        .expect("install");

        assert!(static_cert.is_file(), "static cert must survive");
        assert!(static_key.is_file(), "static key must survive");

        let _ = fs::remove_dir_all(&work);
        let _ = fs::remove_dir_all(&static_dir);
        match prev_cert {
            Some(v) => std::env::set_var("ZENOH_TLS_CONNECT_CERT", v),
            None => std::env::remove_var("ZENOH_TLS_CONNECT_CERT"),
        }
        match prev_key {
            Some(v) => std::env::set_var("ZENOH_TLS_CONNECT_KEY", v),
            None => std::env::remove_var("ZENOH_TLS_CONNECT_KEY"),
        }
        match prev_work {
            Some(v) => std::env::set_var("RELAY_MTLS_WORK_DIR", v),
            None => std::env::remove_var("RELAY_MTLS_WORK_DIR"),
        }
    }

    #[test]
    fn soft_ttl_parses_rfc3339_z() {
        assert_eq!(parse_rfc3339_unix("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339_unix("1970-01-01T00:05:00Z"), Some(300));
        let future = soft_ttl_secs_from_expires_at("2099-01-01T00:00:00Z");
        assert!(future.is_some_and(|s| s > 60));
    }

    #[test]
    fn reinstall_removes_previous_managed_pems() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev_cert = std::env::var("ZENOH_TLS_CONNECT_CERT").ok();
        let prev_key = std::env::var("ZENOH_TLS_CONNECT_KEY").ok();
        let prev_work = std::env::var("RELAY_MTLS_WORK_DIR").ok();
        let stamp = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let work = std::env::temp_dir().join(format!("alchemy-mtls-test-reinstall-{stamp}"));
        std::env::set_var("RELAY_MTLS_WORK_DIR", work.display().to_string());

        let first = install_world_mtls_pems(
            "-----BEGIN CERTIFICATE-----\nFIRST\n-----END CERTIFICATE-----\n",
            "-----BEGIN PRIVATE KEY-----\nFIRST\n-----END PRIVATE KEY-----\n",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "guest",
            Some(60),
        )
        .expect("first install");
        assert!(first.cert_path.is_file());
        assert!(first.key_path.is_file());

        let second = install_world_mtls_pems(
            "-----BEGIN CERTIFICATE-----\nSECOND\n-----END CERTIFICATE-----\n",
            "-----BEGIN PRIVATE KEY-----\nSECOND\n-----END PRIVATE KEY-----\n",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "guest",
            Some(60),
        )
        .expect("second install");
        assert!(second.cert_path.is_file());
        assert!(second.key_path.is_file());
        assert!(
            !first.cert_path.exists(),
            "previous managed cert must be removed"
        );
        assert!(
            !first.key_path.exists(),
            "previous managed key must be removed"
        );

        let _ = fs::remove_dir_all(&work);
        match prev_cert {
            Some(v) => std::env::set_var("ZENOH_TLS_CONNECT_CERT", v),
            None => std::env::remove_var("ZENOH_TLS_CONNECT_CERT"),
        }
        match prev_key {
            Some(v) => std::env::set_var("ZENOH_TLS_CONNECT_KEY", v),
            None => std::env::remove_var("ZENOH_TLS_CONNECT_KEY"),
        }
        match prev_work {
            Some(v) => std::env::set_var("RELAY_MTLS_WORK_DIR", v),
            None => std::env::remove_var("RELAY_MTLS_WORK_DIR"),
        }
    }

    #[test]
    fn sweep_removes_stale_orphan_pems() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev_cert = std::env::var("ZENOH_TLS_CONNECT_CERT").ok();
        let prev_key = std::env::var("ZENOH_TLS_CONNECT_KEY").ok();
        let prev_work = std::env::var("RELAY_MTLS_WORK_DIR").ok();
        let stamp = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let work = std::env::temp_dir().join(format!("alchemy-mtls-test-sweep-{stamp}"));
        fs::create_dir_all(&work).unwrap();
        std::env::set_var("RELAY_MTLS_WORK_DIR", work.display().to_string());

        let aged = SystemTime::now()
            .checked_sub(Duration::from_secs(MTLS_TEMP_STALE_SECS + 30))
            .expect("clock");
        let orphan = work.join("client-orphan-stale.key.pem");
        fs::write(&orphan, "orphan-key").unwrap();
        let file = fs::File::options().write(true).open(&orphan).unwrap();
        file.set_modified(aged).unwrap();
        drop(file);

        let fresh = work.join("world-mtls-orphan-fresh.key.pem");
        fs::write(&fresh, "fresh-key").unwrap();

        // Non-managed name must never be swept even when aged.
        let foreign = work.join("ca-cert.pem");
        fs::write(&foreign, "ca").unwrap();
        let file = fs::File::options().write(true).open(&foreign).unwrap();
        file.set_modified(aged).unwrap();
        drop(file);

        let id = install_world_mtls_pems(
            "-----BEGIN CERTIFICATE-----\nSWEEP\n-----END CERTIFICATE-----\n",
            "-----BEGIN PRIVATE KEY-----\nSWEEP\n-----END PRIVATE KEY-----\n",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "host",
            Some(60),
        )
        .expect("install");

        assert!(!orphan.exists(), "stale managed orphan must be swept");
        assert!(
            fresh.is_file(),
            "fresh managed orphan under stale threshold must remain"
        );
        assert!(foreign.is_file(), "non-managed PEM must not be swept");
        assert!(id.cert_path.is_file());
        assert!(id.key_path.is_file());

        let _ = fs::remove_dir_all(&work);
        match prev_cert {
            Some(v) => std::env::set_var("ZENOH_TLS_CONNECT_CERT", v),
            None => std::env::remove_var("ZENOH_TLS_CONNECT_CERT"),
        }
        match prev_key {
            Some(v) => std::env::set_var("ZENOH_TLS_CONNECT_KEY", v),
            None => std::env::remove_var("ZENOH_TLS_CONNECT_KEY"),
        }
        match prev_work {
            Some(v) => std::env::set_var("RELAY_MTLS_WORK_DIR", v),
            None => std::env::remove_var("RELAY_MTLS_WORK_DIR"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn write_secret_sets_unix_0600() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use std::os::unix::fs::PermissionsExt;
        let prev_cert = std::env::var("ZENOH_TLS_CONNECT_CERT").ok();
        let prev_key = std::env::var("ZENOH_TLS_CONNECT_KEY").ok();
        let prev_work = std::env::var("RELAY_MTLS_WORK_DIR").ok();
        let stamp = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let work = std::env::temp_dir().join(format!("alchemy-mtls-test-mode-{stamp}"));
        std::env::set_var("RELAY_MTLS_WORK_DIR", work.display().to_string());

        let id = install_world_mtls_pems(
            "-----BEGIN CERTIFICATE-----\nMODE\n-----END CERTIFICATE-----\n",
            "-----BEGIN PRIVATE KEY-----\nMODE\n-----END PRIVATE KEY-----\n",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "guest",
            Some(60),
        )
        .expect("install");
        let mode = fs::metadata(&id.key_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        let _ = fs::remove_dir_all(&work);
        match prev_cert {
            Some(v) => std::env::set_var("ZENOH_TLS_CONNECT_CERT", v),
            None => std::env::remove_var("ZENOH_TLS_CONNECT_CERT"),
        }
        match prev_key {
            Some(v) => std::env::set_var("ZENOH_TLS_CONNECT_KEY", v),
            None => std::env::remove_var("ZENOH_TLS_CONNECT_KEY"),
        }
        match prev_work {
            Some(v) => std::env::set_var("RELAY_MTLS_WORK_DIR", v),
            None => std::env::remove_var("RELAY_MTLS_WORK_DIR"),
        }
    }

    #[cfg(windows)]
    #[test]
    fn write_secret_applies_windows_user_dacl() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev_cert = std::env::var("ZENOH_TLS_CONNECT_CERT").ok();
        let prev_key = std::env::var("ZENOH_TLS_CONNECT_KEY").ok();
        let prev_work = std::env::var("RELAY_MTLS_WORK_DIR").ok();
        let stamp = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let work = std::env::temp_dir().join(format!("alchemy-mtls-test-acl-{stamp}"));
        std::env::set_var("RELAY_MTLS_WORK_DIR", work.display().to_string());

        let id = install_world_mtls_pems(
            "-----BEGIN CERTIFICATE-----\nACL\n-----END CERTIFICATE-----\n",
            "-----BEGIN PRIVATE KEY-----\nACL\n-----END PRIVATE KEY-----\n",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "guest",
            Some(60),
        )
        .expect("install with icacls");
        let principal = windows_acl_principal().expect("principal");
        let out = std::process::Command::new("icacls")
            .arg(id.key_path.display().to_string())
            .output()
            .expect("icacls read");
        assert!(out.status.success());
        let listing = String::from_utf8_lossy(&out.stdout);
        let listing_lower = listing.to_ascii_lowercase();
        // Assert DACL content, not icacls line wrapping (long paths may put ACE on the next line).
        assert!(
            !listing.contains("(I)"),
            "expected non-inherited DACL, got:\n{listing}"
        );
        assert!(
            listing_lower.contains(&principal.to_ascii_lowercase()),
            "expected current-user ACE for {principal}, got:\n{listing}"
        );
        assert!(
            listing.contains("(R,W)") || listing.contains("(W,R)"),
            "expected (R,W) for current user, got:\n{listing}"
        );
        for forbidden in [
            "everyone",
            "builtin\\users",
            "builtin\\administrators",
            "nt authority\\system",
        ] {
            assert!(
                !listing_lower.contains(forbidden),
                "{forbidden} must not remain on DACL:\n{listing}"
            );
        }

        let _ = fs::remove_dir_all(&work);
        match prev_cert {
            Some(v) => std::env::set_var("ZENOH_TLS_CONNECT_CERT", v),
            None => std::env::remove_var("ZENOH_TLS_CONNECT_CERT"),
        }
        match prev_key {
            Some(v) => std::env::set_var("ZENOH_TLS_CONNECT_KEY", v),
            None => std::env::remove_var("ZENOH_TLS_CONNECT_KEY"),
        }
        match prev_work {
            Some(v) => std::env::set_var("RELAY_MTLS_WORK_DIR", v),
            None => std::env::remove_var("RELAY_MTLS_WORK_DIR"),
        }
    }
}
