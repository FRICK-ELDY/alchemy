//! PlaceRead JWT client (x006c / x006b).
//!
//! Guarantees: `POST …/place_read` with Room Token + `world_seal`; returns short-lived
//! PlaceRead for assets GET (`x-alchemy-place-read`).
//! Non-guarantees: proactive TTL refresh UI; `extra_blake3` take-out path.

use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;

/// Match `assets_http_agent` connect budget in `main.rs` (5s). Keep policy here explicit so
/// a future PlaceRead TTL re-issue path does not fork timeouts by accident.
const PLACE_READ_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// JSON issue only — shorter than assets blob `timeout_read` (30s). No R2 redirect hop.
const PLACE_READ_READ_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Deserialize)]
struct PlaceReadResponse {
    place_read: String,
    #[serde(default)]
    expires_in: u64,
}

fn agent() -> ureq::Agent {
    // World JSON API: no cross-host redirect follow (assets uses redirects(5) for R2 302).
    ureq::AgentBuilder::new()
        .timeout_connect(PLACE_READ_CONNECT_TIMEOUT)
        .timeout_read(PLACE_READ_READ_TIMEOUT)
        .redirects(0)
        .build()
}

/// Issue PlaceRead for the joined room using the verified Manifest `world_seal`.
pub fn fetch_place_read(
    world_url: &str,
    room_id: &str,
    room_token: &str,
    world_seal: &str,
) -> Result<String> {
    let base = world_url.trim_end_matches('/');
    let url = format!("{base}/api/v1/rooms/{room_id}/place_read");
    let authz = format!("Bearer {room_token}");
    let body = serde_json::json!({ "world_seal": world_seal });
    let resp: PlaceReadResponse = agent()
        .post(&url)
        .set("content-type", "application/json")
        .set("authorization", &authz)
        .send_json(body)
        .context("issue PlaceRead (Room Token + world_seal)")?
        .into_json()
        .context("parse PlaceRead response")?;
    if resp.place_read.is_empty() {
        anyhow::bail!("PlaceRead response missing place_read");
    }
    if resp.expires_in == 0 {
        log::warn!(
            "PlaceRead response expires_in=0; treating token as usable until assets rejects"
        );
    }
    Ok(resp.place_read)
}
