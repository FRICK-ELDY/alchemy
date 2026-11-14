//! Epoch SoA tape (`Tape` / `EpochSheet`) — first-principles continuous memory.
//!
//! Guarantees: `row_id` never moves after alloc; `ReadinessMask` gates readiness;
//! hierarchical bitset skips empty 64-row blocks; `is_finite` rejects NaN/Inf writes;
//! `PendingPatch` applies field diffs without full-sheet copy.
//! Non-guarantees: CRDT / Zenoh / Hydration wiring; Rapier interop.
//!
//! Safety: this module forbids `unsafe` (`#![forbid(unsafe_code)]`). Memory reinterpret
//! uses `bytemuck::Pod` / `Zeroable` only.

#![forbid(unsafe_code)]

use bytemuck::{Pod, Zeroable};

/// Generation-tagged row id (ABA-safe; index immutable for row lifetime).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Pod, Zeroable)]
pub struct NodeHandle {
    pub row_id: u32,
    pub generation: u32,
}

/// Readiness & physics mode bitmask (Awake / OnDestroy / Grab unified).
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct ReadinessMask(pub u64);

impl ReadinessMask {
    pub const ALLOCATED: u64 = 1 << 0;
    pub const TRANSFORM: u64 = 1 << 1;
    pub const MESH: u64 = 1 << 2;
    pub const SHADER: u64 = 1 << 3;
    pub const COLLIDER: u64 = 1 << 4;
    pub const DYNAMIC: u64 = 1 << 5;
    pub const GRABBED: u64 = 1 << 6;
    pub const SLEEPING: u64 = 1 << 7;
    pub const LOGIC: u64 = 1 << 8;
    pub const TOMBSTONE: u64 = 1 << 63;

    #[inline(always)]
    pub const fn is_ready(self, required: u64) -> bool {
        (self.0 & (required | Self::TOMBSTONE)) == required
    }

    #[inline(always)]
    pub const fn contains(self, bit: u64) -> bool {
        (self.0 & bit) == bit
    }

    #[inline(always)]
    pub const fn with_bits(self, bits: u64) -> Self {
        Self(self.0 | bits)
    }

    #[inline(always)]
    pub const fn without_bits(self, bits: u64) -> Self {
        Self(self.0 & !bits)
    }
}

/// Collider shape primitive (16-byte aligned).
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct ColliderShape {
    /// 0: None, 1: Sphere, 2: Capsule, 3: Box, 4: StaticSdfAtom
    pub shape_type: u32,
    /// Sphere: [r,0,0], Capsule: [r, half_h, 0], Box: [hx, hy, hz]
    pub extents: [f32; 3],
}

impl ColliderShape {
    pub const NONE: u32 = 0;
    pub const SPHERE: u32 = 1;
    pub const CAPSULE: u32 = 2;
    pub const BOX: u32 = 3;
    pub const STATIC_SDF_ATOM: u32 = 4;
}

/// Unified XPBD constraint row (joint / Grab / IK / spring / contact).
#[repr(C, align(32))]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct XpbdConstraintRow {
    pub node_a: u32,
    /// World-fixed / Grab target when `u32::MAX`.
    pub node_b: u32,
    pub target_distance: f32,
    pub compliance: f32,
    pub damping: f32,
    pub anchor_offset_a: [f32; 3],
    pub anchor_offset_b: [f32; 3],
    /// Bit 0: inequality contact — skip when `dist >= target_distance` (no attract).
    pub flags: u32,
    pub _pad: [u32; 4],
}

impl XpbdConstraintRow {
    /// Inequality (non-penetration) contact constraint.
    pub const FLAG_INEQUALITY: u32 = 1;

    #[inline]
    pub fn is_inequality(self) -> bool {
        (self.flags & Self::FLAG_INEQUALITY) != 0
    }
}

/// High-precision world transform (binary64 position + XPBD state).
/// Explicit trailing pad keeps size a multiple of 32 with no implicit holes (Pod).
#[repr(C, align(32))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct WorldTransformf64 {
    pub position: [f64; 3],
    pub inv_mass: f64,
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    pub frame_delta_xyz: [f32; 3],
    pub flags: u32,
    pub _pad0: u32,
    pub _pad1: [u32; 4],
}

impl WorldTransformf64 {
    /// Identity-ish default: unit quat, unit scale, dynamic mass.
    pub fn identity_at(position: [f64; 3]) -> Self {
        Self {
            position,
            inv_mass: 1.0,
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
            frame_delta_xyz: [0.0, 0.0, 0.0],
            flags: 0,
            _pad0: 0,
            _pad1: [0; 4],
        }
    }

    #[inline]
    pub fn is_finite(&self) -> bool {
        self.position.iter().all(|v| v.is_finite())
            && self.inv_mass.is_finite()
            && self.rotation.iter().all(|v| v.is_finite())
            && self.scale.iter().all(|v| v.is_finite())
            && self.frame_delta_xyz.iter().all(|v| v.is_finite())
    }
}

/// Field-level diff applied at the frame-boundary patch merge.
#[derive(Clone, Debug, PartialEq)]
pub enum PendingPatch {
    SetReadiness {
        row_id: u32,
        mask: ReadinessMask,
    },
    OrReadiness {
        row_id: u32,
        bits: u64,
    },
    ClearReadiness {
        row_id: u32,
        bits: u64,
    },
    SetCollider {
        row_id: u32,
        shape: ColliderShape,
    },
    SetAssetAtom {
        row_id: u32,
        blake3: [u8; 32],
    },
    SetOwnerScope {
        row_id: u32,
        scope: u32,
    },
    /// Editor transform commit. Skipped when `preserve_physics_coords` and row is DYNAMIC.
    SetTransform {
        row_id: u32,
        transform: WorldTransformf64,
        preserve_physics_coords: bool,
    },
}

/// One epoch of SoA columns. `row_id` is stable from alloc until freelist reuse.
#[derive(Clone, Debug, Default)]
pub struct EpochSheet {
    pub epoch_id: u64,
    pub active_summary_words: Vec<u64>,
    pub readiness: Vec<ReadinessMask>,
    pub world_transforms: Vec<WorldTransformf64>,
    pub colliders: Vec<ColliderShape>,
    pub asset_atoms: Vec<[u8; 32]>,
    pub owner_scope: Vec<u32>,
    pub dirty_summary_words: Vec<u64>,
    /// Per-row generation for ABA-safe handles (not in wire snapshot).
    generations: Vec<u32>,
    free_list: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapeError {
    OutOfBounds,
    Tombstoned,
    StaleGeneration,
    NonFinite,
    PhysicsCoordsPreserved,
}

impl EpochSheet {
    pub fn with_capacity(cap: usize) -> Self {
        let words = summary_word_count(cap);
        Self {
            epoch_id: 0,
            active_summary_words: vec![0; words],
            readiness: Vec::with_capacity(cap),
            world_transforms: Vec::with_capacity(cap),
            colliders: Vec::with_capacity(cap),
            asset_atoms: Vec::with_capacity(cap),
            owner_scope: Vec::with_capacity(cap),
            dirty_summary_words: vec![0; words],
            generations: Vec::with_capacity(cap),
            free_list: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.readiness.len()
    }

    pub fn is_empty(&self) -> bool {
        self.readiness.is_empty()
    }

    /// Allocate a stable row. Freed rows reuse `row_id` with bumped generation.
    pub fn alloc_row(&mut self) -> NodeHandle {
        if let Some(row_id) = self.free_list.pop() {
            let idx = row_id as usize;
            let generation = self.generations[idx].wrapping_add(1);
            self.generations[idx] = generation;
            self.readiness[idx] = ReadinessMask(ReadinessMask::ALLOCATED);
            self.world_transforms[idx] = WorldTransformf64::identity_at([0.0, 0.0, 0.0]);
            self.colliders[idx] = ColliderShape::default();
            self.asset_atoms[idx] = [0; 32];
            self.owner_scope[idx] = 0;
            self.set_active(row_id, true);
            self.mark_dirty(row_id);
            return NodeHandle { row_id, generation };
        }

        let row_id = self.readiness.len() as u32;
        self.readiness.push(ReadinessMask(ReadinessMask::ALLOCATED));
        self.world_transforms
            .push(WorldTransformf64::identity_at([0.0, 0.0, 0.0]));
        self.colliders.push(ColliderShape::default());
        self.asset_atoms.push([0; 32]);
        self.owner_scope.push(0);
        self.generations.push(0);
        self.ensure_summary_capacity(self.readiness.len());
        self.set_active(row_id, true);
        self.mark_dirty(row_id);
        NodeHandle {
            row_id,
            generation: 0,
        }
    }

    /// Soft-delete: row stays in place; handle generation must match to reuse later.
    pub fn tombstone(&mut self, handle: NodeHandle) -> Result<(), TapeError> {
        self.validate_handle(handle)?;
        self.mark_tombstone_slot(handle.row_id);
        Ok(())
    }

    pub fn generation(&self, row_id: u32) -> Option<u32> {
        self.generations.get(row_id as usize).copied()
    }

    /// Grow SoA length by appending slots (never pops `free_list` — safe for Prev mirror sync).
    pub fn ensure_len_append(&mut self, len: usize) {
        while self.readiness.len() < len {
            let row_id = self.readiness.len() as u32;
            self.readiness.push(ReadinessMask(ReadinessMask::TOMBSTONE));
            self.world_transforms
                .push(WorldTransformf64::identity_at([0.0, 0.0, 0.0]));
            self.colliders.push(ColliderShape::default());
            self.asset_atoms.push([0; 32]);
            self.owner_scope.push(0);
            self.generations.push(0);
            self.ensure_summary_capacity(self.readiness.len());
            self.set_active(row_id, false);
        }
    }

    /// Copy one row (including generation) from `src` into this sheet.
    ///
    /// Keeps `free_list` consistent with [`Self::mark_tombstone_slot`] /
    /// SetReadiness resurrect: live→tomb pushes once; tomb→live scrubs.
    pub fn copy_row_from(&mut self, dst_row: u32, src: &EpochSheet, src_row: u32) {
        let d = dst_row as usize;
        let s = src_row as usize;
        if d >= self.readiness.len() || s >= src.readiness.len() {
            return;
        }
        let src_tomb = src.readiness[s].contains(ReadinessMask::TOMBSTONE);
        let was_tomb = self.readiness[d].contains(ReadinessMask::TOMBSTONE);
        self.readiness[d] = src.readiness[s];
        self.world_transforms[d] = src.world_transforms[s];
        self.colliders[d] = src.colliders[s];
        self.asset_atoms[d] = src.asset_atoms[s];
        self.owner_scope[d] = src.owner_scope[s];
        self.generations[d] = src.generations[s];
        self.set_active(dst_row, !src_tomb);
        match (was_tomb, src_tomb) {
            (true, false) => self.free_list.retain(|&id| id != dst_row),
            (false, true) => self.free_list.push(dst_row),
            _ => {}
        }
        self.mark_dirty(dst_row);
    }

    fn mark_tombstone_slot(&mut self, row_id: u32) {
        let idx = row_id as usize;
        if idx >= self.readiness.len() {
            return;
        }
        let was_tomb = self.readiness[idx].contains(ReadinessMask::TOMBSTONE);
        self.readiness[idx] = ReadinessMask(ReadinessMask::TOMBSTONE);
        self.set_active(row_id, false);
        self.mark_dirty(row_id);
        // `was_tomb` avoids duplicate free_list entries without O(n) contains.
        if !was_tomb {
            self.free_list.push(row_id);
        }
    }

    pub fn validate_handle(&self, handle: NodeHandle) -> Result<(), TapeError> {
        let idx = handle.row_id as usize;
        if idx >= self.readiness.len() {
            return Err(TapeError::OutOfBounds);
        }
        if self.generations[idx] != handle.generation {
            return Err(TapeError::StaleGeneration);
        }
        if self.readiness[idx].contains(ReadinessMask::TOMBSTONE) {
            return Err(TapeError::Tombstoned);
        }
        Ok(())
    }

    pub fn set_transform(
        &mut self,
        row_id: u32,
        transform: WorldTransformf64,
    ) -> Result<(), TapeError> {
        let idx = row_id as usize;
        if idx >= self.world_transforms.len() {
            return Err(TapeError::OutOfBounds);
        }
        if self.readiness[idx].contains(ReadinessMask::TOMBSTONE) {
            return Err(TapeError::Tombstoned);
        }
        if !transform.is_finite() {
            return Err(TapeError::NonFinite);
        }
        self.world_transforms[idx] = transform;
        self.mark_dirty(row_id);
        Ok(())
    }

    pub fn apply_patches(&mut self, patches: &[PendingPatch]) -> Vec<(usize, TapeError)> {
        let mut errors = Vec::new();
        for (i, patch) in patches.iter().enumerate() {
            if let Err(e) = self.apply_one(patch) {
                errors.push((i, e));
            }
        }
        self.epoch_id = self.epoch_id.wrapping_add(1);
        errors
    }

    fn apply_one(&mut self, patch: &PendingPatch) -> Result<(), TapeError> {
        match *patch {
            PendingPatch::SetReadiness { row_id, mask } => {
                // Allow writing TOMBSTONE even if already allocated; do not use
                // `row_index` (rejects tombstones) so patches can delete rows.
                let idx = row_id as usize;
                if idx >= self.readiness.len() {
                    return Err(TapeError::OutOfBounds);
                }
                if mask.contains(ReadinessMask::TOMBSTONE) {
                    self.mark_tombstone_slot(row_id);
                } else {
                    let was_tomb = self.readiness[idx].contains(ReadinessMask::TOMBSTONE);
                    if was_tomb {
                        // Resurrection only: O(n) scrub; live→live skips retain.
                        self.free_list.retain(|&id| id != row_id);
                    }
                    self.readiness[idx] = mask;
                    self.set_active(row_id, true);
                    self.mark_dirty(row_id);
                }
            }
            PendingPatch::OrReadiness { row_id, bits } => {
                let idx = row_id as usize;
                if idx >= self.readiness.len() {
                    return Err(TapeError::OutOfBounds);
                }
                if bits & ReadinessMask::TOMBSTONE != 0 {
                    self.mark_tombstone_slot(row_id);
                } else {
                    if self.readiness[idx].contains(ReadinessMask::TOMBSTONE) {
                        return Err(TapeError::Tombstoned);
                    }
                    self.readiness[idx] = self.readiness[idx].with_bits(bits);
                    self.mark_dirty(row_id);
                }
            }
            PendingPatch::ClearReadiness { row_id, bits } => {
                let idx = self.row_index(row_id)?;
                self.readiness[idx] = self.readiness[idx].without_bits(bits);
                self.mark_dirty(row_id);
            }
            PendingPatch::SetCollider { row_id, shape } => {
                let idx = self.row_index(row_id)?;
                self.colliders[idx] = shape;
                self.mark_dirty(row_id);
            }
            PendingPatch::SetAssetAtom { row_id, blake3 } => {
                let idx = self.row_index(row_id)?;
                self.asset_atoms[idx] = blake3;
                self.mark_dirty(row_id);
            }
            PendingPatch::SetOwnerScope { row_id, scope } => {
                let idx = self.row_index(row_id)?;
                self.owner_scope[idx] = scope;
                self.mark_dirty(row_id);
            }
            PendingPatch::SetTransform {
                row_id,
                transform,
                preserve_physics_coords,
            } => {
                let idx = self.row_index(row_id)?;
                if preserve_physics_coords && self.readiness[idx].is_ready(ReadinessMask::DYNAMIC) {
                    return Err(TapeError::PhysicsCoordsPreserved);
                }
                self.set_transform(row_id, transform)?;
            }
        }
        Ok(())
    }

    fn row_index(&self, row_id: u32) -> Result<usize, TapeError> {
        let idx = row_id as usize;
        if idx >= self.readiness.len() {
            return Err(TapeError::OutOfBounds);
        }
        if self.readiness[idx].contains(ReadinessMask::TOMBSTONE) {
            return Err(TapeError::Tombstoned);
        }
        Ok(idx)
    }

    #[inline]
    pub fn set_active(&mut self, row_id: u32, active: bool) {
        let (wi, bi) = word_bit(row_id);
        self.ensure_summary_capacity((row_id as usize) + 1);
        if active {
            self.active_summary_words[wi] |= 1u64 << bi;
        } else {
            self.active_summary_words[wi] &= !(1u64 << bi);
        }
    }

    /// Fixed-index install for MVCC page adapter (`row_id == local_idx`).
    ///
    /// Keeps [`Self::free_list`] consistent with [`PendingPatch::SetReadiness`] /
    /// [`Self::copy_row_from`]. Live writes apply the same finite gate as
    /// [`Self::set_transform`]. Prefer this over raw readiness writes when the same
    /// sheet may also use [`Self::alloc_row`].
    pub fn write_fixed_slot(
        &mut self,
        row_id: u32,
        readiness: ReadinessMask,
        transform: WorldTransformf64,
        collider: ColliderShape,
    ) -> Result<(), TapeError> {
        let idx = row_id as usize;
        if idx >= self.readiness.len() {
            return Err(TapeError::OutOfBounds);
        }
        if readiness.contains(ReadinessMask::TOMBSTONE) {
            self.mark_tombstone_slot(row_id);
            return Ok(());
        }
        if !transform.is_finite() {
            return Err(TapeError::NonFinite);
        }
        let was_tomb = self.readiness[idx].contains(ReadinessMask::TOMBSTONE);
        if was_tomb {
            self.free_list.retain(|&id| id != row_id);
        }
        self.readiness[idx] = readiness;
        self.world_transforms[idx] = transform;
        self.colliders[idx] = collider;
        self.set_active(row_id, true);
        self.mark_dirty(row_id);
        Ok(())
    }

    #[inline]
    pub fn mark_dirty(&mut self, row_id: u32) {
        let (wi, bi) = word_bit(row_id);
        self.ensure_summary_capacity((row_id as usize) + 1);
        self.dirty_summary_words[wi] |= 1u64 << bi;
    }

    pub fn clear_dirty(&mut self) {
        self.dirty_summary_words.fill(0);
    }

    fn ensure_summary_capacity(&mut self, row_count: usize) {
        let need = summary_word_count(row_count);
        if self.active_summary_words.len() < need {
            self.active_summary_words.resize(need, 0);
        }
        if self.dirty_summary_words.len() < need {
            self.dirty_summary_words.resize(need, 0);
        }
    }

    /// Iterate active (non-tombstone summary) row ids via `trailing_zeros` skip.
    pub fn iter_active_rows(&self) -> ActiveRowIter<'_> {
        ActiveRowIter {
            words: &self.active_summary_words,
            word_index: 0,
            current: self.active_summary_words.first().copied().unwrap_or(0),
        }
    }

    pub fn iter_dirty_rows(&self) -> ActiveRowIter<'_> {
        ActiveRowIter {
            words: &self.dirty_summary_words,
            word_index: 0,
            current: self.dirty_summary_words.first().copied().unwrap_or(0),
        }
    }
}

#[inline]
fn word_bit(row_id: u32) -> (usize, u32) {
    ((row_id / 64) as usize, row_id % 64)
}

#[inline]
fn summary_word_count(row_count: usize) -> usize {
    row_count.div_ceil(64)
}

/// Hierarchical bitset scanner (`tzcnt` / `trailing_zeros`).
pub struct ActiveRowIter<'a> {
    words: &'a [u64],
    word_index: usize,
    current: u64,
}

impl Iterator for ActiveRowIter<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.current != 0 {
                let bit = self.current.trailing_zeros();
                self.current &= self.current - 1;
                return Some((self.word_index as u32) * 64 + bit);
            }
            self.word_index += 1;
            if self.word_index >= self.words.len() {
                return None;
            }
            self.current = self.words[self.word_index];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, size_of};

    #[test]
    fn readiness_tombstone_fails_ready() {
        let m = ReadinessMask(ReadinessMask::ALLOCATED | ReadinessMask::TOMBSTONE);
        assert!(!m.is_ready(ReadinessMask::ALLOCATED));
        let ok = ReadinessMask(ReadinessMask::ALLOCATED | ReadinessMask::TRANSFORM);
        assert!(ok.is_ready(ReadinessMask::ALLOCATED | ReadinessMask::TRANSFORM));
    }

    #[test]
    fn pod_layout_asserts() {
        assert_eq!(size_of::<NodeHandle>(), 8);
        assert_eq!(size_of::<ReadinessMask>(), 8);
        assert_eq!(size_of::<ColliderShape>(), 16);
        assert_eq!(align_of::<ColliderShape>(), 16);
        assert_eq!(size_of::<XpbdConstraintRow>(), 64);
        assert_eq!(align_of::<XpbdConstraintRow>(), 32);
        assert_eq!(align_of::<WorldTransformf64>(), 32);
        assert_eq!(size_of::<WorldTransformf64>() % 32, 0);
        let _ = bytemuck::bytes_of(&WorldTransformf64::default());
        let _ = bytemuck::bytes_of(&ColliderShape::default());
        let _ = bytemuck::bytes_of(&XpbdConstraintRow::default());
    }

    #[test]
    fn alloc_tombstone_reuses_row_with_new_generation() {
        let mut sheet = EpochSheet::with_capacity(4);
        let a = sheet.alloc_row();
        assert_eq!(a.row_id, 0);
        assert_eq!(a.generation, 0);
        sheet.tombstone(a).unwrap();
        assert!(sheet.validate_handle(a).is_err());
        let b = sheet.alloc_row();
        assert_eq!(b.row_id, 0);
        assert_eq!(b.generation, 1);
        assert!(sheet.validate_handle(a).is_err());
        assert!(sheet.validate_handle(b).is_ok());
    }

    #[test]
    fn hierarchical_bitset_skips_empty_words() {
        let mut sheet = EpochSheet::with_capacity(128);
        // Force capacity across 2+ words without marking all active.
        for _ in 0..130 {
            let _ = sheet.alloc_row();
        }
        for i in 0..130u32 {
            sheet
                .tombstone(NodeHandle {
                    row_id: i,
                    generation: 0,
                })
                .unwrap();
        }
        // Re-activate sparse rows in word 0 and word 2 territory.
        let h0 = sheet.alloc_row();
        let mut last = h0;
        for _ in 0..128 {
            last = sheet.alloc_row();
        }
        // Clear and set only a few actives far apart.
        sheet.active_summary_words.fill(0);
        sheet.set_active(0, true);
        sheet.set_active(64, true);
        sheet.set_active(129, true);
        let rows: Vec<_> = sheet.iter_active_rows().collect();
        assert_eq!(rows, vec![0, 64, 129]);
        let _ = last;
    }

    #[test]
    fn non_finite_transform_rejected() {
        let mut sheet = EpochSheet::with_capacity(1);
        let h = sheet.alloc_row();
        let mut bad = WorldTransformf64::identity_at([0.0, 0.0, 0.0]);
        bad.position[1] = f64::NAN;
        assert_eq!(
            sheet.set_transform(h.row_id, bad),
            Err(TapeError::NonFinite)
        );
    }

    #[test]
    fn set_readiness_tombstone_enters_free_list() {
        let mut sheet = EpochSheet::with_capacity(2);
        let a = sheet.alloc_row();
        let _ = sheet.alloc_row();
        let errs = sheet.apply_patches(&[PendingPatch::SetReadiness {
            row_id: a.row_id,
            mask: ReadinessMask(ReadinessMask::TOMBSTONE),
        }]);
        assert!(errs.is_empty());
        let reused = sheet.alloc_row();
        assert_eq!(reused.row_id, a.row_id);
        assert_eq!(reused.generation, 1);
        assert!(sheet.validate_handle(a).is_err());
    }

    #[test]
    fn copy_row_from_live_to_tomb_enters_free_list() {
        let mut dst = EpochSheet::with_capacity(1);
        let mut src = EpochSheet::with_capacity(1);
        let h = dst.alloc_row();
        let s = src.alloc_row();
        src.tombstone(s).unwrap();
        dst.copy_row_from(h.row_id, &src, s.row_id);
        assert!(dst.readiness[h.row_id as usize].contains(ReadinessMask::TOMBSTONE));
        let reused = dst.alloc_row();
        assert_eq!(reused.row_id, h.row_id);
    }

    #[test]
    fn set_readiness_resurrect_removes_from_free_list() {
        let mut sheet = EpochSheet::with_capacity(1);
        let a = sheet.alloc_row();
        sheet.apply_patches(&[PendingPatch::SetReadiness {
            row_id: a.row_id,
            mask: ReadinessMask(ReadinessMask::TOMBSTONE),
        }]);
        sheet.apply_patches(&[PendingPatch::SetReadiness {
            row_id: a.row_id,
            mask: ReadinessMask(ReadinessMask::ALLOCATED),
        }]);
        let b = sheet.alloc_row();
        // Must append a new row_id, not reuse resurrected slot via free_list.
        assert_eq!(b.row_id, 1);
    }

    #[test]
    fn or_readiness_tombstone_enters_free_list() {
        let mut sheet = EpochSheet::with_capacity(1);
        let a = sheet.alloc_row();
        let errs = sheet.apply_patches(&[PendingPatch::OrReadiness {
            row_id: a.row_id,
            bits: ReadinessMask::TOMBSTONE,
        }]);
        assert!(errs.is_empty());
        let reused = sheet.alloc_row();
        assert_eq!(reused.row_id, a.row_id);
        assert_eq!(reused.generation, 1);
    }

    #[test]
    fn ensure_len_append_ignores_freelist() {
        let mut sheet = EpochSheet::with_capacity(2);
        let a = sheet.alloc_row();
        sheet.tombstone(a).unwrap();
        assert_eq!(sheet.len(), 1);
        sheet.ensure_len_append(3);
        assert_eq!(sheet.len(), 3);
        // freelist still has `a`; append did not consume it
        let reused = sheet.alloc_row();
        assert_eq!(reused.row_id, a.row_id);
    }

    #[test]
    fn pending_patch_preserves_physics_coords() {
        let mut sheet = EpochSheet::with_capacity(2);
        let a = sheet.alloc_row();
        let b = sheet.alloc_row();
        sheet
            .set_transform(a.row_id, WorldTransformf64::identity_at([1.0, 2.0, 3.0]))
            .unwrap();
        sheet.readiness[a.row_id as usize] =
            ReadinessMask(ReadinessMask::ALLOCATED | ReadinessMask::DYNAMIC);
        let patches = vec![
            PendingPatch::SetTransform {
                row_id: a.row_id,
                transform: WorldTransformf64::identity_at([9.0, 9.0, 9.0]),
                preserve_physics_coords: true,
            },
            PendingPatch::SetTransform {
                row_id: b.row_id,
                transform: WorldTransformf64::identity_at([4.0, 5.0, 6.0]),
                preserve_physics_coords: true,
            },
        ];
        let errs = sheet.apply_patches(&patches);
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].1, TapeError::PhysicsCoordsPreserved);
        assert_eq!(
            sheet.world_transforms[a.row_id as usize].position,
            [1.0, 2.0, 3.0]
        );
        assert_eq!(
            sheet.world_transforms[b.row_id as usize].position,
            [4.0, 5.0, 6.0]
        );
    }

    #[test]
    fn write_fixed_slot_tombstone_and_resurrect_keeps_free_list() {
        let mut sheet = EpochSheet::with_capacity(2);
        sheet.ensure_len_append(2);
        let live = ReadinessMask(
            ReadinessMask::ALLOCATED | ReadinessMask::TRANSFORM | ReadinessMask::DYNAMIC,
        );
        let xf = WorldTransformf64::identity_at([1.0, 0.0, 0.0]);
        sheet
            .write_fixed_slot(0, live, xf, ColliderShape::default())
            .unwrap();
        assert!(!sheet.readiness[0].contains(ReadinessMask::TOMBSTONE));

        sheet
            .write_fixed_slot(
                0,
                ReadinessMask(ReadinessMask::TOMBSTONE),
                WorldTransformf64::default(),
                ColliderShape::default(),
            )
            .unwrap();
        assert!(sheet.readiness[0].contains(ReadinessMask::TOMBSTONE));

        // Freelist should reuse row 0 on alloc after fixed-slot tombstone.
        let reused = sheet.alloc_row();
        assert_eq!(reused.row_id, 0);

        // Pad slot 1 is still TOMBSTONE from ensure_len_append (not on freelist).
        // write_fixed_slot resurrect must scrub if it were listed; live write on 1 works.
        sheet
            .write_fixed_slot(1, live, xf, ColliderShape::default())
            .unwrap();
        assert!(!sheet.readiness[1].contains(ReadinessMask::TOMBSTONE));
        let next = sheet.alloc_row();
        // row 0 was allocated above; freelist empty → append
        assert_eq!(next.row_id, 2);
    }

    #[test]
    fn write_fixed_slot_rejects_non_finite() {
        let mut sheet = EpochSheet::with_capacity(1);
        sheet.ensure_len_append(1);
        let live = ReadinessMask(ReadinessMask::ALLOCATED | ReadinessMask::TRANSFORM);
        let mut bad = WorldTransformf64::identity_at([0.0, 0.0, 0.0]);
        bad.position[0] = f64::NAN;
        assert_eq!(
            sheet.write_fixed_slot(0, live, bad, ColliderShape::default()),
            Err(TapeError::NonFinite)
        );
        assert!(sheet.readiness[0].contains(ReadinessMask::TOMBSTONE));
    }
}
