//! RenderFrame upsert change detection (x0029).
//!
//! Guarantees: translation / scale component epsilons; orientation uses |dot|
//! so unit quaternions `q` and `-q` count as the same pose (double cover).
//! Non-guarantees: color-driven upserts; non-unit quat drift beyond normalize.

pub const MOVE_EPSILON: f64 = 1e-4;
/// Orientation differs when `|dot(qa, qb)| < 1 - ROT_DOT_EPSILON` after normalize.
pub const ROT_DOT_EPSILON: f64 = 1e-5;
pub const SCALE_EPSILON: f64 = 1e-5;

pub fn translation_changed(prev: [f64; 3], next: [f64; 3]) -> bool {
    (prev[0] - next[0]).abs() > MOVE_EPSILON
        || (prev[1] - next[1]).abs() > MOVE_EPSILON
        || (prev[2] - next[2]).abs() > MOVE_EPSILON
}

/// True when orientations differ. `q` and `-q` are the same attitude.
pub fn orientation_changed(prev_xyzw: [f64; 4], next_xyzw: [f64; 4]) -> bool {
    match (normalize_quat(prev_xyzw), normalize_quat(next_xyzw)) {
        (Some(a), Some(b)) => {
            let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
            !dot.is_finite() || dot.abs() < 1.0 - ROT_DOT_EPSILON
        }
        // Invalid / zero quat: force upsert so peers see a defined orientation.
        _ => true,
    }
}

pub fn scale_changed(prev: [f64; 3], next: [f64; 3]) -> bool {
    (prev[0] - next[0]).abs() > SCALE_EPSILON
        || (prev[1] - next[1]).abs() > SCALE_EPSILON
        || (prev[2] - next[2]).abs() > SCALE_EPSILON
}

fn normalize_quat(q: [f64; 4]) -> Option<[f64; 4]> {
    let n2 = q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3];
    if !n2.is_finite() || n2 < 1e-24 {
        return None;
    }
    let inv = 1.0 / n2.sqrt();
    Some([q[0] * inv, q[1] * inv, q[2] * inv, q[3] * inv])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opposite_hemisphere_quats_are_same_orientation() {
        let q = [0.0, 0.0, 0.7071067811865476, 0.7071067811865476];
        let neg = [-q[0], -q[1], -q[2], -q[3]];
        assert!(!orientation_changed(q, neg));
        assert!(!orientation_changed(q, q));
    }

    #[test]
    fn rotation_only_is_detected() {
        let identity = [0.0, 0.0, 0.0, 1.0];
        let yaw90 = [0.0, 0.7071067811865476, 0.0, 0.7071067811865476];
        assert!(orientation_changed(identity, yaw90));
        assert!(!translation_changed([1.0, 2.0, 3.0], [1.0, 2.0, 3.0]));
    }

    #[test]
    fn tiny_renormalization_noise_is_not_a_change() {
        let q = [0.0, 0.0, 0.0, 1.0];
        // ~1e-6 component noise after re-normalize stays within ROT_DOT_EPSILON.
        let noisy = [1e-6, 0.0, 0.0, 1.0];
        assert!(!orientation_changed(q, noisy));
    }

    #[test]
    fn zero_quat_forces_change() {
        assert!(orientation_changed(
            [0.0, 0.0, 0.0, 1.0],
            [0.0, 0.0, 0.0, 0.0]
        ));
    }
}
