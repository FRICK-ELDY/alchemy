//! Inventory take-out / put-back / despawn client helpers (x006d / x006e / x007b / x007f / x0081 / x0098 / x00b2).
//!
//! Guarantees: Drive list / take_out / put_back over assets HTTP; spawn/despawn put on
//! Zenoh `…/edit` as session-signed `EditProposal` (Room AEAD · x00b2); no Drive node_id
//! on the wire to host. Owner may use guest Room Token + Intent bind; host Dual-auth
//! accepts `owner_sub` or host-role (x0098). Put-back is Bearer→assets only (no Zenoh);
//! Despawn removes the room entity via host. Prefab take_out expands children (x007f);
//! client `session.put`s each blake3 sequentially. Desktop paces puts under host
//! `SharedControlRate` (8/10s across edit + legacy inventory) on an auth/inventory
//! worker (not the UI loop); status reports `spawn put N/M` (Zenoh success ≠ host accept).
//! Non-guarantees: atomic multi-spawn rollback; XR-direct HTTP/Zenoh; put-back+despawn TX.

use alchemy_protocol::wire_aead::{self, AeadKind};
use alchemy_protocol::{edit_key, EditProposal, Message};
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

/// Put AEAD-sealed `EditProposal` on `…/edit` (x00b2).
pub fn publish_edit_proposal(
    session: &zenoh::Session,
    room_id: &str,
    wire_cap: &str,
    room_key: &[u8; wire_aead::ROOM_KEY_LEN],
    room_key_epoch: u64,
    proposal: &EditProposal,
) -> Result<()> {
    let key = edit_key(room_id, wire_cap);
    let plain = proposal.encode_to_vec();
    let envelope = wire_aead::seal(
        room_key,
        room_id,
        room_key_epoch,
        AeadKind::EditProposal,
        &plain,
    )
    .map_err(|e| anyhow!("seal EditProposal: {e}"))?;
    session
        .put(&key, envelope)
        .wait()
        .map_err(|e| anyhow!("edit put: {e}"))?;
    Ok(())
}
