//! Hot physics SoA page (x00a3). Layout aligned with mvcc_xpbd.md §4.3.

use bytemuck::{Pod, Zeroable};

pub const ENTITIES_PER_PAGE: usize = 1024;
pub const BITMASK_WORDS: usize = ENTITIES_PER_PAGE / 64;
pub const HISTORY_TICKS: usize = 128;

/// GPU-transferable SoA chunk (POD).
#[repr(C, align(64))]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct GpuTransferableSoA {
    pub pos_x: [f32; ENTITIES_PER_PAGE],
    pub pos_y: [f32; ENTITIES_PER_PAGE],
    pub pos_z: [f32; ENTITIES_PER_PAGE],
    pub inv_mass: [f32; ENTITIES_PER_PAGE],
    pub delta_x: [f32; ENTITIES_PER_PAGE],
    pub delta_y: [f32; ENTITIES_PER_PAGE],
    pub delta_z: [f32; ENTITIES_PER_PAGE],
    pub damping: [f32; ENTITIES_PER_PAGE],
    pub rot_x: [f32; ENTITIES_PER_PAGE],
    pub rot_y: [f32; ENTITIES_PER_PAGE],
    pub rot_z: [f32; ENTITIES_PER_PAGE],
    pub rot_w: [f32; ENTITIES_PER_PAGE],
}

/// CPU MVCC hot page: SoA + cell origin + masks.
///
/// Not `Pod`: trailing padding after masks under `align(64)`. GPU path uses [`GpuTransferableSoA`].
#[repr(C, align(64))]
#[derive(Clone, Copy)]
pub struct HotPhysicsPage {
    pub soa: GpuTransferableSoA,
    pub cell_origin_x: f64,
    pub cell_origin_y: f64,
    pub cell_origin_z: f64,
    pub _pad0: f64,
    pub active_mask: [u64; BITMASK_WORDS],
    pub dirty_entity_mask: [u64; BITMASK_WORDS],
    pub is_grabbed_mask: [u64; BITMASK_WORDS],
    pub sleeping_mask: [u64; BITMASK_WORDS],
}

impl HotPhysicsPage {
    pub fn zeroed_page() -> Self {
        Self {
            soa: Zeroable::zeroed(),
            cell_origin_x: 0.0,
            cell_origin_y: 0.0,
            cell_origin_z: 0.0,
            _pad0: 0.0,
            active_mask: [0; BITMASK_WORDS],
            dirty_entity_mask: [0; BITMASK_WORDS],
            is_grabbed_mask: [0; BITMASK_WORDS],
            sleeping_mask: [0; BITMASK_WORDS],
        }
    }

    /// Mark local entity active (ManifestSeal gate stub for Phase 1 scaffold).
    pub fn set_active(&mut self, local_idx: usize, active: bool) {
        if local_idx >= ENTITIES_PER_PAGE {
            return;
        }
        let word = local_idx / 64;
        let bit = 1u64 << (local_idx % 64);
        if active {
            self.active_mask[word] |= bit;
        } else {
            self.active_mask[word] &= !bit;
        }
    }

    /// Quarantined kinematic pose write: rejects NaN/Inf and inactive slots.
    pub fn try_set_kinematic_pose(&mut self, local_idx: usize, pose: KinematicPoseF32) -> bool {
        if local_idx >= ENTITIES_PER_PAGE {
            return false;
        }
        if !pose.is_finite() {
            return false;
        }
        let word = local_idx / 64;
        let bit = 1u64 << (local_idx % 64);
        if (self.active_mask[word] & bit) == 0 {
            return false;
        }

        let prev_x = self.soa.pos_x[local_idx];
        let prev_y = self.soa.pos_y[local_idx];
        let prev_z = self.soa.pos_z[local_idx];

        self.soa.pos_x[local_idx] = pose.px;
        self.soa.pos_y[local_idx] = pose.py;
        self.soa.pos_z[local_idx] = pose.pz;
        self.soa.delta_x[local_idx] = pose.px - prev_x;
        self.soa.delta_y[local_idx] = pose.py - prev_y;
        self.soa.delta_z[local_idx] = pose.pz - prev_z;
        self.soa.rot_x[local_idx] = pose.qx;
        self.soa.rot_y[local_idx] = pose.qy;
        self.soa.rot_z[local_idx] = pose.qz;
        self.soa.rot_w[local_idx] = pose.qw;

        self.dirty_entity_mask[word] |= bit;
        self.sleeping_mask[word] &= !bit;
        true
    }
}

/// Finite TRS sample for [`HotPhysicsPage::try_set_kinematic_pose`].
#[derive(Clone, Copy, Debug)]
pub struct KinematicPoseF32 {
    pub px: f32,
    pub py: f32,
    pub pz: f32,
    pub qx: f32,
    pub qy: f32,
    pub qz: f32,
    pub qw: f32,
}

impl KinematicPoseF32 {
    pub const fn xyz_quat(px: f32, py: f32, pz: f32, qx: f32, qy: f32, qz: f32, qw: f32) -> Self {
        Self {
            px,
            py,
            pz,
            qx,
            qy,
            qz,
            qw,
        }
    }

    pub fn is_finite(self) -> bool {
        self.px.is_finite()
            && self.py.is_finite()
            && self.pz.is_finite()
            && self.qx.is_finite()
            && self.qy.is_finite()
            && self.qz.is_finite()
            && self.qw.is_finite()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zeroed_page_is_inactive() {
        let page = HotPhysicsPage::zeroed_page();
        assert_eq!(page.active_mask, [0u64; BITMASK_WORDS]);
        assert_eq!(page.soa.pos_x[0], 0.0);
    }

    #[test]
    fn rejects_nan_and_inactive() {
        let mut page = HotPhysicsPage::zeroed_page();
        let ok = KinematicPoseF32::xyz_quat(1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0);
        assert!(!page.try_set_kinematic_pose(0, ok));
        page.set_active(0, true);
        let nan = KinematicPoseF32::xyz_quat(f32::NAN, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0);
        assert!(!page.try_set_kinematic_pose(0, nan));
        let moved = KinematicPoseF32::xyz_quat(1.5, 2.0, -0.5, 0.0, 0.0, 0.0, 1.0);
        assert!(page.try_set_kinematic_pose(0, moved));
        assert_eq!(page.soa.pos_x[0], 1.5);
        assert_eq!(page.soa.delta_x[0], 1.5);
        assert_ne!(page.dirty_entity_mask[0] & 1, 0);
    }
}
