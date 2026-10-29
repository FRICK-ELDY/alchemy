//! Relay Agent allocate client (x004e).
//!
//! Guarantees: POST Allocation Token → write cert/key PEM files under a work dir.
//! Non-guarantees: agent discovery; refresh before cert expiry.

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

/// When `RELAY_ALLOCATE_URL` is set, exchange `allocation_token` for mTLS files.
///
/// Returns `Ok(None)` when the env is unset/empty (local tcp path).
pub fn allocate_if_configured(allocation_token: &str) -> Result<Option<AllocatedIdentity>> {
    let Ok(url) = std::env::var("RELAY_ALLOCATE_URL") else {
        return Ok(None);
    };
    if url.is_empty() {
        return Ok(None);
    }
    allocate(&url, allocation_token).map(Some)
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
    let cert_path = work.join(format!("client-{stamp}.cert.pem"));
    let key_path = work.join(format!("client-{stamp}.key.pem"));
    write_secret(&cert_path, &resp.cert_pem)?;
    write_secret(&key_path, &resp.key_pem)?;

    Ok(AllocatedIdentity {
        cert_path,
        key_path,
        wire_cap: resp.wire_cap,
        role: resp.role,
        cn: resp.cn,
        expires_in: resp.expires_in,
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
