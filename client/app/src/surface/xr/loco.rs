//! VR thumbstick → world MoveIntent helpers (x000a).
//!
//! Binding (both hands identical):
//! - Stick Y + controller aim → move (forward/back along aim; L+R add)
//! - Stick X → comfort turn (see `crate::turn`; L+R add, clamped)
//!
//! Comfort yaw: stage aim → world Intent via `R(-yaw)` so it matches the yawed floor.
//! Non-guarantees: OpenXR action binding (see vk_loop), host speed override protocol.

use glam::{Quat, Vec3};

/// Default stick deadzone (per-axis for Y move / used by turn module for X).
pub const STICK_DEADZONE: f32 = 0.15;
/// Two hands can stack move Intent up to this length (same direction adds).
pub const MAX_MOVE_FROM_HANDS: f32 = 2.0;

fn clamp_len(v: Vec3, max: f32) -> Vec3 {
    let l = v.length();
    if l > max && l > 1e-6 {
        v * (max / l)
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

/// Stick Y only → move along controller aim (no strafe from X).
pub fn stick_y_to_world(orientation: Quat, stick_y: f32, deadzone: f32) -> Vec3 {
    stick_to_world(orientation, 0.0, stick_y, deadzone)
}

/// Add left and right hand world vectors.
/// Same direction stacks (up to [`MAX_MOVE_FROM_HANDS`]); opposite directions cancel.
pub fn combine_hands(left: Vec3, right: Vec3) -> Vec3 {
    clamp_len(left + right, MAX_MOVE_FROM_HANDS)
}

/// Sum both sticks' X for turn, clamp to [-1, 1].
pub fn combine_turn_axes(left_x: f32, right_x: f32) -> f32 {
    (left_x + right_x).clamp(-1.0, 1.0)
}

/// Scale by `max_speed / default_speed`; keep two-hand stack up to [`MAX_MOVE_FROM_HANDS`].
pub fn apply_max_speed_scale(v: Vec3, max_speed: f32, default_speed: f32) -> Vec3 {
    if !max_speed.is_finite() || !default_speed.is_finite() || default_speed <= 0.0 {
        return Vec3::ZERO;
    }
    let factor = (max_speed / default_speed).clamp(0.0, 1.0);
    clamp_len(v * factor, MAX_MOVE_FROM_HANDS)
}

/// Both hands: Y → aim-relative move (additive); X is ignored here (turn is separate).
pub fn intent_from_sticks(
    left_ori: Quat,
    left_stick: (f32, f32),
    right_ori: Quat,
    right_stick: (f32, f32),
    max_speed: f32,
    default_speed: f32,
    deadzone: f32,
) -> Vec3 {
    let l = stick_y_to_world(left_ori, left_stick.1, deadzone);
    let r = stick_y_to_world(right_ori, right_stick.1, deadzone);
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
    fn combine_hands_same_direction_stacks() {
        let c = combine_hands(Vec3::NEG_Z, Vec3::NEG_Z);
        assert!((c.length() - 2.0).abs() < 1e-5, "got {c:?}");
        assert!((c.z + 2.0).abs() < 1e-5);
    }

    #[test]
    fn combine_hands_opposite_cancels() {
        let c = combine_hands(Vec3::NEG_Z, Vec3::Z);
        assert!(c.length() < 1e-5, "got {c:?}");
    }

    #[test]
    fn combine_hands_orthogonal_diagonal() {
        let c = combine_hands(Vec3::X, Vec3::NEG_Z);
        assert!((c.length() - std::f32::consts::SQRT_2).abs() < 1e-4);
    }

    #[test]
    fn max_speed_half_scales() {
        let v = apply_max_speed_scale(Vec3::NEG_Z, 2.0, 4.0);
        assert!((v.z + 0.5).abs() < 1e-5);
    }

    #[test]
    fn both_full_forward_reaches_two() {
        let v = intent_from_sticks(
            Quat::IDENTITY,
            (0.0, 1.0),
            Quat::IDENTITY,
            (0.0, 1.0),
            4.0,
            4.0,
            0.0,
        );
        assert!(
            (v.z + 2.0).abs() < 1e-4,
            "two hands same way should stack to 2, got {v:?}"
        );
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
    fn intent_ignores_stick_x_both_hands() {
        let v = intent_from_sticks(
            Quat::IDENTITY,
            (1.0, 0.0),
            Quat::IDENTITY,
            (-1.0, 0.0),
            4.0,
            4.0,
            0.0,
        );
        assert!(v.length() < 1e-5, "stick X must not strafe, got {v:?}");
    }

    #[test]
    fn both_hands_y_add_along_aim() {
        let v = intent_from_sticks(
            Quat::IDENTITY,
            (0.0, 0.5),
            Quat::IDENTITY,
            (0.0, 0.5),
            4.0,
            4.0,
            0.0,
        );
        // 0.5+0.5 = 1.0 forward (−Z)
        assert!(
            (v.z + 1.0).abs() < 1e-4,
            "expected stacked 1.0 forward, got {v:?}"
        );
    }

    #[test]
    fn combine_turn_axes_sums_and_clamps() {
        assert!((combine_turn_axes(0.4, 0.4) - 0.8).abs() < 1e-5);
        assert!((combine_turn_axes(1.0, 1.0) - 1.0).abs() < 1e-5);
        assert!((combine_turn_axes(-0.7, -0.7) + 1.0).abs() < 1e-5);
    }

    #[test]
    fn stage_to_world_inverse_matches_floor_yaw() {
        let stage = Vec3::NEG_Z;
        let world = stage_move_to_world(stage, std::f32::consts::PI);
        assert!(
            (world.z - 1.0).abs() < 1e-4 && world.x.abs() < 1e-4,
            "expected +Z after 180°, got {world:?}"
        );
        let world90 = stage_move_to_world(stage, std::f32::consts::FRAC_PI_2);
        assert!(
            (world90.x - 1.0).abs() < 1e-4 && world90.z.abs() < 1e-4,
            "expected +X after +90° yaw, got {world90:?}"
        );
    }
}
