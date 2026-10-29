//! Guest Room Token HTTP client (x0021 / x003e / x004e).
//!
//! Guarantees: `POST …/tokens` includes `session_pub`; returns `wire_cap` + `allocation_token`.
//! Non-guarantees: debug `ALCHEMY_ROOM_TOKEN` bypass cnf alignment.

use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;

/// Bound for world Token HTTP (Manifest sync path must not hang indefinitely).
const TOKEN_HTTP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Deserialize)]
struct TokenResponse {
    token: String,
    /// Required from world-server; missing field fails deserialize (no silent `0`).
    expires_in: u64,
    /// Empty when talking to a pre-x004e world; `require_wire_cap` fails closed (or WIRE_CAP env).
    #[serde(default)]
    wire_cap: String,
    /// Empty when Relay is unused / old world; allocate is skipped.
    #[serde(default)]
    allocation_token: String,
}

#[derive(Debug, Clone)]
pub struct GuestToken {
    pub token: String,
    pub expires_in: u64,
    pub wire_cap: String,
    pub allocation_token: String,
}

fn token_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(TOKEN_HTTP_TIMEOUT)
        .timeout_read(TOKEN_HTTP_TIMEOUT)
        .build()
}

/// Issue a guest Room Token bound to `session_pub_b64` (`cnf.spk`, x003e).
pub fn fetch_guest_token(
    world_url: &str,
    room_id: &str,
    auth_token: &str,
    session_pub_b64: &str,
) -> Result<GuestToken> {
    // x0025: env bypass is debug-only (release fail-closed).
    // x003e: bypass JWT must include cnf.spk for this session_pub or host bind fails.
    if cfg!(debug_assertions) {
        if let Ok(token) = std::env::var("ALCHEMY_ROOM_TOKEN") {
            if !token.is_empty() {
                let wire_cap = zenoh_connect::require_wire_cap(None)
                    .unwrap_or_else(|_| alchemy_protocol::LOCAL_WIRE_CAP.to_string());
                return Ok(GuestToken {
                    token,
                    // Env override: no TTL → skip auto-refresh.
                    expires_in: 0,
                    wire_cap,
                    allocation_token: String::new(),
                });
            }
        }
    }

    // x0021: do not POST /rooms — owner is claimed only by host create.
    let base = world_url.trim_end_matches('/');
    let authz = format!("Bearer {auth_token}");
    let token_url = format!("{base}/api/v1/rooms/{room_id}/tokens");
    let resp: TokenResponse = token_agent()
        .post(&token_url)
        .set("content-type", "application/json")
        .set("authorization", &authz)
        .send_json(serde_json::json!({
            "role": "guest",
            "session_pub": session_pub_b64,
        }))
        .context("issue guest Room Token (is host up and room created?)")?
        .into_json()
        .context("parse token response")?;
    if resp.expires_in == 0 {
        anyhow::bail!("guest Room Token response missing positive expires_in");
    }
    let wire_cap = zenoh_connect::require_wire_cap(Some(resp.wire_cap.as_str()))
        .context("guest token wire_cap")?;
    Ok(GuestToken {
        token: resp.token,
        expires_in: resp.expires_in,
        wire_cap,
        allocation_token: resp.allocation_token,
    })
}
