//! Input sanitization (Slice D / x0011 / x0021 / x0030 / x0041).
//!
//! Guarantees: non-finite move_* / aim_* never reach physics; move magnitude
//! capped to unit ball; Grab start requires player↔entity within [`MAX_GRAB_START_DISTANCE`];
//! hold aim / hold offset use **the same** radius (not a separate budget).
//! Non-guarantees: Intent Hz (see `intent_rate` / x0040); Steal; rotation follow;
//! final entity pose reprojected to ≤MAX (P1-6 allows up to ~2·MAX = aim球+offset).

use alchemy_protocol::InputEvent;

/// Protocol contract: move_* are ~-1..1; host clamps to this sphere.
pub const MAX_MOVE_MAGNITUDE: f32 = 1.0;

/// Grab start gate, hold aim, and hold offset share this one SSoT (x0030 / x0041).
/// Name keeps x0021 call sites; value is always [`alchemy_shared_sim::MAX_GRAB_HIT_DISTANCE`].
pub use alchemy_shared_sim::MAX_GRAB_HIT_DISTANCE as MAX_GRAB_START_DISTANCE;

/// Returns true when all spatial fields are finite (not NaN / Inf).
pub fn input_event_is_finite(event: &InputEvent) -> bool {
    event.move_x.is_finite()
        && event.move_y.is_finite()
        && event.move_z.is_finite()
        && event.aim_x.is_finite()
        && event.aim_y.is_finite()
        && event.aim_z.is_finite()
}

/// Project `(x,y,z)` onto a ball of radius `max` about the origin (f64 length).
fn clamp_vec_length(x: f32, y: f32, z: f32, max: f32) -> (f32, f32, f32) {
    if !(x.is_finite() && y.is_finite() && z.is_finite() && max.is_finite() && max >= 0.0) {
        return (0.0, 0.0, 0.0);
    }
    let max64 = max as f64;
    let len = ((x as f64).powi(2) + (y as f64).powi(2) + (z as f64).powi(2)).sqrt();
    if len > max64 && len > 0.0 {
        let s = max64 / len;
        (
            (x as f64 * s) as f32,
            (y as f64 * s) as f32,
            (z as f64 * s) as f32,
        )
    } else {
        (x, y, z)
    }
}

/// Project move onto the unit ball (identity when already inside).
///
/// Length is computed in `f64` so values like `1e30` do not overflow `f32` squares to Inf.
pub fn clamp_move_vector(mx: f32, my: f32, mz: f32) -> (f32, f32, f32) {
    clamp_vec_length(mx, my, mz, MAX_MOVE_MAGNITUDE)
}

/// True when player (user_root) and entity are within Grab start distance.
pub fn within_grab_start_distance(player: (f32, f32, f32), entity: (f32, f32, f32)) -> bool {
    let dx = player.0 - entity.0;
    let dy = player.1 - entity.1;
    let dz = player.2 - entity.2;
    let dist_sq = dx * dx + dy * dy + dz * dz;
    dist_sq.is_finite() && dist_sq <= MAX_GRAB_START_DISTANCE * MAX_GRAB_START_DISTANCE
}

/// Clamp hold aim so `|aim - user_root| ≤ [`MAX_GRAB_START_DISTANCE`]` (x0041).
pub fn clamp_hold_aim(user_root: (f32, f32, f32), aim: (f32, f32, f32)) -> (f32, f32, f32) {
    let (dx, dy, dz) = clamp_vec_length(
        aim.0 - user_root.0,
        aim.1 - user_root.1,
        aim.2 - user_root.2,
        MAX_GRAB_START_DISTANCE,
    );
    (user_root.0 + dx, user_root.1 + dy, user_root.2 + dz)
}

/// Cap Grab hold offset length to [`MAX_GRAB_START_DISTANCE`] (x0041; same meter as start).
pub fn clamp_hold_offset(ox: f32, oy: f32, oz: f32) -> (f32, f32, f32) {
    clamp_vec_length(ox, oy, oz, MAX_GRAB_START_DISTANCE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(x: f32, y: f32, z: f32) -> InputEvent {
        InputEvent {
            room_token: "t".into(),
            client_time_ms: 0,
            move_x: x,
            move_y: y,
            move_z: z,
            buttons: 0,
            grab_entity_id: 0,
            aim_x: 0.0,
            aim_y: 0.0,
            aim_z: 0.0,
            leave: false,
            session_pub: vec![],
            intent_seq: 0,
            intent_sig: vec![],
        }
    }

    #[test]
    fn accepts_finite() {
        assert!(input_event_is_finite(&sample(1.0, -0.5, 0.0)));
    }

    #[test]
    fn rejects_nan() {
        assert!(!input_event_is_finite(&sample(f32::NAN, 0.0, 0.0)));
    }

    #[test]
    fn rejects_inf() {
        assert!(!input_event_is_finite(&sample(0.0, f32::INFINITY, 0.0)));
        assert!(!input_event_is_finite(&sample(0.0, 0.0, f32::NEG_INFINITY)));
    }

    #[test]
    fn rejects_non_finite_aim() {
        let mut e = sample(0.0, 0.0, 0.0);
        e.aim_y = f32::NAN;
        assert!(!input_event_is_finite(&e));
    }

    #[test]
    fn clamp_move_leaves_unit_vector() {
        let (x, y, z) = clamp_move_vector(1.0, 0.0, 0.0);
        assert!((x - 1.0).abs() < 1e-6 && y == 0.0 && z == 0.0);
    }

    #[test]
    fn clamp_move_normalizes_huge_magnitude() {
        let (x, y, z) = clamp_move_vector(1e30, 0.0, 0.0);
        let len = (x * x + y * y + z * z).sqrt();
        assert!((len - 1.0).abs() < 1e-5, "len={len} ({x},{y},{z})");
        assert!(x.is_finite() && y.is_finite() && z.is_finite());
    }

    #[test]
    fn grab_distance_gate() {
        assert!(within_grab_start_distance((0.0, 0.0, 0.0), (3.0, 0.0, 0.0)));
        assert!(!within_grab_start_distance(
            (0.0, 0.0, 0.0),
            (6.0, 0.0, 0.0)
        ));
    }

    #[test]
    fn clamp_hold_aim_projects_onto_sphere() {
        let root = (0.0, 0.0, 0.0);
        let inside = clamp_hold_aim(root, (3.0, 0.0, 0.0));
        assert!((inside.0 - 3.0).abs() < 1e-5);

        let far = clamp_hold_aim(root, (100.0, 0.0, 0.0));
        let len = (far.0 * far.0 + far.1 * far.1 + far.2 * far.2).sqrt();
        assert!(
            (len - MAX_GRAB_START_DISTANCE).abs() < 1e-4,
            "len={len} far={far:?}"
        );
        assert!(far.0 > 0.0 && far.1.abs() < 1e-5 && far.2.abs() < 1e-5);
    }

    #[test]
    fn clamp_hold_aim_huge_does_not_overflow() {
        let root = (1.0, 2.0, 3.0);
        let far = clamp_hold_aim(root, (1e30, 2.0, 3.0));
        assert!(far.0.is_finite() && far.1.is_finite() && far.2.is_finite());
        let dx = far.0 - root.0;
        let dy = far.1 - root.1;
        let dz = far.2 - root.2;
        let len = ((dx as f64).powi(2) + (dy as f64).powi(2) + (dz as f64).powi(2)).sqrt();
        assert!((len - MAX_GRAB_START_DISTANCE as f64).abs() < 1e-3);
    }

    #[test]
    fn clamp_hold_offset_caps_length() {
        let (ox, oy, oz) = clamp_hold_offset(12.0, 0.0, 0.0);
        let len = (ox * ox + oy * oy + oz * oz).sqrt();
        assert!((len - MAX_GRAB_START_DISTANCE).abs() < 1e-4);
        let short = clamp_hold_offset(1.0, 0.0, 0.0);
        assert!((short.0 - 1.0).abs() < 1e-6);
    }
}
