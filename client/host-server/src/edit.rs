//! Deterministic edit lane (`…/edit`) — Spawn/Despawn + live Reparent/SetField (x00b2 / x00b3).
//!
//! Guarantees: AEAD `EditProposal` → session Ed25519 → Dual-auth / shared control rate /
//! `control_seq`; single op.
//! Spawn/Despawn: overlay CAS then inbox; **tip + accept Commit after inbox send**
//! (send fail → reject `APPLY`, tip unchanged; VCS debt remains overlay CAS).
//! Reparent/SetField: optimistic ledger mirrors only; **tip + accept Commit after**
//! FrameBridge queue **and** barrier — never rewind tip; queue/barrier/inbox fail →
//! reject `APPLY`.
//!
//! Non-guarantees: multi-op batch; Undo UI; old inventory key removal; explicit Save / VCS.

use alchemy_protocol::edit_op::Op;
use alchemy_protocol::wire_aead::{self, AeadKind};
use alchemy_protocol::wire_crypto::{
    sign_edit_commit, verify_edit_proposal, SigningKey, PUBLIC_KEY_LENGTH,
};
use alchemy_protocol::{
    decode_color_rgba, decode_flags, edit_reject, field_value_hash, known_field_id, EditCommit,
    EditProposal, EditReparent, EditSetField, EditSpawn, Message,
};
use anyhow::{anyhow, Context, Result};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use zenoh::Wait;

use crate::control_rate::SharedControlRate;
use crate::inventory_despawn::ReadyDespawn;
use crate::inventory_spawn::ReadySpawn;
use crate::live_edit::{current_field_value, ReadyLiveEdit, SharedLiveEdit};
use crate::room_aead::{self, SharedRoomAead};
use crate::room_token::{
    append_room_scene_entry, delete_room_scene_entry, RoomScene, SharedHostHttpAuth,
};
use crate::session::SessionTable;

enum ReadyEdit {
    /// Tip+Commit deferred until inbox `send` succeeds (avoids tip advance without Commit).
    Spawn {
        ready: ReadySpawn,
        proposal: EditProposal,
    },
    Despawn {
        ready: ReadyDespawn,
        proposal: EditProposal,
    },
    Live(ReadyLiveEdit),
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
    live_edit: SharedLiveEdit,
    spawn_tx: Sender<ReadySpawn>,
    despawn_tx: Sender<ReadyDespawn>,
    live_tx: Sender<ReadyLiveEdit>,
    host_key: SigningKey,
    control_rate: Arc<Mutex<SharedControlRate>>,
) -> Result<()> {
    let key = alchemy_protocol::edit_key(room_id, wire_cap);
    let room_id = room_id.to_string();
    let subscriber = session
        .declare_subscriber(key.clone())
        .wait()
        .map_err(|e| anyhow!("declare_subscriber edit: {e}"))?;

    thread::Builder::new()
        .name("edit-lane".into())
        .spawn(move || {
            log::info!("subscribed to {key} (x00b3 edit lane)");
            while let Ok(sample) = subscriber.recv() {
                let bytes = sample.payload().to_bytes();
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
                    &live_edit,
                    &control_rate,
                ) {
                    Ok((None, Some(ReadyEdit::Spawn { ready, proposal }))) => {
                        if spawn_tx.send(ready).is_ok() {
                            let tip = match live_edit.lock() {
                                Ok(mut led) => led.bump_tip(),
                                Err(_) => {
                                    log::error!(
                                        "LiveEditLedger poisoned after spawn send; aborting host"
                                    );
                                    std::process::exit(78);
                                }
                            };
                            publish_commit(
                                &session,
                                &key,
                                &room_aead,
                                &room_id,
                                &host_key,
                                accept_commit(&proposal, tip),
                            );
                        } else {
                            let tip = match live_edit.lock() {
                                Ok(g) => g.tip,
                                Err(_) => {
                                    log::error!(
                                        "LiveEditLedger poisoned on spawn inbox closed; aborting host"
                                    );
                                    std::process::exit(78);
                                }
                            };
                            publish_commit(
                                &session,
                                &key,
                                &room_aead,
                                &room_id,
                                &host_key,
                                reject_commit(&proposal, edit_reject::APPLY, tip),
                            );
                            log::warn!(
                                "edit accept: spawn inbox closed after CAS (reject APPLY; overlay kept; tip unchanged)"
                            );
                        }
                    }
                    Ok((None, Some(ReadyEdit::Despawn { ready, proposal }))) => {
                        if despawn_tx.send(ready).is_ok() {
                            let tip = match live_edit.lock() {
                                Ok(mut led) => led.bump_tip(),
                                Err(_) => {
                                    log::error!(
                                        "LiveEditLedger poisoned after despawn send; aborting host"
                                    );
                                    std::process::exit(78);
                                }
                            };
                            publish_commit(
                                &session,
                                &key,
                                &room_aead,
                                &room_id,
                                &host_key,
                                accept_commit(&proposal, tip),
                            );
                        } else {
                            let tip = match live_edit.lock() {
                                Ok(g) => g.tip,
                                Err(_) => {
                                    log::error!(
                                        "LiveEditLedger poisoned on despawn inbox closed; aborting host"
                                    );
                                    std::process::exit(78);
                                }
                            };
                            publish_commit(
                                &session,
                                &key,
                                &room_aead,
                                &room_id,
                                &host_key,
                                reject_commit(&proposal, edit_reject::APPLY, tip),
                            );
                            log::warn!(
                                "edit accept: despawn inbox closed after CAS (reject APPLY; overlay kept; tip unchanged)"
                            );
                        }
                    }
                    Ok((None, Some(ReadyEdit::Live(ready)))) => {
                        // Accept Commit is published by HostLoop after barrier — tip stays put.
                        if live_tx.send(ready.clone()).is_err() {
                            let tip = match live_edit.lock() {
                                Ok(mut led) => {
                                    led.rollback_mirrors(&ready);
                                    led.tip
                                }
                                Err(_) => {
                                    log::error!(
                                        "LiveEditLedger poisoned on live inbox closed; aborting host"
                                    );
                                    std::process::exit(78);
                                }
                            };
                            publish_commit(
                                &session,
                                &key,
                                &room_aead,
                                &room_id,
                                &host_key,
                                reject_commit(ready.proposal(), edit_reject::APPLY, tip),
                            );
                            log::warn!(
                                "edit accept: live inbox closed (rolled back mirrors + reject APPLY)"
                            );
                        }
                    }
                    Ok((Some(commit), None)) => {
                        publish_commit(
                            &session,
                            &key,
                            &room_aead,
                            &room_id,
                            &host_key,
                            commit,
                        );
                    }
                    Ok((None, None)) => {
                        log::warn!("edit proposal produced empty outcome");
                    }
                    Ok((Some(_), Some(ReadyEdit::Live(_)))) => {
                        log::warn!("edit live accept must not carry Commit before barrier");
                    }
                    Ok((Some(_), Some(ReadyEdit::Spawn { .. } | ReadyEdit::Despawn { .. }))) => {
                        log::warn!("edit spawn/despawn must not pre-build Commit before inbox send");
                    }
                    Err(err) => {
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
    live_edit: &SharedLiveEdit,
    control_rate: &Arc<Mutex<SharedControlRate>>,
) -> Result<(Option<EditCommit>, Option<ReadyEdit>)> {
    let proposal = EditProposal::decode(plain).context("EditProposal decode")?;
    let reject_rev = {
        let led = live_edit
            .lock()
            .map_err(|_| anyhow!("LiveEditLedger poisoned"))?;
        led.tip
    };
    if proposal.room_id != room_id {
        return Ok((
            Some(reject_commit(&proposal, edit_reject::PAYLOAD, reject_rev)),
            None,
        ));
    }
    if proposal.session_pub.len() != PUBLIC_KEY_LENGTH {
        return Ok((
            Some(reject_commit(&proposal, edit_reject::PAYLOAD, reject_rev)),
            None,
        ));
    }
    if verify_edit_proposal(&proposal).is_err() {
        return Ok((
            Some(reject_commit(&proposal, edit_reject::SIG, reject_rev)),
            None,
        ));
    }
    if proposal.ops.len() != 1 {
        return Ok((
            Some(reject_commit(&proposal, edit_reject::OPS_COUNT, reject_rev)),
            None,
        ));
    }
    let op = match proposal.ops[0].op.as_ref() {
        Some(Op::Spawn(s)) => {
            if !spawn_payload_ok(s) {
                return Ok((
                    Some(reject_commit(&proposal, edit_reject::PAYLOAD, reject_rev)),
                    None,
                ));
            }
            OpKind::Spawn(s.clone())
        }
        Some(Op::Despawn(d)) => OpKind::Despawn(d.entity_id),
        Some(Op::Reparent(r)) => OpKind::Reparent(*r),
        Some(Op::SetField(f)) => OpKind::SetField(f.clone()),
        None => {
            return Ok((
                Some(reject_commit(
                    &proposal,
                    edit_reject::OP_UNSUPPORTED,
                    reject_rev,
                )),
                None,
            ));
        }
    };

    let auth = match http_auth.snapshot() {
        Ok(a) => a,
        Err(_) => {
            return Ok((
                Some(reject_commit(&proposal, edit_reject::AUTH, reject_rev)),
                None,
            ));
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
            return Ok((
                Some(reject_commit(&proposal, edit_reject::AUTH, reject_rev)),
                None,
            ));
        }
    }
    {
        let mut rate = control_rate
            .lock()
            .map_err(|_| anyhow!("SharedControlRate poisoned"))?;
        if !rate.allow() {
            return Ok((
                Some(reject_commit(&proposal, edit_reject::RATE, reject_rev)),
                None,
            ));
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
            return Ok((
                Some(reject_commit(&proposal, edit_reject::AUTH, reject_rev)),
                None,
            ));
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
            live_edit,
        ),
        OpKind::Despawn(entity_id) => apply_despawn(
            &proposal,
            entity_id,
            room_id,
            world_url,
            &auth.token,
            &auth.pop_key,
            shared_scene,
            live_edit,
        ),
        OpKind::Reparent(r) => apply_reparent(&proposal, r, live_edit),
        OpKind::SetField(f) => apply_set_field(&proposal, f, live_edit),
    }
}

enum OpKind {
    Spawn(EditSpawn),
    Despawn(u32),
    Reparent(EditReparent),
    SetField(EditSetField),
}

fn is_blake3_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn spawn_payload_ok(spawn: &EditSpawn) -> bool {
    if !is_blake3_hex(&spawn.blake3) {
        return false;
    }
    if spawn.spawn_ticket.is_empty()
        || spawn.spawn_ticket.len() > alchemy_protocol::decode_limits::MAX_JWT_STRING_BYTES
    {
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
    live_edit: &SharedLiveEdit,
) -> Result<(Option<EditCommit>, Option<ReadyEdit>)> {
    let blake3 = spawn.blake3.to_ascii_lowercase();
    match append_room_scene_entry(
        world_url,
        room_id,
        token,
        pop_key,
        shared_scene,
        &blake3,
        &spawn.spawn_ticket,
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
            // Overlay CAS done (VCS debt). Tip+Commit only after inbox send succeeds.
            // Ledger entity row lands on physics success (HostLoop).
            Ok((
                None,
                Some(ReadyEdit::Spawn {
                    ready: ReadySpawn { entry, scene },
                    proposal: proposal.clone(),
                }),
            ))
        }
        Err(_) => {
            let tip = live_edit
                .lock()
                .map_err(|_| anyhow!("LiveEditLedger poisoned"))?
                .tip;
            Ok((Some(reject_commit(proposal, edit_reject::WORLD, tip)), None))
        }
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
    live_edit: &SharedLiveEdit,
) -> Result<(Option<EditCommit>, Option<ReadyEdit>)> {
    match delete_room_scene_entry(world_url, room_id, token, pop_key, shared_scene, entity_id) {
        Ok(scene) => {
            // Overlay CAS done (VCS debt). Tip+Commit only after inbox send succeeds.
            // Ledger despawn on physics success only (HostLoop).
            Ok((
                None,
                Some(ReadyEdit::Despawn {
                    ready: ReadyDespawn { entity_id, scene },
                    proposal: proposal.clone(),
                }),
            ))
        }
        Err(_) => {
            let tip = live_edit
                .lock()
                .map_err(|_| anyhow!("LiveEditLedger poisoned"))?
                .tip;
            Ok((Some(reject_commit(proposal, edit_reject::WORLD, tip)), None))
        }
    }
}

fn apply_reparent(
    proposal: &EditProposal,
    r: EditReparent,
    live_edit: &SharedLiveEdit,
) -> Result<(Option<EditCommit>, Option<ReadyEdit>)> {
    let mut led = live_edit
        .lock()
        .map_err(|_| anyhow!("LiveEditLedger poisoned"))?;
    let tip = led.tip;
    if !led.entities.contains(&r.child_entity_id) {
        return Ok((
            Some(reject_commit(proposal, edit_reject::NOT_FOUND, tip)),
            None,
        ));
    }
    if r.new_parent_entity_id != 0 && !led.entities.contains(&r.new_parent_entity_id) {
        return Ok((
            Some(reject_commit(proposal, edit_reject::NOT_FOUND, tip)),
            None,
        ));
    }
    if led.parent_of(r.child_entity_id) != r.expected_parent_entity_id {
        return Ok((
            Some(reject_commit(
                proposal,
                edit_reject::STALE_PRECONDITION,
                tip,
            )),
            None,
        ));
    }
    if led.would_cycle(r.child_entity_id, r.new_parent_entity_id) {
        return Ok((Some(reject_commit(proposal, edit_reject::CYCLE, tip)), None));
    }
    let prev_parent = led.parent_of(r.child_entity_id);
    led.parents
        .insert(r.child_entity_id, r.new_parent_entity_id);
    // Tip + accept Commit deferred until HostLoop barrier succeeds.
    Ok((
        None,
        Some(ReadyEdit::Live(ReadyLiveEdit::Reparent {
            proposal: proposal.clone(),
            op: r,
            prev_parent,
        })),
    ))
}

fn apply_set_field(
    proposal: &EditProposal,
    f: EditSetField,
    live_edit: &SharedLiveEdit,
) -> Result<(Option<EditCommit>, Option<ReadyEdit>)> {
    let mut led = live_edit
        .lock()
        .map_err(|_| anyhow!("LiveEditLedger poisoned"))?;
    let tip = led.tip;
    if !known_field_id(f.field_id) {
        return Ok((
            Some(reject_commit(proposal, edit_reject::OP_UNSUPPORTED, tip)),
            None,
        ));
    }
    if !led.entities.contains(&f.entity_id) {
        return Ok((
            Some(reject_commit(proposal, edit_reject::NOT_FOUND, tip)),
            None,
        ));
    }
    let Some(cur) = current_field_value(&led, f.entity_id, f.field_id) else {
        return Ok((
            Some(reject_commit(proposal, edit_reject::NOT_FOUND, tip)),
            None,
        ));
    };
    let cur_hash = field_value_hash(&cur);
    if f.expected_value_hash.as_slice() != cur_hash.as_slice() {
        return Ok((
            Some(reject_commit(
                proposal,
                edit_reject::STALE_PRECONDITION,
                tip,
            )),
            None,
        ));
    }
    let prev_value = cur;
    match f.field_id {
        alchemy_protocol::edit_field_id::FLAGS => {
            let Some(flags) = decode_flags(&f.value) else {
                return Ok((
                    Some(reject_commit(proposal, edit_reject::PAYLOAD, tip)),
                    None,
                ));
            };
            led.flags.insert(f.entity_id, flags);
        }
        alchemy_protocol::edit_field_id::COLOR_RGBA => {
            let Some(rgba) = decode_color_rgba(&f.value) else {
                return Ok((
                    Some(reject_commit(proposal, edit_reject::PAYLOAD, tip)),
                    None,
                ));
            };
            led.colors.insert(f.entity_id, rgba);
        }
        alchemy_protocol::edit_field_id::BLAKE3 => {
            let Ok(hex) = std::str::from_utf8(&f.value) else {
                return Ok((
                    Some(reject_commit(proposal, edit_reject::PAYLOAD, tip)),
                    None,
                ));
            };
            let hex = hex.to_ascii_lowercase();
            if !is_blake3_hex(&hex) {
                return Ok((
                    Some(reject_commit(proposal, edit_reject::PAYLOAD, tip)),
                    None,
                ));
            }
            led.blake3.insert(f.entity_id, hex);
        }
        _ => {
            return Ok((
                Some(reject_commit(proposal, edit_reject::OP_UNSUPPORTED, tip)),
                None,
            ));
        }
    }
    // Tip + accept Commit deferred until HostLoop barrier succeeds.
    Ok((
        None,
        Some(ReadyEdit::Live(ReadyLiveEdit::SetField {
            proposal: proposal.clone(),
            op: f,
            prev_value,
        })),
    ))
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
                log::warn!("edit commit put failed after Ready enqueue: {e}");
            }
        }
        Err(e) => log::warn!("edit commit seal failed after Ready enqueue: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live_edit::new_shared_live_edit;
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
    fn accept_commit_uses_live_tip() {
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
        let c = accept_commit(&p, 42);
        assert!(c.accepted);
        assert_eq!(c.scene_rev, 42);
    }

    #[test]
    fn concurrent_reparent_same_child_one_wins() {
        let live = new_shared_live_edit();
        {
            let mut led = live.lock().unwrap();
            led.entities.insert(10);
            led.entities.insert(1);
            led.entities.insert(2);
            led.parents.insert(10, 1);
        }
        let key = generate_signing_key();
        let mut p1 = EditProposal {
            session_pub: public_key_bytes(&key),
            control_seq: 1,
            base_scene_rev: 0,
            op_id: 1,
            ops: vec![EditOp {
                op: Some(Op::Reparent(EditReparent {
                    child_entity_id: 10,
                    expected_parent_entity_id: 1,
                    new_parent_entity_id: 2,
                })),
            }],
            sig: vec![],
            room_id: "local".into(),
        };
        sign_edit_proposal(&key, &mut p1);
        let r = EditReparent {
            child_entity_id: 10,
            expected_parent_entity_id: 1,
            new_parent_entity_id: 2,
        };
        let (c1, ready) = apply_reparent(&p1, r, &live).unwrap();
        assert!(c1.is_none(), "live accept Commit waits for barrier");
        assert!(ready.is_some());
        {
            let led = live.lock().unwrap();
            assert_eq!(led.tip, 0);
            assert_eq!(led.parent_of(10), 2);
        }

        let mut p2 = p1.clone();
        p2.op_id = 2;
        p2.ops = vec![EditOp {
            op: Some(Op::Reparent(EditReparent {
                child_entity_id: 10,
                expected_parent_entity_id: 1,
                new_parent_entity_id: 0,
            })),
        }];
        sign_edit_proposal(&key, &mut p2);
        let (c2, ready2) = apply_reparent(
            &p2,
            EditReparent {
                child_entity_id: 10,
                expected_parent_entity_id: 1,
                new_parent_entity_id: 0,
            },
            &live,
        )
        .unwrap();
        let c2 = c2.expect("stale reject Commit");
        assert!(!c2.accepted);
        assert_eq!(c2.reject_reason, edit_reject::STALE_PRECONDITION);
        assert!(ready2.is_none());
    }

    #[test]
    fn non_overlapping_set_fields_both_commit() {
        use alchemy_protocol::{encode_flags, hash_flags};
        let live = new_shared_live_edit();
        {
            let mut led = live.lock().unwrap();
            led.entities.insert(10);
            led.entities.insert(11);
            led.flags.insert(10, 0);
            led.flags.insert(11, 0);
        }
        let key = generate_signing_key();
        let mk = |entity: u32, op_id: u64| {
            let mut p = EditProposal {
                session_pub: public_key_bytes(&key),
                control_seq: op_id,
                base_scene_rev: 0,
                op_id,
                ops: vec![EditOp {
                    op: Some(Op::SetField(EditSetField {
                        entity_id: entity,
                        field_id: alchemy_protocol::edit_field_id::FLAGS,
                        value: encode_flags(1).to_vec(),
                        expected_value_hash: hash_flags(0).to_vec(),
                    })),
                }],
                sig: vec![],
                room_id: "local".into(),
            };
            sign_edit_proposal(&key, &mut p);
            p
        };
        let p1 = mk(10, 1);
        let f1 = match p1.ops[0].op.as_ref().unwrap() {
            Op::SetField(f) => f.clone(),
            _ => unreachable!(),
        };
        let (c1, r1) = apply_set_field(&p1, f1, &live).unwrap();
        assert!(c1.is_none());
        assert!(r1.is_some());
        let p2 = mk(11, 2);
        let f2 = match p2.ops[0].op.as_ref().unwrap() {
            Op::SetField(f) => f.clone(),
            _ => unreachable!(),
        };
        let (c2, r2) = apply_set_field(&p2, f2, &live).unwrap();
        assert!(c2.is_none());
        assert!(r2.is_some());
        let led = live.lock().unwrap();
        assert_eq!(led.tip, 0);
        assert_eq!(led.flags.get(&10).copied(), Some(1));
        assert_eq!(led.flags.get(&11).copied(), Some(1));
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
            spawn_ticket: "test.spawn.ticket".into(),
        };
        assert!(spawn_payload_ok(&good));
        let mut short = good.clone();
        short.blake3 = "abcd".into();
        assert!(!spawn_payload_ok(&short));
        let mut bad_scale = good;
        bad_scale.sx = 0.0;
        assert!(!spawn_payload_ok(&bad_scale));
    }
}
