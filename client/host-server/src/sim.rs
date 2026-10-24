//! Minimal Rapier world for Slice D (host-local module; not a shared-sim crate yet).
//!
//! Guarantees: 60Hz-capable step; floor + one dynamic body; f32 state.
//! Non-guarantees: character controller, joints, Formula.

use rapier3d::prelude::*;

const FLOOR_HALF_H: f32 = 0.5;
const BALL_RADIUS: f32 = 0.25;
/// High enough that free-fall is obvious on screen.
pub const SPAWN_Y: f32 = 5.0;
/// Demo gravity (weaker than Earth) so the drop lasts ~2s and is easy to see.
const DEMO_GRAVITY_Y: f32 = -2.2;

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
}

impl PhysicsWorld {
    pub fn new() -> Self {
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
        }
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

    /// Apply finite horizontal input as linvel XZ; keep current Y so gravity still works.
    pub fn apply_input_horizontal(&mut self, vx: f32, vz: f32) {
        if let Some(body) = self.rigid_body_set.get_mut(self.moving) {
            let vy = body.linvel().y;
            body.set_linvel(vector![vx, vy, vz], true);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ball_falls_onto_floor() {
        let mut world = PhysicsWorld::new();
        let (_, y0, _) = world.moving_translation();
        assert!(y0 > 4.5, "spawn above floor");

        for _ in 0..600 {
            world.step();
        }
        let (_, y, _) = world.moving_translation();
        assert!(y > 0.1 && y < 1.0, "expected resting on floor, got y={y}");
    }
}
