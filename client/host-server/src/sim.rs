//! Rapier world + user_root locomotion + grabbable boxes (x0010).
//!
//! Guarantees: 60Hz-capable step; floor + coin + 3×3 cuboids (SoA grabbable); user_root Noclip.
//! Non-guarantees: Grab hold (x0011), Slot/Component runtime, polished Walk.

use alchemy_protocol::LocomotionMode;
use rapier3d::prelude::*;

pub use alchemy_protocol::DEFAULT_MOVE_SPEED;

const FLOOR_HALF_H: f32 = 0.5;
const BALL_RADIUS: f32 = 0.25;
/// High enough that free-fall is obvious on screen.
pub const SPAWN_Y: f32 = 5.0;
/// Demo gravity (weaker than Earth) so the drop lasts ~2s and is easy to see.
const DEMO_GRAVITY_Y: f32 = -2.2;
/// user_root spawn (eye height-ish above floor).
pub const USER_SPAWN: (f32, f32, f32) = (0.0, 1.6, 6.0);

/// First entity id for the 3×3 grabbable grid (10..18).
pub const BOX_ENTITY_FIRST: u32 = 10;
pub const BOX_COUNT: usize = 9;
/// Half-extent of each cuboid (meters). Full size = 0.5m.
pub const BOX_HALF_EXTENT: f32 = 0.25;
/// Center-to-center spacing on XZ (meters).
const BOX_SPACING: f32 = 0.8;
/// Grid center on XZ (toward -Z from user spawn so boxes are in view).
const BOX_GRID_CENTER: (f32, f32) = (0.0, 0.0);

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
    /// Authoritative player root (kinematic; Noclip ignores collision).
    user_root: RigidBodyHandle,
    pub boxes: GrabbableBoxes,
    loco_mode: LocoMode,
    move_speed: f32,
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

        let (ux, uy, uz) = USER_SPAWN;
        let user = RigidBodyBuilder::kinematic_position_based()
            .translation(vector![ux, uy, uz])
            .build();
        let user_root = rigid_body_set.insert(user);
        // Sensor collider so Noclip never blocks; Walk/Fly may use later.
        collider_set.insert_with_parent(
            ColliderBuilder::ball(0.3).sensor(true).build(),
            user_root,
            &mut rigid_body_set,
        );

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
            user_root,
            boxes,
            loco_mode,
            move_speed: speed,
        }
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

    /// Integrate user_root from camera-relative intent (move_* in ~-1..1).
    /// Intent axes: +X right, +Y up, +Z forward (already world-projected by client for XZ).
    pub fn apply_user_intent(&mut self, mx: f32, my: f32, mz: f32, dt: f32) {
        let speed = self.move_speed;
        let Some(body) = self.rigid_body_set.get_mut(self.user_root) else {
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

    pub fn user_root_translation(&self) -> (f32, f32, f32) {
        self.rigid_body_set
            .get(self.user_root)
            .map(|b| {
                let t = b.translation();
                (t.x, t.y, t.z)
            })
            .unwrap_or(USER_SPAWN)
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
            world.apply_user_intent(0.0, -1.0, 0.0, dt);
            world.step();
        }
        let (_, y, _) = world.user_root_translation();
        assert!(y < 0.0, "noclip should pass below floor, got y={y}");
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
        assert!((x4).abs() < 0.01 && (z4).abs() < 0.01);
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
}
