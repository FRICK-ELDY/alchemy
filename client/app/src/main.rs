//! Alchemy client — x0019 Leave / liveliness + auth shell / Presence / Grab / dash + x0021 refresh.
//!
//! Guarantees:
//! - Home tab login / register / logout; Room Token only after auth (x0018)
//! - Explicit Leave + Zenoh liveliness on join/logout (x0019); Intent TTL is host fallback
//! - Access + Room Token auto-refresh at ~80% TTL while joined (x0021 P1-1)
//! - CI fallback: `AUTH_USER` or `ALCHEMY_AUTH_TOKEN` skips UI and joins
//! - Subscribes to SceneManifest / PresenceState / RenderFrame; downloads blob; wgpu mesh
//! - Zenoh keys from `ROOM_ID` via protocol `*_key` helpers (x0017)
//! - Desktop WASD/Space/C → Intent; Grab / OpenXR paths unchanged when joined
//! - Camera follows Presence-assigned local user_root; remotes as colored boxes
//! - Surface boot: OpenXR when runtime present, else Desktop; ESC / Menu dash + Exit
//!
//! Non-guarantees: Steal, XR login keyboard, email verify UI, Quest standalone, WSL HMD

mod auth;
mod host_control;
mod jwt_sub;
mod mesh;
mod scene;
mod surface;
mod system_ui;
mod turn;
mod world_hit;

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::AtomicU32;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use alchemy_protocol::{
    frame_key, input_key, liveliness_key, manifest_key, presence_key, validate_room_id, InputEvent,
    Message, PresenceState, RenderFrame, SceneManifest, PROTOCOL_PIN,
};
use anyhow::{Context, Result};
use auth::AuthGate;
use mesh::{blake3_hex, load_mesh_from_glb, verify_blake3, COIN_BLAKE3_HEX};
use scene::{EntityPose, SharedState, LOCAL_USER_ROOT_UNSET, MESH_ENTITY_ID};
use serde::Deserialize;
use zenoh::bytes::ZBytes;
use zenoh::Wait;

const DEFAULT_ZENOH_CONNECT: &str = "tcp/127.0.0.1:7447";
const DEFAULT_WORLD_URL: &str = "http://127.0.0.1:4003";
const DEFAULT_AUTH_URL: &str = "http://127.0.0.1:4002";
const DEFAULT_ASSETS_URL: &str = "http://127.0.0.1:4004";
const DEFAULT_ROOM_ID: &str = "local";
/// Match net frame rate so Grab hold aim stays smooth enough (x0011).
const INPUT_HZ: u64 = 30;
const MANIFEST_WAIT: Duration = Duration::from_millis(400);

#[derive(Debug, Deserialize)]
struct TokenResponse {
    token: String,
    /// Required from world-server; missing field fails deserialize (no silent `0`).
    expires_in: u64,
}

#[derive(Debug, Clone)]
pub struct GuestToken {
    pub token: String,
    pub expires_in: u64,
}

const TOKEN_REFRESH_POLL: Duration = Duration::from_secs(5);

fn zenoh_connect_endpoint() -> String {
    std::env::var("ZENOH_CONNECT").unwrap_or_else(|_| DEFAULT_ZENOH_CONNECT.to_string())
}

fn open_zenoh() -> Result<zenoh::Session> {
    zenoh_connect::open_client(&zenoh_connect_endpoint())
}

pub fn fetch_guest_token(world_url: &str, room_id: &str, auth_token: &str) -> Result<GuestToken> {
    if let Ok(token) = std::env::var("ALCHEMY_ROOM_TOKEN") {
        if !token.is_empty() {
            return Ok(GuestToken {
                token,
                // Env override: no TTL → skip auto-refresh.
                expires_in: 0,
            });
        }
    }

    // x0021: do not POST /rooms — owner is claimed only by host create.
    let base = world_url.trim_end_matches('/');
    let authz = format!("Bearer {auth_token}");
    let token_url = format!("{base}/api/v1/rooms/{room_id}/tokens");
    let resp: TokenResponse = ureq::post(&token_url)
        .set("content-type", "application/json")
        .set("authorization", &authz)
        .send_json(serde_json::json!({ "role": "guest" }))
        .context("issue guest Room Token (is host up and room created?)")?
        .into_json()
        .context("parse token response")?;
    if resp.expires_in == 0 {
        anyhow::bail!("guest Room Token response missing positive expires_in");
    }
    Ok(GuestToken {
        token: resp.token,
        expires_in: resp.expires_in,
    })
}

fn fetch_blob_bytes(assets_url: &str, blake3: &str, auth_token: &str) -> Result<Vec<u8>> {
    let base = assets_url.trim_end_matches('/');
    let url = format!("{base}/api/v1/blobs/{blake3}");
    let authz = format!("Bearer {auth_token}");
    // Follow 302 → R2 presigned GET (x001d). ureq strips Authorization on cross-host redirect.
    let agent = ureq::AgentBuilder::new().redirects(5).build();
    let resp = agent
        .get(&url)
        .set("authorization", &authz)
        .call()
        .context("assets blob download failed")?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .read_to_end(&mut bytes)
        .context("read blob body")?;
    Ok(bytes)
}

fn load_blob_bytes(assets_url: &str, blake3: &str, auth_token: &str) -> Result<Vec<u8>> {
    match fetch_blob_bytes(assets_url, blake3, auth_token) {
        Ok(bytes) => {
            log::info!(
                "downloaded blob via assets GET ({} bytes, blake3={}; may follow R2 redirect)",
                bytes.len(),
                blake3_hex(&bytes)
            );
            Ok(bytes)
        }
        Err(err) => {
            log::warn!("assets download failed ({err:#}); falling back to fixture");
            load_fixture_bytes()
        }
    }
}

fn load_fixture_bytes() -> Result<Vec<u8>> {
    let path = fixture_path()?;
    std::fs::read(&path).with_context(|| format!("read fixture {}", path.display()))
}

fn fixture_path() -> Result<PathBuf> {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("../fixtures/Coin_PolygonalMind.glb");
    Ok(path)
}

fn client_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn spawn_manifest_subscriber(
    session: zenoh::Session,
    key: String,
    shared: Arc<SharedState>,
    blake3_tx: mpsc::SyncSender<String>,
) -> Result<()> {
    let subscriber = session
        .declare_subscriber(key.clone())
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_subscriber manifest failed: {e}"))?;

    thread::Builder::new()
        .name("zenoh-manifest".into())
        .spawn(move || {
            let _session = session;
            log::info!("subscribed to {key}");
            let mut sent_blake3 = false;
            while let Ok(sample) = subscriber.recv() {
                let bytes: ZBytes = sample.payload().clone();
                let payload = bytes.to_bytes();
                match SceneManifest::decode(payload.as_ref()) {
                    Ok(manifest) => {
                        log::info!(
                            "SceneManifest revision={} entries={}",
                            manifest.revision,
                            manifest.entries.len()
                        );
                        if let Ok(mut map) = shared.entities.lock() {
                            for entry in &manifest.entries {
                                map.entry(entry.entity_id).or_insert(EntityPose {
                                    x: entry.x as f32,
                                    y: entry.y as f32,
                                    z: entry.z as f32,
                                    ..Default::default()
                                });
                            }
                        }
                        if !sent_blake3 {
                            if let Some(entry) = manifest
                                .entries
                                .iter()
                                .find(|e| e.entity_id == MESH_ENTITY_ID)
                            {
                                let _ = blake3_tx.send(entry.blake3.clone());
                                sent_blake3 = true;
                            }
                        }
                    }
                    Err(err) => log::warn!("SceneManifest decode failed: {err}"),
                }
            }
        })
        .context("spawn manifest thread")?;
    Ok(())
}

fn spawn_frame_subscriber(
    session: zenoh::Session,
    key: String,
    shared: Arc<SharedState>,
) -> Result<()> {
    let subscriber = session
        .declare_subscriber(key.clone())
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_subscriber frame failed: {e}"))?;

    thread::Builder::new()
        .name("zenoh-frame".into())
        .spawn(move || {
            let _session = session;
            log::info!("subscribed to {key} (delta upserts/removes)");
            while let Ok(sample) = subscriber.recv() {
                let bytes: ZBytes = sample.payload().clone();
                let payload = bytes.to_bytes();
                match RenderFrame::decode(payload.as_ref()) {
                    Ok(frame) => {
                        if let Ok(mut map) = shared.entities.lock() {
                            for e in frame.upserts {
                                map.insert(
                                    e.id,
                                    EntityPose {
                                        x: e.x as f32,
                                        y: e.y as f32,
                                        z: e.z as f32,
                                        r: e.r,
                                        g: e.g,
                                        b: e.b,
                                        a: e.a,
                                    },
                                );
                            }
                            for id in frame.removes {
                                map.remove(&id);
                            }
                        }
                    }
                    Err(err) => log::warn!("RenderFrame decode failed: {err}"),
                }
            }
        })
        .context("spawn frame thread")?;
    Ok(())
}

fn spawn_presence_subscriber(
    session: zenoh::Session,
    key: String,
    shared: Arc<SharedState>,
) -> Result<()> {
    let subscriber = session
        .declare_subscriber(key.clone())
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_subscriber presence failed: {e}"))?;

    thread::Builder::new()
        .name("zenoh-presence".into())
        .spawn(move || {
            let _session = session;
            log::info!("subscribed to {key} (Presence; local_sub from AuthGate)");
            while let Ok(sample) = subscriber.recv() {
                let bytes: ZBytes = sample.payload().clone();
                let payload = bytes.to_bytes();
                match PresenceState::decode(payload.as_ref()) {
                    Ok(state) => {
                        let local_sub = shared
                            .auth
                            .lock()
                            .ok()
                            .and_then(|g| g.local_sub.clone())
                            .unwrap_or_default();
                        if local_sub.is_empty() {
                            if shared.local_user_root() != LOCAL_USER_ROOT_UNSET {
                                shared.set_local_user_root(LOCAL_USER_ROOT_UNSET);
                            }
                            continue;
                        }
                        let mine = state
                            .assignments
                            .iter()
                            .find(|a| a.sub == local_sub)
                            .map(|a| a.entity_id);
                        match mine {
                            Some(entity_id) => {
                                let prev = shared.local_user_root();
                                if prev != entity_id {
                                    shared.set_local_user_root(entity_id);
                                    log::info!(
                                        "Presence: local sub={local_sub} → user_root entity {entity_id} (revision={})",
                                        state.revision
                                    );
                                }
                            }
                            None => {
                                if shared.local_user_root() != LOCAL_USER_ROOT_UNSET {
                                    shared.set_local_user_root(LOCAL_USER_ROOT_UNSET);
                                    log::warn!(
                                        "Presence: local sub={local_sub} not in assignments (revision={})",
                                        state.revision
                                    );
                                }
                            }
                        }
                    }
                    Err(err) => log::warn!("PresenceState decode failed: {err}"),
                }
            }
        })
        .context("spawn presence thread")?;
    Ok(())
}

fn spawn_input_publisher(
    session: zenoh::Session,
    key: String,
    shared: Arc<SharedState>,
) -> Result<()> {
    let publisher = session
        .declare_publisher(key.clone())
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_publisher input failed: {e}"))?;

    thread::Builder::new()
        .name("zenoh-input-pub".into())
        .spawn(move || {
            log::info!(
                "publishing InputEvent intent to {key} @ {INPUT_HZ}Hz (gated by room_token)"
            );
            let period = Duration::from_nanos(1_000_000_000 / INPUT_HZ);
            let mut next = Instant::now();
            loop {
                let now = Instant::now();
                if now < next {
                    thread::sleep(next - now);
                }
                next += period;

                let room_token = shared.auth.lock().ok().and_then(|g| g.room_token.clone());
                let Some(room_token) = room_token else {
                    continue;
                };

                let intent = shared.intent.lock().ok().map(|g| *g).unwrap_or_default();
                let (mx, my, mz, buttons, grab_entity_id, aim_x, aim_y, aim_z) = if intent.suppress
                {
                    (0.0, 0.0, 0.0, 0, 0, 0.0, 0.0, 0.0)
                } else {
                    (
                        intent.move_x,
                        intent.move_y,
                        intent.move_z,
                        intent.buttons,
                        intent.grab_entity_id,
                        intent.aim_x,
                        intent.aim_y,
                        intent.aim_z,
                    )
                };
                let event = InputEvent {
                    room_token,
                    client_time_ms: client_time_ms(),
                    move_x: mx,
                    move_y: my,
                    move_z: mz,
                    buttons,
                    grab_entity_id,
                    aim_x,
                    aim_y,
                    aim_z,
                    leave: false,
                };
                if let Err(err) = publisher.put(event.encode_to_vec()).wait() {
                    log::warn!("input publish failed: {err}");
                    continue;
                }
                // put().wait() only means the router accepted the sample — not that host verified
                // the new jti. Rebind keeps the old token until the *next* put; host also uses a
                // liveliness reclaim grace so Delete(old) before Intent(new) does not free the slot.
                let (need_rebind, need_retire_prev) = shared
                    .auth
                    .lock()
                    .ok()
                    .map(|g| (g.liveliness_rebind, g.liveliness_retire_prev))
                    .unwrap_or((false, false));
                if need_retire_prev {
                    if let Ok(mut prev) = shared.liveliness_prev.lock() {
                        *prev = None;
                    }
                    if let Ok(mut gate) = shared.auth.lock() {
                        gate.liveliness_retire_prev = false;
                    }
                }
                if need_rebind {
                    match declare_room_alive(&shared, &session, /* retain_previous */ true) {
                        Ok(()) => {
                            if let Ok(mut gate) = shared.auth.lock() {
                                gate.liveliness_rebind = false;
                                gate.liveliness_retire_prev = true;
                            }
                        }
                        Err(err) => log::warn!("liveliness rebind after refresh failed: {err:#}"),
                    }
                }
            }
        })
        .context("spawn input publisher")?;
    Ok(())
}

/// Apply auth session + guest Room Token into SharedState (x0018 join).
pub fn join_room_with_access(shared: &SharedState, access_token: &str) -> Result<()> {
    let guest = fetch_guest_token(&shared.world_url, &shared.room_id, access_token)
        .context("fetch guest Room Token")?;
    let local_sub = jwt_sub::sub_unverified(&guest.token)
        .context("read Room Token sub (unverified payload)")?;
    if let Ok(mut gate) = shared.auth.lock() {
        gate.apply_room(guest.token, local_sub.clone(), guest.expires_in);
    }
    log::info!(
        "joined room={} sub={local_sub} expires_in={}",
        shared.room_id,
        guest.expires_in
    );
    Ok(())
}

/// Background access + Room Token refresh at ~80% TTL (x0021 P1-1).
fn spawn_token_refresh(shared: Arc<SharedState>) -> Result<()> {
    thread::Builder::new()
        .name("token-refresh".into())
        .spawn(move || {
            log::info!("token refresh loop every {}s", TOKEN_REFRESH_POLL.as_secs());
            loop {
                thread::sleep(TOKEN_REFRESH_POLL);
                let now = Instant::now();

                let need_access = shared
                    .auth
                    .lock()
                    .ok()
                    .and_then(|g| g.access_deadline.map(|d| now >= d))
                    .unwrap_or(false);
                if need_access {
                    let refresh_token = shared
                        .auth
                        .lock()
                        .ok()
                        .and_then(|g| g.refresh_token.clone());
                    match refresh_token {
                        Some(rt) => match auth::refresh(&shared.auth_url, &rt) {
                            Ok(session) => {
                                if let Ok(mut gate) = shared.auth.lock() {
                                    gate.apply_auth_session(session);
                                }
                                log::info!("access token refreshed");
                            }
                            Err(err) => {
                                log::warn!("access token refresh failed: {err:#}");
                                if let Ok(mut gate) = shared.auth.lock() {
                                    // Back off one poll interval to avoid tight failure loops.
                                    gate.access_deadline = Some(now + TOKEN_REFRESH_POLL);
                                }
                            }
                        },
                        None => {
                            // No refresh_token (e.g. silent env JWT): stop trying access refresh.
                            if let Ok(mut gate) = shared.auth.lock() {
                                gate.access_deadline = None;
                            }
                        }
                    }
                }

                let need_room = shared
                    .auth
                    .lock()
                    .ok()
                    .map(|g| g.is_joined() && g.room_deadline.map(|d| now >= d).unwrap_or(false))
                    .unwrap_or(false);
                if need_room {
                    let access = shared.auth.lock().ok().and_then(|g| g.access_token.clone());
                    let Some(access) = access else {
                        if let Ok(mut gate) = shared.auth.lock() {
                            gate.room_deadline = Some(now + TOKEN_REFRESH_POLL);
                        }
                        continue;
                    };
                    match fetch_guest_token(&shared.world_url, &shared.room_id, &access) {
                        Ok(guest) => {
                            let local_sub = match jwt_sub::sub_unverified(&guest.token) {
                                Ok(s) => s,
                                Err(err) => {
                                    log::warn!("Room Token refresh: bad sub: {err:#}");
                                    if let Ok(mut gate) = shared.auth.lock() {
                                        gate.room_deadline = Some(now + TOKEN_REFRESH_POLL);
                                    }
                                    continue;
                                }
                            };
                            if let Ok(mut gate) = shared.auth.lock() {
                                if gate.is_joined() {
                                    gate.apply_room_refresh(
                                        guest.token,
                                        local_sub,
                                        guest.expires_in,
                                    );
                                    log::info!(
                                        "Room Token refreshed expires_in={} (liveliness rebind pending)",
                                        guest.expires_in
                                    );
                                }
                            }
                        }
                        Err(err) => {
                            log::warn!("Room Token refresh failed: {err:#}");
                            if let Ok(mut gate) = shared.auth.lock() {
                                gate.room_deadline = Some(now + TOKEN_REFRESH_POLL);
                            }
                        }
                    }
                }
            }
        })
        .context("spawn token refresh")?;
    Ok(())
}

/// Declare Zenoh liveliness for current Room Token `sub`+`jti` (x0022). No-op if not joined.
///
/// `retain_previous`: only for Room Token **refresh** rebind. Stashes the old token in
/// `liveliness_prev` until the next Intent put. Join / re-login pass `false` and drop any
/// previous key immediately (no dual-key leak until leave).
pub fn declare_room_alive(
    shared: &SharedState,
    session: &zenoh::Session,
    retain_previous: bool,
) -> Result<()> {
    let room_token = shared
        .auth
        .lock()
        .ok()
        .and_then(|g| g.room_token.clone())
        .context("declare liveliness: not joined")?;
    let (sub, jti) = jwt_sub::sub_jti_unverified(&room_token)
        .context("declare liveliness: read sub/jti")?;
    let key = liveliness_key(shared.room_id.as_ref(), &sub, &jti);
    let token = session
        .liveliness()
        .declare_token(key.clone())
        .wait()
        .map_err(|e| anyhow::anyhow!("declare liveliness token failed: {e}"))?;
    if let Ok(mut slot) = shared.liveliness.lock() {
        let old = slot.replace(token);
        if let Some(old) = old {
            if retain_previous {
                if let Ok(mut prev) = shared.liveliness_prev.lock() {
                    *prev = Some(old);
                }
            }
            // else: drop `old` immediately (join / re-login)
        }
    }
    log::info!("liveliness declared key={key} retain_previous={retain_previous}");
    Ok(())
}

/// Explicit Leave put + liveliness undeclare + local clear (x0019).
pub fn leave_room(shared: &SharedState, session: &zenoh::Session) {
    let room_token = shared.auth.lock().ok().and_then(|g| g.room_token.clone());
    if let Some(room_token) = room_token {
        let key = input_key(shared.room_id.as_ref());
        match session.declare_publisher(key.clone()).wait() {
            Ok(publisher) => {
                let event = InputEvent {
                    room_token,
                    client_time_ms: client_time_ms(),
                    move_x: 0.0,
                    move_y: 0.0,
                    move_z: 0.0,
                    buttons: 0,
                    grab_entity_id: 0,
                    aim_x: 0.0,
                    aim_y: 0.0,
                    aim_z: 0.0,
                    leave: true,
                };
                if let Err(err) = publisher.put(event.encode_to_vec()).wait() {
                    log::warn!("Leave InputEvent publish failed: {err}");
                } else {
                    log::info!("Leave InputEvent published on {key}");
                }
            }
            Err(err) => log::warn!("Leave publisher declare failed: {err}"),
        }
    }
    // Drop liveliness before clearing auth so host sees Delete promptly.
    if let Ok(mut token) = shared.liveliness.lock() {
        *token = None;
    }
    if let Ok(mut prev) = shared.liveliness_prev.lock() {
        *prev = None;
    }
    shared.leave_room_local();
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let world_url = std::env::var("WORLD_URL").unwrap_or_else(|_| DEFAULT_WORLD_URL.to_string());
    let auth_url = std::env::var("AUTH_URL").unwrap_or_else(|_| DEFAULT_AUTH_URL.to_string());
    let assets_url = std::env::var("ASSETS_URL").unwrap_or_else(|_| DEFAULT_ASSETS_URL.to_string());
    let room_id_raw = std::env::var("ROOM_ID").unwrap_or_else(|_| DEFAULT_ROOM_ID.to_string());
    let room_id = validate_room_id(&room_id_raw)
        .map_err(|e| anyhow::anyhow!("invalid ROOM_ID={room_id_raw:?}: {e}"))?
        .to_string();
    let fallback_blake3 =
        std::env::var("ALCHEMY_BLOB_BLAKE3").unwrap_or_else(|_| COIN_BLAKE3_HEX.to_string());

    let manifest_key_expr = manifest_key(&room_id);
    let frame_key_expr = frame_key(&room_id);
    let input_key_expr = input_key(&room_id);
    let presence_key_expr = presence_key(&room_id);

    let silent = auth::try_silent_session(&auth_url).context("silent auth")?;
    let auth_mode = if silent.is_some() { "silent" } else { "ui" };
    log::info!(
        "client x0022 starting; protocol_pin={PROTOCOL_PIN} room={room_id} auth_mode={auth_mode} manifest={manifest_key_expr} frame={frame_key_expr} input={input_key_expr} presence={presence_key_expr}"
    );

    let mut gate = AuthGate::default();
    if let Some(session) = silent {
        gate.apply_auth_session(session);
    }

    let shared = Arc::new(SharedState {
        local_user_root: AtomicU32::new(LOCAL_USER_ROOT_UNSET),
        room_id: Arc::<str>::from(room_id),
        auth_url: Arc::<str>::from(auth_url),
        world_url: Arc::<str>::from(world_url),
        assets_url: Arc::<str>::from(assets_url.clone()),
        auth: Mutex::new(gate),
        liveliness: Mutex::new(None),
        liveliness_prev: Mutex::new(None),
        entities: Mutex::new(HashMap::new()),
        look: Mutex::new(Default::default()),
        intent: Mutex::new(Default::default()),
    });

    if shared
        .auth
        .lock()
        .ok()
        .and_then(|g| g.access_token.clone())
        .is_some()
    {
        let access = shared
            .auth
            .lock()
            .ok()
            .and_then(|g| g.access_token.clone())
            .expect("access just checked");
        join_room_with_access(&shared, &access).context("silent join room")?;
    }

    let (blake3_tx, blake3_rx) = mpsc::sync_channel::<String>(1);

    spawn_token_refresh(Arc::clone(&shared))?;

    let session = open_zenoh().context("open zenoh session")?;
    if shared
        .auth
        .lock()
        .ok()
        .map(|g| g.is_joined())
        .unwrap_or(false)
    {
        declare_room_alive(&shared, &session, false).context("silent liveliness")?;
    }
    spawn_manifest_subscriber(
        session.clone(),
        manifest_key_expr,
        Arc::clone(&shared),
        blake3_tx,
    )?;
    spawn_presence_subscriber(session.clone(), presence_key_expr, Arc::clone(&shared))?;
    spawn_frame_subscriber(session.clone(), frame_key_expr, Arc::clone(&shared))?;
    spawn_input_publisher(session.clone(), input_key_expr, Arc::clone(&shared))?;

    let blob_blake3 = match blake3_rx.recv_timeout(MANIFEST_WAIT) {
        Ok(hex) => {
            log::info!("Manifest blake3 for entity {MESH_ENTITY_ID}: {hex}");
            hex
        }
        Err(_) => {
            log::warn!(
                "no SceneManifest within {:?}; using fallback blake3={fallback_blake3}",
                MANIFEST_WAIT
            );
            fallback_blake3
        }
    };

    let access_for_blob = shared
        .auth
        .lock()
        .ok()
        .and_then(|g| g.access_token.clone())
        .unwrap_or_default();
    let bytes = {
        let primary = if access_for_blob.is_empty() {
            log::info!("no auth yet — loading fixture mesh (login via Home tab)");
            load_fixture_bytes()?
        } else {
            load_blob_bytes(&assets_url, &blob_blake3, &access_for_blob)?
        };
        match verify_blake3(&primary, &blob_blake3) {
            Ok(()) => primary,
            Err(_) => {
                let fixture = load_fixture_bytes()?;
                verify_blake3(&fixture, COIN_BLAKE3_HEX).context("fixture blake3")?;
                fixture
            }
        }
    };
    let mesh = load_mesh_from_glb(&bytes).context("decode GLB")?;
    log::info!(
        "mesh ready: {} verts, {} indices, radius={:.3}",
        mesh.positions.len(),
        mesh.indices.len(),
        mesh.radius
    );

    let kind = surface::choose();
    surface::run(kind, mesh, shared, Some(session))?;
    Ok(())
}
