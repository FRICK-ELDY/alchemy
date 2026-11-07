//! Inventory take-out spawn control plane (x006d).
//!
//! Guarantees: Zenoh `…/inventory/spawn` is **Room AEAD** (`AeadKind::InventorySpawn`);
//! host role Room Token verified after open; host never receives Drive `node_id`;
//! world HTTP append runs on this worker (not the physics tick); physics only applies
//! a ready `SceneEntry` (+ shared scene / re-seal flag).
//! Non-guarantees: guest take-out; put-back; Prefab multi-child; Intent path;
//! distributed TX rollback of world append when Rapier refuses (scene+reseal still apply).

use alchemy_protocol::wire_aead::{self, AeadKind};
use alchemy_protocol::{inventory_spawn_key, SceneEntry};
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};
use zenoh::Wait;

use crate::room_token::{append_room_scene_entry, install_shared_scene_if_newer, RoomScene};

/// Ready for physics: world already appended; apply Rapier + shared scene + reseal.
#[derive(Debug, Clone)]
pub struct ReadySpawn {
    pub entry: SceneEntry,
    pub scene: RoomScene,
}

#[derive(Debug, Deserialize)]
struct SpawnPayloadJson {
    room_token: String,
    blake3: String,
    #[serde(default)]
    x: f64,
    #[serde(default = "default_y")]
    y: f64,
    #[serde(default = "default_z")]
    z: f64,
    #[serde(default)]
    qx: f64,
    #[serde(default)]
    qy: f64,
    #[serde(default)]
    qz: f64,
    #[serde(default = "default_qw")]
    qw: f64,
    #[serde(default = "default_one")]
    sx: f64,
    #[serde(default = "default_one")]
    sy: f64,
    #[serde(default = "default_one")]
    sz: f64,
}

fn default_y() -> f64 {
    1.5
}
fn default_z() -> f64 {
    2.0
}
fn default_qw() -> f64 {
    1.0
}
fn default_one() -> f64 {
    1.0
}

/// Physics drains at most one ready spawn per tick (no HTTP on the sim thread).
pub struct SpawnInbox {
    rx: Mutex<Receiver<ReadySpawn>>,
}

impl SpawnInbox {
    pub fn try_recv(&self) -> Option<ReadySpawn> {
        self.rx.lock().ok()?.try_recv().ok()
    }
}

fn is_blake3_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn finite_pose(p: &SpawnPayloadJson) -> bool {
    [p.x, p.y, p.z, p.qx, p.qy, p.qz, p.qw, p.sx, p.sy, p.sz]
        .iter()
        .all(|v| v.is_finite())
        && p.sx > 0.0
        && p.sy > 0.0
        && p.sz > 0.0
}

fn parse_payload(bytes: &[u8]) -> Result<SpawnPayloadJson> {
    let parsed: SpawnPayloadJson = serde_json::from_slice(bytes).context("spawn JSON")?;
    if parsed.room_token.trim().is_empty() {
        return Err(anyhow!("missing room_token"));
    }
    if !is_blake3_hex(&parsed.blake3) {
        return Err(anyhow!("invalid blake3"));
    }
    if !finite_pose(&parsed) {
        return Err(anyhow!("non-finite or non-positive pose/scale"));
    }
    Ok(parsed)
}

/// Simple flood guard on the spawn worker (app-level; ACL alone is not enough).
struct SpawnRate {
    window: Duration,
    max: u32,
    start: Instant,
    count: u32,
}

impl SpawnRate {
    fn new(max: u32, window: Duration) -> Self {
        Self {
            window,
            max,
            start: Instant::now(),
            count: 0,
        }
    }

    fn allow(&mut self) -> bool {
        if self.start.elapsed() >= self.window {
            self.start = Instant::now();
            self.count = 0;
        }
        if self.count >= self.max {
            return false;
        }
        self.count += 1;
        true
    }
}

/// Subscribe → AEAD open → host role verify → world HTTP append → enqueue for physics.
pub fn spawn_listener<F>(
    session: zenoh::Session,
    room_id: &str,
    wire_cap: &str,
    room_aead: host_server::room_aead::SharedRoomAead,
    world_url: String,
    verify_host: F,
) -> Result<SpawnInbox>
where
    F: Fn(&str) -> Result<()> + Send + 'static,
{
    let key = inventory_spawn_key(room_id, wire_cap);
    let room_id = room_id.to_string();
    let subscriber = session
        .declare_subscriber(key.clone())
        .wait()
        .map_err(|e| anyhow!("declare_subscriber inventory/spawn: {e}"))?;

    let (tx, rx): (Sender<ReadySpawn>, Receiver<ReadySpawn>) = mpsc::channel();
    let tx = Arc::new(Mutex::new(tx));

    thread::Builder::new()
        .name("inventory-spawn".into())
        .spawn(move || {
            let _session = session;
            let mut rate = SpawnRate::new(8, Duration::from_secs(10));
            log::info!("subscribed to {key} (x006d inventory spawn, AEAD)");
            while let Ok(sample) = subscriber.recv() {
                let bytes = sample.payload().to_bytes();
                // Rate limit only after AEAD open + host verify (Intent intent_rate parity).
                match process_sample(
                    bytes.as_ref(),
                    &room_aead,
                    &room_id,
                    &world_url,
                    &verify_host,
                    &mut rate,
                ) {
                    Ok(ready) => {
                        if let Ok(guard) = tx.lock() {
                            let _ = guard.send(ready);
                        }
                    }
                    Err(err) => log::warn!("inventory spawn rejected: {err:#}"),
                }
            }
        })
        .context("spawn inventory-spawn thread")?;

    Ok(SpawnInbox { rx: Mutex::new(rx) })
}

fn process_sample<F>(
    envelope: &[u8],
    room_aead: &host_server::room_aead::SharedRoomAead,
    room_id: &str,
    world_url: &str,
    verify_host: &F,
    rate: &mut SpawnRate,
) -> Result<ReadySpawn>
where
    F: Fn(&str) -> Result<()>,
{
    wire_aead::check_aead_envelope_bytes(AeadKind::InventorySpawn, envelope.len())
        .map_err(|e| anyhow!("aead envelope: {e}"))?;
    let plain =
        host_server::room_aead::open_shared(room_aead, room_id, AeadKind::InventorySpawn, envelope)
            .map_err(|e| anyhow!("aead open: {e}"))?;
    let parsed = parse_payload(&plain)?;
    verify_host(parsed.room_token.trim())?;
    if !rate.allow() {
        return Err(anyhow!("rate limited"));
    }

    let (scene, entry) = append_room_scene_entry(
        world_url,
        room_id,
        parsed.room_token.trim(),
        &parsed.blake3.to_ascii_lowercase(),
        parsed.x,
        parsed.y,
        parsed.z,
        parsed.qx,
        parsed.qy,
        parsed.qz,
        parsed.qw,
        parsed.sx,
        parsed.sy,
        parsed.sz,
    )
    .context("world scene append")?;

    Ok(ReadySpawn { entry, scene })
}

/// Outcome of applying a ready spawn on the physics thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplyOutcome {
    pub entity_id: u32,
    /// False when `ready.scene.revision` was not strictly newer than shared (stale race).
    pub scene_applied: bool,
    /// False when Manifest/PlaceRead path succeeded but Rapier refused the collider.
    pub rapier_ok: bool,
}

/// Apply a ready spawn on the physics thread (no HTTP).
///
/// Installs `scene` + `force_reseal` **before** Rapier so Manifest/PlaceRead stay
/// consistent if physics insert fails (poison after Rapier would leave the reverse half).
/// Shared scene is revision-monotonic (x007b): stale spawn does not overwrite or Rapier-insert.
pub fn apply_ready_spawn(
    ready: ReadySpawn,
    shared_scene: &Arc<RwLock<RoomScene>>,
    force_reseal: &AtomicBool,
    spawn_mesh: impl FnOnce(&SceneEntry) -> bool,
) -> Result<ApplyOutcome> {
    let entity_id = ready.entry.entity_id;
    let entry = ready.entry.clone();
    let scene_applied = install_shared_scene_if_newer(shared_scene, ready.scene)?;
    if !scene_applied {
        return Ok(ApplyOutcome {
            entity_id,
            scene_applied: false,
            rapier_ok: false,
        });
    }
    force_reseal.store(true, Ordering::SeqCst);
    let rapier_ok = spawn_mesh(&entry);
    Ok(ApplyOutcome {
        entity_id,
        scene_applied: true,
        rapier_ok,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_scene(revision: u64) -> RoomScene {
        RoomScene {
            revision,
            entries: vec![],
            locomotion: None,
        }
    }

    fn sample_entry(entity_id: u32) -> SceneEntry {
        SceneEntry {
            entity_id,
            blake3: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            x: 0.0,
            y: 1.5,
            z: 2.0,
            half_extent: 0.0,
            flags: 0,
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
            qx: 0.0,
            qy: 0.0,
            qz: 0.0,
            qw: 1.0,
            sx: 1.0,
            sy: 1.0,
            sz: 1.0,
        }
    }

    #[test]
    fn parse_payload_requires_blake3_token_and_finite() {
        let ok = br#"{"room_token":"tok","blake3":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","y":2.0}"#;
        let p = parse_payload(ok).unwrap();
        assert_eq!(p.y, 2.0);
        assert!(parse_payload(br#"{"room_token":"t","blake3":"short"}"#).is_err());
        assert!(parse_payload(
            br#"{"blake3":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#
        )
        .is_err());
        let nan = br#"{"room_token":"tok","blake3":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","x":null}"#;
        // null fails JSON number decode → err
        assert!(parse_payload(nan).is_err());
        let bad_scale = br#"{"room_token":"tok","blake3":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","sx":0.0}"#;
        assert!(parse_payload(bad_scale).is_err());
    }

    #[test]
    fn apply_ready_spawn_skips_stale_revision() {
        let shared = Arc::new(RwLock::new(empty_scene(5)));
        let force = AtomicBool::new(false);
        let ready = ReadySpawn {
            entry: sample_entry(100),
            scene: empty_scene(4),
        };
        let mut rapier_called = false;
        let out = apply_ready_spawn(ready, &shared, &force, |_e| {
            rapier_called = true;
            true
        })
        .unwrap();
        assert!(!out.scene_applied);
        assert!(!out.rapier_ok);
        assert!(!rapier_called);
        assert!(!force.load(Ordering::SeqCst));
        assert_eq!(shared.read().unwrap().revision, 5);
    }
}
