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

/// Immutable Prev/Curr pair published atomically (one ArcSwap store).
///
/// `published_at_ns` is an optional publisher clock (0 = unset). Client mirror
/// sets process-monotonic ns so α and faces stay same-generation; host may leave 0.
#[derive(Clone, Debug)]
pub struct EpochFaces {
    pub prev: Arc<EpochSheet>,
    pub curr: Arc<EpochSheet>,
    pub published_at_ns: u64,
}

/// Shared Prev/Curr faces for lock-free render readers.
///
/// Guarantees: [`Self::publish`] / [`Self::publish_arcs`] replace both faces (and
/// optional clock) in **one** store so readers never observe a torn generation.
#[derive(Debug)]
pub struct EpochSnapshotPair {
    faces: ArcSwap<EpochFaces>,
}

impl EpochSnapshotPair {
    pub fn new(prev: EpochSheet, curr: EpochSheet) -> Self {
        Self {
            faces: ArcSwap::from_pointee(EpochFaces {
                prev: Arc::new(prev),
                curr: Arc::new(curr),
                published_at_ns: 0,
            }),
        }
    }

    pub fn publish(&self, prev: EpochSheet, curr: EpochSheet) {
        self.publish_arcs(Arc::new(prev), Arc::new(curr), 0);
    }

    /// Publish existing Arcs (pointer share; callers own clone policy).
    pub fn publish_arcs(&self, prev: Arc<EpochSheet>, curr: Arc<EpochSheet>, published_at_ns: u64) {
        self.faces.store(Arc::new(EpochFaces {
            prev,
            curr,
            published_at_ns,
        }));
    }

    /// Load Prev and Curr from the same published generation.
    pub fn load_pair(&self) -> Arc<EpochFaces> {
        self.faces.load_full()
    }

    pub fn load_curr(&self) -> Arc<EpochSheet> {
        Arc::clone(&self.load_pair().curr)
    }

    pub fn load_prev(&self) -> Arc<EpochSheet> {
        Arc::clone(&self.load_pair().prev)
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
    ///
    /// Pass `bounds: None` to skip LBVH/contact rebuild (predict/solve only). Host
    /// kinematic shadow should prefer [`Self::tick_mirror_publish`] instead.
    pub fn tick(
        &mut self,
        world: &mut XpbdWorld,
        topo: &mut FlatTopology,
        snapshots: &EpochSnapshotPair,
        grab_targets: &[(u32, [f64; 3])],
        bounds: Option<SceneBounds>,
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
        xpbd::step(world, topo, &[], grab_targets, bounds, dt);

        // Ring3 lerp needs distinct faces — publish before velocity commit.
        // Dirty contract: consumers (`extract_dirty_deltas`, GPU dirty pack) must read the
        // *published* Arc (`load_curr()`), not live `world.curr` after this point — we clear
        // live dirty so the next tick stays sparse. Published clones keep this tick's bits.
        snapshots.publish(world.prev.clone(), world.curr.clone());
        world.curr.clear_dirty();
        world.commit_prev_from_curr();
        Ok(errors)
    }

    /// Kinematic / Rapier-shadow barrier: apply patches → (optional reparent) →
    /// publish Prev/Curr → commit. **No** XPBD step, LBVH, or contact collect.
    ///
    /// Skips publish+clone when there were no patches, no reparent, and
    /// `force_publish` is false (faces unchanged). Pass `force_publish` after
    /// direct tape membership edits (alloc/tombstone) that bypass the patch queue.
    ///
    /// Returns `(patch_errors, published)`.
    pub fn tick_mirror_publish(
        &mut self,
        world: &mut XpbdWorld,
        topo: &mut FlatTopology,
        snapshots: &EpochSnapshotPair,
        force_publish: bool,
    ) -> Result<(Vec<(usize, TapeError)>, bool), BridgeError> {
        let patches = std::mem::take(&mut self.pending);
        let had_patches = !patches.is_empty();
        let errors = world.curr.apply_patches(&patches);

        let mut did_reparent = false;
        if self.parents_dirty {
            self.apply_reparent(topo)?;
            self.parents_dirty = false;
            did_reparent = true;
        }

        xpbd::sync_prev_capacity(world);
        let published = had_patches || did_reparent || force_publish;
        if published {
            snapshots.publish(world.prev.clone(), world.curr.clone());
            world.curr.clear_dirty();
            world.commit_prev_from_curr();
        }
        Ok((errors, published))
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
                Some(SceneBounds::default()),
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
                Some(SceneBounds::default()),
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
                Some(SceneBounds::default()),
                1.0 / 60.0,
            )
            .unwrap();
        assert_eq!(world.curr.iter_dirty_rows().count(), 0);
    }

    #[test]
    fn tick_mirror_publish_skips_when_idle() {
        let mut world = XpbdWorld::new(1);
        let h = world.curr.alloc_row();
        world
            .curr
            .set_transform(h.row_id, WorldTransformf64::identity_at([1.0, 2.0, 3.0]))
            .unwrap();
        xpbd::sync_prev_capacity(&mut world);
        world.prev = world.curr.clone();
        let snaps = EpochSnapshotPair::new(world.prev.clone(), world.curr.clone());
        let published_ptr = snaps.load_curr().as_ref() as *const _;
        let mut bridge = FrameBridge::default();
        let mut topo = FlatTopology::default();
        let (_errs, published) = bridge
            .tick_mirror_publish(&mut world, &mut topo, &snaps, false)
            .unwrap();
        assert!(!published);
        // No patches → no re-publish (Arc identity retained).
        assert_eq!(snaps.load_curr().as_ref() as *const _, published_ptr);
    }

    #[test]
    fn tick_mirror_publish_force_without_patches() {
        let mut world = XpbdWorld::new(1);
        let h = world.curr.alloc_row();
        world
            .curr
            .set_transform(h.row_id, WorldTransformf64::identity_at([1.0, 0.0, 0.0]))
            .unwrap();
        xpbd::sync_prev_capacity(&mut world);
        world.prev = world.curr.clone();
        let snaps = EpochSnapshotPair::new(EpochSheet::default(), EpochSheet::default());
        let mut bridge = FrameBridge::default();
        let mut topo = FlatTopology::default();
        let (_errs, published) = bridge
            .tick_mirror_publish(&mut world, &mut topo, &snaps, true)
            .unwrap();
        assert!(published);
        assert_eq!(
            snaps.load_curr().world_transforms[h.row_id as usize].position,
            [1.0, 0.0, 0.0]
        );
    }

    #[test]
    fn tick_mirror_publish_keeps_prev_curr_distinct() {
        let mut world = XpbdWorld::new(1);
        let h = world.curr.alloc_row();
        world
            .curr
            .set_transform(h.row_id, WorldTransformf64::identity_at([0.0, 0.0, 0.0]))
            .unwrap();
        xpbd::sync_prev_capacity(&mut world);
        world.prev = world.curr.clone();
        let snaps = EpochSnapshotPair::new(world.prev.clone(), world.curr.clone());
        let mut bridge = FrameBridge::default();
        bridge.queue_patch(PendingPatch::SetTransform {
            row_id: h.row_id,
            transform: WorldTransformf64::identity_at([5.0, 0.0, 0.0]),
            preserve_physics_coords: false,
        });
        let mut topo = FlatTopology::default();
        let (_errs, published) = bridge
            .tick_mirror_publish(&mut world, &mut topo, &snaps, false)
            .unwrap();
        assert!(published);
        assert_eq!(
            snaps.load_prev().world_transforms[h.row_id as usize].position,
            [0.0, 0.0, 0.0]
        );
        assert_eq!(
            snaps.load_curr().world_transforms[h.row_id as usize].position,
            [5.0, 0.0, 0.0]
        );
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
