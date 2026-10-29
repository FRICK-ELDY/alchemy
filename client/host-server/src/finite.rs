//! Input sanitization (Slice D / x0011 / x0021 / x0030).
//!
//! Guarantees: non-finite move_* / aim_* never reach physics; move magnitude
//! capped to unit ball; Grab start requires player↔entity within [`MAX_GRAB_START_DISTANCE`].
//! Non-guarantees: Intent Hz (see `intent_rate` / x0040); aim still drives hold offset
//! after a valid start.

use alchemy_protocol::InputEvent;

/// Protocol contract: move_* are ~-1..1; host clamps to this sphere.
pub const MAX_MOVE_MAGNITUDE: f32 = 1.0;

/// Match client pick range (`world_hit::MAX_HIT_DISTANCE`); SSoT in shared-sim (x0030).
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

/// Project move onto the unit ball (identity when already inside).
///
/// Length is computed in `f64` so values like `1e30` do not overflow `f32` squares to Inf.
pub fn clamp_move_vector(mx: f32, my: f32, mz: f32) -> (f32, f32, f32) {
    if !(mx.is_finite() && my.is_finite() && mz.is_finite()) {
        return (0.0, 0.0, 0.0);
    }
    let max = MAX_MOVE_MAGNITUDE as f64;
    let len = ((mx as f64).powi(2) + (my as f64).powi(2) + (mz as f64).powi(2)).sqrt();
    if len > max {
        let s = max / len;
        (
            (mx as f64 * s) as f32,
            (my as f64 * s) as f32,
            (mz as f64 * s) as f32,
        )
    } else {
        (mx, my, mz)
    }
}

/// True when aim and entity positions are within Grab start distance.
pub fn within_grab_start_distance(aim: (f32, f32, f32), entity: (f32, f32, f32)) -> bool {
    let dx = aim.0 - entity.0;
    let dy = aim.1 - entity.1;
    let dz = aim.2 - entity.2;
    let dist_sq = dx * dx + dy * dy + dz * dz;
    dist_sq.is_finite() && dist_sq <= MAX_GRAB_START_DISTANCE * MAX_GRAB_START_DISTANCE
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
}
