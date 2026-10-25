//! Comfort turn settings + yaw integration (client-local; x000a).
//!
//! Guarantees: snap / smooth yaw from a 1D stick axis; defaults for Settings display.
//! Snap repeats while held (cooldown), without requiring a full stick release.
//! Non-guarantees: host-authoritative facing, persistence, virtual keyboard edit.
//!
//! Comfort yaw rotates the *viewed world* only. Move Intent stays controller-aim relative.

/// Stick |X| must reach this to fire / keep repeating snap.
pub const SNAP_THRESHOLD: f32 = 0.65;
/// Delay between snap steps while the stick is held past threshold.
pub const SNAP_REPEAT_SEC: f32 = 0.22;
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
/// `snap_cooldown` is seconds until the next snap is allowed (`0` = ready).
/// Holding past [`SNAP_THRESHOLD`] repeats every [`SNAP_REPEAT_SEC`].
/// Releasing below `deadzone` clears the cooldown so the next flick is immediate.
pub fn update_turn_yaw(
    yaw_rad: f32,
    stick_x: f32,
    settings: TurnSettings,
    dt: f32,
    snap_cooldown: &mut f32,
    deadzone: f32,
) -> f32 {
    if !stick_x.is_finite() || !dt.is_finite() || dt <= 0.0 {
        return yaw_rad;
    }
    match settings.mode {
        TurnMode::Snap => {
            if stick_x.abs() < deadzone {
                *snap_cooldown = 0.0;
                return yaw_rad;
            }
            *snap_cooldown = (*snap_cooldown - dt).max(0.0);
            if stick_x.abs() >= SNAP_THRESHOLD && *snap_cooldown <= 0.0 {
                *snap_cooldown = SNAP_REPEAT_SEC;
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
    fn snap_fires_immediately_then_repeats_after_cooldown() {
        let settings = TurnSettings {
            mode: TurnMode::Snap,
            snap_degrees: 30.0,
            ..Default::default()
        };
        let mut cd = 0.0_f32;
        let y1 = update_turn_yaw(0.0, 0.9, settings, 0.016, &mut cd, TURN_DEADZONE);
        assert!((y1 - 30f32.to_radians()).abs() < 1e-4);
        assert!(cd > 0.0);

        // Still held, cooldown not elapsed → no second snap.
        let y2 = update_turn_yaw(y1, 0.9, settings, 0.016, &mut cd, TURN_DEADZONE);
        assert!((y2 - y1).abs() < 1e-6);

        // Advance past repeat interval while held → second snap.
        let y3 = update_turn_yaw(y2, 0.9, settings, SNAP_REPEAT_SEC, &mut cd, TURN_DEADZONE);
        assert!((y3 - (y2 + 30f32.to_radians())).abs() < 1e-4);
    }

    #[test]
    fn snap_release_clears_cooldown_for_immediate_flick() {
        let settings = TurnSettings {
            mode: TurnMode::Snap,
            snap_degrees: 30.0,
            ..Default::default()
        };
        let mut cd = 0.0;
        let y1 = update_turn_yaw(0.0, 0.9, settings, 0.016, &mut cd, TURN_DEADZONE);
        assert!(cd > 0.0);
        let _ = update_turn_yaw(y1, 0.0, settings, 0.016, &mut cd, TURN_DEADZONE);
        assert_eq!(cd, 0.0);
        let y2 = update_turn_yaw(y1, -0.9, settings, 0.016, &mut cd, TURN_DEADZONE);
        assert!((y2 - (y1 - 30f32.to_radians())).abs() < 1e-4);
    }

    #[test]
    fn smooth_integrates_with_dt() {
        let settings = TurnSettings {
            mode: TurnMode::Smooth,
            smooth_deg_per_sec: 90.0,
            ..Default::default()
        };
        let mut cd = 0.0;
        let y = update_turn_yaw(0.0, 1.0, settings, 1.0, &mut cd, 0.0);
        assert!((y - 90f32.to_radians()).abs() < 1e-3);
    }

    #[test]
    fn default_is_snap_30() {
        let t = TurnSettings::default();
        assert_eq!(t.mode, TurnMode::Snap);
        assert!((t.snap_degrees - 30.0).abs() < f32::EPSILON);
    }
}
