//! Late Join RoomSnapshot skeleton (x00e5 枠).
//!
//! Guarantees: data shape for Scene Store full dump + optional execution-face
//! poses; [`RoomSnapshot::from_scene_store`] exports sparse parents/props with
//! **`nodes` sorted by `entity_id` ascending** and props by `prop_key`
//! (deterministic compare / hash input for Verifier / Late Join).
//! Non-guarantees: Zenoh `…/snapshot` / `…/sim_state` wire replacement;
//! PROTOCOL_PIN carriage; Verifier Spot-Check heuristics;
//! [`TrashStore`] TTL / FIFO GC metadata (export is Scene Store rows only —
//! trashed parents appear as `Some(TRASH_ENTITY_ID)`, but lease-tick / queue
//! order is not restored from this snapshot).

use super::{EntityId, PropValue, SceneStore};

/// One sparse Scene Store row in a [`RoomSnapshot`].
#[derive(Clone, Debug, PartialEq)]
pub struct RoomSnapshotNode {
    pub entity_id: EntityId,
    pub parent: Option<EntityId>,
    /// Property bag sorted by `prop_key` ascending (deterministic export).
    pub props: Vec<(u32, PropValue)>,
}

/// Execution-face pose placeholder (not `…/sim_state` wire).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoomSnapshotPose {
    pub entity_id: EntityId,
    pub pos: [f32; 3],
    pub rot: [f32; 4],
    pub scale: [f32; 3],
}

/// Late Join / handover skeleton: Scene Store full dump + optional exec poses.
///
/// Current production Late Join remains RenderFrame `…/snapshot`; planned
/// handover speed restore remains `…/sim_state`. This type does not weaken
/// those contracts.
#[derive(Clone, Debug, PartialEq)]
pub struct RoomSnapshot {
    pub host_epoch: u64,
    pub room_seq: u64,
    pub op_chain_hash: [u8; 32],
    /// Sparse rows sorted by [`RoomSnapshotNode::entity_id`] ascending.
    pub nodes: Vec<RoomSnapshotNode>,
    /// Caller-supplied execution poses (default empty from [`Self::from_scene_store`]).
    ///
    /// **Contract:** when non-empty, must be sorted by [`RoomSnapshotPose::entity_id`]
    /// ascending (same as `nodes`) so Verifier / Late Join compare/hash is stable.
    pub exec_poses: Vec<RoomSnapshotPose>,
}

impl RoomSnapshot {
    /// Export every sparse row (including trashed) with parent + props.
    ///
    /// Order is **`entity_id` ascending** (not SparseSet dense / swap-remove order).
    /// Does **not** include [`super::TrashStore`] TTL/FIFO metadata.
    /// `exec_poses` is left empty — host / Arena attach denser state later.
    pub fn from_scene_store(
        store: &SceneStore,
        host_epoch: u64,
        room_seq: u64,
        op_chain_hash: [u8; 32],
    ) -> Self {
        let mut nodes = Vec::with_capacity(store.len());
        for &entity_id in store.entity_ids() {
            nodes.push(RoomSnapshotNode {
                entity_id,
                parent: store.parent(entity_id),
                props: store.properties(entity_id),
            });
        }
        nodes.sort_by_key(|n| n.entity_id);
        Self {
            host_epoch,
            room_seq,
            op_chain_hash,
            nodes,
            exec_poses: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_store::{ApplyOutcome, CrdtOp, OpTimestamp};

    #[test]
    fn from_scene_store_exports_parents_and_props() {
        let mut store = SceneStore::new();
        assert!(store.seed_node(1, None));
        assert!(store.seed_node(2, Some(1)));
        let outcome = store.apply(CrdtOp::SetProperty {
            ts: OpTimestamp {
                lamport: 1,
                actor_id: 1,
            },
            node: 2,
            prop_key: 7,
            value: PropValue::U32(42),
        });
        assert_eq!(outcome, ApplyOutcome::Applied);

        let snap = RoomSnapshot::from_scene_store(&store, 3, 11, [9u8; 32]);
        assert_eq!(snap.host_epoch, 3);
        assert_eq!(snap.room_seq, 11);
        assert_eq!(snap.op_chain_hash, [9u8; 32]);
        assert!(snap.exec_poses.is_empty());
        assert_eq!(snap.nodes.len(), 2);

        let n1 = snap.nodes.iter().find(|n| n.entity_id == 1).unwrap();
        assert_eq!(n1.parent, None);
        assert!(n1.props.is_empty());

        let n2 = snap.nodes.iter().find(|n| n.entity_id == 2).unwrap();
        assert_eq!(n2.parent, Some(1));
        assert_eq!(n2.props, vec![(7, PropValue::U32(42))]);
    }

    #[test]
    fn from_scene_store_sorts_nodes_by_entity_id() {
        let mut store = SceneStore::new();
        // Insert out of id order so dense walk ≠ entity_id order.
        assert!(store.seed_node(5, None));
        assert!(store.seed_node(1, None));
        assert!(store.seed_node(3, Some(1)));
        let dense: Vec<_> = store.entity_ids().to_vec();
        assert_ne!(dense, vec![1, 3, 5], "precondition: dense ≠ sorted");

        let snap = RoomSnapshot::from_scene_store(&store, 0, 0, [0u8; 32]);
        let ids: Vec<_> = snap.nodes.iter().map(|n| n.entity_id).collect();
        assert_eq!(ids, vec![1, 3, 5]);
    }
}
