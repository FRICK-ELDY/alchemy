//! VR thumbstick → world MoveIntent helpers (x000a).
//!
//! Guarantees: aim-relative stick projection, deadzone, L+R add+clamp, max-speed scale.
//! Right stick X is reserved for comfort turn (see `crate::turn`); only Y adds to move.
//! Comfort yaw: stage aim → world Intent via `R(-yaw)` so it matches the yawed floor.
//! Non-guarantees: OpenXR action binding (see vk_loop), host speed override protocol.

use glam::{Quat, Vec3};

/// Default stick deadzone (radial).
pub const STICK_DEADZONE: f32 = 0.15;

fn clamp_unit(v: Vec3) -> Vec3 {
    let l = v.length();
    if l > 1.0 {
        v / l
    } else {
        v
    }
}

/// Project thumbstick into world axes using controller aim orientation.
/// Stick: +X = right, +Y = forward (−local Z in OpenXR). Pitch included (Noclip up/down).
pub fn stick_to_world(orientation: Quat, stick_x: f32, stick_y: f32, deadzone: f32) -> Vec3 {
    let mag = (stick_x * stick_x + stick_y * stick_y).sqrt();
    if !mag.is_finite() || mag <= deadzone.max(0.0) || mag < 1e-6 {
        return Vec3::ZERO;
    }
    let t = ((mag - deadzone) / (1.0 - deadzone).max(1e-6)).clamp(0.0, 1.0);
    let nx = stick_x / mag * t;
    let ny = stick_y / mag * t;

    let right = orientation * Vec3::X;
    let forward = orientation * Vec3::NEG_Z;
    right * nx + forward * ny
}

/// Add left and right hand world vectors; clamp length to ≤ 1.
pub fn combine_hands(left: Vec3, right: Vec3) -> Vec3 {
    clamp_unit(left + right)
}

/// Scale by `max_speed / default_speed` then re-clamp to unit length.
pub fn apply_max_speed_scale(v: Vec3, max_speed: f32, default_speed: f32) -> Vec3 {
    if !max_speed.is_finite() || !default_speed.is_finite() || default_speed <= 0.0 {
        return Vec3::ZERO;
    }
    let factor = (max_speed / default_speed).clamp(0.0, 1.0);
    clamp_unit(v * factor)
}

/// Left stick = full 2D move; right stick Y = forward/back only (X reserved for turn).
/// When `|right_x| >= turn_deadzone`, right stick is turn-only (no move add).
pub fn intent_from_sticks(
    left_ori: Quat,
    left_stick: (f32, f32),
    right_ori: Quat,
    right_stick: (f32, f32),
    max_speed: f32,
    default_speed: f32,
    deadzone: f32,
    turn_deadzone: f32,
) -> Vec3 {
    let l = stick_to_world(left_ori, left_stick.0, left_stick.1, deadzone);
    let r = if right_stick.0.abs() >= turn_deadzone {
        Vec3::ZERO
    } else {
        stick_to_world(right_ori, 0.0, right_stick.1, deadzone)
    };
    apply_max_speed_scale(combine_hands(l, r), max_speed, default_speed)
}

/// Floor is drawn as `R(yaw) * (world - user_root)`. Intent must use the inverse so that
/// pushing the stick along the controller in the headset moves that way on the floor.
pub fn stage_move_to_world(stage_move: Vec3, view_yaw_rad: f32) -> Vec3 {
    Quat::from_rotation_y(-view_yaw_rad) * stage_move
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_forward_stick_goes_neg_z() {
        let v = stick_to_world(Quat::IDENTITY, 0.0, 1.0, 0.0);
        assert!(v.x.abs() < 1e-5);
        assert!(v.y.abs() < 1e-5);
        assert!((v.z + 1.0).abs() < 1e-5);
    }

    #[test]
    fn identity_right_stick_goes_pos_x() {
        let v = stick_to_world(Quat::IDENTITY, 1.0, 0.0, 0.0);
        assert!((v.x - 1.0).abs() < 1e-5);
        assert!(v.y.abs() < 1e-5);
        assert!(v.z.abs() < 1e-5);
    }

    #[test]
    fn deadzone_zeros_small_tilt() {
        let v = stick_to_world(Quat::IDENTITY, 0.05, 0.05, STICK_DEADZONE);
        assert_eq!(v, Vec3::ZERO);
    }

    #[test]
    fn combine_hands_adds_then_clamps() {
        let c = combine_hands(Vec3::X, Vec3::X);
        assert!((c.length() - 1.0).abs() < 1e-5);
        assert!((c.x - 1.0).abs() < 1e-5);
    }

    #[test]
    fn combine_hands_orthogonal_diagonal() {
        let c = combine_hands(Vec3::X, Vec3::NEG_Z);
        assert!((c.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn max_speed_half_scales() {
        let v = apply_max_speed_scale(Vec3::NEG_Z, 2.0, 4.0);
        assert!((v.z + 0.5).abs() < 1e-5);
    }

    #[test]
    fn max_speed_equal_default_identity() {
        let v = apply_max_speed_scale(Vec3::new(0.0, 0.0, -0.8), 4.0, 4.0);
        assert!((v.z + 0.8).abs() < 1e-5);
    }

    #[test]
    fn pitched_controller_moves_up() {
        let q = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        let v = stick_to_world(q, 0.0, 1.0, 0.0);
        assert!(v.y > 0.9, "expected +Y, got {v:?}");
    }

    #[test]
    fn intent_ignores_right_stick_x() {
        let v = intent_from_sticks(
            Quat::IDENTITY,
            (0.0, 0.0),
            Quat::IDENTITY,
            (1.0, 0.0),
            4.0,
            4.0,
            0.0,
            0.15,
        );
        assert!(v.length() < 1e-5, "right X must not strafe, got {v:?}");
    }

    #[test]
    fn turn_deadzone_suppresses_right_move() {
        let v = intent_from_sticks(
            Quat::IDENTITY,
            (0.0, 0.0),
            Quat::IDENTITY,
            (0.5, 1.0),
            4.0,
            4.0,
            0.0,
            0.15,
        );
        assert!(
            v.length() < 1e-5,
            "turning right stick must not add move, got {v:?}"
        );
    }

    #[test]
    fn stage_to_world_inverse_matches_floor_yaw() {
        // Floor uses R(+yaw). After 180° yaw, stage forward (−Z) must become world +Z
        // so visual slide matches controller-forward on the rotated floor.
        let stage = Vec3::NEG_Z;
        let world = stage_move_to_world(stage, std::f32::consts::PI);
        assert!(
            (world.z - 1.0).abs() < 1e-4 && world.x.abs() < 1e-4,
            "expected +Z after 180°, got {world:?}"
        );
        let world90 = stage_move_to_world(stage, std::f32::consts::FRAC_PI_2);
        // R_y(-90°) * (0,0,-1) = (+1, 0, 0)
        assert!(
            (world90.x - 1.0).abs() < 1e-4 && world90.z.abs() < 1e-4,
            "expected +X after +90° yaw, got {world90:?}"
        );
    }
}
