//! Relay Agent allocate client (x004e) + world-issued PEM install (x0066).
//!
//! Guarantees: POST Allocation Token → write cert/key PEM files under a work dir;
//! world `mtls` PEMs + in-memory key install the same way (short-lived 0600 files).
//! Non-guarantees: agent discovery; refresh before cert expiry; zero disk for Zenoh paths.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const ALLOCATE_HTTP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Deserialize)]
pub struct AllocateResponse {
    pub role: String,
    #[allow(dead_code)]
    pub room_id: String,
    pub wire_cap: String,
    pub cert_pem: String,
    pub key_pem: String,
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

/// True when `ZENOH_TLS_CONNECT_CERT` / `KEY` already point at an identity (e.g. world mtls).
pub fn mtls_env_ready() -> bool {
    let cert = std::env::var("ZENOH_TLS_CONNECT_CERT").unwrap_or_default();
    let key = std::env::var("ZENOH_TLS_CONNECT_KEY").unwrap_or_default();
    !cert.is_empty() && !key.is_empty()
}

/// Install world-issued cert + caller-held private key as short-lived PEM files and env.
///
/// Zenoh 1.x connect_certificate expects paths; files are 0600 under `RELAY_MTLS_WORK_DIR`
/// (or temp/`alchemy-mtls`). Not persisted as shipping assets.
pub fn install_world_mtls_pems(
    client_cert_pem: &str,
    private_key_pem: &str,
    wire_cap: &str,
    role: &str,
) -> Result<AllocatedIdentity> {
    let id = write_identity_pems(client_cert_pem, private_key_pem, wire_cap, role, "world-mtls")?;
    std::env::set_var("ZENOH_TLS_CONNECT_CERT", id.cert_path.display().to_string());
    std::env::set_var("ZENOH_TLS_CONNECT_KEY", id.key_path.display().to_string());
    Ok(id)
}

/// Prefer world-issued cert + in-memory key; else Relay Agent allocate (x0066 / x004e).
pub fn apply_world_mtls_or_allocate(
    client_cert_pem: Option<&str>,
    private_key_pem: Option<&str>,
    wire_cap: &str,
    role: &str,
    allocation_token: &str,
) -> Result<Option<AllocatedIdentity>> {
    if let (Some(cert), Some(key)) = (client_cert_pem, private_key_pem) {
        if !cert.is_empty() && !key.is_empty() {
            return install_world_mtls_pems(cert, key, wire_cap, role).map(Some);
        }
    }
    allocate_if_configured(allocation_token)
}

/// When `RELAY_ALLOCATE_URL` is set, exchange `allocation_token` for mTLS files.
///
/// Returns `Ok(None)` when the env is unset/empty (local tcp path), or when world
/// mTLS env is already installed (x0066 preferred over Relay Agent).
pub fn allocate_if_configured(allocation_token: &str) -> Result<Option<AllocatedIdentity>> {
    if mtls_env_ready() {
        log::info!("skip relay allocate: ZENOH_TLS_CONNECT_* already set (world mtls)");
        return Ok(None);
    }
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
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(ALLOCATE_HTTP_TIMEOUT)
        .timeout_read(ALLOCATE_HTTP_TIMEOUT)
        .build();
    let resp: AllocateResponse = agent
        .post(&url)
        .set("content-type", "application/json")
        .send_json(serde_json::json!({ "allocation_token": allocation_token }))
        .context("relay allocate HTTP")?
        .into_json()
        .context("parse allocate response")?;

    write_identity_pems(
        &resp.cert_pem,
        &resp.key_pem,
        &resp.wire_cap,
        &resp.role,
        "client",
    )
    .map(|mut id| {
        id.cn = resp.cn;
        id.expires_in = resp.expires_in;
        id
    })
}

fn write_identity_pems(
    cert_pem: &str,
    key_pem: &str,
    wire_cap: &str,
    role: &str,
    prefix: &str,
) -> Result<AllocatedIdentity> {
    let work = PathBuf::from(std::env::var("RELAY_MTLS_WORK_DIR").unwrap_or_else(|_| {
        std::env::temp_dir()
            .join("alchemy-mtls")
            .display()
            .to_string()
    }));
    fs::create_dir_all(&work).context("mtls work dir")?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let cert_path = work.join(format!("{prefix}-{stamp}.cert.pem"));
    let key_path = work.join(format!("{prefix}-{stamp}.key.pem"));
    write_secret(&cert_path, cert_pem)?;
    write_secret(&key_path, key_pem)?;

    Ok(AllocatedIdentity {
        cert_path,
        key_path,
        wire_cap: wire_cap.to_string(),
        role: role.to_string(),
        cn: String::new(),
        expires_in: 0,
    })
}

fn write_secret(path: &Path, pem: &str) -> Result<()> {
    fs::write(path, pem).with_context(|| format!("write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_request_world_mtls_from_tls_connect() {
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
    fn allocate_skipped_when_mtls_env_ready() {
        let prev_cert = std::env::var("ZENOH_TLS_CONNECT_CERT").ok();
        let prev_key = std::env::var("ZENOH_TLS_CONNECT_KEY").ok();
        let prev_url = std::env::var("RELAY_ALLOCATE_URL").ok();
        std::env::set_var("ZENOH_TLS_CONNECT_CERT", "/tmp/x.pem");
        std::env::set_var("ZENOH_TLS_CONNECT_KEY", "/tmp/y.pem");
        std::env::set_var("RELAY_ALLOCATE_URL", "http://127.0.0.1:9");
        let out = allocate_if_configured("tok").expect("skip");
        assert!(out.is_none());
        match prev_cert {
            Some(v) => std::env::set_var("ZENOH_TLS_CONNECT_CERT", v),
            None => std::env::remove_var("ZENOH_TLS_CONNECT_CERT"),
        }
        match prev_key {
            Some(v) => std::env::set_var("ZENOH_TLS_CONNECT_KEY", v),
            None => std::env::remove_var("ZENOH_TLS_CONNECT_KEY"),
        }
        match prev_url {
            Some(v) => std::env::set_var("RELAY_ALLOCATE_URL", v),
            None => std::env::remove_var("RELAY_ALLOCATE_URL"),
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
