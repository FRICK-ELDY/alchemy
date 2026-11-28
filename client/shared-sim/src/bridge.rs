//! Frame-boundary patch merge + lock-free EpochSheet snapshot publish.
//!
//! Guarantees: Ring1 patches merge without clobbering DYNAMIC physics coords when
//! flagged; topology rebuild only on parent changes; `arc-swap` publishes immutable
//! snapshots to Ring3 readers; Sim Arena path (x00dc)
//! [`publish_arena_then_rotate`] stores **caller-owned** Prev/Curr sheets then
//! [`crate::sim_arena::SimArena::rotate_faces`] (never live face across rotate;
//! TELEPORT leave-clear stays on x00db).
//!
//! Prev/Curr contract for Ring3: publish **start-of-tick Prev** and **end-of-tick Curr**
//! (distinct faces for α-lerp), then commit Prev←Curr for the next tick's velocity.
//!
//! **Ring3 SSoT** is the Epoch sheet pair the caller passes (host: tick-start
//! `start` + `world.curr`). Arena is Carry-Over / TELEPORT lifetime auxiliary —
//! [`publish_arena_then_rotate`] does **not** overlay Arena f32 poses onto those
//! sheets (stale Work must not clobber f64 curr). Teleport for GPU snap is a
//! **flags-only** stamp via [`stamp_teleport_flags_from_arena_work`] onto a
//! **publish clone** (x00eb; live `world.curr` stays clean). Host `MvccXpbd`
//! marks sheet dirty on page→sheet pose change so Arena sync stays aligned.
//! Pure-Arena tests build sheets from faces via [`epoch_sheet_from_sim_face`]
//! before calling publish.
//!
//! [`FrameBridge::tick`] / [`FrameBridge::tick_mirror_publish`] remain the legacy
//! XPBD-only publish path (no Carry-Over). Host `sim.rs` is the Arena-wired barrier.
//! Non-guarantees: Loro CRDT / Hydration; Zenoh publish.
//!
//! Safety: `#![forbid(unsafe_code)]`.

#![forbid(unsafe_code)]

use std::sync::Arc;

use arc_swap::ArcSwap;

use crate::ids::LocalSlotId;
use crate::sim_arena::{SimArena, SimFace};
use crate::spatial::SceneBounds;
use crate::tape::{EpochSheet, PendingPatch, ReadinessMask, TapeError, WorldTransformf64};
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
    /// **Legacy XPBD-only path** — does not run Sim Arena Carry-Over / rotate.
    /// Host authoritative barrier is `host-server` `PhysicsWorld::step_dt` (x00dc).
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
    /// **Legacy path** (no Sim Arena Carry-Over). See [`Self::tick`].
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
    pub fn apply_reparent(&mut self, topo: &mut FlatTopology) -> Result<(), BridgeError> {
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

/// Build a minimal allocated Epoch sheet whose poses match `face` (Phase 1a tests / seed).
///
/// Copies Arena teleport → [`WorldTransformf64::FLAG_TELEPORT`] (x00eb GPU snap).
pub fn epoch_sheet_from_sim_face(face: &SimFace, epoch_id: u64) -> EpochSheet {
    let n = face.slot_count();
    let mut sheet = EpochSheet::with_capacity(n);
    for slot in 0..n as LocalSlotId {
        let h = sheet.alloc_row();
        debug_assert_eq!(h.row_id, slot);
        let pos = face.pos(slot);
        let rot = face.rot(slot);
        let mut xf = WorldTransformf64::identity_at([
            f64::from(pos[0]),
            f64::from(pos[1]),
            f64::from(pos[2]),
        ]);
        xf.rotation = rot;
        xf.set_teleport(face.is_teleport(slot));
        sheet.world_transforms[h.row_id as usize] = xf;
        sheet.readiness[h.row_id as usize] =
            ReadinessMask(ReadinessMask::ALLOCATED | ReadinessMask::TRANSFORM);
    }
    sheet.epoch_id = epoch_id;
    sheet.clear_dirty();
    sheet
}

/// Copy Arena **Work** teleport bits onto sheet [`WorldTransformf64::flags`] (x00eb).
///
/// Pose columns are untouched (Phase 1a / x00dc as-is contract). Call on a
/// **publish clone** so live `world.curr` does not sticky-carry `FLAG_TELEPORT`
/// into the next tick's Prev via `commit_prev_from_curr`.
///
/// Sparse over [`SimArena::this_tick_dirty`] (not O(n) full scan): teleport is
/// only set via `write_teleport` / SimPatch, which always `mark_dirty`. Live
/// publish clones start with flags cleared, so non-dirty rows need no clear pass.
pub fn stamp_teleport_flags_from_arena_work(sheet: &mut EpochSheet, arena: &SimArena) {
    let n = sheet
        .world_transforms
        .len()
        .min(arena.slot_count())
        .min(sheet.readiness.len());
    let work = arena.work();
    for &slot in arena.this_tick_dirty() {
        let i = slot as usize;
        if i >= n {
            continue;
        }
        sheet.world_transforms[i].set_teleport(work.is_teleport(slot));
    }
}

/// x00dc barrier tail: publish owned Epoch sheets, then rotate Arena faces.
///
/// Order (mandatory):
/// 1. [`EpochSnapshotPair::publish`] the caller-owned `prev_sheet` / `curr_sheet`
///    **as-is** — including `epoch_id` (no Arena field rewrite; host live sheet and
///    Ring3 Arc stay aligned). Do not overlay Arena f32 onto them.
/// 2. [`SimArena::rotate_faces`] (Published-leave TELEPORT clear is in-place on the
///    buffer that becomes Prev — Ring3 already holds owned clones).
///
/// Pure-Arena callers should build sheets from faces first
/// ([`epoch_sheet_from_sim_face`], which sets `epoch_id` from Arena epochs); host
/// passes tick-start / end-of-tick sheets with their own `epoch_id`.
pub fn publish_arena_then_rotate(
    arena: &mut SimArena,
    snapshots: &EpochSnapshotPair,
    prev_sheet: EpochSheet,
    curr_sheet: EpochSheet,
) {
    snapshots.publish(prev_sheet, curr_sheet);
    arena.rotate_faces();
}

/// Sync dirty Epoch rows into Arena Work via `write_*` (host / bridge writers).
///
/// Prefer this over raw [`SimArena::work_mut`] so Carry-Over and Published-leave
/// TELEPORT clear stay on the dirty window. This keeps Arena aligned for Carry-Over;
/// it does **not** feed Ring3 (see [`publish_arena_then_rotate`]). f64→f32 is
/// intentional for Arena columns only.
pub fn sync_dirty_sheet_to_arena_work(arena: &mut SimArena, sheet: &EpochSheet) {
    if arena.slot_count() < sheet.len() {
        arena.resize(sheet.len());
    }
    for row in sheet.iter_dirty_rows() {
        let i = row as usize;
        if i >= sheet.world_transforms.len() || i >= arena.slot_count() {
            continue;
        }
        let xf = &sheet.world_transforms[i];
        arena.write_pos(
            row,
            xf.position[0] as f32,
            xf.position[1] as f32,
            xf.position[2] as f32,
        );
        arena.write_rot(
            row,
            xf.rotation[0],
            xf.rotation[1],
            xf.rotation[2],
            xf.rotation[3],
        );
    }
}

/// Seed every allocated sheet row onto all Arena faces (host bootstrap; not a tick writer).
pub fn seed_arena_from_sheet(arena: &mut SimArena, sheet: &EpochSheet) {
    if arena.slot_count() < sheet.len() {
        arena.resize(sheet.len());
    }
    for row in sheet.iter_active_rows() {
        let i = row as usize;
        if i >= sheet.world_transforms.len() || i >= arena.slot_count() {
            continue;
        }
        let xf = &sheet.world_transforms[i];
        arena.seed_slot(
            row,
            xf.position[0] as f32,
            xf.position[1] as f32,
            xf.position[2] as f32,
            xf.rotation,
        );
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

    /// One Arena tick through the x00dc bridge barrier (carry → write → publish → rotate).
    fn arena_bridge_tick(
        arena: &mut SimArena,
        snaps: &EpochSnapshotPair,
        write: impl FnOnce(&mut SimArena),
    ) {
        arena.carry_over();
        write(arena);
        let prev_sheet = epoch_sheet_from_sim_face(arena.published(), arena.epoch());
        let curr_sheet = epoch_sheet_from_sim_face(arena.work(), arena.work_epoch());
        publish_arena_then_rotate(arena, snaps, prev_sheet, curr_sheet);
    }

    #[test]
    fn arena_bridge_publish_prevents_ancestor_regression_after_three_idle_ticks() {
        let slot = 0u32;
        let mut arena = SimArena::with_slots(2);
        let snaps = EpochSnapshotPair::new(EpochSheet::default(), EpochSheet::default());

        arena_bridge_tick(&mut arena, &snaps, |a| {
            a.write_pos(slot, 10.0, 0.0, 0.0);
            a.write_rot(slot, 0.0, 0.0, 0.0, 1.0);
        });
        arena_bridge_tick(&mut arena, &snaps, |a| {
            a.write_pos(slot, 20.0, 1.0, 0.0);
            a.write_rot(slot, 0.0, 1.0, 0.0, 0.0);
        });

        assert_eq!(
            snaps.load_curr().world_transforms[slot as usize].position,
            [20.0, 1.0, 0.0]
        );

        for tick in 0..3 {
            arena_bridge_tick(&mut arena, &snaps, |_| {});
            let curr = snaps.load_curr().world_transforms[slot as usize].position;
            assert_eq!(
                curr,
                [20.0, 1.0, 0.0],
                "idle bridge tick {tick}: Ring3 curr must not ancestor-regress"
            );
            assert_eq!(
                arena.published().pos(slot),
                [20.0, 1.0, 0.0],
                "idle bridge tick {tick}: pure-Arena path built Ring3 curr from Work→Published; face must keep pose (not a host sheet-SSoT claim)"
            );
        }
    }

    #[test]
    fn arena_bridge_keeps_teleport_on_published_for_one_epoch() {
        // Arena TELEPORT survives one Published epoch. Pure-Arena bridge builds sheets
        // via `epoch_sheet_from_sim_face` (full face copy) — not host sparse stamp.
        let slot = 0u32;
        let mut arena = SimArena::with_slots(1);
        let snaps = EpochSnapshotPair::new(EpochSheet::default(), EpochSheet::default());

        arena_bridge_tick(&mut arena, &snaps, |a| {
            a.write_pos(slot, 1.0, 0.0, 0.0);
            a.write_teleport(slot, true);
        });
        // After publish→rotate, Work (with TELEPORT) is Arena.Published for one epoch.
        assert!(arena.published().is_teleport(slot));
        assert_eq!(
            snaps.load_curr().world_transforms[slot as usize].position,
            [1.0, 0.0, 0.0]
        );
        assert!(
            snaps.load_curr().world_transforms[slot as usize].is_teleport(),
            "x00eb: epoch_sheet_from_sim_face must copy FLAG_TELEPORT for Ring3 snap"
        );

        arena_bridge_tick(&mut arena, &snaps, |_| {});
        assert!(
            !arena.published().is_teleport(slot),
            "Published-leave clear must run on rotate after the snap was taken"
        );
        assert!(
            !snaps.load_curr().world_transforms[slot as usize].is_teleport(),
            "idle tick Work has no teleport → epoch_sheet_from_sim_face rebuilds Curr without FLAG"
        );
        assert_eq!(
            snaps.load_curr().world_transforms[slot as usize].position,
            [1.0, 0.0, 0.0]
        );
    }

    #[test]
    fn stamp_teleport_flags_from_arena_work_does_not_rewrite_pose() {
        let mut arena = SimArena::with_slots(1);
        arena.carry_over();
        arena.write_pos(0, 7.0, 8.0, 9.0);
        arena.write_teleport(0, true);
        let mut sheet = EpochSheet::with_capacity(1);
        let h = sheet.alloc_row();
        sheet
            .set_transform(h.row_id, WorldTransformf64::identity_at([1.0, 2.0, 3.0]))
            .unwrap();
        assert!(!sheet.world_transforms[0].is_teleport());
        stamp_teleport_flags_from_arena_work(&mut sheet, &arena);
        assert!(sheet.world_transforms[0].is_teleport());
        assert_eq!(
            sheet.world_transforms[0].position,
            [1.0, 2.0, 3.0],
            "stamp is flags-only; f64 pose SSoT stays on the sheet"
        );
        arena.write_teleport(0, false);
        stamp_teleport_flags_from_arena_work(&mut sheet, &arena);
        assert!(!sheet.world_transforms[0].is_teleport());
    }

    #[test]
    fn publish_arena_forbids_live_face_across_rotate() {
        // Owned sheets must outlive rotate: mutating live Published after publish must not
        // change the Arc already handed to Ring3.
        let mut arena = SimArena::with_slots(1);
        arena.carry_over();
        arena.write_pos(0, 3.0, 0.0, 0.0);
        let snaps = EpochSnapshotPair::new(EpochSheet::default(), EpochSheet::default());
        let prev_sheet = epoch_sheet_from_sim_face(arena.published(), 0);
        let curr_sheet = epoch_sheet_from_sim_face(arena.work(), 1);
        publish_arena_then_rotate(&mut arena, &snaps, prev_sheet, curr_sheet);
        let loaded = snaps.load_curr();
        assert_eq!(loaded.world_transforms[0].position, [3.0, 0.0, 0.0]);
        // Live Published is the old Work; leave-clear / later writers must not poke the Arc.
        arena.work_mut().set_pos(0, 99.0, 99.0, 99.0);
        assert_eq!(
            snaps.load_curr().world_transforms[0].position,
            [3.0, 0.0, 0.0]
        );
    }

    #[test]
    fn publish_arena_keeps_caller_sheet_over_stale_arena_work() {
        // Regression (x00dc review): dirty-window Work must not clobber f64 sheet curr
        // when page→sheet omitted mark_dirty (MvccXpbd). Ring3 SSoT = caller sheets.
        let mut arena = SimArena::with_slots(1);
        arena.carry_over();
        arena.write_pos(0, 1.0, 0.0, 0.0);
        arena.rotate_faces();
        arena.carry_over();
        // Stale Work still holds 1.0 while authoritative sheet advanced to 9.0.
        let mut prev = EpochSheet::with_capacity(1);
        let mut curr = EpochSheet::with_capacity(1);
        let hp = prev.alloc_row();
        let hc = curr.alloc_row();
        prev.set_transform(hp.row_id, WorldTransformf64::identity_at([8.0, 0.0, 0.0]))
            .unwrap();
        curr.set_transform(hc.row_id, WorldTransformf64::identity_at([9.0, 0.0, 0.0]))
            .unwrap();
        let snaps = EpochSnapshotPair::new(EpochSheet::default(), EpochSheet::default());
        publish_arena_then_rotate(&mut arena, &snaps, prev, curr);
        assert_eq!(
            snaps.load_curr().world_transforms[0].position,
            [9.0, 0.0, 0.0],
            "stale Arena Work must not overlay Ring3 curr"
        );
        assert_eq!(
            snaps.load_prev().world_transforms[0].position,
            [8.0, 0.0, 0.0],
            "caller prev sheet must remain Ring3 prev SSoT"
        );
    }

    #[test]
    fn sync_dirty_sheet_to_arena_work_marks_dirty_window() {
        let mut arena = SimArena::with_slots(2);
        let mut sheet = EpochSheet::with_capacity(2);
        let a = sheet.alloc_row();
        let _b = sheet.alloc_row();
        sheet.clear_dirty();
        sheet
            .set_transform(a.row_id, WorldTransformf64::identity_at([7.0, 8.0, 9.0]))
            .unwrap();
        sync_dirty_sheet_to_arena_work(&mut arena, &sheet);
        assert_eq!(arena.work().pos(a.row_id), [7.0, 8.0, 9.0]);
        assert_eq!(arena.this_tick_dirty(), &[a.row_id]);
    }
}
