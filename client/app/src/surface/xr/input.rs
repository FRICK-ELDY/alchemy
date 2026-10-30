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
pub const QUAD_WIDTH: f32 = 1.2;
pub const QUAD_HEIGHT: f32 = 0.675; // 16:9
pub const QUAD_DISTANCE: f32 = 1.4;

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
}
