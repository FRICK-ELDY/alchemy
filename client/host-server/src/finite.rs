//! Input sanitization (Slice D / x0011).
//!
//! Guarantees: non-finite move_* / aim_* never reach physics.
//! Non-guarantees: magnitude clamps, rate limits.

use alchemy_protocol::InputEvent;

/// Returns true when all spatial fields are finite (not NaN / Inf).
pub fn input_event_is_finite(event: &InputEvent) -> bool {
    event.move_x.is_finite()
        && event.move_y.is_finite()
        && event.move_z.is_finite()
        && event.aim_x.is_finite()
        && event.aim_y.is_finite()
        && event.aim_z.is_finite()
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
}
