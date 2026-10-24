//! Alchemy client — x0009 Desktop Noclip + OpenXR dash/Exit + C-C/D render path.
//!
//! Guarantees:
//! - Subscribes to SceneManifest / RenderFrame; downloads blob; wgpu mesh on Desktop
//! - Desktop WASD/Space/C → InputEvent Intent; dash suppresses move (mouse look deferred)
//! - Camera follows host user_root pose (fixed default look)
//! - Surface boot: OpenXR when runtime present, else Desktop; ESC / Menu dash + Exit
//!
//! Non-guarantees: VR loco, Grab/tools, Quest standalone, WSL HMD, settings contents

mod host_control;
mod mesh;
mod scene;
mod surface;
mod system_ui;

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use alchemy_protocol::{
    InputEvent, Message, RenderFrame, SceneManifest, FRAME_KEY_LOCAL, INPUT_KEY_LOCAL,
    MANIFEST_KEY_LOCAL, PROTOCOL_PIN,
};
use anyhow::{Context, Result};
use mesh::{blake3_hex, load_mesh_from_glb, verify_blake3, COIN_BLAKE3_HEX};
use scene::{EntityPose, SharedState, MESH_ENTITY_ID, USER_ROOT_ENTITY_ID};
use serde::Deserialize;
use zenoh::bytes::ZBytes;
use zenoh::Config;
use zenoh::Wait;

const CONNECT: &str = "tcp/127.0.0.1:7447";
const DEFAULT_WORLD_URL: &str = "http://127.0.0.1:4003";
const DEFAULT_AUTH_URL: &str = "http://127.0.0.1:4002";
const DEFAULT_ASSETS_URL: &str = "http://127.0.0.1:4004";
const DEFAULT_ROOM_ID: &str = "local";
const DEFAULT_AUTH_USER: &str = "alice";
const DEFAULT_AUTH_PASSWORD: &str = "Password1";
const INPUT_HZ: u64 = 10;
const MANIFEST_WAIT: Duration = Duration::from_millis(400);

#[derive(Debug, Deserialize)]
struct TokenResponse {
    token: String,
}

#[derive(Debug, Deserialize)]
struct AuthSession {
    access_token: String,
}

fn open_zenoh() -> Result<zenoh::Session> {
    let mut config = Config::default();
    config
        .insert_json5("mode", r#""client""#)
        .map_err(|e| anyhow::anyhow!("zenoh mode: {e}"))?;
    config
        .insert_json5("connect/endpoints", &format!(r#"["{CONNECT}"]"#))
        .map_err(|e| anyhow::anyhow!("zenoh connect: {e}"))?;
    zenoh::open(config)
        .wait()
        .map_err(|e| anyhow::anyhow!("zenoh open failed: {e}"))
}

fn fetch_auth_token(auth_url: &str) -> Result<String> {
    if let Ok(token) = std::env::var("ALCHEMY_AUTH_TOKEN") {
        if !token.is_empty() {
            return Ok(token);
        }
    }

    let user = std::env::var("AUTH_USER").unwrap_or_else(|_| DEFAULT_AUTH_USER.to_string());
    let password =
        std::env::var("AUTH_PASSWORD").unwrap_or_else(|_| DEFAULT_AUTH_PASSWORD.to_string());
    let base = auth_url.trim_end_matches('/');
    let login_url = format!("{base}/api/v1/auth/login");
    let resp: AuthSession = ureq::post(&login_url)
        .set("content-type", "application/json")
        .send_json(serde_json::json!({
            "identifier": user,
            "password": password
        }))
        .context("auth login failed (is auth-server up?)")?
        .into_json()
        .context("parse auth session")?;
    Ok(resp.access_token)
}

fn fetch_guest_token(world_url: &str, room_id: &str, auth_token: &str) -> Result<String> {
    if let Ok(token) = std::env::var("ALCHEMY_ROOM_TOKEN") {
        if !token.is_empty() {
            return Ok(token);
        }
    }

    let base = world_url.trim_end_matches('/');
    let authz = format!("Bearer {auth_token}");
    let create_url = format!("{base}/api/v1/rooms");
    let _ = ureq::post(&create_url)
        .set("content-type", "application/json")
        .set("authorization", &authz)
        .send_json(serde_json::json!({ "room_id": room_id }));

    let token_url = format!("{base}/api/v1/rooms/{room_id}/tokens");
    let resp: TokenResponse = ureq::post(&token_url)
        .set("content-type", "application/json")
        .set("authorization", &authz)
        .send_json(serde_json::json!({ "role": "guest" }))
        .context("issue guest Room Token")?
        .into_json()
        .context("parse token response")?;
    Ok(resp.token)
}

fn fetch_blob_bytes(assets_url: &str, blake3: &str, auth_token: &str) -> Result<Vec<u8>> {
    let base = assets_url.trim_end_matches('/');
    let url = format!("{base}/api/v1/blobs/{blake3}");
    let authz = format!("Bearer {auth_token}");
    let resp = ureq::get(&url)
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
                "downloaded blob from assets-server ({} bytes, blake3={})",
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
    shared: Arc<SharedState>,
    blake3_tx: mpsc::SyncSender<String>,
) -> Result<()> {
    let subscriber = session
        .declare_subscriber(MANIFEST_KEY_LOCAL)
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_subscriber manifest failed: {e}"))?;

    thread::Builder::new()
        .name("zenoh-manifest".into())
        .spawn(move || {
            let _session = session;
            log::info!("subscribed to {MANIFEST_KEY_LOCAL}");
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

fn spawn_frame_subscriber(session: zenoh::Session, shared: Arc<SharedState>) -> Result<()> {
    let subscriber = session
        .declare_subscriber(FRAME_KEY_LOCAL)
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_subscriber frame failed: {e}"))?;

    thread::Builder::new()
        .name("zenoh-frame".into())
        .spawn(move || {
            let _session = session;
            log::info!("subscribed to {FRAME_KEY_LOCAL} (delta upserts/removes)");
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

fn spawn_input_publisher(
    session: zenoh::Session,
    room_token: String,
    shared: Arc<SharedState>,
) -> Result<()> {
    let publisher = session
        .declare_publisher(INPUT_KEY_LOCAL)
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_publisher input failed: {e}"))?;

    thread::Builder::new()
        .name("zenoh-input-pub".into())
        .spawn(move || {
            let _session = session;
            log::info!("publishing InputEvent intent to {INPUT_KEY_LOCAL} @ {INPUT_HZ}Hz");
            let period = Duration::from_nanos(1_000_000_000 / INPUT_HZ);
            let mut next = Instant::now();
            loop {
                let now = Instant::now();
                if now < next {
                    thread::sleep(next - now);
                }
                next += period;

                let intent = shared.intent.lock().ok().map(|g| *g).unwrap_or_default();
                let (mx, my, mz, buttons) = if intent.suppress {
                    (0.0, 0.0, 0.0, 0)
                } else {
                    (intent.move_x, intent.move_y, intent.move_z, intent.buttons)
                };
                let event = InputEvent {
                    room_token: room_token.clone(),
                    client_time_ms: client_time_ms(),
                    move_x: mx,
                    move_y: my,
                    move_z: mz,
                    buttons,
                };
                if let Err(err) = publisher.put(event.encode_to_vec()).wait() {
                    log::warn!("input publish failed: {err}");
                }
            }
        })
        .context("spawn input publisher")?;
    Ok(())
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let world_url = std::env::var("WORLD_URL").unwrap_or_else(|_| DEFAULT_WORLD_URL.to_string());
    let auth_url = std::env::var("AUTH_URL").unwrap_or_else(|_| DEFAULT_AUTH_URL.to_string());
    let assets_url = std::env::var("ASSETS_URL").unwrap_or_else(|_| DEFAULT_ASSETS_URL.to_string());
    let room_id = std::env::var("ROOM_ID").unwrap_or_else(|_| DEFAULT_ROOM_ID.to_string());
    let fallback_blake3 =
        std::env::var("ALCHEMY_BLOB_BLAKE3").unwrap_or_else(|_| COIN_BLAKE3_HEX.to_string());

    log::info!(
        "client x0009 starting; protocol_pin={PROTOCOL_PIN} manifest={MANIFEST_KEY_LOCAL} frame={FRAME_KEY_LOCAL} input={INPUT_KEY_LOCAL} user_root={USER_ROOT_ENTITY_ID}"
    );

    let auth_token = fetch_auth_token(&auth_url).context("fetch auth JWT")?;
    let room_token =
        fetch_guest_token(&world_url, &room_id, &auth_token).context("fetch guest Room Token")?;
    log::info!("auth JWT + guest Room Token acquired");

    let shared = Arc::new(SharedState {
        entities: Mutex::new(HashMap::new()),
        look: Mutex::new(Default::default()),
        intent: Mutex::new(Default::default()),
    });
    let (blake3_tx, blake3_rx) = mpsc::sync_channel::<String>(1);

    let session = open_zenoh().context("open zenoh session")?;
    spawn_manifest_subscriber(session.clone(), Arc::clone(&shared), blake3_tx)?;
    spawn_frame_subscriber(session.clone(), Arc::clone(&shared))?;
    spawn_input_publisher(session.clone(), room_token, Arc::clone(&shared))?;

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

    let bytes = load_blob_bytes(&assets_url, &blob_blake3, &auth_token)?;
    verify_blake3(&bytes, &blob_blake3).context("reject blob: BLAKE3 mismatch")?;
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
