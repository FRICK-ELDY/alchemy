//! Inventory take-out client helpers (x006d).
//!
//! Guarantees: Drive list / take_out over assets HTTP; spawn put on Zenoh
//! `inventory/spawn` with Room Token + blake3 (no Drive node_id on the wire to host).
//! Non-guarantees: put-back; Prefab multi-child; XR Inventory polish.

use alchemy_protocol::inventory_spawn_key;
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::json;
use zenoh::Wait;

#[derive(Debug, Clone, Deserialize)]
pub struct DriveNodeDto {
    pub node_id: String,
    pub kind: String,
    pub name: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub blake3: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NodesResponse {
    nodes: Vec<DriveNodeDto>,
}

#[derive(Debug, Deserialize)]
struct TakeOutResponse {
    node: DriveNodeDto,
    blobs: Vec<BlobRef>,
}

#[derive(Debug, Deserialize)]
struct BlobRef {
    blake3: String,
}

/// List root Drive nodes for the authenticated user.
pub fn list_drive_nodes(assets_url: &str, access_token: &str) -> Result<Vec<DriveNodeDto>> {
    let base = assets_url.trim_end_matches('/');
    let url = format!("{base}/api/v1/drive/nodes");
    let authz = format!("Bearer {access_token}");
    match ureq::agent()
        .get(&url)
        .set("authorization", &authz)
        .call()
    {
        Ok(resp) => {
            let parsed: NodesResponse = resp.into_json().context("parse drive nodes")?;
            Ok(parsed.nodes)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(anyhow!("drive list status={code} body={body}"))
        }
        Err(err) => Err(anyhow!("drive list failed: {err:#}")),
    }
}

/// Resolve take-out; returns first blob blake3 for a **blob** node (x006d single-mesh).
pub fn take_out_blob_blake3(
    assets_url: &str,
    access_token: &str,
    node_id: &str,
) -> Result<(DriveNodeDto, String)> {
    let base = assets_url.trim_end_matches('/');
    let url = format!("{base}/api/v1/drive/take_out");
    let authz = format!("Bearer {access_token}");
    let body = json!({ "node_id": node_id });
    match ureq::agent()
        .post(&url)
        .set("authorization", &authz)
        .set("content-type", "application/json")
        .send_json(&body)
    {
        Ok(resp) => {
            let parsed: TakeOutResponse = resp.into_json().context("parse take_out")?;
            if parsed.node.kind != "blob" {
                return Err(anyhow!(
                    "x006d supports blob nodes only (got kind={})",
                    parsed.node.kind
                ));
            }
            let blake3 = parsed
                .blobs
                .first()
                .map(|b| b.blake3.clone())
                .ok_or_else(|| anyhow!("take_out returned no blobs"))?;
            Ok((parsed.node, blake3))
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(anyhow!("take_out status={code} body={body}"))
        }
        Err(err) => Err(anyhow!("take_out failed: {err:#}")),
    }
}

/// Put spawn request on Zenoh (plaintext JSON for now; room AEAD can wrap later).
pub fn publish_inventory_spawn(
    session: &zenoh::Session,
    room_id: &str,
    wire_cap: &str,
    room_token: &str,
    blake3: &str,
    x: f64,
    y: f64,
    z: f64,
) -> Result<()> {
    let key = inventory_spawn_key(room_id, wire_cap);
    let payload = json!({
        "room_token": room_token,
        "blake3": blake3,
        "x": x,
        "y": y,
        "z": z,
        "qw": 1.0,
        "sx": 1.0,
        "sy": 1.0,
        "sz": 1.0,
    });
    let bytes = serde_json::to_vec(&payload).context("encode spawn JSON")?;
    let publisher = session
        .declare_publisher(key.clone())
        .wait()
        .map_err(|e| anyhow!("declare_publisher inventory/spawn: {e}"))?;
    publisher
        .put(bytes)
        .wait()
        .map_err(|e| anyhow!("inventory spawn put: {e}"))?;
    log::info!("inventory spawn published on {key}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_node_dto_deserializes() {
        let raw = r#"{"nodes":[{"node_id":"n1","kind":"blob","name":"mesh","blake3":"aa"}]}"#;
        let parsed: NodesResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.nodes[0].kind, "blob");
    }
}
