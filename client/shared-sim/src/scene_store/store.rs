//! Host-Sequenced apply engine: Property-level LWW + cycle MoveNode Reject (x00de)
//! + SimPatch queue generation (x00df).

use std::collections::HashMap;

use super::sparse_set::SparseSet;
use super::{CrdtOp, EntityId, OpTimestamp, PropValue, SimPatch};
use crate::sim_arena::LocalSlotId;

/// Bound ancestor walks so a corrupt / ultra-deep parent chain cannot loop forever.
///
/// A **legitimate** tree deeper than this also yields
/// [`ApplyOutcome::RejectedDepthExceeded`] on Move/re-Spawn under that leaf
/// (defense-in-depth; Phase 1b rooms are expected well below this). Fixed by tests.
pub const MAX_PARENT_HOPS: usize = 256;

/// Outcome of [`SceneStore::apply`] (Reject = No-op on the store).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// Fresh insert ([`CrdtOp::SpawnNode`] on absent id) or successful
    /// [`CrdtOp::MoveNode`] / [`CrdtOp::SetProperty`].
    Applied,
    /// Live [`CrdtOp::SpawnNode`] with newer parent ts: parent LWW only
    /// (props kept; **not** a node regenerate). Same parent write as Move.
    AppliedAsReparent,
    /// Kleppmann-style: `new_parent` is `node` or a descendant of `node`.
    RejectedCycle,
    /// Incoming [`OpTimestamp`] lost Property-level / parent LWW (`<=` stored).
    RejectedStale,
    /// `lamport > max_lamport + MAX_LAMPORT_DRIFT` (future-clock LWW takeover).
    RejectedClockDrift,
    /// Target / parent entity is not live in the store.
    RejectedMissing,
    /// `entity_id == u32::MAX` (sparse ABSENT) or other id contract breach.
    RejectedInvalidId,
    /// Parent chain longer than [`MAX_PARENT_HOPS`] while checking cycles.
    RejectedDepthExceeded,
}

impl ApplyOutcome {
    /// `true` for store-mutating successes ([`Self::Applied`] **or**
    /// [`Self::AppliedAsReparent`]). Prefer this over `== Applied` so live
    /// re-Spawn is not treated as failure (facade / x00e0 callers).
    #[inline]
    pub const fn is_success(self) -> bool {
        matches!(self, Self::Applied | Self::AppliedAsReparent)
    }
}

#[derive(Clone, Debug, PartialEq)]
struct LwwValue {
    ts: OpTimestamp,
    value: PropValue,
}

#[derive(Clone, Debug)]
struct NodeRecord {
    /// Parent link LWW clock (also set on Spawn).
    parent_ts: OpTimestamp,
    parent: Option<EntityId>,
    props: HashMap<u32, LwwValue>,
}

/// Sparse Scene Store with Host-Sequenced CRDT apply + SimPatch queue.
///
/// Guarantees: Property-level LWW on [`CrdtOp::SetProperty`]; parent LWW on
/// Spawn/Move; cyclic [`CrdtOp::MoveNode`] → [`ApplyOutcome::RejectedCycle`]
/// (No-op); lamport beyond [`super::MAX_LAMPORT_DRIFT`] of [`Self::max_lamport`]
/// → [`ApplyOutcome::RejectedClockDrift`] (**reject**, not clamp — no drift
/// ladder). Live re-[`CrdtOp::SpawnNode`] is parent LWW only
/// ([`ApplyOutcome::AppliedAsReparent`]; props kept — **not** regenerate).
/// Successful Spawn / Move / reparent enqueue [`SimPatch`] for Arena Step 0
/// Drain; dense transform / bits / box go via [`Self::enqueue_patch`] (not CRDT).
/// Keys = [`EntityId`].
/// Non-guarantees: edit wire / Zenoh / Save I/O (host facade); TRASH_UUID /
/// SoftLease / `op_chain_hash`; JWT `sub` → actor_id; FreeSlot auto-enqueue
/// (→ x00e1 pending_despawns).
#[derive(Clone, Debug, Default)]
pub struct SceneStore {
    nodes: SparseSet<NodeRecord>,
    /// Highest lamport that successfully applied (drift gate base).
    /// Raised only on `Applied` / `AppliedAsReparent` — never on reject, never
    /// by clamping a far-future clock into the store.
    max_lamport: u64,
    /// Dense diffs for Sim Arena Step 0 Drain (tick-boundary Lost Update barrier).
    patches: Vec<SimPatch>,
}

impl SceneStore {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    #[inline]
    pub fn contains(&self, entity_id: EntityId) -> bool {
        self.nodes.contains(entity_id)
    }

    #[inline]
    pub fn max_lamport(&self) -> u64 {
        self.max_lamport
    }

    /// Parent of `entity_id` (`None` if missing or world-rooted).
    pub fn parent(&self, entity_id: EntityId) -> Option<EntityId> {
        self.nodes.get(entity_id).and_then(|n| n.parent)
    }

    /// Packed live entity ids (dense walk order).
    #[inline]
    pub fn entity_ids(&self) -> &[EntityId] {
        self.nodes.dense_slots()
    }

    /// Current LWW value for `(entity_id, prop_key)`, if any.
    pub fn property(&self, entity_id: EntityId, prop_key: u32) -> Option<&PropValue> {
        self.nodes
            .get(entity_id)
            .and_then(|n| n.props.get(&prop_key).map(|p| &p.value))
    }

    /// Pending dense patches (not yet drained for Arena Step 0).
    #[inline]
    pub fn pending_patches(&self) -> &[SimPatch] {
        &self.patches
    }

    /// Host / boot insert: create or replace a live node **without** LWW or
    /// [`SimPatch`] enqueue. Parent may be absent (forward-ref seed order).
    ///
    /// When `parent` is already live, runs the same cycle / depth gate as
    /// [`CrdtOp::MoveNode`] (fresh or re-seed). Fresh insert can still cycle if
    /// an ancestor already forward-refs `node` as its parent. Missing parent
    /// skips the gate — caller must not leave a permanent cycle after the
    /// full seed pass. Rejects invalid ids (`u32::MAX`, self-parent).
    pub fn seed_node(&mut self, node: EntityId, parent: Option<EntityId>) -> bool {
        if !valid_entity_id(node) {
            return false;
        }
        if let Some(p) = parent {
            if !valid_entity_id(p) || p == node {
                return false;
            }
            if self.nodes.contains(p) && self.cycle_check(node, p).is_err() {
                return false;
            }
        }
        let ts = OpTimestamp::new(self.max_lamport.saturating_add(1), 0);
        if let Some(existing) = self.nodes.get_mut(node) {
            existing.parent_ts = ts;
            existing.parent = parent;
            // Props kept on re-seed (same as live re-Spawn contract).
        } else {
            self.nodes.insert(
                node,
                NodeRecord {
                    parent_ts: ts,
                    parent,
                    props: HashMap::new(),
                },
            );
        }
        self.note_lamport(ts);
        true
    }

    /// Remove a live node. Children that pointed at it become world-rooted
    /// (`parent = None`; `parent_ts` unchanged). Does **not** enqueue
    /// [`SimPatch::FreeSlot`] (→ x00e1). Returns `false` if missing.
    ///
    /// Cost: O(n) orphan scan over dense nodes (no child index yet — memo for
    /// x00e1 if despawn volume warrants a reverse map).
    pub fn remove_node(&mut self, entity_id: EntityId) -> bool {
        if !self.nodes.contains(entity_id) {
            return false;
        }
        let orphans: Vec<EntityId> = self
            .nodes
            .iter()
            .filter(|(_, n)| n.parent == Some(entity_id))
            .map(|(id, _)| id)
            .collect();
        let _ = self.nodes.remove(entity_id);
        for id in orphans {
            if let Some(rec) = self.nodes.get_mut(id) {
                rec.parent = None;
            }
        }
        true
    }

    /// Enqueue a dense patch (transform / bits / box / free). Does not touch
    /// sparse LWW state. Spawn/Move auto-enqueue; callers push the rest here.
    #[inline]
    pub fn enqueue_patch(&mut self, patch: SimPatch) {
        self.patches.push(patch);
    }

    /// Take the patch queue for Arena Drain (leaves the store queue empty).
    #[inline]
    pub fn drain_patches(&mut self) -> Vec<SimPatch> {
        std::mem::take(&mut self.patches)
    }

    /// Apply one host-ordered op. Successful Spawn / Move enqueue [`SimPatch`].
    /// Reject outcomes leave the store unchanged and do **not** enqueue.
    pub fn apply(&mut self, op: CrdtOp) -> ApplyOutcome {
        self.apply_inner(op, true)
    }

    /// Sparse LWW only — **no** [`SimPatch`] enqueue.
    ///
    /// Host LiveEdit facade uses this while FrameBridge owns the dense
    /// Reparent/SetField path. When Arena drain is wired from the ledger SSoT,
    /// switch facade callers to [`Self::apply`] (or drain this queue) so patches
    /// are not double-applied.
    pub fn apply_sparse(&mut self, op: CrdtOp) -> ApplyOutcome {
        self.apply_inner(op, false)
    }

    fn apply_inner(&mut self, op: CrdtOp, enqueue_patches: bool) -> ApplyOutcome {
        let ts = op.timestamp();
        if !ts.within_drift(self.max_lamport) {
            return ApplyOutcome::RejectedClockDrift;
        }
        match op {
            CrdtOp::SpawnNode { node, parent, .. } => {
                self.apply_spawn(ts, node, parent, enqueue_patches)
            }
            CrdtOp::MoveNode {
                node, new_parent, ..
            } => self.apply_move(ts, node, new_parent, enqueue_patches),
            CrdtOp::SetProperty {
                node,
                prop_key,
                value,
                ..
            } => self.apply_set_property(ts, node, prop_key, value),
        }
    }

    fn apply_spawn(
        &mut self,
        ts: OpTimestamp,
        node: EntityId,
        parent: Option<EntityId>,
        enqueue_patches: bool,
    ) -> ApplyOutcome {
        if !valid_entity_id(node) {
            return ApplyOutcome::RejectedInvalidId;
        }
        if let Some(p) = parent {
            if !valid_entity_id(p) {
                return ApplyOutcome::RejectedInvalidId;
            }
            if p == node {
                return ApplyOutcome::RejectedCycle;
            }
            if !self.nodes.contains(p) {
                return ApplyOutcome::RejectedMissing;
            }
        }

        if let Some(existing) = self.nodes.get(node) {
            if ts <= existing.parent_ts {
                return ApplyOutcome::RejectedStale;
            }
            // Live re-Spawn ≠ regenerate: parent LWW only; props stay.
            // Phase 1b provisional (host resend / newer parent ts); dedicated
            // "already live" hard-error remains optional for a later slice.
            return match self.apply_move(ts, node, parent, enqueue_patches) {
                ApplyOutcome::Applied => ApplyOutcome::AppliedAsReparent,
                other => other,
            };
        }

        self.nodes.insert(
            node,
            NodeRecord {
                parent_ts: ts,
                parent,
                props: HashMap::new(),
            },
        );
        self.note_lamport(ts);
        if enqueue_patches {
            self.patches.push(SimPatch::AllocateSlot {
                slot: entity_as_slot(node),
                parent: parent.map(entity_as_slot),
            });
        }
        ApplyOutcome::Applied
    }

    fn apply_move(
        &mut self,
        ts: OpTimestamp,
        node: EntityId,
        new_parent: Option<EntityId>,
        enqueue_patches: bool,
    ) -> ApplyOutcome {
        if !valid_entity_id(node) {
            return ApplyOutcome::RejectedInvalidId;
        }
        if !self.nodes.contains(node) {
            return ApplyOutcome::RejectedMissing;
        }
        if let Some(p) = new_parent {
            if !valid_entity_id(p) {
                return ApplyOutcome::RejectedInvalidId;
            }
            if !self.nodes.contains(p) {
                return ApplyOutcome::RejectedMissing;
            }
            match self.cycle_check(node, p) {
                Ok(()) => {}
                Err(outcome) => return outcome,
            }
        }

        {
            let Some(record) = self.nodes.get_mut(node) else {
                return ApplyOutcome::RejectedMissing;
            };
            if ts <= record.parent_ts {
                return ApplyOutcome::RejectedStale;
            }
            record.parent_ts = ts;
            record.parent = new_parent;
        }
        self.note_lamport(ts);
        if enqueue_patches {
            self.patches.push(SimPatch::SetParent {
                slot: entity_as_slot(node),
                parent: new_parent.map(entity_as_slot),
            });
        }
        ApplyOutcome::Applied
    }

    fn apply_set_property(
        &mut self,
        ts: OpTimestamp,
        node: EntityId,
        prop_key: u32,
        value: PropValue,
    ) -> ApplyOutcome {
        if !valid_entity_id(node) {
            return ApplyOutcome::RejectedInvalidId;
        }
        {
            let Some(record) = self.nodes.get_mut(node) else {
                return ApplyOutcome::RejectedMissing;
            };
            if let Some(prev) = record.props.get(&prop_key) {
                if ts <= prev.ts {
                    return ApplyOutcome::RejectedStale;
                }
            }
            record.props.insert(prop_key, LwwValue { ts, value });
        }
        self.note_lamport(ts);
        ApplyOutcome::Applied
    }

    /// Cycle iff `new_parent == node` or `node` is an ancestor of `new_parent`.
    fn cycle_check(&self, node: EntityId, new_parent: EntityId) -> Result<(), ApplyOutcome> {
        if new_parent == node {
            return Err(ApplyOutcome::RejectedCycle);
        }
        let mut cur = new_parent;
        for _ in 0..MAX_PARENT_HOPS {
            let Some(next) = self.parent(cur) else {
                return Ok(());
            };
            if next == node {
                return Err(ApplyOutcome::RejectedCycle);
            }
            cur = next;
        }
        Err(ApplyOutcome::RejectedDepthExceeded)
    }

    #[inline]
    fn note_lamport(&mut self, ts: OpTimestamp) {
        if ts.lamport > self.max_lamport {
            self.max_lamport = ts.lamport;
        }
    }
}

#[inline]
fn valid_entity_id(id: EntityId) -> bool {
    id != u32::MAX
}

/// Phase 1b: Arena slot index == Scene Store entity id (numerically).
#[inline]
const fn entity_as_slot(id: EntityId) -> LocalSlotId {
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_store::MAX_LAMPORT_DRIFT;

    fn ts(lamport: u64, actor: u64) -> OpTimestamp {
        OpTimestamp::new(lamport, actor)
    }

    fn spawn(store: &mut SceneStore, lamport: u64, node: EntityId, parent: Option<EntityId>) {
        assert_eq!(
            store.apply(CrdtOp::SpawnNode {
                ts: ts(lamport, 1),
                node,
                parent,
            }),
            ApplyOutcome::Applied
        );
    }

    #[test]
    fn move_rejects_cycle_kleppmann() {
        let mut store = SceneStore::new();
        spawn(&mut store, 1, 1, None);
        spawn(&mut store, 2, 2, Some(1));
        spawn(&mut store, 3, 3, Some(2));
        // 1 → 2 → 3; moving 1 under 3 would cycle.
        assert_eq!(
            store.apply(CrdtOp::MoveNode {
                ts: ts(4, 1),
                node: 1,
                new_parent: Some(3),
            }),
            ApplyOutcome::RejectedCycle
        );
        assert_eq!(store.parent(1), None);
        assert_eq!(store.parent(2), Some(1));
        assert_eq!(store.parent(3), Some(2));

        // Self-parent.
        assert_eq!(
            store.apply(CrdtOp::MoveNode {
                ts: ts(5, 1),
                node: 2,
                new_parent: Some(2),
            }),
            ApplyOutcome::RejectedCycle
        );
        assert_eq!(store.parent(2), Some(1));
    }

    #[test]
    fn move_non_cycle_applies() {
        let mut store = SceneStore::new();
        spawn(&mut store, 1, 1, None);
        spawn(&mut store, 2, 2, Some(1));
        spawn(&mut store, 3, 3, Some(2));
        // Move 3 under 1 (valid).
        assert_eq!(
            store.apply(CrdtOp::MoveNode {
                ts: ts(4, 1),
                node: 3,
                new_parent: Some(1),
            }),
            ApplyOutcome::Applied
        );
        assert_eq!(store.parent(3), Some(1));
        // Detach to root.
        assert_eq!(
            store.apply(CrdtOp::MoveNode {
                ts: ts(5, 1),
                node: 3,
                new_parent: None,
            }),
            ApplyOutcome::Applied
        );
        assert_eq!(store.parent(3), None);
    }

    #[test]
    fn property_lww_newer_wins_older_rejected() {
        let mut store = SceneStore::new();
        spawn(&mut store, 1, 10, None);
        let key = 42u32;
        assert_eq!(
            store.apply(CrdtOp::SetProperty {
                ts: ts(2, 1),
                node: 10,
                prop_key: key,
                value: PropValue::U32(100),
            }),
            ApplyOutcome::Applied
        );
        assert_eq!(store.property(10, key), Some(&PropValue::U32(100)));

        // Older lamport → stale.
        assert_eq!(
            store.apply(CrdtOp::SetProperty {
                ts: ts(1, 99),
                node: 10,
                prop_key: key,
                value: PropValue::U32(999),
            }),
            ApplyOutcome::RejectedStale
        );
        assert_eq!(store.property(10, key), Some(&PropValue::U32(100)));

        // Same lamport, lower actor → stale; higher actor → win.
        assert_eq!(
            store.apply(CrdtOp::SetProperty {
                ts: ts(2, 0),
                node: 10,
                prop_key: key,
                value: PropValue::U32(1),
            }),
            ApplyOutcome::RejectedStale
        );
        assert_eq!(
            store.apply(CrdtOp::SetProperty {
                ts: ts(2, 2),
                node: 10,
                prop_key: key,
                value: PropValue::U32(200),
            }),
            ApplyOutcome::Applied
        );
        assert_eq!(store.property(10, key), Some(&PropValue::U32(200)));

        // Independent prop keys do not interfere.
        assert_eq!(
            store.apply(CrdtOp::SetProperty {
                ts: ts(1, 1),
                node: 10,
                prop_key: key + 1,
                value: PropValue::Bool(true),
            }),
            ApplyOutcome::Applied
        );
        assert_eq!(store.property(10, key + 1), Some(&PropValue::Bool(true)));
        assert_eq!(store.property(10, key), Some(&PropValue::U32(200)));
    }

    #[test]
    fn parent_lww_rejects_stale_move() {
        let mut store = SceneStore::new();
        spawn(&mut store, 1, 1, None);
        spawn(&mut store, 2, 2, None);
        spawn(&mut store, 3, 3, Some(1));
        assert_eq!(
            store.apply(CrdtOp::MoveNode {
                ts: ts(10, 1),
                node: 3,
                new_parent: Some(2),
            }),
            ApplyOutcome::Applied
        );
        assert_eq!(store.parent(3), Some(2));
        assert_eq!(
            store.apply(CrdtOp::MoveNode {
                ts: ts(5, 1),
                node: 3,
                new_parent: Some(1),
            }),
            ApplyOutcome::RejectedStale
        );
        assert_eq!(store.parent(3), Some(2));
    }

    #[test]
    fn lamport_drift_reject_blocks_lww_takeover() {
        let mut store = SceneStore::new();
        spawn(&mut store, 10, 1, None);
        assert_eq!(store.max_lamport(), 10);

        // Far-future clock is rejected (No-op); max does not ratchet.
        let far = OpTimestamp::new(u64::MAX, 1);
        assert_eq!(
            store.apply(CrdtOp::SetProperty {
                ts: far,
                node: 1,
                prop_key: 0,
                value: PropValue::U32(1),
            }),
            ApplyOutcome::RejectedClockDrift
        );
        assert_eq!(store.max_lamport(), 10);
        assert!(store.property(1, 0).is_none());

        // Edge of the allowed window still applies.
        let edge = 10 + MAX_LAMPORT_DRIFT;
        assert_eq!(
            store.apply(CrdtOp::SetProperty {
                ts: ts(edge, 1),
                node: 1,
                prop_key: 0,
                value: PropValue::U32(2),
            }),
            ApplyOutcome::Applied
        );
        assert_eq!(store.property(1, 0), Some(&PropValue::U32(2)));
        assert_eq!(store.max_lamport(), edge);

        // One past the new window → drift reject again.
        assert_eq!(
            store.apply(CrdtOp::SetProperty {
                ts: ts(edge + MAX_LAMPORT_DRIFT + 1, 1),
                node: 1,
                prop_key: 0,
                value: PropValue::U32(3),
            }),
            ApplyOutcome::RejectedClockDrift
        );
        assert_eq!(store.property(1, 0), Some(&PropValue::U32(2)));
    }

    #[test]
    fn rejects_absent_sentinel_and_missing() {
        let mut store = SceneStore::new();
        assert_eq!(
            store.apply(CrdtOp::SpawnNode {
                ts: ts(1, 1),
                node: u32::MAX,
                parent: None,
            }),
            ApplyOutcome::RejectedInvalidId
        );
        assert_eq!(
            store.apply(CrdtOp::SetProperty {
                ts: ts(1, 1),
                node: 99,
                prop_key: 0,
                value: PropValue::Bool(false),
            }),
            ApplyOutcome::RejectedMissing
        );
        spawn(&mut store, 1, 1, None);
        assert_eq!(
            store.apply(CrdtOp::MoveNode {
                ts: ts(2, 1),
                node: 1,
                new_parent: Some(99),
            }),
            ApplyOutcome::RejectedMissing
        );
    }

    #[test]
    fn spawn_self_parent_rejects_cycle() {
        let mut store = SceneStore::new();
        assert_eq!(
            store.apply(CrdtOp::SpawnNode {
                ts: ts(1, 1),
                node: 7,
                parent: Some(7),
            }),
            ApplyOutcome::RejectedCycle
        );
        assert!(!store.contains(7));
    }

    #[test]
    fn live_respawn_is_parent_lww_keeps_props() {
        let mut store = SceneStore::new();
        spawn(&mut store, 1, 1, None);
        spawn(&mut store, 2, 2, None);
        spawn(&mut store, 3, 3, Some(1));
        assert_eq!(
            store.apply(CrdtOp::SetProperty {
                ts: ts(4, 1),
                node: 3,
                prop_key: 9,
                value: PropValue::U32(42),
            }),
            ApplyOutcome::Applied
        );

        // Equal ts → stale (not idempotent Applied).
        assert_eq!(
            store.apply(CrdtOp::SpawnNode {
                ts: ts(3, 1),
                node: 3,
                parent: Some(2),
            }),
            ApplyOutcome::RejectedStale
        );
        assert_eq!(store.parent(3), Some(1));

        // Newer ts → AppliedAsReparent; props survive.
        let outcome = store.apply(CrdtOp::SpawnNode {
            ts: ts(5, 1),
            node: 3,
            parent: Some(2),
        });
        assert_eq!(outcome, ApplyOutcome::AppliedAsReparent);
        assert!(outcome.is_success());
        assert!(!ApplyOutcome::RejectedStale.is_success());
        assert_eq!(store.parent(3), Some(2));
        assert_eq!(store.property(3, 9), Some(&PropValue::U32(42)));
        assert_eq!(store.len(), 3);
    }

    #[test]
    fn empty_store_first_lamport_bounded_by_drift() {
        let mut store = SceneStore::new();
        assert_eq!(store.max_lamport(), 0);
        assert_eq!(
            store.apply(CrdtOp::SpawnNode {
                ts: ts(MAX_LAMPORT_DRIFT + 1, 1),
                node: 1,
                parent: None,
            }),
            ApplyOutcome::RejectedClockDrift
        );
        assert!(store.is_empty());
        assert_eq!(
            store.apply(CrdtOp::SpawnNode {
                ts: ts(MAX_LAMPORT_DRIFT, 1),
                node: 1,
                parent: None,
            }),
            ApplyOutcome::Applied
        );
        assert_eq!(store.max_lamport(), MAX_LAMPORT_DRIFT);
    }

    #[test]
    fn move_under_deeper_than_max_parent_hops_rejects_depth() {
        let mut store = SceneStore::new();
        // Chain 0 → 1 → … → MAX (MAX+1 nodes, MAX edges leaf→root).
        spawn(&mut store, 1, 0, None);
        for i in 1..=MAX_PARENT_HOPS as u32 {
            spawn(&mut store, u64::from(i) + 1, i, Some(i - 1));
        }
        let leaf = MAX_PARENT_HOPS as u32;
        let outsider = leaf + 1;
        spawn(&mut store, u64::from(outsider) + 1, outsider, None);
        // Walking leaf→… takes MAX_PARENT_HOPS hops without hitting root → depth.
        assert_eq!(
            store.apply(CrdtOp::MoveNode {
                ts: ts(u64::from(outsider) + 2, 1),
                node: outsider,
                new_parent: Some(leaf),
            }),
            ApplyOutcome::RejectedDepthExceeded
        );
        assert_eq!(store.parent(outsider), None);
    }

    #[test]
    fn seed_node_allows_missing_parent_and_skips_patches() {
        let mut store = SceneStore::new();
        assert!(store.seed_node(2, Some(1))); // parent 1 not live yet
        assert!(store.contains(2));
        assert_eq!(store.parent(2), Some(1));
        assert!(store.pending_patches().is_empty());
        assert!(store.seed_node(1, None));
        assert_eq!(store.entity_ids().len(), 2);
        assert!(!store.seed_node(u32::MAX, None));
        assert!(!store.seed_node(3, Some(3)));
    }

    #[test]
    fn seed_node_rejects_cycle_on_reseed_under_descendant() {
        let mut store = SceneStore::new();
        assert!(store.seed_node(1, None));
        assert!(store.seed_node(2, Some(1)));
        // Re-seed 1 under 2 would cycle.
        assert!(!store.seed_node(1, Some(2)));
        assert_eq!(store.parent(1), None);
    }

    #[test]
    fn seed_node_rejects_fresh_insert_cycle_via_forward_ref_parent() {
        let mut store = SceneStore::new();
        // A → B while B not live yet (forward-ref).
        assert!(store.seed_node(1, Some(2)));
        // Fresh B under A would close 1↔2.
        assert!(!store.seed_node(2, Some(1)));
        assert!(!store.contains(2));
        assert_eq!(store.parent(1), Some(2));
    }

    #[test]
    fn apply_sparse_mutates_without_enqueueing_patches() {
        let mut store = SceneStore::new();
        assert_eq!(
            store.apply_sparse(CrdtOp::SpawnNode {
                ts: ts(1, 1),
                node: 1,
                parent: None,
            }),
            ApplyOutcome::Applied
        );
        assert_eq!(
            store.apply_sparse(CrdtOp::SpawnNode {
                ts: ts(2, 1),
                node: 2,
                parent: Some(1),
            }),
            ApplyOutcome::Applied
        );
        assert!(store.pending_patches().is_empty());
        assert_eq!(
            store.apply_sparse(CrdtOp::MoveNode {
                ts: ts(3, 1),
                node: 2,
                new_parent: None,
            }),
            ApplyOutcome::Applied
        );
        assert_eq!(store.parent(2), None);
        assert!(store.pending_patches().is_empty());
    }

    #[test]
    fn remove_node_clears_orphan_parents() {
        let mut store = SceneStore::new();
        spawn(&mut store, 1, 1, None);
        spawn(&mut store, 2, 2, Some(1));
        spawn(&mut store, 3, 3, Some(1));
        store.drain_patches();
        assert!(store.remove_node(1));
        assert!(!store.contains(1));
        assert_eq!(store.parent(2), None);
        assert_eq!(store.parent(3), None);
        assert!(store.pending_patches().is_empty());
        assert!(!store.remove_node(1));
    }

    #[test]
    fn successful_spawn_and_move_enqueue_sim_patches_reject_does_not() {
        let mut store = SceneStore::new();
        spawn(&mut store, 1, 1, None);
        spawn(&mut store, 2, 2, Some(1));
        assert_eq!(
            store.pending_patches(),
            &[
                SimPatch::AllocateSlot {
                    slot: 1,
                    parent: None,
                },
                SimPatch::AllocateSlot {
                    slot: 2,
                    parent: Some(1),
                },
            ]
        );

        assert_eq!(
            store.apply(CrdtOp::MoveNode {
                ts: ts(3, 1),
                node: 2,
                new_parent: None,
            }),
            ApplyOutcome::Applied
        );
        assert_eq!(
            store.pending_patches().last(),
            Some(&SimPatch::SetParent {
                slot: 2,
                parent: None,
            })
        );

        // Re-attach 2 under 1, then try to cycle 1 under 2.
        assert_eq!(
            store.apply(CrdtOp::MoveNode {
                ts: ts(4, 1),
                node: 2,
                new_parent: Some(1),
            }),
            ApplyOutcome::Applied
        );
        let before = store.pending_patches().len();
        assert_eq!(
            store.apply(CrdtOp::MoveNode {
                ts: ts(5, 1),
                node: 1,
                new_parent: Some(2),
            }),
            ApplyOutcome::RejectedCycle
        );
        assert_eq!(store.pending_patches().len(), before);

        let drained = store.drain_patches();
        assert_eq!(drained.len(), before);
        assert!(store.pending_patches().is_empty());
    }
}
