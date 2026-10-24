//! VR thumbstick → world MoveIntent helpers (x000a).
//!
//! Guarantees: aim-relative stick projection, deadzone, L+R add+clamp, max-speed scale.
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
    if !mag.is_finite() || mag < deadzone {
        return Vec3::ZERO;
    }
    // Remap deadzone → full throw so small tilts still reach 1.0 at edge.
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
/// When max == default, this is identity on an already-unit vector.
pub fn apply_max_speed_scale(v: Vec3, max_speed: f32, default_speed: f32) -> Vec3 {
    if !max_speed.is_finite() || !default_speed.is_finite() || default_speed <= 0.0 {
        return Vec3::ZERO;
    }
    let factor = (max_speed / default_speed).clamp(0.0, 1.0);
    clamp_unit(v * factor)
}

/// Full pipeline: two sticks → scaled Intent axes.
pub fn intent_from_sticks(
    left_ori: Quat,
    left_stick: (f32, f32),
    right_ori: Quat,
    right_stick: (f32, f32),
    max_speed: f32,
    default_speed: f32,
    deadzone: f32,
) -> Vec3 {
    let l = stick_to_world(left_ori, left_stick.0, left_stick.1, deadzone);
    let r = stick_to_world(right_ori, right_stick.0, right_stick.1, deadzone);
    apply_max_speed_scale(combine_hands(l, r), max_speed, default_speed)
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
        // 90° pitch up about X: -Z → +Y
        let q = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        let v = stick_to_world(q, 0.0, 1.0, 0.0);
        assert!(v.y > 0.9, "expected +Y, got {v:?}");
    }
}
