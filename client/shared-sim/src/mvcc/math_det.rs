//! Deterministic math helpers for SoA hot paths (x00a5).
//!
//! Guarantees: finite quarantine; `-0.0` → `+0.0`; normalize uses IEEE `f32` √
//! (same rule as mvcc_xpbd §4.4 — trig/`powf` would need `libm`, not √).
//! [`glam::Vec3`] is storage/API convenience only; this module does not claim
//! glam routes √ through the `libm` feature.
//! Non-guarantees: cross-arch bit identity (mvcc_xpbd Q2).
//!
//! Safety: `#![forbid(unsafe_code)]`.

#![forbid(unsafe_code)]

use glam::Vec3;

/// Finite gate + signed-zero normalize. Non-finite → `fallback`.
#[inline]
pub fn sanitize_f32(val: f32, fallback: f32) -> f32 {
    if val.is_finite() {
        if val == 0.0 {
            0.0
        } else {
            val
        }
    } else {
        fallback
    }
}

/// Unit vector (mvcc_xpbd §4.4 sketch: `inv_len = 1/√len_sq`, `len = len_sq * inv_len`).
/// Degenerate → `(0, 1, 0)` and length `0`.
///
/// Phase 2 scaffolding (constraint normals / contact); not required by x00a5 inertia.
#[inline]
pub fn det_normalize3(x: f32, y: f32, z: f32) -> (f32, f32, f32, f32) {
    let v = Vec3::new(
        sanitize_f32(x, 0.0),
        sanitize_f32(y, 0.0),
        sanitize_f32(z, 0.0),
    );
    let len_sq = v.length_squared();
    if len_sq <= 1e-12 {
        return (0.0, 1.0, 0.0, 0.0);
    }
    let inv_len = 1.0 / len_sq.sqrt();
    let len = len_sq * inv_len;
    let n = v * inv_len;
    (
        sanitize_f32(n.x, 0.0),
        sanitize_f32(n.y, 1.0),
        sanitize_f32(n.z, 0.0),
        sanitize_f32(len, 0.0),
    )
}

/// Camera/page cell origin difference as f32 offset (f64 subtract on CPU).
///
/// Phase 2 scaffolding (`interp.wgsl` `cell_rel_offset`); not required by x00a5 inertia.
#[inline]
pub fn cell_rel_offset_f32(camera_origin: [f64; 3], page_origin: [f64; 3]) -> glam::Vec3 {
    glam::Vec3::new(
        sanitize_f32((camera_origin[0] - page_origin[0]) as f32, 0.0),
        sanitize_f32((camera_origin[1] - page_origin[1]) as f32, 0.0),
        sanitize_f32((camera_origin[2] - page_origin[2]) as f32, 0.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_maps_neg_zero_and_nan() {
        assert_eq!(sanitize_f32(-0.0, 1.0).to_bits(), 0.0f32.to_bits());
        assert_eq!(sanitize_f32(f32::NAN, 2.0), 2.0);
        assert_eq!(sanitize_f32(1.5, 0.0), 1.5);
    }

    #[test]
    fn normalize_degenerate_and_unit() {
        let (x, y, z, len) = det_normalize3(0.0, 0.0, 0.0);
        assert_eq!((x, y, z, len), (0.0, 1.0, 0.0, 0.0));
        let (x, y, z, len) = det_normalize3(3.0, 0.0, 0.0);
        assert!((x - 1.0).abs() < 1e-6);
        assert_eq!(y, 0.0);
        assert_eq!(z, 0.0);
        assert!((len - 3.0).abs() < 1e-6);
    }

    #[test]
    fn normalize_matches_canonical_len_formula() {
        let x = 2.0f32;
        let y = 3.0f32;
        let z = 6.0f32;
        let len_sq = x * x + y * y + z * z;
        let inv_len = 1.0 / len_sq.sqrt();
        let expect_len = len_sq * inv_len;
        let (_, _, _, len) = det_normalize3(x, y, z);
        assert_eq!(len.to_bits(), expect_len.to_bits());
    }

    #[test]
    fn cell_rel_offset_finite() {
        let o = cell_rel_offset_f32([100.0, 0.0, -50.0], [36.0, 0.0, -50.0]);
        assert!((o.x - 64.0).abs() < 1e-5);
        assert_eq!(o.y, 0.0);
        assert_eq!(o.z, 0.0);
    }
}
