//! SoA inertial predict on [`HotPhysicsPage`] (x00a5).
//!
//! Guarantees: scalar reference [`step_inertia_scalar`]; [`step_inertia`] delegates to
//! Safe SIMD (`wide`); same inputs → bit-identical SoA positions.
//! Non-guarantees: **not wired** into [`super::adapter::PageXpbdAdapter`] /
//! [`crate::xpbd::predict_integrate`] (tick path still Epoch/XPBD); constraint solve;
//! tick-level `frame_delta` ownership (caller).
//!
//! Cost note: SIMD covers the four arithmetic ops only; mask build, finite gate,
//! [`sanitize_f32`](super::math_det::sanitize_f32), and [`mark_dirty`](super::page::HotPhysicsPage::mark_dirty)
//! stay scalar so scalar≡SIMD bit identity holds. Net win is modest until constraint
//! batches are lane-driven the same way.
//!
//! `prev` contract: positions at the **start of this substep** (same role as
//! `XpbdWorld::sub_prev_scratch`), not an arena history page. Passing a prior tick
//! page across multiple substeps double-applies gravity relative to `xpbd::step`.
//!
//! Safety: `#![forbid(unsafe_code)]`.

#![forbid(unsafe_code)]

use wide::f32x8;

use super::math_det::sanitize_f32;
use super::page::{HotPhysicsPage, ENTITIES_PER_PAGE};

const LANES: usize = 8;

// Silent tail skip if page size is not a multiple of the SIMD width.
const _: () = assert!(ENTITIES_PER_PAGE.is_multiple_of(LANES));

/// Default SoA inertia entry (SIMD). **Building block only** — not on the host/app
/// tick path yet; call sites must pass substep-scratch `prev` (see module docs).
#[inline]
pub fn step_inertia(curr: &mut HotPhysicsPage, prev: &HotPhysicsPage, gravity: [f32; 3], dt: f32) {
    step_inertia_simd(curr, prev, gravity, dt);
}

/// Scalar reference: `x' = x + (x - x_prev) + g * dt²` for movable dynamic rows.
pub fn step_inertia_scalar(
    curr: &mut HotPhysicsPage,
    prev: &HotPhysicsPage,
    gravity: [f32; 3],
    dt: f32,
) {
    let dt = sanitize_f32(dt, 0.0);
    let dt2 = dt * dt;
    let g = [
        sanitize_f32(gravity[0], 0.0),
        sanitize_f32(gravity[1], 0.0),
        sanitize_f32(gravity[2], 0.0),
    ];
    let g_dt2 = [g[0] * dt2, g[1] * dt2, g[2] * dt2];

    for i in 0..ENTITIES_PER_PAGE {
        if !should_integrate(curr, i) {
            continue;
        }
        let cx = curr.soa.pos_x[i];
        let cy = curr.soa.pos_y[i];
        let cz = curr.soa.pos_z[i];
        let px = prev.soa.pos_x[i];
        let py = prev.soa.pos_y[i];
        let pz = prev.soa.pos_z[i];

        let mut nx = cx + (cx - px) + g_dt2[0];
        let mut ny = cy + (cy - py) + g_dt2[1];
        let mut nz = cz + (cz - pz) + g_dt2[2];
        if !(nx.is_finite() && ny.is_finite() && nz.is_finite()) {
            nx = cx;
            ny = cy;
            nz = cz;
        }
        nx = sanitize_f32(nx, cx);
        ny = sanitize_f32(ny, cy);
        nz = sanitize_f32(nz, cz);

        // Bit compare: IEEE `-0.0 == +0.0` would skip signed-zero normalization.
        let moved = nx.to_bits() != cx.to_bits()
            || ny.to_bits() != cy.to_bits()
            || nz.to_bits() != cz.to_bits();
        curr.soa.pos_x[i] = nx;
        curr.soa.pos_y[i] = ny;
        curr.soa.pos_z[i] = nz;
        if moved {
            curr.mark_dirty(i);
        }
    }
}

/// Safe SIMD (`f32x8`) inertia; must match [`step_inertia_scalar`] bit-for-bit.
///
/// Arithmetic is lane-wide; quarantine / dirty stay scalar (see module cost note).
pub fn step_inertia_simd(
    curr: &mut HotPhysicsPage,
    prev: &HotPhysicsPage,
    gravity: [f32; 3],
    dt: f32,
) {
    let dt = sanitize_f32(dt, 0.0);
    let dt2 = dt * dt;
    let g = [
        sanitize_f32(gravity[0], 0.0),
        sanitize_f32(gravity[1], 0.0),
        sanitize_f32(gravity[2], 0.0),
    ];
    let g_dt2 = [
        f32x8::splat(g[0] * dt2),
        f32x8::splat(g[1] * dt2),
        f32x8::splat(g[2] * dt2),
    ];

    for chunk in 0..(ENTITIES_PER_PAGE / LANES) {
        let base = chunk * LANES;
        let mut integrate = [0.0f32; LANES];
        for (lane, flag) in integrate.iter_mut().enumerate() {
            if should_integrate(curr, base + lane) {
                *flag = 1.0;
            }
        }
        // No movable lanes in this chunk: skip loads/stores.
        if integrate.iter().all(|&v| v == 0.0) {
            continue;
        }

        let cx = f32x8::from(chunk8(&curr.soa.pos_x, base));
        let cy = f32x8::from(chunk8(&curr.soa.pos_y, base));
        let cz = f32x8::from(chunk8(&curr.soa.pos_z, base));
        let px = f32x8::from(chunk8(&prev.soa.pos_x, base));
        let py = f32x8::from(chunk8(&prev.soa.pos_y, base));
        let pz = f32x8::from(chunk8(&prev.soa.pos_z, base));

        let nx = cx + (cx - px) + g_dt2[0];
        let ny = cy + (cy - py) + g_dt2[1];
        let nz = cz + (cz - pz) + g_dt2[2];

        let nx_a = nx.to_array();
        let ny_a = ny.to_array();
        let nz_a = nz.to_array();
        let cx_a = cx.to_array();
        let cy_a = cy.to_array();
        let cz_a = cz.to_array();

        for lane in 0..LANES {
            if integrate[lane] == 0.0 {
                continue;
            }
            let i = base + lane;
            let mut x = nx_a[lane];
            let mut y = ny_a[lane];
            let mut z = nz_a[lane];
            if !(x.is_finite() && y.is_finite() && z.is_finite()) {
                x = cx_a[lane];
                y = cy_a[lane];
                z = cz_a[lane];
            }
            x = sanitize_f32(x, cx_a[lane]);
            y = sanitize_f32(y, cy_a[lane]);
            z = sanitize_f32(z, cz_a[lane]);
            let moved = x.to_bits() != cx_a[lane].to_bits()
                || y.to_bits() != cy_a[lane].to_bits()
                || z.to_bits() != cz_a[lane].to_bits();
            curr.soa.pos_x[i] = x;
            curr.soa.pos_y[i] = y;
            curr.soa.pos_z[i] = z;
            if moved {
                curr.mark_dirty(i);
            }
        }
    }
}

#[inline]
fn should_integrate(page: &HotPhysicsPage, i: usize) -> bool {
    if !page.is_active(i) || page.is_sleeping(i) || page.is_grabbed(i) {
        return false;
    }
    let inv = page.soa.inv_mass[i];
    inv.is_finite() && inv > 0.0
}

/// Contiguous 8-wide load (bounds guaranteed by chunk loop + `ENTITIES_PER_PAGE % LANES == 0`).
#[inline]
fn chunk8(col: &[f32; ENTITIES_PER_PAGE], base: usize) -> [f32; LANES] {
    let mut out = [0.0f32; LANES];
    out.copy_from_slice(&col[base..base + LANES]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mvcc::adapter::pages_bit_equal;
    use crate::mvcc::page::KinematicPoseF32;

    fn seed_pair() -> (HotPhysicsPage, HotPhysicsPage) {
        let mut curr = HotPhysicsPage::zeroed_page();
        let mut prev = HotPhysicsPage::zeroed_page();
        // Dynamic falling body.
        curr.set_active(0, true);
        curr.soa.inv_mass[0] = 1.0;
        assert!(curr.try_set_kinematic_pose(
            0,
            KinematicPoseF32::xyz_quat(0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0)
        ));
        prev.set_active(0, true);
        prev.soa.inv_mass[0] = 1.0;
        prev.soa.pos_y[0] = 1.1; // upward prev → falling after predict
                                 // Static.
        curr.set_active(1, true);
        curr.soa.inv_mass[1] = 0.0;
        assert!(curr.try_set_kinematic_pose(
            1,
            KinematicPoseF32::xyz_quat(2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0)
        ));
        prev.set_active(1, true);
        prev.soa.inv_mass[1] = 0.0;
        prev.soa.pos_x[1] = 2.0;
        // Sleeping dynamic (must not move).
        curr.set_active(2, true);
        curr.set_sleeping(2, true);
        curr.soa.inv_mass[2] = 1.0;
        curr.soa.pos_y[2] = 5.0;
        prev.set_active(2, true);
        prev.soa.inv_mass[2] = 1.0;
        prev.soa.pos_y[2] = 5.2;
        // Grabbed dynamic.
        curr.set_active(3, true);
        curr.set_grabbed(3, true);
        curr.soa.inv_mass[3] = 1.0;
        curr.soa.pos_x[3] = 9.0;
        prev.set_active(3, true);
        prev.soa.inv_mass[3] = 1.0;
        prev.soa.pos_x[3] = 8.0;
        // Lane-7 edge in first chunk + another chunk entity.
        curr.set_active(7, true);
        curr.soa.inv_mass[7] = 1.0;
        curr.soa.pos_z[7] = 0.5;
        prev.set_active(7, true);
        prev.soa.inv_mass[7] = 1.0;
        prev.soa.pos_z[7] = 0.4;
        curr.set_active(16, true);
        curr.soa.inv_mass[16] = 1.0;
        curr.soa.pos_x[16] = -1.0;
        prev.set_active(16, true);
        prev.soa.inv_mass[16] = 1.0;
        prev.soa.pos_x[16] = -1.2;
        (curr, prev)
    }

    #[test]
    fn scalar_and_simd_bit_identical() {
        let (mut a, prev) = seed_pair();
        let (mut b, _) = seed_pair();
        let g = [0.0, -9.81, 0.0];
        let dt = 1.0 / 60.0;
        step_inertia_scalar(&mut a, &prev, g, dt);
        step_inertia_simd(&mut b, &prev, g, dt);
        assert!(
            pages_bit_equal(&a, &b),
            "scalar vs simd SoA/masks must match bit-for-bit"
        );
        // Falling body moved; static / sleeping / grabbed did not.
        assert!(a.soa.pos_y[0] < 1.0);
        assert_eq!(a.soa.pos_x[1], 2.0);
        assert_eq!(a.soa.pos_y[2], 5.0);
        assert_eq!(a.soa.pos_x[3], 9.0);
    }

    #[test]
    fn step_inertia_defaults_to_simd_path() {
        let (mut via_default, prev) = seed_pair();
        let (mut via_simd, _) = seed_pair();
        let g = [0.0, -9.81, 0.0];
        let dt = 1.0 / 60.0;
        step_inertia(&mut via_default, &prev, g, dt);
        step_inertia_simd(&mut via_simd, &prev, g, dt);
        assert!(pages_bit_equal(&via_default, &via_simd));
    }

    #[test]
    fn nan_inf_gravity_and_dt_quarantine_match() {
        let (mut a, prev) = seed_pair();
        let (mut b, _) = seed_pair();
        let g = [f32::NAN, f32::INFINITY, -0.0];
        let dt = f32::NAN;
        step_inertia_scalar(&mut a, &prev, g, dt);
        step_inertia_simd(&mut b, &prev, g, dt);
        assert!(pages_bit_equal(&a, &b));
        // Non-finite g/dt sanitize to 0 → g_dt2 = 0; velocity term (curr−prev) still applies.
        assert_eq!(a.soa.pos_y[0].to_bits(), 0.9f32.to_bits());
    }

    #[test]
    fn neg_zero_positions_sanitize_on_write() {
        let mut curr = HotPhysicsPage::zeroed_page();
        let mut prev = HotPhysicsPage::zeroed_page();
        curr.set_active(0, true);
        curr.soa.inv_mass[0] = 1.0;
        curr.soa.pos_x[0] = -0.0;
        curr.soa.pos_y[0] = -0.0;
        curr.soa.pos_z[0] = -0.0;
        prev.set_active(0, true);
        prev.soa.inv_mass[0] = 1.0;
        prev.soa.pos_x[0] = -0.0;
        prev.soa.pos_y[0] = -0.0;
        prev.soa.pos_z[0] = -0.0;

        let mut via_scalar = curr;
        let mut via_simd = curr;
        step_inertia_scalar(&mut via_scalar, &prev, [0.0, 0.0, 0.0], 1.0 / 60.0);
        step_inertia_simd(&mut via_simd, &prev, [0.0, 0.0, 0.0], 1.0 / 60.0);
        assert!(pages_bit_equal(&via_scalar, &via_simd));
        assert_eq!(via_scalar.soa.pos_x[0].to_bits(), 0.0f32.to_bits());
        assert_eq!(via_scalar.soa.pos_y[0].to_bits(), 0.0f32.to_bits());
        assert_eq!(via_scalar.soa.pos_z[0].to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn dirty_mask_only_for_moved_lanes() {
        let (mut page, prev) = seed_pair();
        // Clear dirty from try_set_kinematic_pose so we observe inertia marks alone.
        page.dirty_entity_mask = [0; crate::mvcc::page::BITMASK_WORDS];
        let before = page.dirty_entity_mask;
        step_inertia(&mut page, &prev, [0.0, -9.81, 0.0], 1.0 / 60.0);
        assert_ne!(page.dirty_entity_mask, before);
        assert_ne!(page.dirty_entity_mask[0] & (1u64 << 0), 0); // falling body
        assert_eq!(page.dirty_entity_mask[0] & (1u64 << 1), 0); // static
        assert_eq!(page.dirty_entity_mask[0] & (1u64 << 2), 0); // sleeping
        assert_eq!(page.dirty_entity_mask[0] & (1u64 << 3), 0); // grabbed
    }
}
