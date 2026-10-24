//! Local host-server — Slice A.
//!
//! Guarantees:
//! - 30 Hz full-frame RenderFrame publish to `alchemy/dev/room/local/frame`
//! - Demo entity motion (orbiting cube) encoded as protobuf
//!
//! Non-guarantees (deferred):
//! - Room Token verification on inputs
//! - Rapier / Formula tick
//! - Input subscribe path
//! - Delta compression

use std::f64::consts::TAU;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use alchemy_protocol::{Entity, Message, RenderFrame, FRAME_KEY_LOCAL, PROTOCOL_PIN};
use anyhow::{Context, Result};
use zenoh::Config;
use zenoh::Wait;

const TICK_HZ: u64 = 30;
const CONNECT: &str = "tcp/127.0.0.1:7447";

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

fn demo_entity(tick: u64) -> Entity {
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

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    log::info!(
        "host-server Slice A starting; protocol_pin={PROTOCOL_PIN} key={FRAME_KEY_LOCAL} connect={CONNECT}"
    );

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

        let frame = RenderFrame {
            tick,
            host_time_ms: host_time_ms(),
            entities: vec![demo_entity(tick)],
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
