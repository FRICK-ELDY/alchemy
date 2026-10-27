//! World-space ray hits against grabbable box OBBs (Desktop + XR, x002b).
//!
//! Guarantees: nearest OBB among caller-supplied grabbable poses within [`MAX_HIT_DISTANCE`].
//! Local half-extents = Manifest `half_extent` × Frame `scale` (per-axis); orientation from
//! `pose.rotation()` — matches Desktop/XR draw TRS (x0029 / x002c).
//! Non-guarantees: mesh triangles, Slot hierarchy, XR dash quad (stays in surface/xr);
//! Grab aim rotation follow on host (translation only); host Rapier / grab-distance use
//! `half_extent` only (client OBB may disagree when Frame scale ≠ 1 — x002c).

use std::collections::HashMap;

use glam::{Quat, Vec3};

use crate::scene::{EntityPose, BOX_HALF_EXTENT};

/// Max ray length for world pointer / Grab pick (meters). Matches XR open-air laser.
/// SSoT: `alchemy_shared_sim::MAX_GRAB_HIT_DISTANCE` (x0030).
pub use alchemy_shared_sim::MAX_GRAB_HIT_DISTANCE as MAX_HIT_DISTANCE;

#[derive(Clone, Copy, Debug)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

/// Closest box hit among grabbable poses.
#[derive(Clone, Copy, Debug)]
pub struct BoxHit {
    pub entity_id: u32,
    pub t: f32,
}

/// Ray vs axis-aligned box (center + uniform half-extent). Returns entry `t` or None.
#[cfg(test)]
fn ray_hit_aabb(ray: Ray, center: Vec3, half: f32) -> Option<f32> {
    ray_hit_aabb_extents(ray, center, Vec3::splat(half))
}

/// Ray vs AABB with per-axis half-extents (local or world axis-aligned).
fn ray_hit_aabb_extents(ray: Ray, center: Vec3, half: Vec3) -> Option<f32> {
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
    let min = center - half;
    let max = center + half;
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

/// Ray vs oriented box: transform ray into box local space, then AABB slab (x002b).
pub fn ray_hit_obb(ray: Ray, center: Vec3, rotation: Quat, half_extents: Vec3) -> Option<f32> {
    // Mirror scales (negative components) keep the same solid extent as draw.
    let half_extents = half_extents.abs();
    let inv = rotation.inverse();
    let local = Ray {
        origin: inv * (ray.origin - center),
        dir: inv * ray.dir,
    };
    ray_hit_aabb_extents(local, Vec3::ZERO, half_extents)
}

/// Local half-extents: `(Manifest half [+ Desktop pad]) * Frame scale` (per-axis).
/// Pad is applied to the Manifest scalar **before** scale, matching `half_for` callers.
fn local_half_extents(half: f32, pose: EntityPose) -> Vec3 {
    Vec3::splat(half.max(0.0)) * pose.scale().abs()
}

/// Nearest grabbable box along `ray` (caller pre-filters the set; uniform Manifest half).
pub fn ray_hit_grabbable_boxes(
    ray: Ray,
    boxes: &[(u32, EntityPose)],
    half_extent: f32,
) -> Option<BoxHit> {
    ray_hit_grabbable_boxes_with_extents(ray, boxes, |_| half_extent)
}

/// Nearest box using per-entity half-extents (`BOX_HALF_EXTENT` fallback) + Frame TRS.
pub fn ray_hit_grabbable_boxes_with_extents(
    ray: Ray,
    boxes: &[(u32, EntityPose)],
    half_for: impl Fn(u32) -> f32,
) -> Option<BoxHit> {
    let mut best: Option<BoxHit> = None;
    for &(id, pose) in boxes {
        let he = local_half_extents(half_for(id), pose);
        let Some(t) = ray_hit_obb(ray, pose.position(), pose.rotation(), he) else {
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

/// Desktop pick pad added to Manifest half **before** Frame scale
/// (`(half + PAD) * scale`, not `half * scale + PAD`).
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
    use std::f32::consts::FRAC_PI_4;

    #[test]
    fn scale_expands_hit_along_axes() {
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
    fn nonuniform_scale_is_per_axis_not_max_element() {
        // Along +X: sx=2 → half_x=0.5; sy=sz=1 → half_y/z=0.25.
        // Old max_element AABB would use half=0.5 on all axes and hit a ray at y=0.4.
        // Per-axis OBB must miss (0.4 > 0.25).
        let ray = Ray {
            origin: Vec3::new(-3.0, 0.4, 0.0),
            dir: Vec3::X,
        };
        let pose = [(
            10,
            EntityPose {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                sx: 2.0,
                sy: 1.0,
                sz: 1.0,
                ..Default::default()
            },
        )];
        assert!(
            ray_hit_grabbable_boxes(ray, &pose, 0.25).is_none(),
            "y=0.4 must miss when half_y=0.25"
        );
        let ray_on_axis = Ray {
            origin: Vec3::new(-3.0, 0.0, 0.0),
            dir: Vec3::X,
        };
        let hit = ray_hit_grabbable_boxes(ray_on_axis, &pose, 0.25).expect("x-axis should hit");
        assert!((hit.t - 2.5).abs() < 0.05, "t={}", hit.t);
    }

    #[test]
    fn negative_scale_matches_positive_extent() {
        let ray = Ray {
            origin: Vec3::new(0.0, 0.0, 3.0),
            dir: Vec3::NEG_Z,
        };
        let mirrored = [(
            10,
            EntityPose {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                sx: -2.0,
                sy: -2.0,
                sz: -2.0,
                ..Default::default()
            },
        )];
        let hit = ray_hit_grabbable_boxes(ray, &mirrored, 0.25).expect("mirror scale hits");
        assert!((hit.t - 2.5).abs() < 0.05, "t={}", hit.t);
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
    fn yaw45_obb_hits_where_identity_aabb_misses() {
        // half=0.5 → identity AABB |x|<=0.5. Ray at x=0.6 misses identity.
        // 45° yaw expands XZ footprint to ~0.707 → OBB hits.
        let ray = Ray {
            origin: Vec3::new(0.6, 0.0, 3.0),
            dir: Vec3::NEG_Z,
        };
        let identity = [(
            10,
            EntityPose {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                ..Default::default()
            },
        )];
        assert!(ray_hit_grabbable_boxes(ray, &identity, 0.5).is_none());

        let q = Quat::from_rotation_y(FRAC_PI_4);
        let rotated = [(
            10,
            EntityPose {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                qx: q.x,
                qy: q.y,
                qz: q.z,
                qw: q.w,
                ..Default::default()
            },
        )];
        let hit = ray_hit_grabbable_boxes(ray, &rotated, 0.5).expect("OBB should hit");
        assert_eq!(hit.entity_id, 10);
        assert!(hit.t > 0.0 && hit.t < MAX_HIT_DISTANCE);
    }

    #[test]
    fn identity_rotation_matches_aabb() {
        let ray = Ray {
            origin: Vec3::new(0.0, 0.0, 2.0),
            dir: Vec3::NEG_Z,
        };
        let pose = EntityPose {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            ..Default::default()
        };
        let t_aabb = ray_hit_aabb(ray, pose.position(), 0.25).unwrap();
        let t_obb = ray_hit_obb(
            ray,
            pose.position(),
            pose.rotation(),
            local_half_extents(0.25, pose),
        )
        .unwrap();
        assert!((t_aabb - t_obb).abs() < 1e-5);
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
