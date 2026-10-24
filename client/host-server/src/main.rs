//! Local host-server — Slice B.
//!
//! Guarantees:
//! - 30 Hz full-frame RenderFrame publish
//! - Subscribe InputEvent; verify Room Token via world-server JWKS; drop invalid
//! - Valid input overrides demo entity pose
//!
//! Non-guarantees:
//! - Rapier / Formula tick
//! - Delta compression
//! - Visibility ACL / verifier

mod room_token;

use std::f64::consts::TAU;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use alchemy_protocol::{
    Entity, InputEvent, Message, RenderFrame, FRAME_KEY_LOCAL, INPUT_KEY_LOCAL, PROTOCOL_PIN,
};
use anyhow::{Context, Result};
use room_token::{fetch_host_token, RoomTokenVerifier};
use zenoh::bytes::ZBytes;
use zenoh::Config;
use zenoh::Wait;

const TICK_HZ: u64 = 30;
const CONNECT: &str = "tcp/127.0.0.1:7447";
const DEFAULT_WORLD_URL: &str = "http://127.0.0.1:4003";
const DEFAULT_ROOM_ID: &str = "local";

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

fn demo_entity(tick: u64, overlay: PoseOverride) -> Entity {
    if overlay.active {
        return Entity {
            id: 1,
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
    Entity {
        id: 1,
        x: angle.cos() * 1.5,
        y: angle.sin() * 0.5,
        z: angle.sin() * 1.5,
        r: 0.2,
        g: 0.7,
        b: 1.0,
        a: 1.0,
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

    log::info!(
        "host-server Slice B starting; protocol_pin={PROTOCOL_PIN} frame={FRAME_KEY_LOCAL} input={INPUT_KEY_LOCAL} world={world_url} room={room_id}"
    );

    let host_token = fetch_host_token(&world_url, &room_id)
        .context("fetch host Room Token (is world-server up?)")?;
    let verifier = Arc::new(RoomTokenVerifier::new(&world_url, &room_id));
    verifier.refresh_jwks().context("initial JWKS fetch")?;
    // Prove the host token itself verifies (fail fast if misconfigured).
    verifier
        .verify(&host_token)
        .context("host Room Token self-check")?;
    log::info!("host Room Token acquired and verified");

    let pose = Arc::new(Mutex::new(PoseOverride::default()));
    spawn_input_gate(Arc::clone(&verifier), Arc::clone(&pose))?;

    let session = open_zenoh().context("open zenoh session")?;
    let publisher = session
        .declare_publisher(FRAME_KEY_LOCAL)
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_publisher failed: {e}"))?;

    let tick_period = Duration::from_nanos(1_000_000_000 / TICK_HZ);
    let mut tick: u64 = 0;
    let mut next = Instant::now();
    let mut last_log = Instant::now();
    let mut publishes: u64 = 0;

    loop {
        let now = Instant::now();
        if now < next {
            std::thread::sleep(next - now);
        }
        next += tick_period;

        let overlay = pose.lock().map(|p| *p).unwrap_or_default();
        let frame = RenderFrame {
            tick,
            host_time_ms: host_time_ms(),
            entities: vec![demo_entity(tick, overlay)],
        };
        let payload = frame.encode_to_vec();
        publisher
            .put(payload)
            .wait()
            .map_err(|e| anyhow::anyhow!("publish failed: {e}"))?;

        tick = tick.wrapping_add(1);
        publishes += 1;

        if last_log.elapsed() >= Duration::from_secs(1) {
            log::info!("published {publishes} frames in last ~1s (tick={tick})");
            publishes = 0;
            last_log = Instant::now();
        }
    }
}
