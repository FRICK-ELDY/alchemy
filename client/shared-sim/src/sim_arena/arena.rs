//! Three SoA faces, end-of-tick role rotation, 2-frame dirty Carry-Over (x00db),
//! and SimPatch Drain onto Work (x00df).

use super::{LocalSlotId, STATE_TELEPORT_THIS_TICK};
use crate::scene_store::SimPatch;

/// Sentinel in durable parent column: no parent (world root / free).
const PARENT_ABSENT: u32 = u32::MAX;

/// Which logical face a buffer currently plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceRole {
    Prev,
    Published,
    Work,
}

/// One SoA face: cell-relative transforms + per-slot state bits.
///
/// Guarantees: all column lengths stay equal ([`Self::slot_count`]) via private
/// `Vec`s and [`Self::resize`]. Parent / attached / enabled / scale / procedural
/// box live on [`SimArena`] durable (non-rotated) columns — not on the face.
///
/// # Panics
///
/// Slot accessors panic if `slot as usize >= slot_count()` (debug builds also
/// `debug_assert`). Callers must use in-range [`LocalSlotId`]s.
#[derive(Clone, Debug, Default)]
pub struct SimFace {
    pos_x: Vec<f32>,
    pos_y: Vec<f32>,
    pos_z: Vec<f32>,
    rot_x: Vec<f32>,
    rot_y: Vec<f32>,
    rot_z: Vec<f32>,
    rot_w: Vec<f32>,
    state_bits: Vec<u64>,
}

impl SimFace {
    pub fn with_slots(slot_count: usize) -> Self {
        let mut face = Self::default();
        face.resize(slot_count);
        face
    }

    /// Resize every SoA column to the same logical slot count.
    pub fn resize(&mut self, slot_count: usize) {
        self.pos_x.resize(slot_count, 0.0);
        self.pos_y.resize(slot_count, 0.0);
        self.pos_z.resize(slot_count, 0.0);
        self.rot_x.resize(slot_count, 0.0);
        self.rot_y.resize(slot_count, 0.0);
        self.rot_z.resize(slot_count, 0.0);
        self.rot_w.resize(slot_count, 1.0);
        self.state_bits.resize(slot_count, 0);
    }

    /// Number of logical slots (equal column lengths). Not `Vec::capacity`.
    #[inline]
    pub fn slot_count(&self) -> usize {
        self.pos_x.len()
    }

    #[inline]
    pub fn pos_x(&self) -> &[f32] {
        &self.pos_x
    }

    #[inline]
    pub fn pos_y(&self) -> &[f32] {
        &self.pos_y
    }

    #[inline]
    pub fn pos_z(&self) -> &[f32] {
        &self.pos_z
    }

    #[inline]
    pub fn rot_x(&self) -> &[f32] {
        &self.rot_x
    }

    #[inline]
    pub fn rot_y(&self) -> &[f32] {
        &self.rot_y
    }

    #[inline]
    pub fn rot_z(&self) -> &[f32] {
        &self.rot_z
    }

    #[inline]
    pub fn rot_w(&self) -> &[f32] {
        &self.rot_w
    }

    #[inline]
    pub fn state_bits(&self) -> &[u64] {
        &self.state_bits
    }

    #[inline]
    fn index(&self, slot: LocalSlotId) -> usize {
        let i = slot as usize;
        debug_assert!(
            i < self.slot_count(),
            "LocalSlotId {slot} out of range (slot_count={})",
            self.slot_count()
        );
        i
    }

    /// # Panics
    /// If `slot` is out of range (see type docs).
    #[inline]
    pub fn set_pos(&mut self, slot: LocalSlotId, x: f32, y: f32, z: f32) {
        let i = self.index(slot);
        self.pos_x[i] = x;
        self.pos_y[i] = y;
        self.pos_z[i] = z;
    }

    /// # Panics
    /// If `slot` is out of range (see type docs).
    #[inline]
    pub fn pos(&self, slot: LocalSlotId) -> [f32; 3] {
        let i = self.index(slot);
        [self.pos_x[i], self.pos_y[i], self.pos_z[i]]
    }

    /// # Panics
    /// If `slot` is out of range (see type docs).
    #[inline]
    pub fn set_rot(&mut self, slot: LocalSlotId, x: f32, y: f32, z: f32, w: f32) {
        let i = self.index(slot);
        self.rot_x[i] = x;
        self.rot_y[i] = y;
        self.rot_z[i] = z;
        self.rot_w[i] = w;
    }

    /// # Panics
    /// If `slot` is out of range (see type docs).
    #[inline]
    pub fn rot(&self, slot: LocalSlotId) -> [f32; 4] {
        let i = self.index(slot);
        [self.rot_x[i], self.rot_y[i], self.rot_z[i], self.rot_w[i]]
    }

    /// # Panics
    /// If `slot` is out of range (see type docs).
    ///
    /// Does not auto-clear on rotate — see [`crate::sim_arena`] module docs.
    #[inline]
    pub fn set_teleport(&mut self, slot: LocalSlotId, on: bool) {
        let i = self.index(slot);
        if on {
            self.state_bits[i] |= STATE_TELEPORT_THIS_TICK;
        } else {
            self.state_bits[i] &= !STATE_TELEPORT_THIS_TICK;
        }
    }

    /// # Panics
    /// If `slot` is out of range (see type docs).
    #[inline]
    pub fn is_teleport(&self, slot: LocalSlotId) -> bool {
        let i = self.index(slot);
        (self.state_bits[i] & STATE_TELEPORT_THIS_TICK) != 0
    }

    #[cfg(test)]
    fn columns_aligned(&self) -> bool {
        let n = self.pos_x.len();
        self.pos_y.len() == n
            && self.pos_z.len() == n
            && self.rot_x.len() == n
            && self.rot_y.len() == n
            && self.rot_z.len() == n
            && self.rot_w.len() == n
            && self.state_bits.len() == n
    }
}

/// Triple-buffered Sim Arena (Prev / Published / Work) with 2-frame dirty
/// Carry-Over and SimPatch Drain.
///
/// Tick contract:
/// 1. Step 0 — [`Self::carry_over`] then [`Self::apply_patches`] (or
///    [`Self::step0_carry_and_apply_patches`]): `Work[slot] = Published[slot]`
///    for `prev_dirty ∪ prev2_dirty`, clear TELEPORT on those Work rows, then
///    drain Ring 1 patches onto Work (no page swap).
/// 2. Writers mutate Work via [`Self::write_pos`] / [`Self::write_rot`] /
///    [`Self::write_teleport`] (or [`Self::work_mut`] + [`Self::mark_dirty`]).
/// 3. [`Self::rotate_faces`] — Work→Published→Prev; clear TELEPORT on the face
///    leaving Published; shift dirty rings; advance [`Self::epoch`].
///
/// Durable (non-rotated) columns hold parent / attached / enabled / scale /
/// procedural-box params updated by patches. Face Carry-Over still copies only
/// **pos + rot + state_bits** (x00db contract unchanged).
///
/// Durable columns are **Arena-internal / Work-update-side**: reading them
/// mid-tick can mix Published face pose with newer parent/scale. Ring3 must not
/// treat them as co-published with Published until a later barrier slice.
#[derive(Debug)]
pub struct SimArena {
    faces: [SimFace; 3],
    /// Index of the face currently playing Prev / Published / Work.
    prev_i: u8,
    published_i: u8,
    work_i: u8,
    /// Monotonic epoch advanced only via [`Self::rotate_faces`].
    epoch: u64,
    /// Slots written during the current Work tick (before rotate).
    this_tick_dirty: Vec<LocalSlotId>,
    /// Dedup flags for [`Self::this_tick_dirty`] (index = slot).
    this_tick_marked: Vec<bool>,
    /// Scratch marks for [`Self::carry_over`] union dedup (index = slot).
    carry_seen: Vec<bool>,
    /// Slots written on the tick that produced current Published.
    prev_dirty_slots: Vec<LocalSlotId>,
    /// Slots written on the tick that produced current Prev.
    prev2_dirty_slots: Vec<LocalSlotId>,
    /// Last Work-generation (`epoch + 1` at write time) that touched pos/rot.
    transform_dirty_epoch: u64,
    /// Last Work-generation that touched face `state_bits` (including teleport).
    state_bits_dirty_epoch: u64,
    /// Last Work-generation that touched durable attached/enabled masks
    /// (SetAttached/SetEnabled **and** Allocate/Free reset clears).
    masks_dirty_epoch: u64,
    /// Durable parent slot (`PARENT_ABSENT` = none). Not face-rotated.
    parent_slot: Vec<u32>,
    /// Resonite-style attached mask (component presence). Not face-rotated.
    attached_masks: Vec<u64>,
    /// Resonite-style enabled mask (60Hz execution). Not face-rotated.
    enabled_masks: Vec<u64>,
    scale_x: Vec<f32>,
    scale_y: Vec<f32>,
    scale_z: Vec<f32>,
    box_half_x: Vec<f32>,
    box_half_y: Vec<f32>,
    box_half_z: Vec<f32>,
    box_corner_radius: Vec<f32>,
}

impl SimArena {
    pub fn with_slots(slot_count: usize) -> Self {
        Self {
            faces: [
                SimFace::with_slots(slot_count),
                SimFace::with_slots(slot_count),
                SimFace::with_slots(slot_count),
            ],
            prev_i: 0,
            published_i: 1,
            work_i: 2,
            epoch: 0,
            this_tick_dirty: Vec::new(),
            this_tick_marked: vec![false; slot_count],
            carry_seen: vec![false; slot_count],
            prev_dirty_slots: Vec::new(),
            prev2_dirty_slots: Vec::new(),
            transform_dirty_epoch: 0,
            state_bits_dirty_epoch: 0,
            masks_dirty_epoch: 0,
            parent_slot: vec![PARENT_ABSENT; slot_count],
            attached_masks: vec![0; slot_count],
            enabled_masks: vec![0; slot_count],
            scale_x: vec![1.0; slot_count],
            scale_y: vec![1.0; slot_count],
            scale_z: vec![1.0; slot_count],
            box_half_x: vec![0.0; slot_count],
            box_half_y: vec![0.0; slot_count],
            box_half_z: vec![0.0; slot_count],
            box_corner_radius: vec![0.0; slot_count],
        }
    }

    /// Logical slots per face (equal across columns). Not `Vec::capacity`.
    pub fn slot_count(&self) -> usize {
        self.faces[0].slot_count()
    }

    pub fn resize(&mut self, slot_count: usize) {
        for face in &mut self.faces {
            face.resize(slot_count);
        }
        self.this_tick_marked.resize(slot_count, false);
        self.carry_seen.resize(slot_count, false);
        self.parent_slot.resize(slot_count, PARENT_ABSENT);
        self.attached_masks.resize(slot_count, 0);
        self.enabled_masks.resize(slot_count, 0);
        self.scale_x.resize(slot_count, 1.0);
        self.scale_y.resize(slot_count, 1.0);
        self.scale_z.resize(slot_count, 1.0);
        self.box_half_x.resize(slot_count, 0.0);
        self.box_half_y.resize(slot_count, 0.0);
        self.box_half_z.resize(slot_count, 0.0);
        self.box_corner_radius.resize(slot_count, 0.0);
        self.this_tick_dirty.retain(|&s| (s as usize) < slot_count);
        self.prev_dirty_slots.retain(|&s| (s as usize) < slot_count);
        self.prev2_dirty_slots
            .retain(|&s| (s as usize) < slot_count);
        self.this_tick_marked.fill(false);
        for &s in &self.this_tick_dirty {
            self.this_tick_marked[s as usize] = true;
        }
    }

    /// Monotonic face-generation counter (advanced only by [`Self::rotate_faces`]).
    #[inline]
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Work generation being built (`epoch + 1` until the next rotate).
    #[inline]
    pub fn work_epoch(&self) -> u64 {
        self.epoch.wrapping_add(1)
    }

    #[inline]
    pub fn transform_dirty_epoch(&self) -> u64 {
        self.transform_dirty_epoch
    }

    #[inline]
    pub fn state_bits_dirty_epoch(&self) -> u64 {
        self.state_bits_dirty_epoch
    }

    /// Last Work-generation that touched durable attached/enabled masks.
    /// Distinct from [`Self::state_bits_dirty_epoch`] (face TELEPORT / state_bits).
    #[inline]
    pub fn masks_dirty_epoch(&self) -> u64 {
        self.masks_dirty_epoch
    }

    #[inline]
    pub fn prev_dirty_slots(&self) -> &[LocalSlotId] {
        &self.prev_dirty_slots
    }

    #[inline]
    pub fn prev2_dirty_slots(&self) -> &[LocalSlotId] {
        &self.prev2_dirty_slots
    }

    #[inline]
    pub fn this_tick_dirty(&self) -> &[LocalSlotId] {
        &self.this_tick_dirty
    }

    #[inline]
    pub fn face_index(&self, role: FaceRole) -> usize {
        match role {
            FaceRole::Prev => self.prev_i as usize,
            FaceRole::Published => self.published_i as usize,
            FaceRole::Work => self.work_i as usize,
        }
    }

    #[inline]
    pub fn prev(&self) -> &SimFace {
        &self.faces[self.prev_i as usize]
    }

    #[inline]
    pub fn published(&self) -> &SimFace {
        &self.faces[self.published_i as usize]
    }

    #[inline]
    pub fn work(&self) -> &SimFace {
        &self.faces[self.work_i as usize]
    }

    /// Escape hatch for bulk Work mutation (x00dc host path should use `write_*`).
    ///
    /// **Must** call [`Self::mark_dirty`] for every touched slot; otherwise the
    /// row misses Carry-Over **and** Published-leave TELEPORT clear (ancestor
    /// regression / sticky snap). Prefer [`Self::write_pos`] /
    /// [`Self::write_rot`] / [`Self::write_teleport`].
    #[inline]
    pub fn work_mut(&mut self) -> &mut SimFace {
        &mut self.faces[self.work_i as usize]
    }

    /// Bootstrap helper: copy the same pose onto **all three** faces (no dirty mark).
    ///
    /// Used once when wiring an existing Epoch sheet into the Arena so all faces
    /// start aligned with sheet poses. Not a tick-path writer — prefer
    /// [`Self::write_pos`] / [`Self::write_rot`] after seed.
    ///
    /// # Panics
    /// If `slot` is out of range (same contract as [`Self::mark_dirty`]).
    pub fn seed_slot(&mut self, slot: LocalSlotId, x: f32, y: f32, z: f32, rot: [f32; 4]) {
        let i = slot as usize;
        assert!(
            i < self.slot_count(),
            "LocalSlotId {slot} out of range (slot_count={})",
            self.slot_count()
        );
        for face in &mut self.faces {
            face.set_pos(slot, x, y, z);
            face.set_rot(slot, rot[0], rot[1], rot[2], rot[3]);
        }
    }

    /// Record `slot` as written this Work tick (idempotent per tick).
    ///
    /// Prefer [`Self::write_pos`] / [`Self::write_rot`] / [`Self::write_teleport`],
    /// which call this automatically. Raw [`Self::work_mut`] writers must call it.
    ///
    /// # Panics
    /// If `slot` is out of range (same contract as [`SimFace`] accessors).
    pub fn mark_dirty(&mut self, slot: LocalSlotId) {
        let i = slot as usize;
        assert!(
            i < self.slot_count(),
            "LocalSlotId {slot} out of range (slot_count={})",
            self.slot_count()
        );
        if self.this_tick_marked[i] {
            return;
        }
        self.this_tick_marked[i] = true;
        self.this_tick_dirty.push(slot);
    }

    /// Write Work position, mark dirty, bump [`Self::transform_dirty_epoch`].
    pub fn write_pos(&mut self, slot: LocalSlotId, x: f32, y: f32, z: f32) {
        self.work_mut().set_pos(slot, x, y, z);
        self.mark_dirty(slot);
        self.transform_dirty_epoch = self.work_epoch();
    }

    /// Write Work rotation, mark dirty, bump [`Self::transform_dirty_epoch`].
    pub fn write_rot(&mut self, slot: LocalSlotId, x: f32, y: f32, z: f32, w: f32) {
        self.work_mut().set_rot(slot, x, y, z, w);
        self.mark_dirty(slot);
        self.transform_dirty_epoch = self.work_epoch();
    }

    /// Write Work teleport bit, mark dirty, bump [`Self::state_bits_dirty_epoch`].
    pub fn write_teleport(&mut self, slot: LocalSlotId, on: bool) {
        self.work_mut().set_teleport(slot, on);
        self.mark_dirty(slot);
        self.state_bits_dirty_epoch = self.work_epoch();
    }

    /// Durable parent for `slot` (`None` = world root / free / absent).
    ///
    /// Work-update-side only — not co-published with face Published pose (x00df).
    #[inline]
    pub fn parent(&self, slot: LocalSlotId) -> Option<LocalSlotId> {
        let i = slot as usize;
        if i >= self.parent_slot.len() {
            return None;
        }
        let p = self.parent_slot[i];
        if p == PARENT_ABSENT {
            None
        } else {
            Some(p)
        }
    }

    /// Durable attached mask. Work-update-side only (see [`Self::parent`]).
    #[inline]
    pub fn attached_bits(&self, slot: LocalSlotId) -> u64 {
        let i = slot as usize;
        if i >= self.attached_masks.len() {
            0
        } else {
            self.attached_masks[i]
        }
    }

    /// Durable enabled mask. Work-update-side only (see [`Self::parent`]).
    #[inline]
    pub fn enabled_bits(&self, slot: LocalSlotId) -> u64 {
        let i = slot as usize;
        if i >= self.enabled_masks.len() {
            0
        } else {
            self.enabled_masks[i]
        }
    }

    /// Durable scale. Work-update-side only (see [`Self::parent`]).
    #[inline]
    pub fn scale(&self, slot: LocalSlotId) -> [f32; 3] {
        let i = slot as usize;
        if i >= self.scale_x.len() {
            [1.0, 1.0, 1.0]
        } else {
            [self.scale_x[i], self.scale_y[i], self.scale_z[i]]
        }
    }

    /// Durable procedural box params. Work-update-side only (see [`Self::parent`]).
    #[inline]
    pub fn procedural_box(&self, slot: LocalSlotId) -> ([f32; 3], f32) {
        let i = slot as usize;
        if i >= self.box_half_x.len() {
            ([0.0, 0.0, 0.0], 0.0)
        } else {
            (
                [self.box_half_x[i], self.box_half_y[i], self.box_half_z[i]],
                self.box_corner_radius[i],
            )
        }
    }

    /// Step 0 barrier: Carry-Over then drain `patches` onto Work (queue order).
    ///
    /// Pos / rot / scale / parent / masks / box: last write wins. Teleport bit:
    /// sticky OR (`SetLocalTransform { teleport: false }` does not clear).
    /// Prefer passing `&store.drain_patches()` (or any `&[SimPatch]`) — no second
    /// `Vec` collect inside Drain.
    pub fn step0_carry_and_apply_patches(&mut self, patches: &[SimPatch]) {
        self.carry_over();
        self.apply_patches(patches);
    }

    /// Drain SimPatch queue onto Work in iterator order (call after Carry-Over).
    ///
    /// Takes `&[SimPatch]` so a `SceneStore::drain_patches()` `Vec` can be passed
    /// without a second Tick-boundary allocation.
    ///
    /// - Pre-grows once to the max [`SimPatch::AllocateSlot`] index only
    ///   (Allocate-first contract from Scene Store). Free / Set* on a slot that
    ///   was never Allocated remain **no-ops** (high-id Set* alone does not grow).
    /// - [`SimPatch::AllocateSlot`] resets durable defaults + identity pose then
    ///   sets parent / TELEPORT (re-Allocate without Free must not keep stale
    ///   scale/masks/box — Free auto-enqueue arrives in x00e1).
    /// - `SetLocalTransform.teleport`: sticky OR (false does not clear).
    /// - `Some(parent)` with `parent == u32::MAX` coerces to root (illegal id).
    /// - Depth Bucket rebuild on SetParent is **not** performed (→ later slice).
    /// - Procedural-box has no dedicated column dirty_epoch yet (deferred).
    pub fn apply_patches(&mut self, patches: &[SimPatch]) {
        let mut max_need = self.slot_count();
        for patch in patches {
            if let SimPatch::AllocateSlot { slot, .. } = patch {
                max_need = max_need.max((*slot as usize).saturating_add(1));
            }
        }
        if max_need > self.slot_count() {
            self.resize(max_need);
        }
        for patch in patches {
            self.apply_one_patch(patch);
        }
    }

    fn apply_one_patch(&mut self, patch: &SimPatch) {
        match *patch {
            SimPatch::AllocateSlot { slot, parent } => {
                self.ensure_slot_capacity(slot);
                self.reset_slot_defaults(slot);
                self.set_parent_column(slot, parent);
                self.write_teleport(slot, true);
            }
            SimPatch::FreeSlot { slot } => {
                if !self.slot_in_range(slot) {
                    return;
                }
                self.reset_slot_defaults(slot);
                self.write_teleport(slot, false);
            }
            SimPatch::SetParent { slot, parent } => {
                if !self.slot_in_range(slot) {
                    return;
                }
                self.set_parent_column(slot, parent);
                // Hierarchy change → GPU snap (Depth Bucket rebuild deferred).
                self.write_teleport(slot, true);
            }
            SimPatch::SetLocalTransform {
                slot,
                pos,
                rot,
                scale,
                teleport,
            } => {
                if !self.slot_in_range(slot) {
                    return;
                }
                let i = slot as usize;
                self.scale_x[i] = scale[0];
                self.scale_y[i] = scale[1];
                self.scale_z[i] = scale[2];
                self.write_pos(slot, pos[0], pos[1], pos[2]);
                self.write_rot(slot, rot[0], rot[1], rot[2], rot[3]);
                // Sticky OR: false leaves an earlier Allocate/SetParent/true bit.
                if teleport {
                    self.write_teleport(slot, true);
                }
            }
            SimPatch::SetAttachedBits { slot, add, remove } => {
                if !self.slot_in_range(slot) {
                    return;
                }
                let i = slot as usize;
                self.attached_masks[i] = (self.attached_masks[i] | add) & !remove;
                self.mark_dirty(slot);
                self.masks_dirty_epoch = self.work_epoch();
            }
            SimPatch::SetEnabledBits { slot, add, remove } => {
                if !self.slot_in_range(slot) {
                    return;
                }
                let i = slot as usize;
                self.enabled_masks[i] = (self.enabled_masks[i] | add) & !remove;
                self.mark_dirty(slot);
                self.masks_dirty_epoch = self.work_epoch();
            }
            SimPatch::SetProceduralBox {
                slot,
                half_extents,
                corner_radius,
            } => {
                if !self.slot_in_range(slot) {
                    return;
                }
                let i = slot as usize;
                self.box_half_x[i] = half_extents[0];
                self.box_half_y[i] = half_extents[1];
                self.box_half_z[i] = half_extents[2];
                self.box_corner_radius[i] = corner_radius;
                // No dedicated box dirty_epoch yet (roadmap §4.1 low-freq defer).
                self.mark_dirty(slot);
            }
        }
    }

    #[inline]
    fn slot_in_range(&self, slot: LocalSlotId) -> bool {
        (slot as usize) < self.slot_count()
    }

    fn ensure_slot_capacity(&mut self, slot: LocalSlotId) {
        let need = (slot as usize).saturating_add(1);
        if need > self.slot_count() {
            self.resize(need);
        }
    }

    /// Reset durable columns + identity Work pose (Allocate / Free shared path).
    ///
    /// Clears attached/enabled and bumps [`Self::masks_dirty_epoch`] so epoch
    /// skippers cannot miss a Free / re-Allocate mask clear (not Set*-only).
    fn reset_slot_defaults(&mut self, slot: LocalSlotId) {
        let i = slot as usize;
        debug_assert!(i < self.parent_slot.len());
        self.parent_slot[i] = PARENT_ABSENT;
        self.attached_masks[i] = 0;
        self.enabled_masks[i] = 0;
        self.masks_dirty_epoch = self.work_epoch();
        self.scale_x[i] = 1.0;
        self.scale_y[i] = 1.0;
        self.scale_z[i] = 1.0;
        self.box_half_x[i] = 0.0;
        self.box_half_y[i] = 0.0;
        self.box_half_z[i] = 0.0;
        self.box_corner_radius[i] = 0.0;
        self.write_pos(slot, 0.0, 0.0, 0.0);
        self.write_rot(slot, 0.0, 0.0, 0.0, 1.0);
    }

    /// `Some(u32::MAX)` is illegal (sentinel = absent) and coerces to root.
    fn set_parent_column(&mut self, slot: LocalSlotId, parent: Option<LocalSlotId>) {
        let i = slot as usize;
        debug_assert!(i < self.parent_slot.len());
        self.parent_slot[i] = match parent {
            Some(p) if p != PARENT_ABSENT => p,
            _ => PARENT_ABSENT,
        };
    }

    #[cfg(test)]
    fn durable_columns_aligned(&self) -> bool {
        let n = self.slot_count();
        self.parent_slot.len() == n
            && self.attached_masks.len() == n
            && self.enabled_masks.len() == n
            && self.scale_x.len() == n
            && self.scale_y.len() == n
            && self.scale_z.len() == n
            && self.box_half_x.len() == n
            && self.box_half_y.len() == n
            && self.box_half_z.len() == n
            && self.box_corner_radius.len() == n
            && self.this_tick_marked.len() == n
            && self.carry_seen.len() == n
    }

    /// Step 0: 2-frame dirty Carry-Over from Published → Work, then clear
    /// `STATE_TELEPORT_THIS_TICK` on those Work rows.
    ///
    /// Copies **pos + rot + state_bits** once per slot in
    /// `prev_dirty_slots ∪ prev2_dirty_slots` (deduped via [`Self::carry_seen`]).
    /// Stationary slots outside those lists are not touched (no full-face copy).
    pub fn carry_over(&mut self) {
        let n_prev = self.prev_dirty_slots.len();
        let n_prev2 = self.prev2_dirty_slots.len();
        for idx in 0..n_prev2 {
            let slot = self.prev2_dirty_slots[idx];
            self.carry_slot_once(slot);
        }
        for idx in 0..n_prev {
            let slot = self.prev_dirty_slots[idx];
            self.carry_slot_once(slot);
        }
        // Clear scratch marks for slots we visited (avoid O(n) full reset).
        for idx in 0..n_prev2 {
            let i = self.prev2_dirty_slots[idx] as usize;
            if i < self.carry_seen.len() {
                self.carry_seen[i] = false;
            }
        }
        for idx in 0..n_prev {
            let i = self.prev_dirty_slots[idx] as usize;
            if i < self.carry_seen.len() {
                self.carry_seen[i] = false;
            }
        }
    }

    fn carry_slot_once(&mut self, slot: LocalSlotId) {
        let i = slot as usize;
        debug_assert!(
            i < self.carry_seen.len(),
            "LocalSlotId {slot} out of range (slot_count={})",
            self.slot_count()
        );
        if self.carry_seen[i] {
            return;
        }
        self.carry_seen[i] = true;
        self.copy_published_row_to_work_clear_teleport(slot);
    }

    fn copy_published_row_to_work_clear_teleport(&mut self, slot: LocalSlotId) {
        let i = slot as usize;
        let pub_i = self.published_i as usize;
        let work_i = self.work_i as usize;
        debug_assert_ne!(pub_i, work_i);
        debug_assert!(
            i < self.slot_count(),
            "LocalSlotId {slot} out of range (slot_count={})",
            self.slot_count()
        );

        let pos_x = self.faces[pub_i].pos_x[i];
        let pos_y = self.faces[pub_i].pos_y[i];
        let pos_z = self.faces[pub_i].pos_z[i];
        let rot_x = self.faces[pub_i].rot_x[i];
        let rot_y = self.faces[pub_i].rot_y[i];
        let rot_z = self.faces[pub_i].rot_z[i];
        let rot_w = self.faces[pub_i].rot_w[i];
        let bits = self.faces[pub_i].state_bits[i] & !STATE_TELEPORT_THIS_TICK;

        self.faces[work_i].pos_x[i] = pos_x;
        self.faces[work_i].pos_y[i] = pos_y;
        self.faces[work_i].pos_z[i] = pos_z;
        self.faces[work_i].rot_x[i] = rot_x;
        self.faces[work_i].rot_y[i] = rot_y;
        self.faces[work_i].rot_z[i] = rot_z;
        self.faces[work_i].rot_w[i] = rot_w;
        self.faces[work_i].state_bits[i] = bits;
    }

    /// End-of-tick role rotation: Work→Published, Published→Prev, Prev→Work.
    ///
    /// Shifts dirty rings: `prev2 ← prev ← this_tick`, then clears this-tick marks.
    /// Does **not** copy rows (use [`Self::carry_over`] at the next tick start).
    ///
    /// Clears [`STATE_TELEPORT_THIS_TICK`] on **dirty rows** of the face leaving
    /// Published (`prev_dirty_slots` = the write set that built that Published;
    /// same sparse window as pose Carry-Over — not a full `state_bits` scan).
    /// Work→Published keeps the bit. Work-row clear after pose copy remains
    /// [`Self::carry_over`]'s job.
    ///
    /// # x00dc publish ordering
    ///
    /// This clear mutates the leaving Published face **in place** as it becomes
    /// Prev. Ring3 must observe a snapshot taken **before** [`Self::rotate_faces`]
    /// (publish-then-rotate). Holding the same face buffer across rotate lets
    /// TELEPORT disappear under the reader.
    pub fn rotate_faces(&mut self) {
        // End TELEPORT lifetime on rows that could have been set when this
        // Published was built (`prev_dirty_slots` before the dirty-ring shift).
        let leaving_published = self.published_i as usize;
        for &slot in &self.prev_dirty_slots {
            let i = slot as usize;
            debug_assert!(
                i < self.faces[leaving_published].state_bits.len(),
                "LocalSlotId {slot} out of range (slot_count={})",
                self.slot_count()
            );
            self.faces[leaving_published].state_bits[i] &= !STATE_TELEPORT_THIS_TICK;
        }

        let old_prev = self.prev_i;
        self.prev_i = self.published_i;
        self.published_i = self.work_i;
        self.work_i = old_prev;
        self.epoch = self.epoch.wrapping_add(1);

        for &slot in &self.this_tick_dirty {
            let i = slot as usize;
            if i < self.this_tick_marked.len() {
                self.this_tick_marked[i] = false;
            }
        }
        self.prev2_dirty_slots = std::mem::take(&mut self.prev_dirty_slots);
        self.prev_dirty_slots = std::mem::take(&mut self.this_tick_dirty);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_arena_has_distinct_face_roles() {
        let arena = SimArena::with_slots(4);
        assert_ne!(
            arena.face_index(FaceRole::Prev),
            arena.face_index(FaceRole::Published)
        );
        assert_ne!(
            arena.face_index(FaceRole::Published),
            arena.face_index(FaceRole::Work)
        );
        assert_ne!(
            arena.face_index(FaceRole::Prev),
            arena.face_index(FaceRole::Work)
        );
        assert_eq!(arena.epoch(), 0);
        assert_eq!(arena.slot_count(), 4);
    }

    #[test]
    fn rotate_advances_roles_and_epoch() {
        let mut arena = SimArena::with_slots(2);
        let i_prev = arena.face_index(FaceRole::Prev);
        let i_pub = arena.face_index(FaceRole::Published);
        let i_work = arena.face_index(FaceRole::Work);

        arena.rotate_faces();

        assert_eq!(arena.face_index(FaceRole::Prev), i_pub);
        assert_eq!(arena.face_index(FaceRole::Published), i_work);
        assert_eq!(arena.face_index(FaceRole::Work), i_prev);
        assert_eq!(arena.epoch(), 1);
    }

    #[test]
    fn work_write_becomes_published_after_rotate() {
        let mut arena = SimArena::with_slots(2);
        arena.write_pos(0, 1.0, 2.0, 3.0);
        arena.write_teleport(0, true);

        assert_eq!(arena.published().pos(0), [0.0, 0.0, 0.0]);
        assert!(!arena.published().is_teleport(0));

        arena.rotate_faces();

        assert_eq!(arena.published().pos(0), [1.0, 2.0, 3.0]);
        assert!(arena.published().is_teleport(0));
        assert_eq!(arena.prev().pos(0), [0.0, 0.0, 0.0]);
        assert_eq!(arena.prev_dirty_slots(), &[0]);
    }

    #[test]
    fn double_rotate_slides_published_to_prev() {
        let mut arena = SimArena::with_slots(1);
        arena.write_pos(0, 10.0, 0.0, 0.0);
        arena.rotate_faces();
        arena.carry_over();
        arena.write_pos(0, 20.0, 0.0, 0.0);
        arena.rotate_faces();

        assert_eq!(arena.prev().pos(0), [10.0, 0.0, 0.0]);
        assert_eq!(arena.published().pos(0), [20.0, 0.0, 0.0]);
    }

    #[test]
    fn teleport_bit_matches_roadmap_constant() {
        assert_eq!(STATE_TELEPORT_THIS_TICK, 1 << 11);
    }

    #[test]
    fn resize_keeps_all_columns_aligned() {
        let mut arena = SimArena::with_slots(2);
        arena.resize(8);
        assert_eq!(arena.slot_count(), 8);
        assert!(arena.prev().columns_aligned());
        assert!(arena.published().columns_aligned());
        assert!(arena.work().columns_aligned());
        assert!(arena.durable_columns_aligned());
        arena.resize(3);
        assert_eq!(arena.slot_count(), 3);
        assert!(arena.work().columns_aligned());
        assert!(arena.durable_columns_aligned());
    }

    #[test]
    fn regression_substrate_recycled_work_keeps_stale_pose_if_carry_over_skipped() {
        // Negative substrate (not the happy path): skipping Carry-Over lets the
        // recycled Work face keep a pose from two publishes ago. Production ticks
        // must call carry_over before writers; see ancestor-regression tests.
        let mut arena = SimArena::with_slots(1);
        arena.write_pos(0, 1.0, 0.0, 0.0);
        arena.rotate_faces();
        arena.write_pos(0, 2.0, 0.0, 0.0);
        arena.rotate_faces();
        arena.rotate_faces();

        assert_eq!(arena.prev().pos(0), [2.0, 0.0, 0.0]);
        assert_eq!(
            arena.work().pos(0),
            [1.0, 0.0, 0.0],
            "without Carry-Over, recycled Work keeps stale pose (why Step 0 exists)"
        );
    }

    #[test]
    fn teleport_survives_into_published_then_clears_on_published_leave() {
        let mut arena = SimArena::with_slots(1);
        arena.write_teleport(0, true);
        arena.rotate_faces();
        assert!(
            arena.published().is_teleport(0),
            "TELEPORT must survive Work→Published for one Ring3 epoch"
        );

        // Next rotate ends that Published epoch — bit must not linger on Prev.
        arena.rotate_faces();
        assert!(
            !arena.prev().is_teleport(0),
            "Published→Prev must clear TELEPORT (lifetime end)"
        );
    }

    #[test]
    fn carry_over_prevents_ancestor_regression_after_three_idle_ticks() {
        // Move → stop → 3 idle ticks: Published must not snap back to an older pose.
        let mut arena = SimArena::with_slots(2);
        let slot = 0u32;
        let idle = 1u32; // never written — proves we do not full-copy the face

        arena.carry_over();
        arena.write_pos(slot, 10.0, 0.0, 0.0);
        arena.write_rot(slot, 0.0, 0.0, 0.0, 1.0);
        arena.rotate_faces();

        arena.carry_over();
        arena.write_pos(slot, 20.0, 1.0, 0.0);
        arena.write_rot(slot, 0.0, 1.0, 0.0, 0.0);
        arena.rotate_faces();

        assert_eq!(arena.published().pos(slot), [20.0, 1.0, 0.0]);
        assert_eq!(arena.published().rot(slot), [0.0, 1.0, 0.0, 0.0]);

        for tick in 0..3 {
            let before_idle = arena.work().pos(idle);
            arena.carry_over();
            assert_eq!(
                arena.work().pos(idle),
                before_idle,
                "idle slot must not be touched by Carry-Over (tick {tick})"
            );
            // Stationary: no write_pos / mark_dirty.
            arena.rotate_faces();
            assert_eq!(
                arena.published().pos(slot),
                [20.0, 1.0, 0.0],
                "ancestor regression on idle tick {tick}"
            );
            assert_eq!(
                arena.published().rot(slot),
                [0.0, 1.0, 0.0, 0.0],
                "rot ancestor regression on idle tick {tick}"
            );
        }
    }

    #[test]
    fn carry_over_clears_teleport_on_work_keeps_published() {
        let mut arena = SimArena::with_slots(1);
        arena.carry_over();
        arena.write_pos(0, 5.0, 0.0, 0.0);
        arena.write_teleport(0, true);
        arena.rotate_faces();

        assert!(arena.published().is_teleport(0));

        arena.carry_over();

        assert!(
            arena.published().is_teleport(0),
            "Published keeps TELEPORT for Ring3 until the next rotate"
        );
        assert!(
            !arena.work().is_teleport(0),
            "Carry-Over must clear TELEPORT on Work after the row copy"
        );
        assert_eq!(arena.work().pos(0), [5.0, 0.0, 0.0]);
    }

    #[test]
    fn teleport_does_not_resurrect_on_published_after_three_idle_ticks() {
        // Review finding: dirty window is only 2 frames; without Published-leave
        // clear, the TP-bearing face recycles to Work outside that window and
        // republishes TELEPORT on idle rotate #3.
        let mut arena = SimArena::with_slots(1);
        arena.carry_over();
        arena.write_pos(0, 1.0, 0.0, 0.0);
        arena.write_teleport(0, true);
        arena.rotate_faces();
        assert!(arena.published().is_teleport(0));

        for tick in 0..3 {
            arena.carry_over();
            assert!(
                !arena.work().is_teleport(0),
                "Work TELEPORT must stay clear on idle tick {tick} (no writer)"
            );
            arena.rotate_faces();
            assert!(
                !arena.published().is_teleport(0),
                "TELEPORT must not resurrect on Published after idle tick {tick}"
            );
        }
    }

    #[test]
    fn work_mut_without_mark_dirty_is_not_carried() {
        let mut arena = SimArena::with_slots(1);
        arena.work_mut().set_pos(0, 9.0, 0.0, 0.0);
        // Deliberately skip mark_dirty — not the supported write path.
        arena.rotate_faces();
        assert!(
            arena.prev_dirty_slots().is_empty(),
            "raw work_mut without mark_dirty must not enter the dirty ring"
        );
        arena.carry_over();
        arena.rotate_faces();
        // Pose was published once via rotate, but is not in the carry window;
        // this documents the footgun rather than the happy path.
        assert!(arena.prev_dirty_slots().is_empty());
        assert!(arena.prev2_dirty_slots().is_empty());
    }

    #[test]
    fn resize_shrink_drops_out_of_range_dirty_slots() {
        let mut arena = SimArena::with_slots(4);
        arena.write_pos(1, 1.0, 0.0, 0.0);
        arena.write_pos(3, 3.0, 0.0, 0.0);
        arena.resize(2);
        assert_eq!(arena.this_tick_dirty(), &[1]);
        arena.rotate_faces();
        assert_eq!(arena.prev_dirty_slots(), &[1]);
        assert!(arena.prev_dirty_slots().iter().all(|&s| (s as usize) < 2));
    }

    #[test]
    fn column_dirty_epoch_tracks_transform_and_state_bits() {
        let mut arena = SimArena::with_slots(1);
        assert_eq!(arena.transform_dirty_epoch(), 0);
        assert_eq!(arena.state_bits_dirty_epoch(), 0);

        arena.write_pos(0, 1.0, 0.0, 0.0);
        assert_eq!(arena.transform_dirty_epoch(), arena.work_epoch());
        let transform_at = arena.transform_dirty_epoch();

        arena.write_teleport(0, true);
        assert_eq!(arena.state_bits_dirty_epoch(), arena.work_epoch());
        assert_eq!(
            arena.transform_dirty_epoch(),
            transform_at,
            "state_bits write must not bump transform_dirty_epoch"
        );
    }

    #[test]
    fn mark_dirty_is_idempotent_within_tick() {
        let mut arena = SimArena::with_slots(1);
        arena.mark_dirty(0);
        arena.mark_dirty(0);
        assert_eq!(arena.this_tick_dirty(), &[0]);
        arena.rotate_faces();
        assert_eq!(arena.prev_dirty_slots(), &[0]);
        assert!(arena.this_tick_dirty().is_empty());
    }

    #[test]
    fn consecutive_patches_same_slot_last_write_wins() {
        // Lost Update barrier: pos/rot/scale last-write-wins; teleport sticky OR.
        let mut arena = SimArena::with_slots(2);
        let slot = 0u32;
        let patches = [
            SimPatch::AllocateSlot { slot, parent: None },
            SimPatch::SetLocalTransform {
                slot,
                pos: [1.0, 0.0, 0.0],
                rot: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
                teleport: true,
            },
            SimPatch::SetLocalTransform {
                slot,
                pos: [2.0, 3.0, 4.0],
                rot: [0.0, 1.0, 0.0, 0.0],
                scale: [2.0, 2.0, 2.0],
                teleport: false, // must NOT clear TELEPORT from earlier patches
            },
            SimPatch::SetLocalTransform {
                slot,
                pos: [9.0, 8.0, 7.0],
                rot: [0.0, 0.0, 1.0, 0.0],
                scale: [3.0, 3.0, 3.0],
                teleport: true,
            },
        ];

        arena.step0_carry_and_apply_patches(&patches);

        assert_eq!(arena.work().pos(slot), [9.0, 8.0, 7.0]);
        assert_eq!(arena.work().rot(slot), [0.0, 0.0, 1.0, 0.0]);
        assert_eq!(arena.scale(slot), [3.0, 3.0, 3.0]);
        assert!(
            arena.work().is_teleport(slot),
            "teleport sticky OR: false mid-queue must not clear"
        );
        assert_eq!(arena.parent(slot), None);
    }

    #[test]
    fn set_local_transform_teleport_false_is_sticky_or_after_allocate() {
        let mut arena = SimArena::with_slots(1);
        arena.step0_carry_and_apply_patches(&[
            SimPatch::AllocateSlot {
                slot: 0,
                parent: None,
            },
            SimPatch::SetLocalTransform {
                slot: 0,
                pos: [1.0, 2.0, 3.0],
                rot: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
                teleport: false,
            },
        ]);
        assert_eq!(arena.work().pos(0), [1.0, 2.0, 3.0]);
        assert!(
            arena.work().is_teleport(0),
            "Allocate TELEPORT must survive later SetLocalTransform {{ teleport: false }}"
        );
    }

    #[test]
    fn parent_some_absent_sentinel_coerces_to_root() {
        let mut arena = SimArena::with_slots(2);
        arena.step0_carry_and_apply_patches(&[
            SimPatch::AllocateSlot {
                slot: 0,
                parent: None,
            },
            SimPatch::AllocateSlot {
                slot: 1,
                parent: Some(PARENT_ABSENT),
            },
            SimPatch::SetParent {
                slot: 0,
                parent: Some(PARENT_ABSENT),
            },
        ]);
        assert_eq!(arena.parent(1), None);
        assert_eq!(arena.parent(0), None);
    }

    #[test]
    fn reallocate_resets_stale_durable_defaults() {
        let mut arena = SimArena::with_slots(1);
        arena.step0_carry_and_apply_patches(&[
            SimPatch::AllocateSlot {
                slot: 0,
                parent: None,
            },
            SimPatch::SetAttachedBits {
                slot: 0,
                add: 0b1111,
                remove: 0,
            },
            SimPatch::SetLocalTransform {
                slot: 0,
                pos: [9.0, 9.0, 9.0],
                rot: [0.0, 1.0, 0.0, 0.0],
                scale: [4.0, 4.0, 4.0],
                teleport: false,
            },
            SimPatch::SetProceduralBox {
                slot: 0,
                half_extents: [1.0, 1.0, 1.0],
                corner_radius: 0.2,
            },
        ]);
        // Advance Work generation so reset's masks bump cannot hide behind the
        // same-tick SetAttachedBits epoch (regression if reset skips bump).
        arena.rotate_faces();
        arena.carry_over();
        let masks_before = arena.masks_dirty_epoch();
        let work_before = arena.work_epoch();
        // Re-Allocate without Free (x00e1 Free auto-enqueue not yet wired).
        arena.apply_patches(&[SimPatch::AllocateSlot {
            slot: 0,
            parent: None,
        }]);
        assert_eq!(arena.attached_bits(0), 0);
        assert_eq!(arena.scale(0), [1.0, 1.0, 1.0]);
        assert_eq!(arena.procedural_box(0), ([0.0, 0.0, 0.0], 0.0));
        assert_eq!(arena.work().pos(0), [0.0, 0.0, 0.0]);
        assert!(arena.work().is_teleport(0));
        assert_eq!(arena.work_epoch(), work_before);
        assert!(
            arena.masks_dirty_epoch() > masks_before,
            "re-Allocate mask clear must advance masks_dirty_epoch after rotate"
        );
        assert_eq!(arena.masks_dirty_epoch(), arena.work_epoch());
    }

    #[test]
    fn mask_patches_bump_masks_epoch_not_state_bits_epoch() {
        let mut arena = SimArena::with_slots(1);
        arena.write_teleport(0, true);
        let state_at = arena.state_bits_dirty_epoch();
        arena.apply_patches(&[SimPatch::SetAttachedBits {
            slot: 0,
            add: 1,
            remove: 0,
        }]);
        assert_eq!(arena.masks_dirty_epoch(), arena.work_epoch());
        assert_eq!(
            arena.state_bits_dirty_epoch(),
            state_at,
            "attached/enabled must not bump face state_bits_dirty_epoch"
        );
    }

    #[test]
    fn free_slot_bumps_masks_dirty_epoch() {
        let mut arena = SimArena::with_slots(1);
        arena.step0_carry_and_apply_patches(&[
            SimPatch::AllocateSlot {
                slot: 0,
                parent: None,
            },
            SimPatch::SetAttachedBits {
                slot: 0,
                add: 0b1,
                remove: 0,
            },
        ]);
        // Same-tick SetAttachedBits already at work_epoch — rotate so Free's
        // reset bump is observable as a strict advance.
        arena.rotate_faces();
        arena.carry_over();
        let masks_before = arena.masks_dirty_epoch();
        arena.apply_patches(&[SimPatch::FreeSlot { slot: 0 }]);
        assert_eq!(arena.attached_bits(0), 0);
        assert!(
            arena.masks_dirty_epoch() > masks_before,
            "Free mask clear must advance masks_dirty_epoch after rotate"
        );
        assert_eq!(arena.masks_dirty_epoch(), arena.work_epoch());
    }

    #[test]
    fn set_without_prior_allocate_is_noop_no_grow() {
        // Allocate-first contract: high-id Set* alone does not grow capacity.
        let mut arena = SimArena::with_slots(1);
        arena.apply_patches(&[SimPatch::SetLocalTransform {
            slot: 9,
            pos: [1.0, 0.0, 0.0],
            rot: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
            teleport: true,
        }]);
        assert_eq!(arena.slot_count(), 1);
    }

    #[test]
    fn patch_drain_allocate_grows_and_set_parent_sets_teleport() {
        let mut arena = SimArena::with_slots(1);
        assert_eq!(arena.slot_count(), 1);

        arena.step0_carry_and_apply_patches(&[
            SimPatch::AllocateSlot {
                slot: 0,
                parent: None,
            },
            SimPatch::AllocateSlot {
                slot: 5,
                parent: Some(0),
            },
            SimPatch::SetParent {
                slot: 5,
                parent: None,
            },
        ]);

        assert!(arena.slot_count() >= 6);
        assert_eq!(arena.parent(5), None);
        assert!(arena.work().is_teleport(5));
    }

    #[test]
    fn patch_bits_and_box_and_free_slot() {
        let mut arena = SimArena::with_slots(1);
        arena.step0_carry_and_apply_patches(&[
            SimPatch::AllocateSlot {
                slot: 0,
                parent: None,
            },
            SimPatch::SetAttachedBits {
                slot: 0,
                add: 0b1010,
                remove: 0,
            },
            SimPatch::SetEnabledBits {
                slot: 0,
                add: 0b0100,
                remove: 0,
            },
            SimPatch::SetAttachedBits {
                slot: 0,
                add: 0b0001,
                remove: 0b1000,
            },
            SimPatch::SetProceduralBox {
                slot: 0,
                half_extents: [0.5, 0.25, 0.125],
                corner_radius: 0.05,
            },
        ]);
        assert_eq!(arena.attached_bits(0), 0b0011);
        assert_eq!(arena.enabled_bits(0), 0b0100);
        assert_eq!(arena.procedural_box(0), ([0.5, 0.25, 0.125], 0.05));

        arena.apply_patches(&[SimPatch::FreeSlot { slot: 0 }]);
        assert_eq!(arena.parent(0), None);
        assert_eq!(arena.attached_bits(0), 0);
        assert_eq!(arena.enabled_bits(0), 0);
        assert_eq!(arena.work().pos(0), [0.0, 0.0, 0.0]);
        assert!(!arena.work().is_teleport(0));
    }

    #[test]
    fn scene_store_drain_into_arena_preserves_carry_over_contract() {
        use crate::scene_store::{CrdtOp, OpTimestamp, SceneStore};

        let mut store = SceneStore::new();
        let mut arena = SimArena::with_slots(4);

        // Establish a Published pose via normal tick writers.
        arena.carry_over();
        arena.write_pos(1, 10.0, 0.0, 0.0);
        arena.rotate_faces();

        assert_eq!(
            store.apply(CrdtOp::SpawnNode {
                ts: OpTimestamp::new(1, 1),
                node: 1,
                parent: None,
            }),
            crate::scene_store::ApplyOutcome::Applied
        );
        store.enqueue_patch(SimPatch::SetLocalTransform {
            slot: 1,
            pos: [42.0, 1.0, 2.0],
            rot: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
            teleport: true,
        });

        let patches = store.drain_patches();
        arena.step0_carry_and_apply_patches(&patches);

        // Carry-Over brought Published [10,0,0] then patches overwrote to final.
        assert_eq!(arena.work().pos(1), [42.0, 1.0, 2.0]);
        assert!(arena.work().is_teleport(1));
        assert!(store.pending_patches().is_empty());
    }
}
