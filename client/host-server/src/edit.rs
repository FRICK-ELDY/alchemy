//! Deterministic edit lane (`…/edit`) — Spawn/Despawn + live Reparent/SetField + Save
//! (x00b2–x00b9 / x00e0 facade / x00e2 SoftLease / x00e3 TRASH).
//!
//! Guarantees: AEAD `EditProposal` → session Ed25519 → Dual-auth / shared control rate /
//! `control_seq`; single op.
//! Spawn/Despawn (x00b9): host-local id + pending ticket/despawn (no live overlay CAS);
//! inbox enqueue only here; **tip + accept Commit after physics/local apply in HostLoop**
//! (inbox closed / apply fail → reject `APPLY` + `release_reserved`, tip unchanged).
//! Reparent/SetField: SoftLease gate (x00e2) then quarantine then optimistic
//! [`LiveEditLedger`] → `scene_store` apply; SoftLease conflict → reject `SOFT_LEASE`;
//! `TRASH_ENTITY_ID` as reparent parent → `NOT_FOUND` (despawn path only; x00e3);
//! **tip + accept Commit after** FrameBridge queue **and** barrier — never rewind tip;
//! queue/barrier/inbox fail → reject `APPLY`.
//! Save (x00b7–x00b9): world packed CAS folds pending spawns/despawns + parent/SetField;
//! install shared scene + `force_reseal` when packed revision advances; **tip unchanged**;
//! accept Commit with current tip.
//!
//! Non-guarantees: multi-op batch; Undo UI; old inventory key removal; Spatial VCS;
//! crash durability of unsaved pending; SoftLease ↔ physical grab;
//! **`control_seq` is committed before physics/apply** — later HostLoop `APPLY` reject
//! still leaves seq advanced (same window as inventory HTTP-era; wider now that tip waits).

use alchemy_protocol::edit_op::Op;
use alchemy_protocol::wire_aead::{self, AeadKind};
use alchemy_protocol::wire_crypto::{
    sign_edit_commit, verify_edit_proposal, SigningKey, PUBLIC_KEY_LENGTH,
};
use alchemy_protocol::{
    decode_color_rgba, decode_f64, decode_flags, edit_reject, field_value_hash, is_pos_field_id,
    known_field_id, EditCommit, EditProposal, EditReparent, EditSetField, EditSpawn, Message,
    SceneEntry,
};
use anyhow::{anyhow, Context, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use zenoh::Wait;

use crate::control_rate::SharedControlRate;
use crate::inventory_despawn::ReadyDespawn;
use crate::inventory_spawn::ReadySpawn;
use alchemy_shared_sim::PropValue;

use crate::live_edit::{
    current_field_value, edit_reject_for_apply, LiveEditLedger, ReadyLiveEdit, SharedLiveEdit,
};
use crate::room_aead::{self, SharedRoomAead};
use crate::room_token::{
    install_shared_scene_if_newer, save_overlay_to_canonical, RoomScene, SceneFieldPatch,
    SceneParentPatch, SceneSaveError, SceneSavePatches, SceneSpawnPatch, SharedHostHttpAuth,
};
use crate::session::SessionTable;
use alchemy_shared_sim::actor_id_from_sub;

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
    force_reseal: Arc<AtomicBool>,
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
                    &force_reseal,
                ) {
                    Ok((None, Some(ReadyEdit::Spawn { mut ready, proposal }))) => {
                        // Tip/Commit deferred to HostLoop after physics-ok (x00b9).
                        let entity_id = ready.entry.entity_id;
                        ready.edit_proposal = Some(proposal.clone());
                        if spawn_tx.send(ready).is_err() {
                            let tip = match live_edit.lock() {
                                Ok(mut led) => {
                                    led.release_reserved(entity_id);
                                    led.tip
                                }
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
                                "edit spawn inbox closed (reject APPLY + release_reserved; tip unchanged)"
                            );
                        }
                    }
                    Ok((None, Some(ReadyEdit::Despawn { mut ready, proposal }))) => {
                        // Tip/Commit deferred to HostLoop after local apply-ok (x00b9).
                        ready.edit_proposal = Some(proposal.clone());
                        if despawn_tx.send(ready).is_err() {
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
                                "edit despawn inbox closed (reject APPLY; tip unchanged)"
                            );
                        }
                    }
                    Ok((None, Some(ReadyEdit::Live(ready)))) => {
                        // Accept Commit is published by HostLoop after barrier — tip stays put.
                        if live_tx.send(ready.clone()).is_err() {
                            let tip = match live_edit.lock() {
                                Ok(mut led) => {
                                    // Tip unchanged + reject APPLY even if compensating apply fails.
                                    if !led.rollback_mirrors(&ready) {
                                        log::warn!(
                                            "live inbox closed: rollback_mirrors failed op_id={}; tip unchanged, reject still published",
                                            ready.proposal().op_id
                                        );
                                    }
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
                        log::warn!(
                            "edit spawn/despawn must not pre-build Commit before HostLoop apply"
                        );
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
    force_reseal: &AtomicBool,
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
        Some(Op::Save(_)) => OpKind::Save,
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
    let actor_id = {
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
        // SoftLease holder from Room Token `sub` (Dual-auth already bound this pub).
        let Some(sub) = table.sub_for_pub(&proposal.session_pub) else {
            return Ok((
                Some(reject_commit(&proposal, edit_reject::AUTH, reject_rev)),
                None,
            ));
        };
        actor_id_from_sub(sub)
    };

    match op {
        OpKind::Spawn(spawn) => apply_spawn(&proposal, &spawn, shared_scene, live_edit),
        OpKind::Despawn(entity_id) => apply_despawn(&proposal, entity_id, shared_scene, live_edit),
        OpKind::Reparent(r) => apply_reparent(&proposal, r, live_edit, actor_id),
        OpKind::SetField(f) => apply_set_field(&proposal, f, live_edit, actor_id),
        OpKind::Save => apply_save(
            &proposal,
            room_id,
            world_url,
            &auth.token,
            &auth.pop_key,
            shared_scene,
            live_edit,
            force_reseal,
        ),
    }
}

enum OpKind {
    Spawn(EditSpawn),
    Despawn(u32),
    Reparent(EditReparent),
    SetField(EditSetField),
    Save,
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

fn apply_spawn(
    proposal: &EditProposal,
    spawn: &EditSpawn,
    shared_scene: &Arc<RwLock<RoomScene>>,
    live_edit: &SharedLiveEdit,
) -> Result<(Option<EditCommit>, Option<ReadyEdit>)> {
    let blake3 = spawn.blake3.to_ascii_lowercase();
    let entity_id = {
        let scene = shared_scene
            .read()
            .map_err(|_| anyhow!("shared scene RwLock poisoned"))?;
        let mut led = live_edit
            .lock()
            .map_err(|_| anyhow!("LiveEditLedger poisoned"))?;
        led.alloc_entity_id(&scene.entries)
    };
    let mut entry = SceneEntry {
        entity_id,
        blake3,
        x: spawn.x,
        y: spawn.y,
        z: spawn.z,
        half_extent: 0.0,
        flags: 0,
        r: 0.85,
        g: 0.85,
        b: 0.9,
        a: 1.0,
        qx: spawn.qx,
        qy: spawn.qy,
        qz: spawn.qz,
        qw: spawn.qw,
        sx: spawn.sx,
        sy: spawn.sy,
        sz: spawn.sz,
        parent_entity_id: 0,
    };
    alchemy_protocol::normalize_scene_entry_trs(&mut entry);
    // Tip/Commit + ledger/pending after physics-ok in HostLoop (x00b9).
    Ok((
        None,
        Some(ReadyEdit::Spawn {
            ready: ReadySpawn {
                entry,
                spawn_ticket: spawn.spawn_ticket.clone(),
                edit_proposal: None,
            },
            proposal: proposal.clone(),
        }),
    ))
}

fn apply_despawn(
    proposal: &EditProposal,
    entity_id: u32,
    shared_scene: &Arc<RwLock<RoomScene>>,
    live_edit: &SharedLiveEdit,
) -> Result<(Option<EditCommit>, Option<ReadyEdit>)> {
    let tip = live_edit
        .lock()
        .map_err(|_| anyhow!("LiveEditLedger poisoned"))?
        .tip;
    let present = shared_scene
        .read()
        .map_err(|_| anyhow!("shared scene RwLock poisoned"))?
        .entries
        .iter()
        .any(|e| e.entity_id == entity_id);
    if !present {
        return Ok((
            Some(reject_commit(proposal, edit_reject::PAYLOAD, tip)),
            None,
        ));
    }
    // Tip/Commit + ledger after local apply-ok in HostLoop (x00b9).
    Ok((
        None,
        Some(ReadyEdit::Despawn {
            ready: ReadyDespawn {
                entity_id,
                edit_proposal: None,
            },
            proposal: proposal.clone(),
        }),
    ))
}

#[allow(clippy::too_many_arguments)]
fn apply_save(
    proposal: &EditProposal,
    room_id: &str,
    world_url: &str,
    token: &str,
    pop_key: &SigningKey,
    shared_scene: &Arc<RwLock<RoomScene>>,
    live_edit: &SharedLiveEdit,
    force_reseal: &AtomicBool,
) -> Result<(Option<EditCommit>, Option<ReadyEdit>)> {
    let (tip, patches) = {
        let led = live_edit
            .lock()
            .map_err(|_| anyhow!("LiveEditLedger poisoned"))?;
        let scene = shared_scene
            .read()
            .map_err(|_| anyhow!("shared scene RwLock poisoned"))?;
        (led.tip, save_patches_from_ledger(&led, &scene))
    };
    match save_overlay_to_canonical(world_url, room_id, token, pop_key, shared_scene, &patches) {
        Ok(scene) => {
            // Packed revision may advance while composed entries stay equal (overlay→canon).
            if install_shared_scene_if_newer(shared_scene, scene)? {
                force_reseal.store(true, Ordering::SeqCst);
            }
            if let Ok(mut led) = live_edit.lock() {
                led.clear_save_pending();
            }
            // Live tip is independent of packed world revision (x00b7).
            Ok((Some(accept_commit(proposal, tip)), None))
        }
        Err(SceneSaveError::Forbidden) => Ok((
            Some(reject_commit(proposal, edit_reject::SAVE_FORBIDDEN, tip)),
            None,
        )),
        Err(SceneSaveError::Conflict) => Ok((
            Some(reject_commit(proposal, edit_reject::SAVE_CONFLICT, tip)),
            None,
        )),
        Err(SceneSaveError::ParentCycle) => {
            Ok((Some(reject_commit(proposal, edit_reject::CYCLE, tip)), None))
        }
        Err(SceneSaveError::InvalidEntry) | Err(SceneSaveError::DuplicateBlake3) => Ok((
            Some(reject_commit(proposal, edit_reject::PAYLOAD, tip)),
            None,
        )),
        Err(SceneSaveError::Other(_)) => {
            Ok((Some(reject_commit(proposal, edit_reject::WORLD, tip)), None))
        }
    }
}

/// Diff live ledger against shared scene so empty Save stays idempotent (x00b8 / x00b9).
fn save_patches_from_ledger(
    led: &crate::live_edit::LiveEditLedger,
    scene: &RoomScene,
) -> SceneSavePatches {
    let mut by_id: std::collections::HashMap<u32, &alchemy_protocol::SceneEntry> =
        std::collections::HashMap::with_capacity(scene.entries.len());
    for e in &scene.entries {
        by_id.insert(e.entity_id, e);
    }

    let mut parents = Vec::new();
    let mut fields = Vec::new();
    let mut spawns = Vec::new();

    for pending in led.pending_spawns.values() {
        let eid = pending.entry.entity_id;
        let mut entry = pending.entry.clone();
        entry.parent_entity_id = led.parent_of(eid);
        if let Some(flags) = led.flags_of(eid) {
            entry.flags = flags;
        }
        if let Some(color) = led.color_of(eid) {
            entry.r = color[0];
            entry.g = color[1];
            entry.b = color[2];
            entry.a = color[3];
        }
        if let Some(bytes) = led.blake3_bytes(eid) {
            if let Ok(hex) = std::str::from_utf8(bytes) {
                entry.blake3 = hex.to_owned();
            }
        }
        spawns.push(SceneSpawnPatch {
            entry,
            spawn_ticket: pending.spawn_ticket.clone(),
        });
    }

    for &eid in led.entity_ids() {
        // Pending spawns carry full entry in `spawns`; skip parent/field diffs for them
        // (already folded into the spawn payload above).
        if led.pending_spawns.contains_key(&eid) {
            continue;
        }
        // Trashed rows stay in sparse until GC/Save clear — not live Save diffs
        // (canonical delete is `pending_despawns` / x00e3).
        if !led.contains_entity(eid) {
            continue;
        }
        let canon = by_id.get(&eid);
        let live_parent = led.parent_of(eid);
        let canon_parent = canon.map(|e| e.parent_entity_id).unwrap_or(0);
        if live_parent != canon_parent {
            parents.push(SceneParentPatch {
                entity_id: eid,
                parent_entity_id: live_parent,
            });
        }

        let mut field = SceneFieldPatch {
            entity_id: eid,
            flags: None,
            r: None,
            g: None,
            b: None,
            a: None,
            blake3: None,
        };
        let mut dirty = false;

        if let Some(live_flags) = led.flags_of(eid) {
            let canon_flags = canon.map(|e| e.flags).unwrap_or(0);
            if live_flags != canon_flags {
                field.flags = Some(live_flags);
                dirty = true;
            }
        }
        if let Some(live_color) = led.color_of(eid) {
            let (cr, cg, cb, ca) = match canon {
                Some(e) => (e.r, e.g, e.b, if e.a == 0.0 { 1.0 } else { e.a }),
                None => (0.0, 0.0, 0.0, 1.0),
            };
            if live_color != [cr, cg, cb, ca] {
                field.r = Some(live_color[0]);
                field.g = Some(live_color[1]);
                field.b = Some(live_color[2]);
                field.a = Some(live_color[3]);
                dirty = true;
            }
        }
        if let Some(bytes) = led.blake3_bytes(eid) {
            if let Ok(live_blake3) = std::str::from_utf8(bytes) {
                let canon_blake3 = canon.map(|e| e.blake3.as_str()).unwrap_or("");
                if live_blake3 != canon_blake3 {
                    field.blake3 = Some(live_blake3.to_owned());
                    dirty = true;
                }
            }
        }

        if dirty {
            fields.push(field);
        }
    }

    let mut despawns: Vec<u32> = led.pending_despawns.iter().copied().collect();
    despawns.sort_unstable();

    SceneSavePatches {
        parents,
        fields,
        spawns,
        despawns,
    }
}

fn apply_reparent(
    proposal: &EditProposal,
    r: EditReparent,
    live_edit: &SharedLiveEdit,
    actor_id: u64,
) -> Result<(Option<EditCommit>, Option<ReadyEdit>)> {
    let mut led = live_edit
        .lock()
        .map_err(|_| anyhow!("LiveEditLedger poisoned"))?;
    let tip = led.tip;
    if !led.contains_entity(r.child_entity_id) {
        return Ok((
            Some(reject_commit(proposal, edit_reject::NOT_FOUND, tip)),
            None,
        ));
    }
    // TRASH_ENTITY_ID is despawn-only (x00e3); not a reparent target.
    if alchemy_shared_sim::is_trash_entity_id(r.new_parent_entity_id)
        || alchemy_shared_sim::is_trash_entity_id(r.child_entity_id)
    {
        return Ok((
            Some(reject_commit(proposal, edit_reject::NOT_FOUND, tip)),
            None,
        ));
    }
    if r.new_parent_entity_id != 0 && !led.contains_entity(r.new_parent_entity_id) {
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
    if led.soft_lease_allows(r.child_entity_id, actor_id).is_err() {
        return Ok((
            Some(reject_commit(proposal, edit_reject::SOFT_LEASE, tip)),
            None,
        ));
    }
    let prev_parent = led.parent_of(r.child_entity_id);
    let outcome = led.apply_reparent(r.child_entity_id, r.new_parent_entity_id);
    if !outcome.is_success() {
        // Store is SSoT for cycle / depth / residual stale-missing after quarantine.
        // SoftLease not refreshed (allows-only above).
        return Ok((
            Some(reject_commit(proposal, edit_reject_for_apply(outcome), tip)),
            None,
        ));
    }
    // Sparse success → refresh SoftLease (barrier fail keeps it for same-actor retry).
    // Same-lock: allows just passed, so Err is unreachable; still compensate if it fires.
    let child = r.child_entity_id;
    let ready = ReadyLiveEdit::Reparent {
        proposal: proposal.clone(),
        op: r,
        prev_parent,
    };
    if led.try_acquire_soft_lease(child, actor_id).is_err() {
        if !led.rollback_mirrors(&ready) {
            log::warn!(
                "SoftLease acquire failed after reparent: rollback_mirrors failed op_id={}; tip unchanged, reject still published",
                ready.proposal().op_id
            );
        }
        return Ok((
            Some(reject_commit(proposal, edit_reject::SOFT_LEASE, tip)),
            None,
        ));
    }
    // Tip + accept Commit deferred until HostLoop barrier succeeds.
    Ok((None, Some(ReadyEdit::Live(ready))))
}

fn apply_set_field(
    proposal: &EditProposal,
    f: EditSetField,
    live_edit: &SharedLiveEdit,
    actor_id: u64,
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
    if !led.contains_entity(f.entity_id) {
        return Ok((
            Some(reject_commit(proposal, edit_reject::NOT_FOUND, tip)),
            None,
        ));
    }
    // POS_*: SoftLease here; Epoch axis hash at physics queue (no sparse write · x00f2).
    if is_pos_field_id(f.field_id) {
        return apply_set_field_pos(proposal, f, &mut led, actor_id, tip);
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
    // Decode before SoftLease so invalid PAYLOAD cannot refresh TTL.
    let prev_value = cur;
    let prop = match f.field_id {
        alchemy_protocol::edit_field_id::FLAGS => {
            let Some(flags) = decode_flags(&f.value) else {
                return Ok((
                    Some(reject_commit(proposal, edit_reject::PAYLOAD, tip)),
                    None,
                ));
            };
            PropValue::U32(flags)
        }
        alchemy_protocol::edit_field_id::COLOR_RGBA => {
            let Some(rgba) = decode_color_rgba(&f.value) else {
                return Ok((
                    Some(reject_commit(proposal, edit_reject::PAYLOAD, tip)),
                    None,
                ));
            };
            PropValue::F32x4(rgba)
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
            PropValue::Bytes(hex.into_bytes())
        }
        _ => {
            return Ok((
                Some(reject_commit(proposal, edit_reject::OP_UNSUPPORTED, tip)),
                None,
            ));
        }
    };
    finish_set_field_ready(proposal, f, &mut led, actor_id, tip, prev_value, prop)
}

/// Inspector world-position SetField (x00f2). SoftLease here; Epoch hash at queue.
///
/// Does **not** write a sparse POS mirror (Epoch is pose SSoT). Avoids fabricating
/// `0.0` prev on rollback when the property was never set.
fn apply_set_field_pos(
    proposal: &EditProposal,
    f: EditSetField,
    led: &mut LiveEditLedger,
    actor_id: u64,
    tip: u64,
) -> Result<(Option<EditCommit>, Option<ReadyEdit>)> {
    if decode_f64(&f.value).is_none() {
        return Ok((
            Some(reject_commit(proposal, edit_reject::PAYLOAD, tip)),
            None,
        ));
    }
    if led.soft_lease_allows(f.entity_id, actor_id).is_err() {
        return Ok((
            Some(reject_commit(proposal, edit_reject::SOFT_LEASE, tip)),
            None,
        ));
    }
    let entity_id = f.entity_id;
    let ready = ReadyLiveEdit::SetField {
        proposal: proposal.clone(),
        op: f,
        // Empty = no sparse POS row to restore on queue/barrier fail.
        prev_value: Vec::new(),
    };
    if led.try_acquire_soft_lease(entity_id, actor_id).is_err() {
        return Ok((
            Some(reject_commit(proposal, edit_reject::SOFT_LEASE, tip)),
            None,
        ));
    }
    Ok((None, Some(ReadyEdit::Live(ready))))
}

fn finish_set_field_ready(
    proposal: &EditProposal,
    f: EditSetField,
    led: &mut LiveEditLedger,
    actor_id: u64,
    tip: u64,
    prev_value: Vec<u8>,
    prop: PropValue,
) -> Result<(Option<EditCommit>, Option<ReadyEdit>)> {
    if led.soft_lease_allows(f.entity_id, actor_id).is_err() {
        return Ok((
            Some(reject_commit(proposal, edit_reject::SOFT_LEASE, tip)),
            None,
        ));
    }
    let set_outcome = led.apply_set_field(f.entity_id, f.field_id, prop);
    if !set_outcome.is_success() {
        // SoftLease not refreshed on sparse failure.
        return Ok((
            Some(reject_commit(
                proposal,
                edit_reject_for_apply(set_outcome),
                tip,
            )),
            None,
        ));
    }
    // Same-lock: allows just passed, so Err is unreachable; still compensate if it fires.
    let entity_id = f.entity_id;
    let ready = ReadyLiveEdit::SetField {
        proposal: proposal.clone(),
        op: f,
        prev_value,
    };
    if led.try_acquire_soft_lease(entity_id, actor_id).is_err() {
        if !led.rollback_mirrors(&ready) {
            log::warn!(
                "SoftLease acquire failed after set_field: rollback_mirrors failed op_id={}; tip unchanged, reject still published",
                ready.proposal().op_id
            );
        }
        return Ok((
            Some(reject_commit(proposal, edit_reject::SOFT_LEASE, tip)),
            None,
        ));
    }
    // Tip + accept Commit deferred until HostLoop barrier succeeds.
    Ok((None, Some(ReadyEdit::Live(ready))))
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
            assert!(led.note_spawn_with_parent(1, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
            assert!(led.note_spawn_with_parent(2, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
            assert!(led.note_spawn_with_parent(10, 1, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
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
        let actor = actor_id_from_sub("alice");
        let (c1, ready) = apply_reparent(&p1, r, &live, actor).unwrap();
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
            actor,
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
            assert!(led.note_spawn_with_parent(10, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
            assert!(led.note_spawn_with_parent(11, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
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
        let actor = actor_id_from_sub("alice");
        let (c1, r1) = apply_set_field(&p1, f1, &live, actor).unwrap();
        assert!(c1.is_none());
        assert!(r1.is_some());
        let p2 = mk(11, 2);
        let f2 = match p2.ops[0].op.as_ref().unwrap() {
            Op::SetField(f) => f.clone(),
            _ => unreachable!(),
        };
        let (c2, r2) = apply_set_field(&p2, f2, &live, actor).unwrap();
        assert!(c2.is_none());
        assert!(r2.is_some());
        let led = live.lock().unwrap();
        assert_eq!(led.tip, 0);
        assert_eq!(led.flags_of(10), Some(1));
        assert_eq!(led.flags_of(11), Some(1));
    }

    #[test]
    fn soft_lease_conflict_rejects_other_actor_reparent() {
        let live = new_shared_live_edit();
        {
            let mut led = live.lock().unwrap();
            assert!(led.note_spawn_with_parent(1, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
            assert!(led.note_spawn_with_parent(2, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
            assert!(led.note_spawn_with_parent(10, 1, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
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
        let alice = actor_id_from_sub("alice");
        let bob = actor_id_from_sub("bob");
        let (c1, ready) = apply_reparent(&p1, r, &live, alice).unwrap();
        assert!(c1.is_none());
        assert!(ready.is_some());

        let mut p2 = p1.clone();
        p2.op_id = 2;
        // Parent already moved; use expected=2 so SoftLease is the reject reason.
        p2.ops = vec![EditOp {
            op: Some(Op::Reparent(EditReparent {
                child_entity_id: 10,
                expected_parent_entity_id: 2,
                new_parent_entity_id: 0,
            })),
        }];
        sign_edit_proposal(&key, &mut p2);
        let (c2, ready2) = apply_reparent(
            &p2,
            EditReparent {
                child_entity_id: 10,
                expected_parent_entity_id: 2,
                new_parent_entity_id: 0,
            },
            &live,
            bob,
        )
        .unwrap();
        let c2 = c2.expect("SoftLease reject Commit");
        assert!(!c2.accepted);
        assert_eq!(c2.reject_reason, edit_reject::SOFT_LEASE);
        assert!(ready2.is_none());
        let led = live.lock().unwrap();
        assert_eq!(led.parent_of(10), 2, "parent unchanged on SoftLease reject");
    }

    #[test]
    fn soft_lease_conflict_rejects_other_actor_set_field() {
        use alchemy_protocol::{encode_flags, hash_flags};
        let live = new_shared_live_edit();
        {
            let mut led = live.lock().unwrap();
            assert!(led.note_spawn_with_parent(10, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        }
        let key = generate_signing_key();
        let alice = actor_id_from_sub("alice");
        let bob = actor_id_from_sub("bob");
        let mut p1 = EditProposal {
            session_pub: public_key_bytes(&key),
            control_seq: 1,
            base_scene_rev: 0,
            op_id: 1,
            ops: vec![EditOp {
                op: Some(Op::SetField(EditSetField {
                    entity_id: 10,
                    field_id: alchemy_protocol::edit_field_id::FLAGS,
                    value: encode_flags(1).to_vec(),
                    expected_value_hash: hash_flags(0).to_vec(),
                })),
            }],
            sig: vec![],
            room_id: "local".into(),
        };
        sign_edit_proposal(&key, &mut p1);
        let f1 = match p1.ops[0].op.as_ref().unwrap() {
            Op::SetField(f) => f.clone(),
            _ => unreachable!(),
        };
        let (c1, ready) = apply_set_field(&p1, f1, &live, alice).unwrap();
        assert!(c1.is_none());
        assert!(ready.is_some());

        let mut p2 = p1.clone();
        p2.op_id = 2;
        p2.ops = vec![EditOp {
            op: Some(Op::SetField(EditSetField {
                entity_id: 10,
                field_id: alchemy_protocol::edit_field_id::FLAGS,
                value: encode_flags(2).to_vec(),
                expected_value_hash: hash_flags(1).to_vec(),
            })),
        }];
        sign_edit_proposal(&key, &mut p2);
        let f2 = match p2.ops[0].op.as_ref().unwrap() {
            Op::SetField(f) => f.clone(),
            _ => unreachable!(),
        };
        let (c2, ready2) = apply_set_field(&p2, f2, &live, bob).unwrap();
        let c2 = c2.expect("SoftLease reject Commit");
        assert!(!c2.accepted);
        assert_eq!(c2.reject_reason, edit_reject::SOFT_LEASE);
        assert!(ready2.is_none());
        let led = live.lock().unwrap();
        assert_eq!(
            led.flags_of(10),
            Some(1),
            "flags unchanged on SoftLease reject"
        );
    }

    #[test]
    fn soft_lease_not_refreshed_on_payload_reject() {
        use alchemy_protocol::hash_flags;
        let live = new_shared_live_edit();
        let alice = actor_id_from_sub("alice");
        let expires = {
            let mut led = live.lock().unwrap();
            assert!(led.note_spawn_with_parent(10, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
            assert!(led.try_acquire_soft_lease(10, alice).is_ok());
            led.soft_lease_of(10).unwrap().lease_expires_tick
        };
        let key = generate_signing_key();
        let mut p = EditProposal {
            session_pub: public_key_bytes(&key),
            control_seq: 1,
            base_scene_rev: 0,
            op_id: 1,
            ops: vec![EditOp {
                op: Some(Op::SetField(EditSetField {
                    entity_id: 10,
                    field_id: alchemy_protocol::edit_field_id::FLAGS,
                    value: vec![0xff], // invalid flags encoding
                    expected_value_hash: hash_flags(0).to_vec(),
                })),
            }],
            sig: vec![],
            room_id: "local".into(),
        };
        sign_edit_proposal(&key, &mut p);
        let f = match p.ops[0].op.as_ref().unwrap() {
            Op::SetField(f) => f.clone(),
            _ => unreachable!(),
        };
        let (c, ready) = apply_set_field(&p, f, &live, alice).unwrap();
        let c = c.expect("PAYLOAD reject");
        assert!(!c.accepted);
        assert_eq!(c.reject_reason, edit_reject::PAYLOAD);
        assert!(ready.is_none());
        let led = live.lock().unwrap();
        assert_eq!(
            led.soft_lease_of(10).unwrap().lease_expires_tick,
            expires,
            "invalid PAYLOAD must not refresh SoftLease TTL"
        );
        assert_eq!(led.flags_of(10), Some(0));
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
