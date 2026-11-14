//! Deterministic edit lane (`…/edit`) — Spawn/Despawn foundation (x00b2 / mvcc_xpbd §4.7).
//!
//! Guarantees: AEAD `EditProposal` → session Ed25519 (`wire_crypto` prost sign) → Dual-auth /
//! shared control rate / `control_seq`; single Spawn or Despawn only; world overlay CAS reuse;
//! **`EditCommit.scene_rev` = packed overlay `RoomScene.revision` after CAS** (same space as
//! inventory append/delete). Accept path: enqueue Ready **then** publish `EditCommit` (so a
//! commit implies the physics inbox accepted the row). Reject path publishes
//! `accepted=false` with current overlay revision.
//!
//! Rate: `SharedControlRate::allow` runs after Dual-auth evaluate; **`commit_control` failure
//! refunds** the slot. WORLD (CAS) failures **after** a successful `commit_control` do
//! **not** refund — `control_seq` already advanced (same trade-off as legacy inventory JSON).
//! Spawn blake3 / finite TRS checks run **before** evaluate/rate (legacy `parse_payload` parity),
//! so bad payload does not consume seq or rate.
//!
//! Non-guarantees: Reparent/SetField; multi-op batch; snapshot `scene_rev` gap; Undo UI;
//! FrameBridge structure apply; old inventory key removal; **EditCommit for inventory
//! dual-subscribe** (compat path advances overlay revision but does not emit EditCommit —
//! live ordered commits are `…/edit` only during the dual-subscribe window).
//! Decode / AEAD failures before a typed proposal exist produce **no** EditCommit (cannot
//! correlate `op_id`); typed gate failures return reject commits.
//! **Inbox closed after CAS** (shutdown): overlay may already have advanced; Ready/EditCommit
//! are best-effort (warn only; no compensating world rollback).

use alchemy_protocol::edit_op::Op;
use alchemy_protocol::wire_aead::{self, AeadKind};
use alchemy_protocol::wire_crypto::{
    sign_edit_commit, verify_edit_proposal, SigningKey, PUBLIC_KEY_LENGTH,
};
use alchemy_protocol::{edit_key, edit_reject, EditCommit, EditProposal, EditSpawn, Message};
use anyhow::{anyhow, Context, Result};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use zenoh::Wait;

use crate::control_rate::SharedControlRate;
use crate::inventory_despawn::ReadyDespawn;
use crate::inventory_spawn::ReadySpawn;
use crate::room_aead::{self, SharedRoomAead};
use crate::room_token::{
    append_room_scene_entry, delete_room_scene_entry, RoomScene, SharedHostHttpAuth,
};
use crate::session::SessionTable;

enum ReadyEdit {
    Spawn(ReadySpawn),
    Despawn(ReadyDespawn),
}

fn overlay_revision(shared_scene: &Arc<RwLock<RoomScene>>) -> Result<u64> {
    Ok(shared_scene
        .read()
        .map_err(|_| anyhow!("shared scene RwLock poisoned"))?
        .revision)
}

#[allow(clippy::too_many_arguments)]
pub fn edit_listener(
    session: zenoh::Session,
    room_id: &str,
    wire_cap: &str,
    room_aead: SharedRoomAead,
    world_url: String,
    sessions: Arc<Mutex<SessionTable>>,
    http_auth: SharedHostHttpAuth,
    shared_scene: Arc<RwLock<RoomScene>>,
    spawn_tx: Sender<ReadySpawn>,
    despawn_tx: Sender<ReadyDespawn>,
    host_key: SigningKey,
    control_rate: Arc<Mutex<SharedControlRate>>,
) -> Result<()> {
    let key = edit_key(room_id, wire_cap);
    let room_id = room_id.to_string();
    let subscriber = session
        .declare_subscriber(key.clone())
        .wait()
        .map_err(|e| anyhow!("declare_subscriber edit: {e}"))?;

    thread::Builder::new()
        .name("edit-lane".into())
        .spawn(move || {
            log::info!("subscribed to {key} (x00b2 edit lane)");
            while let Ok(sample) = subscriber.recv() {
                let bytes = sample.payload().to_bytes();
                // Own EditCommit puts share the key; kind mismatch → skip quietly.
                if wire_aead::check_aead_envelope_bytes(AeadKind::EditProposal, bytes.len())
                    .is_err()
                {
                    continue;
                }
                let plain = match room_aead::open_shared(
                    &room_aead,
                    &room_id,
                    AeadKind::EditProposal,
                    bytes.as_ref(),
                ) {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                match process_proposal(
                    &plain,
                    &room_id,
                    &world_url,
                    &sessions,
                    &http_auth,
                    &shared_scene,
                    &control_rate,
                ) {
                    Ok((commit, Some(ReadyEdit::Spawn(ready)))) => {
                        // CAS already committed; inbox closed (shutdown) → best-effort only.
                        if spawn_tx.send(ready).is_ok() {
                            publish_commit(
                                &session,
                                &key,
                                &room_aead,
                                &room_id,
                                &host_key,
                                commit,
                            );
                        } else {
                            log::warn!(
                                "edit accept: spawn inbox closed after CAS (no Ready/EditCommit; overlay kept)"
                            );
                        }
                    }
                    Ok((commit, Some(ReadyEdit::Despawn(ready)))) => {
                        if despawn_tx.send(ready).is_ok() {
                            publish_commit(
                                &session,
                                &key,
                                &room_aead,
                                &room_id,
                                &host_key,
                                commit,
                            );
                        } else {
                            log::warn!(
                                "edit accept: despawn inbox closed after CAS (no Ready/EditCommit; overlay kept)"
                            );
                        }
                    }
                    Ok((commit, None)) => {
                        publish_commit(
                            &session,
                            &key,
                            &room_aead,
                            &room_id,
                            &host_key,
                            commit,
                        );
                    }
                    Err(err) => {
                        // Untyped failures (e.g. prost decode) — no op_id to correlate.
                        log::warn!("edit proposal dropped without EditCommit: {err:#}");
                    }
                }
            }
        })
        .context("spawn edit-lane thread")?;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn process_proposal(
    plain: &[u8],
    room_id: &str,
    world_url: &str,
    sessions: &Arc<Mutex<SessionTable>>,
    http_auth: &SharedHostHttpAuth,
    shared_scene: &Arc<RwLock<RoomScene>>,
    control_rate: &Arc<Mutex<SharedControlRate>>,
) -> Result<(EditCommit, Option<ReadyEdit>)> {
    let proposal = EditProposal::decode(plain).context("EditProposal decode")?;
    let live_rev = overlay_revision(shared_scene)?;
    if proposal.room_id != room_id {
        return Ok((
            reject_commit(&proposal, edit_reject::PAYLOAD, live_rev),
            None,
        ));
    }
    if proposal.session_pub.len() != PUBLIC_KEY_LENGTH {
        return Ok((
            reject_commit(&proposal, edit_reject::PAYLOAD, live_rev),
            None,
        ));
    }
    if verify_edit_proposal(&proposal).is_err() {
        return Ok((reject_commit(&proposal, edit_reject::SIG, live_rev), None));
    }
    if proposal.ops.len() != 1 {
        return Ok((
            reject_commit(&proposal, edit_reject::OPS_COUNT, live_rev),
            None,
        ));
    }
    let op = match proposal.ops[0].op.as_ref() {
        Some(Op::Spawn(s)) => {
            // Before Dual-auth / rate / commit_control — invalid blake3/TRS must not advance seq.
            if !spawn_payload_ok(s) {
                return Ok((
                    reject_commit(&proposal, edit_reject::PAYLOAD, live_rev),
                    None,
                ));
            }
            OpKind::Spawn(s.clone())
        }
        Some(Op::Despawn(d)) => OpKind::Despawn(d.entity_id),
        None => {
            return Ok((
                reject_commit(&proposal, edit_reject::OP_UNSUPPORTED, live_rev),
                None,
            ));
        }
    };

    let auth = match http_auth.snapshot() {
        Ok(a) => a,
        Err(_) => {
            return Ok((reject_commit(&proposal, edit_reject::AUTH, live_rev), None));
        }
    };
    let owner_sub = shared_scene
        .read()
        .map_err(|_| anyhow!("shared scene RwLock poisoned"))?
        .owner_sub
        .clone();
    {
        let table = sessions
            .lock()
            .map_err(|_| anyhow!("SessionTable poisoned"))?;
        if table
            .evaluate_control(
                &proposal.session_pub,
                proposal.control_seq,
                &auth.sub,
                owner_sub.as_deref(),
            )
            .is_err()
        {
            return Ok((reject_commit(&proposal, edit_reject::AUTH, live_rev), None));
        }
    }
    {
        let mut rate = control_rate
            .lock()
            .map_err(|_| anyhow!("SharedControlRate poisoned"))?;
        if !rate.allow() {
            return Ok((reject_commit(&proposal, edit_reject::RATE, live_rev), None));
        }
    }
    {
        let mut table = sessions
            .lock()
            .map_err(|_| anyhow!("SessionTable poisoned"))?;
        if table
            .commit_control(
                &proposal.session_pub,
                proposal.control_seq,
                &auth.sub,
                owner_sub.as_deref(),
            )
            .is_err()
        {
            if let Ok(mut rate) = control_rate.lock() {
                rate.refund();
            }
            return Ok((reject_commit(&proposal, edit_reject::AUTH, live_rev), None));
        }
    }

    match op {
        OpKind::Spawn(spawn) => apply_spawn(
            &proposal,
            &spawn,
            room_id,
            world_url,
            &auth.token,
            &auth.pop_key,
            shared_scene,
        ),
        OpKind::Despawn(entity_id) => apply_despawn(
            &proposal,
            entity_id,
            room_id,
            world_url,
            &auth.token,
            &auth.pop_key,
            shared_scene,
        ),
    }
}

enum OpKind {
    Spawn(EditSpawn),
    Despawn(u32),
}

fn is_blake3_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn spawn_payload_ok(spawn: &EditSpawn) -> bool {
    if !is_blake3_hex(&spawn.blake3) {
        return false;
    }
    let pose = [
        spawn.x, spawn.y, spawn.z, spawn.qx, spawn.qy, spawn.qz, spawn.qw, spawn.sx, spawn.sy,
        spawn.sz,
    ];
    pose.iter().all(|v| v.is_finite()) && spawn.sx > 0.0 && spawn.sy > 0.0 && spawn.sz > 0.0
}

#[allow(clippy::too_many_arguments)]
fn apply_spawn(
    proposal: &EditProposal,
    spawn: &EditSpawn,
    room_id: &str,
    world_url: &str,
    token: &str,
    pop_key: &SigningKey,
    shared_scene: &Arc<RwLock<RoomScene>>,
) -> Result<(EditCommit, Option<ReadyEdit>)> {
    // blake3 / TRS already gated in `process_proposal` before evaluate/rate.
    let blake3 = spawn.blake3.to_ascii_lowercase();
    match append_room_scene_entry(
        world_url,
        room_id,
        token,
        pop_key,
        shared_scene,
        &blake3,
        spawn.x,
        spawn.y,
        spawn.z,
        spawn.qx,
        spawn.qy,
        spawn.qz,
        spawn.qw,
        spawn.sx,
        spawn.sy,
        spawn.sz,
    ) {
        Ok((scene, entry)) => {
            let rev = scene.revision;
            Ok((
                accept_commit(proposal, rev),
                Some(ReadyEdit::Spawn(ReadySpawn { entry, scene })),
            ))
        }
        Err(_) => Ok((
            reject_commit(
                proposal,
                edit_reject::WORLD,
                overlay_revision(shared_scene)?,
            ),
            None,
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_despawn(
    proposal: &EditProposal,
    entity_id: u32,
    room_id: &str,
    world_url: &str,
    token: &str,
    pop_key: &SigningKey,
    shared_scene: &Arc<RwLock<RoomScene>>,
) -> Result<(EditCommit, Option<ReadyEdit>)> {
    match delete_room_scene_entry(world_url, room_id, token, pop_key, shared_scene, entity_id) {
        Ok(scene) => {
            let rev = scene.revision;
            Ok((
                accept_commit(proposal, rev),
                Some(ReadyEdit::Despawn(ReadyDespawn { entity_id, scene })),
            ))
        }
        Err(_) => Ok((
            reject_commit(
                proposal,
                edit_reject::WORLD,
                overlay_revision(shared_scene)?,
            ),
            None,
        )),
    }
}

fn reject_commit(proposal: &EditProposal, reason: u32, scene_rev: u64) -> EditCommit {
    EditCommit {
        scene_rev,
        op_id: proposal.op_id,
        author_session_pub: proposal.session_pub.clone(),
        accepted: false,
        reject_reason: reason,
        ops: proposal.ops.clone(),
        host_sig: vec![],
    }
}

fn accept_commit(proposal: &EditProposal, scene_rev: u64) -> EditCommit {
    EditCommit {
        scene_rev,
        op_id: proposal.op_id,
        author_session_pub: proposal.session_pub.clone(),
        accepted: true,
        reject_reason: edit_reject::NONE,
        ops: proposal.ops.clone(),
        host_sig: vec![],
    }
}

fn publish_commit(
    session: &zenoh::Session,
    key: &str,
    room_aead: &SharedRoomAead,
    room_id: &str,
    host_key: &SigningKey,
    mut commit: EditCommit,
) {
    sign_edit_commit(host_key, &mut commit);
    let plain = commit.encode_to_vec();
    match room_aead::seal_shared(room_aead, room_id, AeadKind::EditCommit, &plain) {
        Ok(env) => {
            if let Err(e) = session.put(key, env).wait() {
                // Ready already queued — Manifest/frame still converge; commit is best-effort notify.
                log::warn!("edit commit put failed after Ready enqueue: {e}");
            }
        }
        Err(e) => log::warn!("edit commit seal failed after Ready enqueue: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alchemy_protocol::wire_crypto::{
        generate_signing_key, public_key_bytes, sign_edit_proposal,
    };
    use alchemy_protocol::{edit_op::Op, EditDespawn, EditOp, EditSpawn};

    #[test]
    fn reject_commit_preserves_op_id() {
        let mut p = EditProposal {
            session_pub: public_key_bytes(&generate_signing_key()),
            control_seq: 1,
            base_scene_rev: 0,
            op_id: 99,
            ops: vec![EditOp {
                op: Some(Op::Despawn(EditDespawn { entity_id: 3 })),
            }],
            sig: vec![],
            room_id: "local".into(),
        };
        sign_edit_proposal(&generate_signing_key(), &mut p);
        let c = reject_commit(&p, edit_reject::OPS_COUNT, 5);
        assert!(!c.accepted);
        assert_eq!(c.op_id, 99);
        assert_eq!(c.scene_rev, 5);
        assert_eq!(c.reject_reason, edit_reject::OPS_COUNT);
    }

    #[test]
    fn accept_commit_uses_overlay_revision() {
        let key = generate_signing_key();
        let mut p = EditProposal {
            session_pub: public_key_bytes(&key),
            control_seq: 1,
            base_scene_rev: 10,
            op_id: 1,
            ops: vec![],
            sig: vec![],
            room_id: "local".into(),
        };
        sign_edit_proposal(&key, &mut p);
        let c = accept_commit(&p, 1_000_042);
        assert!(c.accepted);
        assert_eq!(c.scene_rev, 1_000_042);
    }

    #[test]
    fn spawn_payload_ok_rejects_bad_blake3_and_scale() {
        let good = EditSpawn {
            blake3: "ab".repeat(32),
            x: 0.0,
            y: 0.0,
            z: 0.0,
            qx: 0.0,
            qy: 0.0,
            qz: 0.0,
            qw: 1.0,
            sx: 1.0,
            sy: 1.0,
            sz: 1.0,
        };
        assert!(spawn_payload_ok(&good));
        let mut short = good.clone();
        short.blake3 = "abcd".into();
        assert!(!spawn_payload_ok(&short));
        let mut bad_scale = good;
        bad_scale.sx = 0.0;
        assert!(!spawn_payload_ok(&bad_scale));
    }

    #[test]
    fn spawn_payload_shape_roundtrip() {
        let key = generate_signing_key();
        let mut p = EditProposal {
            session_pub: public_key_bytes(&key),
            control_seq: 2,
            base_scene_rev: 1,
            op_id: 7,
            ops: vec![EditOp {
                op: Some(Op::Spawn(EditSpawn {
                    blake3: "ab".repeat(32),
                    x: 0.0,
                    y: 1.5,
                    z: 2.0,
                    qx: 0.0,
                    qy: 0.0,
                    qz: 0.0,
                    qw: 1.0,
                    sx: 1.0,
                    sy: 1.0,
                    sz: 1.0,
                })),
            }],
            sig: vec![],
            room_id: "local".into(),
        };
        sign_edit_proposal(&key, &mut p);
        assert!(verify_edit_proposal(&p).is_ok());
        let bytes = p.encode_to_vec();
        let decoded = EditProposal::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded.op_id, 7);
        assert!(matches!(decoded.ops[0].op.as_ref(), Some(Op::Spawn(_))));
    }
}
