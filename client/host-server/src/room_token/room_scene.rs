//! World-authored room scene: JSON DTOs, fetch, append, and delete (x0027 / x006d / x007b).
//!
//! Guarantees: host Room Token Bearer + `cnf.spk` PoP on every request (x0081); scene entries
//! TRS-normalized on parse; shared-scene install is **revision-monotonic** (strictly newer only;
//! x007b spawn/despawn race).
//! Non-guarantees: scene persistence, Manifest sealing.

use super::http::{credential_agent, is_blake3_hex};
use super::http_pop_client::{request_path, with_pop};
use alchemy_protocol::wire_crypto::SigningKey;
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::sync::{Arc, RwLock};

/// World-authored scene definition for a room (x0027).
/// Built from JSON DTOs — prost types are not serde-deserializable.
#[derive(Debug, Clone)]
pub struct RoomScene {
    pub revision: u64,
    pub entries: Vec<alchemy_protocol::SceneEntry>,
    pub locomotion: Option<alchemy_protocol::LocomotionConfig>,
}

#[derive(Debug, Deserialize)]
struct SceneEntryJson {
    entity_id: u32,
    #[serde(default)]
    blake3: String,
    #[serde(default)]
    x: f64,
    #[serde(default)]
    y: f64,
    #[serde(default)]
    z: f64,
    #[serde(default)]
    half_extent: f32,
    #[serde(default)]
    flags: u32,
    #[serde(default)]
    r: f32,
    #[serde(default)]
    g: f32,
    #[serde(default)]
    b: f32,
    #[serde(default)]
    a: f32,
    #[serde(default)]
    qx: f64,
    #[serde(default)]
    qy: f64,
    #[serde(default)]
    qz: f64,
    #[serde(default)]
    qw: f64,
    #[serde(default)]
    sx: f64,
    #[serde(default)]
    sy: f64,
    #[serde(default)]
    sz: f64,
}

#[derive(Debug, Deserialize)]
struct LocomotionJson {
    #[serde(default)]
    mode: String,
    #[serde(default)]
    move_speed: f32,
}

#[derive(Debug, Deserialize)]
struct SceneResponseJson {
    revision: u64,
    entries: Vec<SceneEntryJson>,
    locomotion: Option<LocomotionJson>,
}

fn loco_mode_from_str(mode: &str) -> alchemy_protocol::LocomotionMode {
    match mode.to_ascii_lowercase().as_str() {
        "fly" => alchemy_protocol::LocomotionMode::Fly,
        "walk" => alchemy_protocol::LocomotionMode::Walk,
        "noclip" | "" => alchemy_protocol::LocomotionMode::Noclip,
        other => {
            log::warn!("unknown locomotion mode {other:?}; falling back to Noclip");
            alchemy_protocol::LocomotionMode::Noclip
        }
    }
}

fn scene_from_response(parsed: SceneResponseJson) -> RoomScene {
    let entries = parsed
        .entries
        .into_iter()
        .map(|e| {
            let mut entry = alchemy_protocol::SceneEntry {
                entity_id: e.entity_id,
                blake3: e.blake3,
                x: e.x,
                y: e.y,
                z: e.z,
                half_extent: e.half_extent,
                flags: e.flags,
                r: e.r,
                g: e.g,
                b: e.b,
                a: e.a,
                qx: e.qx,
                qy: e.qy,
                qz: e.qz,
                qw: e.qw,
                sx: e.sx,
                sy: e.sy,
                sz: e.sz,
            };
            alchemy_protocol::normalize_scene_entry_trs(&mut entry);
            entry
        })
        .collect();
    let locomotion = parsed
        .locomotion
        .map(|l| alchemy_protocol::LocomotionConfig {
            mode: loco_mode_from_str(&l.mode).into(),
            move_speed: l.move_speed,
        });
    RoomScene {
        revision: parsed.revision,
        entries,
        locomotion,
    }
}

/// Replace `shared` only when `incoming.revision` is **strictly greater**.
///
/// Spawn / despawn workers complete HTTP out of order; physics must not let an older
/// revision overwrite a newer shared scene (x007b). Equal revision is a no-op.
pub fn install_shared_scene_if_newer(
    shared: &Arc<RwLock<RoomScene>>,
    incoming: RoomScene,
) -> Result<bool> {
    let mut guard = shared
        .write()
        .map_err(|_| anyhow!("shared scene RwLock poisoned"))?;
    if incoming.revision > guard.revision {
        *guard = incoming;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Fetch world DemoScene for this room (host Room Token + PoP).
pub fn fetch_room_scene(
    world_url: &str,
    room_id: &str,
    host_room_token: &str,
    pop_key: &SigningKey,
) -> Result<RoomScene> {
    let base = world_url.trim_end_matches('/');
    let url = format!("{base}/api/v1/rooms/{room_id}/scene");
    let path = request_path(&url);
    let authz = format!("Bearer {host_room_token}");
    let req = with_pop(
        credential_agent().get(&url).set("authorization", &authz),
        pop_key,
        "GET",
        path,
        b"",
    );
    match req.call() {
        Ok(resp) => {
            let parsed: SceneResponseJson = resp.into_json().context("parse scene response")?;
            Ok(scene_from_response(parsed))
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(anyhow!("scene fetch failed status={code} body={body}"))
        }
        Err(err) => Err(anyhow!("scene fetch request failed: {err:#}")),
    }
}

/// Append one mesh entry to world scene (x006d). Returns updated scene + new entry.
#[allow(clippy::too_many_arguments)] // world HTTP body fields (TRS) stay flat at the wire edge
pub fn append_room_scene_entry(
    world_url: &str,
    room_id: &str,
    host_room_token: &str,
    pop_key: &SigningKey,
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
) -> Result<(RoomScene, alchemy_protocol::SceneEntry)> {
    if !is_blake3_hex(blake3) {
        return Err(anyhow!("invalid blake3 for scene append"));
    }
    let base = world_url.trim_end_matches('/');
    let url = format!("{base}/api/v1/rooms/{room_id}/scene/entries");
    let path = request_path(&url);
    let authz = format!("Bearer {host_room_token}");
    let raw = serde_json::to_vec(&serde_json::json!({
        "blake3": blake3,
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
    }))
    .context("encode scene append body")?;
    let req = with_pop(
        credential_agent()
            .post(&url)
            .set("authorization", &authz)
            .set("content-type", "application/json"),
        pop_key,
        "POST",
        path,
        &raw,
    );
    match req.send(raw.as_slice()) {
        Ok(resp) => {
            let parsed: SceneResponseJson = resp.into_json().context("parse scene append")?;
            let scene = scene_from_response(parsed);
            let entry = scene
                .entries
                .iter()
                .rev()
                .find(|e| e.blake3.eq_ignore_ascii_case(blake3))
                .cloned()
                .ok_or_else(|| anyhow!("appended blake3 missing from scene response"))?;
            Ok((scene, entry))
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(anyhow!("scene append failed status={code} body={body}"))
        }
        Err(err) => Err(anyhow!("scene append request failed: {err:#}")),
    }
}

/// Delete one mesh entry from world scene by entity_id (x007b). Returns updated scene.
pub fn delete_room_scene_entry(
    world_url: &str,
    room_id: &str,
    host_room_token: &str,
    pop_key: &SigningKey,
    entity_id: u32,
) -> Result<RoomScene> {
    let base = world_url.trim_end_matches('/');
    let url = format!("{base}/api/v1/rooms/{room_id}/scene/entries/{entity_id}");
    let path = request_path(&url);
    let authz = format!("Bearer {host_room_token}");
    let req = with_pop(
        credential_agent()
            .delete(&url)
            .set("authorization", &authz),
        pop_key,
        "DELETE",
        path,
        b"",
    );
    match req.call() {
        Ok(resp) => {
            let parsed: SceneResponseJson = resp.into_json().context("parse scene delete")?;
            let scene = scene_from_response(parsed);
            if scene.entries.iter().any(|e| e.entity_id == entity_id) {
                return Err(anyhow!("deleted entity_id still present in scene response"));
            }
            Ok(scene)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(anyhow!("scene delete failed status={code} body={body}"))
        }
        Err(err) => Err(anyhow!("scene delete request failed: {err:#}")),
    }
}
