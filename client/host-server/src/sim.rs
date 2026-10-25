//! Rapier world + dual user_root (x0014) + grabbable boxes + Grab hold.
//!
//! Guarantees: 60Hz-capable step; floor + coin + 3×3 cuboids (SoA grabbable);
//! up to 2 kinematic user_roots (entity 3/4); per-slot Grab; no Steal.
//! Non-guarantees: rotation follow, throw impulse, Slot/Component, polished Walk.

use alchemy_protocol::LocomotionMode;
use rapier3d::prelude::*;

pub use alchemy_protocol::DEFAULT_MOVE_SPEED;

const FLOOR_HALF_H: f32 = 0.5;
const BALL_RADIUS: f32 = 0.25;
/// High enough that free-fall is obvious on screen.
pub const SPAWN_Y: f32 = 5.0;
/// Demo gravity (weaker than Earth) so the drop lasts ~2s and is easy to see.
const DEMO_GRAVITY_Y: f32 = -2.2;
/// user_root spawn (eye height-ish above floor). Slot 1 is offset on +X.
pub const USER_SPAWN: [(f32, f32, f32); USER_ROOT_SLOTS] = [(0.0, 1.6, 6.0), (1.5, 1.6, 6.0)];

/// First entity id for the 3×3 grabbable grid (10..18).
pub const BOX_ENTITY_FIRST: u32 = 10;
pub const BOX_COUNT: usize = 9;
/// Half-extent of each cuboid (meters). Full size = 0.5m.
pub const BOX_HALF_EXTENT: f32 = 0.25;
/// Center-to-center spacing on XZ (meters).
const BOX_SPACING: f32 = 0.8;
/// Grid center on XZ — ~2.5 m in front of [`USER_SPAWN`] (z=6 → z=3.5) so XR facing -Z sees them.
const BOX_GRID_CENTER: (f32, f32) = (0.0, 3.5);

/// Max simultaneous players (x0014). Entity ids = [`USER_ROOT_ENTITY_FIRST`] .. +SLOTS-1.
pub const USER_ROOT_SLOTS: usize = 2;
pub const USER_ROOT_ENTITY_FIRST: u32 = 3;

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
            // UNSPECIFIED and NOCLIP → Noclip (world default).
            LocomotionMode::Noclip | LocomotionMode::Unspecified => Self::Noclip,
        }
    }
}

/// Structure-of-arrays for the 3×3 grabbable cuboids (x0010).
pub struct GrabbableBoxes {
    pub handles: [RigidBodyHandle; BOX_COUNT],
    pub entity_ids: [u32; BOX_COUNT],
    pub grabbable: [bool; BOX_COUNT],
    pub half_extent: f32,
}

impl GrabbableBoxes {
    fn spawn(rigid_body_set: &mut RigidBodySet, collider_set: &mut ColliderSet) -> Self {
        let mut handles = Vec::with_capacity(BOX_COUNT);
        let mut entity_ids = [0u32; BOX_COUNT];
        let mut grabbable = [false; BOX_COUNT];
        let he = BOX_HALF_EXTENT;
        let (cx, cz) = BOX_GRID_CENTER;
        // Row-major: i = row*3+col, row/col in {0,1,2}, centered on (cx, cz).
        for i in 0..BOX_COUNT {
            let row = (i / 3) as f32;
            let col = (i % 3) as f32;
            let x = cx + (col - 1.0) * BOX_SPACING;
            let z = cz + (row - 1.0) * BOX_SPACING;
            let y = he; // rest on floor top (y=0)
            let body = RigidBodyBuilder::dynamic()
                .translation(vector![x, y, z])
                .build();
            let handle = rigid_body_set.insert(body);
            collider_set.insert_with_parent(
                ColliderBuilder::cuboid(he, he, he)
                    .restitution(0.05)
                    .build(),
                handle,
                rigid_body_set,
            );
            handles.push(handle);
            entity_ids[i] = BOX_ENTITY_FIRST + i as u32;
            grabbable[i] = true;
        }
        Self {
            handles: handles.try_into().expect("BOX_COUNT handles"),
            entity_ids,
            grabbable,
            half_extent: he,
        }
    }
}

/// Active Grab hold for one user_root slot (x0011 / x0014).
#[derive(Clone, Copy, Debug)]
struct ActiveGrab {
    index: usize,
    /// World offset frozen at grab start: `box_translation - aim`.
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
    moving: RigidBodyHandle,
    /// Authoritative player roots (kinematic; Noclip ignores collision).
    user_roots: [RigidBodyHandle; USER_ROOT_SLOTS],
    pub boxes: GrabbableBoxes,
    loco_mode: LocoMode,
    move_speed: f32,
    grabs: [Option<ActiveGrab>; USER_ROOT_SLOTS],
}

impl PhysicsWorld {
    pub fn new(loco_mode: LocoMode, move_speed: f32) -> Self {
        let mut rigid_body_set = RigidBodySet::new();
        let mut collider_set = ColliderSet::new();

        // Floor top surface at y = 0.
        let floor = RigidBodyBuilder::fixed()
            .translation(vector![0.0, -FLOOR_HALF_H, 0.0])
            .build();
        let floor_handle = rigid_body_set.insert(floor);
        collider_set.insert_with_parent(
            ColliderBuilder::cuboid(20.0, FLOOR_HALF_H, 20.0).build(),
            floor_handle,
            &mut rigid_body_set,
        );

        let ball = RigidBodyBuilder::dynamic()
            .translation(vector![0.0, SPAWN_Y, 0.0])
            .linvel(vector![0.1, 0.0, 0.05])
            .build();
        let moving = rigid_body_set.insert(ball);
        collider_set.insert_with_parent(
            ColliderBuilder::ball(BALL_RADIUS).restitution(0.15).build(),
            moving,
            &mut rigid_body_set,
        );

        let boxes = GrabbableBoxes::spawn(&mut rigid_body_set, &mut collider_set);

        let mut user_root_handles = Vec::with_capacity(USER_ROOT_SLOTS);
        for spawn in USER_SPAWN {
            let (ux, uy, uz) = spawn;
            let user = RigidBodyBuilder::kinematic_position_based()
                .translation(vector![ux, uy, uz])
                .build();
            let handle = rigid_body_set.insert(user);
            // Sensor collider so Noclip never blocks; Walk/Fly may use later.
            collider_set.insert_with_parent(
                ColliderBuilder::ball(0.3).sensor(true).build(),
                handle,
                &mut rigid_body_set,
            );
            user_root_handles.push(handle);
        }
        let user_roots: [RigidBodyHandle; USER_ROOT_SLOTS] = user_root_handles
            .try_into()
            .expect("USER_ROOT_SLOTS handles");

        let speed = if move_speed > 0.0 {
            move_speed
        } else {
            DEFAULT_MOVE_SPEED
        };

        // World dynamics always feel gravity; user_root is kinematic so Noclip is unaffected.
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
            moving,
            user_roots,
            boxes,
            loco_mode,
            move_speed: speed,
            grabs: [None; USER_ROOT_SLOTS],
        }
    }

    pub fn user_root_entity_id(slot: usize) -> u32 {
        USER_ROOT_ENTITY_FIRST + slot as u32
    }

    fn entity_index(&self, entity_id: u32) -> Option<usize> {
        let first = BOX_ENTITY_FIRST;
        let last = first + BOX_COUNT as u32 - 1;
        if !(first..=last).contains(&entity_id) {
            return None;
        }
        let i = (entity_id - first) as usize;
        if self.boxes.grabbable[i] {
            Some(i)
        } else {
            None
        }
    }

    fn box_held_by_other(&self, slot: usize, box_index: usize) -> bool {
        self.grabs
            .iter()
            .enumerate()
            .any(|(s, g)| s != slot && g.map(|ag| ag.index == box_index).unwrap_or(false))
    }

    /// Begin / hold / release Grab for one user_root slot. Steal is ignored (x0014).
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
            // Hold: keep following even if client stops sending id (defensive).
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
        let Some(body) = self.rigid_body_set.get_mut(handle) else {
            return;
        };
        let t = *body.translation();
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
        // Snapshot pose before type change so dynamic starts at the held position.
        let pos = *body.position();
        body.set_body_type(RigidBodyType::Dynamic, true);
        body.set_position(pos, true);
        body.set_linvel(vector![0.0, 0.0, 0.0], true);
        body.set_angvel(vector![0.0, 0.0, 0.0], true);
        body.wake_up(true);
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

    /// Integrate one user_root from camera-relative intent (move_* in ~-1..1).
    pub fn apply_user_intent(&mut self, slot: usize, mx: f32, my: f32, mz: f32, dt: f32) {
        if slot >= USER_ROOT_SLOTS {
            return;
        }
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
                // Minimal: horizontal only; Y from gravity on coin path — user_root stays kinematic XZ.
                let dx = mx * speed * dt;
                let dz = mz * speed * dt;
                body.set_next_kinematic_translation(vector![t.x + dx, t.y, t.z + dz]);
            }
        }
    }

    pub fn moving_translation(&self) -> (f32, f32, f32) {
        self.rigid_body_set
            .get(self.moving)
            .map(|b| {
                let t = b.translation();
                (t.x, t.y, t.z)
            })
            .unwrap_or((0.0, SPAWN_Y, 0.0))
    }

    pub fn user_root_translation(&self, slot: usize) -> (f32, f32, f32) {
        let fallback = USER_SPAWN.get(slot).copied().unwrap_or(USER_SPAWN[0]);
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
        self.rigid_body_set
            .get(self.boxes.handles[index])
            .map(|b| {
                let t = b.translation();
                (t.x, t.y, t.z)
            })
            .unwrap_or((0.0, BOX_HALF_EXTENT, 0.0))
    }
}

/// Stable tint per box index (distinct on Desktop / XR).
pub fn box_color(index: usize) -> (f32, f32, f32, f32) {
    const PALETTE: [(f32, f32, f32); BOX_COUNT] = [
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
    let (r, g, b) = PALETTE[index % BOX_COUNT];
    (r, g, b, 1.0)
}

/// Distinct tint per user_root slot (remote avatar placeholder).
pub fn user_root_color(slot: usize) -> (f32, f32, f32, f32) {
    match slot {
        0 => (0.4, 0.7, 1.0, 1.0),
        _ => (1.0, 0.55, 0.25, 1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ball_falls_onto_floor() {
        let mut world = PhysicsWorld::new(LocoMode::Walk, DEFAULT_MOVE_SPEED);
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
        let mut world = PhysicsWorld::new(LocoMode::Noclip, 10.0);
        let dt = 1.0 / 60.0;
        // Drive straight down through the floor plane.
        for _ in 0..120 {
            world.apply_user_intent(0, 0.0, -1.0, 0.0, dt);
            world.step();
        }
        let (_, y, _) = world.user_root_translation(0);
        assert!(y < 0.0, "noclip should pass below floor, got y={y}");
    }

    #[test]
    fn two_slots_move_independently() {
        let mut world = PhysicsWorld::new(LocoMode::Noclip, 10.0);
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
    fn unspecified_maps_to_noclip() {
        assert_eq!(LocoMode::from_proto(0), LocoMode::Noclip);
        assert_eq!(LocoMode::from_proto(1), LocoMode::Noclip);
    }

    #[test]
    fn boxes_spawn_as_3x3_grabbable() {
        let world = PhysicsWorld::new(LocoMode::Noclip, DEFAULT_MOVE_SPEED);
        assert_eq!(world.boxes.entity_ids[0], BOX_ENTITY_FIRST);
        assert_eq!(
            world.boxes.entity_ids[BOX_COUNT - 1],
            BOX_ENTITY_FIRST + (BOX_COUNT as u32) - 1
        );
        assert!(world.boxes.grabbable.iter().all(|&g| g));
        assert!((world.boxes.half_extent - BOX_HALF_EXTENT).abs() < 1e-6);
        let (x0, y0, z0) = world.box_translation(0);
        let (x4, y4, z4) = world.box_translation(4); // center
        let (x8, _, z8) = world.box_translation(8);
        assert!((y0 - BOX_HALF_EXTENT).abs() < 0.01);
        assert!((y4 - BOX_HALF_EXTENT).abs() < 0.01);
        assert!(
            (x4).abs() < 0.01 && (z4 - BOX_GRID_CENTER.1).abs() < 0.01,
            "center=({x4},{z4})"
        );
        assert!((x8 - x0).abs() > 1.0 || (z8 - z0).abs() > 1.0);
    }

    #[test]
    fn boxes_rest_on_floor_after_settle() {
        let mut world = PhysicsWorld::new(LocoMode::Noclip, DEFAULT_MOVE_SPEED);
        for _ in 0..300 {
            world.step();
        }
        for i in 0..BOX_COUNT {
            let (_, y, _) = world.box_translation(i);
            assert!(y > 0.15 && y < 0.6, "box {i} expected on floor, y={y}");
        }
    }

    #[test]
    fn grab_hold_follows_aim_then_drops() {
        let mut world = PhysicsWorld::new(LocoMode::Noclip, DEFAULT_MOVE_SPEED);
        for _ in 0..120 {
            world.step();
        }
        let id = BOX_ENTITY_FIRST + 4; // center
        let (bx, by, bz) = world.box_translation(4);
        let aim0 = (bx - 0.1, by, bz - 0.2);
        world.apply_grab(0, true, id, aim0);
        assert_eq!(world.grab_entity_id(0), Some(id));
        // Lift high and clear of the 3×3 grid so release falls to the floor.
        let aim1 = (aim0.0 + 3.0, aim0.1 + 2.5, aim0.2);
        for _ in 0..10 {
            world.apply_grab(0, true, id, aim1);
            world.step();
        }
        let (x, y, z) = world.box_translation(4);
        assert!(
            (x - (bx + 3.0)).abs() < 0.05 && (y - (by + 2.5)).abs() < 0.05 && (z - bz).abs() < 0.05,
            "expected follow to ({},{},{}), got ({x},{y},{z})",
            bx + 3.0,
            by + 2.5,
            bz
        );
        world.apply_grab(0, false, 0, aim1);
        assert!(world.grab_entity_id(0).is_none());
        for _ in 0..300 {
            world.step();
        }
        let (_, y_after, _) = world.box_translation(4);
        assert!(
            y_after < 1.0 && y_after < y - 1.0,
            "released box should fall toward floor, y_hold={y} y_after={y_after}"
        );
    }

    #[test]
    fn grab_ignores_non_box_entity() {
        let mut world = PhysicsWorld::new(LocoMode::Noclip, DEFAULT_MOVE_SPEED);
        world.apply_grab(0, true, 1, (0.0, 1.0, 0.0));
        assert!(world.grab_entity_id(0).is_none());
    }

    #[test]
    fn grab_no_steal_from_other_slot() {
        let mut world = PhysicsWorld::new(LocoMode::Noclip, DEFAULT_MOVE_SPEED);
        for _ in 0..60 {
            world.step();
        }
        let id = BOX_ENTITY_FIRST + 4;
        let (bx, by, bz) = world.box_translation(4);
        world.apply_grab(0, true, id, (bx, by, bz));
        assert_eq!(world.grab_entity_id(0), Some(id));
        world.apply_grab(1, true, id, (bx + 1.0, by, bz));
        assert!(world.grab_entity_id(1).is_none());
        assert_eq!(world.grab_entity_id(0), Some(id));
    }
}
