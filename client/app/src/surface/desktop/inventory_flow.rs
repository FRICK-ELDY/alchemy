//! Desktop Inventory / put-back / take-out / Room Save flows (x006e / x007f / x0098 / x00b7).
//!
//! Guarantees: XR peer requests map onto the same Desktop `UiAction` paths; soft UI gates
//! fail closed on missing access / Intent session; every terminal put-back publishes
//! `Some(ok)` to the inventory mirror; take-out spawn puts are paced to the host
//! `SpawnRate` on the auth worker.
//! Non-guarantee: soft gates are UX only — the compute host / world stay the authority
//! (unknown catalog owner passes through to the host Dual check).

use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use crate::scene::SharedState;
use crate::system_ui::{
    rematch_scene_selection, DashTab, InventoryPeerRequest, SystemUi, UiAction,
};

use super::job::{arm_auth_job, spawn_auth_http, AuthJobOutcome};
use super::support::{catalog_owner_sub_for_room, publish_inventory_mirror_from_ui};

/// Map XR peer request → Desktop `UiAction`, applying XR selection into local state.
pub(super) fn apply_inventory_peer_request(
    ui: &mut SystemUi,
    shared: &SharedState,
    req: &InventoryPeerRequest,
) -> Option<UiAction> {
    match req {
        InventoryPeerRequest::Open => Some(UiAction::InventoryOpen),
        InventoryPeerRequest::Refresh => Some(UiAction::InventoryRefresh),
        InventoryPeerRequest::TakeOut { node_id, kind } => {
            // Override carries ids without injecting a ghost Drive row into the list/mirror.
            ui.state.inventory_take_out_override = Some((node_id.clone(), kind.clone()));
            if let Some(i) = ui
                .state
                .inventory_nodes
                .iter()
                .position(|(id, _, _)| id == node_id)
            {
                ui.state.inventory_selected = Some(i);
            }
            Some(UiAction::InventoryTakeOut)
        }
        InventoryPeerRequest::PutBack {
            entity_ids,
            correlation,
        } => {
            ui.state.scene_meshes = shared.scene_meshes();
            ui.state.scene_selected = rematch_scene_selection(&ui.state.scene_meshes, entity_ids);
            ui.state.inventory_selected = None;
            ui.state.inventory_put_back_correlation = *correlation;
            if ui.state.scene_selected.is_empty() {
                ui.state.inventory_status =
                    "XR put-back: selected meshes not in scene (Refresh / wait Manifest)".into();
                publish_put_back_outcome(ui, shared, false);
                ui.rebuild();
                return None;
            }
            Some(UiAction::InventoryPutBack)
        }
        InventoryPeerRequest::Despawn { entity_id } => {
            ui.state.scene_meshes = shared.scene_meshes();
            ui.state.scene_selected =
                rematch_scene_selection(&ui.state.scene_meshes, &[*entity_id]);
            ui.state.inventory_selected = None;
            if ui.state.scene_selected.is_empty() {
                ui.state.inventory_status =
                    "XR despawn: entity not in scene (Refresh / wait Manifest)".into();
                publish_inventory_mirror_from_ui(ui, shared);
                ui.rebuild();
                return None;
            }
            Some(UiAction::InventoryDespawn)
        }
        InventoryPeerRequest::RoomSaveChanges => Some(UiAction::RoomSaveChanges),
    }
}

/// Drive list + Manifest scene re-sync (Live only).
pub(super) fn inventory_refresh(ui: &mut SystemUi, shared: &SharedState) {
    if ui.state.inventory_blocked() {
        ui.state.tab = DashTab::Home;
        ui.state.home_status = "Inventory is available when Live (x007f)".into();
        ui.mark_dash_chrome_dirty();
        ui.rebuild();
        return;
    }
    let assets = shared.get_assets_url();
    let token = shared.auth.lock().ok().and_then(|g| g.access_token.clone());
    let prev_scene: Vec<u32> = ui
        .state
        .scene_selected
        .iter()
        .filter_map(|&i| ui.state.scene_meshes.get(i).map(|(eid, _)| *eid))
        .collect();
    ui.state.scene_meshes = shared.scene_meshes();
    // Keep Desktop/XR scene multi-select across Drive Refresh (XR Open stacks Refresh).
    ui.state.scene_selected = rematch_scene_selection(&ui.state.scene_meshes, &prev_scene);
    match token {
        Some(access) => match crate::inventory::list_drive_nodes(&assets, &access) {
            Ok(nodes) => {
                ui.state.inventory_nodes = nodes
                    .into_iter()
                    .map(|n| (n.node_id, n.kind, n.name))
                    .collect();
                ui.state.inventory_selected = None;
                ui.state.inventory_status = format!(
                    "{} drive / {} scene",
                    ui.state.inventory_nodes.len(),
                    ui.state.scene_meshes.len()
                );
            }
            Err(err) => {
                ui.state.inventory_status = format!("refresh failed: {err:#}");
                log::warn!("inventory refresh: {err:#}");
            }
        },
        None => {
            ui.state.inventory_status = "sign in required".into();
        }
    }
    publish_inventory_mirror_from_ui(ui, shared);
    ui.rebuild();
}

/// Inventory tab open: scene list re-sync without Drive HTTP (x006e).
pub(super) fn inventory_open(ui: &mut SystemUi, shared: &SharedState) {
    if ui.state.inventory_blocked() {
        ui.state.tab = DashTab::Home;
        ui.state.home_status = "Inventory is available when Live (x007f)".into();
        ui.mark_dash_chrome_dirty();
        ui.mark_dash_form_dirty();
        ui.rebuild();
        return;
    }
    // Scene list is Manifest-derived; refresh without Drive HTTP (x006e).
    let prev_scene: Vec<u32> = ui
        .state
        .scene_selected
        .iter()
        .filter_map(|&i| ui.state.scene_meshes.get(i).map(|(eid, _)| *eid))
        .collect();
    ui.state.scene_meshes = shared.scene_meshes();
    ui.state.scene_selected = rematch_scene_selection(&ui.state.scene_meshes, &prev_scene);
    publish_inventory_mirror_from_ui(ui, shared);
    ui.rebuild();
}

/// Drive take_out + paced EditSpawn puts on the auth worker (x007f / x0098).
pub(super) fn inventory_take_out(
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    zenoh: &Option<zenoh::Session>,
    auth_job: &mut Option<(u64, Receiver<AuthJobOutcome>)>,
) {
    // Consume XR override immediately so busy/blocked early returns cannot
    // leak it into a later Desktop TakeOut (re-fire from XR to retry).
    let override_target = ui.state.inventory_take_out_override.take();
    if ui.state.inventory_blocked() {
        ui.state.tab = DashTab::Home;
        ui.state.home_status = "Inventory is available when Live (x007f)".into();
        ui.mark_dash_chrome_dirty();
        ui.rebuild();
        return;
    }
    if auth_job.is_some() {
        inventory_status_rebuild(ui, shared, "busy — wait for auth/inventory job");
        return;
    }
    let selected = override_target.or_else(|| {
        ui.state.inventory_selected.and_then(|i| {
            ui.state
                .inventory_nodes
                .get(i)
                .map(|(id, kind, _)| (id.clone(), kind.clone()))
        })
    });
    let Some((node_id, kind)) = selected else {
        inventory_status_rebuild(ui, shared, "select a blob or prefab node first");
        return;
    };
    if kind != "blob" && kind != "prefab" {
        inventory_status_rebuild(ui, shared, "x007f: blob/prefab nodes only");
        return;
    }
    let assets = shared.get_assets_url();
    let room_id_for_gate = shared.get_room_id();
    let catalog_owner = catalog_owner_sub_for_room(ui, room_id_for_gate.as_ref());
    let (access, host_room_token, local_sub) = {
        let Ok(auth) = shared.auth.lock() else {
            inventory_status_rebuild(ui, shared, "auth lock failed");
            return;
        };
        (
            auth.access_token.clone(),
            auth.host_room_token.clone(),
            auth.local_sub.clone(),
        )
    };
    let intent_bind_pending = shared
        .intent_session
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|s| s.bind_pending()));
    if let Some(msg) = inventory_control_gate_status(
        &access,
        intent_bind_pending,
        &host_room_token,
        local_sub.as_deref(),
        catalog_owner.as_deref(),
    ) {
        inventory_status_rebuild(ui, shared, msg);
        return;
    }
    let Some(access) = access else {
        inventory_status_rebuild(ui, shared, "sign in required");
        return;
    };
    // x0098: session-signed; host accepts owner_sub or host-role bind.
    let Some(session) = zenoh.as_ref().cloned() else {
        inventory_status_rebuild(ui, shared, "zenoh not connected");
        return;
    };
    let Some(wire_cap) = shared.wire_cap() else {
        inventory_status_rebuild(ui, shared, "no wire_cap");
        return;
    };
    let Some((room_key, room_key_epoch)) = shared.live_room_aead() else {
        inventory_status_rebuild(ui, shared, "room_key missing");
        return;
    };
    let room_id = shared.get_room_id().to_string();
    let wire_cap = wire_cap.to_string();
    let shared_job = Arc::clone(shared);
    ui.state.inventory_status = "take_out…".into();
    publish_inventory_mirror_from_ui(ui, shared);
    ui.rebuild();
    let (tx, rx) = mpsc::channel();
    let epoch = arm_auth_job(shared, auth_job, rx);
    spawn_auth_http(move || {
        let status = run_paced_inventory_take_out(
            &shared_job,
            epoch,
            &tx,
            &session,
            TakeOutSpawnArgs {
                assets: &assets,
                access: &access,
                room_id: &room_id,
                wire_cap: &wire_cap,
                room_key: room_key.as_ref(),
                room_key_epoch,
                node_id: &node_id,
            },
        );
        if shared_job.auth_work_is_current(epoch) {
            let _ = tx.send(AuthJobOutcome::InventoryTakeOutDone { status });
        }
    });
}

/// Apply inventory worker outcomes (progress keeps job armed; Done clears in poll_auth_job).
pub(super) fn apply_inventory_job_outcome(
    ui: &mut SystemUi,
    shared: &SharedState,
    outcome: AuthJobOutcome,
) {
    match outcome {
        AuthJobOutcome::InventoryTakeOutProgress { status } => {
            ui.state.inventory_status = status;
            publish_inventory_mirror_from_ui(ui, shared);
            ui.rebuild();
        }
        AuthJobOutcome::InventoryTakeOutDone { status } => {
            ui.state.inventory_status = status;
            publish_inventory_mirror_from_ui(ui, shared);
            ui.rebuild();
        }
        other => {
            debug_assert!(
                !other.is_inventory_outcome(),
                "non-inventory outcome passed to inventory_flow"
            );
        }
    }
}

/// Save selected scene meshes to Drive as a prefab; children stay in room (x006e).
pub(super) fn inventory_put_back(ui: &mut SystemUi, shared: &SharedState) {
    if ui.state.inventory_blocked() {
        ui.state.tab = DashTab::Home;
        ui.state.home_status = "Inventory is available when Live (x007f)".into();
        ui.state.inventory_status = "Inventory is available when Live (x007f)".into();
        // Peer/Context Save must not hang on Live reject (x00ce).
        publish_put_back_outcome(ui, shared, false);
        ui.mark_dash_chrome_dirty();
        ui.rebuild();
        return;
    }
    // Always re-sync from SharedState so Put back sees post-spawn Manifest (x006e).
    let prev_entities: Vec<u32> = ui
        .state
        .scene_selected
        .iter()
        .filter_map(|&i| ui.state.scene_meshes.get(i).map(|(eid, _)| *eid))
        .collect();
    ui.state.scene_meshes = shared.scene_meshes();
    ui.state.scene_selected = prev_entities
        .iter()
        .filter_map(|eid| ui.state.scene_meshes.iter().position(|(id, _)| id == eid))
        .collect();
    if ui.state.scene_selected.is_empty() {
        ui.state.inventory_status =
            "select one or more scene meshes first (Refresh if empty)".into();
        publish_put_back_outcome(ui, shared, false);
        ui.rebuild();
        return;
    }
    let entity_ids: Vec<u32> = ui
        .state
        .scene_selected
        .iter()
        .filter_map(|&i| ui.state.scene_meshes.get(i).map(|(eid, _)| *eid))
        .collect();
    run_put_back_for_entities(ui, shared, &entity_ids);
}

/// Signed EditDespawn put for the selected scene mesh (x0098).
pub(super) fn inventory_despawn(
    ui: &mut SystemUi,
    shared: &SharedState,
    zenoh: &Option<zenoh::Session>,
) {
    if ui.state.inventory_blocked() {
        ui.state.tab = DashTab::Home;
        ui.state.home_status = "Inventory is available when Live (x007f)".into();
        ui.mark_dash_chrome_dirty();
        ui.rebuild();
        return;
    }
    let prev_entity = ui
        .state
        .scene_selected
        .first()
        .and_then(|&i| ui.state.scene_meshes.get(i).map(|(eid, _)| *eid));
    ui.state.scene_meshes = shared.scene_meshes();
    if let Some(eid) = prev_entity {
        ui.state.scene_selected = ui
            .state
            .scene_meshes
            .iter()
            .position(|(id, _)| *id == eid)
            .into_iter()
            .collect();
    } else {
        ui.state
            .scene_selected
            .retain(|&i| i < ui.state.scene_meshes.len());
    }
    let selected = ui.state.scene_selected.first().and_then(|&i| {
        ui.state
            .scene_meshes
            .get(i)
            .map(|(eid, b3)| (*eid, b3.clone()))
    });
    let Some((entity_id, blake3)) = selected else {
        inventory_status_rebuild(ui, shared, "select a scene mesh first (Refresh if empty)");
        return;
    };
    let room_id_for_gate = shared.get_room_id();
    let catalog_owner = catalog_owner_sub_for_room(ui, room_id_for_gate.as_ref());
    let (access, host_room_token, local_sub) = {
        let Ok(auth) = shared.auth.lock() else {
            inventory_status_rebuild(ui, shared, "auth lock failed");
            return;
        };
        (
            auth.access_token.clone(),
            auth.host_room_token.clone(),
            auth.local_sub.clone(),
        )
    };
    let intent_bind_pending = shared
        .intent_session
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|s| s.bind_pending()));
    if let Some(msg) = inventory_control_gate_status(
        &access,
        intent_bind_pending,
        &host_room_token,
        local_sub.as_deref(),
        catalog_owner.as_deref(),
    ) {
        inventory_status_rebuild(ui, shared, msg);
        return;
    }
    let Some(session) = zenoh.as_ref() else {
        inventory_status_rebuild(ui, shared, "zenoh not connected");
        return;
    };
    let Some(wire_cap) = shared.wire_cap() else {
        inventory_status_rebuild(ui, shared, "no wire_cap");
        return;
    };
    let Some((room_key, room_key_epoch)) = shared.live_room_aead() else {
        inventory_status_rebuild(ui, shared, "room_key missing");
        return;
    };
    let room_id = shared.get_room_id();
    let proposal = {
        let Ok(mut gate) = shared.intent_session.lock() else {
            inventory_status_rebuild(ui, shared, "intent_session lock failed");
            return;
        };
        let Some(intent) = gate.as_mut() else {
            inventory_status_rebuild(ui, shared, "Intent session required (join/bind first)");
            return;
        };
        let base_scene_rev = shared
            .last_edit_scene_rev
            .lock()
            .ok()
            .and_then(|g| *g)
            .unwrap_or(0);
        intent.sign_edit_despawn(room_id.as_ref(), base_scene_rev, entity_id)
    };
    match crate::inventory::publish_edit_proposal(
        session,
        room_id.as_ref(),
        wire_cap.as_ref(),
        room_key.as_ref(),
        room_key_epoch,
        &proposal,
    ) {
        Ok(()) => {
            ui.state.inventory_status = format!(
                "despawn requested #{entity_id} ({})",
                &blake3[..8.min(blake3.len())]
            );
            ui.state.scene_selected.clear();
        }
        Err(err) => {
            ui.state.inventory_status = format!("despawn put failed: {err:#}");
            log::warn!("inventory despawn: {err:#}");
        }
    }
    publish_inventory_mirror_from_ui(ui, shared);
    ui.rebuild();
}

/// Room Save: signed EditSave put; result arrives via `room_save_status` (x00b7).
pub(super) fn room_save_changes(
    ui: &mut SystemUi,
    shared: &SharedState,
    zenoh: &Option<zenoh::Session>,
) {
    if !ui.state.inventory_live_allowed() {
        ui.state.room_save_status = "join a Live session first".into();
        ui.mark_dash_chrome_dirty();
        ui.rebuild();
        return;
    }
    if ui.state.room_save_busy() {
        return;
    }
    if shared
        .pending_save_op_id
        .lock()
        .ok()
        .and_then(|g| *g)
        .is_some()
    {
        ui.state.room_save_status = "saving…".into();
        ui.mark_dash_chrome_dirty();
        ui.rebuild();
        return;
    }
    let (access, host_room_token) = {
        let Ok(auth) = shared.auth.lock() else {
            ui.state.room_save_status = "auth lock failed".into();
            ui.mark_dash_chrome_dirty();
            ui.rebuild();
            return;
        };
        (auth.access_token.clone(), auth.host_room_token.clone())
    };
    let intent_bind_pending = shared
        .intent_session
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|s| s.bind_pending()));
    if let Some(msg) = room_save_gate_status(&access, intent_bind_pending, &host_room_token) {
        ui.state.room_save_enabled = false;
        ui.state.room_save_status = msg.into();
        ui.mark_dash_chrome_dirty();
        ui.rebuild();
        return;
    }
    let Some(session) = zenoh.as_ref() else {
        ui.state.room_save_status = "zenoh not connected".into();
        ui.mark_dash_chrome_dirty();
        ui.rebuild();
        return;
    };
    let Some(wire_cap) = shared.wire_cap() else {
        ui.state.room_save_status = "no wire_cap".into();
        ui.mark_dash_chrome_dirty();
        ui.rebuild();
        return;
    };
    let Some((room_key, room_key_epoch)) = shared.live_room_aead() else {
        ui.state.room_save_status = "room_key missing".into();
        ui.mark_dash_chrome_dirty();
        ui.rebuild();
        return;
    };
    let room_id = shared.get_room_id();
    let proposal = {
        let Ok(mut gate) = shared.intent_session.lock() else {
            ui.state.room_save_status = "intent_session lock failed".into();
            ui.mark_dash_chrome_dirty();
            ui.rebuild();
            return;
        };
        let Some(intent) = gate.as_mut() else {
            ui.state.room_save_status = "no Intent session (bind first)".into();
            ui.mark_dash_chrome_dirty();
            ui.rebuild();
            return;
        };
        let base_scene_rev = shared
            .last_edit_scene_rev
            .lock()
            .ok()
            .and_then(|g| *g)
            .unwrap_or(0);
        intent.sign_edit_save(room_id.as_ref(), base_scene_rev)
    };
    let op_id = proposal.op_id;
    match crate::inventory::publish_edit_proposal(
        session,
        room_id.as_ref(),
        wire_cap.as_ref(),
        room_key.as_ref(),
        room_key_epoch,
        &proposal,
    ) {
        Ok(()) => {
            if let Ok(mut slot) = shared.pending_save_op_id.lock() {
                *slot = Some(op_id);
            }
            ui.state.room_save_status = "saving…".into();
        }
        Err(err) => {
            ui.state.room_save_status = format!("save put failed: {err:#}");
        }
    }
    ui.mark_dash_chrome_dirty();
    ui.rebuild();
}

/// Put_back terminal: always `Some(ok)` + consume peer correlation after publish (x00ce).
fn publish_put_back_outcome(ui: &mut SystemUi, shared: &SharedState, ok: bool) {
    let correlation = ui.state.inventory_put_back_correlation;
    if shared.publish_inventory_mirror(
        ui.state.inventory_nodes.clone(),
        ui.state.inventory_status.clone(),
        Some(ok),
        correlation,
    ) {
        ui.state.inventory_put_back_correlation = None;
    }
}

fn inventory_status_rebuild(ui: &mut SystemUi, shared: &SharedState, status: impl Into<String>) {
    ui.state.inventory_status = status.into();
    publish_inventory_mirror_from_ui(ui, shared);
    ui.rebuild();
}

/// Shared Inventory Put back / Context Save path (x006e / x00ce).
fn run_put_back_for_entities(ui: &mut SystemUi, shared: &SharedState, entity_ids: &[u32]) {
    let _ = run_put_back_for_entities_result(ui, shared, entity_ids);
}

/// `Ok(status)` on Drive success, `Err(status)` on fail-closed paths (for Context radial UI).
pub(super) fn run_put_back_for_entities_result(
    ui: &mut SystemUi,
    shared: &SharedState,
    entity_ids: &[u32],
) -> Result<String, String> {
    let meshes = shared.scene_meshes();
    let children = {
        let world = shared.world.lock().ok();
        match crate::inventory::put_back_children_for_entities(&meshes, entity_ids, |eid| {
            world.as_ref().and_then(|w| {
                w.entities.get(&eid).map(|p| {
                    (
                        [p.x as f64, p.y as f64, p.z as f64],
                        [p.qx as f64, p.qy as f64, p.qz as f64, p.qw as f64],
                        [p.sx as f64, p.sy as f64, p.sz as f64],
                    )
                })
            })
        }) {
            Ok(c) => c,
            Err(msg) => {
                ui.state.inventory_status = msg.to_string();
                publish_put_back_outcome(ui, shared, false);
                ui.rebuild();
                return Err(msg.to_string());
            }
        }
    };
    if children.is_empty() {
        ui.state.inventory_status = "no put-back children".into();
        publish_put_back_outcome(ui, shared, false);
        ui.rebuild();
        return Err("no put-back children".into());
    }
    let assets = shared.get_assets_url();
    let access = {
        let Ok(auth) = shared.auth.lock() else {
            ui.state.inventory_status = "auth lock failed".into();
            publish_put_back_outcome(ui, shared, false);
            ui.rebuild();
            return Err("auth lock failed".into());
        };
        auth.access_token.clone()
    };
    let Some(access) = access else {
        ui.state.inventory_status = "sign in required".into();
        publish_put_back_outcome(ui, shared, false);
        ui.rebuild();
        return Err("sign in required".into());
    };
    let name = crate::inventory::put_back_auto_name(&children);
    let result =
        match crate::inventory::put_back_prefab_children(&assets, &access, &name, &children) {
            Ok(node) => {
                if let Ok(nodes) = crate::inventory::list_drive_nodes(&assets, &access) {
                    ui.state.inventory_nodes = nodes
                        .into_iter()
                        .map(|n| (n.node_id, n.kind, n.name))
                        .collect();
                }
                ui.state.scene_meshes = shared.scene_meshes();
                Ok(format!(
                    "put-back ok {} ({} children stay in room)",
                    node.name,
                    children.len()
                ))
            }
            Err(err) => {
                log::warn!("inventory put_back: {err:#}");
                Err(format!("put_back failed: {err:#}"))
            }
        };
    let put_back_ok = result.is_ok();
    match &result {
        Ok(status) | Err(status) => {
            ui.state.inventory_status = status.clone();
        }
    }
    publish_put_back_outcome(ui, shared, put_back_ok);
    ui.rebuild();
    result
}

pub(super) fn shared_has_host_inventory_token(shared: &SharedState) -> bool {
    shared
        .auth
        .lock()
        .ok()
        .is_some_and(|g| g.host_room_token.is_some())
}

/// Soft UI gate for Take-out/Despawn (x0098). Final auth is always on the compute host.
///
/// Allows when Intent is live and either:
/// - `host_room_token` looks like `role=host` (compute-host Dual path), or
/// - catalog `owner_sub` for the current room matches `local_sub` (owner Dual path), or
/// - catalog has **no** owner row (`catalog_owner_sub == None`) — **intentionally does not
///   early-reject** (Advanced fixed `ROOM_ID`, catalog not loaded, or row missing). Host Dual
///   remains the authority; non-owner may still reach Drive/Zenoh until host rejects.
///
/// Rejects early only when catalog **positively** says another owner and there is no host
/// Token, so known non-owner guests do not burn Drive take_out + Zenoh puts first.
///
/// `intent_bind_pending`: `None` = no Intent session / lock failed; `Some(true)` = bind
/// still in progress; `Some(false)` = session present and not pending.
fn inventory_control_gate_status(
    access: &Option<String>,
    intent_bind_pending: Option<bool>,
    host_room_token: &Option<String>,
    local_sub: Option<&str>,
    catalog_owner_sub: Option<&str>,
) -> Option<&'static str> {
    match (access.as_ref(), intent_bind_pending) {
        (None, _) => return Some("sign in required"),
        (Some(_), None) => return Some("Intent session required (join/bind first)"),
        (Some(_), Some(true)) => return Some("Intent bind in progress — wait a moment"),
        _ => {}
    }
    let host_role = host_room_token.as_deref().is_some_and(|t| {
        crate::inventory::room_token_role_unverified(t).as_deref() == Some("host")
    });
    if host_role {
        return None;
    }
    match (local_sub, catalog_owner_sub) {
        (Some(local), Some(owner)) if local == owner => None,
        // Catalog miss / Advanced ROOM_ID: do not early-reject (host Dual decides).
        (_, None) => None,
        _ => Some("owner or host role required for take-out/despawn"),
    }
}

/// Soft Save gate (x00b7): Live + Intent + **host Room Token**.
///
/// World still requires `active_host ∩ scene_writer` on the host HTTP sub — soft UI
/// cannot see `editor_subs`, so a non-writer compute host may still get `SAVE_FORBIDDEN`.
fn room_save_gate_status(
    access: &Option<String>,
    intent_bind_pending: Option<bool>,
    host_room_token: &Option<String>,
) -> Option<&'static str> {
    match (access.as_ref(), intent_bind_pending) {
        (None, _) => return Some("sign in required"),
        (Some(_), None) => return Some("Intent session required (join/bind first)"),
        (Some(_), Some(true)) => return Some("Intent bind in progress — wait a moment"),
        _ => {}
    }
    let host_role = host_room_token.as_deref().is_some_and(|t| {
        crate::inventory::room_token_role_unverified(t).as_deref() == Some("host")
    });
    if host_role {
        None
    } else {
        Some("active host Token required to Save (scene writer enforced by world)")
    }
}

/// Soft Save enable + consume `EditCommit` status from SharedState (x00b7).
/// Marks chrome dirty so XR echoes enable/status via SharedDashLogic.
pub(super) fn sync_room_save_ui(ui: &mut SystemUi, shared: &SharedState) -> bool {
    let mut dirty = false;
    let (access, host_room_token) = match shared.auth.lock() {
        Ok(auth) => (auth.access_token.clone(), auth.host_room_token.clone()),
        Err(_) => (None, None),
    };
    let intent_bind_pending = shared
        .intent_session
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|s| s.bind_pending()));
    // Live gate matches Inventory / XR (`inventory_live_allowed`), not bare `worlds_in_room`.
    let enabled = ui.state.inventory_live_allowed()
        && room_save_gate_status(&access, intent_bind_pending, &host_room_token).is_none();
    if ui.state.room_save_enabled != enabled {
        ui.state.room_save_enabled = enabled;
        dirty = true;
    }
    if let Ok(mut slot) = shared.room_save_status.lock() {
        if let Some(status) = slot.take() {
            if ui.state.room_save_status != status {
                ui.state.room_save_status = status;
                dirty = true;
            }
        }
    }
    if dirty {
        ui.mark_dash_chrome_dirty();
    }
    dirty
}

/// Host `SpawnRate(8/10s)` — client spaces puts so host does not silently reject.
const HOST_SPAWN_MAX_PER_WINDOW: u32 = 8;
const HOST_SPAWN_WINDOW_MS: u64 = 10_000;

/// Wire / Drive args for paced take-out (keeps `run_paced_inventory_take_out` under clippy arity).
struct TakeOutSpawnArgs<'a> {
    assets: &'a str,
    access: &'a str,
    room_id: &'a str,
    wire_cap: &'a str,
    room_key: &'a [u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN],
    room_key_epoch: u64,
    node_id: &'a str,
}

/// HTTP take_out + paced Zenoh puts off the UI thread (x007f).
fn run_paced_inventory_take_out(
    shared: &SharedState,
    epoch: u64,
    tx: &std::sync::mpsc::Sender<AuthJobOutcome>,
    session: &zenoh::Session,
    args: TakeOutSpawnArgs<'_>,
) -> String {
    let TakeOutSpawnArgs {
        assets,
        access,
        room_id,
        wire_cap,
        room_key,
        room_key_epoch,
        node_id,
    } = args;
    let (blobs, spawn_ticket) =
        match crate::inventory::take_out_blobs(assets, access, node_id, Some(room_id)) {
            Ok((_node, blobs, ticket)) => {
                let Some(ticket) = ticket.filter(|t| !t.is_empty()) else {
                    return "take_out missing spawn_ticket".into();
                };
                (blobs, ticket)
            }
            Err(err) => {
                log::warn!("inventory take_out: {err:#}");
                return format!("take_out failed: {err:#}");
            }
        };
    let total = blobs.len();
    if total == 0 {
        return "take_out returned 0 blobs".into();
    }
    let gap_ms = HOST_SPAWN_WINDOW_MS / u64::from(HOST_SPAWN_MAX_PER_WINDOW);
    let gap = std::time::Duration::from_millis(gap_ms);
    let mut spawned = 0usize;
    for (i, blob) in blobs.iter().enumerate() {
        if !shared.auth_work_is_current(epoch) {
            return format!("spawn put aborted {spawned}/{total} (left/logout)");
        }
        if i > 0 {
            std::thread::sleep(gap);
            if !shared.auth_work_is_current(epoch) {
                return format!("spawn put aborted {spawned}/{total} (left/logout)");
            }
        }
        let (x, y, z, qx, qy, qz, qw, sx, sy, sz) = if let Some(trs) = blob.trs.as_ref() {
            (
                trs.translation[0],
                trs.translation[1],
                trs.translation[2],
                trs.rotation[0],
                trs.rotation[1],
                trs.rotation[2],
                trs.rotation[3],
                trs.scale[0],
                trs.scale[1],
                trs.scale[2],
            )
        } else {
            // Default pose with slight X offset so children do not stack.
            (i as f64 * 0.35, 1.5, 2.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0)
        };
        let proposal = {
            let Ok(mut gate) = shared.intent_session.lock() else {
                return format!("spawn put {spawned}/{total} then failed: intent_session lock");
            };
            let Some(intent) = gate.as_mut() else {
                return format!(
                    "spawn put {spawned}/{total} then failed: no Intent session (bind first)"
                );
            };
            // Overlay EditCommit watermark (not Manifest revision).
            let base_scene_rev = shared
                .last_edit_scene_rev
                .lock()
                .ok()
                .and_then(|g| *g)
                .unwrap_or(0);
            intent.sign_edit_spawn(
                room_id,
                base_scene_rev,
                &blob.blake3,
                [x, y, z, qx, qy, qz, qw, sx, sy, sz],
                &spawn_ticket,
            )
        };
        match crate::inventory::publish_edit_proposal(
            session,
            room_id,
            wire_cap,
            room_key,
            room_key_epoch,
            &proposal,
        ) {
            Ok(()) => {
                spawned += 1;
                if shared.auth_work_is_current(epoch) {
                    let _ = tx.send(AuthJobOutcome::InventoryTakeOutProgress {
                        status: format!(
                            "spawn put {spawned}/{total} (paced ≤{HOST_SPAWN_MAX_PER_WINDOW}/{HOST_SPAWN_WINDOW_MS}ms)"
                        ),
                    });
                }
            }
            Err(err) => {
                // Leave/close often races the put; prefer abort copy when epoch already bumped.
                if !shared.auth_work_is_current(epoch) {
                    return format!("spawn put aborted {spawned}/{total} (left/logout)");
                }
                log::warn!("inventory spawn: {err:#}");
                return format!("spawn put {spawned}/{total} then failed: {err:#}");
            }
        }
    }
    format!(
        "spawn put {spawned}/{total} (paced ≤{HOST_SPAWN_MAX_PER_WINDOW}/{HOST_SPAWN_WINDOW_MS}ms; host may still reject)"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn access() -> Option<String> {
        Some("access".into())
    }

    /// Minimal JWT-shaped token whose unverified payload has `role`.
    fn role_token(role: &str) -> String {
        use base64::Engine;
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(format!(r#"{{"role":"{role}"}}"#));
        format!("hdr.{payload}.sig")
    }

    #[test]
    fn inventory_gate_allows_host_role_token() {
        let host = Some(role_token("host"));
        assert!(inventory_control_gate_status(
            &access(),
            Some(false),
            &host,
            Some("guest-sub"),
            Some("other-owner"),
        )
        .is_none());
    }

    #[test]
    fn inventory_gate_allows_catalog_owner_match() {
        assert!(inventory_control_gate_status(
            &access(),
            Some(false),
            &None,
            Some("owner-a"),
            Some("owner-a"),
        )
        .is_none());
    }

    #[test]
    fn inventory_gate_rejects_other_catalog_owner_without_host_token() {
        let msg = inventory_control_gate_status(
            &access(),
            Some(false),
            &None,
            Some("guest-sub"),
            Some("owner-a"),
        );
        assert_eq!(
            msg,
            Some("owner or host role required for take-out/despawn")
        );
    }

    #[test]
    fn inventory_gate_unknown_catalog_passes_to_host() {
        // Advanced ROOM_ID / catalog miss: early gate must not false-negative.
        assert!(
            inventory_control_gate_status(&access(), Some(false), &None, Some("anyone"), None,)
                .is_none()
        );
    }
}
