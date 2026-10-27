//! World-space ray hits against grabbable box AABBs (Desktop + XR).
//!
//! Guarantees: nearest AABB among caller-supplied grabbable poses within [`MAX_HIT_DISTANCE`].
//! Half-extent uses Manifest size × Frame `scale` max component (x0029).
//! Non-guarantees: OBB / `pose.rotation()`（描画とヒットは回転で乖離しうる・x0029）;
//! mesh triangles, Slot hierarchy, XR dash quad (stays in surface/xr).

use std::collections::HashMap;

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

/// Nearest grabbable box along `ray` (caller pre-filters the set; uniform half-extent).
pub fn ray_hit_grabbable_boxes(
    ray: Ray,
    boxes: &[(u32, EntityPose)],
    half_extent: f32,
) -> Option<BoxHit> {
    ray_hit_grabbable_boxes_with_extents(ray, boxes, |_| half_extent)
}

/// Nearest box using per-entity half-extents (`BOX_HALF_EXTENT` fallback).
pub fn ray_hit_grabbable_boxes_with_extents(
    ray: Ray,
    boxes: &[(u32, EntityPose)],
    half_for: impl Fn(u32) -> f32,
) -> Option<BoxHit> {
    let mut best: Option<BoxHit> = None;
    for &(id, pose) in boxes {
        // Definition × execution scale; rotation ignored (AABB, not OBB).
        let half = half_for(id) * pose.scale().max_element().max(0.0);
        let Some(t) = ray_hit_aabb(ray, pose.position(), half) else {
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

/// Desktop pick pad: floor boxes sit below eye height; slight AABB inflate helps look-aim.
pub const DESKTOP_PICK_HALF_PAD: f32 = 0.12;

pub fn pick_grabbable_desktop(
    ray: Ray,
    boxes: &[(u32, EntityPose)],
    half_extents: &HashMap<u32, f32>,
) -> Option<BoxHit> {
    ray_hit_grabbable_boxes_with_extents(ray, boxes, |id| {
        half_extents.get(&id).copied().unwrap_or(BOX_HALF_EXTENT) + DESKTOP_PICK_HALF_PAD
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aabb_half_uses_frame_scale_max() {
        let ray = Ray {
            origin: Vec3::new(0.0, 0.0, 3.0),
            dir: Vec3::NEG_Z,
        };
        let unit = [(
            10,
            EntityPose {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                ..Default::default()
            },
        )];
        // half=0.25 → enter at t=2.75; with scale 2 → half=0.5 → enter at t=2.5
        let t_unit = ray_hit_grabbable_boxes(ray, &unit, 0.25).unwrap().t;
        let scaled = [(
            10,
            EntityPose {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                sx: 2.0,
                sy: 2.0,
                sz: 2.0,
                ..Default::default()
            },
        )];
        let t_scaled = ray_hit_grabbable_boxes(ray, &scaled, 0.25).unwrap().t;
        assert!(t_scaled < t_unit);
        assert!((t_scaled - 2.5).abs() < 0.05, "t_scaled={t_scaled}");
    }

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

    #[test]
    fn eye_height_horizontal_misses_floor_box_exact_half() {
        let eye = EntityPose {
            x: 0.0,
            y: 1.6,
            z: 6.0,
            ..Default::default()
        };
        let ray = desktop_look_ray(eye, crate::scene::CameraLook::default());
        let boxes = [(
            14,
            EntityPose {
                x: 0.0,
                y: 0.25,
                z: 3.5,
                ..Default::default()
            },
        )];
        assert!(pick_grabbable(ray, &boxes).is_none());
    }

    #[test]
    fn looking_down_hits_floor_box() {
        let eye = EntityPose {
            x: 0.0,
            y: 1.6,
            z: 6.0,
            ..Default::default()
        };
        // ~atan(1.35/2.5) down toward (0, 0.25, 3.5).
        let look = crate::scene::CameraLook {
            yaw: 0.0,
            pitch: -0.5,
        };
        let ray = desktop_look_ray(eye, look);
        let boxes = [(
            14,
            EntityPose {
                x: 0.0,
                y: 0.25,
                z: 3.5,
                ..Default::default()
            },
        )];
        let hit =
            pick_grabbable_desktop(ray, &boxes, &HashMap::new()).expect("look-down should hit");
        assert_eq!(hit.entity_id, 14);
    }
}
