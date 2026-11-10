//! Inventory take-out / put-back / despawn client helpers (x006d / x006e / x007b / x007f / x0081).
//!
//! Guarantees: Drive list / take_out / put_back over assets HTTP; spawn/despawn put on
//! Zenoh `inventory/spawn|despawn` sealed with Room AEAD and **session-signed** (no Room
//! Token on the bus · x0081); no Drive node_id on the wire to host. Put-back is
//! Bearer→assets only (no Zenoh); Despawn removes the room entity via host.
//! Prefab take_out expands children (x007f); client `session.put`s each blake3
//! sequentially (same style as room Intent puts; declare_publisher not required).
//! Desktop paces puts under host `SpawnRate(8/10s)` on an auth/inventory worker
//! (not the UI loop); status reports `spawn put N/M` (Zenoh success ≠ host accept).
//! Non-guarantees: atomic multi-spawn rollback; host spawn ack to client;
//! XR-direct HTTP/Zenoh; put-back+despawn TX.

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
    blobs: Vec<TakeOutBlob>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TakeOutBlob {
    pub blake3: String,
    #[serde(default)]
    #[allow(dead_code)] // reserved for UI labels / host batch later
    pub local_id: Option<String>,
    #[serde(default)]
    pub trs: Option<TakeOutTrs>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TakeOutTrs {
    pub translation: [f64; 3],
    pub rotation: [f64; 4],
    pub scale: [f64; 3],
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

/// Resolve take-out for a blob or prefab node (x006d / x007f).
pub fn take_out_blobs(
    assets_url: &str,
    access_token: &str,
    node_id: &str,
) -> Result<(DriveNodeDto, Vec<TakeOutBlob>)> {
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
            match parsed.node.kind.as_str() {
                "blob" | "prefab" => {}
                other => {
                    return Err(anyhow!(
                        "x007f supports blob/prefab take_out only (got kind={other})"
                    ));
                }
            }
            if parsed.blobs.is_empty() {
                return Err(anyhow!("take_out returned no blobs"));
            }
            Ok((parsed.node, parsed.blobs))
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(anyhow!("take_out status={code} body={body}"))
        }
        Err(err) => Err(anyhow!("take_out failed: {err:#}")),
    }
}

/// Prefab put-back child: `(local_id, blake3, optional TRS)`.
pub type PrefabPutBackChild = (String, String, Option<([f64; 3], [f64; 4], [f64; 3])>);

/// Save a Prefab via assets put-back (x006e / x007f multi-child). Returns Drive node.
pub fn put_back_prefab_children(
    assets_url: &str,
    access_token: &str,
    name: &str,
    children: &[PrefabPutBackChild],
) -> Result<DriveNodeDto> {
    if children.is_empty() {
        return Err(anyhow!("put_back requires at least one child"));
    }
    let base = assets_url.trim_end_matches('/');
    let url = format!("{base}/api/v1/drive/put_back");
    let authz = format!("Bearer {access_token}");
    let payload_children: Vec<_> = children
        .iter()
        .enumerate()
        .map(|(i, (local_id, blake3, trs))| {
            let mut child = json!({
                "local_id": if local_id.is_empty() {
                    format!("mesh_{i}")
                } else {
                    local_id.clone()
                },
                "blake3": blake3,
            });
            if let Some((translation, rotation, scale)) = trs {
                child["trs"] = json!({
                    "translation": translation,
                    "rotation": rotation,
                    "scale": scale,
                });
            }
            child
        })
        .collect();
    let body = json!({
        "name": name,
        "payload": {
            "schema_version": 1,
            "children": payload_children,
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

/// Convenience: single-child put-back (x006e).
#[allow(dead_code)]
pub fn put_back_prefab(
    assets_url: &str,
    access_token: &str,
    name: &str,
    blake3: &str,
    trs: Option<([f64; 3], [f64; 4], [f64; 3])>,
) -> Result<DriveNodeDto> {
    put_back_prefab_children(
        assets_url,
        access_token,
        name,
        &[(String::from("mesh_0"), blake3.to_string(), trs)],
    )
}

/// Best-effort role peek from an unverified JWT payload (UI gate only).
pub fn room_token_role_unverified(room_token: &str) -> Option<String> {
    let payload = room_token.split('.').nth(1)?;
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, payload)
        .or_else(|_| base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE, payload))
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("role")?.as_str().map(|s| s.to_string())
}

/// Put AEAD-sealed spawn request on Zenoh (x006d / x0069 / x0081).
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)] // room + pose wire args; mirrors host spawn payload
pub fn publish_inventory_spawn(
    session: &zenoh::Session,
    room_id: &str,
    wire_cap: &str,
    room_key: &[u8; wire_aead::ROOM_KEY_LEN],
    room_key_epoch: u64,
    control: &crate::intent_session::ControlSig,
    blake3: &str,
    x: f64,
    y: f64,
    z: f64,
) -> Result<()> {
    publish_inventory_spawn_pose(
        session,
        room_id,
        wire_cap,
        room_key,
        room_key_epoch,
        control,
        blake3,
        x,
        y,
        z,
        0.0,
        0.0,
        0.0,
        1.0,
        1.0,
        1.0,
        1.0,
    )
}

/// Spawn with full TRS (x007f prefab child). Session-signed — no Room Token (x0081).
#[allow(clippy::too_many_arguments)]
pub fn publish_inventory_spawn_pose(
    session: &zenoh::Session,
    room_id: &str,
    wire_cap: &str,
    room_key: &[u8; wire_aead::ROOM_KEY_LEN],
    room_key_epoch: u64,
    control: &crate::intent_session::ControlSig,
    blake3: &str,
    x: f64,
    y: f64,
    z: f64,
    qx: f64,
    qy: f64,
    qz: f64,
    qw: f64,
    sx: f64,
    sy: f64,
    sz: f64,
) -> Result<()> {
    let key = inventory_spawn_key(room_id, wire_cap);
    let payload = json!({
        "session_pub": control.session_pub_b64,
        "control_seq": control.control_seq,
        "control_sig": control.control_sig_b64,
        "blake3": blake3.to_ascii_lowercase(),
        "x": x,
        "y": y,
        "z": z,
        "qx": qx,
        "qy": qy,
        "qz": qz,
        "qw": qw,
        "sx": sx,
        "sy": sy,
        "sz": sz,
    });
    let plain = serde_json::to_vec(&payload).context("encode spawn JSON")?;
    let envelope = wire_aead::seal(
        room_key,
        room_id,
        room_key_epoch,
        AeadKind::InventorySpawn,
        &plain,
    )
    .map_err(|e| anyhow!("seal spawn: {e}"))?;
    session
        .put(&key, envelope)
        .wait()
        .map_err(|e| anyhow!("spawn put: {e}"))?;
    Ok(())
}

/// Put AEAD-sealed despawn request on Zenoh (x007b / x0081).
pub fn publish_inventory_despawn(
    session: &zenoh::Session,
    room_id: &str,
    wire_cap: &str,
    room_key: &[u8; wire_aead::ROOM_KEY_LEN],
    room_key_epoch: u64,
    control: &crate::intent_session::ControlSig,
    entity_id: u32,
) -> Result<()> {
    let key = inventory_despawn_key(room_id, wire_cap);
    let payload = json!({
        "session_pub": control.session_pub_b64,
        "control_seq": control.control_seq,
        "control_sig": control.control_sig_b64,
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
    .map_err(|e| anyhow!("seal despawn: {e}"))?;
    session
        .put(&key, envelope)
        .wait()
        .map_err(|e| anyhow!("despawn put: {e}"))?;
    Ok(())
}
