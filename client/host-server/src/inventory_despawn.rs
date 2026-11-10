//! Inventory scene despawn control plane (x007b / x0081 / x0082).
//!
//! Guarantees: Zenoh `…/inventory/despawn` is **Room AEAD**; session Ed25519 + SessionTable
//! (**`role=host` bind** + `sub` == host Token sub); world HTTP delete uses host Token + PoP
//! + **scene revision CAS**; physics applies shared scene / re-seal and best-effort Rapier
//! remove. Order: evaluate → rate → commit seq → HTTP (unbound/guest cannot burn shared rate).
//! Non-guarantees: guest despawn for non-host sub; put-back coupling; Intent path;
//! **HTTP failure after `control_seq` commit** (seq stays advanced); evaluate→rate→commit
//! のあいだの host→guest 降格で rate のみ消費（Intent と同様・許容）。

use alchemy_protocol::control_plane;
use alchemy_protocol::inventory_despawn_key;
use alchemy_protocol::wire_aead::{self, AeadKind};
use alchemy_protocol::wire_crypto::PUBLIC_KEY_LENGTH;
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};
use zenoh::Wait;

use host_server::room_token::{
    delete_room_scene_entry, install_shared_scene_if_newer, RoomScene, SharedHostHttpAuth,
};
use host_server::session::SessionTable;

/// Ready for physics: world already deleted; apply shared scene + reseal + Rapier remove.
#[derive(Debug, Clone)]
pub struct ReadyDespawn {
    pub entity_id: u32,
    pub scene: RoomScene,
}

#[derive(Debug, Deserialize)]
struct DespawnPayloadJson {
    session_pub: String,
    control_seq: u64,
    control_sig: String,
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
    if parsed.session_pub.trim().is_empty() || parsed.control_sig.trim().is_empty() {
        return Err(anyhow!("missing session_pub/control_sig"));
    }
    Ok(parsed)
}

fn decode_b64(field: &str, value: &str) -> Result<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value.trim())
        .with_context(|| format!("{field} base64url"))
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

/// Subscribe → AEAD open → session verify → world HTTP delete → enqueue for physics.
pub fn despawn_listener(
    session: zenoh::Session,
    room_id: &str,
    wire_cap: &str,
    room_aead: host_server::room_aead::SharedRoomAead,
    world_url: String,
    sessions: Arc<Mutex<SessionTable>>,
    http_auth: SharedHostHttpAuth,
    shared_scene: Arc<RwLock<RoomScene>>,
) -> Result<DespawnInbox> {
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
            log::info!("subscribed to {key} (x0081 inventory despawn, session sig)");
            while let Ok(sample) = subscriber.recv() {
                let bytes = sample.payload().to_bytes();
                match process_sample(
                    bytes.as_ref(),
                    &room_aead,
                    &room_id,
                    &world_url,
                    &sessions,
                    &http_auth,
                    &shared_scene,
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

fn process_sample(
    envelope: &[u8],
    room_aead: &host_server::room_aead::SharedRoomAead,
    room_id: &str,
    world_url: &str,
    sessions: &Arc<Mutex<SessionTable>>,
    http_auth: &SharedHostHttpAuth,
    shared_scene: &Arc<RwLock<RoomScene>>,
    rate: &mut DespawnRate,
) -> Result<ReadyDespawn> {
    wire_aead::check_aead_envelope_bytes(AeadKind::InventoryDespawn, envelope.len())
        .map_err(|e| anyhow!("aead envelope: {e}"))?;
    let plain = host_server::room_aead::open_shared(
        room_aead,
        room_id,
        AeadKind::InventoryDespawn,
        envelope,
    )
    .map_err(|e| anyhow!("aead open: {e}"))?;
    let parsed = parse_payload(&plain)?;
    let session_pub = decode_b64("session_pub", &parsed.session_pub)?;
    if session_pub.len() != PUBLIC_KEY_LENGTH {
        return Err(anyhow!("session_pub length {}", session_pub.len()));
    }
    let control_sig = decode_b64("control_sig", &parsed.control_sig)?;
    control_plane::verify_despawn(
        &session_pub,
        room_id,
        parsed.control_seq,
        parsed.entity_id,
        &control_sig,
    )
    .map_err(|e| anyhow!("control sig: {e}"))?;

    let auth = http_auth.snapshot()?;
    {
        let table = sessions
            .lock()
            .map_err(|_| anyhow!("SessionTable poisoned"))?;
        table.evaluate_control(&session_pub, parsed.control_seq, &auth.sub)?;
    }
    if !rate.allow() {
        return Err(anyhow!("rate limited"));
    }
    {
        let mut table = sessions
            .lock()
            .map_err(|_| anyhow!("SessionTable poisoned"))?;
        table.commit_control(&session_pub, parsed.control_seq, &auth.sub)?;
    }

    let scene = delete_room_scene_entry(
        world_url,
        room_id,
        &auth.token,
        &auth.pop_key,
        shared_scene,
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
    pub scene_applied: bool,
    pub rapier_ok: bool,
}

/// Apply a ready despawn on the physics thread (no HTTP).
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
    fn parse_rejects_legacy_room_token_payload() {
        let legacy = br#"{"room_token":"tok","entity_id":100}"#;
        assert!(parse_payload(legacy).is_err());
        assert!(parse_payload(br#"{"entity_id":1}"#).is_err());
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
