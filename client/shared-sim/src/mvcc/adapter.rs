//! HotPhysicsPage ↔ Epoch/XPBD adapter (Phase 1 DoD; host/app 本線は未配線).
//!
//! Guarantees: page-path body-body contacts via existing [`crate::xpbd::step`] + LBVH;
//! prev reconstructed from `frame_delta` (`pos - delta`); cell_origin f64 + local f32.
//! Non-guarantees: zero heap inside XPBD scratch; history is fixed [`crate::mvcc::SnapshotSlot`]
//! (not a per-commit page `Vec` growth); SIMD/`wide` on this path; host wiring.
//! Scratch [`XpbdWorld`] is fixed-index only (`local_idx == row_id`);
//! do not call [`EpochSheet::alloc_row`] on it.

#![forbid(unsafe_code)]

use crate::spatial::SceneBounds;
use crate::tape::{ColliderShape, EpochSheet, ReadinessMask, WorldTransformf64};
use crate::topology::FlatTopology;
use crate::xpbd::{self, XpbdWorld};

use super::page::{HotPhysicsPage, ENTITIES_PER_PAGE};

/// Side table for colliders (not part of GPU SoA page).
#[derive(Clone, Debug)]
pub struct PageColliderTable {
    shapes: Box<[ColliderShape]>,
}

impl PageColliderTable {
    pub fn new() -> Self {
        Self {
            shapes: vec![ColliderShape::default(); ENTITIES_PER_PAGE].into_boxed_slice(),
        }
    }

    pub fn set(&mut self, local_idx: usize, shape: ColliderShape) {
        if local_idx < ENTITIES_PER_PAGE {
            self.shapes[local_idx] = shape;
        }
    }

    pub fn get(&self, local_idx: usize) -> ColliderShape {
        if local_idx < ENTITIES_PER_PAGE {
            self.shapes[local_idx]
        } else {
            ColliderShape::default()
        }
    }
}

impl Default for PageColliderTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Drives XPBD on a [`HotPhysicsPage`] through a reusable Epoch scratch world.
pub struct PageXpbdAdapter {
    world: XpbdWorld,
    topo: FlatTopology,
    pub colliders: PageColliderTable,
    pub bounds: SceneBounds,
}

impl PageXpbdAdapter {
    pub fn new(bounds: SceneBounds) -> Self {
        Self {
            world: XpbdWorld::new(ENTITIES_PER_PAGE),
            topo: FlatTopology::default(),
            colliders: PageColliderTable::new(),
            bounds,
        }
    }

    /// Spawn / refresh a dynamic (or kinematic) entity into the page + collider table.
    pub fn seed_entity(
        &mut self,
        page: &mut HotPhysicsPage,
        local_idx: usize,
        local_pos: [f32; 3],
        inv_mass: f32,
        shape: ColliderShape,
        frame_delta: [f32; 3],
    ) {
        if local_idx >= ENTITIES_PER_PAGE {
            return;
        }
        if !local_pos
            .iter()
            .chain(frame_delta.iter())
            .all(|v| v.is_finite())
            || !inv_mass.is_finite()
        {
            return;
        }
        page.set_active(local_idx, true);
        page.soa.pos_x[local_idx] = local_pos[0];
        page.soa.pos_y[local_idx] = local_pos[1];
        page.soa.pos_z[local_idx] = local_pos[2];
        page.soa.delta_x[local_idx] = frame_delta[0];
        page.soa.delta_y[local_idx] = frame_delta[1];
        page.soa.delta_z[local_idx] = frame_delta[2];
        page.soa.inv_mass[local_idx] = inv_mass;
        page.soa.rot_x[local_idx] = 0.0;
        page.soa.rot_y[local_idx] = 0.0;
        page.soa.rot_z[local_idx] = 0.0;
        page.soa.rot_w[local_idx] = 1.0;
        page.soa.drot_x[local_idx] = 0.0;
        page.soa.drot_y[local_idx] = 0.0;
        page.soa.drot_z[local_idx] = 0.0;
        page.soa.inv_inertia_x[local_idx] = 0.0;
        page.soa.inv_inertia_y[local_idx] = 0.0;
        page.soa.inv_inertia_z[local_idx] = 0.0;
        page.clear_holder(local_idx);
        page.clear_sleeping(local_idx);
        page.mark_dirty(local_idx);
        self.colliders.set(local_idx, shape);
    }

    /// Pull page → Epoch sheets (Curr from pos, Prev from `pos - frame_delta`),
    /// step with contacts, push back.
    ///
    /// Handover / guest restore is the same path: publish `pos` + `delta`, then
    /// `step_page` on a fresh (or any) adapter — `pull_page` rebuilds Prev.
    pub fn step_page(&mut self, page: &mut HotPhysicsPage, dt: f64) {
        self.pull_page(page);
        xpbd::sync_prev_capacity(&mut self.world);
        xpbd::step(
            &mut self.world,
            &mut self.topo,
            &[],
            &[],
            Some(self.bounds),
            dt,
        );
        self.push_page(page);
        // Intentionally no `commit_prev_from_curr`: next `pull_page` rebuilds Prev
        // from page deltas (avoids full-sheet clone on the page path).
    }

    fn pull_page(&mut self, page: &mut HotPhysicsPage) {
        ensure_sheet_len(&mut self.world.curr, ENTITIES_PER_PAGE);
        ensure_sheet_len(&mut self.world.prev, ENTITIES_PER_PAGE);
        self.world.curr.clear_dirty();
        self.world.prev.clear_dirty();

        let ox = page.cell_origin_x;
        let oy = page.cell_origin_y;
        let oz = page.cell_origin_z;

        for idx in 0..ENTITIES_PER_PAGE {
            let row = idx as u32;
            if !page.is_active(idx) {
                tombstone_fixed(&mut self.world.curr, row);
                tombstone_fixed(&mut self.world.prev, row);
                continue;
            }

            let shape = self.colliders.get(idx);
            let inv_mass = page.soa.inv_mass[idx] as f64;
            let curr_local = [
                page.soa.pos_x[idx],
                page.soa.pos_y[idx],
                page.soa.pos_z[idx],
            ];
            let delta = [
                page.soa.delta_x[idx],
                page.soa.delta_y[idx],
                page.soa.delta_z[idx],
            ];
            let prev_local = [
                curr_local[0] - delta[0],
                curr_local[1] - delta[1],
                curr_local[2] - delta[2],
            ];

            let mut ready = ReadinessMask(
                ReadinessMask::ALLOCATED
                    | ReadinessMask::TRANSFORM
                    | ReadinessMask::COLLIDER
                    | ReadinessMask::DYNAMIC,
            );
            if page.is_grabbed(idx) {
                ready = ready.with_bits(ReadinessMask::GRABBED);
            }
            if page.is_sleeping(idx) {
                ready = ready.with_bits(ReadinessMask::SLEEPING);
            }

            let mut curr_xf = WorldTransformf64::identity_at([
                ox + f64::from(curr_local[0]),
                oy + f64::from(curr_local[1]),
                oz + f64::from(curr_local[2]),
            ]);
            curr_xf.inv_mass = inv_mass;
            curr_xf.rotation = [
                page.soa.rot_x[idx],
                page.soa.rot_y[idx],
                page.soa.rot_z[idx],
                page.soa.rot_w[idx],
            ];
            curr_xf.frame_delta_xyz = delta;

            let mut prev_xf = WorldTransformf64::identity_at([
                ox + f64::from(prev_local[0]),
                oy + f64::from(prev_local[1]),
                oz + f64::from(prev_local[2]),
            ]);
            prev_xf.inv_mass = inv_mass;
            prev_xf.rotation = curr_xf.rotation;

            if self
                .world
                .curr
                .write_fixed_slot(row, ready, curr_xf, shape)
                .is_err()
                || self
                    .world
                    .prev
                    .write_fixed_slot(row, ready, prev_xf, shape)
                    .is_err()
            {
                // Quarantine: deactivate, scrub SoA NaN/Inf, tombstone sheet rows.
                page.quarantine_slot(idx);
                tombstone_fixed(&mut self.world.curr, row);
                tombstone_fixed(&mut self.world.prev, row);
            }
        }
    }

    fn push_page(&mut self, page: &mut HotPhysicsPage) {
        let ox = page.cell_origin_x;
        let oy = page.cell_origin_y;
        let oz = page.cell_origin_z;
        page.dirty_entity_mask = [0; super::page::BITMASK_WORDS];

        for idx in 0..ENTITIES_PER_PAGE {
            if !page.is_active(idx) {
                continue;
            }
            if idx >= self.world.curr.len() {
                continue;
            }
            let xf = self.world.curr.world_transforms[idx];
            if !xf.is_finite() {
                continue;
            }
            let lx = (xf.position[0] - ox) as f32;
            let ly = (xf.position[1] - oy) as f32;
            let lz = (xf.position[2] - oz) as f32;
            if !(lx.is_finite() && ly.is_finite() && lz.is_finite()) {
                continue;
            }
            page.soa.pos_x[idx] = lx;
            page.soa.pos_y[idx] = ly;
            page.soa.pos_z[idx] = lz;
            page.soa.delta_x[idx] = xf.frame_delta_xyz[0];
            page.soa.delta_y[idx] = xf.frame_delta_xyz[1];
            page.soa.delta_z[idx] = xf.frame_delta_xyz[2];
            page.soa.inv_mass[idx] = xf.inv_mass as f32;
            page.soa.rot_x[idx] = xf.rotation[0];
            page.soa.rot_y[idx] = xf.rotation[1];
            page.soa.rot_z[idx] = xf.rotation[2];
            page.soa.rot_w[idx] = xf.rotation[3];

            let sleeping = self.world.curr.readiness[idx].contains(ReadinessMask::SLEEPING);
            page.set_sleeping(idx, sleeping);
            if row_is_dirty(&self.world.curr, idx as u32) || xf.frame_delta_xyz != [0.0, 0.0, 0.0] {
                page.mark_dirty(idx);
            }
        }
    }
}

fn ensure_sheet_len(sheet: &mut EpochSheet, len: usize) {
    sheet.ensure_len_append(len);
}

fn tombstone_fixed(sheet: &mut EpochSheet, row: u32) {
    let _ = sheet.write_fixed_slot(
        row,
        ReadinessMask(ReadinessMask::TOMBSTONE),
        WorldTransformf64::default(),
        ColliderShape::default(),
    );
}

fn row_is_dirty(sheet: &EpochSheet, row_id: u32) -> bool {
    let wi = (row_id / 64) as usize;
    let bi = row_id % 64;
    sheet
        .dirty_summary_words
        .get(wi)
        .is_some_and(|w| (w & (1u64 << bi)) != 0)
}

/// Bit-exact page fingerprint for determinism tests (soa + origins + all masks).
pub fn page_fingerprint(page: &HotPhysicsPage) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut h = DefaultHasher::new();
    bytemuck::bytes_of(&page.soa).hash(&mut h);
    page.cell_origin_x.to_bits().hash(&mut h);
    page.cell_origin_y.to_bits().hash(&mut h);
    page.cell_origin_z.to_bits().hash(&mut h);
    for w in &page.active_mask {
        w.hash(&mut h);
    }
    for w in &page.dirty_entity_mask {
        w.hash(&mut h);
    }
    for w in page.grabbed_mask_words() {
        w.hash(&mut h);
    }
    for w in &page.sleeping_mask {
        w.hash(&mut h);
    }
    page.holder_slots().hash(&mut h);
    page.holder_since_ticks().hash(&mut h);
    h.finish()
}

/// True when two pages match bit-for-bit on SoA + all masks + holders (incl. dirty).
pub fn pages_bit_equal(a: &HotPhysicsPage, b: &HotPhysicsPage) -> bool {
    bytemuck::bytes_of(&a.soa) == bytemuck::bytes_of(&b.soa)
        && a.cell_origin_x.to_bits() == b.cell_origin_x.to_bits()
        && a.cell_origin_y.to_bits() == b.cell_origin_y.to_bits()
        && a.cell_origin_z.to_bits() == b.cell_origin_z.to_bits()
        && a.active_mask == b.active_mask
        && a.dirty_entity_mask == b.dirty_entity_mask
        && a.grabbed_mask_words() == b.grabbed_mask_words()
        && a.sleeping_mask == b.sleeping_mask
        && a.holder_slots() == b.holder_slots()
        && a.holder_since_ticks() == b.holder_since_ticks()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mvcc::arena::MvccPageArena;
    use crate::mvcc::page::HISTORY_TICKS;

    fn box_shape(half: f32) -> ColliderShape {
        ColliderShape {
            shape_type: ColliderShape::BOX,
            extents: [half, half, half],
        }
    }

    fn sphere_shape(r: f32) -> ColliderShape {
        ColliderShape {
            shape_type: ColliderShape::SPHERE,
            extents: [r, 0.0, 0.0],
        }
    }

    #[test]
    fn page_path_body_body_boxes_do_not_remain_overlapping() {
        let bounds = SceneBounds {
            min: [-10.0, -10.0, -10.0],
            max: [10.0, 10.0, 10.0],
        };
        let mut adapter = PageXpbdAdapter::new(bounds);
        adapter.world.gravity = [0.0, 0.0, 0.0];
        adapter.world.substeps = 8;

        let mut page = HotPhysicsPage::zeroed_page();
        let half = 0.25_f32;
        adapter.seed_entity(
            &mut page,
            0,
            [-0.1, 0.0, 0.0],
            1.0,
            box_shape(half),
            [0.0, 0.0, 0.0],
        );
        adapter.seed_entity(
            &mut page,
            1,
            [0.1, 0.0, 0.0],
            1.0,
            box_shape(half),
            [0.0, 0.0, 0.0],
        );

        let dt = 1.0 / 60.0;
        for _ in 0..40 {
            adapter.step_page(&mut page, dt);
        }

        let dx = (page.soa.pos_x[0] - page.soa.pos_x[1]) as f64;
        let dy = (page.soa.pos_y[0] - page.soa.pos_y[1]) as f64;
        let dz = (page.soa.pos_z[0] - page.soa.pos_z[1]) as f64;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        let radius = (3.0_f64 * (half as f64).powi(2)).sqrt();
        let min_sep = 2.0 * radius;
        assert!(
            dist + 1e-3 >= min_sep,
            "boxes still overlapping: dist={dist} min_sep={min_sep}"
        );
        assert!(page.soa.pos_x[0].is_finite() && page.soa.pos_x[1].is_finite());
    }

    #[test]
    fn arena_cow_1k_entities_500_ticks_page_alloc_bounded() {
        let mut arena = MvccPageArena::new(HISTORY_TICKS + 32);
        let bounds = SceneBounds::default();
        let mut adapter = PageXpbdAdapter::new(bounds);
        adapter.world.gravity = [0.0, -9.81, 0.0];
        adapter.world.substeps = 2;

        let mut page_id = arena.alloc_page().expect("initial page");
        {
            let page = arena.make_mut(&mut page_id).expect("mut");
            for i in 0..1000 {
                let x = (i % 32) as f32 * 0.5 - 8.0;
                let z = (i / 32) as f32 * 0.5 - 8.0;
                adapter.seed_entity(
                    page,
                    i,
                    [x, 2.0 + (i % 7) as f32 * 0.01, z],
                    1.0,
                    sphere_shape(0.1),
                    [0.0, 0.0, 0.0],
                );
            }
        }
        arena.commit_transfer_slice(0, &[page_id]).unwrap();

        const MAX_ALLOCS_PER_TICK: u32 = 2;
        let dt = 1.0 / 60.0;
        for tick in 1u64..=500 {
            arena.begin_tick();
            {
                let cap = arena.capacity();
                let total = arena.total_page_allocs();
                let page = arena.make_mut(&mut page_id).unwrap_or_else(|e| {
                    panic!("make_mut failed at tick {tick}: {e:?}; capacity={cap} total_allocs={total}")
                });
                adapter.step_page(page, dt);
            }
            assert!(
                arena.tick_page_allocs() <= MAX_ALLOCS_PER_TICK,
                "tick {tick} page allocs {}",
                arena.tick_page_allocs()
            );
            arena.commit_transfer_slice(tick, &[page_id]).unwrap();
        }
        assert_eq!(arena.latest_tick(), 500);
        assert_eq!(arena.total_page_allocs(), 501);
    }

    #[test]
    fn snapshot_at_rollback_checkout_resimulates() {
        let bounds = SceneBounds {
            min: [-20.0, -20.0, -20.0],
            max: [20.0, 20.0, 20.0],
        };
        let dt = 1.0 / 60.0;
        let mut arena = MvccPageArena::new(HISTORY_TICKS + 16);
        let mut adapter = PageXpbdAdapter::new(bounds);
        adapter.world.gravity = [0.0, -9.81, 0.0];
        adapter.world.substeps = 4;

        let mut page_id = arena.alloc_page().expect("page");
        {
            let page = arena.make_mut(&mut page_id).expect("mut");
            for i in 0..8 {
                adapter.seed_entity(
                    page,
                    i,
                    [i as f32 * 0.6, 1.0 + i as f32 * 0.1, 0.0],
                    1.0,
                    sphere_shape(0.2),
                    [0.0, 0.0, 0.0],
                );
            }
        }
        arena.commit_transfer_slice(0, &[page_id]).unwrap();

        // Advance far enough that HISTORY wraps, then checkout an in-window tick.
        for tick in 1u64..=200 {
            {
                let page = arena.make_mut(&mut page_id).expect("make_mut");
                adapter.step_page(page, dt);
            }
            arena.commit_transfer_slice(tick, &[page_id]).unwrap();
        }

        let rollback_tick = 200 - 50; // still inside 128-tick ring
        let snap = arena
            .snapshot_at(rollback_tick)
            .expect("rollback snapshot live");
        let rolled_id = snap.pages_slice()[0];
        let mut rolled = *arena.page(rolled_id).expect("rollback page live");

        let continuous_at_200 = *arena.page(page_id).expect("tip");

        let mut replay = PageXpbdAdapter::new(bounds);
        replay.world.gravity = [0.0, -9.81, 0.0];
        replay.world.substeps = 4;
        replay.colliders = adapter.colliders.clone();
        for _ in rollback_tick..200 {
            replay.step_page(&mut rolled, dt);
        }
        assert!(
            pages_bit_equal(&continuous_at_200, &rolled),
            "rollback resim mismatch fp_cont={} fp_roll={}",
            page_fingerprint(&continuous_at_200),
            page_fingerprint(&rolled)
        );
    }

    #[test]
    fn bit_exact_replay_from_tick_50_to_100() {
        let bounds = SceneBounds {
            min: [-20.0, -20.0, -20.0],
            max: [20.0, 20.0, 20.0],
        };
        let dt = 1.0 / 60.0;

        let run_to = |end_tick: u64, checkout_at: Option<u64>| -> HotPhysicsPage {
            let mut arena = MvccPageArena::new(HISTORY_TICKS + 16);
            let mut adapter = PageXpbdAdapter::new(bounds);
            adapter.world.gravity = [0.0, -9.81, 0.0];
            adapter.world.substeps = 4;

            let mut page_id = arena.alloc_page().expect("page");
            {
                let page = arena.make_mut(&mut page_id).expect("mut");
                for i in 0..8 {
                    adapter.seed_entity(
                        page,
                        i,
                        [i as f32 * 0.6, 1.0 + i as f32 * 0.1, 0.0],
                        1.0,
                        sphere_shape(0.2),
                        [0.0, 0.0, 0.0],
                    );
                }
            }
            arena.commit_transfer_slice(0, &[page_id]).unwrap();

            let mut checkout: Option<HotPhysicsPage> = None;
            for tick in 1..=end_tick {
                {
                    let page = arena.make_mut(&mut page_id).expect("make_mut");
                    adapter.step_page(page, dt);
                }
                arena.commit_transfer_slice(tick, &[page_id]).unwrap();
                if Some(tick) == checkout_at {
                    checkout = Some(*arena.page(page_id).expect("snap"));
                }
            }

            if let Some(mut mid) = checkout {
                let mut replay = PageXpbdAdapter::new(bounds);
                replay.world.gravity = [0.0, -9.81, 0.0];
                replay.world.substeps = 4;
                replay.colliders = adapter.colliders.clone();
                for _ in checkout_at.unwrap()..end_tick {
                    replay.step_page(&mut mid, dt);
                }
                mid
            } else {
                *arena.page(page_id).expect("final")
            }
        };

        let continuous = run_to(100, None);
        let replayed = run_to(100, Some(50));
        assert!(
            pages_bit_equal(&continuous, &replayed),
            "fingerprint continuous={} replayed={}",
            page_fingerprint(&continuous),
            page_fingerprint(&replayed)
        );
    }

    #[test]
    fn pull_quarantines_non_finite_active_entity() {
        let mut adapter = PageXpbdAdapter::new(SceneBounds::default());
        adapter.world.gravity = [0.0, 0.0, 0.0];
        let mut page = HotPhysicsPage::zeroed_page();
        adapter.seed_entity(
            &mut page,
            0,
            [0.0, 1.0, 0.0],
            1.0,
            sphere_shape(0.2),
            [0.0, 0.0, 0.0],
        );
        page.soa.pos_x[0] = f32::NAN;
        adapter.step_page(&mut page, 1.0 / 60.0);
        assert!(!page.is_active(0));
        assert!(page.soa.pos_x[0].is_finite());
        assert_eq!(page.soa.pos_x[0], 0.0);
        assert!(adapter.world.curr.readiness[0].contains(ReadinessMask::TOMBSTONE));
    }

    #[test]
    fn frame_delta_restore_matches_continuous_tick() {
        let bounds = SceneBounds::default();
        let dt = 1.0 / 60.0;
        let mut continuous = PageXpbdAdapter::new(bounds);
        continuous.world.gravity = [0.0, -9.81, 0.0];
        continuous.world.substeps = 4;

        let mut page_cont = HotPhysicsPage::zeroed_page();
        continuous.seed_entity(
            &mut page_cont,
            0,
            [0.0, 5.0, 0.0],
            1.0,
            sphere_shape(0.25),
            [0.0, 0.0, 0.0],
        );

        for _ in 0..10 {
            continuous.step_page(&mut page_cont, dt);
        }
        let published = page_cont;

        let mut page_a = published;
        continuous.step_page(&mut page_a, dt);

        // Guest path: only published pos+delta; fresh adapter rebuilds Prev via pull.
        let mut page_b = published;
        let mut restored = PageXpbdAdapter::new(bounds);
        restored.world.gravity = [0.0, -9.81, 0.0];
        restored.world.substeps = 4;
        restored.colliders = continuous.colliders.clone();
        restored.step_page(&mut page_b, dt);

        assert!(
            pages_bit_equal(&page_a, &page_b),
            "handover trajectory mismatch: cont=({},{}) restore=({},{})",
            page_a.soa.pos_y[0],
            page_a.soa.delta_y[0],
            page_b.soa.pos_y[0],
            page_b.soa.delta_y[0]
        );
    }
}
