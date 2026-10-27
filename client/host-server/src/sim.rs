//! Rapier world driven by SceneManifest entries (x0027).
//!
//! Guarantees: floor (host-local) + mesh ball + boxes + user_roots from Manifest;
//! per-slot Grab; no Steal; `release_slot` resets to Manifest spawn.
//! Non-guarantees: rotation follow, throw impulse, shared-sim, EntityKind.

use alchemy_protocol::{
    scene_entry_grabbable, scene_entry_is_box, scene_entry_is_user_root, LocomotionMode,
    SceneEntry, DEFAULT_MOVE_SPEED,
};
use rapier3d::prelude::*;
use std::collections::HashMap;

pub use alchemy_protocol::{USER_ROOT_ENTITY_FIRST, USER_ROOT_SLOTS};

const FLOOR_HALF_H: f32 = 0.5;
const BALL_RADIUS: f32 = 0.25;
const DEMO_GRAVITY_Y: f32 = -2.2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocoMode {
    Noclip,
    Fly,
    Walk,
}

impl LocoMode {
    pub fn from_proto(mode: i32) -> Self {
        match LocomotionMode::try_from(mode).unwrap_or(LocomotionMode::Unspecified) {
            LocomotionMode::Fly => Self::Fly,
            LocomotionMode::Walk => Self::Walk,
            LocomotionMode::Noclip | LocomotionMode::Unspecified => Self::Noclip,
        }
    }
}

/// Cuboids from Manifest (`half_extent > 0`). Grab is per-entry via `grabbable`.
/// Non-Grab → Fixed (static scenery); Grab → Dynamic (x0027).
pub struct CuboidBoxes {
    pub handles: Vec<RigidBodyHandle>,
    pub entity_ids: Vec<u32>,
    pub grabbable: Vec<bool>,
    pub half_extents: Vec<f32>,
}

impl CuboidBoxes {
    fn empty() -> Self {
        Self {
            handles: Vec::new(),
            entity_ids: Vec::new(),
            grabbable: Vec::new(),
            half_extents: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.handles.len()
    }
}

#[derive(Clone, Copy, Debug)]
struct ActiveGrab {
    index: usize,
    offset: Vector<Real>,
}

pub struct PhysicsWorld {
    gravity: Vector<Real>,
    integration_parameters: IntegrationParameters,
    physics_pipeline: PhysicsPipeline,
    island_manager: IslandManager,
    broad_phase: DefaultBroadPhase,
    narrow_phase: NarrowPhase,
    rigid_body_set: RigidBodySet,
    collider_set: ColliderSet,
    impulse_joint_set: ImpulseJointSet,
    multibody_joint_set: MultibodyJointSet,
    ccd_solver: CCDSolver,
    /// Primary mesh entity (dynamic ball), if any.
    mesh: Option<(u32, RigidBodyHandle)>,
    user_roots: [RigidBodyHandle; USER_ROOT_SLOTS],
    user_root_spawns: [(f32, f32, f32); USER_ROOT_SLOTS],
    pub boxes: CuboidBoxes,
    colors: HashMap<u32, (f32, f32, f32, f32)>,
    loco_mode: LocoMode,
    move_speed: f32,
    grabs: [Option<ActiveGrab>; USER_ROOT_SLOTS],
}

impl PhysicsWorld {
    /// Build physics from world-authored Manifest entries (x0027).
    pub fn from_entries(entries: &[SceneEntry], loco_mode: LocoMode, move_speed: f32) -> Self {
        let mut rigid_body_set = RigidBodySet::new();
        let mut collider_set = ColliderSet::new();
        let mut colors = HashMap::new();

        let floor = RigidBodyBuilder::fixed()
            .translation(vector![0.0, -FLOOR_HALF_H, 0.0])
            .build();
        let floor_handle = rigid_body_set.insert(floor);
        collider_set.insert_with_parent(
            ColliderBuilder::cuboid(20.0, FLOOR_HALF_H, 20.0).build(),
            floor_handle,
            &mut rigid_body_set,
        );

        let mut mesh: Option<(u32, RigidBodyHandle)> = None;
        let mut boxes = CuboidBoxes::empty();
        let mut user_root_spawns = [(0.0_f32, 1.6_f32, 6.0_f32); USER_ROOT_SLOTS];
        let mut roots_seen = [false; USER_ROOT_SLOTS];

        for entry in entries {
            let color = entry_color(entry);
            colors.insert(entry.entity_id, color);

            if scene_entry_is_user_root(entry.entity_id) {
                let slot = (entry.entity_id - USER_ROOT_ENTITY_FIRST) as usize;
                if slot < USER_ROOT_SLOTS {
                    user_root_spawns[slot] = (entry.x as f32, entry.y as f32, entry.z as f32);
                    roots_seen[slot] = true;
                }
                continue;
            }

            if !entry.blake3.is_empty() {
                if entry.half_extent > 0.0 {
                    log::warn!(
                        "scene entry {} has blake3 + half_extent; mesh wins (box ignored)",
                        entry.entity_id
                    );
                }
                if mesh.is_none() {
                    let ball = RigidBodyBuilder::dynamic()
                        .translation(vector![entry.x as f32, entry.y as f32, entry.z as f32])
                        .linvel(vector![0.1, 0.0, 0.05])
                        .build();
                    let handle = rigid_body_set.insert(ball);
                    collider_set.insert_with_parent(
                        ColliderBuilder::ball(BALL_RADIUS).restitution(0.15).build(),
                        handle,
                        &mut rigid_body_set,
                    );
                    mesh = Some((entry.entity_id, handle));
                }
                continue;
            }

            if scene_entry_is_box(&entry.blake3, entry.half_extent, entry.entity_id) {
                let he = entry.half_extent;
                let can_grab = scene_entry_grabbable(entry.flags);
                let body = if can_grab {
                    RigidBodyBuilder::dynamic()
                        .translation(vector![entry.x as f32, entry.y as f32, entry.z as f32])
                        .build()
                } else {
                    // Static scenery: Fixed so knocks / mesh ball do not displace it.
                    RigidBodyBuilder::fixed()
                        .translation(vector![entry.x as f32, entry.y as f32, entry.z as f32])
                        .build()
                };
                let handle = rigid_body_set.insert(body);
                collider_set.insert_with_parent(
                    ColliderBuilder::cuboid(he, he, he)
                        .restitution(0.05)
                        .build(),
                    handle,
                    &mut rigid_body_set,
                );
                boxes.handles.push(handle);
                boxes.entity_ids.push(entry.entity_id);
                boxes.grabbable.push(can_grab);
                boxes.half_extents.push(he);
            }
        }

        let mut user_root_handles = Vec::with_capacity(USER_ROOT_SLOTS);
        for slot in 0..USER_ROOT_SLOTS {
            let (ux, uy, uz) = user_root_spawns[slot];
            let user = RigidBodyBuilder::kinematic_position_based()
                .translation(vector![ux, uy, uz])
                .build();
            let handle = rigid_body_set.insert(user);
            collider_set.insert_with_parent(
                ColliderBuilder::ball(0.3).sensor(true).build(),
                handle,
                &mut rigid_body_set,
            );
            user_root_handles.push(handle);
            if !roots_seen[slot] {
                colors.insert(USER_ROOT_ENTITY_FIRST + slot as u32, user_root_color(slot));
            }
        }
        let user_roots: [RigidBodyHandle; USER_ROOT_SLOTS] = user_root_handles
            .try_into()
            .expect("USER_ROOT_SLOTS handles");

        let speed = if move_speed > 0.0 {
            move_speed
        } else {
            DEFAULT_MOVE_SPEED
        };
        let _ = loco_mode;

        Self {
            gravity: vector![0.0, DEMO_GRAVITY_Y, 0.0],
            integration_parameters: IntegrationParameters::default(),
            physics_pipeline: PhysicsPipeline::new(),
            island_manager: IslandManager::new(),
            broad_phase: DefaultBroadPhase::new(),
            narrow_phase: NarrowPhase::new(),
            rigid_body_set,
            collider_set,
            impulse_joint_set: ImpulseJointSet::new(),
            multibody_joint_set: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
            mesh,
            user_roots,
            user_root_spawns,
            boxes,
            colors,
            loco_mode,
            move_speed: speed,
            grabs: [None; USER_ROOT_SLOTS],
        }
    }

    pub fn mesh_entity_id(&self) -> Option<u32> {
        self.mesh.map(|(id, _)| id)
    }

    pub fn color_for(&self, entity_id: u32) -> (f32, f32, f32, f32) {
        self.colors
            .get(&entity_id)
            .copied()
            .unwrap_or((1.0, 1.0, 1.0, 1.0))
    }

    pub fn user_root_entity_id(slot: usize) -> u32 {
        USER_ROOT_ENTITY_FIRST + slot as u32
    }

    fn entity_index(&self, entity_id: u32) -> Option<usize> {
        self.boxes
            .entity_ids
            .iter()
            .position(|&id| id == entity_id)
            .filter(|&i| self.boxes.grabbable.get(i).copied().unwrap_or(false))
    }

    fn box_held_by_other(&self, slot: usize, box_index: usize) -> bool {
        self.grabs
            .iter()
            .enumerate()
            .any(|(s, g)| s != slot && g.map(|ag| ag.index == box_index).unwrap_or(false))
    }

    pub fn apply_grab(
        &mut self,
        slot: usize,
        want_grab: bool,
        entity_id: u32,
        aim: (f32, f32, f32),
    ) {
        if slot >= USER_ROOT_SLOTS {
            return;
        }
        let aim_v = vector![aim.0, aim.1, aim.2];
        if !want_grab {
            self.release_grab(slot);
            return;
        }
        if let Some(g) = self.grabs[slot] {
            self.follow_grab(g, aim_v);
            return;
        }
        let Some(index) = self.entity_index(entity_id) else {
            return;
        };
        if self.box_held_by_other(slot, index) {
            return;
        }
        let handle = self.boxes.handles[index];
        let (ux, uy, uz) = self.user_root_translation(slot);
        let Some(body) = self.rigid_body_set.get_mut(handle) else {
            return;
        };
        let t = *body.translation();
        if !crate::finite::within_grab_start_distance((ux, uy, uz), (t.x, t.y, t.z)) {
            return;
        }
        let offset = t - aim_v;
        body.set_body_type(RigidBodyType::KinematicPositionBased, true);
        body.set_linvel(vector![0.0, 0.0, 0.0], true);
        body.set_angvel(vector![0.0, 0.0, 0.0], true);
        body.set_next_kinematic_translation(t);
        let g = ActiveGrab { index, offset };
        self.grabs[slot] = Some(g);
        self.follow_grab(g, aim_v);
    }

    fn follow_grab(&mut self, g: ActiveGrab, aim: Vector<Real>) {
        let handle = self.boxes.handles[g.index];
        let Some(body) = self.rigid_body_set.get_mut(handle) else {
            return;
        };
        body.set_next_kinematic_translation(aim + g.offset);
    }

    fn release_grab(&mut self, slot: usize) {
        let Some(g) = self.grabs[slot].take() else {
            return;
        };
        let handle = self.boxes.handles[g.index];
        let Some(body) = self.rigid_body_set.get_mut(handle) else {
            return;
        };
        let pos = *body.position();
        body.set_body_type(RigidBodyType::Dynamic, true);
        body.set_position(pos, true);
        body.set_linvel(vector![0.0, 0.0, 0.0], true);
        body.set_angvel(vector![0.0, 0.0, 0.0], true);
        body.wake_up(true);
    }

    pub fn release_slot(&mut self, slot: usize) {
        if slot >= USER_ROOT_SLOTS {
            return;
        }
        self.release_grab(slot);
        let (sx, sy, sz) = self.user_root_spawns[slot];
        let Some(body) = self.rigid_body_set.get_mut(self.user_roots[slot]) else {
            return;
        };
        body.set_next_kinematic_translation(vector![sx, sy, sz]);
        body.set_translation(vector![sx, sy, sz], true);
    }

    pub fn grab_entity_id(&self, slot: usize) -> Option<u32> {
        self.grabs
            .get(slot)
            .and_then(|g| g.map(|ag| self.boxes.entity_ids[ag.index]))
    }

    pub fn loco_mode(&self) -> LocoMode {
        self.loco_mode
    }

    pub fn step(&mut self) {
        self.physics_pipeline.step(
            &self.gravity,
            &self.integration_parameters,
            &mut self.island_manager,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.rigid_body_set,
            &mut self.collider_set,
            &mut self.impulse_joint_set,
            &mut self.multibody_joint_set,
            &mut self.ccd_solver,
            None,
            &(),
            &(),
        );
    }

    pub fn apply_user_intent(&mut self, slot: usize, mx: f32, my: f32, mz: f32, dt: f32) {
        if slot >= USER_ROOT_SLOTS {
            return;
        }
        let (mx, my, mz) = crate::finite::clamp_move_vector(mx, my, mz);
        let speed = self.move_speed;
        let Some(body) = self.rigid_body_set.get_mut(self.user_roots[slot]) else {
            return;
        };
        let t = *body.translation();
        match self.loco_mode {
            LocoMode::Noclip | LocoMode::Fly => {
                let dx = mx * speed * dt;
                let dy = my * speed * dt;
                let dz = mz * speed * dt;
                body.set_next_kinematic_translation(vector![t.x + dx, t.y + dy, t.z + dz]);
            }
            LocoMode::Walk => {
                let dx = mx * speed * dt;
                let dz = mz * speed * dt;
                body.set_next_kinematic_translation(vector![t.x + dx, t.y, t.z + dz]);
            }
        }
    }

    pub fn moving_translation(&self) -> (f32, f32, f32) {
        let Some((_, handle)) = self.mesh else {
            return (0.0, 5.0, 0.0);
        };
        self.rigid_body_set
            .get(handle)
            .map(|b| {
                let t = b.translation();
                (t.x, t.y, t.z)
            })
            .unwrap_or((0.0, 5.0, 0.0))
    }

    pub fn user_root_translation(&self, slot: usize) -> (f32, f32, f32) {
        let fallback = self
            .user_root_spawns
            .get(slot)
            .copied()
            .unwrap_or((0.0, 1.6, 6.0));
        if slot >= USER_ROOT_SLOTS {
            return fallback;
        }
        self.rigid_body_set
            .get(self.user_roots[slot])
            .map(|b| {
                let t = b.translation();
                (t.x, t.y, t.z)
            })
            .unwrap_or(fallback)
    }

    pub fn box_translation(&self, index: usize) -> (f32, f32, f32) {
        let he = self.boxes.half_extents.get(index).copied().unwrap_or(0.25);
        self.boxes
            .handles
            .get(index)
            .and_then(|h| self.rigid_body_set.get(*h))
            .map(|b| {
                let t = b.translation();
                (t.x, t.y, t.z)
            })
            .unwrap_or((0.0, he, 0.0))
    }
}

fn entry_color(entry: &SceneEntry) -> (f32, f32, f32, f32) {
    if entry.a > 0.0 {
        (entry.r, entry.g, entry.b, entry.a)
    } else {
        (1.0, 1.0, 1.0, 1.0)
    }
}

/// Distinct tint per user_root slot when Manifest omits color.
pub fn user_root_color(slot: usize) -> (f32, f32, f32, f32) {
    const PALETTE: [(f32, f32, f32); USER_ROOT_SLOTS] = [
        (0.4, 0.7, 1.0),
        (1.0, 0.55, 0.25),
        (0.45, 0.9, 0.5),
        (0.9, 0.45, 0.85),
    ];
    let (r, g, b) = PALETTE[slot % USER_ROOT_SLOTS];
    (r, g, b, 1.0)
}

/// Unit-test fixture mirroring `World.DemoScene` layout.
/// Production SSoT is Elixir `DemoScene`; keep this in sync manually (x0027 debt).
#[cfg(test)]
pub fn demo_entries_for_test() -> Vec<SceneEntry> {
    use alchemy_protocol::SCENE_FLAG_GRABBABLE;
    let coin = SceneEntry {
        entity_id: 1,
        blake3: "fa24848fa7b971718ce6e8dc9bd2d211808fd64e4ddf555feb66d4f82ee1e72a".into(),
        x: 0.0,
        y: 5.0,
        z: 0.0,
        half_extent: 0.0,
        flags: 0,
        r: 0.2,
        g: 0.9,
        b: 0.4,
        a: 1.0,
    };
    let roots = [
        (0.0, 1.6, 6.0),
        (1.5, 1.6, 6.0),
        (-1.5, 1.6, 6.0),
        (3.0, 1.6, 6.0),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (x, y, z))| SceneEntry {
        entity_id: USER_ROOT_ENTITY_FIRST + i as u32,
        blake3: String::new(),
        x,
        y,
        z,
        half_extent: 0.0,
        flags: 0,
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    });
    let palette = [
        (0.90, 0.35, 0.30),
        (0.90, 0.60, 0.20),
        (0.85, 0.80, 0.25),
        (0.35, 0.75, 0.35),
        (0.25, 0.70, 0.75),
        (0.30, 0.45, 0.90),
        (0.55, 0.35, 0.85),
        (0.85, 0.35, 0.70),
        (0.70, 0.70, 0.75),
    ];
    let boxes = palette.into_iter().enumerate().map(|(i, (r, g, b))| {
        let row = (i / 3) as f64;
        let col = (i % 3) as f64;
        SceneEntry {
            entity_id: 10 + i as u32,
            blake3: String::new(),
            x: (col - 1.0) * 0.8,
            y: 0.25,
            z: 3.5 + (row - 1.0) * 0.8,
            half_extent: 0.25,
            flags: SCENE_FLAG_GRABBABLE,
            r,
            g,
            b,
            a: 1.0,
        }
    });
    std::iter::once(coin).chain(roots).chain(boxes).collect()
}

#[cfg(test)]
fn test_world() -> PhysicsWorld {
    PhysicsWorld::from_entries(
        &demo_entries_for_test(),
        LocoMode::Noclip,
        DEFAULT_MOVE_SPEED,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ball_falls_onto_floor() {
        let mut world = PhysicsWorld::from_entries(
            &demo_entries_for_test(),
            LocoMode::Walk,
            DEFAULT_MOVE_SPEED,
        );
        let (_, y0, _) = world.moving_translation();
        assert!(y0 > 4.5, "spawn above floor");
        for _ in 0..600 {
            world.step();
        }
        let (_, y, _) = world.moving_translation();
        assert!(y > 0.1 && y < 1.0, "expected resting on floor, got y={y}");
    }

    #[test]
    fn noclip_passes_through_floor() {
        let mut world =
            PhysicsWorld::from_entries(&demo_entries_for_test(), LocoMode::Noclip, 10.0);
        let dt = 1.0 / 60.0;
        for _ in 0..120 {
            world.apply_user_intent(0, 0.0, -1.0, 0.0, dt);
            world.step();
        }
        let (_, y, _) = world.user_root_translation(0);
        assert!(y < 0.0, "noclip should pass below floor, got y={y}");
    }

    #[test]
    fn two_slots_move_independently() {
        let mut world =
            PhysicsWorld::from_entries(&demo_entries_for_test(), LocoMode::Noclip, 10.0);
        let dt = 1.0 / 60.0;
        for _ in 0..60 {
            world.apply_user_intent(0, 1.0, 0.0, 0.0, dt);
            world.apply_user_intent(1, -1.0, 0.0, 0.0, dt);
            world.step();
        }
        let (x0, _, _) = world.user_root_translation(0);
        let (x1, _, _) = world.user_root_translation(1);
        assert!(x0 > 0.5, "slot0 should move +X, got {x0}");
        assert!(x1 < 1.0, "slot1 should move -X from 1.5, got {x1}");
        assert!(x0 > x1, "slot0 ahead of slot1 on X");
    }

    #[test]
    fn four_slots_have_distinct_spawns() {
        let world = test_world();
        assert_eq!(USER_ROOT_SLOTS, 4);
        assert_eq!(PhysicsWorld::user_root_entity_id(3), 6);
        let mut xs = Vec::with_capacity(USER_ROOT_SLOTS);
        for slot in 0..USER_ROOT_SLOTS {
            let (x, y, z) = world.user_root_translation(slot);
            assert!((y - 1.6).abs() < 0.01 && (z - 6.0).abs() < 0.01);
            xs.push(x);
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for w in xs.windows(2) {
            assert!(
                (w[1] - w[0]).abs() > 0.5,
                "spawns should differ on X: {xs:?}"
            );
        }
    }

    #[test]
    fn unspecified_maps_to_noclip() {
        assert_eq!(LocoMode::from_proto(0), LocoMode::Noclip);
        assert_eq!(LocoMode::from_proto(1), LocoMode::Noclip);
    }

    #[test]
    fn boxes_spawn_as_3x3_grabbable() {
        let world = test_world();
        assert_eq!(world.boxes.len(), 9);
        assert_eq!(world.boxes.entity_ids[0], 10);
        assert_eq!(world.boxes.entity_ids[8], 18);
        assert!(world.boxes.grabbable.iter().all(|&g| g));
        let (x4, y4, z4) = world.box_translation(4);
        assert!((y4 - 0.25).abs() < 0.01);
        assert!(
            x4.abs() < 0.01 && (z4 - 3.5).abs() < 0.01,
            "center=({x4},{z4})"
        );
    }

    #[test]
    fn non_grabbable_box_is_fixed() {
        let entries = vec![SceneEntry {
            entity_id: 20,
            blake3: String::new(),
            x: 0.0,
            y: 0.25,
            z: 0.0,
            half_extent: 0.25,
            flags: 0,
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        }];
        let mut world = PhysicsWorld::from_entries(&entries, LocoMode::Noclip, DEFAULT_MOVE_SPEED);
        assert_eq!(world.boxes.len(), 1);
        assert!(!world.boxes.grabbable[0]);
        let (_, y0, _) = world.box_translation(0);
        for _ in 0..120 {
            world.step();
        }
        let (_, y1, _) = world.box_translation(0);
        assert!(
            (y1 - y0).abs() < 1e-4,
            "Fixed non-Grab box must not settle under gravity: {y0} → {y1}"
        );
    }

    #[test]
    fn boxes_rest_on_floor_after_settle() {
        let mut world = test_world();
        for _ in 0..300 {
            world.step();
        }
        for i in 0..world.boxes.len() {
            let (_, y, _) = world.box_translation(i);
            assert!(y > 0.15 && y < 0.6, "box {i} expected on floor, y={y}");
        }
    }

    #[test]
    fn grab_hold_follows_aim_then_drops() {
        let mut world = test_world();
        for _ in 0..120 {
            world.step();
        }
        let id = 14u32;
        let (bx, by, bz) = world.box_translation(4);
        let aim0 = (bx - 0.1, by, bz - 0.2);
        world.apply_grab(0, true, id, aim0);
        assert_eq!(world.grab_entity_id(0), Some(id));
        let aim1 = (aim0.0 + 3.0, aim0.1 + 2.5, aim0.2);
        for _ in 0..10 {
            world.apply_grab(0, true, id, aim1);
            world.step();
        }
        let (x, y, z) = world.box_translation(4);
        assert!(
            (x - (bx + 3.0)).abs() < 0.05 && (y - (by + 2.5)).abs() < 0.05 && (z - bz).abs() < 0.05,
            "expected follow, got ({x},{y},{z})"
        );
        world.apply_grab(0, false, 0, aim1);
        assert!(world.grab_entity_id(0).is_none());
        for _ in 0..300 {
            world.step();
        }
        let (_, y_after, _) = world.box_translation(4);
        assert!(y_after < 1.0 && y_after < y - 1.0);
    }

    #[test]
    fn grab_ignores_non_box_entity() {
        let mut world = test_world();
        world.apply_grab(0, true, 1, (0.0, 1.0, 0.0));
        assert!(world.grab_entity_id(0).is_none());
    }

    #[test]
    fn release_slot_clears_grab_and_resets_spawn() {
        let mut world =
            PhysicsWorld::from_entries(&demo_entries_for_test(), LocoMode::Noclip, 10.0);
        for _ in 0..30 {
            world.step();
        }
        let id = 14u32;
        let (bx, by, bz) = world.box_translation(4);
        world.apply_grab(0, true, id, (bx, by, bz));
        assert_eq!(world.grab_entity_id(0), Some(id));
        let dt = 1.0 / 60.0;
        for _ in 0..60 {
            world.apply_user_intent(0, 1.0, 0.0, 0.0, dt);
            world.apply_grab(0, true, id, (bx + 1.0, by, bz));
            world.step();
        }
        world.release_slot(0);
        assert!(world.grab_entity_id(0).is_none());
        let (x, y, z) = world.user_root_translation(0);
        let (sx, sy, sz) = world.user_root_spawns[0];
        assert!((x - sx).abs() < 0.01 && (y - sy).abs() < 0.01 && (z - sz).abs() < 0.01);
    }

    #[test]
    fn grab_no_steal_from_other_slot() {
        let mut world = test_world();
        for _ in 0..60 {
            world.step();
        }
        let id = 14u32;
        let (bx, by, bz) = world.box_translation(4);
        world.apply_grab(0, true, id, (bx, by, bz));
        world.apply_grab(1, true, id, (bx + 1.0, by, bz));
        assert!(world.grab_entity_id(1).is_none());
        assert_eq!(world.grab_entity_id(0), Some(id));
    }

    #[test]
    fn grab_start_rejects_when_player_far() {
        let mut world =
            PhysicsWorld::from_entries(&demo_entries_for_test(), LocoMode::Noclip, 20.0);
        let dt = 1.0 / 60.0;
        for _ in 0..120 {
            world.apply_user_intent(0, 1.0, 0.0, 0.0, dt);
            world.step();
        }
        let id = 14u32;
        let (bx, by, bz) = world.box_translation(4);
        let (ux, uy, uz) = world.user_root_translation(0);
        assert!(!crate::finite::within_grab_start_distance(
            (ux, uy, uz),
            (bx, by, bz)
        ));
        world.apply_grab(0, true, id, (bx, by, bz));
        assert!(world.grab_entity_id(0).is_none());
    }

    #[test]
    fn huge_move_magnitude_does_not_teleport() {
        let mut world = test_world();
        let (x0, y0, z0) = world.user_root_translation(0);
        let dt = 1.0 / 60.0;
        world.apply_user_intent(0, 1e30, 0.0, 0.0, dt);
        world.step();
        let (x1, y1, z1) = world.user_root_translation(0);
        let dx = (x1 - x0).abs();
        assert!(
            dx < DEFAULT_MOVE_SPEED * dt * 1.1,
            "expected clamped step, dx={dx} from ({x0},{y0},{z0}) to ({x1},{y1},{z1})"
        );
    }
}
