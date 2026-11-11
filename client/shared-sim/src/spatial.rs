//! Flat Morton LBVH + narrow-phase primitives + raycast / capsule extrude.
//!
//! Guarantees: pointer-free 1D spatial index; collider kinds limited to Sphere /
//! Capsule / Box / Ray (+ StaticSDF Atom stub); readiness COLLIDER bit gates queries.
//! Non-guarantees: concave mesh GJK/EPA; asset-compiler SDF generation.
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

/// Deterministic LSD radix sort of `(morton, row_id)`.
/// Key is `morton << 32 | row_id` so equal Morton codes break ties by `row_id` ascending
/// without a follow-up comparison sort (LSD radix is stable).
pub fn radix_sort_morton(pairs: &mut [(u32, u32)]) {
    if pairs.len() < 2 {
        return;
    }
    let mut keys: Vec<u64> = pairs
        .iter()
        .map(|&(m, r)| ((m as u64) << 32) | (r as u64))
        .collect();
    let mut tmp_keys = vec![0u64; keys.len()];
    let mut tmp_pairs = vec![(0u32, 0u32); pairs.len()];
    for shift in (0..64).step_by(8) {
        let mut counts = [0usize; 256];
        for &k in &keys {
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
        keys.copy_from_slice(&tmp_keys);
        pairs.copy_from_slice(&tmp_pairs);
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
    pub sorted_leaves: Vec<(u32, u32)>,
}

impl FlatLbvh {
    pub fn build_from_sheet(sheet: &EpochSheet, bounds: SceneBounds) -> Self {
        let mut pairs = Vec::new();
        let mut leaf_aabbs = Vec::new();
        for row in sheet.iter_active_rows() {
            let idx = row as usize;
            if !sheet.readiness[idx].is_ready(ReadinessMask::COLLIDER) {
                continue;
            }
            let aabb = collider_world_aabb(&sheet.world_transforms[idx], &sheet.colliders[idx]);
            let code = morton_code30(aabb.center(), bounds);
            pairs.push((code, row));
            leaf_aabbs.push((row, aabb));
        }
        radix_sort_morton(&mut pairs);
        build_lbvh_from_sorted(pairs, &leaf_aabbs)
    }
}

fn build_lbvh_from_sorted(sorted: Vec<(u32, u32)>, leaf_aabbs: &[(u32, Aabb)]) -> FlatLbvh {
    if sorted.is_empty() {
        return FlatLbvh::default();
    }
    let aabb_of = |row: u32| -> Aabb {
        leaf_aabbs
            .iter()
            .find(|(r, _)| *r == row)
            .map(|(_, a)| *a)
            .unwrap_or_default()
    };

    let mut nodes = Vec::new();
    // Create leaves in sorted order.
    let mut leaf_indices = Vec::with_capacity(sorted.len());
    for &(_code, row) in &sorted {
        let idx = nodes.len() as u32;
        nodes.push(LbvhNode {
            aabb: aabb_of(row),
            left: u32::MAX,
            right: u32::MAX,
            row_id: Some(row),
        });
        leaf_indices.push(idx);
    }

    // Bottom-up pairwise merge (simple flat LBVH).
    let mut level = leaf_indices;
    while level.len() > 1 {
        let mut next = Vec::with_capacity(level.len().div_ceil(2));
        let mut i = 0;
        while i < level.len() {
            if i + 1 == level.len() {
                next.push(level[i]);
                break;
            }
            let left = level[i];
            let right = level[i + 1];
            let aabb = nodes[left as usize].aabb.merge(nodes[right as usize].aabb);
            let parent = nodes.len() as u32;
            nodes.push(LbvhNode {
                aabb,
                left,
                right,
                row_id: None,
            });
            next.push(parent);
            i += 2;
        }
        level = next;
    }

    FlatLbvh {
        root: level.first().copied(),
        nodes,
        sorted_leaves: sorted,
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
}
