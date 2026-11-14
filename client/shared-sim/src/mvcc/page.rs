//! Hot physics SoA page (x00a3 / x00a6 layout reservation). Layout: mvcc_xpbd.md §4.3.

use alchemy_protocol::USER_ROOT_SLOTS_MAX;
use bytemuck::{Pod, Zeroable};

pub const ENTITIES_PER_PAGE: usize = 1024;
pub const BITMASK_WORDS: usize = ENTITIES_PER_PAGE / 64;
pub const HISTORY_TICKS: usize = 128;
/// Snapshot page cap (1024×64 = 65,536 entity). Exceed → Err (no silent truncate).
pub const MAX_PAGES_PER_SNAPSHOT: usize = 64;
/// Unheld marker. Room DoS ceiling is [`USER_ROOT_SLOTS_MAX`] so u8 has headroom.
pub const NO_HOLDER: u8 = u8::MAX;
/// Q8 provisional: cell size meters (local coords nominally in [-32, 32)).
pub const CELL_SIZE_M: f64 = 64.0;
/// Q8 provisional: cross when `|local| > 32 + hysteresis`.
pub const CELL_HYSTERESIS_M: f32 = 2.0;

/// GPU-transferable SoA chunk (POD). Includes reserved rotation / inertia columns (x00a6).
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
    /// Rotation vector (axis × angle rad) for one tick. ω = dθ/dt. Reserved; rigid XPBD later.
    pub drot_x: [f32; ENTITIES_PER_PAGE],
    pub drot_y: [f32; ENTITIES_PER_PAGE],
    pub drot_z: [f32; ENTITIES_PER_PAGE],
    pub inv_inertia_x: [f32; ENTITIES_PER_PAGE],
    pub inv_inertia_y: [f32; ENTITIES_PER_PAGE],
    pub inv_inertia_z: [f32; ENTITIES_PER_PAGE],
}

/// CPU MVCC hot page: SoA + cell origin + masks + holder columns.
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
    /// Summarizes `holder_slot != NO_HOLDER`. Write only via [`Self::set_holder`] /
    /// [`Self::clear_holder`].
    is_grabbed_mask: [u64; BITMASK_WORDS],
    pub sleeping_mask: [u64; BITMASK_WORDS],
    /// Grab owner (`user_root` slot index). [`NO_HOLDER`] = unheld.
    /// Writes only via [`Self::set_holder`] / [`Self::clear_holder`] (keeps grab mask synced).
    /// Valid held values: `0..USER_ROOT_SLOTS_MAX`. Q7: smaller slot wins + lease.
    holder_slot: [u8; ENTITIES_PER_PAGE],
    holder_since_tick: [u32; ENTITIES_PER_PAGE],
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
            holder_slot: [NO_HOLDER; ENTITIES_PER_PAGE],
            holder_since_tick: [0; ENTITIES_PER_PAGE],
        }
    }

    #[inline]
    fn mask_bit(local_idx: usize) -> Option<(usize, u64)> {
        if local_idx >= ENTITIES_PER_PAGE {
            return None;
        }
        Some((local_idx / 64, 1u64 << (local_idx % 64)))
    }

    /// Mark local entity active (ManifestSeal gate stub for Phase 1 scaffold).
    pub fn set_active(&mut self, local_idx: usize, active: bool) {
        let Some((word, bit)) = Self::mask_bit(local_idx) else {
            return;
        };
        if active {
            self.active_mask[word] |= bit;
        } else {
            self.active_mask[word] &= !bit;
        }
    }

    #[inline]
    pub fn is_active(&self, local_idx: usize) -> bool {
        let Some((word, bit)) = Self::mask_bit(local_idx) else {
            return false;
        };
        (self.active_mask[word] & bit) != 0
    }

    #[inline]
    pub fn is_grabbed(&self, local_idx: usize) -> bool {
        let Some((word, bit)) = Self::mask_bit(local_idx) else {
            return false;
        };
        (self.is_grabbed_mask[word] & bit) != 0
    }

    #[inline]
    fn set_grabbed_mask(&mut self, local_idx: usize, grabbed: bool) {
        let Some((word, bit)) = Self::mask_bit(local_idx) else {
            return;
        };
        if grabbed {
            self.is_grabbed_mask[word] |= bit;
        } else {
            self.is_grabbed_mask[word] &= !bit;
        }
    }

    /// Read-only view of the grab summary bitmask (for fingerprints / equality).
    #[inline]
    pub fn grabbed_mask_words(&self) -> &[u64; BITMASK_WORDS] {
        &self.is_grabbed_mask
    }

    #[inline]
    pub fn holder_at(&self, local_idx: usize) -> u8 {
        self.holder_slot
            .get(local_idx)
            .copied()
            .unwrap_or(NO_HOLDER)
    }

    #[inline]
    pub fn holder_since_at(&self, local_idx: usize) -> u32 {
        self.holder_since_tick.get(local_idx).copied().unwrap_or(0)
    }

    /// Full holder columns (read-only; fingerprints / bit-equality).
    #[inline]
    pub fn holder_slots(&self) -> &[u8; ENTITIES_PER_PAGE] {
        &self.holder_slot
    }

    #[inline]
    pub fn holder_since_ticks(&self) -> &[u32; ENTITIES_PER_PAGE] {
        &self.holder_since_tick
    }

    /// `NO_HOLDER` or `0..USER_ROOT_SLOTS_MAX`.
    #[inline]
    pub fn is_valid_holder_slot(slot: u8) -> bool {
        slot == NO_HOLDER || (slot as usize) < USER_ROOT_SLOTS_MAX
    }

    /// Set holder; keeps the private grab summary mask in sync.
    ///
    /// Returns `false` if `local_idx` is out of range or `slot` is not
    /// [`NO_HOLDER`] / `0..USER_ROOT_SLOTS_MAX` (leaves page unchanged).
    pub fn set_holder(&mut self, local_idx: usize, slot: u8, since_tick: u32) -> bool {
        if local_idx >= ENTITIES_PER_PAGE || !Self::is_valid_holder_slot(slot) {
            return false;
        }
        self.holder_slot[local_idx] = slot;
        self.holder_since_tick[local_idx] = since_tick;
        self.set_grabbed_mask(local_idx, slot != NO_HOLDER);
        self.mark_dirty(local_idx);
        true
    }

    #[inline]
    pub fn clear_holder(&mut self, local_idx: usize) {
        let _ = self.set_holder(local_idx, NO_HOLDER, 0);
    }

    #[inline]
    pub fn is_sleeping(&self, local_idx: usize) -> bool {
        let Some((word, bit)) = Self::mask_bit(local_idx) else {
            return false;
        };
        (self.sleeping_mask[word] & bit) != 0
    }

    #[inline]
    pub fn set_sleeping(&mut self, local_idx: usize, sleeping: bool) {
        let Some((word, bit)) = Self::mask_bit(local_idx) else {
            return;
        };
        if sleeping {
            self.sleeping_mask[word] |= bit;
        } else {
            self.sleeping_mask[word] &= !bit;
        }
    }

    #[inline]
    pub fn clear_sleeping(&mut self, local_idx: usize) {
        self.set_sleeping(local_idx, false);
    }

    #[inline]
    pub fn mark_dirty(&mut self, local_idx: usize) {
        let Some((word, bit)) = Self::mask_bit(local_idx) else {
            return;
        };
        self.dirty_entity_mask[word] |= bit;
    }

    /// Deactivate and scrub SoA pose/delta/drot/holder to finite defaults (quarantine).
    pub fn quarantine_slot(&mut self, local_idx: usize) {
        if local_idx >= ENTITIES_PER_PAGE {
            return;
        }
        self.set_active(local_idx, false);
        self.clear_holder(local_idx);
        self.clear_sleeping(local_idx);
        self.soa.pos_x[local_idx] = 0.0;
        self.soa.pos_y[local_idx] = 0.0;
        self.soa.pos_z[local_idx] = 0.0;
        self.soa.delta_x[local_idx] = 0.0;
        self.soa.delta_y[local_idx] = 0.0;
        self.soa.delta_z[local_idx] = 0.0;
        self.soa.drot_x[local_idx] = 0.0;
        self.soa.drot_y[local_idx] = 0.0;
        self.soa.drot_z[local_idx] = 0.0;
        self.soa.inv_mass[local_idx] = 0.0;
        self.soa.inv_inertia_x[local_idx] = 0.0;
        self.soa.inv_inertia_y[local_idx] = 0.0;
        self.soa.inv_inertia_z[local_idx] = 0.0;
        self.soa.damping[local_idx] = 0.0;
        self.soa.rot_x[local_idx] = 0.0;
        self.soa.rot_y[local_idx] = 0.0;
        self.soa.rot_z[local_idx] = 0.0;
        self.soa.rot_w[local_idx] = 1.0;
        self.mark_dirty(local_idx);
    }

    /// Quarantined kinematic pose write: rejects NaN/Inf and inactive slots.
    /// Does not touch `drot_*` (rotation XPBD reserved).
    pub fn try_set_kinematic_pose(&mut self, local_idx: usize, pose: KinematicPoseF32) -> bool {
        if !self.is_active(local_idx) || !pose.is_finite() {
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

        self.mark_dirty(local_idx);
        self.clear_sleeping(local_idx);
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
        assert_eq!(page.holder_at(0), NO_HOLDER);
        assert_eq!(page.soa.drot_x[0], 0.0);
        assert_eq!(page.soa.inv_inertia_x[0], 0.0);
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

    #[test]
    fn layout_columns_roundtrip_bit_exact() {
        let mut page = HotPhysicsPage::zeroed_page();
        page.set_active(3, true);
        page.soa.pos_x[3] = 1.25;
        page.soa.delta_y[3] = -0.5;
        page.soa.drot_x[3] = 0.01;
        page.soa.drot_y[3] = -0.02;
        page.soa.drot_z[3] = 0.03;
        page.soa.inv_inertia_x[3] = 2.0;
        page.soa.inv_inertia_y[3] = 2.0;
        page.soa.inv_inertia_z[3] = 2.0;
        assert!(page.set_holder(3, 7, 42));

        let bytes = bytemuck::bytes_of(&page.soa).to_vec();
        let restored: GpuTransferableSoA = bytemuck::pod_read_unaligned(&bytes);
        assert_eq!(restored.drot_x[3].to_bits(), page.soa.drot_x[3].to_bits());
        assert_eq!(
            restored.inv_inertia_z[3].to_bits(),
            page.soa.inv_inertia_z[3].to_bits()
        );
        assert_eq!(page.holder_at(3), 7);
        assert_eq!(page.holder_since_at(3), 42);
        assert!(page.is_grabbed(3));
        page.clear_holder(3);
        assert_eq!(page.holder_at(3), NO_HOLDER);
        assert!(!page.is_grabbed(3));
    }

    #[test]
    fn set_holder_rejects_out_of_range_slot() {
        let mut page = HotPhysicsPage::zeroed_page();
        page.set_active(0, true);
        assert!(!page.set_holder(0, USER_ROOT_SLOTS_MAX as u8, 1));
        assert!(!page.set_holder(0, NO_HOLDER - 1, 1)); // 254 when MAX=32
        assert_eq!(page.holder_at(0), NO_HOLDER);
        assert!(!page.is_grabbed(0));
        assert!(page.set_holder(0, (USER_ROOT_SLOTS_MAX as u8) - 1, 9));
        assert_eq!(page.holder_at(0), (USER_ROOT_SLOTS_MAX as u8) - 1);
        assert!(page.is_grabbed(0));
        assert!(page.set_holder(0, NO_HOLDER, 0));
        assert!(!page.is_grabbed(0));
    }
}
