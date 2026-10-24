//! Local host-server — Slice D (+ E host_control shutdown stub).
//!
//! Guarantees:
//! - Internal Rapier tick at 60Hz; network delta publish at 30Hz
//! - Input: Room Token verify + `is_finite` (NaN/Inf dropped before physics)
//! - Low-frequency SceneManifest (blake3 refs; no GLB bytes)
//! - Delta RenderFrame: only moved entities (C-C contract retained)
//! - Slice E: `host_control` Subscribe → `shutdown` exits process
//!
//! Non-guarantees:
//! - Formula / character controller / signed Manifest / verifier / launcher

mod finite;
mod host_control;
mod room_token;
mod sim;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use alchemy_protocol::{
    Entity, InputEvent, Message, RenderFrame, SceneEntry, SceneManifest, FRAME_KEY_LOCAL,
    HOST_CONTROL_KEY_LOCAL, INPUT_KEY_LOCAL, MANIFEST_KEY_LOCAL, PROTOCOL_PIN,
};
use anyhow::{Context, Result};
use finite::input_event_is_finite;
use room_token::{default_auth_url, fetch_auth_token, fetch_host_token, RoomTokenVerifier};
use sim::PhysicsWorld;
use zenoh::bytes::ZBytes;
use zenoh::Config;
use zenoh::Wait;

const PHYSICS_HZ: u64 = 60;
const NET_HZ: u64 = 30;
/// Publish a network frame every N physics steps (60/30 = 2).
const PHYSICS_PER_NET: u64 = PHYSICS_HZ / NET_HZ;
const CONNECT: &str = "tcp/127.0.0.1:7447";
const DEFAULT_WORLD_URL: &str = "http://127.0.0.1:4003";
const DEFAULT_ROOM_ID: &str = "local";
/// Coin_PolygonalMind.glb (CC0) — same as client fixtures.
const COIN_BLAKE3_HEX: &str = "fa24848fa7b971718ce6e8dc9bd2d211808fd64e4ddf555feb66d4f82ee1e72a";
/// Pose change below this is treated as static (sleep / omit from upserts).
const MOVE_EPSILON: f64 = 1e-4;
/// Republish Manifest so late joiners still see blob refs (still ≪ frame rate).
const MANIFEST_REPUBLISH: Duration = Duration::from_secs(5);

const ENTITY_MOVING: u32 = 1;
const ENTITY_STATIC: u32 = 2;

#[derive(Clone, Copy, Debug)]
struct PoseOverride {
    x: f32,
    y: f32,
    z: f32,
    active: bool,
}

impl Default for PoseOverride {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            active: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Pose {
    x: f64,
    y: f64,
    z: f64,
    r: f32,
    g: f32,
    b: f32,
    a: f32,
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

fn host_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn static_marker_pose() -> Pose {
    Pose {
        x: 3.0,
        y: 0.5,
        z: 0.0,
        r: 0.9,
        g: 0.3,
        b: 0.2,
        a: 1.0,
    }
}

fn moving_pose_from_sim(world: &PhysicsWorld) -> Pose {
    let (x, y, z) = world.moving_translation();
    Pose {
        x: f64::from(x),
        y: f64::from(y),
        z: f64::from(z),
        r: 0.2,
        g: 0.9,
        b: 0.4,
        a: 1.0,
    }
}

fn to_entity(id: u32, pose: Pose) -> Entity {
    Entity {
        id,
        x: pose.x,
        y: pose.y,
        z: pose.z,
        r: pose.r,
        g: pose.g,
        b: pose.b,
        a: pose.a,
    }
}

fn pose_changed(prev: Pose, next: Pose) -> bool {
    (prev.x - next.x).abs() > MOVE_EPSILON
        || (prev.y - next.y).abs() > MOVE_EPSILON
        || (prev.z - next.z).abs() > MOVE_EPSILON
}

fn build_manifest(room_id: &str, blake3: &str) -> SceneManifest {
    SceneManifest {
        room_id: room_id.to_string(),
        revision: 1,
        entries: vec![SceneEntry {
            entity_id: ENTITY_MOVING,
            blake3: blake3.to_string(),
            x: 0.0,
            y: f64::from(sim::SPAWN_Y),
            z: 0.0,
        }],
    }
}

fn spawn_input_gate(
    session: zenoh::Session,
    verifier: Arc<RoomTokenVerifier>,
    pose: Arc<Mutex<PoseOverride>>,
) -> Result<()> {
    let subscriber = session
        .declare_subscriber(INPUT_KEY_LOCAL)
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_subscriber input failed: {e}"))?;

    thread::Builder::new()
        .name("zenoh-input".into())
        .spawn(move || {
            let _session = session;
            log::info!("subscribed to {INPUT_KEY_LOCAL} (Room Token + is_finite gate)");
            let mut accepted: u64 = 0;
            let mut dropped_token: u64 = 0;
            let mut dropped_finite: u64 = 0;
            let mut last_log = Instant::now();
            while let Ok(sample) = subscriber.recv() {
                let bytes: ZBytes = sample.payload().clone();
                let payload = bytes.to_bytes();
                match InputEvent::decode(payload.as_ref()) {
                    Ok(event) => match verifier.verify(&event.room_token) {
                        Ok(_claims) => {
                            if !input_event_is_finite(&event) {
                                dropped_finite += 1;
                                log::warn!(
                                    "drop InputEvent: non-finite move_*=({:.3},{:.3},{:.3})",
                                    event.move_x,
                                    event.move_y,
                                    event.move_z
                                );
                            } else if let Ok(mut guard) = pose.lock() {
                                guard.x = event.move_x;
                                guard.y = event.move_y;
                                guard.z = event.move_z;
                                guard.active = true;
                                accepted += 1;
                            }
                        }
                        Err(err) => {
                            dropped_token += 1;
                            log::warn!("drop InputEvent: Room Token invalid: {err:#}");
                        }
                    },
                    Err(err) => {
                        dropped_token += 1;
                        log::warn!("drop InputEvent: decode failed: {err}");
                    }
                }
                if last_log.elapsed() >= Duration::from_secs(5) {
                    log::info!(
                        "input gate: accepted={accepted} dropped_token={dropped_token} dropped_non_finite={dropped_finite} (last ~5s)"
                    );
                    accepted = 0;
                    dropped_token = 0;
                    dropped_finite = 0;
                    last_log = Instant::now();
                }
            }
        })
        .context("spawn input thread")?;
    Ok(())
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let world_url = std::env::var("WORLD_URL").unwrap_or_else(|_| DEFAULT_WORLD_URL.to_string());
    let room_id = std::env::var("ROOM_ID").unwrap_or_else(|_| DEFAULT_ROOM_ID.to_string());
    let auth_url = default_auth_url();
    let blob_blake3 =
        std::env::var("ALCHEMY_BLOB_BLAKE3").unwrap_or_else(|_| COIN_BLAKE3_HEX.to_string());

    log::info!(
        "host-server Slice D/E starting; protocol_pin={PROTOCOL_PIN} physics_hz={PHYSICS_HZ} net_hz={NET_HZ} manifest={MANIFEST_KEY_LOCAL} frame={FRAME_KEY_LOCAL} input={INPUT_KEY_LOCAL} host_control={HOST_CONTROL_KEY_LOCAL} world={world_url} auth={auth_url} room={room_id}"
    );

    let auth_token = fetch_auth_token(&auth_url).context("fetch auth JWT")?;
    let host_token = fetch_host_token(&world_url, &room_id, &auth_token)
        .context("fetch host Room Token (is world-server up? auth Bearer required)")?;
    let verifier = Arc::new(RoomTokenVerifier::new(&world_url, &room_id));
    verifier.refresh_jwks().context("initial JWKS fetch")?;
    verifier
        .verify(&host_token)
        .context("host Room Token self-check")?;
    log::info!("auth JWT + host Room Token acquired and verified");

    let pose = Arc::new(Mutex::new(PoseOverride::default()));

    let session = open_zenoh().context("open zenoh session")?;
    spawn_input_gate(session.clone(), Arc::clone(&verifier), Arc::clone(&pose))?;
    host_control::spawn_listener(session.clone())?;

    let manifest_pub = session
        .declare_publisher(MANIFEST_KEY_LOCAL)
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_publisher manifest failed: {e}"))?;
    let frame_pub = session
        .declare_publisher(FRAME_KEY_LOCAL)
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_publisher frame failed: {e}"))?;
    // `session` stays alive on this thread for publishers + worker clones.

    let manifest = build_manifest(&room_id, &blob_blake3);
    let manifest_bytes = manifest.encode_to_vec();
    manifest_pub
        .put(manifest_bytes.clone())
        .wait()
        .map_err(|e| anyhow::anyhow!("manifest publish failed: {e}"))?;
    log::info!(
        "published SceneManifest revision={} entries={} blake3={}",
        manifest.revision,
        manifest.entries.len(),
        blob_blake3
    );
    let mut last_manifest = Instant::now();

    let mut physics = PhysicsWorld::new();
    log::info!(
        "Rapier world ready (floor + entity {ENTITY_MOVING} dropping from y={:.1})",
        sim::SPAWN_Y
    );

    let physics_period = Duration::from_nanos(1_000_000_000 / PHYSICS_HZ);
    let mut next_physics = Instant::now();
    let mut physics_steps: u64 = 0;
    let mut net_tick: u64 = 0;
    let mut last_log = Instant::now();
    let mut publishes: u64 = 0;
    let mut upsert_entities: u64 = 0;
    let mut empty_delta_frames: u64 = 0;
    let mut physics_steps_logged: u64 = 0;
    let mut last_sent: HashMap<u32, Pose> = HashMap::new();

    loop {
        let now = Instant::now();
        if now < next_physics {
            std::thread::sleep(next_physics - now);
        }
        next_physics += physics_period;

        if last_manifest.elapsed() >= MANIFEST_REPUBLISH {
            manifest_pub
                .put(manifest_bytes.clone())
                .wait()
                .map_err(|e| anyhow::anyhow!("manifest republish failed: {e}"))?;
            last_manifest = Instant::now();
            log::info!(
                "republished SceneManifest revision={} (late-joiner assist)",
                manifest.revision
            );
        }

        let overlay = pose.lock().map(|p| *p).unwrap_or_default();
        if overlay.active && (overlay.x.abs() > 1e-4 || overlay.z.abs() > 1e-4) {
            // Y comes from gravity / bounce — horizontal input only.
            physics.apply_input_horizontal(overlay.x, overlay.z);
        }
        physics.step();
        physics_steps = physics_steps.wrapping_add(1);
        physics_steps_logged += 1;

        if !physics_steps.is_multiple_of(PHYSICS_PER_NET) {
            continue;
        }

        let poses = [
            (ENTITY_MOVING, moving_pose_from_sim(&physics)),
            (ENTITY_STATIC, static_marker_pose()),
        ];

        let mut upserts = Vec::new();
        for (id, next_pose) in poses {
            let should_send = match last_sent.get(&id) {
                None => true,
                Some(prev) => pose_changed(*prev, next_pose),
            };
            if should_send {
                upserts.push(to_entity(id, next_pose));
                last_sent.insert(id, next_pose);
            }
        }

        let upsert_len = upserts.len() as u64;
        if upserts.is_empty() {
            empty_delta_frames += 1;
            net_tick = net_tick.wrapping_add(1);
        } else {
            upsert_entities += upsert_len;
            let frame = RenderFrame {
                tick: net_tick,
                host_time_ms: host_time_ms(),
                upserts,
                removes: vec![],
            };
            frame_pub
                .put(frame.encode_to_vec())
                .wait()
                .map_err(|e| anyhow::anyhow!("frame publish failed: {e}"))?;
            net_tick = net_tick.wrapping_add(1);
            publishes += 1;
        }

        if last_log.elapsed() >= Duration::from_secs(1) {
            let (mx, my, mz) = physics.moving_translation();
            log::info!(
                "physics: steps={physics_steps_logged} pos=({mx:.3},{my:.3},{mz:.3}); delta: frames={publishes} upsert_entities={upsert_entities} skipped_empty={empty_delta_frames} net_tick={net_tick}"
            );
            publishes = 0;
            upsert_entities = 0;
            empty_delta_frames = 0;
            physics_steps_logged = 0;
            last_log = Instant::now();
        }
    }
}
