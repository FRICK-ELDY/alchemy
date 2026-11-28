//! Official Dash → GpuPanelHeader + Shell/Chrome GpuUiPrim (x00f6 U0).
//!
//! Guarantees: 1 open Dash = 1 board; Shell/Chrome/tabs as RoundedBox + G0 glass;
//! short ASCII MsdfGlyph tab labels; append-friendly pack (caller clears).
//! Non-guarantees: Facet body prim migration (U0.1); CJK; Image/Texture prim;
//! LayoutTree replacement; Facet rearrange.

#![forbid(unsafe_code)]

use glam::Mat4;

use super::glass::{
    control_payload, shell_payload, shell_rect_inset, CHROME_FILL, CONTROL_HOT, CONTROL_IDLE,
    SHELL_BORDER, SHELL_FILL, TAB_ON, TEXT,
};
use super::msdf::{push_ascii_glyphs, AsciiGlyphBlit};
use super::prim::{pack_panel_kind, GpuPanelHeader, GpuUiPrim, UiPrimKind};

/// Chrome geometry shared with `system_ui::dash::rebuild` (top-left layout space).
pub const DASH_TOP_H: f32 = 44.0;
pub const DASH_BOTTOM_H: f32 = 72.0;

/// Physical board size / distance (meters). XR `QUAD_*` re-exports these (single SSoT).
pub const DASH_WIDTH_M: f32 = 1.2;
pub const DASH_HEIGHT_M: f32 = 0.675;
pub const DASH_DISTANCE_M: f32 = 1.4;

/// Which bottom tab is active (mirrors `DashTab` without depending on `system_ui`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfficialDashTab {
    Home,
    Inventory,
    Room,
    Settings,
    Exit,
}

/// Minimal view for packing — surface fills this from `DashState` + pose.
#[derive(Clone, Debug)]
pub struct OfficialDashView {
    pub width_px: f32,
    pub height_px: f32,
    pub width_m: f32,
    pub height_m: f32,
    /// Rotation+translation only (scale lives in `width_m`/`height_m`).
    pub world_matrix: Mat4,
    /// Laser/pointer is on this board (G0 active shell glow).
    pub shell_active: bool,
    pub show_bottom_tabs: bool,
    pub active_tab: OfficialDashTab,
    /// Tab under pointer (optional hot chrome).
    pub hovered_tab: Option<OfficialDashTab>,
}

impl Default for OfficialDashView {
    fn default() -> Self {
        Self {
            width_px: 960.0,
            height_px: 540.0,
            width_m: DASH_WIDTH_M,
            height_m: DASH_HEIGHT_M,
            world_matrix: Mat4::IDENTITY,
            shell_active: false,
            show_bottom_tabs: true,
            active_tab: OfficialDashTab::Home,
            hovered_tab: None,
        }
    }
}

/// Layout (top-left, +Y down) → board UV rect (bottom-left, +Y up).
#[inline]
pub fn layout_rect_to_board(x: f32, y: f32, w: f32, h: f32, height_px: f32) -> [f32; 4] {
    [x, height_px - y - h, w, h]
}

fn prim_box(
    panel: u16,
    rect: [f32; 4],
    clip: [f32; 4],
    color: u32,
    border: u32,
    payload: [u32; 4],
) -> GpuUiPrim {
    GpuUiPrim {
        rect,
        clip_rect: clip,
        payload,
        panel_and_kind: pack_panel_kind(panel, UiPrimKind::RoundedBox, 0),
        color_rgba: color,
        border_rgba: border,
        binding_id: 0,
    }
}

/// Append one official Dash board (Shell + Chrome + tabs). Does **not** clear outputs.
///
/// `panel_index` / `prim_start` must match the slot this header will occupy after append.
pub fn pack_official_dash(
    view: &OfficialDashView,
    panel_index: u16,
    out_headers: &mut Vec<GpuPanelHeader>,
    out_prims: &mut Vec<GpuUiPrim>,
) {
    let prim_start = out_prims.len() as u32;
    let w = view.width_px.max(1.0);
    let h = view.height_px.max(1.0);
    let clip = [0.0, 0.0, w, h];
    let border = SHELL_BORDER.packed();

    out_prims.push(prim_box(
        panel_index,
        shell_rect_inset(w, h),
        clip,
        SHELL_FILL.packed(),
        border,
        shell_payload(view.shell_active),
    ));

    // Top chrome bar (layout y=0..TOP_H). Close/session chips stay on CPU (no decorative "X").
    let top = layout_rect_to_board(0.0, 0.0, w, DASH_TOP_H, h);
    out_prims.push(prim_box(
        panel_index,
        top,
        clip,
        CHROME_FILL.packed(),
        border,
        control_payload(),
    ));

    let show_bottom = view.show_bottom_tabs;
    let by = if show_bottom { h - DASH_BOTTOM_H } else { h };
    // Content card omitted in U0 — Facet bodies are CPU-opaque; empty Card would only show in
    // translucent gaps (→ U0.1 when bodies move to prims).

    if show_bottom {
        let bottom = layout_rect_to_board(0.0, by, w, DASH_BOTTOM_H, h);
        out_prims.push(prim_box(
            panel_index,
            bottom,
            clip,
            CHROME_FILL.packed(),
            border,
            control_payload(),
        ));

        let tab_w = 72.0;
        let gap = 8.0;
        let tabs_total = tab_w * 5.0 + gap * 4.0;
        let tab0_x = (w - tabs_total) * 0.5;
        let tabs = [
            (OfficialDashTab::Home, "Home"),
            (OfficialDashTab::Inventory, "Inv"),
            (OfficialDashTab::Room, "Room"),
            (OfficialDashTab::Settings, "Set"),
            (OfficialDashTab::Exit, "Exit"),
        ];
        for (i, (tab, label_text)) in tabs.iter().enumerate() {
            let x = tab0_x + (tab_w + gap) * i as f32;
            let r = layout_rect_to_board(x, by + 8.0, tab_w, DASH_BOTTOM_H - 16.0, h);
            let active = view.active_tab == *tab;
            let hot = view.hovered_tab == Some(*tab);
            let fill = if active {
                TAB_ON.packed()
            } else if hot {
                CONTROL_HOT.packed()
            } else {
                CONTROL_IDLE.packed()
            };
            out_prims.push(prim_box(
                panel_index,
                r,
                clip,
                fill,
                border,
                control_payload(),
            ));
            // Same layout band as former CPU `push_label` (by+46) — below the 32px icon.
            let label_h = 16.0;
            let label_r = layout_rect_to_board(x + 10.0, by + 46.0, tab_w - 20.0, label_h, h);
            push_ascii_glyphs(
                out_prims,
                AsciiGlyphBlit {
                    panel: panel_index,
                    origin_x: label_r[0],
                    origin_y: label_r[1],
                    glyph_w: 11.0,
                    glyph_h: label_h,
                    clip,
                    color_rgba: TEXT.packed(),
                    binding_id: 0,
                    text: label_text,
                },
            );
        }
    }

    out_headers.push(GpuPanelHeader {
        world_matrix: view.world_matrix.to_cols_array_2d(),
        dimensions: [w, h, view.width_m, view.height_m],
        prim_start_index: prim_start,
        prim_count: out_prims.len() as u32 - prim_start,
        lod_level: 0,
        _pad: 0,
    });
}

/// World matrix for a head-locked dash board (no baked scale).
pub fn dash_board_world_matrix(
    head_pos: glam::Vec3,
    head_orient: glam::Quat,
    distance_m: f32,
) -> Mat4 {
    let forward = head_orient * glam::Vec3::NEG_Z;
    let center = head_pos + forward * distance_m;
    Mat4::from_rotation_translation(head_orient, center)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spatial_lens::glass::{shell_payload, shell_rect_inset};
    use crate::spatial_lens::prim::unpack_kind;

    /// Inverse of [`layout_rect_to_board`] Y (test-only; XR hits use `ray_hit_dash_quad`).
    fn board_uv_to_layout(uv_u: f32, uv_v: f32, height_px: f32) -> (f32, f32) {
        (uv_u, height_px - uv_v)
    }

    #[test]
    fn layout_board_y_roundtrip() {
        let h = 540.0;
        let board = layout_rect_to_board(10.0, 40.0, 100.0, 20.0, h);
        assert!((board[1] - (h - 60.0)).abs() < 1e-4);
        let (lx, ly) = board_uv_to_layout(board[0] + 5.0, board[1] + 10.0, h);
        assert!((lx - 15.0).abs() < 1e-4);
        assert!((ly - 50.0).abs() < 1e-4);
    }

    #[test]
    fn pack_emits_one_panel_with_shell() {
        let view = OfficialDashView {
            shell_active: false,
            ..OfficialDashView::default()
        };
        let mut headers = Vec::new();
        let mut prims = Vec::new();
        pack_official_dash(&view, 0, &mut headers, &mut prims);
        assert_eq!(headers.len(), 1);
        assert!(prims.len() >= 4);
        assert_eq!(headers[0].prim_count, prims.len() as u32);
        assert_eq!(unpack_kind(prims[0].panel_and_kind), UiPrimKind::RoundedBox);
        assert_eq!(prims[0].rect, shell_rect_inset(960.0, 540.0));
        assert_eq!(prims[0].payload, shell_payload(false));
    }

    #[test]
    fn pack_active_shell_raises_glow() {
        let view = OfficialDashView {
            shell_active: true,
            ..OfficialDashView::default()
        };
        let mut headers = Vec::new();
        let mut prims = Vec::new();
        pack_official_dash(&view, 0, &mut headers, &mut prims);
        assert_eq!(prims[0].payload, shell_payload(true));
    }

    #[test]
    fn pack_appends_second_panel() {
        let mut headers = Vec::new();
        let mut prims = Vec::new();
        pack_official_dash(&OfficialDashView::default(), 0, &mut headers, &mut prims);
        let n0 = prims.len();
        pack_official_dash(&OfficialDashView::default(), 1, &mut headers, &mut prims);
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[1].prim_start_index, n0 as u32);
        assert_eq!(headers[1].prim_count, (prims.len() - n0) as u32);
    }
}
