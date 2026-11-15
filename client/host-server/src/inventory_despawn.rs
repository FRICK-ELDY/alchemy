//! Inventory scene despawn control plane (x007b / x0081 / x0098 / x00b9).
//!
//! Guarantees: Zenoh `…/inventory/despawn` is **Room AEAD**; session Ed25519 + SessionTable
//! Dual auth (**`role=host` ∧ `sub` == host Token sub** **or** **`sub` == `owner_sub`**);
//! **x00b9**: host-local hide / pending despawn (no live overlay CAS); physics applies local
//! shared scene / re-seal and best-effort XPBD mesh remove. Persistence is explicit Save.
//! Order: evaluate → rate → commit seq → local Ready (unbound / non-owner guest cannot burn
//! shared rate).
//!
//! Non-guarantees: Invite / arbitrary-guest ACL; put-back coupling; Intent path;
//! crash durability of unsaved pending; evaluate→rate→commit のあいだの host→guest 降格で
//! rate のみ消費；`owner_sub` 未取得時の owner 経路;
//! **`control_seq` は local apply 前に commit 済み** — inbox / physics 失敗でも seq は進む
//! （edit lane と同契約）。

use alchemy_protocol::control_plane;
use alchemy_protocol::inventory_despawn_key;
use alchemy_protocol::wire_aead::{self, AeadKind};
use alchemy_protocol::wire_crypto::PUBLIC_KEY_LENGTH;
use alchemy_protocol::EditProposal;
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use zenoh::Wait;

use crate::control_rate::SharedControlRate;
use host_server::room_token::{remove_local_scene_entry, RoomScene, SharedHostHttpAuth};
use host_server::session::SessionTable;

/// Ready for physics: host-local despawn; apply local scene + reseal + XPBD remove.
/// `edit_proposal`: `Some` when from `…/edit` — tip/Commit deferred until apply-ok (x00b9).
#[derive(Debug, Clone)]
pub struct ReadyDespawn {
    pub entity_id: u32,
    pub edit_proposal: Option<EditProposal>,
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
    /// Shared channel for inventory despawn + `…/edit` (x00b2 dual-subscribe).
    pub fn pair() -> (Sender<ReadyDespawn>, Self) {
        let (tx, rx) = mpsc::channel();
        (tx, Self { rx: Mutex::new(rx) })
    }

    pub fn try_recv(&self) -> Option<ReadyDespawn> {
        self.rx.lock().ok()?.try_recv().ok()
    }

    /// Closed inbox for HostLoop unit tests (x0099); `try_recv` always returns `None`.
    #[cfg(test)]
    pub fn empty_for_test() -> Self {
        let (_tx, rx) = mpsc::channel();
        Self { rx: Mutex::new(rx) }
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

/// Subscribe → AEAD open → session verify → host-local Ready → enqueue for physics.
#[allow(clippy::too_many_arguments)] // wire + shared_scene wiring stays flat at the listener edge
pub fn despawn_listener(
    session: zenoh::Session,
    room_id: &str,
    wire_cap: &str,
    room_aead: host_server::room_aead::SharedRoomAead,
    sessions: Arc<Mutex<SessionTable>>,
    http_auth: SharedHostHttpAuth,
    shared_scene: Arc<RwLock<RoomScene>>,
    tx: Sender<ReadyDespawn>,
    control_rate: Arc<Mutex<SharedControlRate>>,
) -> Result<()> {
    let key = inventory_despawn_key(room_id, wire_cap);
    let room_id = room_id.to_string();
    let subscriber = session
        .declare_subscriber(key.clone())
        .wait()
        .map_err(|e| anyhow!("declare_subscriber inventory/despawn: {e}"))?;

    thread::Builder::new()
        .name("inventory-despawn".into())
        .spawn(move || {
            let _session = session;
            log::info!(
                "subscribed to {key} (x0081 inventory despawn, session sig; shared rate x00b2; x00b9 local)"
            );
            while let Ok(sample) = subscriber.recv() {
                let bytes = sample.payload().to_bytes();
                match process_sample(
                    bytes.as_ref(),
                    &room_aead,
                    &room_id,
                    &sessions,
                    &http_auth,
                    &shared_scene,
                    &control_rate,
                ) {
                    Ok(ready) => {
                        let _ = tx.send(ready);
                    }
                    Err(err) => log::warn!("inventory despawn rejected: {err:#}"),
                }
            }
        })
        .context("spawn inventory-despawn thread")?;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn process_sample(
    envelope: &[u8],
    room_aead: &host_server::room_aead::SharedRoomAead,
    room_id: &str,
    sessions: &Arc<Mutex<SessionTable>>,
    http_auth: &SharedHostHttpAuth,
    shared_scene: &Arc<RwLock<RoomScene>>,
    control_rate: &Arc<Mutex<SharedControlRate>>,
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
    let owner_sub = shared_scene
        .read()
        .map_err(|_| anyhow!("shared scene RwLock poisoned"))?
        .owner_sub
        .clone();
    {
        let table = sessions
            .lock()
            .map_err(|_| anyhow!("SessionTable poisoned"))?;
        table.evaluate_control(
            &session_pub,
            parsed.control_seq,
            &auth.sub,
            owner_sub.as_deref(),
        )?;
    }
    {
        let mut rate = control_rate
            .lock()
            .map_err(|_| anyhow!("SharedControlRate poisoned"))?;
        if !rate.allow() {
            return Err(anyhow!("rate limited"));
        }
    }
    {
        let mut table = sessions
            .lock()
            .map_err(|_| anyhow!("SessionTable poisoned"))?;
        if let Err(err) = table.commit_control(
            &session_pub,
            parsed.control_seq,
            &auth.sub,
            owner_sub.as_deref(),
        ) {
            if let Ok(mut rate) = control_rate.lock() {
                rate.refund();
            }
            return Err(err);
        }
    }

    // Presence check only — no world CAS (x00b9).
    let present = shared_scene
        .read()
        .map_err(|_| anyhow!("shared scene RwLock poisoned"))?
        .entries
        .iter()
        .any(|e| e.entity_id == parsed.entity_id);
    if !present {
        return Err(anyhow!("entity_id not in live scene"));
    }

    Ok(ReadyDespawn {
        entity_id: parsed.entity_id,
        edit_proposal: None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplyOutcome {
    pub entity_id: u32,
    pub scene_applied: bool,
    /// False when scene applied but XPBD/sim could not remove the mesh row.
    pub physics_ok: bool,
}

/// Apply a ready despawn on the physics thread (no HTTP; x00b9 local scene).
pub fn apply_ready_despawn(
    ready: ReadyDespawn,
    shared_scene: &Arc<RwLock<RoomScene>>,
    force_reseal: &AtomicBool,
    remove_mesh: impl FnOnce(u32) -> bool,
) -> Result<ApplyOutcome> {
    let entity_id = ready.entity_id;
    let scene_applied = remove_local_scene_entry(shared_scene, entity_id)?;
    if !scene_applied {
        return Ok(ApplyOutcome {
            entity_id,
            scene_applied: false,
            physics_ok: false,
        });
    }
    force_reseal.store(true, Ordering::SeqCst);
    let physics_ok = remove_mesh(entity_id);
    Ok(ApplyOutcome {
        entity_id,
        scene_applied: true,
        physics_ok,
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
            owner_sub: None,
        }
    }

    #[test]
    fn parse_rejects_legacy_room_token_payload() {
        let legacy = br#"{"room_token":"tok","entity_id":100}"#;
        assert!(parse_payload(legacy).is_err());
        assert!(parse_payload(br#"{"entity_id":1}"#).is_err());
    }

    #[test]
    fn apply_ready_despawn_local_remove_and_ok_on_physics_miss() {
        let mut scene = empty_scene(1);
        scene.entries.push(alchemy_protocol::SceneEntry {
            entity_id: 100,
            blake3: "aa".repeat(32),
            x: 0.0,
            y: 0.0,
            z: 0.0,
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
            parent_entity_id: 0,
        });
        let shared = Arc::new(RwLock::new(scene));
        let force = AtomicBool::new(false);
        let ready = ReadyDespawn {
            entity_id: 100,
            edit_proposal: None,
        };
        let mut physics_called = false;
        let out = apply_ready_despawn(ready, &shared, &force, |_id| {
            physics_called = true;
            false
        })
        .unwrap();
        assert!(out.scene_applied);
        assert!(!out.physics_ok);
        assert!(physics_called);
        assert!(force.load(Ordering::SeqCst));
        assert!(shared.read().unwrap().entries.is_empty());
        assert_eq!(shared.read().unwrap().revision, 1);
    }

    #[test]
    fn apply_ready_despawn_skips_missing_entity() {
        let shared = Arc::new(RwLock::new(empty_scene(5)));
        let force = AtomicBool::new(false);
        let ready = ReadyDespawn {
            entity_id: 100,
            edit_proposal: None,
        };
        let mut physics_called = false;
        let out = apply_ready_despawn(ready, &shared, &force, |_id| {
            physics_called = true;
            true
        })
        .unwrap();
        assert!(!out.scene_applied);
        assert!(!out.physics_ok);
        assert!(!physics_called);
        assert!(!force.load(Ordering::SeqCst));
        assert_eq!(shared.read().unwrap().revision, 5);
    }
}
