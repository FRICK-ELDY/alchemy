//! Resonite-style interaction laser state (x00bd Stickiness / press-lock; x00ea Primary gate;
//! x00ee Settings ↔ prefs).
//!
//! Guarantees: angle-modulated smooth + sticky UV lock without heap alloc;
//! Primary visibility gate (`laser_show_primary_min`, default 0.1) separate from Grab≈0.7;
//! Settings-exposed fields + clamp/step for L1 prefs UI.
//! Non-guarantees: bezier laser draw; haptics; double-click; CRDT scrub commit;
//! Desktop beam overlay when `show_lasers_in_desktop_mode` (→ L3); XR smooth wiring (→ L2).

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// Float knobs exposed in Settings (± buttons · x00ee).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaserFloatParam {
    SmoothSpeed,
    ModulateStartAngle,
    ModulateEndAngle,
    ModulateExponent,
    ModulateSpeedMultiplier,
    Stickiness,
    ShowPrimaryMin,
}

impl LaserFloatParam {
    pub const ALL: [Self; 7] = [
        Self::SmoothSpeed,
        Self::ModulateStartAngle,
        Self::ModulateEndAngle,
        Self::ModulateExponent,
        Self::ModulateSpeedMultiplier,
        Self::Stickiness,
        Self::ShowPrimaryMin,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::SmoothSpeed => "SmoothSpeed",
            Self::ModulateStartAngle => "ModulateStartAngle",
            Self::ModulateEndAngle => "ModulateEndAngle",
            Self::ModulateExponent => "ModulateExponent",
            Self::ModulateSpeedMultiplier => "ModulateSpeedMultiplier",
            Self::Stickiness => "StickThreshold",
            Self::ShowPrimaryMin => "ShowPrimaryMin",
        }
    }

    pub fn step(self) -> f32 {
        match self {
            Self::SmoothSpeed => 1.0,
            Self::ModulateStartAngle => 0.5,
            Self::ModulateEndAngle => 1.0,
            Self::ModulateExponent => 0.1,
            Self::ModulateSpeedMultiplier => 0.5,
            Self::Stickiness => 0.05,
            Self::ShowPrimaryMin => 0.05,
        }
    }

    pub fn range(self) -> (f32, f32) {
        match self {
            Self::SmoothSpeed => (0.0, 60.0),
            Self::ModulateStartAngle => (0.0, 45.0),
            Self::ModulateEndAngle => (1.0, 90.0),
            Self::ModulateExponent => (0.1, 8.0),
            Self::ModulateSpeedMultiplier => (1.0, 32.0),
            Self::Stickiness => (0.0, 1.0),
            Self::ShowPrimaryMin => (0.0, 1.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InteractionLaserConfig {
    pub laser_smooth_speed: f32,
    pub modulate_start_angle_deg: f32,
    pub modulate_end_angle_deg: f32,
    pub modulate_exponent: f32,
    pub modulate_speed_multiplier: f32,
    pub laser_stickiness: f32,
    /// XR beam drawn only when aiming-hand Primary ≥ this (x00ea L0). Not Grab/UI click.
    pub laser_show_primary_min: f32,
    /// Resonite ShowInDesktop (x00ee). Desktop beam overlay is L3; does not gate Lens pick.
    pub show_lasers_in_desktop_mode: bool,
}

impl Default for InteractionLaserConfig {
    fn default() -> Self {
        Self {
            laser_smooth_speed: 12.0,
            modulate_start_angle_deg: 1.5,
            modulate_end_angle_deg: 18.0,
            modulate_exponent: 1.8,
            modulate_speed_multiplier: 8.0,
            laser_stickiness: 0.5,
            laser_show_primary_min: 0.1,
            show_lasers_in_desktop_mode: false,
        }
    }
}

impl InteractionLaserConfig {
    /// Whether the XR interaction beam should be drawn for this Primary analog.
    #[inline]
    pub fn beam_visible(&self, primary: f32) -> bool {
        primary >= self.laser_show_primary_min
    }

    /// Clamp all floats into Settings ranges (prefs load / ± nudges).
    /// Non-finite values fall back to [`Default`] so chrome `PartialEq` stays stable.
    pub fn clamp(mut self) -> Self {
        let defaults = Self::default();
        for p in LaserFloatParam::ALL {
            let (lo, hi) = p.range();
            let raw = self.get_float(p);
            let v = if raw.is_finite() {
                raw.clamp(lo, hi)
            } else {
                defaults.get_float(p)
            };
            self.set_float(p, v);
        }
        if self.modulate_end_angle_deg < self.modulate_start_angle_deg {
            self.modulate_end_angle_deg = self.modulate_start_angle_deg;
        }
        self
    }

    pub fn get_float(self, param: LaserFloatParam) -> f32 {
        match param {
            LaserFloatParam::SmoothSpeed => self.laser_smooth_speed,
            LaserFloatParam::ModulateStartAngle => self.modulate_start_angle_deg,
            LaserFloatParam::ModulateEndAngle => self.modulate_end_angle_deg,
            LaserFloatParam::ModulateExponent => self.modulate_exponent,
            LaserFloatParam::ModulateSpeedMultiplier => self.modulate_speed_multiplier,
            LaserFloatParam::Stickiness => self.laser_stickiness,
            LaserFloatParam::ShowPrimaryMin => self.laser_show_primary_min,
        }
    }

    pub fn set_float(&mut self, param: LaserFloatParam, value: f32) {
        match param {
            LaserFloatParam::SmoothSpeed => self.laser_smooth_speed = value,
            LaserFloatParam::ModulateStartAngle => self.modulate_start_angle_deg = value,
            LaserFloatParam::ModulateEndAngle => self.modulate_end_angle_deg = value,
            LaserFloatParam::ModulateExponent => self.modulate_exponent = value,
            LaserFloatParam::ModulateSpeedMultiplier => self.modulate_speed_multiplier = value,
            LaserFloatParam::Stickiness => self.laser_stickiness = value,
            LaserFloatParam::ShowPrimaryMin => self.laser_show_primary_min = value,
        }
    }

    /// Nudge a float by ±step and clamp. Returns true if the value changed.
    pub fn nudge_float(&mut self, param: LaserFloatParam, dir: i8) -> bool {
        let before = *self;
        let step = param.step() * (dir as f32).signum();
        let next = self.get_float(param) + step;
        self.set_float(param, next);
        *self = self.clamp();
        *self != before
    }

    pub fn format_float_label(self, param: LaserFloatParam) -> String {
        let v = self.get_float(param);
        match param {
            LaserFloatParam::SmoothSpeed => format!("{}: {v:.0}", param.label()),
            LaserFloatParam::ModulateStartAngle | LaserFloatParam::ModulateEndAngle => {
                format!("{}: {v:.1}°", param.label())
            }
            LaserFloatParam::ModulateExponent
            | LaserFloatParam::ModulateSpeedMultiplier
            | LaserFloatParam::Stickiness
            | LaserFloatParam::ShowPrimaryMin => {
                format!("{}: {v:.2}", param.label())
            }
        }
    }
}

/// Per-hand laser runtime state (wired fields only).
#[derive(Clone, Debug)]
pub struct HandLaserState {
    pub smoothed_dir: [f32; 3],
    pub sticky_target_binding: u32,
    pub sticky_rect_px: [f32; 4],
    pub locked_uv_px: [f32; 2],
    pub prev_trigger_val: f32,
    pub press_lock_timer: f32,
}

impl Default for HandLaserState {
    fn default() -> Self {
        Self {
            smoothed_dir: [0.0, 0.0, -1.0],
            sticky_target_binding: 0,
            sticky_rect_px: [0.0; 4],
            locked_uv_px: [0.0; 2],
            prev_trigger_val: 0.0,
            press_lock_timer: 0.0,
        }
    }
}

impl HandLaserState {
    /// Instantly align smoothed aim with `dir` (Lens open snap; skip converge lag).
    pub fn snap_direction(&mut self, dir: [f32; 3]) {
        let len_sq = dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2];
        if len_sq < 1e-12 {
            self.smoothed_dir = [0.0, 0.0, -1.0];
            return;
        }
        let inv = 1.0 / len_sq.sqrt();
        self.smoothed_dir = [dir[0] * inv, dir[1] * inv, dir[2] * inv];
        self.press_lock_timer = 0.0;
        self.sticky_target_binding = 0;
    }

    /// Angle-modulated smoothing (architecture §4.4).
    pub fn update_smoothed_direction(
        &mut self,
        raw_dir: [f32; 3],
        cfg: &InteractionLaserConfig,
        dt: f32,
    ) -> [f32; 3] {
        let dot = (self.smoothed_dir[0] * raw_dir[0]
            + self.smoothed_dir[1] * raw_dir[1]
            + self.smoothed_dir[2] * raw_dir[2])
            .clamp(-1.0, 1.0);
        let angle_deg = dot.acos().to_degrees();

        let span = (cfg.modulate_end_angle_deg - cfg.modulate_start_angle_deg).max(0.001);
        let t_mod = ((angle_deg - cfg.modulate_start_angle_deg) / span).clamp(0.0, 1.0);
        let boost = 1.0 + (cfg.modulate_speed_multiplier - 1.0) * t_mod.powf(cfg.modulate_exponent);
        let eff_speed = cfg.laser_smooth_speed * boost;

        let alpha = 1.0 - (-eff_speed * dt).exp();
        let nx = self.smoothed_dir[0] + (raw_dir[0] - self.smoothed_dir[0]) * alpha;
        let ny = self.smoothed_dir[1] + (raw_dir[1] - self.smoothed_dir[1]) * alpha;
        let nz = self.smoothed_dir[2] + (raw_dir[2] - self.smoothed_dir[2]) * alpha;
        let len_inv = 1.0 / (nx * nx + ny * ny + nz * nz).sqrt().max(1e-6);

        self.smoothed_dir = [nx * len_inv, ny * len_inv, nz * len_inv];
        self.smoothed_dir
    }

    /// Stickiness + trigger press-lock. Returns `(uv_px, binding_id)`.
    pub fn apply_stickiness_and_press_lock(
        &mut self,
        raw_uv_px: [f32; 2],
        hovered_binding: u32,
        hovered_rect: [f32; 4],
        trigger_analog: f32,
        cfg: &InteractionLaserConfig,
        dt: f32,
    ) -> ([f32; 2], u32) {
        let d_trig = trigger_analog - self.prev_trigger_val;
        self.prev_trigger_val = trigger_analog;
        if trigger_analog > 0.08 && d_trig > 0.01 && self.sticky_target_binding != 0 {
            self.press_lock_timer = 0.15;
            self.locked_uv_px = raw_uv_px;
        }
        if self.press_lock_timer > 0.0 {
            self.press_lock_timer = (self.press_lock_timer - dt).max(0.0);
            return (self.locked_uv_px, self.sticky_target_binding);
        }

        let margin = cfg.laser_stickiness * 24.0;
        if self.sticky_target_binding != 0 && cfg.laser_stickiness > 0.0 {
            let [rx, ry, rw, rh] = self.sticky_rect_px;
            let in_expanded = raw_uv_px[0] >= rx - margin
                && raw_uv_px[0] <= rx + rw + margin
                && raw_uv_px[1] >= ry - margin
                && raw_uv_px[1] <= ry + rh + margin;

            if in_expanded {
                let k = cfg.laser_stickiness * 0.65;
                let cx = raw_uv_px[0].clamp(rx, rx + rw);
                let cy = raw_uv_px[1].clamp(ry, ry + rh);
                let uv = [
                    raw_uv_px[0] + (cx - raw_uv_px[0]) * k,
                    raw_uv_px[1] + (cy - raw_uv_px[1]) * k,
                ];
                self.locked_uv_px = uv;
                return (uv, self.sticky_target_binding);
            }
        }

        if hovered_binding != 0 {
            self.sticky_target_binding = hovered_binding;
            self.sticky_rect_px = hovered_rect;
            self.locked_uv_px = raw_uv_px;
            (raw_uv_px, hovered_binding)
        } else {
            self.sticky_target_binding = 0;
            (raw_uv_px, 0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smooth_moves_toward_raw() {
        let mut h = HandLaserState::default();
        let cfg = InteractionLaserConfig::default();
        let d = h.update_smoothed_direction([0.0, 1.0, 0.0], &cfg, 1.0 / 60.0);
        assert!(d[1] > 0.0);
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-4);
    }

    #[test]
    fn snap_direction_sets_unit() {
        let mut h = HandLaserState::default();
        h.snap_direction([3.0, 0.0, 0.0]);
        assert!((h.smoothed_dir[0] - 1.0).abs() < 1e-5);
        assert!(h.sticky_target_binding == 0);
    }

    #[test]
    fn stickiness_keeps_prior_target_in_margin() {
        let mut h = HandLaserState {
            sticky_target_binding: 42,
            sticky_rect_px: [10.0, 10.0, 40.0, 20.0],
            ..Default::default()
        };
        let cfg = InteractionLaserConfig {
            laser_stickiness: 1.0,
            ..Default::default()
        };
        let (uv, id) =
            h.apply_stickiness_and_press_lock([55.0, 20.0], 0, [0.0; 4], 0.0, &cfg, 1.0 / 60.0);
        assert_eq!(id, 42);
        assert!(uv[0] < 55.0);
        assert!(uv[0] >= 50.0 - 1e-3);
    }

    #[test]
    fn press_lock_holds_binding() {
        let mut h = HandLaserState {
            sticky_target_binding: 7,
            sticky_rect_px: [0.0, 0.0, 10.0, 10.0],
            locked_uv_px: [5.0, 5.0],
            prev_trigger_val: 0.0,
            ..Default::default()
        };
        let cfg = InteractionLaserConfig::default();
        let (_uv, id) = h.apply_stickiness_and_press_lock(
            [5.0, 5.0],
            7,
            [0.0, 0.0, 10.0, 10.0],
            0.2,
            &cfg,
            0.0,
        );
        assert_eq!(id, 7);
        assert!(h.press_lock_timer > 0.0);
        let (_uv2, id2) =
            h.apply_stickiness_and_press_lock([100.0, 100.0], 0, [0.0; 4], 0.5, &cfg, 0.01);
        assert_eq!(id2, 7);
    }

    #[test]
    fn beam_visible_gates_on_primary_min() {
        let cfg = InteractionLaserConfig::default();
        assert!((cfg.laser_show_primary_min - 0.1).abs() < 1e-6);
        assert!(!cfg.beam_visible(0.0));
        assert!(!cfg.beam_visible(0.099));
        assert!(cfg.beam_visible(0.1));
        assert!(cfg.beam_visible(0.7));
        // Grab/UI threshold stays independent: visibility can be true below 0.7.
        assert!(cfg.beam_visible(0.2));
        let tight = InteractionLaserConfig {
            laser_show_primary_min: 0.5,
            ..Default::default()
        };
        assert!(!tight.beam_visible(0.2));
    }

    #[test]
    fn show_in_desktop_defaults_off() {
        assert!(!InteractionLaserConfig::default().show_lasers_in_desktop_mode);
    }

    #[test]
    fn nudge_float_clamps() {
        let mut cfg = InteractionLaserConfig::default();
        for _ in 0..100 {
            cfg.nudge_float(LaserFloatParam::Stickiness, 1);
        }
        assert!((cfg.laser_stickiness - 1.0).abs() < 1e-5);
        for _ in 0..100 {
            cfg.nudge_float(LaserFloatParam::Stickiness, -1);
        }
        assert!((cfg.laser_stickiness - 0.0).abs() < 1e-5);
    }

    #[test]
    fn serde_roundtrip_defaults() {
        let cfg = InteractionLaserConfig::default();
        let raw = serde_json::to_string(&cfg).unwrap();
        let back: InteractionLaserConfig = serde_json::from_str(&raw).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn serde_missing_keys_use_default() {
        let back: InteractionLaserConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(back, InteractionLaserConfig::default());
    }

    #[test]
    fn clamp_replaces_non_finite() {
        let cfg = InteractionLaserConfig {
            laser_smooth_speed: f32::NAN,
            laser_stickiness: f32::INFINITY,
            ..Default::default()
        }
        .clamp();
        let d = InteractionLaserConfig::default();
        assert_eq!(cfg.laser_smooth_speed, d.laser_smooth_speed);
        assert_eq!(cfg.laser_stickiness, d.laser_stickiness);
        assert_eq!(cfg, cfg); // PartialEq stable after clamp
    }
}
