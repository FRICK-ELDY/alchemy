//! Inventory take-out / put-back / despawn client helpers (x006d / x006e / x007b).
//!
//! Guarantees: Drive list / take_out / put_back over assets HTTP; spawn/despawn put on
//! Zenoh `inventory/spawn|despawn` sealed with Room AEAD (host Room Token never leaves
//! the room AEAD boundary); no Drive node_id on the wire to host. Put-back is
//! Bearer→assets only (no Zenoh); Despawn removes the room entity via host.
//! Non-guarantees: Prefab multi-child UI; XR Inventory polish; put-back+despawn TX.

use alchemy_protocol::wire_aead::{self, AeadKind};
use alchemy_protocol::{inventory_despawn_key, inventory_spawn_key};
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
    match ureq::agent().get(&url).set("authorization", &authz).call() {
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

/// Save a single-child Prefab via assets put-back (x006e). Returns Drive node.
pub fn put_back_prefab(
    assets_url: &str,
    access_token: &str,
    name: &str,
    blake3: &str,
    trs: Option<([f64; 3], [f64; 4], [f64; 3])>,
) -> Result<DriveNodeDto> {
    let base = assets_url.trim_end_matches('/');
    let url = format!("{base}/api/v1/drive/put_back");
    let authz = format!("Bearer {access_token}");
    let mut child = json!({
        "local_id": "mesh_0",
        "blake3": blake3,
    });
    if let Some((translation, rotation, scale)) = trs {
        child["trs"] = json!({
            "translation": translation,
            "rotation": rotation,
            "scale": scale,
        });
    }
    let body = json!({
        "name": name,
        "payload": {
            "schema_version": 1,
            "children": [child],
        },
    });
    match ureq::agent()
        .post(&url)
        .set("authorization", &authz)
        .set("content-type", "application/json")
        .send_json(&body)
    {
        Ok(resp) => {
            #[derive(Deserialize)]
            struct PutBackResponse {
                node: DriveNodeDto,
            }
            let parsed: PutBackResponse = resp.into_json().context("parse put_back")?;
            if parsed.node.kind != "prefab" {
                return Err(anyhow!(
                    "put_back expected prefab (got kind={})",
                    parsed.node.kind
                ));
            }
            Ok(parsed.node)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(anyhow!("put_back status={code} body={body}"))
        }
        Err(err) => Err(anyhow!("put_back failed: {err:#}")),
    }
}

/// Best-effort role peek from an unverified JWT payload (UI gate only).
///
/// Host still verifies signature + role after AEAD open. This avoids guest UX
/// that reports "spawn requested" when host will reject.
pub fn room_token_role_unverified(room_token: &str) -> Option<String> {
    let payload = room_token.split('.').nth(1)?;
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, payload)
        .or_else(|_| base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE, payload))
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("role")?.as_str().map(|s| s.to_string())
}

/// Put AEAD-sealed spawn request on Zenoh (x006d / x0069).
#[allow(clippy::too_many_arguments)] // room + pose wire args; mirrors host spawn payload
pub fn publish_inventory_spawn(
    session: &zenoh::Session,
    room_id: &str,
    wire_cap: &str,
    room_key: &[u8; wire_aead::ROOM_KEY_LEN],
    room_key_epoch: u64,
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
    let plain = serde_json::to_vec(&payload).context("encode spawn JSON")?;
    let envelope = wire_aead::seal(
        room_key,
        room_id,
        room_key_epoch,
        AeadKind::InventorySpawn,
        &plain,
    )
    .map_err(|e| anyhow!("AEAD seal inventory spawn: {e}"))?;
    let publisher = session
        .declare_publisher(key.clone())
        .wait()
        .map_err(|e| anyhow!("declare_publisher inventory/spawn: {e}"))?;
    publisher
        .put(envelope)
        .wait()
        .map_err(|e| anyhow!("inventory spawn put: {e}"))?;
    log::info!("inventory spawn published on {key} (AEAD)");
    Ok(())
}

/// Put AEAD-sealed despawn request on Zenoh (x007b / x0069).
pub fn publish_inventory_despawn(
    session: &zenoh::Session,
    room_id: &str,
    wire_cap: &str,
    room_key: &[u8; wire_aead::ROOM_KEY_LEN],
    room_key_epoch: u64,
    room_token: &str,
    entity_id: u32,
) -> Result<()> {
    let key = inventory_despawn_key(room_id, wire_cap);
    let payload = json!({
        "room_token": room_token,
        "entity_id": entity_id,
    });
    let plain = serde_json::to_vec(&payload).context("encode despawn JSON")?;
    let envelope = wire_aead::seal(
        room_key,
        room_id,
        room_key_epoch,
        AeadKind::InventoryDespawn,
        &plain,
    )
    .map_err(|e| anyhow!("AEAD seal inventory despawn: {e}"))?;
    let publisher = session
        .declare_publisher(key.clone())
        .wait()
        .map_err(|e| anyhow!("declare_publisher inventory/despawn: {e}"))?;
    publisher
        .put(envelope)
        .wait()
        .map_err(|e| anyhow!("inventory despawn put: {e}"))?;
    log::info!("inventory despawn published on {key} entity_id={entity_id} (AEAD)");
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

    #[test]
    fn role_unverified_reads_payload() {
        // header.payload.sig — payload = {"role":"host"}
        let payload = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            br#"{"role":"host"}"#,
        );
        let tok = format!("x.{payload}.y");
        assert_eq!(room_token_role_unverified(&tok).as_deref(), Some("host"));
    }
}
