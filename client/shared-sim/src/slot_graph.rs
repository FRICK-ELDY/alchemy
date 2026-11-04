//! Runtime parent/child slots for shared-sim (x0056 / x0059).
//!
//! Guarantees: child→parent for `user_root → held`; deterministic
//! [`SlotGraph::attach_held`] / [`SlotGraph::clear_held`]; general
//! [`SlotGraph::reparent`] with cycle rejection (x0059).
//! Non-guarantees: Rapier / Intent / Zenoh; persistence / blake3 / Drive;
//! pass/place semantics (→ x005a); undo (→ x005b).

use std::collections::{HashMap, HashSet};

/// Bound walks over possibly corrupted / deserialized graphs.
const MAX_GRAPH_HOPS: usize = 256;

/// Error from [`SlotGraph::reparent`] / [`SlotGraph::attach_held`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReparentError {
    /// `child == parent` or ancestor cycle.
    Cycle,
    /// Parent chain longer than [`MAX_GRAPH_HOPS`] (corrupt graph).
    DepthExceeded,
}

/// Runtime parent links (`child → parent`). World-rooted entities have no entry.
///
/// SoA-friendly: one map reused across ticks; no per-tick allocation beyond
/// occasional insert/remove.
#[derive(Clone, Debug, Default)]
pub struct SlotGraph {
    /// child entity_id → parent entity_id
    parent_of: HashMap<u32, u32>,
}

impl SlotGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parent of `child`, if any (world root → `None`).
    pub fn parent(&self, child: u32) -> Option<u32> {
        self.parent_of.get(&child).copied()
    }

    /// True when `child` is currently held by `holder` (typically a user_root).
    pub fn is_held_by(&self, child: u32, holder: u32) -> bool {
        self.parent(child) == Some(holder)
    }

    /// Attach `held` under `holder` (held Slot). Overwrites prior parent.
    ///
    /// Deterministic: same inputs → same graph. Does not validate entity kinds
    /// (caller / host owns that). Self-parent is a no-op (`Ok(())`).
    ///
    /// Returns [`ReparentError::DepthExceeded`] / [`ReparentError::Cycle`] when
    /// the graph is corrupt — callers must not ignore `DepthExceeded`.
    pub fn attach_held(&mut self, holder: u32, held: u32) -> Result<(), ReparentError> {
        if holder == held {
            return Ok(());
        }
        self.reparent(held, Some(holder))
    }

    /// Clear held parent for `held` (world-rooted again).
    pub fn clear_held(&mut self, held: u32) {
        let _ = self.reparent(held, None);
    }

    /// Clear every child whose parent is `holder`.
    pub fn clear_held_by(&mut self, holder: u32) {
        let children: Vec<u32> = self.children_of(holder);
        for child in children {
            self.parent_of.remove(&child);
        }
    }

    /// Direct children of `parent` (allocation for callers that need a list).
    pub fn children_of(&self, parent: u32) -> Vec<u32> {
        self.parent_of
            .iter()
            .filter_map(|(&child, &p)| (p == parent).then_some(child))
            .collect()
    }

    /// Walk ancestors of `start` (exclusive of `start`).
    /// Stops after [`MAX_GRAPH_HOPS`] to avoid infinite loops on corrupt cycles.
    pub fn ancestors(&self, start: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut cur = start;
        for _ in 0..MAX_GRAPH_HOPS {
            let Some(p) = self.parent(cur) else {
                break;
            };
            out.push(p);
            cur = p;
        }
        out
    }

    /// Descendants of `root` (not including `root`), depth-first.
    ///
    /// Bounded by visit-set + [`MAX_GRAPH_HOPS`] so a corrupt cycle cannot
    /// loop forever or grow without bound.
    pub fn descendants(&self, root: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut stack = self.children_of(root);
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            if out.len() >= MAX_GRAPH_HOPS {
                break;
            }
            out.push(id);
            stack.extend(self.children_of(id));
        }
        out
    }

    /// Set `child`'s parent to `parent` (`None` = world root). Rejects cycles.
    ///
    /// Held API (`attach_held` / `clear_held`) is this path with held semantics.
    pub fn reparent(&mut self, child: u32, parent: Option<u32>) -> Result<(), ReparentError> {
        if let Some(p) = parent {
            if p == child {
                return Err(ReparentError::Cycle);
            }
            // Would create cycle if `child` is an ancestor of `p`.
            let mut cur = p;
            for _ in 0..MAX_GRAPH_HOPS {
                let Some(next) = self.parent(cur) else {
                    self.parent_of.insert(child, p);
                    return Ok(());
                };
                if next == child {
                    return Err(ReparentError::Cycle);
                }
                cur = next;
            }
            return Err(ReparentError::DepthExceeded);
        }
        self.parent_of.remove(&child);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attach_and_clear_held_are_deterministic() {
        let mut g = SlotGraph::new();
        let root = 1_000_001;
        let box_id = 10;

        assert_eq!(g.parent(box_id), None);
        g.attach_held(root, box_id).unwrap();
        assert_eq!(g.parent(box_id), Some(root));
        assert!(g.is_held_by(box_id, root));
        assert_eq!(g.children_of(root), vec![box_id]);

        g.clear_held(box_id);
        assert_eq!(g.parent(box_id), None);
        assert!(!g.is_held_by(box_id, root));
        assert!(g.children_of(root).is_empty());
    }

    #[test]
    fn attach_held_overwrites_prior_parent() {
        let mut g = SlotGraph::new();
        g.attach_held(1_000_001, 10).unwrap();
        g.attach_held(1_000_002, 10).unwrap();
        assert_eq!(g.parent(10), Some(1_000_002));
        assert!(g.children_of(1_000_001).is_empty());
        assert_eq!(g.children_of(1_000_002), vec![10]);
    }

    #[test]
    fn clear_held_by_releases_all_children() {
        let mut g = SlotGraph::new();
        let root = 1_000_001;
        g.attach_held(root, 10).unwrap();
        g.attach_held(root, 11).unwrap();
        g.attach_held(1_000_002, 12).unwrap();
        g.clear_held_by(root);
        assert_eq!(g.parent(10), None);
        assert_eq!(g.parent(11), None);
        assert_eq!(g.parent(12), Some(1_000_002));
    }

    #[test]
    fn attach_held_self_parent_is_ok_noop() {
        let mut g = SlotGraph::new();
        assert!(g.attach_held(10, 10).is_ok());
        assert_eq!(g.parent(10), None);
    }

    #[test]
    fn reparent_rejects_cycle() {
        let mut g = SlotGraph::new();
        assert!(g.reparent(10, Some(20)).is_ok());
        assert!(g.reparent(20, Some(30)).is_ok());
        assert_eq!(g.reparent(30, Some(10)), Err(ReparentError::Cycle));
        assert_eq!(g.reparent(10, Some(10)), Err(ReparentError::Cycle));
        assert_eq!(g.parent(30), None);
        assert_eq!(g.ancestors(10), vec![20, 30]);
    }

    #[test]
    fn reparent_none_clears_and_descendants() {
        let mut g = SlotGraph::new();
        g.reparent(10, Some(1)).unwrap();
        g.reparent(11, Some(10)).unwrap();
        let mut desc = g.descendants(1);
        desc.sort_unstable();
        assert_eq!(desc, vec![10, 11]);
        g.reparent(10, None).unwrap();
        assert_eq!(g.parent(10), None);
        assert_eq!(g.parent(11), Some(10));
    }

    #[test]
    fn descendants_terminates_on_corrupt_cycle() {
        let mut g = SlotGraph::new();
        // Inject cycle via raw map (reparent would refuse).
        g.parent_of.insert(10, 20);
        g.parent_of.insert(20, 10);
        let desc = g.descendants(10);
        assert!(desc.len() <= 2);
        assert!(desc.contains(&20));
    }
}
