//! Entity → page directory and atomic cell crossing (x00a7; Q8 provisional).
//!
//! Guarantees: wire id is `entity_id: u32`; at most one live [`PageSlot`] per entity;
//! [`Self::cross_cell`] validates then writes dest SoA then relocates directory then scrubs
//! src — **directory mapping is never half-applied**; cell axes use **center origin**
//! (local ∈ [-32, 32)); hysteresis avoids flap.
//! Non-guarantees: spatial_interest delivery cells; host wiring; multi-page occupancy;
//! crash-atomicity of dest SoA vs directory (single-threaded tick: dest may briefly hold
//! a ghost local before relocate; callers must not observe mid-call).

use super::page::{HotPhysicsPage, CELL_HYSTERESIS_M, CELL_SIZE_M, ENTITIES_PER_PAGE, NO_HOLDER};
use super::PageId;

/// World cell id (same key family as spatial_interest delivery cells).
///
/// Cell `i` is centered at `i * CELL_SIZE_M`; local coords are world − origin ∈ [-32, 32).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellId(pub [i32; 3]);

impl CellId {
    /// Center-origin partitioning: `floor((v + 32) / 64)`.
    #[inline]
    fn axis_from_world(v: f64) -> i32 {
        let half = CELL_SIZE_M * 0.5;
        ((v + half) / CELL_SIZE_M).floor() as i32
    }

    pub fn from_world_f64(x: f64, y: f64, z: f64) -> Self {
        Self([
            Self::axis_from_world(x),
            Self::axis_from_world(y),
            Self::axis_from_world(z),
        ])
    }

    /// Cell center in world coordinates.
    pub fn origin_f64(self) -> [f64; 3] {
        [
            f64::from(self.0[0]) * CELL_SIZE_M,
            f64::from(self.0[1]) * CELL_SIZE_M,
            f64::from(self.0[2]) * CELL_SIZE_M,
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageSlot {
    pub cell: CellId,
    pub page: PageId,
    pub local: u16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DirectoryError {
    UnknownEntity(u32),
    /// `entity` is already mapped to a slot.
    AlreadyMapped(u32),
    /// Destination `(page, local)` owned by `existing` (or `u32::MAX` if page-active orphan).
    SlotOccupied {
        entity: u32,
        existing: u32,
    },
    LocalOutOfRange(u16),
    InactiveSource,
    /// Dest page already has actives but `cell_origin` ≠ `dest_cell` center.
    OriginMismatch {
        expected: [f64; 3],
        actual: [f64; 3],
    },
}

/// Sparse map: entity_id → page placement.
pub struct EntityDirectory {
    slots: Vec<Option<PageSlot>>,
    owners: Vec<Option<u32>>,
}

impl EntityDirectory {
    pub fn new(entity_capacity: usize) -> Self {
        Self {
            slots: vec![None; entity_capacity],
            owners: Vec::new(),
        }
    }

    pub fn get(&self, entity_id: u32) -> Option<PageSlot> {
        self.slots.get(entity_id as usize).copied().flatten()
    }

    fn ensure_entity(&mut self, entity_id: u32) {
        let need = entity_id as usize + 1;
        if self.slots.len() < need {
            self.slots.resize(need, None);
        }
    }

    fn owner_key(page: PageId, local: u16) -> usize {
        (page.0 as usize) * ENTITIES_PER_PAGE + local as usize
    }

    fn ensure_owner_slot(&mut self, key: usize) {
        if self.owners.len() <= key {
            self.owners.resize(key + 1, None);
        }
    }

    fn page_has_active(page: &HotPhysicsPage) -> bool {
        page.active_mask.iter().any(|&w| w != 0)
    }

    /// Insert entity into a free local slot. Fails if entity already mapped or local taken.
    pub fn insert(&mut self, entity_id: u32, slot: PageSlot) -> Result<(), DirectoryError> {
        if slot.local as usize >= ENTITIES_PER_PAGE {
            return Err(DirectoryError::LocalOutOfRange(slot.local));
        }
        self.ensure_entity(entity_id);
        if self.slots[entity_id as usize].is_some() {
            return Err(DirectoryError::AlreadyMapped(entity_id));
        }
        let key = Self::owner_key(slot.page, slot.local);
        self.ensure_owner_slot(key);
        if let Some(other) = self.owners[key] {
            return Err(DirectoryError::SlotOccupied {
                entity: entity_id,
                existing: other,
            });
        }
        self.owners[key] = Some(entity_id);
        self.slots[entity_id as usize] = Some(slot);
        Ok(())
    }

    pub fn remove(&mut self, entity_id: u32) -> Result<PageSlot, DirectoryError> {
        self.ensure_entity(entity_id);
        let slot = self.slots[entity_id as usize]
            .take()
            .ok_or(DirectoryError::UnknownEntity(entity_id))?;
        let key = Self::owner_key(slot.page, slot.local);
        if let Some(owner) = self.owners.get_mut(key) {
            *owner = None;
        }
        Ok(slot)
    }

    /// Infallible after validation: move entity mapping from old slot to `new_slot`.
    fn relocate_unchecked(&mut self, entity_id: u32, old: PageSlot, new_slot: PageSlot) {
        let old_key = Self::owner_key(old.page, old.local);
        let new_key = Self::owner_key(new_slot.page, new_slot.local);
        self.ensure_owner_slot(new_key);
        if let Some(owner) = self.owners.get_mut(old_key) {
            *owner = None;
        }
        self.owners[new_key] = Some(entity_id);
        self.slots[entity_id as usize] = Some(new_slot);
    }

    /// Decide whether local coords should move to a neighbor cell (hysteresis).
    pub fn should_cross_cell(cell: CellId, local: [f32; 3]) -> Option<CellId> {
        let half = (CELL_SIZE_M as f32) * 0.5;
        let limit = half + CELL_HYSTERESIS_M;
        let mut next = cell.0;
        let mut crossed = false;
        for i in 0..3 {
            if local[i] > limit {
                next[i] += 1;
                crossed = true;
            } else if local[i] < -limit {
                next[i] -= 1;
                crossed = true;
            }
        }
        if crossed {
            Some(CellId(next))
        } else {
            None
        }
    }

    /// Atomic cell cross: validate → write dest → relocate directory → scrub src.
    ///
    /// Never overwrites `cell_origin` on a page that already has active entities.
    pub fn cross_cell(
        &mut self,
        entity_id: u32,
        src_page: &mut HotPhysicsPage,
        dest_page: &mut HotPhysicsPage,
        dest_page_id: PageId,
        dest_cell: CellId,
        dest_local: u16,
    ) -> Result<PageSlot, DirectoryError> {
        let src = self
            .get(entity_id)
            .ok_or(DirectoryError::UnknownEntity(entity_id))?;
        let src_i = src.local as usize;
        if !src_page.is_active(src_i) {
            return Err(DirectoryError::InactiveSource);
        }
        if dest_local as usize >= ENTITIES_PER_PAGE {
            return Err(DirectoryError::LocalOutOfRange(dest_local));
        }
        let dest_i = dest_local as usize;
        let dest_key = Self::owner_key(dest_page_id, dest_local);
        self.ensure_owner_slot(dest_key);
        if let Some(other) = self.owners[dest_key] {
            return Err(DirectoryError::SlotOccupied {
                entity: entity_id,
                existing: other,
            });
        }
        if dest_page.is_active(dest_i) {
            return Err(DirectoryError::SlotOccupied {
                entity: entity_id,
                existing: u32::MAX,
            });
        }

        // Contract: `cell_origin_*` on a live page must be bit-identical to
        // `dest_cell.origin_f64()` (write only via that helper / empty-page init here).
        // No epsilon — alternate writers that recompute origin differently must not exist.
        let dest_origin = dest_cell.origin_f64();
        if Self::page_has_active(dest_page) {
            let actual = [
                dest_page.cell_origin_x,
                dest_page.cell_origin_y,
                dest_page.cell_origin_z,
            ];
            if actual != dest_origin {
                return Err(DirectoryError::OriginMismatch {
                    expected: dest_origin,
                    actual,
                });
            }
        }

        let world = [
            src_page.cell_origin_x + f64::from(src_page.soa.pos_x[src_i]),
            src_page.cell_origin_y + f64::from(src_page.soa.pos_y[src_i]),
            src_page.cell_origin_z + f64::from(src_page.soa.pos_z[src_i]),
        ];
        let local = [
            (world[0] - dest_origin[0]) as f32,
            (world[1] - dest_origin[1]) as f32,
            (world[2] - dest_origin[2]) as f32,
        ];

        if !Self::page_has_active(dest_page) {
            dest_page.cell_origin_x = dest_origin[0];
            dest_page.cell_origin_y = dest_origin[1];
            dest_page.cell_origin_z = dest_origin[2];
        }

        dest_page.set_active(dest_i, true);
        dest_page.soa.pos_x[dest_i] = local[0];
        dest_page.soa.pos_y[dest_i] = local[1];
        dest_page.soa.pos_z[dest_i] = local[2];
        dest_page.soa.delta_x[dest_i] = src_page.soa.delta_x[src_i];
        dest_page.soa.delta_y[dest_i] = src_page.soa.delta_y[src_i];
        dest_page.soa.delta_z[dest_i] = src_page.soa.delta_z[src_i];
        dest_page.soa.drot_x[dest_i] = src_page.soa.drot_x[src_i];
        dest_page.soa.drot_y[dest_i] = src_page.soa.drot_y[src_i];
        dest_page.soa.drot_z[dest_i] = src_page.soa.drot_z[src_i];
        dest_page.soa.inv_mass[dest_i] = src_page.soa.inv_mass[src_i];
        dest_page.soa.inv_inertia_x[dest_i] = src_page.soa.inv_inertia_x[src_i];
        dest_page.soa.inv_inertia_y[dest_i] = src_page.soa.inv_inertia_y[src_i];
        dest_page.soa.inv_inertia_z[dest_i] = src_page.soa.inv_inertia_z[src_i];
        dest_page.soa.damping[dest_i] = src_page.soa.damping[src_i];
        dest_page.soa.rot_x[dest_i] = src_page.soa.rot_x[src_i];
        dest_page.soa.rot_y[dest_i] = src_page.soa.rot_y[src_i];
        dest_page.soa.rot_z[dest_i] = src_page.soa.rot_z[src_i];
        dest_page.soa.rot_w[dest_i] = src_page.soa.rot_w[src_i];
        let holder = src_page.holder_at(src_i);
        let since = src_page.holder_since_at(src_i);
        if holder == NO_HOLDER {
            dest_page.clear_holder(dest_i);
        } else {
            let _ = dest_page.set_holder(dest_i, holder, since);
        }
        dest_page.set_sleeping(dest_i, src_page.is_sleeping(src_i));
        dest_page.mark_dirty(dest_i);

        let new_slot = PageSlot {
            cell: dest_cell,
            page: dest_page_id,
            local: dest_local,
        };
        // Directory update is infallible after the checks above.
        self.relocate_unchecked(entity_id, src, new_slot);
        src_page.quarantine_slot(src_i);
        Ok(new_slot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mvcc::page::HotPhysicsPage;

    #[test]
    fn from_world_uses_center_origin() {
        // World -10 is in cell 0 (center 0), not cell -1.
        assert_eq!(CellId::from_world_f64(-10.0, 0.0, 0.0), CellId([0, 0, 0]));
        assert_eq!(CellId::from_world_f64(0.0, 0.0, 0.0), CellId([0, 0, 0]));
        assert_eq!(CellId::from_world_f64(31.9, 0.0, 0.0), CellId([0, 0, 0]));
        assert_eq!(CellId::from_world_f64(32.0, 0.0, 0.0), CellId([1, 0, 0]));
        assert_eq!(CellId::from_world_f64(-32.0, 0.0, 0.0), CellId([0, 0, 0]));
        assert_eq!(CellId::from_world_f64(-32.1, 0.0, 0.0), CellId([-1, 0, 0]));
        assert_eq!(CellId([1, 0, 0]).origin_f64()[0], 64.0);
    }

    #[test]
    fn insert_get_remove() {
        let mut dir = EntityDirectory::new(8);
        let slot = PageSlot {
            cell: CellId([0, 0, 0]),
            page: PageId(1),
            local: 3,
        };
        dir.insert(10, slot).unwrap();
        assert_eq!(dir.get(10), Some(slot));
        assert_eq!(dir.remove(10).unwrap(), slot);
        assert!(dir.get(10).is_none());
    }

    #[test]
    fn insert_reports_true_occupant() {
        let mut dir = EntityDirectory::new(8);
        dir.insert(
            1,
            PageSlot {
                cell: CellId([0, 0, 0]),
                page: PageId(0),
                local: 0,
            },
        )
        .unwrap();
        let err = dir.insert(
            2,
            PageSlot {
                cell: CellId([0, 0, 0]),
                page: PageId(0),
                local: 0,
            },
        );
        assert_eq!(
            err,
            Err(DirectoryError::SlotOccupied {
                entity: 2,
                existing: 1
            })
        );
        assert!(matches!(
            dir.insert(
                1,
                PageSlot {
                    cell: CellId([1, 0, 0]),
                    page: PageId(1),
                    local: 0,
                },
            ),
            Err(DirectoryError::AlreadyMapped(1))
        ));
    }

    #[test]
    fn hysteresis_avoids_flap_near_boundary() {
        let cell = CellId([0, 0, 0]);
        assert!(EntityDirectory::should_cross_cell(cell, [30.0, 0.0, 0.0]).is_none());
        assert!(EntityDirectory::should_cross_cell(cell, [33.0, 0.0, 0.0]).is_none());
        assert_eq!(
            EntityDirectory::should_cross_cell(cell, [35.0, 0.0, 0.0]),
            Some(CellId([1, 0, 0]))
        );
    }

    #[test]
    fn cross_cell_atomic_single_occupancy() {
        let mut dir = EntityDirectory::new(4);
        let mut src = HotPhysicsPage::zeroed_page();
        let mut dest = HotPhysicsPage::zeroed_page();
        src.cell_origin_x = 0.0;
        src.cell_origin_y = 0.0;
        src.cell_origin_z = 0.0;
        src.set_active(0, true);
        src.soa.pos_x[0] = 35.0;
        src.soa.pos_y[0] = 1.0;
        src.soa.delta_x[0] = 0.1;
        src.soa.drot_y[0] = 0.2;
        assert!(src.set_holder(0, 2, 11));

        let src_slot = PageSlot {
            cell: CellId([0, 0, 0]),
            page: PageId(0),
            local: 0,
        };
        dir.insert(1, src_slot).unwrap();

        let dest_cell = CellId([1, 0, 0]);
        let new_slot = dir
            .cross_cell(1, &mut src, &mut dest, PageId(1), dest_cell, 5)
            .unwrap();

        assert!(!src.is_active(0));
        assert!(dest.is_active(5));
        assert_eq!(dir.get(1), Some(new_slot));
        assert_eq!(new_slot.local, 5);
        assert_eq!(new_slot.cell, dest_cell);
        assert!((dest.soa.pos_x[5] - (-29.0)).abs() < 1e-3);
        assert_eq!(dest.soa.delta_x[5].to_bits(), 0.1f32.to_bits());
        assert_eq!(dest.soa.drot_y[5].to_bits(), 0.2f32.to_bits());
        assert_eq!(dest.holder_at(5), 2);
        assert_eq!(src.holder_at(0), NO_HOLDER);
    }

    #[test]
    fn cross_cell_rejects_origin_overwrite_on_occupied_page() {
        let mut dir = EntityDirectory::new(4);
        let mut src = HotPhysicsPage::zeroed_page();
        let mut dest = HotPhysicsPage::zeroed_page();
        src.set_active(0, true);
        src.soa.pos_x[0] = 35.0;
        dest.cell_origin_x = 999.0; // wrong for cell 1
        dest.set_active(1, true); // already occupied page
        dest.soa.pos_x[1] = 0.0;
        dir.insert(
            1,
            PageSlot {
                cell: CellId([0, 0, 0]),
                page: PageId(0),
                local: 0,
            },
        )
        .unwrap();
        dir.insert(
            2,
            PageSlot {
                cell: CellId([1, 0, 0]),
                page: PageId(1),
                local: 1,
            },
        )
        .unwrap();

        let err = dir.cross_cell(1, &mut src, &mut dest, PageId(1), CellId([1, 0, 0]), 5);
        assert!(matches!(err, Err(DirectoryError::OriginMismatch { .. })));
        assert!(src.is_active(0));
        assert_eq!(dir.get(1).unwrap().local, 0);
        assert!(!dest.is_active(5));
    }

    #[test]
    fn entity_never_on_two_pages() {
        let mut dir = EntityDirectory::new(4);
        dir.insert(
            1,
            PageSlot {
                cell: CellId([0, 0, 0]),
                page: PageId(0),
                local: 0,
            },
        )
        .unwrap();
        let err = dir.insert(
            1,
            PageSlot {
                cell: CellId([1, 0, 0]),
                page: PageId(1),
                local: 0,
            },
        );
        assert!(matches!(err, Err(DirectoryError::AlreadyMapped(1))));
    }
}
