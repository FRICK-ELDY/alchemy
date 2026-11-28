//! XR UI intent: menu toggle + controller ray vs head-locked dash quad / world boxes.
//!
//! system_ui stays device-agnostic; this module maps XR poses → canvas pixels.
//! Laser beam model matrices are pure here (session only draws).
//! World OBB hits live in [`crate::world_hit`] (shared with Desktop Grab; x002b).
//! Grab hold Intent (entity + world aim) is written in wgpu_session (x0011).

use glam::{Mat4, Quat, Vec3};

use crate::scene::EntityPose;
pub use crate::world_hit::Ray;
use crate::world_hit::MAX_HIT_DISTANCE as LASER_MAX_LENGTH;

/// Head-locked dash quad in view/local space (meters). Centered, facing -Z of head.
/// SSoT: [`crate::spatial_lens::DASH_WIDTH_M`] / `DASH_HEIGHT_M` / `DASH_DISTANCE_M`.
pub const QUAD_WIDTH: f32 = crate::spatial_lens::DASH_WIDTH_M;
pub const QUAD_HEIGHT: f32 = crate::spatial_lens::DASH_HEIGHT_M;
pub const QUAD_DISTANCE: f32 = crate::spatial_lens::DASH_DISTANCE_M;

/// Stop the beam this far short of the dash plane so the tip sits in front (same projection as UI).
pub const LASER_TIP_GAP: f32 = 0.02;

/// Half-extent of the unit laser box on X/Y before scale (meters at length=1 → scaled by radius).
pub const LASER_RADIUS: f32 = 0.004;

/// Plane hit of aim ray vs dash quad. `t` is ray distance; `(px,py)` are canvas pixels if in bounds.
#[derive(Clone, Copy, Debug)]
pub struct DashQuadHit {
    pub t: f32,
    pub px: f32,
    pub py: f32,
}

/// Intersect ray with the dash quad plane. Returns canvas pixel coords if hit (front face).
pub fn ray_hit_dash_quad(
    head_pos: Vec3,
    head_orient: Quat,
    ray: Ray,
    canvas_w: f32,
    canvas_h: f32,
) -> Option<(f32, f32)> {
    ray_hit_dash_quad_full(head_pos, head_orient, ray, canvas_w, canvas_h).map(|h| (h.px, h.py))
}

/// Same as [`ray_hit_dash_quad`] but also returns ray distance `t`.
pub fn ray_hit_dash_quad_full(
    head_pos: Vec3,
    head_orient: Quat,
    ray: Ray,
    canvas_w: f32,
    canvas_h: f32,
) -> Option<DashQuadHit> {
    let forward = head_orient * Vec3::NEG_Z;
    let right = head_orient * Vec3::X;
    let up = head_orient * Vec3::Y;
    let center = head_pos + forward * QUAD_DISTANCE;
    let normal = -forward;

    let denom = ray.dir.dot(normal);
    if denom.abs() < 1e-5 {
        return None;
    }
    let t = (center - ray.origin).dot(normal) / denom;
    if t < 0.0 {
        return None;
    }
    let hit = ray.origin + ray.dir * t;
    let local = hit - center;
    let x = local.dot(right);
    let y = local.dot(up);
    let half_w = QUAD_WIDTH * 0.5;
    let half_h = QUAD_HEIGHT * 0.5;
    if x.abs() > half_w || y.abs() > half_h {
        return None;
    }
    // Canvas: origin top-left, +Y down (same as Desktop mouse).
    let u = (x / QUAD_WIDTH) + 0.5;
    let v = 0.5 - (y / QUAD_HEIGHT);
    Some(DashQuadHit {
        t,
        px: u * canvas_w,
        py: v * canvas_h,
    })
}

/// Beam length for drawing: hit distance minus [`LASER_TIP_GAP`], or [`LASER_MAX_LENGTH`] on miss.
pub fn laser_length(
    head_pos: Vec3,
    head_orient: Quat,
    ray: Ray,
    canvas_w: f32,
    canvas_h: f32,
) -> f32 {
    ray_hit_dash_quad_full(head_pos, head_orient, ray, canvas_w, canvas_h)
        .map(|h| (h.t - LASER_TIP_GAP).max(0.01))
        .unwrap_or(LASER_MAX_LENGTH)
        .clamp(0.01, LASER_MAX_LENGTH)
}

/// Beam length against grabbable box AABBs (dash closed / world pointer, x0010).
pub fn laser_length_world(ray: Ray, boxes: &[(u32, EntityPose)]) -> f32 {
    crate::world_hit::pick_grabbable(ray, boxes)
        .map(|h| (h.t - LASER_TIP_GAP).max(0.01))
        .unwrap_or(LASER_MAX_LENGTH)
        .clamp(0.01, LASER_MAX_LENGTH)
}

pub fn pose_to_ray(pos: Vec3, orient: Quat) -> Ray {
    Ray {
        origin: pos,
        dir: (orient * Vec3::NEG_Z).normalize_or_zero(),
    }
}

/// Align `raw_orient` so local -Z matches `smoothed_dir`, preserving twist (x00f5 L2).
///
/// Uses the shortest arc from the raw forward to the smoothed unit direction.
/// Degenerate / near-zero `smoothed_dir` returns `raw_orient` unchanged.
/// Near-opposite (`dot ≈ -1`) uses a stable perpendicular 180° flip (avoids
/// `from_rotation_arc` axis jitter).
pub fn aim_orient_from_smoothed_dir(raw_orient: Quat, smoothed_dir: Vec3) -> Quat {
    let target = smoothed_dir.normalize_or_zero();
    if target.length_squared() < 1e-12 {
        return raw_orient;
    }
    let raw_fwd = (raw_orient * Vec3::NEG_Z).normalize_or_zero();
    if raw_fwd.length_squared() < 1e-12 {
        return Quat::from_rotation_arc(Vec3::NEG_Z, target) * raw_orient;
    }
    let dot = raw_fwd.dot(target);
    if dot < -0.999_999 {
        let helper = if raw_fwd.x.abs() < 0.9 {
            Vec3::X
        } else {
            Vec3::Y
        };
        let axis = raw_fwd.cross(helper).normalize_or_zero();
        if axis.length_squared() < 1e-12 {
            return Quat::from_rotation_arc(Vec3::NEG_Z, target) * raw_orient;
        }
        return Quat::from_axis_angle(axis, std::f32::consts::PI) * raw_orient;
    }
    Quat::from_rotation_arc(raw_fwd, target) * raw_orient
}

pub fn openxr_pose_to_isometry(pose: openxr::Posef) -> (Vec3, Quat) {
    let p = pose.position;
    let o = pose.orientation;
    (
        Vec3::new(p.x, p.y, p.z),
        Quat::from_xyzw(o.x, o.y, o.z, o.w),
    )
}

/// Model matrix for the dash quad (world), for drawing.
pub fn dash_quad_model(head_pos: Vec3, head_orient: Quat) -> Mat4 {
    let forward = head_orient * Vec3::NEG_Z;
    let center = head_pos + forward * QUAD_DISTANCE;
    Mat4::from_scale_rotation_translation(
        Vec3::new(QUAD_WIDTH, QUAD_HEIGHT, 1.0),
        head_orient,
        center,
    )
}

/// Freeze Context board in world at open/re-anchor (aim forward + face head).
///
/// Returns `(center, face_orient, model)`. Board stays put until close / re-anchor.
pub fn freeze_context_board(
    aim_pos: Vec3,
    aim_orient: Quat,
    head_orient: Quat,
    canvas_w: f32,
    canvas_h: f32,
) -> (Vec3, Quat, Mat4) {
    let forward = aim_orient * Vec3::NEG_Z;
    let center = aim_pos + forward * crate::context_radial::CONTEXT_FORWARD_M;
    let model = context_board_model_at(center, head_orient, canvas_w, canvas_h);
    (center, head_orient, model)
}

/// Model matrix for a world-anchored Context board (aspect-matched to canvas).
pub fn context_board_model_at(
    center: Vec3,
    face_orient: Quat,
    canvas_w: f32,
    canvas_h: f32,
) -> Mat4 {
    let (bw, bh) = crate::context_radial::context_board_size(canvas_w, canvas_h);
    Mat4::from_scale_rotation_translation(Vec3::new(bw, bh, 1.0), face_orient, center)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ContextBoardRayHit {
    /// Inside the board rect — use exact UV.
    Inside { px: f32, py: f32 },
    /// Outside hard rect but within leave pad — pointer clamped to board edge.
    Edge { px: f32, py: f32 },
    /// No plane hit or beyond leave pad.
    Miss,
}

/// Classify aim vs frozen board: Inside / Edge (keep open) / Miss (leave candidate).
pub fn classify_context_board_ray(
    board_center: Vec3,
    board_orient: Quat,
    ray: Ray,
    canvas_w: f32,
    canvas_h: f32,
) -> ContextBoardRayHit {
    let Some((_t, x, y)) = context_board_hit_local(board_center, board_orient, ray) else {
        return ContextBoardRayHit::Miss;
    };
    let (board_w, board_h) = crate::context_radial::context_board_size(canvas_w, canvas_h);
    let half_w = board_w * 0.5;
    let half_h = board_h * 0.5;
    let leave_w = half_w * crate::context_radial::MENU_LEAVE_PAD;
    let leave_h = half_h * crate::context_radial::MENU_LEAVE_PAD;
    if x.abs() > leave_w || y.abs() > leave_h {
        return ContextBoardRayHit::Miss;
    }
    let cx = x.clamp(-half_w, half_w);
    let cy = y.clamp(-half_h, half_h);
    let u = (cx / board_w) + 0.5;
    let v = 0.5 - (cy / board_h);
    let px = u * canvas_w;
    let py = v * canvas_h;
    if x.abs() <= half_w && y.abs() <= half_h {
        ContextBoardRayHit::Inside { px, py }
    } else {
        ContextBoardRayHit::Edge { px, py }
    }
}

/// Laser length to the frozen Context board plane (or max if miss).
pub fn laser_length_context_board(board_center: Vec3, board_orient: Quat, ray: Ray) -> f32 {
    match context_board_hit_local(board_center, board_orient, ray) {
        Some((t, _, _)) if t > 0.0 => (t - LASER_TIP_GAP).max(0.01),
        _ => LASER_MAX_LENGTH,
    }
}

fn context_board_hit_local(
    board_center: Vec3,
    board_orient: Quat,
    ray: Ray,
) -> Option<(f32, f32, f32)> {
    let forward = board_orient * Vec3::NEG_Z;
    let right = board_orient * Vec3::X;
    let up = board_orient * Vec3::Y;
    let normal = -forward;
    let denom = ray.dir.dot(normal);
    if denom.abs() < 1e-5 {
        return None;
    }
    let t = (board_center - ray.origin).dot(normal) / denom;
    if t < 0.0 {
        return None;
    }
    let hit = ray.origin + ray.dir * t;
    let local = hit - board_center;
    Some((t, local.dot(right), local.dot(up)))
}

/// Model for a unit laser box along local -Z from origin to length 1, scaled to `length`.
///
/// Mesh is expected as a box from z=0 .. z=-1 (aim forward = -Z), cross-section ±0.5 on X/Y
/// before `LASER_RADIUS` scale (so radius = LASER_RADIUS when half-extent was 0.5).
pub fn laser_model(origin: Vec3, orient: Quat, length: f32) -> Mat4 {
    let len = length.max(0.01);
    Mat4::from_scale_rotation_translation(
        Vec3::new(LASER_RADIUS * 2.0, LASER_RADIUS * 2.0, len),
        orient,
        origin,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_ray_hits_quad_center() {
        let head_pos = Vec3::new(0.0, 1.6, 0.0);
        let head_orient = Quat::IDENTITY;
        let ray = Ray {
            origin: head_pos,
            dir: Vec3::NEG_Z,
        };
        let (px, py) = ray_hit_dash_quad(head_pos, head_orient, ray, 960.0, 540.0).unwrap();
        assert!((px - 480.0).abs() < 1.0, "px={px}");
        assert!((py - 270.0).abs() < 1.0, "py={py}");
    }

    #[test]
    fn miss_when_ray_goes_sideways() {
        let head_pos = Vec3::ZERO;
        let head_orient = Quat::IDENTITY;
        let ray = Ray {
            origin: head_pos,
            dir: Vec3::X,
        };
        assert!(ray_hit_dash_quad(head_pos, head_orient, ray, 100.0, 100.0).is_none());
    }

    #[test]
    fn center_hit_length_stops_short_of_quad() {
        let head_pos = Vec3::new(0.0, 1.6, 0.0);
        let head_orient = Quat::IDENTITY;
        let ray = Ray {
            origin: head_pos,
            dir: Vec3::NEG_Z,
        };
        let len = laser_length(head_pos, head_orient, ray, 960.0, 540.0);
        let expected = QUAD_DISTANCE - LASER_TIP_GAP;
        assert!(
            (len - expected).abs() < 0.01,
            "len={len} expected ~{expected}"
        );
    }

    #[test]
    fn miss_uses_max_length() {
        let head_pos = Vec3::ZERO;
        let head_orient = Quat::IDENTITY;
        let ray = Ray {
            origin: head_pos,
            dir: Vec3::X,
        };
        assert!(
            (laser_length(head_pos, head_orient, ray, 100.0, 100.0) - LASER_MAX_LENGTH).abs()
                < 1e-5
        );
    }

    #[test]
    fn laser_model_places_origin_at_aim() {
        let origin = Vec3::new(1.0, 2.0, 3.0);
        let m = laser_model(origin, Quat::IDENTITY, 2.0);
        let tip_local = Vec3::new(0.0, 0.0, -1.0);
        let tip = m.transform_point3(tip_local);
        assert!(
            (tip - Vec3::new(1.0, 2.0, 1.0)).length() < 1e-4,
            "tip={tip:?}"
        );
    }

    #[test]
    fn aim_orient_from_smoothed_dir_points_neg_z() {
        let raw = Quat::from_rotation_y(0.4);
        let target = Vec3::new(0.2, 0.5, -0.8).normalize();
        let aligned = aim_orient_from_smoothed_dir(raw, target);
        assert!((aligned.length() - 1.0).abs() < 1e-5);
        let fwd = (aligned * Vec3::NEG_Z).normalize();
        assert!(
            (fwd - target).length() < 1e-4,
            "fwd={fwd:?} target={target:?}"
        );
    }

    #[test]
    fn aim_orient_from_smoothed_dir_zero_keeps_raw() {
        let raw = Quat::from_rotation_x(0.3);
        let aligned = aim_orient_from_smoothed_dir(raw, Vec3::ZERO);
        assert!((aligned.dot(raw) - 1.0).abs() < 1e-5 || (aligned.dot(raw) + 1.0).abs() < 1e-5);
    }

    #[test]
    fn aim_orient_near_opposite_stays_unit_and_points_target() {
        let raw = Quat::IDENTITY; // -Z
        let target = Vec3::Z; // opposite
        let aligned = aim_orient_from_smoothed_dir(raw, target);
        assert!((aligned.length() - 1.0).abs() < 1e-4);
        let fwd = (aligned * Vec3::NEG_Z).normalize();
        assert!((fwd - target).length() < 1e-3, "fwd={fwd:?}");
        // Second call must not jitter the forward (stable axis).
        let again = aim_orient_from_smoothed_dir(raw, target);
        let fwd2 = (again * Vec3::NEG_Z).normalize();
        assert!((fwd - fwd2).length() < 1e-5);
    }

    #[test]
    fn aim_orient_pose_to_ray_matches_smoothed_and_keeps_twist() {
        let pos = Vec3::new(1.0, 2.0, 3.0);
        let raw = Quat::from_euler(glam::EulerRot::YXZ, 0.35, -0.2, 0.55);
        let raw_up = (raw * Vec3::Y).normalize();
        let target = Vec3::new(0.15, 0.4, -0.9).normalize();
        let aligned = aim_orient_from_smoothed_dir(raw, target);
        let ray = pose_to_ray(pos, aligned);
        assert_eq!(ray.origin, pos);
        assert!(
            (ray.dir - target).length() < 1e-4,
            "ray.dir={:?} target={target:?}",
            ray.dir
        );
        // Twist: local +Y after align should stay near the raw up projected off target.
        let aligned_up = (aligned * Vec3::Y).normalize();
        let raw_up_rej = (raw_up - target * raw_up.dot(target)).normalize_or_zero();
        let aligned_up_rej = (aligned_up - target * aligned_up.dot(target)).normalize_or_zero();
        if raw_up_rej.length_squared() > 1e-6 && aligned_up_rej.length_squared() > 1e-6 {
            assert!(
                raw_up_rej.dot(aligned_up_rej) > 0.98,
                "twist drifted: raw_up_rej={raw_up_rej:?} aligned_up_rej={aligned_up_rej:?}"
            );
        }
    }

    #[test]
    fn frozen_context_board_hit_at_center() {
        let aim_pos = Vec3::new(0.0, 1.4, 0.0);
        let aim_orient = Quat::IDENTITY;
        let head_orient = Quat::IDENTITY;
        let (center, orient, _) =
            freeze_context_board(aim_pos, aim_orient, head_orient, 960.0, 540.0);
        let ray = pose_to_ray(aim_pos, aim_orient);
        match classify_context_board_ray(center, orient, ray, 960.0, 540.0) {
            ContextBoardRayHit::Inside { px, py } => {
                assert!((px - 480.0).abs() < 2.0, "px={px}");
                assert!((py - 270.0).abs() < 2.0, "py={py}");
            }
            other => panic!("expected Inside, got {other:?}"),
        }
    }

    #[test]
    fn frozen_context_board_miss_closes_path() {
        let aim_pos = Vec3::ZERO;
        let (center, orient, _) =
            freeze_context_board(aim_pos, Quat::IDENTITY, Quat::IDENTITY, 960.0, 540.0);
        let ray = Ray {
            origin: aim_pos,
            dir: Vec3::X,
        };
        assert_eq!(
            classify_context_board_ray(center, orient, ray, 960.0, 540.0),
            ContextBoardRayHit::Miss
        );
    }

    #[test]
    fn context_board_edge_keeps_clamped_pointer() {
        let aim_pos = Vec3::new(0.0, 1.4, 0.0);
        let (center, orient, _) =
            freeze_context_board(aim_pos, Quat::IDENTITY, Quat::IDENTITY, 960.0, 540.0);
        let (bw, _bh) = crate::context_radial::context_board_size(960.0, 540.0);
        // Aim slightly past the right edge but inside leave pad.
        let edge_x = bw * 0.5 * 1.1;
        let origin = center + Vec3::new(edge_x, 0.0, 0.3);
        let ray = Ray {
            origin,
            dir: Vec3::NEG_Z,
        };
        match classify_context_board_ray(center, orient, ray, 960.0, 540.0) {
            ContextBoardRayHit::Edge { px, .. } => {
                assert!((px - 960.0).abs() < 1.0, "clamped px={px}");
            }
            other => panic!("expected Edge, got {other:?}"),
        }
    }
}
