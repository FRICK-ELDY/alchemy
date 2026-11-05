//! Inventory scene despawn control plane (x007b).
//!
//! Guarantees: Zenoh `…/inventory/despawn` is **Room AEAD** (`AeadKind::InventoryDespawn`);
//! host role Room Token verified after open; world HTTP delete runs on this worker (not the
//! physics tick); physics applies shared scene / re-seal and best-effort Rapier remove.
//! Non-guarantees: guest despawn; put-back coupling; Intent path; Rapier miss zero-failure.

use alchemy_protocol::inventory_despawn_key;
use alchemy_protocol::wire_aead::{self, AeadKind};
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};
use zenoh::Wait;

use crate::room_token::{delete_room_scene_entry, install_shared_scene_if_newer, RoomScene};

/// Ready for physics: world already deleted; apply shared scene + reseal + Rapier remove.
#[derive(Debug, Clone)]
pub struct ReadyDespawn {
    pub entity_id: u32,
    pub scene: RoomScene,
}

#[derive(Debug, Deserialize)]
struct DespawnPayloadJson {
    room_token: String,
    entity_id: u32,
}

/// Physics drains at most one ready despawn per tick (no HTTP on the sim thread).
pub struct DespawnInbox {
    rx: Mutex<Receiver<ReadyDespawn>>,
}

impl DespawnInbox {
    pub fn try_recv(&self) -> Option<ReadyDespawn> {
        self.rx.lock().ok()?.try_recv().ok()
    }
}

fn parse_payload(bytes: &[u8]) -> Result<DespawnPayloadJson> {
    let parsed: DespawnPayloadJson = serde_json::from_slice(bytes).context("despawn JSON")?;
    if parsed.room_token.trim().is_empty() {
        return Err(anyhow!("missing room_token"));
    }
    Ok(parsed)
}

struct DespawnRate {
    window: Duration,
    max: u32,
    start: Instant,
    count: u32,
}

impl DespawnRate {
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

/// Subscribe → AEAD open → host role verify → world HTTP delete → enqueue for physics.
pub fn despawn_listener<F>(
    session: zenoh::Session,
    room_id: &str,
    wire_cap: &str,
    room_key: Arc<[u8; wire_aead::ROOM_KEY_LEN]>,
    world_url: String,
    verify_host: F,
) -> Result<DespawnInbox>
where
    F: Fn(&str) -> Result<()> + Send + 'static,
{
    let key = inventory_despawn_key(room_id, wire_cap);
    let room_id = room_id.to_string();
    let subscriber = session
        .declare_subscriber(key.clone())
        .wait()
        .map_err(|e| anyhow!("declare_subscriber inventory/despawn: {e}"))?;

    let (tx, rx): (Sender<ReadyDespawn>, Receiver<ReadyDespawn>) = mpsc::channel();
    let tx = Arc::new(Mutex::new(tx));

    thread::Builder::new()
        .name("inventory-despawn".into())
        .spawn(move || {
            let _session = session;
            let mut rate = DespawnRate::new(8, Duration::from_secs(10));
            log::info!("subscribed to {key} (x007b inventory despawn, AEAD)");
            while let Ok(sample) = subscriber.recv() {
                let bytes = sample.payload().to_bytes();
                match process_sample(
                    bytes.as_ref(),
                    &room_key,
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
                    Err(err) => log::warn!("inventory despawn rejected: {err:#}"),
                }
            }
        })
        .context("spawn inventory-despawn thread")?;

    Ok(DespawnInbox { rx: Mutex::new(rx) })
}

fn process_sample<F>(
    envelope: &[u8],
    room_key: &[u8; wire_aead::ROOM_KEY_LEN],
    room_id: &str,
    world_url: &str,
    verify_host: &F,
    rate: &mut DespawnRate,
) -> Result<ReadyDespawn>
where
    F: Fn(&str) -> Result<()>,
{
    wire_aead::check_aead_envelope_bytes(AeadKind::InventoryDespawn, envelope.len())
        .map_err(|e| anyhow!("aead envelope: {e}"))?;
    let plain = wire_aead::open(
        room_key.as_ref(),
        room_id,
        AeadKind::InventoryDespawn,
        envelope,
    )
    .map_err(|e| anyhow!("aead open: {e}"))?;
    let parsed = parse_payload(&plain)?;
    verify_host(parsed.room_token.trim())?;
    if !rate.allow() {
        return Err(anyhow!("rate limited"));
    }

    let scene = delete_room_scene_entry(
        world_url,
        room_id,
        parsed.room_token.trim(),
        parsed.entity_id,
    )
    .context("world scene delete")?;

    Ok(ReadyDespawn {
        entity_id: parsed.entity_id,
        scene,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplyOutcome {
    pub entity_id: u32,
    /// False when `ready.scene.revision` was not strictly newer than shared (stale race).
    pub scene_applied: bool,
    /// False when Manifest/PlaceRead path succeeded but Rapier had no such mesh.
    pub rapier_ok: bool,
}

/// Apply a ready despawn on the physics thread (no HTTP).
///
/// Installs `scene` + `force_reseal` **before** Rapier remove so Manifest/PlaceRead stay
/// authoritative if physics already lacks the entity (spawn-failure leftover).
/// Shared scene is revision-monotonic (x007b): stale despawn does not overwrite or Rapier-remove.
pub fn apply_ready_despawn(
    ready: ReadyDespawn,
    shared_scene: &Arc<RwLock<RoomScene>>,
    force_reseal: &AtomicBool,
    remove_mesh: impl FnOnce(u32) -> bool,
) -> Result<ApplyOutcome> {
    let entity_id = ready.entity_id;
    let scene_applied = install_shared_scene_if_newer(shared_scene, ready.scene)?;
    if !scene_applied {
        return Ok(ApplyOutcome {
            entity_id,
            scene_applied: false,
            rapier_ok: false,
        });
    }
    force_reseal.store(true, Ordering::SeqCst);
    let rapier_ok = remove_mesh(entity_id);
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

    #[test]
    fn parse_payload_requires_token_and_entity() {
        let ok = br#"{"room_token":"tok","entity_id":100}"#;
        let p = parse_payload(ok).unwrap();
        assert_eq!(p.entity_id, 100);
        assert!(parse_payload(br#"{"entity_id":1}"#).is_err());
        assert!(parse_payload(br#"{"room_token":"","entity_id":1}"#).is_err());
    }

    #[test]
    fn apply_ready_despawn_scene_before_rapier_and_ok_on_miss() {
        let shared = Arc::new(RwLock::new(empty_scene(1)));
        let force = AtomicBool::new(false);
        let ready = ReadyDespawn {
            entity_id: 100,
            scene: empty_scene(2),
        };
        let mut rapier_called = false;
        let out = apply_ready_despawn(ready, &shared, &force, |_id| {
            rapier_called = true;
            false
        })
        .unwrap();
        assert!(out.scene_applied);
        assert!(!out.rapier_ok);
        assert!(rapier_called);
        assert!(force.load(Ordering::SeqCst));
        assert_eq!(shared.read().unwrap().revision, 2);
    }

    #[test]
    fn apply_ready_despawn_skips_stale_revision() {
        let shared = Arc::new(RwLock::new(empty_scene(5)));
        let force = AtomicBool::new(false);
        let ready = ReadyDespawn {
            entity_id: 100,
            scene: empty_scene(4),
        };
        let mut rapier_called = false;
        let out = apply_ready_despawn(ready, &shared, &force, |_id| {
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
