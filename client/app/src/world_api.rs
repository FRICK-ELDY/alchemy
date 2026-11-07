//! world-server Room HTTP helpers (x0037 + x007c catalog).
//!
//! Guarantees: create-with-visibility; owner visibility update; reports soft-skip distinctly;
//! catalog list returns visible rooms only (no room_key / Token / wire_cap).
//! Non-guarantees: room_id rename / Zenoh resubscribe; display-name persistence;
//! authoritative host_status in catalog.
//!
//! Ownership note (x0021 / x0037): host normally `POST /rooms` first (same account as client).
//! Client may also `POST` when host has not created yet — then host must use the **same**
//! auth identity or host Token will 403. Prefer launcher order: host up → client Start.

use anyhow::{bail, Context, Result};
use serde::Deserialize;

pub use crate::access_level::AccessLevel;

/// One visible room from `GET /api/v1/rooms` (x007c).
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct RoomSummary {
    pub room_id: String,
    pub visibility: String,
    pub owner_sub: String,
    pub role_hint: String,
}

#[derive(Debug, Deserialize)]
struct RoomsListResponse {
    rooms: Vec<RoomSummary>,
}

/// List rooms visible to the bearer (x007c). `mine_only` → `?mine=1`.
pub fn list_rooms(world_url: &str, access_token: &str, mine_only: bool) -> Result<Vec<RoomSummary>> {
    let base = world_url.trim_end_matches('/');
    let authz = format!("Bearer {access_token}");
    let url = if mine_only {
        format!("{base}/api/v1/rooms?mine=1")
    } else {
        format!("{base}/api/v1/rooms")
    };

    match ureq::get(&url).set("authorization", &authz).call() {
        Ok(resp) => {
            let body: RoomsListResponse = resp.into_json().context("parse rooms list")?;
            Ok(body.rooms)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            bail!("list rooms failed status={code} body={body}")
        }
        Err(err) => Err(err).context("list rooms request failed"),
    }
}

/// Result of ensure-create / owner PUT (before join).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisibilityEnsure {
    /// Created room or owner PUT applied the requested level.
    Applied,
    /// Room existed and caller is not owner (or missing): existing ACL unchanged.
    LeftExisting,
}

/// Ensure room exists with requested `visibility`.
///
/// - `201` create → [`VisibilityEnsure::Applied`]
/// - `409` + owner PUT ok → [`VisibilityEnsure::Applied`]
/// - `409` + PUT 403/404 → [`VisibilityEnsure::LeftExisting`] (join may still proceed)
pub fn ensure_room_visibility(
    world_url: &str,
    room_id: &str,
    access_token: &str,
    level: AccessLevel,
) -> Result<VisibilityEnsure> {
    let base = world_url.trim_end_matches('/');
    let authz = format!("Bearer {access_token}");
    let visibility = level.as_str();
    let create_url = format!("{base}/api/v1/rooms");

    match ureq::post(&create_url)
        .set("content-type", "application/json")
        .set("authorization", &authz)
        .send_json(serde_json::json!({
            "room_id": room_id,
            "visibility": visibility,
        })) {
        Ok(_) => {
            log::info!("created room={room_id} visibility={visibility}");
            Ok(VisibilityEnsure::Applied)
        }
        Err(ureq::Error::Status(409, _)) => {
            log::info!("room={room_id} exists; updating visibility if owner");
            set_visibility(world_url, room_id, access_token, level)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            bail!("create room failed status={code} body={body}")
        }
        Err(err) => Err(err).context("create room request failed"),
    }
}

fn set_visibility(
    world_url: &str,
    room_id: &str,
    access_token: &str,
    level: AccessLevel,
) -> Result<VisibilityEnsure> {
    let base = world_url.trim_end_matches('/');
    let authz = format!("Bearer {access_token}");
    let visibility = level.as_str();
    let url = format!("{base}/api/v1/rooms/{room_id}/visibility");

    match ureq::put(&url)
        .set("content-type", "application/json")
        .set("authorization", &authz)
        .send_json(serde_json::json!({ "visibility": visibility }))
    {
        Ok(_) => {
            log::info!("updated room={room_id} visibility={visibility}");
            Ok(VisibilityEnsure::Applied)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            // 2xx arrives as Ok(_); Status path only soft-skips 403/404.
            match map_put_visibility_http_status(code) {
                Ok(VisibilityEnsure::LeftExisting) => {
                    log::info!(
                        "visibility unchanged (not owner or missing); join will use current ACL"
                    );
                    Ok(VisibilityEnsure::LeftExisting)
                }
                Ok(VisibilityEnsure::Applied) | Err(_) => {
                    bail!("set visibility failed status={code} body={body}")
                }
            }
        }
        Err(err) => Err(err).context("set visibility request failed"),
    }
}

/// Pure mapping for owner PUT **error** statuses (unit-tested; no HTTP).
/// Success (2xx) is handled on the `Ok(_)` ureq path, not here.
fn map_put_visibility_http_status(status: u16) -> Result<VisibilityEnsure> {
    match status {
        403 | 404 => Ok(VisibilityEnsure::LeftExisting),
        code => bail!("set visibility failed status={code}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_ensure_variants_are_distinct() {
        assert_ne!(VisibilityEnsure::Applied, VisibilityEnsure::LeftExisting);
        assert_eq!(AccessLevel::Public.as_str(), "public");
    }

    #[test]
    fn put_error_status_maps_soft_skip_only() {
        assert_eq!(
            map_put_visibility_http_status(403).unwrap(),
            VisibilityEnsure::LeftExisting
        );
        assert_eq!(
            map_put_visibility_http_status(404).unwrap(),
            VisibilityEnsure::LeftExisting
        );
        assert!(map_put_visibility_http_status(200).is_err());
        assert!(map_put_visibility_http_status(500).is_err());
        assert!(map_put_visibility_http_status(422).is_err());
    }
}
