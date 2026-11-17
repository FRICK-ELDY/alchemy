//! Shared leaf helpers for desktop flows (x00d0).
//!
//! Guarantees: no imports from `session_flow` / `inventory_flow` / `hotswitch_flow` /
//! `job` — this module is the dependency sink that breaks flow↔job triangles.
//! Non-guarantee: business orchestration stays in the flow modules.

use crate::scene::SharedState;
use crate::system_ui::SystemUi;

/// Even when Idle: sets cancel_spawn so an in-flight ensure_running aborts.
pub(super) fn stop_supervised_host(shared: &SharedState) {
    shared.host_supervisor.shutdown_graceful();
}

/// Publish Drive list + status to the XR inventory mirror (no put-back correlation).
pub(super) fn publish_inventory_mirror_from_ui(ui: &SystemUi, shared: &SharedState) {
    shared.publish_inventory_mirror(
        ui.state.inventory_nodes.clone(),
        ui.state.inventory_status.clone(),
        None,
        None,
    );
}

/// Catalog `owner_sub` for the given room id (empty → `None`).
pub(super) fn catalog_owner_sub_for_room(ui: &SystemUi, room_id: &str) -> Option<String> {
    ui.state
        .worlds_rooms
        .iter()
        .find(|(id, ..)| id == room_id)
        .map(|(_, _, _, owner)| owner.clone())
        .filter(|s| !s.is_empty())
}
