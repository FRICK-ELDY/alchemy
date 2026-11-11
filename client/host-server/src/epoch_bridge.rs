//! Shadow FrameBridge on the host 60Hz loop (x008f).
//!
//! Guarantees: after each Rapier step, reconcile Manifest entities into tape rows,
//! mirror **moved** Rapier poses as kinematic (`inv_mass = 0`, no `DYNAMIC`), then
//! [`FrameBridge::tick_mirror_publish`] (no XPBD / LBVH / contact) so Prev/Curr
//! ArcSwap faces update for Ring3 readers.
//! Non-guarantees: XPBD authority on the wire; Rapier/SlotGraph removal; SceneGpu bind.
//!
//! Wire Frame sampling stays on Rapier (`main` / `network_worker`).
//!
//! Budget (shadow): spawn/despawn-only reconcile; pose patches only when TRS drifts;
//! idle ticks skip sheet clone publish.

use std::collections::{HashMap, HashSet};

use alchemy_protocol::scene_entry_is_user_root;
use alchemy_shared_sim::bridge::BridgeError;
use alchemy_shared_sim::xpbd::XpbdWorld;
use alchemy_shared_sim::{
    ColliderShape, EpochSnapshotPair, FlatTopology, FrameBridge, NodeHandle, PendingPatch,
    ReadinessMask, WorldTransformf64, PHYS_BALL_RADIUS,
};

use crate::sim::PhysicsWorld;

/// Matches `ColliderBuilder::ball(0.3)` on Rapier user_roots (`sim.rs`).
const USER_ROOT_SENSOR_RADIUS: f32 = 0.3;

/// Squared position epsilon (meters²) before queueing a mirror patch.
const POSE_EPS2: f64 = 1e-10;
/// Scale / inv_mass abs epsilon for mirror dirty check.
const TRS_EPS: f32 = 1e-5;
/// `1 - |dot(q,q')|` threshold; treats `q` and `-q` as the same orientation.
const QUAT_DOT_EPS: f32 = 1e-5;

/// Host-owned shadow sim: Rapier remains wire-authoritative.
pub struct EpochBridgeHost {
    bridge: FrameBridge,
    world: XpbdWorld,
    topo: FlatTopology,
    snapshots: EpochSnapshotPair,
    /// Live Manifest / physics entities → tape handles.
    entity_rows: HashMap<u32, NodeHandle>,
    /// Ensure failed once; ignored by membership until Rapier drops the id (no per-tick force_publish).
    ensure_skipped: HashSet<u32>,
    /// Scratch for iterating `entity_rows` keys without per-tick HashMap alloc churn.
    id_scratch: Vec<u32>,
    /// Successful `tick_after_rapier` / bootstrap barrier calls (includes idle skips).
    pub barrier_ticks: u64,
    /// Times ArcSwap Prev/Curr were actually republished.
    pub publishes: u64,
    /// Soft failures (tombstone / set_transform / missing pose / patch errors).
    pub mirror_warns: u64,
}

impl EpochBridgeHost {
    /// Seed kinematic mirror rows from the current Rapier world.
    pub fn from_physics(physics: &PhysicsWorld) -> Self {
        let cap = physics.mesh_entity_ids().count()
            + physics.boxes.len()
            + physics.user_root_slot_count()
            + 8;
        let mut world = XpbdWorld::new(cap);
        // Shadow: no gravity / XPBD work — rows stay non-DYNAMIC with inv_mass=0.
        world.gravity = [0.0, 0.0, 0.0];
        world.substeps = 1;

        let mut host = Self {
            bridge: FrameBridge::default(),
            world,
            topo: FlatTopology::default(),
            snapshots: EpochSnapshotPair::new(
                alchemy_shared_sim::EpochSheet::default(),
                alchemy_shared_sim::EpochSheet::default(),
            ),
            entity_rows: HashMap::new(),
            ensure_skipped: HashSet::new(),
            id_scratch: Vec::new(),
            barrier_ticks: 0,
            publishes: 0,
            mirror_warns: 0,
        };
        let _ = host.reconcile(physics);
        // Force initial Curr face (membership edited live sheet; may have no pose patches).
        match host.finish_mirror_barrier(/*force_publish=*/ true) {
            Ok(()) => {}
            Err(err) => {
                log::warn!("epoch bridge bootstrap mirror failed: {err:?}");
                host.mirror_warns = host.mirror_warns.wrapping_add(1);
            }
        }
        host
    }

    /// Published Prev/Curr faces (Ring3 / metrics consumers; unused on wire path yet).
    #[allow(dead_code)] // retained API for SceneGpu / epoch_view follow-ups
    pub fn snapshots(&self) -> &EpochSnapshotPair {
        &self.snapshots
    }

    /// Lookup tape row for a Manifest `entity_id`.
    #[allow(dead_code)] // retained API for Grab / debug X-Ray follow-ups
    pub fn entity_row(&self, entity_id: u32) -> Option<NodeHandle> {
        self.entity_rows.get(&entity_id).copied()
    }

    pub fn entity_count(&self) -> usize {
        self.entity_rows.len()
    }

    /// Rapier step 後: reconcile (if membership changed) → dirty pose patches →
    /// [`FrameBridge::tick_mirror_publish`]. Tape membership edits force publish even
    /// when TRS patches are empty (alloc/tombstone bypass the patch queue).
    pub fn tick_after_rapier(
        &mut self,
        physics: &PhysicsWorld,
        _dt: f64,
    ) -> Result<(), BridgeError> {
        let force_publish = if self.membership_changed(physics) {
            self.reconcile(physics)
        } else {
            false
        };
        self.queue_dirty_pose_patches(physics);
        self.finish_mirror_barrier(force_publish)
    }

    fn finish_mirror_barrier(&mut self, force_publish: bool) -> Result<(), BridgeError> {
        let (patch_errors, published) = self.bridge.tick_mirror_publish(
            &mut self.world,
            &mut self.topo,
            &self.snapshots,
            force_publish,
        )?;
        if !patch_errors.is_empty() {
            self.mirror_warns = self.mirror_warns.wrapping_add(patch_errors.len() as u64);
            log::warn!(
                "epoch bridge mirror patch errors count={} first={:?}",
                patch_errors.len(),
                patch_errors.first()
            );
        }
        self.barrier_ticks = self.barrier_ticks.wrapping_add(1);
        if published {
            self.publishes = self.publishes.wrapping_add(1);
        }
        Ok(())
    }

    fn mirrored(&self, entity_id: u32) -> bool {
        self.entity_rows.contains_key(&entity_id) || self.ensure_skipped.contains(&entity_id)
    }

    fn membership_changed(&self, physics: &PhysicsWorld) -> bool {
        for id in physics.mesh_entity_ids() {
            if !self.mirrored(id) {
                return true;
            }
        }
        for id in &physics.boxes.entity_ids {
            if !self.mirrored(*id) {
                return true;
            }
        }
        for slot in 0..physics.user_root_slot_count() {
            let id = PhysicsWorld::user_root_entity_id(slot);
            if !self.mirrored(id) {
                return true;
            }
        }
        self.entity_rows
            .keys()
            .any(|&id| !physics_has_entity(physics, id))
            || self
                .ensure_skipped
                .iter()
                .any(|&id| !physics_has_entity(physics, id))
    }

    /// Returns `true` when live tape membership changed (alloc / tombstone).
    fn reconcile(&mut self, physics: &PhysicsWorld) -> bool {
        let mut tape_mutated = false;

        // Drop quarantine for ids Rapier no longer owns.
        self.ensure_skipped
            .retain(|&id| physics_has_entity(physics, id));

        // Drop rows Rapier no longer owns (no HashSet alloc: scan map keys).
        let stale: Vec<u32> = self
            .entity_rows
            .keys()
            .copied()
            .filter(|&id| !physics_has_entity(physics, id))
            .collect();
        for id in stale {
            if let Some(handle) = self.entity_rows.remove(&id) {
                match self.world.curr.tombstone(handle) {
                    Ok(()) => tape_mutated = true,
                    Err(err) => {
                        // Keep map↔tape aligned so the next tick retries despawn.
                        self.entity_rows.insert(id, handle);
                        self.mirror_warns = self.mirror_warns.wrapping_add(1);
                        log::warn!("epoch bridge tombstone entity_id={id} failed: {err:?}");
                    }
                }
            }
        }

        for id in physics.mesh_entity_ids() {
            tape_mutated |= self.ensure_entity(physics, id);
        }
        for &id in &physics.boxes.entity_ids {
            tape_mutated |= self.ensure_entity(physics, id);
        }
        for slot in 0..physics.user_root_slot_count() {
            tape_mutated |= self.ensure_entity(physics, PhysicsWorld::user_root_entity_id(slot));
        }

        alchemy_shared_sim::xpbd::sync_prev_capacity(&mut self.world);
        tape_mutated
    }

    /// Returns `true` when a new live row was allocated.
    fn ensure_entity(&mut self, physics: &PhysicsWorld, entity_id: u32) -> bool {
        if self.entity_rows.contains_key(&entity_id) {
            self.ensure_skipped.remove(&entity_id);
            return false;
        }
        if self.ensure_skipped.contains(&entity_id) {
            return false;
        }
        let Some((pose, scale)) = pose_scale_for(physics, entity_id) else {
            self.mirror_warns = self.mirror_warns.wrapping_add(1);
            self.ensure_skipped.insert(entity_id);
            log::warn!("epoch bridge ensure quarantined unknown entity_id={entity_id}");
            return false;
        };
        let handle = self.world.curr.alloc_row();
        let (mask, collider) = row_kind(physics, entity_id);
        if let Err(err) = self
            .world
            .curr
            .set_transform(handle.row_id, kinematic_transform(pose, scale))
        {
            self.mirror_warns = self.mirror_warns.wrapping_add(1);
            log::warn!("epoch bridge set_transform entity_id={entity_id} failed: {err:?}");
            let _ = self.world.curr.tombstone(handle);
            self.ensure_skipped.insert(entity_id);
            return false;
        }
        self.world.curr.readiness[handle.row_id as usize] = mask;
        self.world.curr.colliders[handle.row_id as usize] = collider;
        self.entity_rows.insert(entity_id, handle);
        self.ensure_skipped.remove(&entity_id);
        true
    }

    fn refill_id_scratch(&mut self) {
        self.id_scratch.clear();
        self.id_scratch.extend(self.entity_rows.keys().copied());
    }

    fn queue_dirty_pose_patches(&mut self, physics: &PhysicsWorld) {
        self.refill_id_scratch();
        for i in 0..self.id_scratch.len() {
            let entity_id = self.id_scratch[i];
            self.queue_pose_patch(physics, entity_id);
        }
    }

    fn queue_pose_patch(&mut self, physics: &PhysicsWorld, entity_id: u32) {
        let Some(handle) = self.entity_rows.get(&entity_id).copied() else {
            return;
        };
        let Some((pose, scale)) = pose_scale_for(physics, entity_id) else {
            self.mirror_warns = self.mirror_warns.wrapping_add(1);
            debug_assert!(
                false,
                "entity_id={entity_id} in entity_rows but missing from Rapier"
            );
            log::warn!("epoch bridge pose missing for mapped entity_id={entity_id}");
            return;
        };
        let xf = kinematic_transform(pose, scale);
        let idx = handle.row_id as usize;
        if let Some(cur) = self.world.curr.world_transforms.get(idx) {
            if transforms_close(cur, &xf) {
                return;
            }
        }
        self.bridge.queue_patch(PendingPatch::SetTransform {
            row_id: handle.row_id,
            transform: xf,
            preserve_physics_coords: false,
        });
    }
}

fn physics_has_entity(physics: &PhysicsWorld, entity_id: u32) -> bool {
    if scene_entry_is_user_root(entity_id) {
        let slot = (entity_id - alchemy_protocol::USER_ROOT_ENTITY_FIRST) as usize;
        return slot < physics.user_root_slot_count();
    }
    if physics.mesh_entity_ids().any(|id| id == entity_id) {
        return true;
    }
    physics.boxes.entity_ids.contains(&entity_id)
}

fn row_kind(physics: &PhysicsWorld, entity_id: u32) -> (ReadinessMask, ColliderShape) {
    if scene_entry_is_user_root(entity_id) {
        return (
            ReadinessMask(
                ReadinessMask::ALLOCATED | ReadinessMask::TRANSFORM | ReadinessMask::COLLIDER,
            ),
            ColliderShape {
                shape_type: ColliderShape::SPHERE,
                extents: [USER_ROOT_SENSOR_RADIUS, 0.0, 0.0],
            },
        );
    }
    if physics.mesh_entity_ids().any(|id| id == entity_id) {
        return (
            ReadinessMask(
                ReadinessMask::ALLOCATED
                    | ReadinessMask::TRANSFORM
                    | ReadinessMask::MESH
                    | ReadinessMask::COLLIDER,
            ),
            ColliderShape {
                shape_type: ColliderShape::SPHERE,
                extents: [PHYS_BALL_RADIUS, 0.0, 0.0],
            },
        );
    }
    if let Some(i) = physics
        .boxes
        .entity_ids
        .iter()
        .position(|&id| id == entity_id)
    {
        let he = physics.boxes.half_extents.get(i).copied().unwrap_or(0.25);
        return (
            ReadinessMask(
                ReadinessMask::ALLOCATED | ReadinessMask::TRANSFORM | ReadinessMask::COLLIDER,
            ),
            ColliderShape {
                shape_type: ColliderShape::BOX,
                extents: [he, he, he],
            },
        );
    }
    (
        ReadinessMask(ReadinessMask::ALLOCATED | ReadinessMask::TRANSFORM),
        ColliderShape::default(),
    )
}

type PoseScale = (((f32, f32, f32), [f32; 4]), [f32; 3]);

fn pose_scale_for(physics: &PhysicsWorld, entity_id: u32) -> Option<PoseScale> {
    let scale = physics.scale_for(entity_id);
    if scene_entry_is_user_root(entity_id) {
        let slot = (entity_id - alchemy_protocol::USER_ROOT_ENTITY_FIRST) as usize;
        if slot >= physics.user_root_slot_count() {
            return None;
        }
        return Some((physics.user_root_pose(slot), scale));
    }
    if let Some(pose) = physics.mesh_pose_for(entity_id) {
        return Some((pose, scale));
    }
    if let Some(i) = physics
        .boxes
        .entity_ids
        .iter()
        .position(|&id| id == entity_id)
    {
        return Some((physics.box_pose(i), scale));
    }
    None
}

fn quat_orientations_close(a: [f32; 4], b: [f32; 4]) -> bool {
    let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    1.0 - dot.abs() <= QUAT_DOT_EPS
}

fn transforms_close(a: &WorldTransformf64, b: &WorldTransformf64) -> bool {
    let dx = a.position[0] - b.position[0];
    let dy = a.position[1] - b.position[1];
    let dz = a.position[2] - b.position[2];
    if dx * dx + dy * dy + dz * dz > POSE_EPS2 {
        return false;
    }
    if !quat_orientations_close(a.rotation, b.rotation) {
        return false;
    }
    for i in 0..3 {
        if (a.scale[i] - b.scale[i]).abs() > TRS_EPS {
            return false;
        }
    }
    (a.inv_mass - b.inv_mass).abs() < 1e-12
}

fn kinematic_transform((t, q): ((f32, f32, f32), [f32; 4]), scale: [f32; 3]) -> WorldTransformf64 {
    WorldTransformf64 {
        position: [t.0 as f64, t.1 as f64, t.2 as f64],
        inv_mass: 0.0,
        rotation: q,
        scale,
        frame_delta_xyz: [0.0, 0.0, 0.0],
        flags: 0,
        _pad0: 0,
        _pad1: [0; 4],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{demo_entries_for_test, LocoMode, PhysicsWorld};

    fn demo_physics() -> PhysicsWorld {
        PhysicsWorld::from_entries(&demo_entries_for_test(), LocoMode::Noclip, 3.0)
    }

    #[test]
    fn seeds_mesh_boxes_and_user_roots() {
        let physics = demo_physics();
        let bridge = EpochBridgeHost::from_physics(&physics);
        assert!(bridge.entity_count() >= 1 + physics.boxes.len() + physics.user_root_slot_count());
        let mesh_id = physics.mesh_entity_id().expect("demo mesh");
        assert!(bridge.entity_row(mesh_id).is_some());
        assert!(bridge.barrier_ticks >= 1);
        assert!(bridge.publishes >= 1);
        assert_eq!(bridge.mirror_warns, 0);
    }

    #[test]
    fn tick_keeps_prev_curr_distinct_when_rapier_moves() {
        let mut physics = demo_physics();
        let mut bridge = EpochBridgeHost::from_physics(&physics);
        let mesh_id = physics.mesh_entity_id().expect("demo mesh");
        let row = bridge.entity_row(mesh_id).expect("row").row_id as usize;

        let before = bridge.snapshots().load_curr().world_transforms[row].position;
        for _ in 0..30 {
            physics.step();
            bridge
                .tick_after_rapier(&physics, 1.0 / 60.0)
                .expect("bridge tick");
        }
        let prev = bridge.snapshots().load_prev().world_transforms[row].position;
        let curr = bridge.snapshots().load_curr().world_transforms[row].position;
        assert_ne!(
            prev, curr,
            "moving mesh should publish distinct Prev/Curr; before={before:?} prev={prev:?} curr={curr:?}"
        );
        let (rt, _) = physics.mesh_pose_for(mesh_id).unwrap();
        assert!((curr[0] - rt.0 as f64).abs() < 1e-4);
        assert!((curr[1] - rt.1 as f64).abs() < 1e-4);
        assert!((curr[2] - rt.2 as f64).abs() < 1e-4);
    }

    #[test]
    fn idle_tick_skips_republish_arc() {
        let physics = demo_physics();
        let mut bridge = EpochBridgeHost::from_physics(&physics);
        // Settle: one more mirror with whatever dirty state, then freeze physics.
        bridge
            .tick_after_rapier(&physics, 1.0 / 60.0)
            .expect("tick");
        let ptr = bridge.snapshots().load_curr().as_ref() as *const _;
        let pubs = bridge.publishes;
        let barriers = bridge.barrier_ticks;
        bridge
            .tick_after_rapier(&physics, 1.0 / 60.0)
            .expect("idle tick");
        assert_eq!(
            bridge.snapshots().load_curr().as_ref() as *const _,
            ptr,
            "unchanged Rapier poses must not re-clone EpochSheet"
        );
        assert_eq!(bridge.publishes, pubs, "idle must not count as publish");
        assert_eq!(bridge.barrier_ticks, barriers + 1);
    }

    #[test]
    fn reconcile_follows_mesh_spawn_and_despawn() {
        let mut physics = demo_physics();
        let mut bridge = EpochBridgeHost::from_physics(&physics);
        let n0 = bridge.entity_count();

        let mut entry = demo_entries_for_test()
            .into_iter()
            .find(|e| !e.blake3.is_empty())
            .expect("mesh template");
        entry.entity_id = 9_001;
        entry.x = 2.0;
        assert!(physics.spawn_mesh_entry(&entry));
        let pubs_before_spawn = bridge.publishes;
        bridge
            .tick_after_rapier(&physics, 1.0 / 60.0)
            .expect("tick");
        assert_eq!(bridge.entity_count(), n0 + 1);
        assert!(bridge.entity_row(9_001).is_some());
        assert!(
            bridge.publishes > pubs_before_spawn,
            "membership spawn must force ArcSwap publish"
        );
        let row = bridge.entity_row(9_001).unwrap().row_id as usize;
        let published = bridge.snapshots().load_curr();
        assert!(
            !published.readiness[row].contains(ReadinessMask::TOMBSTONE),
            "spawned row must appear on published Curr"
        );
        assert!(
            (published.world_transforms[row].position[0] - 2.0).abs() < 1e-4,
            "spawn pose must be on published Curr"
        );

        assert!(physics.despawn_mesh_entity(9_001));
        let pubs_before_despawn = bridge.publishes;
        bridge
            .tick_after_rapier(&physics, 1.0 / 60.0)
            .expect("tick");
        assert_eq!(bridge.entity_count(), n0);
        assert!(bridge.entity_row(9_001).is_none());
        assert!(
            bridge.publishes > pubs_before_despawn,
            "membership despawn must force ArcSwap publish"
        );
        let after = bridge.snapshots().load_curr();
        assert!(
            after.readiness[row].contains(ReadinessMask::TOMBSTONE),
            "despawned row must be TOMBSTONE on published Curr"
        );
    }

    #[test]
    fn shadow_rows_are_kinematic_not_dynamic() {
        let physics = demo_physics();
        let bridge = EpochBridgeHost::from_physics(&physics);
        let mesh_id = physics.mesh_entity_id().unwrap();
        let row = bridge.entity_row(mesh_id).unwrap().row_id as usize;
        let sheet = bridge.snapshots().load_curr();
        assert!(!sheet.readiness[row].contains(ReadinessMask::DYNAMIC));
        assert_eq!(sheet.world_transforms[row].inv_mass, 0.0);
    }

    #[test]
    fn user_root_collider_matches_rapier_ball() {
        let physics = demo_physics();
        let bridge = EpochBridgeHost::from_physics(&physics);
        let id = PhysicsWorld::user_root_entity_id(0);
        let row = bridge.entity_row(id).unwrap().row_id as usize;
        let c = bridge.snapshots().load_curr().colliders[row];
        assert_eq!(c.shape_type, ColliderShape::SPHERE);
        assert!((c.extents[0] - USER_ROOT_SENSOR_RADIUS).abs() < 1e-6);
    }

    #[test]
    fn quat_double_cover_is_not_dirty() {
        let a = WorldTransformf64 {
            position: [1.0, 2.0, 3.0],
            inv_mass: 0.0,
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
            frame_delta_xyz: [0.0, 0.0, 0.0],
            flags: 0,
            _pad0: 0,
            _pad1: [0; 4],
        };
        let mut b = a;
        b.rotation = [0.0, 0.0, 0.0, -1.0];
        assert!(transforms_close(&a, &b));
    }

    #[test]
    fn ensure_failure_quarantines_without_repeat_force_publish() {
        let physics = demo_physics();
        let mut bridge = EpochBridgeHost::from_physics(&physics);
        let box_id = physics.boxes.entity_ids[0];
        // Simulate ensure failure: Rapier still owns id, mirror gave up.
        let _ = bridge.entity_rows.remove(&box_id);
        bridge.ensure_skipped.insert(box_id);
        assert!(
            !bridge.membership_changed(&physics),
            "quarantined id must count as mirrored for membership"
        );
        let pubs = bridge.publishes;
        bridge
            .tick_after_rapier(&physics, 1.0 / 60.0)
            .expect("tick");
        assert_eq!(
            bridge.publishes, pubs,
            "quarantined holes must not force per-tick publish"
        );
        assert!(bridge.ensure_skipped.contains(&box_id));
    }

    #[test]
    fn quarantine_clears_when_physics_drops_id() {
        let physics = demo_physics();
        let mut bridge = EpochBridgeHost::from_physics(&physics);
        bridge.ensure_skipped.insert(42);
        let _ = bridge.reconcile(&physics);
        assert!(
            !bridge.ensure_skipped.contains(&42),
            "quarantine clears when Rapier does not own the id"
        );
    }
}
