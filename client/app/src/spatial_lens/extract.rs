//! Hierarchy / Inspector → GpuUiPrim packing (x00bb / x00bc).
//!
//! Guarantees: viewport-window extract only; DirectNumber pose fields without String;
//! selected_row scrub when row is not live; scroll clamp to non-empty window.
//! Non-guarantees: MSDF labels; SetField scrub commit; WorldRoot sync; Lens-side collapse
//! toggle (collapse set is shared with Settings; Lens MVP reads only).

#![forbid(unsafe_code)]

use glam::{Mat4, Quat, Vec3};

use super::glass::{
    card_payload, control_payload, shell_payload, shell_rect_inset, CARD_FILL, CHROME_FILL,
    CONTROL_HOT, CONTROL_IDLE, ROW_SELECT, SHELL_BORDER, SHELL_FILL, TEXT,
};
use super::prim::{
    pack_direct_number, pack_panel_kind, pack_rgba8, GpuPanelHeader, GpuUiPrim, UiPrimKind,
};
use crate::epoch_view::{hierarchy_panel_rows, EpochViewState, HierarchyRowView};
use crate::scene::{CameraLook, EntityPose};

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
pub fn scrub_selected_row(state: &mut SpatialLensState, epoch: &EpochViewState) {
    if let Some(row) = state.selected_row {
        if !row_is_live(epoch, row) {
            state.selected_row = None;
        }
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

    // Title chrome (MSDF later). Number = selected entity_id, or -1 when none.
    out_prims.push(prim_box(
        0,
        [pad, chrome_y, state.width_px - 2.0 * pad, chrome_h],
        clip,
        CHROME_FILL.packed(),
        border,
        0,
        control_payload(),
    ));
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
        [pad + 8.0, chrome_y + 4.0, 160.0, 24.0],
        clip,
        title_val,
        0,
        num_col,
        0,
    ));

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
        pack_inspector_fields(
            epoch,
            row_id,
            &mut y,
            clip,
            state.width_px,
            out_prims,
            num_col,
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

fn pack_inspector_fields(
    epoch: &EpochViewState,
    row_id: u32,
    y_top: &mut f32,
    clip: [f32; 4],
    width_px: f32,
    out: &mut Vec<GpuUiPrim>,
    num_col: u32,
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

    let fields = [
        (INSPECT_FIELD_POS_X, xf.position[0], 3u8),
        (INSPECT_FIELD_POS_Y, xf.position[1], 3),
        (INSPECT_FIELD_POS_Z, xf.position[2], 3),
        (INSPECT_FIELD_READY, ready, 0),
    ];
    let pad = content_pad();
    let mut y = (*y_top - pad).min(clip[3] - pad - 32.0);
    for (binding, value, dec) in fields {
        y -= ROW_H;
        if y < pad {
            break;
        }
        out.push(prim_number(
            0,
            [pad * 2.0, y, width_px - 4.0 * pad, ROW_H - 4.0],
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
        scrub_selected_row(&mut state, &epoch);
        assert_eq!(state.selected_row, None);
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
