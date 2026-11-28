//! G0 lightweight glass tokens (feel_edit_experience §4.4).
//!
//! Guarantees: single SSoT for Shell/Chrome/Card/Control/Field/Accent RGBA + RoundedBox
//! geometry; same values for Desktop/XR now and Web later; `packed()` derived from `rgba`
//! (no hand-synced second channel).
//! Non-guarantees: background blur (G1); Facet body full prim migration (U0.1); CSS glass.
//!
//! LOD2 plate color in `uber_sdf.wgsl` must stay equal to [`LOD2_SHELL`] — enforced by
//! `gpu` tests via [`lod2_wgsl_vec4_literal`].

#![forbid(unsafe_code)]

use super::prim::{pack_rgba8, rounded_box_payload};

/// Linear RGBA in 0..=1. Packed RGBA8 is always derived via [`GlassColor::packed`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassColor {
    pub rgba: [f32; 4],
}

impl GlassColor {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { rgba: [r, g, b, a] }
    }

    /// RGBA8 pack (R in low byte). Rounding matches CPU Canvas / GpuUiPrim.
    #[inline]
    pub fn packed(self) -> u32 {
        pack_rgba8(
            (self.rgba[0] * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.rgba[1] * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.rgba[2] * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.rgba[3] * 255.0).round().clamp(0.0, 255.0) as u8,
        )
    }
}

// --- Role tokens (dash.rs legacy colors normalized) ---

/// Panel outer shell fill — α≈0.92 dark smoke (Lens danger ②).
pub const SHELL_FILL: GlassColor = GlassColor::new(0.06, 0.07, 0.09, 0.92);

/// Shell edge / glow base (brighter neutral, low α).
pub const SHELL_BORDER: GlassColor = GlassColor::new(0.45, 0.52, 0.62, 0.55);

/// Top/bottom chrome bars.
pub const CHROME_FILL: GlassColor = GlassColor::new(0.10, 0.11, 0.14, 0.96);

/// Inner cards / panel faces.
pub const CARD_FILL: GlassColor = GlassColor::new(0.12, 0.13, 0.16, 0.98);

/// Buttons / tabs off.
pub const CONTROL_IDLE: GlassColor = GlassColor::new(0.14, 0.16, 0.20, 1.0);

/// Hover / hot control.
pub const CONTROL_HOT: GlassColor = GlassColor::new(0.22, 0.24, 0.30, 0.95);

/// Close / chrome secondary control (legacy `CLOSE_BTN`).
pub const CONTROL_CLOSE: GlassColor = GlassColor::new(0.18, 0.19, 0.22, 1.0);

/// Danger-tinted button face (legacy `BTN_DANGER`).
pub const CONTROL_DANGER: GlassColor = GlassColor::new(0.22, 0.12, 0.12, 1.0);

/// Active tab chip (legacy `TAB_ON`).
pub const TAB_ON: GlassColor = GlassColor::new(0.55, 0.15, 0.18, 0.95);

/// Hierarchy / list row selection highlight.
pub const ROW_SELECT: GlassColor = GlassColor::new(0.188, 0.353, 0.549, 1.0);

/// Text field idle.
pub const FIELD_IDLE: GlassColor = GlassColor::new(0.10, 0.11, 0.14, 1.0);

/// Text field focus.
pub const FIELD_FOCUS: GlassColor = GlassColor::new(0.16, 0.22, 0.34, 1.0);

pub const ACCENT: GlassColor = GlassColor::new(0.35, 0.45, 0.95, 1.0);

pub const DANGER: GlassColor = GlassColor::new(0.75, 0.20, 0.22, 1.0);

pub const TEXT: GlassColor = GlassColor::new(0.92, 0.93, 0.95, 1.0);

pub const MUTED: GlassColor = GlassColor::new(0.65, 0.68, 0.72, 1.0);

/// LOD2 plate-only color (ShellFill RGB, slightly lower α).
/// WGSL must embed the same literals — see [`lod2_wgsl_vec4_literal`].
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "synced to uber_sdf via gpu tests")
)]
pub const LOD2_SHELL: GlassColor = GlassColor::new(0.06, 0.07, 0.09, 0.85);

// --- RoundedBox geometry (px) ---

pub const SHELL_RADIUS: f32 = 14.0;
pub const SHELL_BORDER_T: f32 = 1.5;
/// Idle shell soft edge (0..=0.25 band in contract).
pub const SHELL_GLOW_IDLE: f32 = 0.20;
/// Active = laser/hover on the front-most shell only (≤1.0). Not selection.
pub const SHELL_GLOW_ACTIVE: f32 = 0.85;

/// Soft-edge scale in `glow_w = border_t + SCALE * glow` (must match `uber_sdf.wgsl`).
pub const SHELL_GLOW_PAD_SCALE: f32 = 6.0;

pub const CARD_RADIUS: f32 = 8.0;
pub const CARD_BORDER_T: f32 = 1.0;

pub const CONTROL_RADIUS: f32 = 6.0;
pub const CONTROL_BORDER_T: f32 = 1.0;

#[inline]
pub fn shell_glow(active: bool) -> f32 {
    if active {
        SHELL_GLOW_ACTIVE
    } else {
        SHELL_GLOW_IDLE
    }
}

/// Soft-edge reach in px — same as WGSL `glow_w = border_t + SCALE * glow`.
#[inline]
pub fn shell_glow_extent(glow: f32) -> f32 {
    if glow > 0.0 {
        SHELL_BORDER_T + SHELL_GLOW_PAD_SCALE * glow
    } else {
        0.0
    }
}

/// Board inset pad: always sized for **active** glow so hover does not jump the shell rect.
///
/// Lens content margin (`extract` `PAD`) must be `>=` this value so chrome/rows stay
/// inside the shell fill if `SHELL_GLOW_ACTIVE` / `SHELL_GLOW_PAD_SCALE` grow.
#[inline]
pub fn shell_board_inset_pad() -> f32 {
    shell_glow_extent(SHELL_GLOW_ACTIVE)
}

/// Shell fill rect inset so outer soft glow stays inside the panel clip (board UV).
/// Rect is stable across idle/active; only `shell_payload` glow strength changes.
#[inline]
pub fn shell_rect_inset(width_px: f32, height_px: f32) -> [f32; 4] {
    let pad = shell_board_inset_pad();
    let w = (width_px - 2.0 * pad).max(1.0);
    let h = (height_px - 2.0 * pad).max(1.0);
    [pad, pad, w, h]
}

/// Exact WGSL **statement** that computes soft-edge reach (not a comment).
///
/// gpu tests require this full line so renaming SCALE cannot hide behind a stale comment.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "synced to uber_sdf via gpu tests")
)]
pub fn shell_glow_extent_wgsl_stmt() -> String {
    format!(
        "let glow_w = select(0.0, border_t + {:.1} * glow, glow > 0.0);",
        SHELL_GLOW_PAD_SCALE
    )
}

#[inline]
pub fn shell_payload(active: bool) -> [u32; 4] {
    rounded_box_payload(SHELL_RADIUS, SHELL_BORDER_T, shell_glow(active))
}

#[inline]
pub fn card_payload() -> [u32; 4] {
    rounded_box_payload(CARD_RADIUS, CARD_BORDER_T, 0.0)
}

#[inline]
pub fn control_payload() -> [u32; 4] {
    rounded_box_payload(CONTROL_RADIUS, CONTROL_BORDER_T, 0.0)
}

/// Exact `vec4(...)` fragment expected in `uber_sdf.wgsl` for LOD2.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "synced to uber_sdf via gpu tests")
)]
pub fn lod2_wgsl_vec4_literal() -> String {
    let c = LOD2_SHELL.rgba;
    format!("vec4({:.2}, {:.2}, {:.2}, {:.2})", c[0], c[1], c[2], c[3])
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_TOKENS: &[GlassColor] = &[
        SHELL_FILL,
        SHELL_BORDER,
        CHROME_FILL,
        CARD_FILL,
        CONTROL_IDLE,
        CONTROL_HOT,
        CONTROL_CLOSE,
        CONTROL_DANGER,
        TAB_ON,
        ROW_SELECT,
        FIELD_IDLE,
        FIELD_FOCUS,
        ACCENT,
        DANGER,
        TEXT,
        MUTED,
        LOD2_SHELL,
    ];

    #[test]
    fn shell_fill_alpha_near_092() {
        assert!((SHELL_FILL.rgba[3] - 0.92).abs() < 0.001);
        assert_eq!(SHELL_FILL.packed(), pack_rgba8(15, 18, 23, 235));
    }

    #[test]
    fn packed_matches_rgba_for_all_tokens() {
        for c in ALL_TOKENS {
            let expected = pack_rgba8(
                (c.rgba[0] * 255.0).round() as u8,
                (c.rgba[1] * 255.0).round() as u8,
                (c.rgba[2] * 255.0).round() as u8,
                (c.rgba[3] * 255.0).round() as u8,
            );
            assert_eq!(c.packed(), expected, "rgba={:?}", c.rgba);
        }
    }

    #[test]
    fn shell_payload_active_uses_higher_glow() {
        let idle = shell_payload(false);
        let active = shell_payload(true);
        assert_eq!(idle[0], SHELL_RADIUS.to_bits());
        assert_eq!(idle[1], SHELL_BORDER_T.to_bits());
        assert_eq!(idle[2], SHELL_GLOW_IDLE.to_bits());
        assert_eq!(idle[3], 0);
        assert_eq!(active[2], SHELL_GLOW_ACTIVE.to_bits());
    }

    #[test]
    fn shell_glow_extent_includes_border_t() {
        let glow = SHELL_GLOW_ACTIVE;
        let extent = shell_glow_extent(glow);
        assert!((extent - (SHELL_BORDER_T + SHELL_GLOW_PAD_SCALE * glow)).abs() < 1e-5);
        assert!(extent > SHELL_GLOW_PAD_SCALE * glow);
    }

    #[test]
    fn shell_rect_inset_stable_and_covers_active_extent() {
        let pad = shell_board_inset_pad();
        let rect = shell_rect_inset(420.0, 520.0);
        assert!((rect[0] - pad).abs() < 1e-5);
        assert!((rect[1] - pad).abs() < 1e-5);
        // Soft edge for active glow reaches board edge, not past clip.
        assert!((rect[0] + rect[2] + pad - 420.0).abs() < 1e-3);
        assert!((rect[1] + rect[3] + pad - 520.0).abs() < 1e-3);
        assert_eq!(
            shell_glow_extent_wgsl_stmt(),
            "let glow_w = select(0.0, border_t + 6.0 * glow, glow > 0.0);"
        );
    }

    #[test]
    fn lod2_matches_shell_rgb() {
        assert_eq!(LOD2_SHELL.rgba[0], SHELL_FILL.rgba[0]);
        assert_eq!(LOD2_SHELL.rgba[1], SHELL_FILL.rgba[1]);
        assert_eq!(LOD2_SHELL.rgba[2], SHELL_FILL.rgba[2]);
        assert!(LOD2_SHELL.rgba[3] < SHELL_FILL.rgba[3]);
        assert_eq!(lod2_wgsl_vec4_literal(), "vec4(0.06, 0.07, 0.09, 0.85)");
    }
}
