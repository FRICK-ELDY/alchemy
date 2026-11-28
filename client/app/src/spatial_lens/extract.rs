//! Hierarchy / Inspector → GpuUiPrim packing (x00bb / x00bc / x00ed / x00f2 / x00f3 / x00f4).
//!
//! Guarantees: viewport-window extract only; DirectNumber pose fields without String;
//! short ASCII MsdfGlyph labels (Tier-1 Static MSDF); selected_row scrub when row is not live;
//! scroll clamp to non-empty window; Inspector POS scrub draft preview (x00f2);
//! SoftLease chrome (`LK` + holder; mine/other color · x00f3);
//! multi-axis draft_xyz + 2D scrub (x00f4).
//! Non-guarantees: CJK labels; Zenoh volatile scrub stream; Grab release pose Commit;
//! WorldRoot sync; Lens-side collapse toggle (collapse set is shared with Settings; Lens MVP reads only).

#![forbid(unsafe_code)]

use glam::{Mat4, Quat, Vec3};

use super::glass::{
    card_payload, control_payload, shell_payload, shell_rect_inset, ACCENT, CARD_FILL, CHROME_FILL,
    CONTROL_DANGER, CONTROL_HOT, CONTROL_IDLE, DANGER, MUTED, ROW_SELECT, SHELL_BORDER, SHELL_FILL,
    TEXT,
};
use super::msdf::{push_ascii_glyphs, AsciiGlyphBlit};
use super::prim::{
    pack_direct_number, pack_panel_kind, pack_rgba8, GpuPanelHeader, GpuUiPrim, UiPrimKind,
};
use crate::epoch_view::{hierarchy_panel_rows, EpochViewState, HierarchyRowView};
use crate::scene::{CameraLook, EntityPose};

/// Active Inspector POS scrub (drag → release Commit · x00f2 / x00f4). Local only.
#[derive(Clone, Debug)]
pub struct InspectorScrub {
    /// Primary axis binding that started the scrub (UV Δu).
    pub field_binding: u32,
    pub row: u32,
    pub entity_id: u32,
    pub start_uv: [f32; 2],
    /// Last panel-hit UV; reused while primary held and the ray misses the board.
    pub last_uv: [f32; 2],
    pub base_xyz: [f64; 3],
    pub draft_xyz: [f64; 3],
    /// Per-axis dirty vs base (2D scrub may mark primary + cyclic next).
    pub dirty: [bool; 3],
    /// Expected hashes at scrub begin (refreshed from Curr on release).
    pub expected_hash: [[u8; 32]; 3],
}

/// SoftLease chrome for the selected entity (x00f3). Filled by surface tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoftLeaseChrome {
    pub holder_actor_id: u64,
    /// True when `holder_actor_id` matches local Room Token `sub` hash.
    pub is_mine: bool,
}

/// Personal spatial lens runtime (local only).
#[derive(Clone, Debug)]
pub struct SpatialLensState {
    pub visible: bool,
    /// Panel world matrix. MVP: fixed after open; snapped in front of eye on toggle-on.
    pub world_matrix: Mat4,
    pub width_px: f32,
    pub height_px: f32,
    pub width_m: f32,
    pub height_m: f32,
    pub hierarchy_scroll: usize,
    pub hierarchy_window: usize,
    pub selected_row: Option<u32>,
    pub hovered_binding: u32,
    /// In-flight POS scrub (None when idle).
    pub scrub: Option<InspectorScrub>,
    /// SoftLease hint for current selection (None = unlocked / unknown).
    pub soft_lease: Option<SoftLeaseChrome>,
}

impl Default for SpatialLensState {
    fn default() -> Self {
        Self {
            visible: false,
            world_matrix: Mat4::from_translation(Vec3::new(0.0, 1.4, -1.5)),
            width_px: 420.0,
            height_px: 520.0,
            width_m: 0.84,
            height_m: 1.04,
            hierarchy_scroll: 0,
            hierarchy_window: 12,
            selected_row: None,
            hovered_binding: 0,
            scrub: None,
            soft_lease: None,
        }
    }
}

/// Binding id layout: `1 + row_id` for hierarchy rows; `0x8000_0000 | field` for inspector.
pub fn hierarchy_binding(row_id: u32) -> u32 {
    row_id.saturating_add(1)
}

pub fn decode_hierarchy_binding(binding: u32) -> Option<u32> {
    if binding == 0 || (binding & 0x8000_0000) != 0 {
        None
    } else {
        Some(binding - 1)
    }
}

pub const INSPECT_FIELD_POS_X: u32 = 0x8000_0001;
pub const INSPECT_FIELD_POS_Y: u32 = 0x8000_0002;
pub const INSPECT_FIELD_POS_Z: u32 = 0x8000_0003;
pub const INSPECT_FIELD_READY: u32 = 0x8000_0004;

/// Meters of world translation per Lens UV pixel while scrubbing (x00f2).
pub const INSPECTOR_SCRUB_M_PER_PX: f64 = 0.01;

/// UV px dead-zone for secondary-axis (Δv) dirty; primary uses only `f64::EPSILON` (x00f4).
pub const INSPECTOR_SCRUB_SECONDARY_DEADZONE_PX: f64 = 2.0;

/// Map Inspector binding → `edit_field_id::POS_*` (readiness is not editable).
pub fn decode_inspector_pos_binding(binding: u32) -> Option<u32> {
    match binding {
        INSPECT_FIELD_POS_X => Some(alchemy_protocol::edit_field_id::POS_X),
        INSPECT_FIELD_POS_Y => Some(alchemy_protocol::edit_field_id::POS_Y),
        INSPECT_FIELD_POS_Z => Some(alchemy_protocol::edit_field_id::POS_Z),
        _ => None,
    }
}

/// Axis index 0..2 for an Inspector POS binding.
pub fn inspector_pos_axis(binding: u32) -> Option<usize> {
    match binding {
        INSPECT_FIELD_POS_X => Some(0),
        INSPECT_FIELD_POS_Y => Some(1),
        INSPECT_FIELD_POS_Z => Some(2),
        _ => None,
    }
}

/// Cyclic next axis for 2D scrub secondary (Δv): X→Y, Y→Z, Z→X (x00f4).
#[inline]
pub fn inspector_secondary_axis(primary: usize) -> usize {
    (primary + 1) % 3
}

/// Wire `edit_field_id::POS_*` for axis 0..2.
#[inline]
pub fn pos_field_id_for_axis(axis: usize) -> Option<u32> {
    match axis {
        0 => Some(alchemy_protocol::edit_field_id::POS_X),
        1 => Some(alchemy_protocol::edit_field_id::POS_Y),
        2 => Some(alchemy_protocol::edit_field_id::POS_Z),
        _ => None,
    }
}

/// Wire `entity_id` for a live tape row (`owner_scope`), if present.
pub fn entity_id_for_row(epoch: &EpochViewState, row: u32) -> Option<u32> {
    let curr = epoch.snapshots.load_curr();
    let eid = *curr.owner_scope.get(row as usize)?;
    (eid != 0).then_some(eid)
}

/// World XYZ on Curr for `row` (one Curr load; all finite).
pub fn world_pos_xyz(epoch: &EpochViewState, row: u32) -> Option<[f64; 3]> {
    let curr = epoch.snapshots.load_curr();
    let xf = curr.world_transforms.get(row as usize)?;
    let p = xf.position;
    (p[0].is_finite() && p[1].is_finite() && p[2].is_finite()).then_some(p)
}

const ROW_H: f32 = 28.0;
/// Preferred content margin from board edge (px).
const PAD_MIN: f32 = 8.0;

/// Content pad ≥ shell inset so chrome/rows cannot sit outside shell fill if glow SCALE grows.
#[inline]
fn content_pad() -> f32 {
    PAD_MIN.max(super::glass::shell_board_inset_pad())
}

/// Whether `row` is live on Curr (not TOMBSTONE / missing).
pub fn row_is_live(epoch: &EpochViewState, row: u32) -> bool {
    let curr = epoch.snapshots.load_curr();
    let idx = row as usize;
    curr.readiness
        .get(idx)
        .is_some_and(|m| !m.contains(alchemy_shared_sim::tape::ReadinessMask::TOMBSTONE))
}

/// Clear Lens selection when the row is tombstoned / gone (mirror [`crate::epoch_view::scrub_xray_row`]).
///
/// Returns `entity_id` of a dropped Inspector scrub so the caller can clear
/// [`crate::pending_local_ops::PendingLocalGate`] (extract has no SharedState).
pub fn scrub_selected_row(state: &mut SpatialLensState, epoch: &EpochViewState) -> Option<u32> {
    if let Some(row) = state.selected_row {
        if !row_is_live(epoch, row) {
            state.selected_row = None;
        }
    }
    let drop_scrub = state
        .scrub
        .as_ref()
        .is_some_and(|s| state.selected_row != Some(s.row) || !row_is_live(epoch, s.row));
    if drop_scrub {
        state.scrub.take().map(|s| s.entity_id)
    } else {
        None
    }
}

/// Snap panel ~`distance_m` along the Desktop look ray (same forward as `desktop_look_ray`).
///
/// Position always follows full pitch so aim and board stay aligned. Near-parallel
/// `from_rotation_arc(Z, ±Z)` is replaced with a yaw-based facing (board still on the ray).
pub fn place_in_front_of_eye(
    state: &mut SpatialLensState,
    eye: EntityPose,
    look: CameraLook,
    distance_m: f32,
) {
    let yaw = look.yaw;
    let pitch = look.pitch.clamp(-1.54, 1.54);
    let mut forward = Vec3::new(
        yaw.sin() * pitch.cos(),
        pitch.sin(),
        -yaw.cos() * pitch.cos(),
    )
    .normalize_or_zero();
    if forward.length_squared() < 1e-8 {
        forward = Vec3::NEG_Z;
    }
    let eye_pos = Vec3::new(eye.x, eye.y, eye.z);
    let pos = eye_pos + forward * distance_m.max(0.4);
    let mut face = -forward;
    if face.length_squared() < 1e-8 {
        face = Vec3::NEG_Z;
    }
    // Panel local +Z faces the user.
    let rot = if face.cross(Vec3::Z).length_squared() < 1e-8 {
        // face ≈ ±world Z (look along ±Z): from_rotation_arc(Z, ±Z) is degenerate.
        let horizontal = Vec3::new(yaw.sin(), 0.0, -yaw.cos()).normalize_or_zero();
        let right = horizontal.cross(Vec3::Y).normalize_or_zero();
        let up = face.cross(right).normalize_or_zero();
        let right = up.cross(face).normalize_or_zero();
        Quat::from_mat3(&glam::Mat3::from_cols(right, up, face))
    } else {
        Quat::from_rotation_arc(Vec3::Z, face)
    };
    state.world_matrix = Mat4::from_rotation_translation(rot, pos);
}

fn prim_box(
    panel: u16,
    rect: [f32; 4],
    clip: [f32; 4],
    color: u32,
    border: u32,
    binding: u32,
    payload: [u32; 4],
) -> GpuUiPrim {
    GpuUiPrim {
        rect,
        clip_rect: clip,
        payload,
        panel_and_kind: pack_panel_kind(panel, UiPrimKind::RoundedBox, 0),
        color_rgba: color,
        border_rgba: border,
        binding_id: binding,
    }
}

fn prim_number(
    panel: u16,
    rect: [f32; 4],
    clip: [f32; 4],
    value: f64,
    decimals: u8,
    color: u32,
    binding: u32,
) -> GpuUiPrim {
    GpuUiPrim {
        rect,
        clip_rect: clip,
        payload: pack_direct_number(value, decimals),
        panel_and_kind: pack_panel_kind(panel, UiPrimKind::DirectNumber, 0),
        color_rgba: color,
        border_rgba: pack_rgba8(0, 0, 0, 0),
        binding_id: binding,
    }
}

/// Pack Hierarchy window + optional Inspector into prims (panel 0).
///
/// Clamps scroll inside the same walk: if the window is empty and scroll > 0,
/// decrements and retries (no separate full-window clamp pass).
///
/// Returns how many hierarchy rows were emitted (for scroll UX).
pub fn pack_personal_lens(
    state: &mut SpatialLensState,
    epoch: &EpochViewState,
    out_headers: &mut Vec<GpuPanelHeader>,
    out_prims: &mut Vec<GpuUiPrim>,
) -> usize {
    out_headers.clear();
    out_prims.clear();
    if !state.visible {
        return 0;
    }

    let rows: Vec<HierarchyRowView> = loop {
        let rows = hierarchy_panel_rows(epoch, state.hierarchy_scroll, state.hierarchy_window);
        if !rows.is_empty() || state.hierarchy_scroll == 0 {
            break rows;
        }
        state.hierarchy_scroll -= 1;
    };
    let row_count = rows.len();

    let clip = [0.0, 0.0, state.width_px, state.height_px];
    let border = SHELL_BORDER.packed();
    let row_bg = CONTROL_IDLE.packed();
    let row_sel = ROW_SELECT.packed();
    let row_hov = CONTROL_HOT.packed();
    let num_col = TEXT.packed();
    // G0: active glow = laser/hover on this front-most board (Lens danger ②).
    // Visible alone stays at idle (0.20); selection does not raise shell glow.
    // Inset pad is always active-sized so hover does not jump the shell vs chrome/rows.
    let shell_active = state.hovered_binding != 0;

    out_prims.push(prim_box(
        0,
        shell_rect_inset(state.width_px, state.height_px),
        clip,
        SHELL_FILL.packed(),
        border,
        0,
        shell_payload(shell_active),
    ));

    let pad = content_pad();
    let chrome_h = 32.0;
    // Vertical margins use the same `pad` as horizontal (shell inset safe).
    let chrome_y = state.height_px - pad - chrome_h;

    // Title chrome: "ID" + entity; SoftLease → tint + "LK" + holder (x00f3).
    let (chrome_fill, chrome_border, lease_label_col) = match state.soft_lease {
        Some(SoftLeaseChrome { is_mine: true, .. }) => {
            (CHROME_FILL.packed(), ACCENT.packed(), ACCENT.packed())
        }
        Some(SoftLeaseChrome { is_mine: false, .. }) => {
            (CONTROL_DANGER.packed(), DANGER.packed(), DANGER.packed())
        }
        None => (CHROME_FILL.packed(), border, MUTED.packed()),
    };
    out_prims.push(prim_box(
        0,
        [pad, chrome_y, state.width_px - 2.0 * pad, chrome_h],
        clip,
        chrome_fill,
        chrome_border,
        0,
        control_payload(),
    ));
    push_ascii_glyphs(
        out_prims,
        AsciiGlyphBlit {
            panel: 0,
            origin_x: pad + 8.0,
            origin_y: chrome_y + 6.0,
            glyph_w: 14.0,
            glyph_h: 20.0,
            clip,
            color_rgba: num_col,
            binding_id: 0,
            text: "ID",
        },
    );
    let title_val = state
        .selected_row
        .and_then(|row| {
            epoch
                .snapshots
                .load_curr()
                .owner_scope
                .get(row as usize)
                .copied()
        })
        .map(f64::from)
        .unwrap_or(-1.0);
    out_prims.push(prim_number(
        0,
        [pad + 40.0, chrome_y + 4.0, 120.0, 24.0],
        clip,
        title_val,
        0,
        num_col,
        0,
    ));
    if let Some(lease) = state.soft_lease {
        let lk_x = pad + 170.0;
        push_ascii_glyphs(
            out_prims,
            AsciiGlyphBlit {
                panel: 0,
                origin_x: lk_x,
                origin_y: chrome_y + 6.0,
                glyph_w: 14.0,
                glyph_h: 20.0,
                clip,
                color_rgba: lease_label_col,
                binding_id: 0,
                text: "LK",
            },
        );
        // Zero-String: DirectNumber is f64-scaled int; chrome shows **lower 32 bits only**.
        // Distinct FNV actor_ids that collide in the low 32 bits look identical here
        // (full u64 / Presence name is non-guarantee · x00f3).
        let holder_lo = (lease.holder_actor_id & 0xffff_ffff) as f64;
        out_prims.push(prim_number(
            0,
            [lk_x + 36.0, chrome_y + 4.0, 160.0, 24.0],
            clip,
            holder_lo,
            0,
            lease_label_col,
            0,
        ));
    }

    let mut y = chrome_y - pad - ROW_H;

    for row in &rows {
        let binding = hierarchy_binding(row.row_id);
        let indent = pad + (row.depth as f32) * 14.0;
        let mut fill = row_bg;
        if state.selected_row == Some(row.row_id) {
            fill = row_sel;
        } else if state.hovered_binding == binding {
            fill = row_hov;
        }
        out_prims.push(prim_box(
            0,
            [indent, y, state.width_px - indent - pad, ROW_H - 2.0],
            clip,
            fill,
            border,
            binding,
            control_payload(),
        ));
        out_prims.push(prim_number(
            0,
            [indent + 8.0, y + 2.0, 100.0, ROW_H - 6.0],
            clip,
            f64::from(row.entity_id),
            0,
            num_col,
            binding,
        ));
        y -= ROW_H;
    }

    y -= pad;
    out_prims.push(prim_box(
        0,
        [pad, pad, state.width_px - 2.0 * pad, y.max(pad)],
        clip,
        CARD_FILL.packed(),
        border,
        0,
        card_payload(),
    ));

    if let Some(row_id) = state.selected_row {
        let draft = state.scrub.as_ref().filter(|s| s.row == row_id);
        pack_inspector_fields(
            epoch,
            row_id,
            &mut y,
            clip,
            state.width_px,
            out_prims,
            num_col,
            draft,
        );
    }

    out_headers.push(GpuPanelHeader {
        world_matrix: state.world_matrix.to_cols_array_2d(),
        dimensions: [
            state.width_px,
            state.height_px,
            state.width_m,
            state.height_m,
        ],
        prim_start_index: 0,
        prim_count: out_prims.len() as u32,
        lod_level: 0,
        _pad: 0,
    });
    row_count
}

#[allow(clippy::too_many_arguments)] // layout + draft scrub; keep flat like other pack helpers
fn pack_inspector_fields(
    epoch: &EpochViewState,
    row_id: u32,
    y_top: &mut f32,
    clip: [f32; 4],
    width_px: f32,
    out: &mut Vec<GpuUiPrim>,
    num_col: u32,
    draft: Option<&InspectorScrub>,
) {
    let faces = epoch.snapshots.load_pair();
    let idx = row_id as usize;
    if !row_is_live(epoch, row_id) {
        return;
    }
    let Some(xf) = faces.curr.world_transforms.get(idx) else {
        return;
    };
    let ready = faces
        .curr
        .readiness
        .get(idx)
        .map(|m| m.0 as f64)
        .unwrap_or(0.0);

    let mut px = xf.position[0];
    let mut py = xf.position[1];
    let mut pz = xf.position[2];
    if let Some(s) = draft {
        // Dirty axes: absolute draft; clean axes keep Curr (physics drift · x00f4).
        if s.dirty[0] {
            px = s.draft_xyz[0];
        }
        if s.dirty[1] {
            py = s.draft_xyz[1];
        }
        if s.dirty[2] {
            pz = s.draft_xyz[2];
        }
    }

    let fields = [
        (INSPECT_FIELD_POS_X, "X", px, 3u8),
        (INSPECT_FIELD_POS_Y, "Y", py, 3),
        (INSPECT_FIELD_POS_Z, "Z", pz, 3),
        (INSPECT_FIELD_READY, "R", ready, 0),
    ];
    let pad = content_pad();
    let mut y = (*y_top - pad).min(clip[3] - pad - 32.0);
    let label_w = 16.0;
    for (binding, label, value, dec) in fields {
        y -= ROW_H;
        if y < pad {
            break;
        }
        // Labels are decoration-only (hit goes to the DirectNumber field).
        push_ascii_glyphs(
            out,
            AsciiGlyphBlit {
                panel: 0,
                origin_x: pad * 2.0,
                origin_y: y + 4.0,
                glyph_w: label_w,
                glyph_h: ROW_H - 8.0,
                clip,
                color_rgba: num_col,
                binding_id: 0,
                text: label,
            },
        );
        out.push(prim_number(
            0,
            [
                pad * 2.0 + label_w + 4.0,
                y,
                width_px - 4.0 * pad - label_w - 4.0,
                ROW_H - 4.0,
            ],
            clip,
            value,
            dec,
            num_col,
            binding,
        ));
    }
}

/// Demo prims for Uber-SDF pipeline tests (no Epoch).
#[cfg_attr(not(test), allow(dead_code))]
pub fn pack_demo_panel(
    tick: f64,
    out_headers: &mut Vec<GpuPanelHeader>,
    out_prims: &mut Vec<GpuUiPrim>,
) {
    out_headers.clear();
    out_prims.clear();
    let w = 320.0_f32;
    let h = 180.0_f32;
    let clip = [0.0, 0.0, w, h];
    out_prims.push(prim_box(
        0,
        shell_rect_inset(w, h),
        clip,
        SHELL_FILL.packed(),
        SHELL_BORDER.packed(),
        0,
        shell_payload(true),
    ));
    push_ascii_glyphs(
        out_prims,
        AsciiGlyphBlit {
            panel: 0,
            origin_x: 24.0,
            origin_y: 36.0,
            glyph_w: 18.0,
            glyph_h: 28.0,
            clip,
            color_rgba: TEXT.packed(),
            binding_id: 0,
            text: "MSDF",
        },
    );
    out_prims.push(prim_number(
        0,
        [24.0, 80.0, 200.0, 36.0],
        clip,
        tick,
        3,
        TEXT.packed(),
        1,
    ));
    let m = Mat4::from_translation(Vec3::new(0.0, 1.4, -1.5));
    out_headers.push(GpuPanelHeader {
        world_matrix: m.to_cols_array_2d(),
        dimensions: [w, h, 0.64, 0.36],
        prim_start_index: 0,
        prim_count: out_prims.len() as u32,
        lod_level: 0,
        _pad: 0,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::epoch_view::{sync_topology_flat_roots, EpochViewState};
    use alchemy_shared_sim::tape::{EpochSheet, ReadinessMask, WorldTransformf64};
    use std::sync::Arc;

    fn epoch_with_one_row() -> EpochViewState {
        let state = EpochViewState::new();
        {
            let mut live = EpochSheet::with_capacity(2);
            let a = live.alloc_row();
            live.owner_scope[a.row_id as usize] = 42;
            live.set_transform(a.row_id, WorldTransformf64::identity_at([1.25, 2.5, -3.0]))
                .unwrap();
            state
                .snapshots
                .publish_arcs(Arc::new(live.clone()), Arc::new(live), 1);
        }
        sync_topology_flat_roots(&state);
        state
    }

    #[test]
    fn pack_lens_emits_hierarchy_and_inspector_numbers() {
        let epoch = epoch_with_one_row();
        let mut state = SpatialLensState {
            visible: true,
            selected_row: Some(0),
            ..Default::default()
        };
        let mut headers = Vec::new();
        let mut prims = Vec::new();
        let n = pack_personal_lens(&mut state, &epoch, &mut headers, &mut prims);
        assert_eq!(n, 1);
        assert_eq!(headers.len(), 1);
        assert!(prims.len() >= 3);
        assert!(prims
            .iter()
            .any(|p| unpack_is_number(p) && p.binding_id == hierarchy_binding(0)));
        assert!(prims
            .iter()
            .any(|p| p.binding_id == INSPECT_FIELD_POS_X && unpack_is_number(p)));
        state.hovered_binding = hierarchy_binding(0);
        pack_personal_lens(&mut state, &epoch, &mut headers, &mut prims);
        assert_eq!(headers[0].prim_count as usize, prims.len());
    }

    #[test]
    fn pack_lens_shell_insets_for_glow_and_idle_until_hover() {
        use super::super::glass::{
            shell_board_inset_pad, shell_payload, shell_rect_inset, SHELL_FILL, SHELL_GLOW_ACTIVE,
        };
        use super::super::prim::unpack_kind;

        let epoch = epoch_with_one_row();
        let mut state = SpatialLensState {
            visible: true,
            selected_row: Some(0),
            hovered_binding: 0,
            ..Default::default()
        };
        let mut headers = Vec::new();
        let mut prims = Vec::new();
        pack_personal_lens(&mut state, &epoch, &mut headers, &mut prims);
        let shell = prims
            .iter()
            .find(|p| {
                unpack_kind(p.panel_and_kind) == UiPrimKind::RoundedBox
                    && p.binding_id == 0
                    && p.color_rgba == SHELL_FILL.packed()
            })
            .expect("shell RoundedBox");
        let expected_rect = shell_rect_inset(state.width_px, state.height_px);
        // Selection alone keeps idle glow; inset always active-sized (no hover jump).
        assert_eq!(shell.payload, shell_payload(false));
        assert_eq!(shell.rect, expected_rect);
        let pad = shell_board_inset_pad();
        assert!((shell.rect[0] - pad).abs() < 1e-4);
        assert!(shell.rect[0] + shell.rect[2] + pad <= state.width_px + 1e-3);

        state.hovered_binding = hierarchy_binding(0);
        pack_personal_lens(&mut state, &epoch, &mut headers, &mut prims);
        let shell = prims
            .iter()
            .find(|p| {
                unpack_kind(p.panel_and_kind) == UiPrimKind::RoundedBox
                    && p.binding_id == 0
                    && p.color_rgba == SHELL_FILL.packed()
            })
            .expect("shell RoundedBox active");
        assert_eq!(shell.payload, shell_payload(true));
        assert_eq!(f32::from_bits(shell.payload[2]), SHELL_GLOW_ACTIVE);
        assert_eq!(shell.rect, expected_rect);
    }

    #[test]
    fn secondary_axis_cycles_and_world_pos_helpers() {
        assert_eq!(inspector_secondary_axis(0), 1);
        assert_eq!(inspector_secondary_axis(1), 2);
        assert_eq!(inspector_secondary_axis(2), 0);
        assert_eq!(
            pos_field_id_for_axis(0),
            Some(alchemy_protocol::edit_field_id::POS_X)
        );
        let epoch = epoch_with_one_row();
        assert_eq!(world_pos_xyz(&epoch, 0), Some([1.25, 2.5, -3.0]));
        assert_eq!(entity_id_for_row(&epoch, 0), Some(42));
    }

    #[test]
    fn scrub_selected_row_clears_tombstone() {
        let epoch = epoch_with_one_row();
        let mut state = SpatialLensState {
            visible: true,
            selected_row: Some(0),
            ..Default::default()
        };
        {
            let mut live = (*epoch.snapshots.load_curr()).clone();
            live.readiness[0] = ReadinessMask(ReadinessMask::TOMBSTONE);
            epoch
                .snapshots
                .publish_arcs(Arc::new(live.clone()), Arc::new(live), 2);
        }
        assert_eq!(scrub_selected_row(&mut state, &epoch), None);
        assert_eq!(state.selected_row, None);
    }

    #[test]
    fn scrub_selected_row_returns_entity_when_scrub_dropped() {
        let epoch = epoch_with_one_row();
        let mut state = SpatialLensState {
            visible: true,
            selected_row: Some(0),
            scrub: Some(InspectorScrub {
                field_binding: INSPECT_FIELD_POS_X,
                row: 0,
                entity_id: 42,
                start_uv: [0.0, 0.0],
                last_uv: [10.0, 20.0],
                base_xyz: [0.0, 0.0, 0.0],
                draft_xyz: [0.0, 0.0, 0.0],
                dirty: [false; 3],
                expected_hash: [[0; 32]; 3],
            }),
            ..Default::default()
        };
        {
            let mut live = (*epoch.snapshots.load_curr()).clone();
            live.readiness[0] = ReadinessMask(ReadinessMask::TOMBSTONE);
            epoch
                .snapshots
                .publish_arcs(Arc::new(live.clone()), Arc::new(live), 2);
        }
        assert_eq!(scrub_selected_row(&mut state, &epoch), Some(42));
        assert!(state.scrub.is_none());
    }

    fn unpack_is_number(p: &GpuUiPrim) -> bool {
        super::super::prim::unpack_kind(p.panel_and_kind) == UiPrimKind::DirectNumber
    }

    #[test]
    fn demo_panel_has_direct_number() {
        let mut h = Vec::new();
        let mut p = Vec::new();
        pack_demo_panel(12.345, &mut h, &mut p);
        assert_eq!(h.len(), 1);
        assert!(p.iter().any(unpack_is_number));
        assert!(p.iter().any(unpack_is_msdf));
    }

    fn unpack_is_msdf(p: &GpuUiPrim) -> bool {
        super::super::prim::unpack_kind(p.panel_and_kind) == UiPrimKind::MsdfGlyph
    }

    #[test]
    fn pack_lens_emits_msdf_title_and_inspector_labels() {
        let epoch = epoch_with_one_row();
        let mut state = SpatialLensState {
            visible: true,
            selected_row: Some(0),
            ..Default::default()
        };
        let mut headers = Vec::new();
        let mut prims = Vec::new();
        pack_personal_lens(&mut state, &epoch, &mut headers, &mut prims);
        let msdf: Vec<_> = prims.iter().filter(|p| unpack_is_msdf(p)).collect();
        // "ID" (2) + X/Y/Z/R labels (4) minimum when inspector open.
        assert!(msdf.len() >= 6, "msdf_n={}", msdf.len());
        // Decoration must not steal laser hits from DirectNumber fields.
        assert!(
            msdf.iter().all(|p| p.binding_id == 0),
            "MsdfGlyph labels must use binding_id=0"
        );
    }

    #[test]
    fn pack_lens_soft_lease_chrome_emits_lk_and_danger_fill() {
        use super::super::glass::CONTROL_DANGER;
        use super::super::prim::unpack_kind;

        let epoch = epoch_with_one_row();
        let mut state = SpatialLensState {
            visible: true,
            selected_row: Some(0),
            soft_lease: Some(SoftLeaseChrome {
                holder_actor_id: 0x1234_5678_9abc_def0,
                is_mine: false,
            }),
            ..Default::default()
        };
        let mut headers = Vec::new();
        let mut prims = Vec::new();
        pack_personal_lens(&mut state, &epoch, &mut headers, &mut prims);
        assert!(
            prims.iter().any(|p| {
                unpack_kind(p.panel_and_kind) == UiPrimKind::RoundedBox
                    && p.color_rgba == CONTROL_DANGER.packed()
            }),
            "other SoftLease chrome uses CONTROL_DANGER"
        );
        let msdf: Vec<_> = prims.iter().filter(|p| unpack_is_msdf(p)).collect();
        // "ID" (2) + "LK" (2) + inspector labels.
        assert!(msdf.len() >= 8, "msdf_n={}", msdf.len());
        // Lower 32 bits of holder as DirectNumber.
        let holder_lo = 0x9abc_def0u32 as f64;
        assert!(
            prims.iter().any(|p| {
                unpack_is_number(p)
                    && p.binding_id == 0
                    && (super::super::prim::unpack_direct_number(p.payload) - holder_lo).abs() < 0.5
            }),
            "holder DirectNumber missing"
        );
    }

    fn eye_at_origin() -> EntityPose {
        EntityPose::default()
    }

    fn panel_center(state: &SpatialLensState) -> Vec3 {
        let m = state.world_matrix;
        m.transform_point3(Vec3::ZERO)
    }

    fn panel_face_normal(state: &SpatialLensState) -> Vec3 {
        state
            .world_matrix
            .transform_vector3(Vec3::Z)
            .normalize_or_zero()
    }

    #[test]
    fn place_in_front_look_along_neg_z_uses_degenerate_basis() {
        // yaw=0,pitch=0 → forward=-Z → face=+Z (parallel to local +Z).
        let mut state = SpatialLensState::default();
        let look = CameraLook {
            yaw: 0.0,
            pitch: 0.0,
        };
        place_in_front_of_eye(&mut state, eye_at_origin(), look, 1.4);
        let center = panel_center(&state);
        assert!(
            (center - Vec3::new(0.0, 0.0, -1.4)).length() < 1e-4,
            "center={center:?}"
        );
        let n = panel_face_normal(&state);
        assert!(
            (n - Vec3::Z).length() < 1e-3 || (n + Vec3::Z).length() < 1e-3,
            "normal={n:?}"
        );
        assert!(state.world_matrix.is_finite());
    }

    #[test]
    fn place_in_front_look_along_pos_z_uses_degenerate_basis() {
        // yaw=π → forward=+Z → face=-Z.
        let mut state = SpatialLensState::default();
        let look = CameraLook {
            yaw: std::f32::consts::PI,
            pitch: 0.0,
        };
        place_in_front_of_eye(&mut state, eye_at_origin(), look, 1.4);
        let center = panel_center(&state);
        assert!(
            (center - Vec3::new(0.0, 0.0, 1.4)).length() < 1e-3,
            "center={center:?}"
        );
        assert!(state.world_matrix.is_finite());
        let n = panel_face_normal(&state);
        assert!(n.is_finite() && n.length() > 0.9, "normal={n:?}");
    }

    #[test]
    fn place_in_front_near_zenith_stays_on_look_ray() {
        let mut state = SpatialLensState::default();
        let look = CameraLook {
            yaw: 0.3,
            pitch: 1.54,
        };
        place_in_front_of_eye(&mut state, eye_at_origin(), look, 1.4);
        let forward = Vec3::new(
            look.yaw.sin() * look.pitch.cos(),
            look.pitch.sin(),
            -look.yaw.cos() * look.pitch.cos(),
        )
        .normalize_or_zero();
        let center = panel_center(&state);
        let along = center.dot(forward);
        assert!(
            (along - 1.4).abs() < 1e-3,
            "along={along} center={center:?}"
        );
        assert!((center - forward * along).length() < 1e-3);
        assert!(state.world_matrix.is_finite());
    }

    #[test]
    fn place_in_front_near_nadir_stays_on_look_ray() {
        let mut state = SpatialLensState::default();
        let look = CameraLook {
            yaw: -0.5,
            pitch: -1.54,
        };
        place_in_front_of_eye(&mut state, eye_at_origin(), look, 1.4);
        let forward = Vec3::new(
            look.yaw.sin() * look.pitch.cos(),
            look.pitch.sin(),
            -look.yaw.cos() * look.pitch.cos(),
        )
        .normalize_or_zero();
        let center = panel_center(&state);
        let along = center.dot(forward);
        assert!((along - 1.4).abs() < 1e-3, "along={along}");
        assert!(state.world_matrix.is_finite());
    }
}
