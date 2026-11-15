//! World-authored room scene: JSON DTOs, fetch, append, and delete (x0027 / x006d / x007b / x0082 / x0098).
//!
//! Guarantees: host Room Token Bearer + `cnf.spk` PoP on every request (x0081); scene entries
//! TRS-normalized on parse; shared-scene install is **revision-monotonic** (strictly newer only;
//! x007b spawn/despawn race) and is owned by the **physics apply** path; append/delete send
//! `expected_revision` (seeded from shared) and retry on `scene_revision_conflict` only (x0082)
//! **without** writing shared on success or conflict refetch (so apply's strictly-newer gate
//! still runs XPBD/sim insert + `force_reseal`). GET scene carries `owner_sub` (x0098); install
//! **preserves** a prior `owner_sub` when an append/delete response omits it.
//! Non-guarantees: scene persistence, Manifest sealing; infinite CAS retries.

use super::http::{credential_agent, is_blake3_hex};
use super::http_pop_client::{request_path, with_pop};
use alchemy_protocol::wire_crypto::SigningKey;
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::sync::{Arc, RwLock};

/// World-authored scene definition for a room (x0027 / x0098).
/// Built from JSON DTOs — prost types are not serde-deserializable.
#[derive(Debug, Clone)]
pub struct RoomScene {
    pub revision: u64,
    pub entries: Vec<alchemy_protocol::SceneEntry>,
    pub locomotion: Option<alchemy_protocol::LocomotionConfig>,
    /// Room ledger owner (`GET …/scene`); used by inventory control Dual auth (x0098).
    pub owner_sub: Option<String>,
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
    /// Creative hierarchy parent (x00b8). 0 / omitted = world root.
    #[serde(default)]
    parent_entity_id: u32,
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
    /// Present on GET scene (x0098); append/delete responses may omit.
    #[serde(default)]
    owner_sub: Option<String>,
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
                parent_entity_id: e.parent_entity_id,
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
    let owner_sub = parsed
        .owner_sub
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    RoomScene {
        revision: parsed.revision,
        entries,
        locomotion,
        owner_sub,
    }
}

/// Replace `shared` only when `incoming.revision` is **strictly greater**.
///
/// Spawn / despawn workers complete HTTP out of order; physics must not let an older
/// revision overwrite a newer shared scene (x007b). Equal revision is a no-op.
/// x0098: **never trust** `incoming.owner_sub` when shared already has one (append/delete
/// or polluted scene JSON must not diverge from the ledger value seeded by GET).
/// Only adopt incoming when shared has no owner yet (blank ignored).
/// Non-goal here: mid-lifetime ownership transfer without host restart — a later GET would
/// also be ignored once set; that needs an explicit GET-only refresh path.
pub fn install_shared_scene_if_newer(
    shared: &Arc<RwLock<RoomScene>>,
    incoming: RoomScene,
) -> Result<bool> {
    let mut guard = shared
        .write()
        .map_err(|_| anyhow!("shared scene RwLock poisoned"))?;
    if incoming.revision > guard.revision {
        let owner_sub = guard.owner_sub.clone().or_else(|| {
            incoming
                .owner_sub
                .as_ref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        });
        *guard = RoomScene {
            owner_sub,
            ..incoming
        };
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

/// Max optimistic CAS attempts for scene append/delete (x0082).
const MAX_SCENE_CAS_ATTEMPTS: u32 = 5;

#[derive(Debug, Deserialize)]
struct SceneErrorJson {
    error: String,
}

fn is_scene_revision_conflict(body: &str) -> bool {
    serde_json::from_str::<SceneErrorJson>(body)
        .map(|e| e.error == "scene_revision_conflict")
        .unwrap_or(false)
}

fn shared_revision(shared: &Arc<RwLock<RoomScene>>) -> Result<u64> {
    Ok(shared
        .read()
        .map_err(|_| anyhow!("shared scene RwLock poisoned"))?
        .revision)
}

/// Re-read world revision for CAS retry. Does **not** install into `shared_scene` —
/// physics apply owns shared updates (strictly-newer + XPBD/sim / reseal).
fn fetch_expected_revision(
    world_url: &str,
    room_id: &str,
    host_room_token: &str,
    pop_key: &SigningKey,
) -> Result<u64> {
    Ok(fetch_room_scene(world_url, room_id, host_room_token, pop_key)?.revision)
}

fn cas_backoff(attempt: u32) {
    // Light delay before retry; Slice guarantees finite attempts only.
    let ms = u64::from(attempt).saturating_mul(5).min(25);
    if ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

/// Append one mesh entry to world scene (x006d / x0082). Returns updated scene + new entry.
///
/// Sends `expected_revision` from `shared_scene` and retries on `scene_revision_conflict` only.
#[allow(clippy::too_many_arguments)] // world HTTP body fields (TRS) stay flat at the wire edge
pub fn append_room_scene_entry(
    world_url: &str,
    room_id: &str,
    host_room_token: &str,
    pop_key: &SigningKey,
    shared_scene: &Arc<RwLock<RoomScene>>,
    blake3: &str,
    spawn_ticket: &str,
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
    if spawn_ticket.is_empty() {
        return Err(anyhow!("spawn_ticket required for scene append"));
    }
    let mut expected = shared_revision(shared_scene)?;
    for attempt in 1..=MAX_SCENE_CAS_ATTEMPTS {
        match append_room_scene_entry_once(
            world_url,
            room_id,
            host_room_token,
            pop_key,
            expected,
            blake3,
            spawn_ticket,
            x,
            y,
            z,
            qx,
            qy,
            qz,
            qw,
            sx,
            sy,
            sz,
        ) {
            Ok((scene, entry)) => return Ok((scene, entry)),
            Err(CasAttempt::Conflict) if attempt < MAX_SCENE_CAS_ATTEMPTS => {
                cas_backoff(attempt);
                expected = fetch_expected_revision(world_url, room_id, host_room_token, pop_key)
                    .with_context(|| {
                        format!("scene append CAS refetch after conflict (attempt {attempt})")
                    })?;
            }
            Err(CasAttempt::Conflict) => {
                return Err(anyhow!(
                    "scene append CAS exhausted after {MAX_SCENE_CAS_ATTEMPTS} attempts"
                ));
            }
            Err(CasAttempt::Forbidden) => {
                return Err(anyhow!("scene_writer_forbidden"));
            }
            Err(CasAttempt::Other(err)) => return Err(err),
        }
    }
    Err(anyhow!(
        "scene append CAS exhausted after {MAX_SCENE_CAS_ATTEMPTS} attempts"
    ))
}

#[allow(clippy::too_many_arguments)]
fn append_room_scene_entry_once(
    world_url: &str,
    room_id: &str,
    host_room_token: &str,
    pop_key: &SigningKey,
    expected_revision: u64,
    blake3: &str,
    spawn_ticket: &str,
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
) -> std::result::Result<(RoomScene, alchemy_protocol::SceneEntry), CasAttempt> {
    let base = world_url.trim_end_matches('/');
    // x009e: inventory take-out writes session_overlay (not canonical scene).
    let url = format!("{base}/api/v1/rooms/{room_id}/overlay/entries");
    let path = request_path(&url);
    let authz = format!("Bearer {host_room_token}");
    let raw = serde_json::to_vec(&serde_json::json!({
        "blake3": blake3,
        "spawn_ticket": spawn_ticket,
        "expected_revision": expected_revision,
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
    .map_err(|e| CasAttempt::Other(anyhow!("encode overlay append body: {e:#}")))?;
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
            let parsed: SceneResponseJson = resp
                .into_json()
                .map_err(|e| CasAttempt::Other(anyhow!("parse overlay append: {e:#}")))?;
            // x0098: only GET scene is the owner_sub source; ignore mutation body fields.
            let mut scene = scene_from_response(parsed);
            scene.owner_sub = None;
            let entry = scene
                .entries
                .iter()
                .rev()
                .find(|e| e.blake3.eq_ignore_ascii_case(blake3))
                .cloned()
                .ok_or_else(|| {
                    CasAttempt::Other(anyhow!("appended blake3 missing from overlay response"))
                })?;
            Ok((scene, entry))
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            if code == 409 && is_scene_revision_conflict(&body) {
                Err(CasAttempt::Conflict)
            } else if code == 403 && is_scene_writer_forbidden(&body) {
                Err(CasAttempt::Forbidden)
            } else {
                Err(CasAttempt::Other(anyhow!(
                    "overlay append failed status={code} body={body}"
                )))
            }
        }
        Err(err) => Err(CasAttempt::Other(anyhow!(
            "overlay append request failed: {err:#}"
        ))),
    }
}

/// Delete one mesh entry from world scene by entity_id (x007b / x0082). Returns updated scene.
///
/// Sends JSON `expected_revision` (PoP-bound) and retries on `scene_revision_conflict` only.
pub fn delete_room_scene_entry(
    world_url: &str,
    room_id: &str,
    host_room_token: &str,
    pop_key: &SigningKey,
    shared_scene: &Arc<RwLock<RoomScene>>,
    entity_id: u32,
) -> Result<RoomScene> {
    let mut expected = shared_revision(shared_scene)?;
    for attempt in 1..=MAX_SCENE_CAS_ATTEMPTS {
        match delete_room_scene_entry_once(
            world_url,
            room_id,
            host_room_token,
            pop_key,
            expected,
            entity_id,
        ) {
            Ok(scene) => return Ok(scene),
            Err(CasAttempt::Conflict) if attempt < MAX_SCENE_CAS_ATTEMPTS => {
                cas_backoff(attempt);
                expected = fetch_expected_revision(world_url, room_id, host_room_token, pop_key)
                    .with_context(|| {
                        format!("scene delete CAS refetch after conflict (attempt {attempt})")
                    })?;
            }
            Err(CasAttempt::Conflict) => {
                return Err(anyhow!(
                    "scene delete CAS exhausted after {MAX_SCENE_CAS_ATTEMPTS} attempts"
                ));
            }
            Err(CasAttempt::Forbidden) => {
                return Err(anyhow!("scene_writer_forbidden"));
            }
            Err(CasAttempt::Other(err)) => return Err(err),
        }
    }
    Err(anyhow!(
        "scene delete CAS exhausted after {MAX_SCENE_CAS_ATTEMPTS} attempts"
    ))
}

fn delete_room_scene_entry_once(
    world_url: &str,
    room_id: &str,
    host_room_token: &str,
    pop_key: &SigningKey,
    expected_revision: u64,
    entity_id: u32,
) -> std::result::Result<RoomScene, CasAttempt> {
    let base = world_url.trim_end_matches('/');
    // x009e: inventory despawn removes from session_overlay.
    let url = format!("{base}/api/v1/rooms/{room_id}/overlay/entries/{entity_id}");
    let path = request_path(&url);
    let authz = format!("Bearer {host_room_token}");
    let raw = serde_json::to_vec(&serde_json::json!({
        "expected_revision": expected_revision,
    }))
    .map_err(|e| CasAttempt::Other(anyhow!("encode overlay delete body: {e:#}")))?;
    let req = with_pop(
        credential_agent()
            .delete(&url)
            .set("authorization", &authz)
            .set("content-type", "application/json"),
        pop_key,
        "DELETE",
        path,
        &raw,
    );
    match req.send(raw.as_slice()) {
        Ok(resp) => {
            let parsed: SceneResponseJson = resp
                .into_json()
                .map_err(|e| CasAttempt::Other(anyhow!("parse scene delete: {e:#}")))?;
            // x0098: only GET scene is the owner_sub source; ignore mutation body fields.
            let mut scene = scene_from_response(parsed);
            scene.owner_sub = None;
            if scene.entries.iter().any(|e| e.entity_id == entity_id) {
                return Err(CasAttempt::Other(anyhow!(
                    "deleted entity_id still present in scene response"
                )));
            }
            Ok(scene)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            if code == 409 && is_scene_revision_conflict(&body) {
                Err(CasAttempt::Conflict)
            } else if code == 403 && is_scene_writer_forbidden(&body) {
                Err(CasAttempt::Forbidden)
            } else {
                Err(CasAttempt::Other(anyhow!(
                    "scene delete failed status={code} body={body}"
                )))
            }
        }
        Err(err) => Err(CasAttempt::Other(anyhow!(
            "scene delete request failed: {err:#}"
        ))),
    }
}

enum CasAttempt {
    Conflict,
    /// World 403 `scene_writer_forbidden` / host forbidden (x00b7 Save).
    Forbidden,
    Other(anyhow::Error),
}

/// Typed Save failure for EditCommit mapping (x00b7 / x00b8 / x00b9).
#[derive(Debug)]
pub enum SceneSaveError {
    Forbidden,
    /// Packed revision conflict after CAS retries exhausted (or single attempt).
    Conflict,
    ParentCycle,
    InvalidEntry,
    /// blake3 already present in merged scene (x00b9 spawn fold).
    DuplicateBlake3,
    Other(anyhow::Error),
}

impl std::fmt::Display for SceneSaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Forbidden => write!(f, "scene_writer_forbidden"),
            Self::Conflict => write!(f, "scene_revision_conflict"),
            Self::ParentCycle => write!(f, "parent_cycle"),
            Self::InvalidEntry => write!(f, "invalid_entry"),
            Self::DuplicateBlake3 => write!(f, "duplicate_blake3"),
            Self::Other(err) => write!(f, "{err:#}"),
        }
    }
}

impl std::error::Error for SceneSaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Other(err) => Some(err.as_ref()),
            _ => None,
        }
    }
}

fn is_scene_writer_forbidden(body: &str) -> bool {
    scene_error_code(body).as_deref() == Some("scene_writer_forbidden")
}

fn is_spawn_ticket_forbidden(body: &str) -> bool {
    matches!(
        scene_error_code(body).as_deref(),
        Some("spawn_ticket_required" | "spawn_ticket_invalid")
    )
}

fn is_duplicate_blake3(body: &str) -> bool {
    scene_error_code(body).as_deref() == Some("duplicate_blake3")
}

fn scene_error_code(body: &str) -> Option<String> {
    serde_json::from_str::<SceneErrorJson>(body)
        .ok()
        .map(|e| e.error)
}

/// Live deltas folded into Save (x00b8 / x00b9). Empty = overlay-only merge (or no-op).
#[derive(Debug, Clone, Default)]
pub struct SceneSavePatches {
    pub parents: Vec<SceneParentPatch>,
    pub fields: Vec<SceneFieldPatch>,
    pub spawns: Vec<SceneSpawnPatch>,
    pub despawns: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct SceneParentPatch {
    pub entity_id: u32,
    pub parent_entity_id: u32,
}

#[derive(Debug, Clone)]
pub struct SceneFieldPatch {
    pub entity_id: u32,
    pub flags: Option<u32>,
    pub r: Option<f32>,
    pub g: Option<f32>,
    pub b: Option<f32>,
    pub a: Option<f32>,
    pub blake3: Option<String>,
}

/// Host-local spawn folded into Save (x00b9); world verifies `spawn_ticket`.
#[derive(Debug, Clone)]
pub struct SceneSpawnPatch {
    pub entry: alchemy_protocol::SceneEntry,
    pub spawn_ticket: String,
}

/// Append/remove mesh rows on the host working-tree scene without world CAS (x00b9).
/// Packed `revision` is unchanged; caller must `force_reseal` after apply.
pub fn append_local_scene_entry(
    shared: &Arc<RwLock<RoomScene>>,
    entry: alchemy_protocol::SceneEntry,
) -> Result<bool> {
    let mut scene = shared
        .write()
        .map_err(|_| anyhow!("shared scene RwLock poisoned"))?;
    if scene.entries.iter().any(|e| e.entity_id == entry.entity_id) {
        return Ok(false);
    }
    if scene
        .entries
        .iter()
        .any(|e| e.blake3.eq_ignore_ascii_case(&entry.blake3))
    {
        return Ok(false);
    }
    scene.entries.push(entry);
    Ok(true)
}

pub fn remove_local_scene_entry(shared: &Arc<RwLock<RoomScene>>, entity_id: u32) -> Result<bool> {
    let mut scene = shared
        .write()
        .map_err(|_| anyhow!("shared scene RwLock poisoned"))?;
    let before = scene.entries.len();
    scene.entries.retain(|e| e.entity_id != entity_id);
    Ok(scene.entries.len() != before)
}

/// x00b7 / x00b8 / x00b9: merge session_overlay into canonical, apply optional
/// spawn/despawn/parent/field patches, clear overlay (one packed CAS).
///
/// Returns composed scene. Empty overlay **and** empty patches is idempotent.
/// Retries on `scene_revision_conflict` only.
pub fn save_overlay_to_canonical(
    world_url: &str,
    room_id: &str,
    host_room_token: &str,
    pop_key: &SigningKey,
    shared_scene: &Arc<RwLock<RoomScene>>,
    patches: &SceneSavePatches,
) -> std::result::Result<RoomScene, SceneSaveError> {
    let mut expected = shared_revision(shared_scene).map_err(SceneSaveError::Other)?;
    for attempt in 1..=MAX_SCENE_CAS_ATTEMPTS {
        match save_overlay_to_canonical_once(
            world_url,
            room_id,
            host_room_token,
            pop_key,
            expected,
            patches,
        ) {
            Ok(scene) => return Ok(scene),
            Err(SceneSaveError::Conflict) if attempt < MAX_SCENE_CAS_ATTEMPTS => {
                cas_backoff(attempt);
                expected = fetch_expected_revision(world_url, room_id, host_room_token, pop_key)
                    .map_err(|e| {
                        SceneSaveError::Other(e.context(format!(
                            "scene save CAS refetch after conflict (attempt {attempt})"
                        )))
                    })?;
            }
            Err(err) => return Err(err),
        }
    }
    Err(SceneSaveError::Conflict)
}

fn save_overlay_to_canonical_once(
    world_url: &str,
    room_id: &str,
    host_room_token: &str,
    pop_key: &SigningKey,
    expected_revision: u64,
    patches: &SceneSavePatches,
) -> std::result::Result<RoomScene, SceneSaveError> {
    let base = world_url.trim_end_matches('/');
    let url = format!("{base}/api/v1/rooms/{room_id}/scene/save");
    let path = request_path(&url);
    let authz = format!("Bearer {host_room_token}");
    let parents: Vec<serde_json::Value> = patches
        .parents
        .iter()
        .map(|p| {
            serde_json::json!({
                "entity_id": p.entity_id,
                "parent_entity_id": p.parent_entity_id,
            })
        })
        .collect();
    let fields: Vec<serde_json::Value> = patches
        .fields
        .iter()
        .map(|f| {
            let mut m = serde_json::Map::new();
            m.insert("entity_id".into(), serde_json::json!(f.entity_id));
            if let Some(flags) = f.flags {
                m.insert("flags".into(), serde_json::json!(flags));
            }
            if let Some(r) = f.r {
                m.insert("r".into(), serde_json::json!(r));
            }
            if let Some(g) = f.g {
                m.insert("g".into(), serde_json::json!(g));
            }
            if let Some(b) = f.b {
                m.insert("b".into(), serde_json::json!(b));
            }
            if let Some(a) = f.a {
                m.insert("a".into(), serde_json::json!(a));
            }
            if let Some(ref blake3) = f.blake3 {
                m.insert("blake3".into(), serde_json::json!(blake3));
            }
            serde_json::Value::Object(m)
        })
        .collect();
    let spawns: Vec<serde_json::Value> = patches
        .spawns
        .iter()
        .map(|s| {
            let e = &s.entry;
            serde_json::json!({
                "entity_id": e.entity_id,
                "blake3": e.blake3,
                "spawn_ticket": s.spawn_ticket,
                "x": e.x,
                "y": e.y,
                "z": e.z,
                "qx": e.qx,
                "qy": e.qy,
                "qz": e.qz,
                "qw": e.qw,
                "sx": e.sx,
                "sy": e.sy,
                "sz": e.sz,
                "r": e.r,
                "g": e.g,
                "b": e.b,
                "a": e.a,
                "flags": e.flags,
                "parent_entity_id": e.parent_entity_id,
            })
        })
        .collect();
    let despawns: Vec<serde_json::Value> = patches
        .despawns
        .iter()
        .map(|id| serde_json::json!(id))
        .collect();
    let raw = serde_json::to_vec(&serde_json::json!({
        "expected_revision": expected_revision,
        "parents": parents,
        "fields": fields,
        "spawns": spawns,
        "despawns": despawns,
    }))
    .map_err(|e| SceneSaveError::Other(anyhow!("encode scene save body: {e:#}")))?;
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
            let parsed: SceneResponseJson = resp
                .into_json()
                .map_err(|e| SceneSaveError::Other(anyhow!("parse scene save: {e:#}")))?;
            let mut scene = scene_from_response(parsed);
            scene.owner_sub = None;
            Ok(scene)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            if code == 409 && is_scene_revision_conflict(&body) {
                Err(SceneSaveError::Conflict)
            } else if code == 409 && is_duplicate_blake3(&body) {
                Err(SceneSaveError::DuplicateBlake3)
            } else if code == 403
                && (is_scene_writer_forbidden(&body) || is_spawn_ticket_forbidden(&body))
            {
                Err(SceneSaveError::Forbidden)
            } else if code == 422 {
                match scene_error_code(&body).as_deref() {
                    Some("parent_cycle") => Err(SceneSaveError::ParentCycle),
                    Some("invalid_entry") => Err(SceneSaveError::InvalidEntry),
                    _ => Err(SceneSaveError::Other(anyhow!(
                        "scene save failed status={code} body={body}"
                    ))),
                }
            } else {
                Err(SceneSaveError::Other(anyhow!(
                    "scene save failed status={code} body={body}"
                )))
            }
        }
        Err(err) => Err(SceneSaveError::Other(anyhow!(
            "scene save request failed: {err:#}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(revision: u64, owner_sub: Option<&str>) -> RoomScene {
        RoomScene {
            revision,
            entries: vec![],
            locomotion: None,
            owner_sub: owner_sub.map(str::to_string),
        }
    }

    #[test]
    fn install_preserves_owner_sub_when_incoming_omits() {
        let shared = Arc::new(RwLock::new(scene(1, Some("owner-a"))));
        assert!(install_shared_scene_if_newer(&shared, scene(2, None)).unwrap());
        assert_eq!(shared.read().unwrap().owner_sub.as_deref(), Some("owner-a"));
    }

    #[test]
    fn install_rejects_blank_owner_sub_overwrite() {
        let shared = Arc::new(RwLock::new(scene(1, Some("owner-a"))));
        assert!(install_shared_scene_if_newer(&shared, scene(2, Some("  "))).unwrap());
        assert_eq!(
            shared.read().unwrap().owner_sub.as_deref(),
            Some("owner-a"),
            "blank owner_sub must not wipe prior value"
        );
        assert!(install_shared_scene_if_newer(&shared, scene(3, Some(""))).unwrap());
        assert_eq!(shared.read().unwrap().owner_sub.as_deref(), Some("owner-a"));
    }

    #[test]
    fn install_keeps_prior_owner_sub_even_if_incoming_differs() {
        let shared = Arc::new(RwLock::new(scene(1, Some("owner-a"))));
        assert!(install_shared_scene_if_newer(&shared, scene(2, Some("polluted"))).unwrap());
        assert_eq!(
            shared.read().unwrap().owner_sub.as_deref(),
            Some("owner-a"),
            "mutation / polluted owner_sub must not overwrite GET-seeded value"
        );
    }

    #[test]
    fn install_adopts_incoming_owner_only_when_shared_empty() {
        let shared = Arc::new(RwLock::new(scene(1, None)));
        assert!(install_shared_scene_if_newer(&shared, scene(2, Some("owner-a"))).unwrap());
        assert_eq!(shared.read().unwrap().owner_sub.as_deref(), Some("owner-a"));
    }

    #[test]
    fn install_ignores_stale_revision() {
        let shared = Arc::new(RwLock::new(scene(5, Some("owner-a"))));
        assert!(!install_shared_scene_if_newer(&shared, scene(5, Some("owner-b"))).unwrap());
        assert!(!install_shared_scene_if_newer(&shared, scene(4, Some("owner-b"))).unwrap());
        assert_eq!(shared.read().unwrap().owner_sub.as_deref(), Some("owner-a"));
        assert_eq!(shared.read().unwrap().revision, 5);
    }
}
