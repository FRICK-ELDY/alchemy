//! Native XPBD solver over EpochPrev / EpochCurr tapes.
//!
//! Guarantees: position-based constraints (no force blow-up); Grab via GRABBED bit;
//! sleeping / dirty bitsets for sparse work; finite gate on writes.
//! Non-guarantees: full-body IK UX; Zenoh delta publish; Dual-AST Kernel;
//! Grab-vs-contact priority under heavy overlap (sensor exclusion is best-effort).
//!
//! Safety: `#![forbid(unsafe_code)]`.

#![forbid(unsafe_code)]

use crate::spatial::{collect_overlap_pairs, FlatLbvh};
use crate::tape::{ColliderShape, EpochSheet, ReadinessMask, XpbdConstraintRow};
use crate::topology::FlatTopology;

/// Dual-buffered simulation state.
#[derive(Clone, Debug, Default)]
pub struct XpbdWorld {
    pub prev: EpochSheet,
    pub curr: EpochSheet,
    pub gravity: [f64; 3],
    pub substeps: u32,
    pub sleep_epsilon: f32,
    /// Reused across substeps (not part of the published face).
    constraint_scratch: Vec<XpbdConstraintRow>,
    sub_prev_scratch: Vec<[f64; 3]>,
    x_before_scratch: Vec<[f64; 3]>,
}

impl XpbdWorld {
    pub fn new(capacity: usize) -> Self {
        Self {
            prev: EpochSheet::with_capacity(capacity),
            curr: EpochSheet::with_capacity(capacity),
            gravity: [0.0, -9.81, 0.0],
            substeps: 4,
            sleep_epsilon: 1e-4,
            constraint_scratch: Vec::new(),
            sub_prev_scratch: Vec::with_capacity(capacity),
            x_before_scratch: Vec::with_capacity(capacity),
        }
    }

    /// Mirror curr into prev after a committed tick (call after solve).
    pub fn commit_prev_from_curr(&mut self) {
        self.prev = self.curr.clone();
    }
}

/// Inertial predict: Curr = Curr + (Curr - sub_prev_scratch) + g * dt².
///
/// Reads [`XpbdWorld::sub_prev_scratch`] (position at start of this substep).
pub fn predict_integrate(world: &mut XpbdWorld, dt: f64) {
    let dt2 = dt * dt;
    let rows: Vec<u32> = world.curr.iter_active_rows().collect();
    let g = world.gravity;
    for row in rows {
        let idx = row as usize;
        if idx >= world.curr.len() || idx >= world.sub_prev_scratch.len() {
            continue;
        }
        let mask = world.curr.readiness[idx];
        if !mask.is_ready(ReadinessMask::DYNAMIC) || mask.contains(ReadinessMask::SLEEPING) {
            continue;
        }
        if mask.contains(ReadinessMask::GRABBED) {
            continue;
        }
        let inv_mass = world.curr.world_transforms[idx].inv_mass;
        if inv_mass == 0.0 {
            continue;
        }
        let curr = world.curr.world_transforms[idx].position;
        let prev = world.sub_prev_scratch[idx];
        let mut next = [
            curr[0] + (curr[0] - prev[0]) + g[0] * dt2,
            curr[1] + (curr[1] - prev[1]) + g[1] * dt2,
            curr[2] + (curr[2] - prev[2]) + g[2] * dt2,
        ];
        if !next.iter().all(|v| v.is_finite()) {
            next = curr;
        }
        let mut xf = world.curr.world_transforms[idx];
        xf.frame_delta_xyz = [
            (next[0] - curr[0]) as f32,
            (next[1] - curr[1]) as f32,
            (next[2] - curr[2]) as f32,
        ];
        xf.position = next;
        let _ = world.curr.set_transform(row, xf);
    }
}

/// Graph-color constraints so each batch has disjoint node endpoints.
pub fn color_constraints(constraints: &[XpbdConstraintRow], topo: &mut FlatTopology) {
    let mut remaining: Vec<XpbdConstraintRow> = constraints.to_vec();
    topo.constraint_color_batches.clear();
    topo.color_batch_offsets.clear();
    topo.color_batch_offsets.push(0);
    while !remaining.is_empty() {
        let mut used = std::collections::HashSet::new();
        let mut batch = Vec::new();
        let mut rest = Vec::new();
        for c in remaining {
            let conflict =
                used.contains(&c.node_a) || (c.node_b != u32::MAX && used.contains(&c.node_b));
            if conflict {
                rest.push(c);
            } else {
                used.insert(c.node_a);
                if c.node_b != u32::MAX {
                    used.insert(c.node_b);
                }
                batch.push(c);
            }
        }
        topo.constraint_color_batches.extend(batch);
        topo.color_batch_offsets
            .push(topo.constraint_color_batches.len() as u32);
        remaining = rest;
    }
}

/// Solve distance constraints (hard + soft via compliance) one batch color at a time.
pub fn solve_distance_batches(
    sheet: &mut EpochSheet,
    topo: &FlatTopology,
    grab_targets: &[(u32, [f64; 3])],
    dt: f64,
) {
    let alpha_scale = if dt > 0.0 { 1.0 / (dt * dt) } else { 0.0 };
    for bi in 0..topo.color_batch_offsets.len().saturating_sub(1) {
        let start = topo.color_batch_offsets[bi] as usize;
        let end = topo.color_batch_offsets[bi + 1] as usize;
        for c in &topo.constraint_color_batches[start..end] {
            solve_one(sheet, c, grab_targets, alpha_scale);
        }
    }
}

fn solve_one(
    sheet: &mut EpochSheet,
    c: &XpbdConstraintRow,
    grab_targets: &[(u32, [f64; 3])],
    alpha_scale: f64,
) {
    let a = c.node_a as usize;
    if a >= sheet.world_transforms.len() {
        return;
    }
    let off_a = [
        c.anchor_offset_a[0] as f64,
        c.anchor_offset_a[1] as f64,
        c.anchor_offset_a[2] as f64,
    ];
    if c.node_b == u32::MAX {
        // World / Grab anchor: drive node_a so (position + anchor_offset_a) → target.
        let target = grab_targets
            .iter()
            .find(|(row, _)| *row == c.node_a)
            .map(|(_, p)| *p);
        let Some(target) = target else {
            return;
        };
        let desired = [
            target[0] - off_a[0],
            target[1] - off_a[1],
            target[2] - off_a[2],
        ];
        let mut xf = sheet.world_transforms[a];
        if xf.inv_mass == 0.0 {
            return;
        }
        let compliance = c.compliance as f64;
        if compliance <= 0.0 {
            xf.position = desired;
        } else {
            let alpha = compliance * alpha_scale;
            let w = 1.0 / (1.0 + alpha);
            xf.position = [
                xf.position[0] + (desired[0] - xf.position[0]) * w,
                xf.position[1] + (desired[1] - xf.position[1]) * w,
                xf.position[2] + (desired[2] - xf.position[2]) * w,
            ];
        }
        if xf.is_finite() {
            let _ = sheet.set_transform(c.node_a, xf);
            sheet.mark_dirty(c.node_a);
        }
        return;
    }

    let b = c.node_b as usize;
    if b >= sheet.world_transforms.len() {
        return;
    }
    let off_b = [
        c.anchor_offset_b[0] as f64,
        c.anchor_offset_b[1] as f64,
        c.anchor_offset_b[2] as f64,
    ];
    let ba = sheet.world_transforms[a].position;
    let bb = sheet.world_transforms[b].position;
    let pa = [ba[0] + off_a[0], ba[1] + off_a[1], ba[2] + off_a[2]];
    let pb = [bb[0] + off_b[0], bb[1] + off_b[1], bb[2] + off_b[2]];
    let wa = sheet.world_transforms[a].inv_mass;
    let wb = sheet.world_transforms[b].inv_mass;
    if wa + wb == 0.0 {
        return;
    }
    let mut d = [pa[0] - pb[0], pa[1] - pb[1], pa[2] - pb[2]];
    let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if dist < 1e-12 {
        d = [1.0, 0.0, 0.0];
    } else {
        d = [d[0] / dist, d[1] / dist, d[2] / dist];
    }
    let rest = c.target_distance as f64;
    let c_err = dist - rest;
    // Inequality contact: only push apart when penetrating (dist < rest).
    // Equality joints / springs still attract when too far.
    if c.is_inequality() && c_err >= 0.0 {
        return;
    }
    let alpha = (c.compliance as f64) * alpha_scale;
    let denom = wa + wb + alpha;
    if denom == 0.0 {
        return;
    }
    let lambda = -c_err / denom;
    let corr = [d[0] * lambda, d[1] * lambda, d[2] * lambda];
    if wa > 0.0 {
        let mut xa = sheet.world_transforms[a];
        xa.position = [
            xa.position[0] + corr[0] * wa,
            xa.position[1] + corr[1] * wa,
            xa.position[2] + corr[2] * wa,
        ];
        if xa.is_finite() {
            let _ = sheet.set_transform(c.node_a, xa);
            sheet.mark_dirty(c.node_a);
        }
    }
    if wb > 0.0 {
        let mut xb = sheet.world_transforms[b];
        xb.position = [
            xb.position[0] - corr[0] * wb,
            xb.position[1] - corr[1] * wb,
            xb.position[2] - corr[2] * wb,
        ];
        if xb.is_finite() {
            let _ = sheet.set_transform(c.node_b, xb);
            sheet.mark_dirty(c.node_b);
        }
    }
    let _ = c.damping; // reserved for velocity-level damping on soft constraints
}

/// Build Grab constraints for rows with GRABBED bit (extends `out`).
pub fn grab_constraint_rows_into(sheet: &EpochSheet, out: &mut Vec<XpbdConstraintRow>) {
    for row in sheet.iter_active_rows() {
        let idx = row as usize;
        if sheet.readiness[idx].contains(ReadinessMask::GRABBED) {
            out.push(XpbdConstraintRow {
                node_a: row,
                node_b: u32::MAX,
                ..Default::default()
            });
        }
    }
}

/// Convert overlapping pairs into inequality non-penetration constraints (extends `out`).
///
/// Separated bodies (`dist >= ra+rb`) are not pulled together (x00a8).
///
/// Pair filter (x00ad follow-up): a row participates if it is **DYNAMIC**, or a
/// **static BOX** (world geometry). Non-DYNAMIC spheres (host `user_root` sensors)
/// do **not** emit contacts — they must not shove dynamics on proximity.
pub fn contact_constraints_from_lbvh_into(
    sheet: &EpochSheet,
    lbvh: &FlatLbvh,
    out: &mut Vec<XpbdConstraintRow>,
) {
    for (a, b) in collect_overlap_pairs(lbvh) {
        if !row_emits_body_contact(sheet, a) || !row_emits_body_contact(sheet, b) {
            continue;
        }
        let ia = a as usize;
        let ib = b as usize;
        let ra = collider_radius_estimate(&sheet.colliders[ia]);
        let rb = collider_radius_estimate(&sheet.colliders[ib]);
        out.push(XpbdConstraintRow {
            node_a: a,
            node_b: b,
            target_distance: ra + rb,
            flags: XpbdConstraintRow::FLAG_INEQUALITY,
            ..Default::default()
        });
    }
}

#[inline]
fn row_emits_body_contact(sheet: &EpochSheet, row: u32) -> bool {
    let idx = row as usize;
    if idx >= sheet.len() {
        return false;
    }
    let mask = sheet.readiness[idx];
    if !mask.is_ready(ReadinessMask::COLLIDER) {
        return false;
    }
    if mask.is_ready(ReadinessMask::DYNAMIC) {
        return true;
    }
    // Fixed world boxes (inv_mass typically 0, no DYNAMIC bit).
    sheet.colliders[idx].shape_type == ColliderShape::BOX
}

fn collider_radius_estimate(shape: &ColliderShape) -> f32 {
    match shape.shape_type {
        ColliderShape::SPHERE => shape.extents[0],
        ColliderShape::CAPSULE => shape.extents[0] + shape.extents[1],
        ColliderShape::BOX => {
            let e = shape.extents;
            (e[0] * e[0] + e[1] * e[1] + e[2] * e[2]).sqrt()
        }
        _ => shape.extents[0],
    }
}

/// Sleep nearly-static dynamic bodies; wake on Grab or large delta.
pub fn update_sleeping(world: &mut XpbdWorld) {
    let n = world.curr.len().min(world.prev.len());
    let eps2 = (world.sleep_epsilon as f64) * (world.sleep_epsilon as f64);
    let rows: Vec<u32> = world.curr.iter_active_rows().collect();
    for row in rows {
        let idx = row as usize;
        if idx >= n {
            continue;
        }
        let mut mask = world.curr.readiness[idx];
        if !mask.is_ready(ReadinessMask::DYNAMIC) {
            continue;
        }
        if mask.contains(ReadinessMask::GRABBED) {
            mask = mask.without_bits(ReadinessMask::SLEEPING);
            world.curr.readiness[idx] = mask;
            world.curr.mark_dirty(row);
            continue;
        }
        let c = world.curr.world_transforms[idx].position;
        let p = world.prev.world_transforms[idx].position;
        let d2 = (c[0] - p[0]).powi(2) + (c[1] - p[1]).powi(2) + (c[2] - p[2]).powi(2);
        if d2 < eps2 {
            mask = mask.with_bits(ReadinessMask::SLEEPING);
        } else {
            mask = mask.without_bits(ReadinessMask::SLEEPING);
            world.curr.mark_dirty(row);
        }
        world.curr.readiness[idx] = mask;
    }
}

/// Extract dirty row transforms for network publish (API only).
#[derive(Clone, Debug)]
pub struct TransformDelta {
    pub row_id: u32,
    pub position: [f64; 3],
    pub rotation: [f32; 4],
}

/// Extract dirty transforms. Prefer the **published** Curr Arc from
/// [`crate::bridge::EpochSnapshotPair::load_curr`] after a tick — live `world.curr`
/// is cleared of dirty bits at the end of [`crate::bridge::FrameBridge::tick`].
pub fn extract_dirty_deltas(sheet: &EpochSheet) -> Vec<TransformDelta> {
    let mut out = Vec::new();
    for row in sheet.iter_dirty_rows() {
        let idx = row as usize;
        if idx >= sheet.world_transforms.len() {
            continue;
        }
        if sheet.readiness[idx].contains(ReadinessMask::SLEEPING) {
            continue;
        }
        let xf = &sheet.world_transforms[idx];
        out.push(TransformDelta {
            row_id: row,
            position: xf.position,
            rotation: xf.rotation,
        });
    }
    out
}

/// One XPBD tick: predict → color → solve (grab + contacts + user constraints) → sleep.
///
/// Does **not** commit Prev←Curr. Caller must `publish(prev, curr)` for Ring3, then
/// [`XpbdWorld::commit_prev_from_curr`] so next tick's velocity uses end-of-frame Curr.
///
/// Frame `world.prev` stays at start-of-tick for publish. Substep Verlet uses a scratch
/// `sub_prev` advanced after each substep so gravity is not re-applied against the frame start.
///
/// When `bounds` is `Some`, rebuilds Flat LBVH each substep (contacts track motion).
pub fn step(
    world: &mut XpbdWorld,
    topo: &mut FlatTopology,
    user_constraints: &[XpbdConstraintRow],
    grab_targets: &[(u32, [f64; 3])],
    bounds: Option<crate::spatial::SceneBounds>,
    dt: f64,
) {
    let sub_dt = dt / world.substeps.max(1) as f64;
    // Substep Verlet x_old — starts as frame Prev (usually == Curr after sync).
    // Frame `world.prev` stays start-of-tick for Ring3 publish.
    let n = world.curr.len();
    world.sub_prev_scratch.clear();
    world.sub_prev_scratch.reserve(n);
    for i in 0..n {
        let p = if i < world.prev.len() {
            world.prev.world_transforms[i].position
        } else {
            world.curr.world_transforms[i].position
        };
        world.sub_prev_scratch.push(p);
    }
    world.x_before_scratch.clear();
    world.x_before_scratch.resize(n, [0.0; 3]);
    for _ in 0..world.substeps.max(1) {
        let n = world.curr.len();
        if world.x_before_scratch.len() < n {
            world.x_before_scratch.resize(n, [0.0; 3]);
        }
        while world.sub_prev_scratch.len() < n {
            let i = world.sub_prev_scratch.len();
            world
                .sub_prev_scratch
                .push(world.curr.world_transforms[i].position);
        }
        for (i, xf) in world.curr.world_transforms.iter().enumerate() {
            world.x_before_scratch[i] = xf.position;
        }
        predict_integrate(world, sub_dt);
        world.constraint_scratch.clear();
        world.constraint_scratch.extend_from_slice(user_constraints);
        grab_constraint_rows_into(&world.curr, &mut world.constraint_scratch);
        if let Some(b) = bounds {
            let tree = FlatLbvh::build_from_sheet(&world.curr, b);
            contact_constraints_from_lbvh_into(&world.curr, &tree, &mut world.constraint_scratch);
        }
        let constraints = std::mem::take(&mut world.constraint_scratch);
        color_constraints(&constraints, topo);
        world.constraint_scratch = constraints;
        solve_distance_batches(&mut world.curr, topo, grab_targets, sub_dt);
        // Classic Verlet: x_old ← pre-predict Curr (not post-solve).
        std::mem::swap(&mut world.sub_prev_scratch, &mut world.x_before_scratch);
    }
    update_sleeping(world);
}

/// Align Prev to Curr for ABA-safe Verlet.
///
/// Copy Curr → Prev when:
/// - generation mismatch (freelist reuse), or
/// - Prev slot is TOMBSTONE / inactive while Curr is live (newly appended gen=0 pads,
///   both gens are 0 so mismatch alone is not enough).
///
/// Uses [`EpochSheet::ensure_len_append`] (never freelist) so Prev growth cannot loop.
pub fn sync_prev_capacity(world: &mut XpbdWorld) {
    world.prev.ensure_len_append(world.curr.len());
    for i in 0..world.curr.len() {
        let row = i as u32;
        let prev_gen = world.prev.generation(row).unwrap_or(0);
        let curr_gen = world.curr.generation(row).unwrap_or(0);
        let prev_tomb = world.prev.readiness[i].contains(ReadinessMask::TOMBSTONE);
        let curr_live = !world.curr.readiness[i].contains(ReadinessMask::TOMBSTONE);
        if prev_gen != curr_gen || (prev_tomb && curr_live) {
            world.prev.copy_row_from(row, &world.curr, row);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spatial::{FlatLbvh, SceneBounds};
    use crate::tape::WorldTransformf64;
    use crate::topology::validate_color_batches;

    fn dynamic_at(sheet: &mut EpochSheet, pos: [f64; 3]) -> u32 {
        let h = sheet.alloc_row();
        let mut xf = WorldTransformf64::identity_at(pos);
        xf.inv_mass = 1.0;
        sheet.set_transform(h.row_id, xf).unwrap();
        sheet.readiness[h.row_id as usize] = ReadinessMask(
            ReadinessMask::ALLOCATED | ReadinessMask::DYNAMIC | ReadinessMask::TRANSFORM,
        );
        h.row_id
    }

    #[test]
    fn free_fall_moves_down_static_stays() {
        let mut world = XpbdWorld::new(2);
        let dyn_row = dynamic_at(&mut world.curr, [0.0, 10.0, 0.0]);
        let static_row = {
            let h = world.curr.alloc_row();
            let mut xf = WorldTransformf64::identity_at([0.0, 0.0, 0.0]);
            xf.inv_mass = 0.0;
            world.curr.set_transform(h.row_id, xf).unwrap();
            world.curr.readiness[h.row_id as usize] =
                ReadinessMask(ReadinessMask::ALLOCATED | ReadinessMask::DYNAMIC);
            h.row_id
        };
        sync_prev_capacity(&mut world);
        world.prev = world.curr.clone();
        world.sub_prev_scratch = world
            .prev
            .world_transforms
            .iter()
            .map(|t| t.position)
            .collect();
        predict_integrate(&mut world, 1.0 / 60.0);
        assert!(world.curr.world_transforms[dyn_row as usize].position[1] < 10.0);
        assert_eq!(
            world.curr.world_transforms[static_row as usize].position,
            [0.0, 0.0, 0.0]
        );
    }

    #[test]
    fn sync_prev_does_not_loop_when_prev_has_freelist() {
        let mut world = XpbdWorld::new(2);
        let a = world.prev.alloc_row();
        world.prev.tombstone(a).unwrap();
        // Curr longer than Prev occupied slots but Prev free_list non-empty.
        let _ = world.curr.alloc_row();
        let _ = world.curr.alloc_row();
        sync_prev_capacity(&mut world);
        assert_eq!(world.prev.len(), world.curr.len());
    }

    #[test]
    fn sync_prev_resets_aba_generation_mismatch() {
        let mut world = XpbdWorld::new(1);
        let old = world.curr.alloc_row();
        world
            .curr
            .set_transform(old.row_id, WorldTransformf64::identity_at([1.0, 2.0, 3.0]))
            .unwrap();
        sync_prev_capacity(&mut world);
        world.prev = world.curr.clone();
        world.curr.tombstone(old).unwrap();
        let neu = world.curr.alloc_row();
        assert_eq!(neu.row_id, old.row_id);
        world
            .curr
            .set_transform(neu.row_id, WorldTransformf64::identity_at([9.0, 9.0, 9.0]))
            .unwrap();
        // Without sync, prev still has [1,2,3] at same row_id → false velocity.
        sync_prev_capacity(&mut world);
        assert_eq!(
            world.prev.world_transforms[neu.row_id as usize].position,
            [9.0, 9.0, 9.0]
        );
        assert_eq!(
            world.prev.generation(neu.row_id),
            world.curr.generation(neu.row_id)
        );
    }

    #[test]
    fn sync_prev_copies_appended_gen0_live_over_tombstone_pad() {
        let mut world = XpbdWorld::new(0);
        // Curr grows with a live gen=0 row; Prev pad from ensure_len is also gen=0 TOMBSTONE.
        let h = world.curr.alloc_row();
        world
            .curr
            .set_transform(h.row_id, WorldTransformf64::identity_at([3.0, 4.0, 5.0]))
            .unwrap();
        world.curr.readiness[h.row_id as usize] =
            ReadinessMask(ReadinessMask::ALLOCATED | ReadinessMask::DYNAMIC);
        sync_prev_capacity(&mut world);
        assert_eq!(
            world.prev.world_transforms[h.row_id as usize].position,
            [3.0, 4.0, 5.0]
        );
        assert!(!world.prev.readiness[h.row_id as usize].contains(ReadinessMask::TOMBSTONE));
    }

    #[test]
    fn multi_substep_fall_matches_single_order() {
        let mut one = XpbdWorld::new(1);
        let mut four = XpbdWorld::new(1);
        one.substeps = 1;
        four.substeps = 4;
        for w in [&mut one, &mut four] {
            let _ = dynamic_at(&mut w.curr, [0.0, 10.0, 0.0]);
            sync_prev_capacity(w);
            w.prev = w.curr.clone();
        }
        let mut topo = FlatTopology::default();
        let dt = 1.0 / 60.0;
        step(&mut one, &mut topo, &[], &[], None, dt);
        step(&mut four, &mut topo, &[], &[], None, dt);
        let drop1 = 10.0 - one.curr.world_transforms[0].position[1];
        let drop4 = 10.0 - four.curr.world_transforms[0].position[1];
        assert!(drop1 > 0.0 && drop4 > 0.0);
        // Must not explode (~4×) nor collapse to ~1/n (solve-after sub_prev=curr bug).
        let ratio = drop4 / drop1;
        assert!(
            (0.5..=2.0).contains(&ratio),
            "4-substep/1-substep drop ratio {ratio} (drop1={drop1} drop4={drop4})"
        );
    }

    #[test]
    fn hard_distance_converges() {
        let mut world = XpbdWorld::new(2);
        let a = dynamic_at(&mut world.curr, [0.0, 0.0, 0.0]);
        let b = dynamic_at(&mut world.curr, [2.0, 0.0, 0.0]);
        sync_prev_capacity(&mut world);
        world.prev = world.curr.clone();
        let mut topo = FlatTopology::default();
        let constraints = [XpbdConstraintRow {
            node_a: a,
            node_b: b,
            target_distance: 1.0,
            ..Default::default()
        }];
        color_constraints(&constraints, &mut topo);
        assert!(validate_color_batches(&topo));
        for _ in 0..20 {
            solve_distance_batches(&mut world.curr, &topo, &[], 1.0 / 60.0);
        }
        let pa = world.curr.world_transforms[a as usize].position;
        let pb = world.curr.world_transforms[b as usize].position;
        let dist =
            ((pa[0] - pb[0]).powi(2) + (pa[1] - pb[1]).powi(2) + (pa[2] - pb[2]).powi(2)).sqrt();
        assert!((dist - 1.0).abs() < 1e-3, "dist={dist}");
        assert!(dist.is_finite());
    }

    #[test]
    fn coloring_chain_and_star_disjoint() {
        let mut topo = FlatTopology::default();
        // chain 0-1, 1-2, 2-3
        let chain = [
            XpbdConstraintRow {
                node_a: 0,
                node_b: 1,
                ..Default::default()
            },
            XpbdConstraintRow {
                node_a: 1,
                node_b: 2,
                ..Default::default()
            },
            XpbdConstraintRow {
                node_a: 2,
                node_b: 3,
                ..Default::default()
            },
        ];
        color_constraints(&chain, &mut topo);
        assert!(validate_color_batches(&topo));
        // star: 0-1, 0-2, 0-3 → each own color
        color_constraints(
            &[
                XpbdConstraintRow {
                    node_a: 0,
                    node_b: 1,
                    ..Default::default()
                },
                XpbdConstraintRow {
                    node_a: 0,
                    node_b: 2,
                    ..Default::default()
                },
                XpbdConstraintRow {
                    node_a: 0,
                    node_b: 3,
                    ..Default::default()
                },
            ],
            &mut topo,
        );
        assert!(validate_color_batches(&topo));
        assert!(topo.color_batch_offsets.len() >= 4); // 3 batches + terminal offset
    }

    #[test]
    fn grab_follows_target_then_releases() {
        let mut world = XpbdWorld::new(1);
        let row = dynamic_at(&mut world.curr, [0.0, 0.0, 0.0]);
        world.curr.readiness[row as usize] =
            world.curr.readiness[row as usize].with_bits(ReadinessMask::GRABBED);
        sync_prev_capacity(&mut world);
        world.prev = world.curr.clone();
        let mut topo = FlatTopology::default();
        let mut grabs = Vec::new();
        grab_constraint_rows_into(&world.curr, &mut grabs);
        color_constraints(&grabs, &mut topo);
        solve_distance_batches(
            &mut world.curr,
            &topo,
            &[(row, [5.0, 1.0, 2.0])],
            1.0 / 60.0,
        );
        assert_eq!(
            world.curr.world_transforms[row as usize].position,
            [5.0, 1.0, 2.0]
        );
        world.curr.readiness[row as usize] =
            world.curr.readiness[row as usize].without_bits(ReadinessMask::GRABBED);
        assert!(!world.curr.readiness[row as usize].contains(ReadinessMask::GRABBED));
    }

    #[test]
    fn contact_keeps_sphere_above_floor_estimate() {
        let mut world = XpbdWorld::new(2);
        let ball = dynamic_at(&mut world.curr, [0.0, 0.4, 0.0]);
        let floor = {
            let h = world.curr.alloc_row();
            let mut xf = WorldTransformf64::identity_at([0.0, -1.0, 0.0]);
            xf.inv_mass = 0.0;
            world.curr.set_transform(h.row_id, xf).unwrap();
            world.curr.colliders[h.row_id as usize] = ColliderShape {
                shape_type: ColliderShape::BOX,
                extents: [5.0, 1.0, 5.0],
            };
            world.curr.readiness[h.row_id as usize] = ReadinessMask(
                ReadinessMask::ALLOCATED | ReadinessMask::COLLIDER | ReadinessMask::TRANSFORM,
            );
            h.row_id
        };
        world.curr.colliders[ball as usize] = ColliderShape {
            shape_type: ColliderShape::SPHERE,
            extents: [0.5, 0.0, 0.0],
        };
        world.curr.readiness[ball as usize] =
            world.curr.readiness[ball as usize].with_bits(ReadinessMask::COLLIDER);
        sync_prev_capacity(&mut world);
        world.prev = world.curr.clone();
        let tree = FlatLbvh::build_from_sheet(&world.curr, SceneBounds::default());
        let mut topo = FlatTopology::default();
        let mut contacts = Vec::new();
        contact_constraints_from_lbvh_into(&world.curr, &tree, &mut contacts);
        color_constraints(&contacts, &mut topo);
        for _ in 0..30 {
            solve_distance_batches(&mut world.curr, &topo, &[], 1.0 / 60.0);
        }
        let y = world.curr.world_transforms[ball as usize].position[1];
        let floor_y = world.curr.world_transforms[floor as usize].position[1];
        // radius estimate separation should push ball up relative to deep overlap
        assert!(y.is_finite());
        assert!(y > floor_y);
    }

    #[test]
    fn contact_never_attracts() {
        // Two spheres far apart with an inequality contact at rest=1 must not pull together.
        let mut world = XpbdWorld::new(2);
        let a = dynamic_at(&mut world.curr, [0.0, 0.0, 0.0]);
        let b = dynamic_at(&mut world.curr, [5.0, 0.0, 0.0]);
        sync_prev_capacity(&mut world);
        world.prev = world.curr.clone();
        let mut topo = FlatTopology::default();
        let constraints = [XpbdConstraintRow {
            node_a: a,
            node_b: b,
            target_distance: 1.0,
            flags: XpbdConstraintRow::FLAG_INEQUALITY,
            ..Default::default()
        }];
        color_constraints(&constraints, &mut topo);
        for _ in 0..20 {
            solve_distance_batches(&mut world.curr, &topo, &[], 1.0 / 60.0);
        }
        let pa = world.curr.world_transforms[a as usize].position;
        let pb = world.curr.world_transforms[b as usize].position;
        let dist =
            ((pa[0] - pb[0]).powi(2) + (pa[1] - pb[1]).powi(2) + (pa[2] - pb[2]).powi(2)).sqrt();
        assert!(
            (dist - 5.0).abs() < 1e-6,
            "inequality must not attract: dist={dist}"
        );
    }

    #[test]
    fn contact_never_attracts_via_lbvh_step() {
        // Overlapping spheres: LBVH must emit FLAG_INEQUALITY rows, push apart, then stop
        // (no pull-back once separated).
        let mut world = XpbdWorld::new(2);
        world.gravity = [0.0, 0.0, 0.0];
        world.substeps = 8;
        let a = dynamic_at(&mut world.curr, [-0.1, 0.0, 0.0]);
        let b = dynamic_at(&mut world.curr, [0.1, 0.0, 0.0]);
        let radius = 0.5_f32;
        world.curr.colliders[a as usize] = ColliderShape {
            shape_type: ColliderShape::SPHERE,
            extents: [radius, 0.0, 0.0],
        };
        world.curr.colliders[b as usize] = ColliderShape {
            shape_type: ColliderShape::SPHERE,
            extents: [radius, 0.0, 0.0],
        };
        world.curr.readiness[a as usize] =
            world.curr.readiness[a as usize].with_bits(ReadinessMask::COLLIDER);
        world.curr.readiness[b as usize] =
            world.curr.readiness[b as usize].with_bits(ReadinessMask::COLLIDER);
        sync_prev_capacity(&mut world);
        world.prev = world.curr.clone();

        let bounds = SceneBounds {
            min: [-10.0, -10.0, -10.0],
            max: [10.0, 10.0, 10.0],
        };
        let tree = FlatLbvh::build_from_sheet(&world.curr, bounds);
        let mut contacts = Vec::new();
        contact_constraints_from_lbvh_into(&world.curr, &tree, &mut contacts);
        assert!(
            !contacts.is_empty(),
            "overlapping spheres must produce LBVH contact rows"
        );
        assert!(
            contacts.iter().all(|c| c.is_inequality()),
            "every LBVH contact row must carry FLAG_INEQUALITY"
        );

        let mut topo = FlatTopology::default();
        let dt = 1.0 / 60.0;
        for _ in 0..40 {
            step(&mut world, &mut topo, &[], &[], Some(bounds), dt);
        }
        let pa = world.curr.world_transforms[a as usize].position;
        let pb = world.curr.world_transforms[b as usize].position;
        let dist =
            ((pa[0] - pb[0]).powi(2) + (pa[1] - pb[1]).powi(2) + (pa[2] - pb[2]).powi(2)).sqrt();
        let min_sep = f64::from(radius) * 2.0;
        assert!(
            dist + 1e-3 >= min_sep,
            "must separate overlapping spheres: dist={dist} min_sep={min_sep}"
        );

        // Place exactly at rest with zero frame_delta; inequality must not pull closer.
        {
            let xa = &mut world.curr.world_transforms[a as usize];
            xa.position = [-f64::from(radius), 0.0, 0.0];
            xa.frame_delta_xyz = [0.0, 0.0, 0.0];
            let xb = &mut world.curr.world_transforms[b as usize];
            xb.position = [f64::from(radius), 0.0, 0.0];
            xb.frame_delta_xyz = [0.0, 0.0, 0.0];
        }
        world.prev = world.curr.clone();
        for _ in 0..20 {
            step(&mut world, &mut topo, &[], &[], Some(bounds), dt);
        }
        let pa2 = world.curr.world_transforms[a as usize].position;
        let pb2 = world.curr.world_transforms[b as usize].position;
        let dist2 =
            ((pa2[0] - pb2[0]).powi(2) + (pa2[1] - pb2[1]).powi(2) + (pa2[2] - pb2[2]).powi(2))
                .sqrt();
        assert!(
            dist2 + 1e-4 >= min_sep,
            "resting contact must not attract: dist={dist2} min_sep={min_sep}"
        );
        assert!(
            (dist2 - min_sep).abs() < 1e-3,
            "resting spheres should stay near rest distance: dist={dist2}"
        );
    }

    #[test]
    fn sensor_sphere_emits_no_contact_with_dynamic() {
        // Non-DYNAMIC SPHERE (user_root sensor) × DYNAMIC must yield 0 contact rows.
        let mut sheet = EpochSheet::with_capacity(2);
        let sensor = {
            let h = sheet.alloc_row();
            let mut xf = WorldTransformf64::identity_at([0.0, 0.0, 0.0]);
            xf.inv_mass = 0.0;
            sheet.set_transform(h.row_id, xf).unwrap();
            sheet.colliders[h.row_id as usize] = ColliderShape {
                shape_type: ColliderShape::SPHERE,
                extents: [0.5, 0.0, 0.0],
            };
            sheet.readiness[h.row_id as usize] = ReadinessMask(
                ReadinessMask::ALLOCATED | ReadinessMask::TRANSFORM | ReadinessMask::COLLIDER,
            );
            h.row_id
        };
        let dyn_row = {
            let h = sheet.alloc_row();
            let mut xf = WorldTransformf64::identity_at([0.2, 0.0, 0.0]);
            xf.inv_mass = 1.0;
            sheet.set_transform(h.row_id, xf).unwrap();
            sheet.colliders[h.row_id as usize] = ColliderShape {
                shape_type: ColliderShape::SPHERE,
                extents: [0.5, 0.0, 0.0],
            };
            sheet.readiness[h.row_id as usize] = ReadinessMask(
                ReadinessMask::ALLOCATED
                    | ReadinessMask::TRANSFORM
                    | ReadinessMask::COLLIDER
                    | ReadinessMask::DYNAMIC,
            );
            h.row_id
        };
        let _ = (sensor, dyn_row);
        let tree = FlatLbvh::build_from_sheet(&sheet, SceneBounds::default());
        let mut contacts = Vec::new();
        contact_constraints_from_lbvh_into(&sheet, &tree, &mut contacts);
        assert!(
            contacts.is_empty(),
            "sensor SPHERE must not emit body contact, got {}",
            contacts.len()
        );
    }

    #[test]
    fn static_box_emits_contact_with_dynamic() {
        // Non-DYNAMIC BOX (wall) × DYNAMIC must yield ≥1 contact when overlapping.
        let mut sheet = EpochSheet::with_capacity(2);
        let wall = {
            let h = sheet.alloc_row();
            let mut xf = WorldTransformf64::identity_at([0.0, 0.0, 0.0]);
            xf.inv_mass = 0.0;
            sheet.set_transform(h.row_id, xf).unwrap();
            sheet.colliders[h.row_id as usize] = ColliderShape {
                shape_type: ColliderShape::BOX,
                extents: [0.5, 0.5, 0.5],
            };
            sheet.readiness[h.row_id as usize] = ReadinessMask(
                ReadinessMask::ALLOCATED | ReadinessMask::TRANSFORM | ReadinessMask::COLLIDER,
            );
            h.row_id
        };
        let dyn_row = {
            let h = sheet.alloc_row();
            let mut xf = WorldTransformf64::identity_at([0.2, 0.0, 0.0]);
            xf.inv_mass = 1.0;
            sheet.set_transform(h.row_id, xf).unwrap();
            sheet.colliders[h.row_id as usize] = ColliderShape {
                shape_type: ColliderShape::SPHERE,
                extents: [0.5, 0.0, 0.0],
            };
            sheet.readiness[h.row_id as usize] = ReadinessMask(
                ReadinessMask::ALLOCATED
                    | ReadinessMask::TRANSFORM
                    | ReadinessMask::COLLIDER
                    | ReadinessMask::DYNAMIC,
            );
            h.row_id
        };
        let _ = (wall, dyn_row);
        let tree = FlatLbvh::build_from_sheet(&sheet, SceneBounds::default());
        let mut contacts = Vec::new();
        contact_constraints_from_lbvh_into(&sheet, &tree, &mut contacts);
        assert_eq!(
            contacts.len(),
            1,
            "static BOX × DYNAMIC must emit one contact"
        );
        assert!(contacts[0].is_inequality());
    }

    #[test]
    fn soft_leaves_more_error_than_hard() {
        let mut hard = XpbdWorld::new(2);
        let mut soft = XpbdWorld::new(2);
        for w in [&mut hard, &mut soft] {
            let _a = dynamic_at(&mut w.curr, [0.0, 0.0, 0.0]);
            let _b = dynamic_at(&mut w.curr, [2.0, 0.0, 0.0]);
            sync_prev_capacity(w);
            w.prev = w.curr.clone();
        }
        let mut th = FlatTopology::default();
        let mut ts = FlatTopology::default();
        color_constraints(
            &[XpbdConstraintRow {
                node_a: 0,
                node_b: 1,
                target_distance: 1.0,
                compliance: 0.0,
                ..Default::default()
            }],
            &mut th,
        );
        color_constraints(
            &[XpbdConstraintRow {
                node_a: 0,
                node_b: 1,
                target_distance: 1.0,
                compliance: 0.5,
                ..Default::default()
            }],
            &mut ts,
        );
        for _ in 0..5 {
            solve_distance_batches(&mut hard.curr, &th, &[], 1.0 / 60.0);
            solve_distance_batches(&mut soft.curr, &ts, &[], 1.0 / 60.0);
        }
        let dist = |s: &EpochSheet| {
            let a = s.world_transforms[0].position;
            let b = s.world_transforms[1].position;
            ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
        };
        let eh = (dist(&hard.curr) - 1.0).abs();
        let es = (dist(&soft.curr) - 1.0).abs();
        assert!(es + 1e-6 >= eh, "soft={es} hard={eh}");
        assert!(dist(&soft.curr).is_finite());
    }

    #[test]
    fn sleeping_and_dirty_extraction() {
        let mut world = XpbdWorld::new(1);
        let row = dynamic_at(&mut world.curr, [0.0, 0.0, 0.0]);
        sync_prev_capacity(&mut world);
        world.prev = world.curr.clone();
        world.curr.clear_dirty();
        update_sleeping(&mut world);
        assert!(world.curr.readiness[row as usize].contains(ReadinessMask::SLEEPING));
        // Grab wakes
        world.curr.readiness[row as usize] =
            world.curr.readiness[row as usize].with_bits(ReadinessMask::GRABBED);
        update_sleeping(&mut world);
        assert!(!world.curr.readiness[row as usize].contains(ReadinessMask::SLEEPING));
        world.curr.mark_dirty(row);
        let deltas = extract_dirty_deltas(&world.curr);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].row_id, row);
    }
}
