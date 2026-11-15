//! Panel OBB → local UV → binding lookup (x00bd).
//!
//! Guarantees: ray vs panel board; O(n_prims) rect pick inside clip (n small = window).
//! Non-guarantees: Flat Morton LBVH (panel count stays tiny for personal lenses).

#![forbid(unsafe_code)]

use glam::{Mat4, Quat, Vec3};

use super::prim::{unpack_kind, GpuPanelHeader, GpuUiPrim, UiPrimKind};
use crate::world_hit::{ray_hit_obb, Ray};

/// Hit result on a spatial lens panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelHit {
    pub panel_index: u32,
    pub t: f32,
    pub uv_px: [f32; 2],
    pub binding_id: u32,
    pub rect_px: [f32; 4],
}

/// World-space center / rotation / half-extents for a panel board (thin OBB).
pub fn panel_obb(header: &GpuPanelHeader) -> (Vec3, Quat, Vec3) {
    let m = Mat4::from_cols_array_2d(&header.world_matrix);
    let (_, rot, pos) = m.to_scale_rotation_translation();
    let half_w = header.dimensions[2] * 0.5;
    let half_h = header.dimensions[3] * 0.5;
    let half_d = 0.01; // thin board
    (pos, rot, Vec3::new(half_w, half_h, half_d))
}

/// Ray → panel-local pixel UV. Origin of panel local XY is bottom-left of the board face.
pub fn ray_panel_uv(ray: Ray, header: &GpuPanelHeader) -> Option<(f32, [f32; 2])> {
    let (center, rot, half) = panel_obb(header);
    let t = ray_hit_obb(ray, center, rot, half)?;
    let hit = ray.origin + ray.dir * t;
    let local = rot.inverse() * (hit - center);
    let w_m = header.dimensions[2].max(1e-6);
    let h_m = header.dimensions[3].max(1e-6);
    let w_px = header.dimensions[0];
    let h_px = header.dimensions[1];
    // Local X/Y in meters → pixels (center origin → bottom-left).
    let u = (local.x / w_m + 0.5) * w_px;
    let v = (local.y / h_m + 0.5) * h_px;
    if u < 0.0 || v < 0.0 || u > w_px || v > h_px {
        return None;
    }
    Some((t, [u, v]))
}

/// Pick interactive prim under UV (top-most by list order = last wins).
pub fn pick_binding_at(
    prims: &[GpuUiPrim],
    start: u32,
    count: u32,
    uv_px: [f32; 2],
) -> Option<(u32, [f32; 4])> {
    let start = start as usize;
    let end = start.saturating_add(count as usize).min(prims.len());
    let mut best: Option<(u32, [f32; 4])> = None;
    for p in &prims[start..end] {
        if p.binding_id == 0 {
            continue;
        }
        match unpack_kind(p.panel_and_kind) {
            UiPrimKind::RoundedBox | UiPrimKind::DirectNumber | UiPrimKind::MsdfGlyph => {}
            UiPrimKind::CubicBezierWire => continue,
        }
        let [x, y, w, h] = p.rect;
        let [c0, c1, c2, c3] = p.clip_rect;
        if uv_px[0] < c0 || uv_px[1] < c1 || uv_px[0] > c2 || uv_px[1] > c3 {
            continue;
        }
        if uv_px[0] >= x && uv_px[0] <= x + w && uv_px[1] >= y && uv_px[1] <= y + h {
            best = Some((p.binding_id, p.rect));
        }
    }
    best
}

/// Nearest panel hit among headers.
pub fn pick_panel(ray: Ray, headers: &[GpuPanelHeader], prims: &[GpuUiPrim]) -> Option<PanelHit> {
    let mut best: Option<PanelHit> = None;
    for (i, h) in headers.iter().enumerate() {
        let Some((t, uv)) = ray_panel_uv(ray, h) else {
            continue;
        };
        if let Some(b) = &best {
            if t >= b.t {
                continue;
            }
        }
        let (binding, rect) =
            pick_binding_at(prims, h.prim_start_index, h.prim_count, uv).unwrap_or((0, [0.0; 4]));
        best = Some(PanelHit {
            panel_index: i as u32,
            t,
            uv_px: uv,
            binding_id: binding,
            rect_px: rect,
        });
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Mat4;

    fn facing_panel() -> GpuPanelHeader {
        // Panel at z=-1.5, facing +Z (toward origin), 0.8m x 0.6m, 400x300 px.
        let m = Mat4::from_translation(Vec3::new(0.0, 1.4, -1.5));
        GpuPanelHeader {
            world_matrix: m.to_cols_array_2d(),
            dimensions: [400.0, 300.0, 0.8, 0.6],
            prim_start_index: 0,
            prim_count: 1,
            lod_level: 0,
            _pad: 0,
        }
    }

    #[test]
    fn ray_hits_panel_center() {
        let h = facing_panel();
        let ray = Ray {
            origin: Vec3::new(0.0, 1.4, 0.0),
            dir: Vec3::new(0.0, 0.0, -1.0),
        };
        let (t, uv) = ray_panel_uv(ray, &h).expect("hit");
        assert!(t > 1.0 && t < 2.0);
        assert!((uv[0] - 200.0).abs() < 5.0, "u={}", uv[0]);
        assert!((uv[1] - 150.0).abs() < 5.0, "v={}", uv[1]);
    }

    #[test]
    fn pick_binding_respects_rect() {
        let prim = GpuUiPrim {
            rect: [10.0, 20.0, 80.0, 24.0],
            clip_rect: [0.0, 0.0, 400.0, 300.0],
            binding_id: 99,
            panel_and_kind: super::super::prim::pack_panel_kind(0, UiPrimKind::RoundedBox, 0),
            ..Default::default()
        };
        let hit = pick_binding_at(&[prim], 0, 1, [50.0, 30.0]).expect("bind");
        assert_eq!(hit.0, 99);
        assert!(pick_binding_at(&[prim], 0, 1, [5.0, 5.0]).is_none());
    }
}
