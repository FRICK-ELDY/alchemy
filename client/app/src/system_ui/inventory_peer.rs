//! XR → Desktop Inventory peer channel (x007f).
//!
//! Guarantees: selection / ids travel with the request so Desktop need not share
//! per-surface `inventory_selected` / `scene_selected`; Desktop publishes Drive
//! list + status via [`InventoryMirror`] for XR display.
//! Non-guarantees: HTTP / Zenoh (Desktop surface owns those); host spawn ack.

/// XR-authored Inventory op with the ids XR selected (Desktop executes).
#[derive(Clone, Debug)]
pub enum InventoryPeerRequest {
    /// Open Inventory chrome + sync scene list (no Drive HTTP).
    Open,
    /// Reload Drive nodes on Desktop; result lands in [`InventoryMirror`].
    Refresh,
    TakeOut {
        node_id: String,
        kind: String,
    },
    PutBack {
        entity_ids: Vec<u32>,
    },
    Despawn {
        entity_id: u32,
    },
}

/// Desktop → XR snapshot of Drive rows + status (scene meshes stay on SharedState).
#[derive(Clone, Debug, Default)]
pub struct InventoryMirror {
    pub revision: u64,
    pub nodes: Vec<(String, String, String)>,
    pub status: String,
}

/// Re-resolve Drive selection by `node_id` after list replace (not by stale index).
pub fn rematch_inventory_selection(
    nodes: &[(String, String, String)],
    prev_node_id: Option<&str>,
) -> Option<usize> {
    let id = prev_node_id?;
    nodes.iter().position(|(nid, _, _)| nid == id)
}

/// Re-resolve scene multi-select by entity id after Manifest refresh.
pub fn rematch_scene_selection(meshes: &[(u32, String)], entity_ids: &[u32]) -> Vec<usize> {
    entity_ids
        .iter()
        .filter_map(|eid| meshes.iter().position(|(id, _)| id == eid))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rematch_inventory_keeps_node_id_across_reorder() {
        let prev = Some("b");
        let nodes = vec![
            ("a".into(), "blob".into(), "A".into()),
            ("b".into(), "prefab".into(), "B".into()),
        ];
        assert_eq!(rematch_inventory_selection(&nodes, prev), Some(1));
        let reordered = vec![
            ("b".into(), "prefab".into(), "B".into()),
            ("a".into(), "blob".into(), "A".into()),
        ];
        assert_eq!(rematch_inventory_selection(&reordered, prev), Some(0));
    }

    #[test]
    fn rematch_inventory_clears_missing_node() {
        let nodes = vec![("a".into(), "blob".into(), "A".into())];
        assert_eq!(rematch_inventory_selection(&nodes, Some("gone")), None);
    }

    #[test]
    fn rematch_scene_keeps_entity_ids() {
        let meshes = vec![(10, "aa".into()), (20, "bb".into()), (30, "cc".into())];
        assert_eq!(rematch_scene_selection(&meshes, &[20, 10]), vec![1, 0]);
        assert_eq!(rematch_scene_selection(&meshes, &[99, 30]), vec![2]);
    }
}
