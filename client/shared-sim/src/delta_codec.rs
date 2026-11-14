//! Dirty-delta codec for wire publish (x00ae / mvcc_xpbd §4.5).
//!
//! Guarantees: OR-accumulate dirty across skipped net ticks; page_slot key contract A
//! (`clear_slot` on alloc/reuse); capacity must match the MVCC **pool** size
//! ([`WireDirtyAccumulator::from_arena_capacity`]); Smallest-Three pack with
//! `q ≡ -q` pose identity; `extract_dirty_records_into` reuses `out`.
//! Pose deltas only cover **active** locals (`acc & active`); quarantine /
//! despawn / structure edits stay on §4.7 `…/edit` + x0024 snapshot — not this lane.
//! Non-guarantees: PROTOCOL_PIN quantized pose bump; host wiring is x00b1 (`host-server`);
//! `…/sim_state`; K>1 redundancy window (Q6); Spatial Interest.

use crate::mvcc::{HotPhysicsPage, MvccPageArena, BITMASK_WORDS, ENTITIES_PER_PAGE};

/// One entity pose delta for a publish (entity_id is authoritative on the wire).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityDeltaRecord {
    pub entity_id: u32,
    pub local_hint: u16,
    pub pos_xyz: [f32; 3],
    /// Smallest-Three 32-bit quaternion.
    pub packed_quat: u32,
}

/// Out-of-range page pool slot for absorb / clear (contract A keys = `PageId.0`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirtyAccumError {
    SlotOutOfRange { page_slot: usize, capacity: usize },
}

/// OR-accumulator keyed by page **pool** index (`PageId.0`; contract A: clear on alloc/reuse).
///
/// Capacity must equal [`crate::mvcc::MvccPageArena`] pool size (often
/// `HISTORY_TICKS + N`, not [`crate::mvcc::MAX_PAGES_PER_SNAPSHOT`]). Snapshot page
/// count is a per-commit bound; pool slots can be `>= 64`.
pub struct WireDirtyAccumulator {
    acc: Box<[[u64; BITMASK_WORDS]]>,
}

impl WireDirtyAccumulator {
    /// Allocate masks for `page_slot_capacity` pool slots (tests / low-level).
    ///
    /// Host wiring must not invent sizes like `64` (= snapshot page cap). Use
    /// [`Self::for_arena`] so capacity tracks [`MvccPageArena::capacity`].
    pub fn new(page_slot_capacity: usize) -> Self {
        let cap = page_slot_capacity.max(1);
        let acc = vec![[0u64; BITMASK_WORDS]; cap].into_boxed_slice();
        Self { acc }
    }

    /// Preferred host constructor: capacity is taken from the live arena pool.
    #[inline]
    pub fn for_arena(arena: &MvccPageArena) -> Self {
        Self::from_arena_capacity(arena.capacity())
    }

    /// Capacity = pool length (same domain as `PageId.0`). Prefer [`Self::for_arena`].
    #[inline]
    pub fn from_arena_capacity(pool_capacity_pages: usize) -> Self {
        Self::new(pool_capacity_pages)
    }

    #[inline]
    pub fn slot_capacity(&self) -> usize {
        self.acc.len()
    }

    /// OR `page.dirty_entity_mask` into the accumulator for `page_slot`.
    pub fn absorb(
        &mut self,
        page_slot: usize,
        page: &HotPhysicsPage,
    ) -> Result<(), DirtyAccumError> {
        let capacity = self.acc.len();
        let Some(slot) = self.acc.get_mut(page_slot) else {
            return Err(DirtyAccumError::SlotOutOfRange {
                page_slot,
                capacity,
            });
        };
        for (dst, src) in slot.iter_mut().zip(page.dirty_entity_mask.iter()) {
            *dst |= *src;
        }
        Ok(())
    }

    /// Clear one page slot (required on alloc / pool reuse — contract A).
    pub fn clear_slot(&mut self, page_slot: usize) -> Result<(), DirtyAccumError> {
        let capacity = self.acc.len();
        let Some(slot) = self.acc.get_mut(page_slot) else {
            return Err(DirtyAccumError::SlotOutOfRange {
                page_slot,
                capacity,
            });
        };
        *slot = [0u64; BITMASK_WORDS];
        Ok(())
    }

    /// Clear all slots after a successful publish.
    pub fn clear_after_publish(&mut self) {
        for slot in self.acc.iter_mut() {
            *slot = [0u64; BITMASK_WORDS];
        }
    }

    /// Move bits into an in-flight sample (x00b1): clear them from the accumulator now.
    ///
    /// Pack-time detach so a later delayed outcome cannot erase re-dirties absorbed after
    /// pack. On publish **failure** / empty delta, callers must [`Self::restore_bits`];
    /// on **success**, discard the mask (already removed from `acc`).
    pub fn detach_bits(
        &mut self,
        page_slot: usize,
        inflight_mask: &[u64; BITMASK_WORDS],
    ) -> Result<(), DirtyAccumError> {
        let capacity = self.acc.len();
        let Some(slot) = self.acc.get_mut(page_slot) else {
            return Err(DirtyAccumError::SlotOutOfRange {
                page_slot,
                capacity,
            });
        };
        for (dst, bit) in slot.iter_mut().zip(inflight_mask.iter()) {
            *dst &= !bit;
        }
        Ok(())
    }

    /// OR bits back after a failed / empty publish (inverse of [`Self::detach_bits`]).
    pub fn restore_bits(
        &mut self,
        page_slot: usize,
        inflight_mask: &[u64; BITMASK_WORDS],
    ) -> Result<(), DirtyAccumError> {
        let capacity = self.acc.len();
        let Some(slot) = self.acc.get_mut(page_slot) else {
            return Err(DirtyAccumError::SlotOutOfRange {
                page_slot,
                capacity,
            });
        };
        for (dst, bit) in slot.iter_mut().zip(inflight_mask.iter()) {
            *dst |= *bit;
        }
        Ok(())
    }

    /// Accumulated dirty mask for a page slot, if in range.
    #[inline]
    pub fn slot_mask(&self, page_slot: usize) -> Option<&[u64; BITMASK_WORDS]> {
        self.acc.get(page_slot)
    }
}

/// Largest remaining quaternion component magnitude after dropping the max axis.
const SMALLEST_THREE_RANGE: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// Pack unit quaternion to 32 bits (2-bit omitted axis + 3×10-bit).
///
/// Sign is flipped so the omitted (largest) component is non-negative (`q ≡ -q`).
pub fn pack_quaternion_smallest_three(qx: f32, qy: f32, qz: f32, qw: f32) -> u32 {
    let mut q = [qx, qy, qz, qw];
    let mut len_sq = 0.0f32;
    for c in &q {
        len_sq += *c * *c;
    }
    if len_sq.is_finite() && len_sq > 0.0 {
        let inv = len_sq.sqrt().recip();
        for c in &mut q {
            *c *= inv;
        }
    } else {
        q = [0.0, 0.0, 0.0, 1.0];
    }

    let mut max_i = 0usize;
    let mut max_abs = q[0].abs();
    for (i, c) in q.iter().enumerate().skip(1) {
        let a = c.abs();
        if a > max_abs {
            max_abs = a;
            max_i = i;
        }
    }
    if q[max_i] < 0.0 {
        for c in &mut q {
            *c = -*c;
        }
    }

    let mut others = [0.0f32; 3];
    let mut j = 0;
    for (i, c) in q.iter().enumerate() {
        if i != max_i {
            others[j] = *c;
            j += 1;
        }
    }

    let a = quantize_smallest_three_component(others[0]);
    let b = quantize_smallest_three_component(others[1]);
    let c = quantize_smallest_three_component(others[2]);
    ((max_i as u32) << 30) | (a << 20) | (b << 10) | c
}

/// Unpack Smallest-Three; omitted component is reconstructed non-negative.
pub fn unpack_quaternion_smallest_three(packed: u32) -> (f32, f32, f32, f32) {
    let max_i = ((packed >> 30) & 0b11) as usize;
    let a = dequantize_smallest_three_component((packed >> 20) & 0x3FF);
    let b = dequantize_smallest_three_component((packed >> 10) & 0x3FF);
    let c = dequantize_smallest_three_component(packed & 0x3FF);

    let mut q = [0.0f32; 4];
    let mut j = 0;
    for (i, slot) in q.iter_mut().enumerate() {
        if i == max_i {
            continue;
        }
        *slot = [a, b, c][j];
        j += 1;
    }
    let sum_sq = a * a + b * b + c * c;
    let omitted = (1.0 - sum_sq).max(0.0).sqrt();
    q[max_i] = omitted;

    let mut len_sq = 0.0f32;
    for c in &q {
        len_sq += *c * *c;
    }
    if len_sq.is_finite() && len_sq > 0.0 {
        let inv = len_sq.sqrt().recip();
        for c in &mut q {
            *c *= inv;
        }
    } else {
        q = [0.0, 0.0, 0.0, 1.0];
    }
    (q[0], q[1], q[2], q[3])
}

#[inline]
fn quantize_smallest_three_component(v: f32) -> u32 {
    let t = ((v / SMALLEST_THREE_RANGE) * 0.5 + 0.5).clamp(0.0, 1.0);
    (t * 1023.0 + 0.5).floor() as u32
}

#[inline]
fn dequantize_smallest_three_component(bits: u32) -> f32 {
    let t = (bits.min(1023) as f32) / 1023.0;
    (t * 2.0 - 1.0) * SMALLEST_THREE_RANGE
}

/// Append dirty **active** entities into `out` (cleared first; buffer reused by caller).
///
/// # Caller invariants
/// - `entity_of[local]` must be the wire-authoritative `entity_id` for every local that
///   can be `active & dirty`. This function emits that value as-is (including `0` if the
///   mapping was left unset) — it does not invent or validate IDs.
/// - Inactive locals (quarantine / despawn) are filtered by `acc & active` even if their
///   dirty bit remains; structure removal is §4.7 / snapshot, not this pose lane.
pub fn extract_dirty_records_into(
    page: &HotPhysicsPage,
    entity_of: &[u32; ENTITIES_PER_PAGE],
    acc_mask: &[u64; BITMASK_WORDS],
    out: &mut Vec<EntityDeltaRecord>,
) {
    out.clear();
    for (word_i, (acc_word, active_word)) in
        acc_mask.iter().zip(page.active_mask.iter()).enumerate()
    {
        let mut remaining = *acc_word & *active_word;
        while remaining != 0 {
            let bit = remaining.trailing_zeros() as usize;
            remaining &= remaining - 1;
            let local = word_i * 64 + bit;
            if local >= ENTITIES_PER_PAGE {
                break;
            }
            let packed = pack_quaternion_smallest_three(
                page.soa.rot_x[local],
                page.soa.rot_y[local],
                page.soa.rot_z[local],
                page.soa.rot_w[local],
            );
            out.push(EntityDeltaRecord {
                // Caller fills `entity_of`; unset slots emit `0` as wire id (see tests).
                entity_id: entity_of[local],
                local_hint: local as u16,
                pos_xyz: [
                    page.soa.pos_x[local],
                    page.soa.pos_y[local],
                    page.soa.pos_z[local],
                ],
                packed_quat: packed,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mvcc::KinematicPoseF32;

    fn identity_pose(px: f32, py: f32, pz: f32) -> KinematicPoseF32 {
        KinematicPoseF32 {
            px,
            py,
            pz,
            qx: 0.0,
            qy: 0.0,
            qz: 0.0,
            qw: 1.0,
        }
    }

    fn quat_dot(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> f32 {
        a.0 * b.0 + a.1 * b.1 + a.2 * b.2 + a.3 * b.3
    }

    fn quat_pose_error(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> f32 {
        // Same orientation if |dot| ≈ 1.
        1.0 - quat_dot(a, b).abs().min(1.0)
    }

    #[test]
    fn smallest_three_roundtrip() {
        let samples = [
            (0.0, 0.0, 0.0, 1.0),
            (1.0, 0.0, 0.0, 0.0),
            (0.0, 1.0, 0.0, 0.0),
            (0.0, 0.0, 1.0, 0.0),
            (0.5, 0.5, 0.5, 0.5),
            (0.18257418, 0.36514837, 0.5477226, 0.73029673),
            (-0.3, 0.4, -0.5, 0.7),
        ];
        for (qx, qy, qz, qw) in samples {
            let packed = pack_quaternion_smallest_three(qx, qy, qz, qw);
            let unpacked = unpack_quaternion_smallest_three(packed);
            let mut n = (qx, qy, qz, qw);
            let len = (n.0 * n.0 + n.1 * n.1 + n.2 * n.2 + n.3 * n.3).sqrt();
            n = (n.0 / len, n.1 / len, n.2 / len, n.3 / len);
            assert!(
                quat_pose_error(n, unpacked) < 2e-3,
                "pose error too large for ({qx},{qy},{qz},{qw}) -> {unpacked:?}"
            );
        }
    }

    #[test]
    fn quat_negation_same_pose() {
        let q = (0.18257418, 0.36514837, 0.5477226, 0.73029673);
        let nq = (-q.0, -q.1, -q.2, -q.3);
        let p = pack_quaternion_smallest_three(q.0, q.1, q.2, q.3);
        let pn = pack_quaternion_smallest_three(nq.0, nq.1, nq.2, nq.3);
        assert_eq!(p, pn, "q and -q must pack identically");
        let u = unpack_quaternion_smallest_three(p);
        assert!(quat_pose_error(q, u) < 2e-3);
        assert!(quat_pose_error(nq, u) < 2e-3);
    }

    #[test]
    fn dirty_survives_skipped_net_tick() {
        let mut page = HotPhysicsPage::zeroed_page();
        page.set_active(3, true);
        assert!(page.try_set_kinematic_pose(3, identity_pose(1.0, 2.0, 3.0)));

        let mut acc = WireDirtyAccumulator::from_arena_capacity(4);
        let slot = 1usize;
        // Sim tick N: dirty, absorb, then page clears dirty (as fork/commit would).
        acc.absorb(slot, &page).expect("in range");
        page.dirty_entity_mask = [0; BITMASK_WORDS];

        // Skipped net tick: no new dirty on page, still absorb (OR keeps prior).
        acc.absorb(slot, &page).expect("in range");

        let mut entity_of = [0u32; ENTITIES_PER_PAGE];
        entity_of[3] = 42;
        let mut out = Vec::new();
        extract_dirty_records_into(
            &page,
            &entity_of,
            acc.slot_mask(slot).expect("slot"),
            &mut out,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].entity_id, 42);
        assert_eq!(out[0].local_hint, 3);
        assert_eq!(out[0].pos_xyz, [1.0, 2.0, 3.0]);

        acc.clear_after_publish();
        extract_dirty_records_into(
            &page,
            &entity_of,
            acc.slot_mask(slot).expect("slot"),
            &mut out,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn detach_bits_preserves_post_pack_redirty() {
        let mut page = HotPhysicsPage::zeroed_page();
        page.set_active(3, true);
        assert!(page.try_set_kinematic_pose(3, identity_pose(1.0, 0.0, 0.0)));

        let mut acc = WireDirtyAccumulator::from_arena_capacity(1);
        acc.absorb(0, &page).expect("in range");
        let mut inflight = [0u64; BITMASK_WORDS];
        inflight[0] = 1u64 << 3;
        acc.detach_bits(0, &inflight).expect("detach");
        assert_eq!(acc.slot_mask(0).unwrap()[0] & (1 << 3), 0);

        // Re-dirty after pack (new absorb) must survive a late "success" (no restore).
        page.dirty_entity_mask = [0; BITMASK_WORDS];
        page.mark_dirty(3);
        acc.absorb(0, &page).expect("in range");
        assert_ne!(acc.slot_mask(0).unwrap()[0] & (1 << 3), 0);

        // Failure path: restore of the old inflight must OR, not wipe the re-dirty.
        acc.restore_bits(0, &inflight).expect("restore");
        assert_ne!(acc.slot_mask(0).unwrap()[0] & (1 << 3), 0);
    }

    #[test]
    fn reuse_clears_stale_dirty() {
        let mut page_old = HotPhysicsPage::zeroed_page();
        page_old.set_active(7, true);
        assert!(page_old.try_set_kinematic_pose(7, identity_pose(9.0, 0.0, 0.0)));

        let mut acc = WireDirtyAccumulator::from_arena_capacity(2);
        let slot = 0usize;
        acc.absorb(slot, &page_old).expect("in range");

        // Contract A: clear before reusing the pool slot for a different page/entity set.
        acc.clear_slot(slot).expect("in range");

        let mut page_new = HotPhysicsPage::zeroed_page();
        page_new.set_active(1, true);
        assert!(page_new.try_set_kinematic_pose(1, identity_pose(0.0, 5.0, 0.0)));
        acc.absorb(slot, &page_new).expect("in range");

        let mut entity_of = [0u32; ENTITIES_PER_PAGE];
        entity_of[1] = 100;
        entity_of[7] = 999; // must not appear
        let mut out = Vec::new();
        extract_dirty_records_into(
            &page_new,
            &entity_of,
            acc.slot_mask(slot).expect("slot"),
            &mut out,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].entity_id, 100);
        assert_eq!(out[0].local_hint, 1);
        assert!(!out.iter().any(|r| r.entity_id == 999));
    }

    #[test]
    fn absorb_out_of_range_is_err() {
        let page = HotPhysicsPage::zeroed_page();
        // Pool habit is HISTORY_TICKS+N; capacity must match — undersized accumulator errs.
        let mut acc = WireDirtyAccumulator::from_arena_capacity(64);
        assert_eq!(
            acc.absorb(64, &page),
            Err(DirtyAccumError::SlotOutOfRange {
                page_slot: 64,
                capacity: 64,
            })
        );
        assert_eq!(
            acc.clear_slot(128),
            Err(DirtyAccumError::SlotOutOfRange {
                page_slot: 128,
                capacity: 64,
            })
        );
    }

    #[test]
    fn for_arena_matches_pool_capacity() {
        let arena = MvccPageArena::new(128 + 16);
        let acc = WireDirtyAccumulator::for_arena(&arena);
        assert_eq!(acc.slot_capacity(), arena.capacity());
        assert_eq!(acc.slot_capacity(), 144);
    }

    #[test]
    fn extract_skips_inactive_even_if_dirty_acc() {
        // Quarantine / despawn: deactivate + mark_dirty. Pose dirty lane must not emit
        // inactive locals; structure removal is §4.7 / x0024 snapshot.
        let mut page = HotPhysicsPage::zeroed_page();
        page.set_active(0, true);
        assert!(page.try_set_kinematic_pose(0, identity_pose(1.0, 0.0, 0.0)));
        page.quarantine_slot(1);
        assert!(!page.is_active(1));
        assert_ne!(page.dirty_entity_mask[0] & (1u64 << 1), 0);

        let mut acc = WireDirtyAccumulator::from_arena_capacity(1);
        acc.absorb(0, &page).expect("in range");

        let mut entity_of = [0u32; ENTITIES_PER_PAGE];
        entity_of[0] = 10;
        entity_of[1] = 11;
        let mut out = Vec::new();
        extract_dirty_records_into(&page, &entity_of, acc.slot_mask(0).unwrap(), &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].entity_id, 10);
    }

    #[test]
    fn extract_emits_entity_of_verbatim() {
        // Caller invariant: unset entity_of entries are emitted as wire id 0.
        let mut page = HotPhysicsPage::zeroed_page();
        page.set_active(2, true);
        assert!(page.try_set_kinematic_pose(2, identity_pose(0.0, 1.0, 0.0)));
        let mut acc = WireDirtyAccumulator::from_arena_capacity(1);
        acc.absorb(0, &page).expect("in range");
        let entity_of = [0u32; ENTITIES_PER_PAGE]; // deliberately unset
        let mut out = Vec::new();
        extract_dirty_records_into(&page, &entity_of, acc.slot_mask(0).unwrap(), &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].entity_id, 0);
        assert_eq!(out[0].local_hint, 2);
    }

    #[test]
    fn extract_reuses_out_buffer() {
        let mut page = HotPhysicsPage::zeroed_page();
        page.set_active(0, true);
        assert!(page.try_set_kinematic_pose(0, identity_pose(1.0, 0.0, 0.0)));
        // Match arena habit: pool is HISTORY_TICKS+N, not snapshot page cap.
        let mut acc = WireDirtyAccumulator::from_arena_capacity(128 + 16);
        acc.absorb(0, &page).expect("in range");
        let entity_of = [7u32; ENTITIES_PER_PAGE];
        let mut out = Vec::with_capacity(8);
        extract_dirty_records_into(&page, &entity_of, acc.slot_mask(0).unwrap(), &mut out);
        let cap = out.capacity();
        acc.clear_after_publish();
        extract_dirty_records_into(&page, &entity_of, acc.slot_mask(0).unwrap(), &mut out);
        assert!(out.is_empty());
        assert!(out.capacity() >= cap);
    }
}
