//! World-space ray hits against grabbable box AABBs (Desktop + XR).
//!
//! Guarantees: nearest AABB entry among entity ids 10..18 within [`MAX_HIT_DISTANCE`].
//! Non-guarantees: mesh triangles, Slot hierarchy, XR dash quad (stays in surface/xr).

use glam::Vec3;

use crate::scene::{EntityPose, BOX_HALF_EXTENT};

/// Max ray length for world pointer / Grab pick (meters). Matches XR open-air laser.
pub const MAX_HIT_DISTANCE: f32 = 5.0;

#[derive(Clone, Copy, Debug)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

/// Closest AABB hit among grabbable boxes.
#[derive(Clone, Copy, Debug)]
pub struct BoxHit {
    pub entity_id: u32,
    pub t: f32,
}

/// Ray vs axis-aligned box (center + uniform half-extent). Returns entry `t` or None.
pub fn ray_hit_aabb(ray: Ray, center: Vec3, half: f32) -> Option<f32> {
    let dir = ray.dir;
    if dir.length_squared() < 1e-12 {
        return None;
    }
    let inv = Vec3::new(
        if dir.x.abs() > 1e-8 {
            1.0 / dir.x
        } else {
            f32::INFINITY.copysign(dir.x)
        },
        if dir.y.abs() > 1e-8 {
            1.0 / dir.y
        } else {
            f32::INFINITY.copysign(dir.y)
        },
        if dir.z.abs() > 1e-8 {
            1.0 / dir.z
        } else {
            f32::INFINITY.copysign(dir.z)
        },
    );
    let min = center - Vec3::splat(half);
    let max = center + Vec3::splat(half);
    let t0 = (min - ray.origin) * inv;
    let t1 = (max - ray.origin) * inv;
    let tmin = t0.min(t1);
    let tmax = t0.max(t1);
    let t_enter = tmin.x.max(tmin.y).max(tmin.z);
    let t_exit = tmax.x.min(tmax.y).min(tmax.z);
    if t_exit < 0.0 || t_enter > t_exit {
        return None;
    }
    let t = if t_enter >= 0.0 { t_enter } else { t_exit };
    if t < 0.0 {
        None
    } else {
        Some(t)
    }
}

/// Nearest grabbable box along `ray` (client-local pose table).
pub fn ray_hit_grabbable_boxes(
    ray: Ray,
    boxes: &[(u32, EntityPose)],
    half_extent: f32,
) -> Option<BoxHit> {
    let mut best: Option<BoxHit> = None;
    for &(id, pose) in boxes {
        if !EntityPose::is_grabbable_box(id) {
            continue;
        }
        let Some(t) = ray_hit_aabb(ray, pose.position(), half_extent) else {
            continue;
        };
        if t > MAX_HIT_DISTANCE {
            continue;
        }
        if best.map(|b| t < b.t).unwrap_or(true) {
            best = Some(BoxHit { entity_id: id, t });
        }
    }
    best
}

/// Look-forward ray from Desktop eye + [`crate::scene::CameraLook`].
pub fn desktop_look_ray(eye: EntityPose, look: crate::scene::CameraLook) -> Ray {
    let pitch = look.pitch.clamp(-1.54, 1.54);
    let forward = Vec3::new(
        look.yaw.sin() * pitch.cos(),
        pitch.sin(),
        -look.yaw.cos() * pitch.cos(),
    )
    .normalize_or_zero();
    Ray {
        origin: eye.position(),
        dir: forward,
    }
}

/// Convenience: hit using default [`BOX_HALF_EXTENT`].
pub fn pick_grabbable(ray: Ray, boxes: &[(u32, EntityPose)]) -> Option<BoxHit> {
    ray_hit_grabbable_boxes(ray, boxes, BOX_HALF_EXTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aabb_hit_from_front() {
        let ray = Ray {
            origin: Vec3::new(0.0, 0.25, 2.0),
            dir: Vec3::NEG_Z,
        };
        let t = ray_hit_aabb(ray, Vec3::new(0.0, 0.25, 0.0), 0.25).unwrap();
        assert!((t - 1.75).abs() < 0.01, "t={t}");
    }

    #[test]
    fn grabbable_boxes_picks_nearest() {
        let ray = Ray {
            origin: Vec3::new(0.0, 0.25, 3.0),
            dir: Vec3::NEG_Z,
        };
        let boxes = [
            (
                10,
                EntityPose {
                    x: 0.0,
                    y: 0.25,
                    z: 0.0,
                    ..Default::default()
                },
            ),
            (
                11,
                EntityPose {
                    x: 0.0,
                    y: 0.25,
                    z: 1.0,
                    ..Default::default()
                },
            ),
        ];
        let hit = ray_hit_grabbable_boxes(ray, &boxes, 0.25).unwrap();
        assert_eq!(hit.entity_id, 11);
        assert!((hit.t - 1.75).abs() < 0.05, "t={}", hit.t);
    }

    #[test]
    fn desktop_look_ray_default_faces_neg_z() {
        let eye = EntityPose {
            x: 0.0,
            y: 1.6,
            z: 6.0,
            ..Default::default()
        };
        let ray = desktop_look_ray(eye, crate::scene::CameraLook::default());
        assert!((ray.origin - Vec3::new(0.0, 1.6, 6.0)).length() < 1e-5);
        assert!((ray.dir - Vec3::NEG_Z).length() < 1e-5);
    }
}
