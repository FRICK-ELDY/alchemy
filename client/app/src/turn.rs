//! Comfort turn settings + yaw integration (client-local; x000a).
//!
//! Guarantees: snap / smooth yaw from a 1D stick axis; defaults for Settings display.
//! Snap fires once per flick; stick must return below deadzone before the next snap.
//! Non-guarantees: host-authoritative facing, persistence, virtual keyboard edit.
//!
//! Comfort yaw rotates the *viewed world* only. Move Intent stays controller-aim relative.

/// Stick |X| must reach this to fire a snap (when armed).
pub const SNAP_THRESHOLD: f32 = 0.65;
/// Default snap turn step (degrees).
pub const DEFAULT_SNAP_DEGREES: f32 = 30.0;
/// Default smooth turn rate (degrees / second).
pub const DEFAULT_SMOOTH_TURN_DEG_PER_SEC: f32 = 90.0;
/// Default axis deadzone for turn.
pub const TURN_DEADZONE: f32 = 0.15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnMode {
    Snap,
    Smooth,
}

impl TurnMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Snap => "Snap",
            Self::Smooth => "Smooth",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::Snap => Self::Smooth,
            Self::Smooth => Self::Snap,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TurnSettings {
    pub mode: TurnMode,
    pub snap_degrees: f32,
    pub smooth_deg_per_sec: f32,
}

impl Default for TurnSettings {
    fn default() -> Self {
        Self {
            mode: TurnMode::Snap,
            snap_degrees: DEFAULT_SNAP_DEGREES,
            smooth_deg_per_sec: DEFAULT_SMOOTH_TURN_DEG_PER_SEC,
        }
    }
}

/// Update comfort-turn yaw (radians).
///
/// `snap_armed` is set when the stick returns below `deadzone`. A snap fires only while
/// armed and `|stick_x| >= SNAP_THRESHOLD`, then disarms until the next release.
pub fn update_turn_yaw(
    yaw_rad: f32,
    stick_x: f32,
    settings: TurnSettings,
    dt: f32,
    snap_armed: &mut bool,
    deadzone: f32,
) -> f32 {
    if !stick_x.is_finite() || !dt.is_finite() || dt <= 0.0 {
        return yaw_rad;
    }
    match settings.mode {
        TurnMode::Snap => {
            if stick_x.abs() < deadzone {
                *snap_armed = true;
                return yaw_rad;
            }
            if *snap_armed && stick_x.abs() >= SNAP_THRESHOLD {
                *snap_armed = false;
                let step = settings.snap_degrees.to_radians().abs();
                return yaw_rad + stick_x.signum() * step;
            }
            yaw_rad
        }
        TurnMode::Smooth => {
            if stick_x.abs() < deadzone {
                return yaw_rad;
            }
            let t = ((stick_x.abs() - deadzone) / (1.0 - deadzone).max(1e-6)).clamp(0.0, 1.0);
            let rate = settings.smooth_deg_per_sec.to_radians();
            yaw_rad + stick_x.signum() * t * rate * dt
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_once_per_flick_no_hold_repeat() {
        let settings = TurnSettings {
            mode: TurnMode::Snap,
            snap_degrees: 30.0,
            ..Default::default()
        };
        let mut armed = true;
        let y1 = update_turn_yaw(0.0, 0.9, settings, 0.016, &mut armed, TURN_DEADZONE);
        assert!((y1 - 30f32.to_radians()).abs() < 1e-4);
        assert!(!armed);

        let y2 = update_turn_yaw(y1, 0.9, settings, 1.0, &mut armed, TURN_DEADZONE);
        assert!((y2 - y1).abs() < 1e-6, "holding must not repeat snap");

        let _ = update_turn_yaw(y2, 0.0, settings, 0.016, &mut armed, TURN_DEADZONE);
        assert!(armed);
        let y3 = update_turn_yaw(y2, -0.9, settings, 0.016, &mut armed, TURN_DEADZONE);
        assert!((y3 - (y2 - 30f32.to_radians())).abs() < 1e-4);
    }

    #[test]
    fn smooth_integrates_with_dt() {
        let settings = TurnSettings {
            mode: TurnMode::Smooth,
            smooth_deg_per_sec: 90.0,
            ..Default::default()
        };
        let mut armed = true;
        let y = update_turn_yaw(0.0, 1.0, settings, 1.0, &mut armed, 0.0);
        assert!((y - 90f32.to_radians()).abs() < 1e-3);
    }

    #[test]
    fn default_is_snap_30() {
        let t = TurnSettings::default();
        assert_eq!(t.mode, TurnMode::Snap);
        assert!((t.snap_degrees - 30.0).abs() < f32::EPSILON);
    }
}
