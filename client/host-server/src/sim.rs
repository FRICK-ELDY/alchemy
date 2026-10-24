//! Rapier world + user_root locomotion (x0009).
//!
//! Guarantees: 60Hz-capable step; floor + coin dynamic body; user_root Noclip integrate.
//! Non-guarantees: polished Walk character controller, joints, Formula.

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

        let gravity_y = match loco_mode {
            LocoMode::Walk => DEMO_GRAVITY_Y,
            LocoMode::Noclip | LocoMode::Fly => 0.0,
        };

        Self {
            gravity: vector![0.0, gravity_y, 0.0],
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
}
