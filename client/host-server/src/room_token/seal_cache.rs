//! World ManifestSeal fetch + cache for unchanged-body republish (x0026 / x0039).
//!
//! Guarantees: seal request carries canonical blake3 + `host_attestation` (world rejects mismatch);
//! empty seal / zero TTL responses are errors.
//! Non-guarantees: Manifest construction (see `credential_worker`).

use super::http::{credential_agent, is_blake3_hex};
use super::timing::{hard_expire_or_hint, refresh_deadline, BYPASS_ROOM_TOKEN_TTL_HINT};
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::time::{Duration, Instant};

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
        Err(ureq::Error::Status(409, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            // x0080: Room Token may still verify — lost compute-host race.
            Err(anyhow!(
                "host_lease_conflict (409): stop hosting / wait for takeover; body={body}"
            ))
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
