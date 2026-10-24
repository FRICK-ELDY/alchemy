//! Local host-server — Slice C-C.
//!
//! Guarantees:
//! - Low-frequency SceneManifest publish (entity_id → blake3; no GLB bytes)
//! - 30 Hz RenderFrame delta: only moved / spawned entities in `upserts`
//! - Subscribe InputEvent; verify Room Token; valid input overrides entity 1 pose
//!
//! Non-guarantees:
//! - Rapier / Formula tick
//! - Signed Manifest (world-server)
//! - Visibility ACL / verifier

mod room_token;

use std::collections::HashMap;
use std::f64::consts::TAU;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use alchemy_protocol::{
    Entity, InputEvent, Message, RenderFrame, SceneEntry, SceneManifest, FRAME_KEY_LOCAL,
    INPUT_KEY_LOCAL, MANIFEST_KEY_LOCAL, PROTOCOL_PIN,
};
use anyhow::{Context, Result};
use room_token::{default_auth_url, fetch_auth_token, fetch_host_token, RoomTokenVerifier};
use zenoh::bytes::ZBytes;
use zenoh::Config;
use zenoh::Wait;

const TICK_HZ: u64 = 30;
const CONNECT: &str = "tcp/127.0.0.1:7447";
const DEFAULT_WORLD_URL: &str = "http://127.0.0.1:4003";
const DEFAULT_ROOM_ID: &str = "local";
/// Coin_PolygonalMind.glb (CC0) — same as client fixtures.
const COIN_BLAKE3_HEX: &str =
    "fa24848fa7b971718ce6e8dc9bd2d211808fd64e4ddf555feb66d4f82ee1e72a";
/// Pose change below this is treated as static (sleep / omit from upserts).
const MOVE_EPSILON: f64 = 1e-4;
/// Republish Manifest so late joiners still see blob refs (still ≪ frame rate).
const MANIFEST_REPUBLISH: Duration = Duration::from_secs(5);

const ENTITY_MOVING: u32 = 1;
const ENTITY_STATIC: u32 = 2;

#[derive(Clone, Copy, Debug)]
struct PoseOverride {
    x: f64,
    y: f64,
    z: f64,
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

fn moving_pose(tick: u64, overlay: PoseOverride) -> Pose {
    if overlay.active {
        return Pose {
            x: overlay.x,
            y: overlay.y,
            z: overlay.z,
            r: 0.2,
            g: 0.9,
            b: 0.4,
            a: 1.0,
        };
    }
    let t = tick as f64 / TICK_HZ as f64;
    let angle = t * TAU / 4.0;
    Pose {
        x: angle.cos() * 1.5,
        y: angle.sin() * 0.5,
        z: angle.sin() * 1.5,
        r: 0.2,
        g: 0.7,
        b: 1.0,
        a: 1.0,
    }
}

fn static_pose() -> Pose {
    Pose {
        x: 3.0,
        y: 0.0,
        z: 0.0,
        r: 0.9,
        g: 0.3,
        b: 0.2,
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
            y: 0.0,
            z: 0.0,
        }],
    }
}

fn spawn_input_gate(
    verifier: Arc<RoomTokenVerifier>,
    pose: Arc<Mutex<PoseOverride>>,
) -> Result<()> {
    let session = open_zenoh().context("open zenoh for input")?;
    let subscriber = session
        .declare_subscriber(INPUT_KEY_LOCAL)
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_subscriber input failed: {e}"))?;

    thread::Builder::new()
        .name("zenoh-input".into())
        .spawn(move || {
            log::info!("subscribed to {INPUT_KEY_LOCAL} (Room Token gate)");
            let mut accepted: u64 = 0;
            let mut dropped: u64 = 0;
            let mut last_log = Instant::now();
            while let Ok(sample) = subscriber.recv() {
                let bytes: ZBytes = sample.payload().clone();
                let payload = bytes.to_bytes();
                match InputEvent::decode(payload.as_ref()) {
                    Ok(event) => match verifier.verify(&event.room_token) {
                        Ok(_claims) => {
                            if let Ok(mut guard) = pose.lock() {
                                guard.x = event.move_x as f64;
                                guard.y = event.move_y as f64;
                                guard.z = event.move_z as f64;
                                guard.active = true;
                            }
                            accepted += 1;
                        }
                        Err(err) => {
                            dropped += 1;
                            log::warn!("drop InputEvent: Room Token invalid: {err:#}");
                        }
                    },
                    Err(err) => {
                        dropped += 1;
                        log::warn!("drop InputEvent: decode failed: {err}");
                    }
                }
                if last_log.elapsed() >= Duration::from_secs(5) {
                    log::info!("input gate: accepted={accepted} dropped={dropped} (last ~5s)");
                    accepted = 0;
                    dropped = 0;
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
        "host-server Slice C-C starting; protocol_pin={PROTOCOL_PIN} manifest={MANIFEST_KEY_LOCAL} frame={FRAME_KEY_LOCAL} input={INPUT_KEY_LOCAL} world={world_url} auth={auth_url} room={room_id}"
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
    spawn_input_gate(Arc::clone(&verifier), Arc::clone(&pose))?;

    let session = open_zenoh().context("open zenoh session")?;
    let manifest_pub = session
        .declare_publisher(MANIFEST_KEY_LOCAL)
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_publisher manifest failed: {e}"))?;
    let frame_pub = session
        .declare_publisher(FRAME_KEY_LOCAL)
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_publisher frame failed: {e}"))?;

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

    let tick_period = Duration::from_nanos(1_000_000_000 / TICK_HZ);
    let mut tick: u64 = 0;
    let mut next = Instant::now();
    let mut last_log = Instant::now();
    let mut publishes: u64 = 0;
    let mut upsert_entities: u64 = 0;
    let mut empty_delta_frames: u64 = 0;
    let mut last_sent: HashMap<u32, Pose> = HashMap::new();

    loop {
        let now = Instant::now();
        if now < next {
            std::thread::sleep(next - now);
        }
        next += tick_period;

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
        let poses = [
            (ENTITY_MOVING, moving_pose(tick, overlay)),
            (ENTITY_STATIC, static_pose()),
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
            // Static sleep: nothing moved — do not publish an empty high-frequency frame.
            empty_delta_frames += 1;
            tick = tick.wrapping_add(1);
        } else {
            upsert_entities += upsert_len;
            let frame = RenderFrame {
                tick,
                host_time_ms: host_time_ms(),
                upserts,
                removes: vec![],
            };
            frame_pub
                .put(frame.encode_to_vec())
                .wait()
                .map_err(|e| anyhow::anyhow!("frame publish failed: {e}"))?;
            tick = tick.wrapping_add(1);
            publishes += 1;
        }

        if last_log.elapsed() >= Duration::from_secs(1) {
            log::info!(
                "delta: frames={publishes} upsert_entities={upsert_entities} skipped_empty={empty_delta_frames} tick={tick}"
            );
            publishes = 0;
            upsert_entities = 0;
            empty_delta_frames = 0;
            last_log = Instant::now();
        }
    }
}
