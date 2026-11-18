//! Flat Morton LBVH + narrow-phase primitives + raycast / capsule extrude.
//!
//! Guarantees: pointer-free 1D spatial index; **O(n) rebuild** via `(morton, leaf_index)`
//! AABB indexing (x00d8); **tick-local refit** when topology is unchanged; collider kinds
//! limited to Sphere / Capsule / Box / Ray (+ StaticSDF Atom stub); readiness COLLIDER bit
//! gates queries. SceneBudget entries ≤ 4096 (x00a2) is the host LBVH sizing ceiling;
//! authority tick is 60 Hz / 16.6 ms with **1 rebuild + (substeps−1) refits** per tick.
//! Build target < 0.2 ms @ thousands is aspirational; wall-clock is measured by
//! `#[ignore]` timing tests, not CI SLA.
//! Non-guarantees: concave mesh GJK/EPA; asset-compiler SDF generation; exact OBB;
//! Morton order freshness across ticks (depends on per-tick rebuild); Karras parallel build;
//! Epoch sheet clone reduction (P1-1).
//!
//! Safety: `#![forbid(unsafe_code)]`.

#![forbid(unsafe_code)]

use crate::tape::{ColliderShape, EpochSheet, ReadinessMask, WorldTransformf64};

/// Axis-aligned bounds in world space (f32 for LBVH nodes).
#[derive(Clone, Copy, Debug, Default)]
pub struct Aabb {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Aabb {
    pub fn from_center_half(center: [f32; 3], half: [f32; 3]) -> Self {
        Self {
            min: [
                center[0] - half[0],
                center[1] - half[1],
                center[2] - half[2],
            ],
            max: [
                center[0] + half[0],
                center[1] + half[1],
                center[2] + half[2],
            ],
        }
    }

    pub fn center(&self) -> [f32; 3] {
        [
            0.5 * (self.min[0] + self.max[0]),
            0.5 * (self.min[1] + self.max[1]),
            0.5 * (self.min[2] + self.max[2]),
        ]
    }

    pub fn merge(self, other: Self) -> Self {
        Self {
            min: [
                self.min[0].min(other.min[0]),
                self.min[1].min(other.min[1]),
                self.min[2].min(other.min[2]),
            ],
            max: [
                self.max[0].max(other.max[0]),
                self.max[1].max(other.max[1]),
                self.max[2].max(other.max[2]),
            ],
        }
    }

    pub fn overlaps(&self, other: &Self) -> bool {
        self.min[0] <= other.max[0]
            && self.max[0] >= other.min[0]
            && self.min[1] <= other.max[1]
            && self.max[1] >= other.min[1]
            && self.min[2] <= other.max[2]
            && self.max[2] >= other.min[2]
    }

    pub fn contains_point(&self, p: [f32; 3]) -> bool {
        p[0] >= self.min[0]
            && p[0] <= self.max[0]
            && p[1] >= self.min[1]
            && p[1] <= self.max[1]
            && p[2] >= self.min[2]
            && p[2] <= self.max[2]
    }
}

/// Scene bounds used to normalize positions into Morton space.
#[derive(Clone, Copy, Debug)]
pub struct SceneBounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Default for SceneBounds {
    fn default() -> Self {
        Self {
            min: [-100.0, -100.0, -100.0],
            max: [100.0, 100.0, 100.0],
        }
    }
}

/// Expand a 10-bit value into 30-bit spaced bits (xxxx -> x00x00x00...).
#[inline]
fn expand_bits(mut v: u32) -> u32 {
    v &= 0x0000_03FF;
    v = (v | (v << 16)) & 0x0300_00FF;
    v = (v | (v << 8)) & 0x0300_F00F;
    v = (v | (v << 4)) & 0x030C_30C3;
    v = (v | (v << 2)) & 0x0924_9249;
    v
}

/// 30-bit Morton code from world position within `bounds`.
pub fn morton_code30(pos: [f32; 3], bounds: SceneBounds) -> u32 {
    let mut codes = [0u32; 3];
    for i in 0..3 {
        let span = (bounds.max[i] - bounds.min[i]).max(1e-6);
        let t = ((pos[i] - bounds.min[i]) / span).clamp(0.0, 1.0);
        codes[i] = (t * 1023.0) as u32;
    }
    expand_bits(codes[0]) | (expand_bits(codes[1]) << 1) | (expand_bits(codes[2]) << 2)
}

/// Deterministic LSD radix sort of `(morton, leaf_index)` (or any `(key, tie)` pair).
/// Key is `morton << 32 | leaf_index` so equal Morton codes break ties by the second
/// field ascending without a follow-up comparison sort (LSD radix is stable).
///
/// Allocates temporary buffers; prefer [`radix_sort_morton_with_scratch`] on hot paths.
pub fn radix_sort_morton(pairs: &mut [(u32, u32)]) {
    let mut keys = Vec::new();
    let mut tmp_keys = Vec::new();
    let mut tmp_pairs = Vec::new();
    radix_sort_morton_with_scratch(pairs, &mut keys, &mut tmp_keys, &mut tmp_pairs);
}

/// Same as [`radix_sort_morton`], reusing `keys` / `tmp_keys` / `tmp_pairs` capacity.
pub fn radix_sort_morton_with_scratch(
    pairs: &mut [(u32, u32)],
    keys: &mut Vec<u64>,
    tmp_keys: &mut Vec<u64>,
    tmp_pairs: &mut Vec<(u32, u32)>,
) {
    if pairs.len() < 2 {
        return;
    }
    keys.clear();
    keys.reserve(pairs.len());
    for &(m, leaf) in pairs.iter() {
        keys.push(((m as u64) << 32) | (leaf as u64));
    }
    tmp_keys.clear();
    tmp_keys.resize(keys.len(), 0);
    tmp_pairs.clear();
    tmp_pairs.resize(pairs.len(), (0, 0));
    for shift in (0..64).step_by(8) {
        let mut counts = [0usize; 256];
        for &k in keys.iter() {
            counts[((k >> shift) & 0xFF) as usize] += 1;
        }
        let mut offsets = [0usize; 256];
        for i in 1..256 {
            offsets[i] = offsets[i - 1] + counts[i - 1];
        }
        for i in 0..keys.len() {
            let bucket = ((keys[i] >> shift) & 0xFF) as usize;
            let slot = offsets[bucket];
            tmp_keys[slot] = keys[i];
            tmp_pairs[slot] = pairs[i];
            offsets[bucket] += 1;
        }
        keys.copy_from_slice(tmp_keys);
        pairs.copy_from_slice(tmp_pairs);
    }
}

/// Flat LBVH node: internal nodes use `left`/`right` child indices; leaves set `row_id`.
#[derive(Clone, Debug)]
pub struct LbvhNode {
    pub aabb: Aabb,
    pub left: u32,
    pub right: u32,
    /// `Some(row)` for leaf; `None` for internal.
    pub row_id: Option<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct FlatLbvh {
    pub nodes: Vec<LbvhNode>,
    /// Root index, or `None` if empty.
    pub root: Option<u32>,
    /// `(morton, row_id)` in Morton-sorted leaf order (debug / consumers).
    pub sorted_leaves: Vec<(u32, u32)>,
    /// Contiguous leaf node indices in Morton order (for O(n) refit leaf walk).
    leaf_node_indices: Vec<u32>,
    /// Rebuild scratch: `(morton, leaf_index)` before/after radix sort.
    scratch_pairs: Vec<(u32, u32)>,
    scratch_leaf_aabbs: Vec<Aabb>,
    scratch_leaf_rows: Vec<u32>,
    /// Pairwise-merge level buffers (reused across rebuilds).
    scratch_level: Vec<u32>,
    scratch_level_next: Vec<u32>,
    /// Radix-sort scratch (reused across rebuilds; see [`radix_sort_morton_with_scratch`]).
    scratch_radix_keys: Vec<u64>,
    scratch_radix_tmp_keys: Vec<u64>,
    scratch_radix_tmp_pairs: Vec<(u32, u32)>,
    /// Test-only call counters (x00d8 regression for rebuild-once / refit-rest).
    #[cfg(test)]
    pub test_rebuild_calls: u32,
    #[cfg(test)]
    pub test_refit_ok_calls: u32,
    #[cfg(test)]
    pub test_refit_fail_calls: u32,
}

impl FlatLbvh {
    /// Allocate a fresh tree (calls [`Self::rebuild_from_sheet`]).
    pub fn build_from_sheet(sheet: &EpochSheet, bounds: SceneBounds) -> Self {
        let mut tree = Self::default();
        tree.rebuild_from_sheet(sheet, bounds);
        tree
    }

    /// Full O(n) rebuild: Morton codes + radix sort + bottom-up pairwise merge.
    /// Reuses `nodes` / sorted leaf buffers / rebuild scratch capacity.
    pub fn rebuild_from_sheet(&mut self, sheet: &EpochSheet, bounds: SceneBounds) {
        #[cfg(test)]
        {
            self.test_rebuild_calls = self.test_rebuild_calls.saturating_add(1);
        }
        self.nodes.clear();
        self.sorted_leaves.clear();
        self.leaf_node_indices.clear();
        self.root = None;
        self.scratch_pairs.clear();
        self.scratch_leaf_aabbs.clear();
        self.scratch_leaf_rows.clear();

        for row in sheet.iter_active_rows() {
            let idx = row as usize;
            if !sheet.readiness[idx].is_ready(ReadinessMask::COLLIDER) {
                continue;
            }
            let aabb = collider_world_aabb(&sheet.world_transforms[idx], &sheet.colliders[idx]);
            let code = morton_code30(aabb.center(), bounds);
            let leaf_index = self.scratch_leaf_rows.len() as u32;
            self.scratch_pairs.push((code, leaf_index));
            self.scratch_leaf_aabbs.push(aabb);
            self.scratch_leaf_rows.push(row);
        }
        // Sort pairs with resident radix scratch (avoid per-rebuild keys/tmp alloc).
        {
            let pairs = &mut self.scratch_pairs;
            let keys = &mut self.scratch_radix_keys;
            let tmp_keys = &mut self.scratch_radix_tmp_keys;
            let tmp_pairs = &mut self.scratch_radix_tmp_pairs;
            radix_sort_morton_with_scratch(pairs, keys, tmp_keys, tmp_pairs);
        }
        self.finish_build_from_scratch();
    }

    fn finish_build_from_scratch(&mut self) {
        if self.scratch_pairs.is_empty() {
            return;
        }

        self.scratch_level.clear();
        self.scratch_level.reserve(self.scratch_pairs.len());
        let pair_len = self.scratch_pairs.len();
        for i in 0..pair_len {
            let (code, leaf_index) = self.scratch_pairs[i];
            let li = leaf_index as usize;
            let row = self.scratch_leaf_rows[li];
            let aabb = self.scratch_leaf_aabbs[li];
            let idx = self.nodes.len() as u32;
            self.nodes.push(LbvhNode {
                aabb,
                left: u32::MAX,
                right: u32::MAX,
                row_id: Some(row),
            });
            self.leaf_node_indices.push(idx);
            self.sorted_leaves.push((code, row));
            self.scratch_level.push(idx);
        }

        // Bottom-up pairwise merge (simple flat LBVH).
        // Invariant: every parent index is strictly greater than both child indices
        // (parents are always appended after children).
        while self.scratch_level.len() > 1 {
            self.scratch_level_next.clear();
            self.scratch_level_next
                .reserve(self.scratch_level.len().div_ceil(2));
            let mut i = 0;
            while i < self.scratch_level.len() {
                if i + 1 == self.scratch_level.len() {
                    self.scratch_level_next.push(self.scratch_level[i]);
                    break;
                }
                let left = self.scratch_level[i];
                let right = self.scratch_level[i + 1];
                let aabb = self.nodes[left as usize]
                    .aabb
                    .merge(self.nodes[right as usize].aabb);
                let parent = self.nodes.len() as u32;
                debug_assert!(left < parent && right < parent);
                self.nodes.push(LbvhNode {
                    aabb,
                    left,
                    right,
                    row_id: None,
                });
                self.scratch_level_next.push(parent);
                i += 2;
            }
            std::mem::swap(&mut self.scratch_level, &mut self.scratch_level_next);
        }

        self.root = self.scratch_level.first().copied();
    }

    /// Update leaf AABBs from `sheet` and re-merge internals in place (topology fixed).
    ///
    /// Returns `false` if leaf count / row readiness no longer match this tree
    /// (caller should [`Self::rebuild_from_sheet`]). **On `false`, the tree is left
    /// unmodified** — all validation runs before any leaf AABB write. Intended caller
    /// today is [`crate::xpbd::step`] only; other call sites must honor the same contract.
    pub fn refit_from_sheet(&mut self, sheet: &EpochSheet) -> bool {
        let leaf_count = self.leaf_node_indices.len();
        let mut sheet_colliders = 0usize;
        for row in sheet.iter_active_rows() {
            if sheet.readiness[row as usize].is_ready(ReadinessMask::COLLIDER) {
                sheet_colliders += 1;
            }
        }
        if sheet_colliders != leaf_count {
            #[cfg(test)]
            {
                self.test_refit_fail_calls = self.test_refit_fail_calls.saturating_add(1);
            }
            return false;
        }
        if leaf_count == 0 {
            let ok = self.root.is_none();
            #[cfg(test)]
            {
                if ok {
                    self.test_refit_ok_calls = self.test_refit_ok_calls.saturating_add(1);
                } else {
                    self.test_refit_fail_calls = self.test_refit_fail_calls.saturating_add(1);
                }
            }
            return ok;
        }
        // Validate every leaf before mutating any AABB (fail-closed / no partial update).
        for &node_idx in &self.leaf_node_indices {
            let Some(row) = self.nodes[node_idx as usize].row_id else {
                #[cfg(test)]
                {
                    self.test_refit_fail_calls = self.test_refit_fail_calls.saturating_add(1);
                }
                return false;
            };
            let idx = row as usize;
            if idx >= sheet.len() || !sheet.readiness[idx].is_ready(ReadinessMask::COLLIDER) {
                #[cfg(test)]
                {
                    self.test_refit_fail_calls = self.test_refit_fail_calls.saturating_add(1);
                }
                return false;
            }
        }
        for &node_idx in &self.leaf_node_indices {
            let row = self.nodes[node_idx as usize]
                .row_id
                .expect("leaf row_id validated above");
            let idx = row as usize;
            self.nodes[node_idx as usize].aabb =
                collider_world_aabb(&sheet.world_transforms[idx], &sheet.colliders[idx]);
        }
        // Parents are always appended after children during build, so ascending
        // index order is a valid bottom-up pass.
        for i in 0..self.nodes.len() {
            if self.nodes[i].row_id.is_some() {
                continue;
            }
            let left = self.nodes[i].left as usize;
            let right = self.nodes[i].right as usize;
            debug_assert!(
                left < i && right < i,
                "refit requires parent index > child indices (build invariant)"
            );
            let merged = self.nodes[left].aabb.merge(self.nodes[right].aabb);
            self.nodes[i].aabb = merged;
        }
        #[cfg(test)]
        {
            self.test_refit_ok_calls = self.test_refit_ok_calls.saturating_add(1);
        }
        true
    }
}

pub fn collider_world_aabb(xf: &WorldTransformf64, shape: &ColliderShape) -> Aabb {
    let c = [
        xf.position[0] as f32,
        xf.position[1] as f32,
        xf.position[2] as f32,
    ];
    match shape.shape_type {
        ColliderShape::SPHERE => {
            let r = shape.extents[0];
            Aabb::from_center_half(c, [r, r, r])
        }
        ColliderShape::CAPSULE => {
            let r = shape.extents[0];
            let hh = shape.extents[1];
            Aabb::from_center_half(c, [r, hh + r, r])
        }
        ColliderShape::BOX => Aabb::from_center_half(c, shape.extents),
        ColliderShape::STATIC_SDF_ATOM => {
            // Stub: treat as large static box until Atom SDF is hydrated.
            Aabb::from_center_half(c, [shape.extents[0], shape.extents[1], shape.extents[2]])
        }
        _ => Aabb::from_center_half(c, [0.0, 0.0, 0.0]),
    }
}

/// Ray vs AABB (slab method). Returns entry t if hit.
pub fn ray_aabb(origin: [f32; 3], dir: [f32; 3], aabb: &Aabb) -> Option<f32> {
    let mut tmin = 0.0f32;
    let mut tmax = f32::MAX;
    for i in 0..3 {
        if dir[i].abs() < 1e-8 {
            if origin[i] < aabb.min[i] || origin[i] > aabb.max[i] {
                return None;
            }
            continue;
        }
        let inv = 1.0 / dir[i];
        let mut t0 = (aabb.min[i] - origin[i]) * inv;
        let mut t1 = (aabb.max[i] - origin[i]) * inv;
        if t0 > t1 {
            std::mem::swap(&mut t0, &mut t1);
        }
        tmin = tmin.max(t0);
        tmax = tmax.min(t1);
        if tmin > tmax {
            return None;
        }
    }
    if tmax < 0.0 {
        None
    } else {
        Some(tmin.max(0.0))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    pub row_id: u32,
    pub t: f32,
    pub point: [f32; 3],
}

/// Narrow-phase ray vs collider (Sphere / Capsule / Box). Returns hit t.
pub fn ray_collider(
    origin: [f32; 3],
    dir: [f32; 3],
    xf: &WorldTransformf64,
    shape: &ColliderShape,
) -> Option<f32> {
    let c = [
        xf.position[0] as f32,
        xf.position[1] as f32,
        xf.position[2] as f32,
    ];
    match shape.shape_type {
        ColliderShape::SPHERE => ray_sphere(origin, dir, c, shape.extents[0]),
        ColliderShape::CAPSULE => {
            let hh = shape.extents[1];
            let r = shape.extents[0];
            let a = [c[0], c[1] - hh, c[2]];
            let b = [c[0], c[1] + hh, c[2]];
            ray_capsule(origin, dir, a, b, r)
        }
        ColliderShape::BOX => {
            let aabb = Aabb::from_center_half(c, shape.extents);
            ray_aabb(origin, dir, &aabb)
        }
        ColliderShape::STATIC_SDF_ATOM => {
            let aabb = Aabb::from_center_half(c, shape.extents);
            ray_aabb(origin, dir, &aabb)
        }
        _ => None,
    }
}

fn ray_sphere(origin: [f32; 3], dir: [f32; 3], center: [f32; 3], radius: f32) -> Option<f32> {
    let oc = [
        origin[0] - center[0],
        origin[1] - center[1],
        origin[2] - center[2],
    ];
    let a = dot(dir, dir);
    let b = 2.0 * dot(oc, dir);
    let c = dot(oc, oc) - radius * radius;
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 || a == 0.0 {
        return None;
    }
    let s = disc.sqrt();
    let t0 = (-b - s) / (2.0 * a);
    let t1 = (-b + s) / (2.0 * a);
    if t0 >= 0.0 {
        Some(t0)
    } else if t1 >= 0.0 {
        Some(t1)
    } else {
        None
    }
}

fn ray_capsule(
    origin: [f32; 3],
    dir: [f32; 3],
    a: [f32; 3],
    b: [f32; 3],
    radius: f32,
) -> Option<f32> {
    // Exact: end-cap spheres + finite cylinder (segment-clamped). No axis sampling.
    let mut best = min_hit(
        ray_sphere(origin, dir, a, radius),
        ray_sphere(origin, dir, b, radius),
    );
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ab_len2 = dot(ab, ab);
    if ab_len2 < 1e-12 {
        return best;
    }
    let ao = [origin[0] - a[0], origin[1] - a[1], origin[2] - a[2]];
    // |(ao + t dir) × ab|^2 = r² |ab|²
    let aoxab = cross(ao, ab);
    let dxab = cross(dir, ab);
    let qa = dot(dxab, dxab);
    let qb = 2.0 * dot(aoxab, dxab);
    let qc = dot(aoxab, aoxab) - radius * radius * ab_len2;
    if qa > 1e-12 {
        let disc = qb * qb - 4.0 * qa * qc;
        if disc >= 0.0 {
            let s = disc.sqrt();
            for &t in &[(-qb - s) / (2.0 * qa), (-qb + s) / (2.0 * qa)] {
                if t < 0.0 {
                    continue;
                }
                let p = [
                    origin[0] + dir[0] * t,
                    origin[1] + dir[1] * t,
                    origin[2] + dir[2] * t,
                ];
                let ap = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
                let u = dot(ap, ab) / ab_len2;
                if u > 0.0 && u < 1.0 {
                    best = min_hit(best, Some(t));
                }
            }
        }
    }
    best
}

#[inline]
fn min_hit(a: Option<f32>, b: Option<f32>) -> Option<f32> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

#[inline]
fn cross(u: [f32; 3], v: [f32; 3]) -> [f32; 3] {
    [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ]
}

#[inline]
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn raycast(
    lbvh: &FlatLbvh,
    sheet: &EpochSheet,
    origin: [f32; 3],
    dir: [f32; 3],
    max_t: f32,
) -> Option<RayHit> {
    let root = lbvh.root?;
    let mut stack = vec![root];
    let mut best: Option<RayHit> = None;
    while let Some(ni) = stack.pop() {
        let node = &lbvh.nodes[ni as usize];
        let Some(t_box) = ray_aabb(origin, dir, &node.aabb) else {
            continue;
        };
        if t_box > max_t {
            continue;
        }
        if let Some(best_hit) = &best {
            if t_box > best_hit.t {
                continue;
            }
        }
        if let Some(row) = node.row_id {
            let idx = row as usize;
            if idx >= sheet.readiness.len() {
                continue;
            }
            if !sheet.readiness[idx].is_ready(ReadinessMask::COLLIDER) {
                continue;
            }
            if let Some(t) = ray_collider(
                origin,
                dir,
                &sheet.world_transforms[idx],
                &sheet.colliders[idx],
            ) {
                if t >= 0.0 && t <= max_t && best.as_ref().map(|h| t < h.t).unwrap_or(true) {
                    best = Some(RayHit {
                        row_id: row,
                        t,
                        point: [
                            origin[0] + dir[0] * t,
                            origin[1] + dir[1] * t,
                            origin[2] + dir[2] * t,
                        ],
                    });
                }
            }
        } else {
            if node.left != u32::MAX {
                stack.push(node.left);
            }
            if node.right != u32::MAX {
                stack.push(node.right);
            }
        }
    }
    best
}

/// Collect overlapping leaf pairs via LBVH traversal (not leaf O(n²)).
pub fn collect_overlap_pairs(lbvh: &FlatLbvh) -> Vec<(u32, u32)> {
    let mut pairs = Vec::new();
    let Some(root) = lbvh.root else {
        return pairs;
    };
    collect_self_overlaps(lbvh, root, &mut pairs);
    pairs
}

fn collect_self_overlaps(lbvh: &FlatLbvh, node: u32, pairs: &mut Vec<(u32, u32)>) {
    let n = &lbvh.nodes[node as usize];
    if n.row_id.is_some() {
        return;
    }
    let left = n.left;
    let right = n.right;
    if left != u32::MAX {
        collect_self_overlaps(lbvh, left, pairs);
    }
    if right != u32::MAX {
        collect_self_overlaps(lbvh, right, pairs);
    }
    if left != u32::MAX && right != u32::MAX {
        collect_cross_overlaps(lbvh, left, right, pairs);
    }
}

fn collect_cross_overlaps(lbvh: &FlatLbvh, a: u32, b: u32, pairs: &mut Vec<(u32, u32)>) {
    let na = &lbvh.nodes[a as usize];
    let nb = &lbvh.nodes[b as usize];
    if !na.aabb.overlaps(&nb.aabb) {
        return;
    }
    match (na.row_id, nb.row_id) {
        (Some(ra), Some(rb)) if ra != rb => {
            pairs.push((ra.min(rb), ra.max(rb)));
        }
        (Some(_), None) => {
            if nb.left != u32::MAX {
                collect_cross_overlaps(lbvh, a, nb.left, pairs);
            }
            if nb.right != u32::MAX {
                collect_cross_overlaps(lbvh, a, nb.right, pairs);
            }
        }
        (None, Some(_)) => {
            if na.left != u32::MAX {
                collect_cross_overlaps(lbvh, na.left, b, pairs);
            }
            if na.right != u32::MAX {
                collect_cross_overlaps(lbvh, na.right, b, pairs);
            }
        }
        (None, None) => {
            if na.left != u32::MAX {
                if nb.left != u32::MAX {
                    collect_cross_overlaps(lbvh, na.left, nb.left, pairs);
                }
                if nb.right != u32::MAX {
                    collect_cross_overlaps(lbvh, na.left, nb.right, pairs);
                }
            }
            if na.right != u32::MAX {
                if nb.left != u32::MAX {
                    collect_cross_overlaps(lbvh, na.right, nb.left, pairs);
                }
                if nb.right != u32::MAX {
                    collect_cross_overlaps(lbvh, na.right, nb.right, pairs);
                }
            }
        }
        _ => {}
    }
}

/// Capsule character extrude against static boxes/spheres (position-based).
pub fn extrude_capsule(sheet: &EpochSheet, capsule_row: u32, iterations: u32) -> Option<[f64; 3]> {
    let idx = capsule_row as usize;
    if idx >= sheet.colliders.len() {
        return None;
    }
    if sheet.colliders[idx].shape_type != ColliderShape::CAPSULE {
        return None;
    }
    let r = sheet.colliders[idx].extents[0] as f64;
    let hh = sheet.colliders[idx].extents[1] as f64;
    let mut pos = sheet.world_transforms[idx].position;
    for _ in 0..iterations {
        for other in sheet.iter_active_rows() {
            if other == capsule_row {
                continue;
            }
            let oi = other as usize;
            if !sheet.readiness[oi].is_ready(ReadinessMask::COLLIDER) {
                continue;
            }
            let opos = sheet.world_transforms[oi].position;
            let shape = &sheet.colliders[oi];
            match shape.shape_type {
                ColliderShape::BOX => {
                    let hx = shape.extents[0] as f64;
                    let hy = shape.extents[1] as f64;
                    let hz = shape.extents[2] as f64;
                    // Expand box by capsule radius; push out of AABB.
                    let min = [opos[0] - hx - r, opos[1] - hy - r - hh, opos[2] - hz - r];
                    let max = [opos[0] + hx + r, opos[1] + hy + r + hh, opos[2] + hz + r];
                    if pos[0] >= min[0]
                        && pos[0] <= max[0]
                        && pos[1] >= min[1]
                        && pos[1] <= max[1]
                        && pos[2] >= min[2]
                        && pos[2] <= max[2]
                    {
                        let dx = (pos[0] - opos[0]).abs();
                        let dy = (pos[1] - opos[1]).abs();
                        let dz = (pos[2] - opos[2]).abs();
                        let px = (hx + r) - dx;
                        let py = (hy + r + hh) - dy;
                        let pz = (hz + r) - dz;
                        if px <= py && px <= pz {
                            pos[0] += if pos[0] >= opos[0] { px } else { -px };
                        } else if py <= px && py <= pz {
                            pos[1] += if pos[1] >= opos[1] { py } else { -py };
                        } else {
                            pos[2] += if pos[2] >= opos[2] { pz } else { -pz };
                        }
                    }
                }
                ColliderShape::SPHERE => {
                    let sr = shape.extents[0] as f64 + r;
                    let d = [pos[0] - opos[0], pos[1] - opos[1], pos[2] - opos[2]];
                    let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                    if dist < sr && dist > 1e-8 {
                        let push = (sr - dist) / dist;
                        pos[0] += d[0] * push;
                        pos[1] += d[1] * push;
                        pos[2] += d[2] * push;
                    }
                }
                _ => {}
            }
        }
    }
    if pos.iter().all(|v| v.is_finite()) {
        Some(pos)
    } else {
        None
    }
}

/// Static SDF Atom handle stub (blake3 of precomputed field). Runtime O(1) ref only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StaticSdfAtomRef {
    pub blake3: [u8; 32],
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::{ColliderShape, EpochSheet, ReadinessMask, WorldTransformf64};

    #[test]
    fn morton_nearby_points_close_in_code_space() {
        let b = SceneBounds::default();
        let a = morton_code30([0.0, 0.0, 0.0], b);
        let c = morton_code30([0.1, 0.0, 0.0], b);
        let far = morton_code30([50.0, 50.0, 50.0], b);
        let da = a.abs_diff(c);
        let df = a.abs_diff(far);
        assert!(da < df);
    }

    #[test]
    fn morton_golden_origin() {
        let b = SceneBounds {
            min: [0.0, 0.0, 0.0],
            max: [1.0, 1.0, 1.0],
        };
        // center of cell 0
        assert_eq!(morton_code30([0.0, 0.0, 0.0], b), 0);
    }

    #[test]
    fn radix_sort_deterministic_ties() {
        let mut pairs = vec![(5, 3), (1, 9), (5, 1), (5, 2), (1, 2)];
        radix_sort_morton(&mut pairs);
        assert_eq!(pairs, vec![(1, 2), (1, 9), (5, 1), (5, 2), (5, 3)]);
    }

    #[test]
    fn lbvh_root_contains_leaves() {
        let mut sheet = EpochSheet::with_capacity(2);
        let a = sheet.alloc_row();
        let b = sheet.alloc_row();
        sheet
            .set_transform(a.row_id, WorldTransformf64::identity_at([-1.0, 0.0, 0.0]))
            .unwrap();
        sheet
            .set_transform(b.row_id, WorldTransformf64::identity_at([1.0, 0.0, 0.0]))
            .unwrap();
        sheet.colliders[0] = ColliderShape {
            shape_type: ColliderShape::SPHERE,
            extents: [0.5, 0.0, 0.0],
        };
        sheet.colliders[1] = ColliderShape {
            shape_type: ColliderShape::SPHERE,
            extents: [0.5, 0.0, 0.0],
        };
        sheet.readiness[0] = ReadinessMask(ReadinessMask::ALLOCATED | ReadinessMask::COLLIDER);
        sheet.readiness[1] = ReadinessMask(ReadinessMask::ALLOCATED | ReadinessMask::COLLIDER);
        let tree = FlatLbvh::build_from_sheet(&sheet, SceneBounds::default());
        let root = tree.root.unwrap() as usize;
        assert!(tree.nodes[root].aabb.min[0] <= -1.5);
        assert!(tree.nodes[root].aabb.max[0] >= 1.5);
    }

    #[test]
    fn ray_sphere_hit_and_miss() {
        let xf = WorldTransformf64::identity_at([0.0, 0.0, 0.0]);
        let shape = ColliderShape {
            shape_type: ColliderShape::SPHERE,
            extents: [1.0, 0.0, 0.0],
        };
        assert!(ray_collider([-5.0, 0.0, 0.0], [1.0, 0.0, 0.0], &xf, &shape).is_some());
        assert!(ray_collider([-5.0, 3.0, 0.0], [1.0, 0.0, 0.0], &xf, &shape).is_none());
    }

    #[test]
    fn ray_capsule_hits_midshaft_not_only_caps() {
        // Vertical capsule; ray at y=0 midshaft — exact cylinder (not axis samples).
        let hit = ray_capsule(
            [-5.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 1.0, 0.0],
            0.5,
        );
        assert!(hit.is_some());
        assert!((hit.unwrap() - 4.5).abs() < 1e-3);
        assert!(ray_capsule(
            [-5.0, 5.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 1.0, 0.0],
            0.5,
        )
        .is_none());
    }

    #[test]
    fn raycast_respects_collider_bit_and_occlusion() {
        let mut sheet = EpochSheet::with_capacity(2);
        let near = sheet.alloc_row();
        let far = sheet.alloc_row();
        sheet
            .set_transform(near.row_id, WorldTransformf64::identity_at([0.0, 0.0, 0.0]))
            .unwrap();
        sheet
            .set_transform(far.row_id, WorldTransformf64::identity_at([3.0, 0.0, 0.0]))
            .unwrap();
        for i in 0..2 {
            sheet.colliders[i] = ColliderShape {
                shape_type: ColliderShape::SPHERE,
                extents: [0.5, 0.0, 0.0],
            };
            sheet.readiness[i] = ReadinessMask(ReadinessMask::ALLOCATED | ReadinessMask::COLLIDER);
        }
        let tree = FlatLbvh::build_from_sheet(&sheet, SceneBounds::default());
        let hit = raycast(&tree, &sheet, [-5.0, 0.0, 0.0], [1.0, 0.0, 0.0], 100.0).unwrap();
        assert_eq!(hit.row_id, near.row_id);

        // Disable near collider
        sheet.readiness[near.row_id as usize] = ReadinessMask(ReadinessMask::ALLOCATED);
        let tree2 = FlatLbvh::build_from_sheet(&sheet, SceneBounds::default());
        let hit2 = raycast(&tree2, &sheet, [-5.0, 0.0, 0.0], [1.0, 0.0, 0.0], 100.0).unwrap();
        assert_eq!(hit2.row_id, far.row_id);
    }

    #[test]
    fn lbvh_overlap_pairs_find_neighbors_not_all_pairs() {
        let mut sheet = EpochSheet::with_capacity(3);
        let a = sheet.alloc_row();
        let b = sheet.alloc_row();
        let c = sheet.alloc_row();
        sheet
            .set_transform(a.row_id, WorldTransformf64::identity_at([0.0, 0.0, 0.0]))
            .unwrap();
        sheet
            .set_transform(b.row_id, WorldTransformf64::identity_at([0.5, 0.0, 0.0]))
            .unwrap();
        sheet
            .set_transform(c.row_id, WorldTransformf64::identity_at([50.0, 0.0, 0.0]))
            .unwrap();
        for i in 0..3 {
            sheet.colliders[i] = ColliderShape {
                shape_type: ColliderShape::SPHERE,
                extents: [0.4, 0.0, 0.0],
            };
            sheet.readiness[i] = ReadinessMask(ReadinessMask::ALLOCATED | ReadinessMask::COLLIDER);
        }
        let tree = FlatLbvh::build_from_sheet(&sheet, SceneBounds::default());
        let pairs = collect_overlap_pairs(&tree);
        assert!(pairs.contains(&(a.row_id.min(b.row_id), a.row_id.max(b.row_id))));
        assert!(!pairs.iter().any(|&(x, y)| {
            (x == a.row_id && y == c.row_id) || (x == b.row_id && y == c.row_id)
        }));
    }

    #[test]
    fn capsule_extrude_out_of_box() {
        let mut sheet = EpochSheet::with_capacity(2);
        let cap = sheet.alloc_row();
        let wall = sheet.alloc_row();
        sheet
            .set_transform(cap.row_id, WorldTransformf64::identity_at([0.0, 0.0, 0.0]))
            .unwrap();
        sheet
            .set_transform(wall.row_id, WorldTransformf64::identity_at([0.0, 0.0, 0.0]))
            .unwrap();
        sheet.colliders[cap.row_id as usize] = ColliderShape {
            shape_type: ColliderShape::CAPSULE,
            extents: [0.3, 0.5, 0.0],
        };
        sheet.colliders[wall.row_id as usize] = ColliderShape {
            shape_type: ColliderShape::BOX,
            extents: [1.0, 1.0, 1.0],
        };
        sheet.readiness[cap.row_id as usize] =
            ReadinessMask(ReadinessMask::ALLOCATED | ReadinessMask::COLLIDER);
        sheet.readiness[wall.row_id as usize] =
            ReadinessMask(ReadinessMask::ALLOCATED | ReadinessMask::COLLIDER);
        let pos = extrude_capsule(&sheet, cap.row_id, 8).unwrap();
        // Must be pushed outside expanded box.
        assert!(pos[0].abs() >= 1.0 || pos[1].abs() >= 1.0 || pos[2].abs() >= 1.0);
    }

    fn sheet_with_n_spheres(n: usize, spacing: f64) -> EpochSheet {
        let mut sheet = EpochSheet::with_capacity(n);
        for i in 0..n {
            let h = sheet.alloc_row();
            let x = (i as f64) * spacing;
            sheet
                .set_transform(h.row_id, WorldTransformf64::identity_at([x, 0.0, 0.0]))
                .unwrap();
            sheet.colliders[h.row_id as usize] = ColliderShape {
                shape_type: ColliderShape::SPHERE,
                extents: [0.4, 0.0, 0.0],
            };
            sheet.readiness[h.row_id as usize] =
                ReadinessMask(ReadinessMask::ALLOCATED | ReadinessMask::COLLIDER);
        }
        sheet
    }

    #[test]
    fn lbvh_refit_tracks_leaf_motion() {
        let mut sheet = sheet_with_n_spheres(2, 2.0);
        let bounds = SceneBounds::default();
        let mut tree = FlatLbvh::build_from_sheet(&sheet, bounds);
        let root_before = tree.nodes[tree.root.unwrap() as usize].aabb;

        sheet
            .set_transform(0, WorldTransformf64::identity_at([10.0, 0.0, 0.0]))
            .unwrap();
        sheet
            .set_transform(1, WorldTransformf64::identity_at([10.5, 0.0, 0.0]))
            .unwrap();
        assert!(tree.refit_from_sheet(&sheet));
        let root_after = tree.nodes[tree.root.unwrap() as usize].aabb;
        assert!(root_after.min[0] > root_before.min[0]);
        assert!(root_after.max[0] > root_before.max[0]);
        let pairs = collect_overlap_pairs(&tree);
        assert!(pairs.contains(&(0, 1)));
    }

    #[test]
    fn lbvh_refit_rejects_topology_change() {
        let mut sheet = sheet_with_n_spheres(2, 2.0);
        let mut tree = FlatLbvh::build_from_sheet(&sheet, SceneBounds::default());
        // Drop collider readiness on leaf 0 → count mismatch.
        sheet.readiness[0] = ReadinessMask(ReadinessMask::ALLOCATED);
        assert!(!tree.refit_from_sheet(&sheet));

        // Grow leaf set → count mismatch.
        sheet = sheet_with_n_spheres(2, 2.0);
        tree.rebuild_from_sheet(&sheet, SceneBounds::default());
        let extra = sheet.alloc_row();
        sheet
            .set_transform(
                extra.row_id,
                WorldTransformf64::identity_at([4.0, 0.0, 0.0]),
            )
            .unwrap();
        sheet.colliders[extra.row_id as usize] = ColliderShape {
            shape_type: ColliderShape::SPHERE,
            extents: [0.4, 0.0, 0.0],
        };
        sheet.readiness[extra.row_id as usize] =
            ReadinessMask(ReadinessMask::ALLOCATED | ReadinessMask::COLLIDER);
        assert!(!tree.refit_from_sheet(&sheet));
    }

    #[test]
    fn lbvh_rebuild_reuses_capacity() {
        let sheet = sheet_with_n_spheres(8, 1.0);
        let mut tree = FlatLbvh::build_from_sheet(&sheet, SceneBounds::default());
        let cap = tree.nodes.capacity();
        let pairs_cap = tree.scratch_pairs.capacity();
        let radix_cap = tree.scratch_radix_keys.capacity();
        tree.rebuild_from_sheet(&sheet, SceneBounds::default());
        assert!(tree.nodes.capacity() >= cap);
        assert!(tree.scratch_pairs.capacity() >= pairs_cap);
        assert!(tree.scratch_radix_keys.capacity() >= radix_cap);
        assert_eq!(tree.leaf_node_indices.len(), 8);
    }

    fn assert_aabbs_unchanged(tree: &FlatLbvh, before: &[Aabb]) {
        assert_eq!(tree.nodes.len(), before.len());
        for (i, node) in tree.nodes.iter().enumerate() {
            assert_eq!(node.aabb.min, before[i].min);
            assert_eq!(node.aabb.max, before[i].max);
        }
    }

    #[test]
    fn lbvh_refit_fail_leaves_tree_unmodified() {
        // Path A: sheet collider count ≠ leaf_count (early return).
        let mut sheet = sheet_with_n_spheres(2, 2.0);
        let mut tree = FlatLbvh::build_from_sheet(&sheet, SceneBounds::default());
        let before: Vec<Aabb> = tree.nodes.iter().map(|n| n.aabb).collect();
        sheet.readiness[0] = ReadinessMask(ReadinessMask::ALLOCATED);
        assert!(!tree.refit_from_sheet(&sheet));
        assert_aabbs_unchanged(&tree, &before);

        // Path B: counts match, but a tree leaf's row lost COLLIDER (validate loop).
        // Swap COLLIDER from row 0 onto a new row so sheet_colliders == leaf_count == 2.
        sheet = sheet_with_n_spheres(2, 2.0);
        tree.rebuild_from_sheet(&sheet, SceneBounds::default());
        let before: Vec<Aabb> = tree.nodes.iter().map(|n| n.aabb).collect();
        sheet.readiness[0] = ReadinessMask(ReadinessMask::ALLOCATED);
        let extra = sheet.alloc_row();
        sheet
            .set_transform(
                extra.row_id,
                WorldTransformf64::identity_at([4.0, 0.0, 0.0]),
            )
            .unwrap();
        sheet.colliders[extra.row_id as usize] = ColliderShape {
            shape_type: ColliderShape::SPHERE,
            extents: [0.4, 0.0, 0.0],
        };
        sheet.readiness[extra.row_id as usize] =
            ReadinessMask(ReadinessMask::ALLOCATED | ReadinessMask::COLLIDER);
        let mut sheet_colliders = 0usize;
        for row in sheet.iter_active_rows() {
            if sheet.readiness[row as usize].is_ready(ReadinessMask::COLLIDER) {
                sheet_colliders += 1;
            }
        }
        assert_eq!(sheet_colliders, tree.leaf_node_indices.len());
        assert!(!tree.refit_from_sheet(&sheet));
        assert_aabbs_unchanged(&tree, &before);
    }

    /// Manual: `cargo test -p alchemy-shared-sim lbvh_build_timing -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn lbvh_build_timing_1024_4096() {
        use std::time::Instant;
        let bounds = SceneBounds::default();
        let mut rebuild_ms_by_n = [0.0f64; 2];
        for (slot, &n) in [1024usize, 4096usize].iter().enumerate() {
            let mut sheet = sheet_with_n_spheres(n, 0.5);
            let mut tree = FlatLbvh::default();
            let t0 = Instant::now();
            tree.rebuild_from_sheet(&sheet, bounds);
            let rebuild_ms = t0.elapsed().as_secs_f64() * 1000.0;
            rebuild_ms_by_n[slot] = rebuild_ms;

            for i in 0..n {
                let p = sheet.world_transforms[i].position;
                sheet
                    .set_transform(
                        i as u32,
                        WorldTransformf64::identity_at([p[0] + 0.01, p[1], p[2]]),
                    )
                    .unwrap();
            }
            let t1 = Instant::now();
            assert!(tree.refit_from_sheet(&sheet));
            let refit_ms = t1.elapsed().as_secs_f64() * 1000.0;

            eprintln!("n={n}: rebuild={rebuild_ms:.3}ms refit={refit_ms:.3}ms");
            // Absolute: generous for debug/slow hosts, but far below O(n²) at 4096.
            assert!(
                rebuild_ms < 200.0,
                "rebuild n={n} took {rebuild_ms:.1}ms (expected O(n))"
            );
            assert!(
                refit_ms <= rebuild_ms * 2.0 + 1.0,
                "refit slower than rebuild unexpectedly"
            );
        }
        // Scaling: 4× n should not approach 16× time (O(n²)). Allow 10× headroom for noise.
        let ratio = rebuild_ms_by_n[1] / rebuild_ms_by_n[0].max(1e-9);
        eprintln!("rebuild scale 4096/1024 = {ratio:.2}×");
        assert!(
            ratio < 10.0,
            "rebuild time scaled {ratio:.1}× for 4× n (O(n²) would be ~16×)"
        );
    }
}
