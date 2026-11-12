//! Flat topology schedules — depth buckets, Euler tour, constraint color stubs.
//!
//! Guarantees: never reorders `EpochSheet` rows; only rebuilds indirect index arrays
//! from `parent_of`. Cycle parents are rejected.
//! Non-guarantees: XR Hierarchy widget rendering; XPBD color fill (see `xpbd`).
//!
//! Safety: `#![forbid(unsafe_code)]`.

#![forbid(unsafe_code)]

use crate::tape::{EpochSheet, WorldTransformf64, XpbdConstraintRow};

/// Indirect schedule over immutable SoA rows.
///
/// Contract: `parent_of[row] = parent_row` or `u32::MAX` for roots. Reparent rebuilds
/// this struct only — never moves `EpochSheet` columns.
#[derive(Clone, Debug, Default)]
pub struct FlatTopology {
    pub parent_of: Vec<u32>,
    pub depth_sorted_rows: Vec<u32>,
    pub depth_offsets: Vec<u32>,
    pub euler_tin: Vec<u32>,
    pub euler_tout: Vec<u32>,
    pub euler_order: Vec<u32>,
    /// Filled by XPBD graph coloring; stub may be empty until colored.
    pub constraint_color_batches: Vec<XpbdConstraintRow>,
    pub color_batch_offsets: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopologyError {
    Cycle,
    OutOfBounds,
    Empty,
}

impl FlatTopology {
    pub fn from_parents(parent_of: Vec<u32>) -> Result<Self, TopologyError> {
        let mut topo = Self {
            parent_of,
            ..Default::default()
        };
        topo.rebuild()?;
        Ok(topo)
    }

    /// All rows are world roots (`parent_of = MAX`). Used by client Hierarchy until
    /// held / Prefab parents are mirrored (x0094).
    ///
    /// All-`MAX` parents cannot cycle; bake failure is treated as a logic bug and
    /// returns empty (caller should see `parent_of.len() != sheet.len()`).
    pub fn flat_roots(len: usize) -> Self {
        if len == 0 {
            return Self::default();
        }
        match Self::from_parents(vec![u32::MAX; len]) {
            Ok(topo) => topo,
            Err(err) => {
                debug_assert!(false, "flat_roots bake failed for len={len}: {err:?}");
                Self::default()
            }
        }
    }

    pub fn rebuild(&mut self) -> Result<(), TopologyError> {
        let n = self.parent_of.len();
        if n == 0 {
            self.depth_sorted_rows.clear();
            self.depth_offsets.clear();
            self.euler_tin.clear();
            self.euler_tout.clear();
            self.euler_order.clear();
            return Ok(());
        }
        for (i, &p) in self.parent_of.iter().enumerate() {
            if p != u32::MAX && p as usize >= n {
                return Err(TopologyError::OutOfBounds);
            }
            if p == i as u32 {
                return Err(TopologyError::Cycle);
            }
        }
        detect_cycles(&self.parent_of)?;
        bake_depth_buckets(self)?;
        bake_euler_tour(self)?;
        Ok(())
    }

    /// Jump index used by collapsed Hierarchy virtual scroll: `i <- tout[u] + 1` in order space.
    pub fn collapsed_skip_order_index(&self, row: u32) -> Option<u32> {
        let idx = row as usize;
        if idx >= self.euler_tout.len() {
            return None;
        }
        Some(self.euler_tout[idx].saturating_add(1))
    }
}

fn detect_cycles(parent_of: &[u32]) -> Result<(), TopologyError> {
    let n = parent_of.len();
    let mut state = vec![0u8; n]; // 0=unseen, 1=stack, 2=done
    for start in 0..n {
        if state[start] != 0 {
            continue;
        }
        let mut cur = start;
        loop {
            match state[cur] {
                1 => return Err(TopologyError::Cycle),
                2 => break,
                _ => {
                    state[cur] = 1;
                    let p = parent_of[cur];
                    if p == u32::MAX {
                        state[cur] = 2;
                        break;
                    }
                    cur = p as usize;
                    if cur >= n {
                        return Err(TopologyError::OutOfBounds);
                    }
                }
            }
        }
        // Mark path done by walking again from start.
        let mut cur = start;
        while state[cur] == 1 {
            state[cur] = 2;
            let p = parent_of[cur];
            if p == u32::MAX {
                break;
            }
            cur = p as usize;
        }
    }
    Ok(())
}

fn bake_depth_buckets(topo: &mut FlatTopology) -> Result<(), TopologyError> {
    let n = topo.parent_of.len();
    let mut depth = vec![0u32; n];
    for (i, depth_i) in depth.iter_mut().enumerate() {
        let mut d = 0u32;
        let mut cur = i;
        let mut guard = 0usize;
        while topo.parent_of[cur] != u32::MAX {
            cur = topo.parent_of[cur] as usize;
            d += 1;
            guard += 1;
            if guard > n {
                return Err(TopologyError::Cycle);
            }
        }
        *depth_i = d;
    }
    let max_depth = depth.iter().copied().max().unwrap_or(0);
    let mut counts = vec![0u32; (max_depth as usize) + 1];
    for &d in &depth {
        counts[d as usize] += 1;
    }
    let mut offsets = vec![0u32; counts.len() + 1];
    for i in 0..counts.len() {
        offsets[i + 1] = offsets[i] + counts[i];
    }
    let mut cursor = offsets.clone();
    let mut sorted = vec![0u32; n];
    // Stable by row_id within depth: iterate rows ascending.
    for row in 0..n as u32 {
        let d = depth[row as usize] as usize;
        let slot = cursor[d] as usize;
        sorted[slot] = row;
        cursor[d] += 1;
    }
    topo.depth_sorted_rows = sorted;
    topo.depth_offsets = offsets;
    Ok(())
}

fn bake_euler_tour(topo: &mut FlatTopology) -> Result<(), TopologyError> {
    let n = topo.parent_of.len();
    let mut children: Vec<Vec<u32>> = vec![Vec::new(); n];
    let mut roots = Vec::new();
    for (i, &p) in topo.parent_of.iter().enumerate() {
        if p == u32::MAX {
            roots.push(i as u32);
        } else {
            children[p as usize].push(i as u32);
        }
    }
    for c in &mut children {
        c.sort_unstable();
    }
    roots.sort_unstable();

    let mut tin = vec![0u32; n];
    let mut tout = vec![0u32; n];
    let mut order = Vec::with_capacity(n);
    let mut time = 0u32;

    fn dfs(
        row: u32,
        children: &[Vec<u32>],
        tin: &mut [u32],
        tout: &mut [u32],
        order: &mut Vec<u32>,
        time: &mut u32,
    ) {
        tin[row as usize] = *time;
        order.push(row);
        *time += 1;
        for &child in &children[row as usize] {
            dfs(child, children, tin, tout, order, time);
        }
        tout[row as usize] = (*time).saturating_sub(1);
    }

    for root in roots {
        dfs(root, &children, &mut tin, &mut tout, &mut order, &mut time);
    }
    topo.euler_tin = tin;
    topo.euler_tout = tout;
    topo.euler_order = order;
    Ok(())
}

/// Propagate parent world transforms into children (depth-bucket SoA loop).
///
/// Local translation is stored in `locals`; output written to `sheet.world_transforms`.
/// Rows themselves are never reordered.
pub fn propagate_world_transforms(
    topo: &FlatTopology,
    locals: &[WorldTransformf64],
    sheet: &mut EpochSheet,
) -> Result<(), TopologyError> {
    let n = topo.parent_of.len();
    if locals.len() != n || sheet.world_transforms.len() < n {
        return Err(TopologyError::OutOfBounds);
    }
    if n == 0 {
        return Err(TopologyError::Empty);
    }
    for &row in &topo.depth_sorted_rows {
        let idx = row as usize;
        let parent = topo.parent_of[idx];
        if parent == u32::MAX {
            sheet.world_transforms[idx] = locals[idx];
        } else {
            let p = &sheet.world_transforms[parent as usize];
            let l = &locals[idx];
            let mut world = *l;
            world.position = [
                p.position[0] + l.position[0],
                p.position[1] + l.position[1],
                p.position[2] + l.position[2],
            ];
            // Scale multiply (rotation composition deferred — translation-first Phase).
            world.scale = [
                p.scale[0] * l.scale[0],
                p.scale[1] * l.scale[1],
                p.scale[2] * l.scale[2],
            ];
            if !world.is_finite() {
                world = *p;
            }
            sheet.world_transforms[idx] = world;
        }
        sheet.mark_dirty(row);
    }
    Ok(())
}

/// Stub: verify each color batch has no shared node endpoints (filled by `xpbd::color_constraints`).
pub fn validate_color_batches(topo: &FlatTopology) -> bool {
    if topo.color_batch_offsets.is_empty() {
        return true;
    }
    for bi in 0..topo.color_batch_offsets.len().saturating_sub(1) {
        let start = topo.color_batch_offsets[bi] as usize;
        let end = topo.color_batch_offsets[bi + 1] as usize;
        let mut used = std::collections::HashSet::new();
        for row in &topo.constraint_color_batches[start..end] {
            if !used.insert(row.node_a) {
                return false;
            }
            if row.node_b != u32::MAX && !used.insert(row.node_b) {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::EpochSheet;

    #[test]
    fn depth_buckets_linear_and_branch() {
        // 0 root, 1 child of 0, 2 child of 1, 3 child of 0
        let topo = FlatTopology::from_parents(vec![u32::MAX, 0, 1, 0]).unwrap();
        assert_eq!(topo.depth_offsets, vec![0, 1, 3, 4]);
        assert_eq!(topo.depth_sorted_rows, vec![0, 1, 3, 2]);
    }

    #[test]
    fn cycle_detected() {
        assert_eq!(
            FlatTopology::from_parents(vec![1, 0]).unwrap_err(),
            TopologyError::Cycle
        );
    }

    #[test]
    fn flat_roots_all_max_parents() {
        let topo = FlatTopology::flat_roots(3);
        assert_eq!(topo.parent_of, vec![u32::MAX, u32::MAX, u32::MAX]);
        assert_eq!(topo.euler_order, vec![0, 1, 2]);
        assert!(FlatTopology::flat_roots(0).euler_order.is_empty());
    }

    #[test]
    fn euler_subtree_contiguous_and_collapse_jump() {
        let topo = FlatTopology::from_parents(vec![u32::MAX, 0, 1, 0]).unwrap();
        // order: 0,1,2,3
        assert_eq!(topo.euler_order, vec![0, 1, 2, 3]);
        assert_eq!(topo.euler_tin[0], 0);
        assert_eq!(topo.euler_tout[0], 3);
        assert_eq!(topo.euler_tin[1], 1);
        assert_eq!(topo.euler_tout[1], 2);
        // collapsing node 1 skips to after its subtree in time domain
        assert_eq!(topo.collapsed_skip_order_index(1), Some(3));
    }

    #[test]
    fn propagate_moves_children_without_reordering_rows() {
        let topo = FlatTopology::from_parents(vec![u32::MAX, 0]).unwrap();
        let mut sheet = EpochSheet::with_capacity(2);
        let _ = sheet.alloc_row();
        let _ = sheet.alloc_row();
        let locals = vec![
            WorldTransformf64::identity_at([10.0, 0.0, 0.0]),
            WorldTransformf64::identity_at([1.0, 2.0, 0.0]),
        ];
        propagate_world_transforms(&topo, &locals, &mut sheet).unwrap();
        assert_eq!(sheet.world_transforms[0].position, [10.0, 0.0, 0.0]);
        assert_eq!(sheet.world_transforms[1].position, [11.0, 2.0, 0.0]);
        // row slots unchanged
        assert_eq!(sheet.len(), 2);
    }
}
