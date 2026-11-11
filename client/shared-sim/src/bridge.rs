//! Frame-boundary patch merge + lock-free EpochSheet snapshot publish.
//!
//! Guarantees: Ring1 patches merge without clobbering DYNAMIC physics coords when
//! flagged; topology rebuild only on parent changes; `arc-swap` publishes immutable
//! snapshots to Ring3 readers.
//!
//! Prev/Curr contract for Ring3: publish **start-of-tick Prev** and **end-of-tick Curr**
//! (distinct faces for α-lerp), then commit Prev←Curr for the next tick's velocity.
//! Non-guarantees: Loro CRDT / Hydration; Zenoh publish.
//!
//! Safety: `#![forbid(unsafe_code)]`.

#![forbid(unsafe_code)]

use std::sync::Arc;

use arc_swap::ArcSwap;

use crate::spatial::SceneBounds;
use crate::tape::{EpochSheet, PendingPatch, TapeError};
use crate::topology::{FlatTopology, TopologyError};
use crate::xpbd::{self, XpbdWorld};

/// Shared Prev/Curr faces for lock-free render readers.
#[derive(Debug)]
pub struct EpochSnapshotPair {
    pub prev: ArcSwap<EpochSheet>,
    pub curr: ArcSwap<EpochSheet>,
}

impl EpochSnapshotPair {
    pub fn new(prev: EpochSheet, curr: EpochSheet) -> Self {
        Self {
            prev: ArcSwap::from_pointee(prev),
            curr: ArcSwap::from_pointee(curr),
        }
    }

    pub fn publish(&self, prev: EpochSheet, curr: EpochSheet) {
        self.prev.store(Arc::new(prev));
        self.curr.store(Arc::new(curr));
    }

    pub fn load_curr(&self) -> Arc<EpochSheet> {
        self.curr.load_full()
    }

    pub fn load_prev(&self) -> Arc<EpochSheet> {
        self.prev.load_full()
    }
}

#[derive(Clone, Debug, Default)]
pub struct FrameBridge {
    pub pending: Vec<PendingPatch>,
    pub parents_dirty: bool,
    /// Optional parent_of override queued from Ring1.
    pub pending_parents: Option<Vec<u32>>,
}

#[derive(Debug)]
pub enum BridgeError {
    Tape(TapeError),
    Topology(TopologyError),
}

impl FrameBridge {
    pub fn queue_patch(&mut self, patch: PendingPatch) {
        self.pending.push(patch);
    }

    pub fn queue_reparent(&mut self, parent_of: Vec<u32>) {
        self.pending_parents = Some(parent_of);
        self.parents_dirty = true;
    }

    /// Frame barrier: apply patches → rebuild topology if needed → XPBD tick →
    /// publish (pre-commit Prev, post-step Curr) → commit Prev←Curr.
    pub fn tick(
        &mut self,
        world: &mut XpbdWorld,
        topo: &mut FlatTopology,
        snapshots: &EpochSnapshotPair,
        grab_targets: &[(u32, [f64; 3])],
        bounds: SceneBounds,
        dt: f64,
    ) -> Result<Vec<(usize, TapeError)>, BridgeError> {
        let patches = std::mem::take(&mut self.pending);
        let errors = world.curr.apply_patches(&patches);

        if self.parents_dirty {
            self.apply_reparent(topo)?;
            self.parents_dirty = false;
        }

        xpbd::sync_prev_capacity(world);
        // Prev still holds start-of-tick; Curr is mutated in place.
        xpbd::step(world, topo, &[], grab_targets, Some(bounds), dt);

        // Ring3 lerp needs distinct faces — publish before velocity commit.
        // Dirty contract: consumers (`extract_dirty_deltas`, GPU dirty pack) must read the
        // *published* Arc (`load_curr()`), not live `world.curr` after this point — we clear
        // live dirty so the next tick stays sparse. Published clones keep this tick's bits.
        snapshots.publish(world.prev.clone(), world.curr.clone());
        world.curr.clear_dirty();
        world.commit_prev_from_curr();
        Ok(errors)
    }

    /// Apply pending parents only if bake succeeds; on failure restore prior topology.
    pub(crate) fn apply_reparent(&mut self, topo: &mut FlatTopology) -> Result<(), BridgeError> {
        let Some(parents) = self.pending_parents.take() else {
            return topo.rebuild().map_err(BridgeError::Topology);
        };
        let backup = topo.clone();
        topo.parent_of = parents.clone();
        if let Err(e) = topo.rebuild() {
            *topo = backup;
            self.pending_parents = Some(parents);
            return Err(BridgeError::Topology(e));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::{ReadinessMask, WorldTransformf64};
    use crate::xpbd::XpbdWorld;

    #[test]
    fn patch_merge_preserves_dynamic_physics_coords() {
        let mut world = XpbdWorld::new(1);
        let h = world.curr.alloc_row();
        world
            .curr
            .set_transform(h.row_id, WorldTransformf64::identity_at([1.0, 2.0, 3.0]))
            .unwrap();
        world.curr.readiness[h.row_id as usize] =
            ReadinessMask(ReadinessMask::ALLOCATED | ReadinessMask::DYNAMIC);
        xpbd::sync_prev_capacity(&mut world);
        world.prev = world.curr.clone();

        let mut bridge = FrameBridge::default();
        bridge.queue_patch(PendingPatch::SetTransform {
            row_id: h.row_id,
            transform: WorldTransformf64::identity_at([9.0, 9.0, 9.0]),
            preserve_physics_coords: true,
        });
        let snaps = EpochSnapshotPair::new(world.prev.clone(), world.curr.clone());
        let mut topo = FlatTopology::default();
        world.gravity = [0.0, 0.0, 0.0];
        let errs = bridge
            .tick(
                &mut world,
                &mut topo,
                &snaps,
                &[],
                SceneBounds::default(),
                1.0 / 60.0,
            )
            .unwrap();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].1, TapeError::PhysicsCoordsPreserved);
        assert_eq!(
            world.curr.world_transforms[h.row_id as usize].position,
            [1.0, 2.0, 3.0]
        );
        assert_ne!(
            snaps.load_curr().world_transforms[h.row_id as usize].position,
            [9.0, 9.0, 9.0]
        );
    }

    #[test]
    fn publish_keeps_prev_curr_distinct_for_lerp() {
        let mut world = XpbdWorld::new(1);
        let h = world.curr.alloc_row();
        world
            .curr
            .set_transform(h.row_id, WorldTransformf64::identity_at([0.0, 10.0, 0.0]))
            .unwrap();
        world.curr.readiness[h.row_id as usize] =
            ReadinessMask(ReadinessMask::ALLOCATED | ReadinessMask::DYNAMIC);
        world.curr.world_transforms[h.row_id as usize].inv_mass = 1.0;
        xpbd::sync_prev_capacity(&mut world);
        world.prev = world.curr.clone();

        let snaps = EpochSnapshotPair::new(world.prev.clone(), world.curr.clone());
        let mut bridge = FrameBridge::default();
        let mut topo = FlatTopology::default();
        bridge
            .tick(
                &mut world,
                &mut topo,
                &snaps,
                &[],
                SceneBounds::default(),
                1.0 / 60.0,
            )
            .unwrap();

        let pub_prev = snaps.load_prev().world_transforms[h.row_id as usize].position;
        let pub_curr = snaps.load_curr().world_transforms[h.row_id as usize].position;
        assert_eq!(pub_prev, [0.0, 10.0, 0.0]);
        assert!(
            pub_curr[1] < pub_prev[1],
            "curr should have fallen under gravity; prev={pub_prev:?} curr={pub_curr:?}"
        );
        // After commit, sim prev matches end-of-tick curr for next velocity step.
        assert_eq!(
            world.prev.world_transforms[h.row_id as usize].position,
            world.curr.world_transforms[h.row_id as usize].position
        );
    }

    #[test]
    fn failed_reparent_restores_topology() {
        let mut bridge = FrameBridge::default();
        let mut topo = FlatTopology::from_parents(vec![u32::MAX, 0]).unwrap();
        bridge.queue_reparent(vec![1, 0]); // cycle
        let err = bridge.apply_reparent(&mut topo);
        assert!(err.is_err());
        assert_eq!(topo.parent_of, vec![u32::MAX, 0]);
        assert_eq!(bridge.pending_parents, Some(vec![1, 0]));
    }

    #[test]
    fn tick_clears_dirty_after_publish() {
        let mut world = XpbdWorld::new(1);
        let h = world.curr.alloc_row();
        world.curr.readiness[h.row_id as usize] =
            ReadinessMask(ReadinessMask::ALLOCATED | ReadinessMask::DYNAMIC);
        world.curr.world_transforms[h.row_id as usize].inv_mass = 1.0;
        xpbd::sync_prev_capacity(&mut world);
        world.prev = world.curr.clone();
        world.gravity = [0.0, 0.0, 0.0];
        let snaps = EpochSnapshotPair::new(world.prev.clone(), world.curr.clone());
        let mut bridge = FrameBridge::default();
        let mut topo = FlatTopology::default();
        bridge
            .tick(
                &mut world,
                &mut topo,
                &snaps,
                &[],
                SceneBounds::default(),
                1.0 / 60.0,
            )
            .unwrap();
        assert_eq!(world.curr.iter_dirty_rows().count(), 0);
    }

    #[test]
    fn arc_swap_readers_see_published_face() {
        let snaps = EpochSnapshotPair::new(EpochSheet::default(), EpochSheet::default());
        let mut sheet = EpochSheet::with_capacity(1);
        let h = sheet.alloc_row();
        sheet
            .set_transform(h.row_id, WorldTransformf64::identity_at([4.0, 5.0, 6.0]))
            .unwrap();
        snaps.publish(EpochSheet::default(), sheet);
        let loaded = snaps.load_curr();
        assert_eq!(loaded.world_transforms[0].position, [4.0, 5.0, 6.0]);
    }
}
