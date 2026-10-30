//! Stereo view / projection helpers for XR wgpu (x0051).
//!
//! Guarantees: same FOV→matrix convention as ash `vk_loop` (Vulkan NDC Y-down);
//! `openxr` types stay in `surface/xr` (not `scene`).
//! Non-guarantees: multiview; ash deletion.

use glam::{Mat4, Vec4};
use openxr as xr;

use super::input::openxr_pose_to_isometry;

const DEFAULT_NEAR: f32 = 0.05;
const DEFAULT_FAR: f32 = 100.0;

/// View matrix from an OpenXR pose (world/stage → eye).
pub fn view_from_pose(pose: xr::Posef) -> Mat4 {
    let (pos, ori) = openxr_pose_to_isometry(pose);
    Mat4::from_rotation_translation(ori, pos).inverse()
}

/// Projection from OpenXR FOV (near/far meters). Matches ash `projection_from_fov`.
pub fn projection_from_fov(fov: xr::Fovf, near: f32, far: f32) -> Mat4 {
    let tan_left = fov.angle_left.tan();
    let tan_right = fov.angle_right.tan();
    let tan_up = fov.angle_up.tan();
    let tan_down = fov.angle_down.tan();
    let tan_width = tan_right - tan_left;
    // Vulkan NDC Y grows downward; OpenXR view Y grows upward → use (down − up).
    let tan_height = tan_down - tan_up;
    Mat4::from_cols(
        Vec4::new(2.0 / tan_width, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 2.0 / tan_height, 0.0, 0.0),
        Vec4::new(
            (tan_right + tan_left) / tan_width,
            (tan_up + tan_down) / tan_height,
            -far / (far - near),
            -1.0,
        ),
        Vec4::new(0.0, 0.0, -(far * near) / (far - near), 0.0),
    )
}

/// Default near/far used by the XR wgpu session.
pub fn projection_from_fov_default(fov: xr::Fovf) -> Mat4 {
    projection_from_fov(fov, DEFAULT_NEAR, DEFAULT_FAR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symmetric_fov_no_div_by_zero() {
        let fov = xr::Fovf {
            angle_left: -1.0,
            angle_right: 1.0,
            angle_up: 1.0,
            angle_down: -1.0,
        };
        let p = projection_from_fov(fov, 0.05, 100.0);
        let cols = p.to_cols_array();
        assert!(cols.iter().all(|v| v.is_finite()));
        // Scale terms non-zero.
        assert!(cols[0].abs() > 0.1);
        assert!(cols[5].abs() > 0.1);
    }

    #[test]
    fn view_from_identity_pose_is_identity() {
        let pose = xr::Posef::IDENTITY;
        let v = view_from_pose(pose);
        let id = Mat4::IDENTITY;
        for (a, b) in v.to_cols_array().iter().zip(id.to_cols_array().iter()) {
            assert!((a - b).abs() < 1e-5, "{a} vs {b}");
        }
    }

    #[test]
    fn view_from_translated_pose_inverts_translation() {
        let mut pose = xr::Posef::IDENTITY;
        pose.position = xr::Vector3f {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        };
        let v = view_from_pose(pose);
        let p = v.transform_point3(glam::Vec3::new(1.0, 2.0, 3.0));
        assert!(p.length() < 1e-4, "expected origin, got {p:?}");
    }
}
