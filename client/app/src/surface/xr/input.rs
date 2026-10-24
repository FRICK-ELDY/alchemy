//! XR UI intent: menu toggle + controller ray vs head-locked dash quad.
//!
//! system_ui stays device-agnostic; this module maps XR poses → canvas pixels.

use glam::{Mat4, Quat, Vec3};

/// Head-locked dash quad in view/local space (meters). Centered, facing -Z of head.
pub const QUAD_WIDTH: f32 = 1.2;
pub const QUAD_HEIGHT: f32 = 0.675; // 16:9
pub const QUAD_DISTANCE: f32 = 1.4;

#[derive(Clone, Copy, Debug)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

/// Intersect ray with the dash quad plane. Returns canvas pixel coords if hit (front face).
pub fn ray_hit_dash_quad(
    head_pos: Vec3,
    head_orient: Quat,
    ray: Ray,
    canvas_w: f32,
    canvas_h: f32,
) -> Option<(f32, f32)> {
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
    Some((u * canvas_w, v * canvas_h))
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
}
