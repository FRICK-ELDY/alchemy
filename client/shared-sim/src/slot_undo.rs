//! Per-operator session undo/redo for Slot reparent transitions (x005b / x00b3 / x00e1).
//!
//! Guarantees: reverse/reapply last reparent recorded for an operator stack;
//! stack cap with drop-oldest on overflow; cycle/depth apply failures restore the
//! entry; [`UndoError::Stale`] (parent mismatch, missing node, or store LWW stale /
//! missing) **discards** the popped entry; Scene Store path applies the inverse as a
//! **fresh** [`CrdtOp::MoveNode`] (latest lamport, caller `actor_id`).
//! Non-guarantees: persistence across leave; undoing another operator's moves;
//! physics velocity restore; Undo Intent / tip / FrameBridge wire (host peel is
//! test-only until Intent); TRASH_UUID (→ x00e3); SoftLease is host edit gate (x00e2).

use std::collections::VecDeque;

use crate::scene_store::{ApplyOutcome, CrdtOp, OpTimestamp, SceneStore};
use crate::slot_graph::{ReparentError, SlotGraph};

/// Undo precondition / apply failure (x00b3 / x00e1 / mvcc_xpbd §4.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UndoError {
    /// Lost stack entry: parent ≠ recorded `to`, child/parent missing after
    /// despawn, or store [`ApplyOutcome::RejectedStale`]. **Entry is discarded**
    /// (not restored) — UI must treat as dropped, not "stack unchanged".
    Stale,
    /// Underlying [`SlotGraph::reparent`] / store cycle·depth rejected the inverse
    /// (**entry restored** on the stack).
    Reparent(ReparentError),
    /// [`SceneStore`] rejected the inverse Move for a non-Stale reason
    /// (**entry restored**). Prefer mapping missing → [`Self::Stale`].
    Store(ApplyOutcome),
}

/// One reversible reparent (`child` moved from `from` → `to`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotTransition {
    pub child: u32,
    pub from: Option<u32>,
    pub to: Option<u32>,
}

impl SlotTransition {
    pub fn inverse(self) -> Self {
        Self {
            child: self.child,
            from: self.to,
            to: self.from,
        }
    }
}

/// Per-operator undo/redo stacks (session-local).
#[derive(Clone, Debug)]
pub struct SlotUndoStack {
    undo: VecDeque<SlotTransition>,
    redo: VecDeque<SlotTransition>,
    cap: usize,
}

impl Default for SlotUndoStack {
    fn default() -> Self {
        Self::new(32)
    }
}

impl SlotUndoStack {
    pub fn new(cap: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            cap: cap.max(1),
        }
    }

    #[inline]
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    #[inline]
    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    pub fn push(&mut self, t: SlotTransition) {
        if self.undo.len() >= self.cap {
            let _ = self.undo.pop_front();
        }
        self.undo.push_back(t);
        self.redo.clear();
    }

    /// Drop undo/redo entries whose `child` matches (live despawn / lost target).
    pub fn discard_child(&mut self, child: u32) {
        self.undo.retain(|t| t.child != child);
        self.redo.retain(|t| t.child != child);
    }

    pub fn undo(&mut self, graph: &mut SlotGraph) -> Result<Option<SlotTransition>, ReparentError> {
        let Some(t) = self.undo.pop_back() else {
            return Ok(None);
        };
        let inv = t.inverse();
        match graph.reparent(inv.child, inv.to) {
            Ok(()) => {
                self.redo.push_back(t);
                Ok(Some(inv))
            }
            Err(e) => {
                self.undo.push_back(t);
                Err(e)
            }
        }
    }

    /// Undo only if `graph.parent(child)` still equals the transition's `to`.
    ///
    /// On [`UndoError::Stale`] the transition is **popped and discarded** (does not rewrite
    /// another operator's parent). Undo UI must treat that as "lost stack entry", not as
    /// "stack unchanged".
    pub fn undo_checked(
        &mut self,
        graph: &mut SlotGraph,
    ) -> Result<Option<SlotTransition>, UndoError> {
        let Some(t) = self.undo.pop_back() else {
            return Ok(None);
        };
        if graph.parent(t.child) != t.to {
            return Err(UndoError::Stale);
        }
        let inv = t.inverse();
        match graph.reparent(inv.child, inv.to) {
            Ok(()) => {
                self.redo.push_back(t);
                Ok(Some(inv))
            }
            Err(e) => {
                self.undo.push_back(t);
                Err(UndoError::Reparent(e))
            }
        }
    }

    pub fn redo(&mut self, graph: &mut SlotGraph) -> Result<Option<SlotTransition>, ReparentError> {
        let Some(t) = self.redo.pop_back() else {
            return Ok(None);
        };
        match graph.reparent(t.child, t.to) {
            Ok(()) => {
                self.undo.push_back(t);
                Ok(Some(t))
            }
            Err(e) => {
                self.redo.push_back(t);
                Err(e)
            }
        }
    }

    /// Undo against [`SceneStore`] (fresh MoveNode; enqueues [`crate::SimPatch`] on success).
    pub fn undo_store(
        &mut self,
        store: &mut SceneStore,
        actor_id: u64,
    ) -> Result<Option<SlotTransition>, UndoError> {
        self.undo_store_inner(store, actor_id, true, false)
    }

    /// Like [`Self::undo_checked`] but applies inverse via Scene Store LWW Move.
    ///
    /// On [`UndoError::Stale`] the transition is popped and discarded.
    pub fn undo_checked_store(
        &mut self,
        store: &mut SceneStore,
        actor_id: u64,
    ) -> Result<Option<SlotTransition>, UndoError> {
        self.undo_store_inner(store, actor_id, true, true)
    }

    /// Facade path: same as [`Self::undo_checked_store`] but uses [`SceneStore::apply_sparse`]
    /// (no `SimPatch` enqueue — FrameBridge still owns dense Reparent).
    pub fn undo_checked_store_sparse(
        &mut self,
        store: &mut SceneStore,
        actor_id: u64,
    ) -> Result<Option<SlotTransition>, UndoError> {
        self.undo_store_inner(store, actor_id, false, true)
    }

    /// Redo against [`SceneStore`] (fresh MoveNode; enqueues patches on success).
    pub fn redo_store(
        &mut self,
        store: &mut SceneStore,
        actor_id: u64,
    ) -> Result<Option<SlotTransition>, UndoError> {
        self.redo_store_inner(store, actor_id, true)
    }

    /// Facade redo via [`SceneStore::apply_sparse`].
    pub fn redo_store_sparse(
        &mut self,
        store: &mut SceneStore,
        actor_id: u64,
    ) -> Result<Option<SlotTransition>, UndoError> {
        self.redo_store_inner(store, actor_id, false)
    }

    fn undo_store_inner(
        &mut self,
        store: &mut SceneStore,
        actor_id: u64,
        enqueue_patches: bool,
        checked: bool,
    ) -> Result<Option<SlotTransition>, UndoError> {
        let Some(t) = self.undo.pop_back() else {
            return Ok(None);
        };
        // Missing child: `parent()` is always `None`, which would falsely match
        // `t.to == None` (world-root reparent) and jam the stack on RejectedMissing.
        if checked && (!store.contains(t.child) || store.parent(t.child) != t.to) {
            return Err(UndoError::Stale);
        }
        let inv = t.inverse();
        match apply_move(store, actor_id, inv.child, inv.to, enqueue_patches) {
            Ok(()) => {
                self.redo.push_back(t);
                Ok(Some(inv))
            }
            // Stale = discard popped entry (do not restore / jam).
            Err(UndoError::Stale) => Err(UndoError::Stale),
            Err(e) => {
                self.undo.push_back(t);
                Err(e)
            }
        }
    }

    fn redo_store_inner(
        &mut self,
        store: &mut SceneStore,
        actor_id: u64,
        enqueue_patches: bool,
    ) -> Result<Option<SlotTransition>, UndoError> {
        let Some(t) = self.redo.pop_back() else {
            return Ok(None);
        };
        match apply_move(store, actor_id, t.child, t.to, enqueue_patches) {
            Ok(()) => {
                self.undo.push_back(t);
                Ok(Some(t))
            }
            Err(UndoError::Stale) => Err(UndoError::Stale),
            Err(e) => {
                self.redo.push_back(t);
                Err(e)
            }
        }
    }
}

fn apply_move(
    store: &mut SceneStore,
    actor_id: u64,
    child: u32,
    new_parent: Option<u32>,
    enqueue_patches: bool,
) -> Result<(), UndoError> {
    let ts = OpTimestamp::new(store.max_lamport().saturating_add(1), actor_id);
    let op = CrdtOp::MoveNode {
        ts,
        node: child,
        new_parent,
    };
    let outcome = if enqueue_patches {
        store.apply(op)
    } else {
        store.apply_sparse(op)
    };
    map_store_outcome(outcome)
}

fn map_store_outcome(outcome: ApplyOutcome) -> Result<(), UndoError> {
    match outcome {
        ApplyOutcome::Applied | ApplyOutcome::AppliedAsReparent => Ok(()),
        // Lost target / lost LWW race → discard stack entry (same as parent mismatch).
        ApplyOutcome::RejectedStale | ApplyOutcome::RejectedMissing => Err(UndoError::Stale),
        ApplyOutcome::RejectedCycle => Err(UndoError::Reparent(ReparentError::Cycle)),
        ApplyOutcome::RejectedDepthExceeded => {
            Err(UndoError::Reparent(ReparentError::DepthExceeded))
        }
        other => Err(UndoError::Store(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawn(store: &mut SceneStore, lamport: u64, node: u32, parent: Option<u32>) {
        assert!(store
            .apply(CrdtOp::SpawnNode {
                ts: OpTimestamp::new(lamport, 1),
                node,
                parent,
            })
            .is_success());
    }

    #[test]
    fn undo_redo_pass_style_reparent() {
        let mut g = SlotGraph::new();
        let mut stack = SlotUndoStack::new(8);
        // A(1) holds 10 → pass to B(2)
        g.attach_held(1, 10).unwrap();
        let t = SlotTransition {
            child: 10,
            from: Some(1),
            to: Some(2),
        };
        g.reparent(10, Some(2)).unwrap();
        stack.push(t);
        assert_eq!(g.parent(10), Some(2));

        let applied = stack.undo(&mut g).unwrap();
        assert_eq!(applied.map(|x| x.to), Some(Some(1)));
        assert_eq!(g.parent(10), Some(1));

        let applied = stack.redo(&mut g).unwrap();
        assert_eq!(applied.map(|x| x.to), Some(Some(2)));
        assert_eq!(g.parent(10), Some(2));
    }

    #[test]
    fn cap_drops_oldest() {
        let mut g = SlotGraph::new();
        let mut stack = SlotUndoStack::new(2);
        stack.push(SlotTransition {
            child: 1,
            from: None,
            to: Some(10),
        });
        stack.push(SlotTransition {
            child: 2,
            from: None,
            to: Some(10),
        });
        stack.push(SlotTransition {
            child: 3,
            from: None,
            to: Some(10),
        });
        // Oldest (child 1) dropped: first undo applies child 3 inverse.
        g.reparent(3, Some(10)).unwrap();
        let u = stack.undo(&mut g).unwrap().unwrap();
        assert_eq!(u.child, 3);
        assert_eq!(g.parent(3), None);
        let u = stack.undo(&mut g).unwrap().unwrap();
        assert_eq!(u.child, 2);
        assert!(stack.undo(&mut g).unwrap().is_none());
    }

    #[test]
    fn undo_checked_stale_after_foreign_reparent() {
        let mut g = SlotGraph::new();
        let mut stack = SlotUndoStack::new(8);
        g.reparent(10, Some(1)).unwrap();
        stack.push(SlotTransition {
            child: 10,
            from: None,
            to: Some(1),
        });
        // Foreign move: parent is no longer `to`.
        g.reparent(10, Some(2)).unwrap();
        assert_eq!(stack.undo_checked(&mut g), Err(UndoError::Stale));
        assert_eq!(g.parent(10), Some(2));
        assert!(stack.undo.is_empty());
    }

    #[test]
    fn undo_failure_restores_stack() {
        let mut g = SlotGraph::new();
        let mut stack = SlotUndoStack::new(8);
        // Record a transition whose inverse would create a cycle on current graph.
        stack.push(SlotTransition {
            child: 1,
            from: Some(2),
            to: Some(3),
        });
        // Graph: 2 → 1 → 3; undo wants 1 → 2 (cycle).
        g.reparent(1, Some(3)).unwrap();
        g.reparent(2, Some(1)).unwrap();
        assert!(stack.undo(&mut g).is_err());
        // Transition still on undo; redo empty.
        assert_eq!(stack.undo.len(), 1);
        assert!(stack.redo.is_empty());
        assert_eq!(g.parent(1), Some(3));
    }

    #[test]
    fn scene_store_undo_redo_fresh_move_op() {
        let mut store = SceneStore::new();
        let mut stack = SlotUndoStack::new(8);
        spawn(&mut store, 1, 1, None);
        spawn(&mut store, 2, 2, None);
        spawn(&mut store, 3, 10, Some(1));
        store.drain_patches();

        assert!(store
            .apply(CrdtOp::MoveNode {
                ts: OpTimestamp::new(4, 1),
                node: 10,
                new_parent: Some(2),
            })
            .is_success());
        stack.push(SlotTransition {
            child: 10,
            from: Some(1),
            to: Some(2),
        });
        assert_eq!(store.parent(10), Some(2));
        let before_lamport = store.max_lamport();

        let applied = stack.undo_checked_store(&mut store, 7).unwrap().unwrap();
        assert_eq!(applied.to, Some(1));
        assert_eq!(store.parent(10), Some(1));
        assert!(store.max_lamport() > before_lamport);
        assert_eq!(
            store.pending_patches().last(),
            Some(&crate::SimPatch::SetParent {
                slot: 10,
                parent: Some(1),
            })
        );

        let applied = stack.redo_store(&mut store, 7).unwrap().unwrap();
        assert_eq!(applied.to, Some(2));
        assert_eq!(store.parent(10), Some(2));
    }

    #[test]
    fn scene_store_undo_checked_stale_discards_entry() {
        let mut store = SceneStore::new();
        let mut stack = SlotUndoStack::new(8);
        spawn(&mut store, 1, 1, None);
        spawn(&mut store, 2, 2, None);
        spawn(&mut store, 3, 10, None);
        stack.push(SlotTransition {
            child: 10,
            from: None,
            to: Some(1),
        });
        assert!(store
            .apply(CrdtOp::MoveNode {
                ts: OpTimestamp::new(4, 1),
                node: 10,
                new_parent: Some(1),
            })
            .is_success());
        // Foreign move.
        assert!(store
            .apply(CrdtOp::MoveNode {
                ts: OpTimestamp::new(5, 2),
                node: 10,
                new_parent: Some(2),
            })
            .is_success());
        assert_eq!(
            stack.undo_checked_store(&mut store, 1),
            Err(UndoError::Stale)
        );
        assert_eq!(store.parent(10), Some(2));
        assert_eq!(stack.undo_len(), 0);
    }

    #[test]
    fn scene_store_sparse_undo_does_not_enqueue_patches() {
        let mut store = SceneStore::new();
        let mut stack = SlotUndoStack::new(8);
        spawn(&mut store, 1, 1, None);
        spawn(&mut store, 2, 2, None);
        spawn(&mut store, 3, 10, Some(1));
        store.drain_patches();
        assert!(store
            .apply_sparse(CrdtOp::MoveNode {
                ts: OpTimestamp::new(4, 1),
                node: 10,
                new_parent: Some(2),
            })
            .is_success());
        stack.push(SlotTransition {
            child: 10,
            from: Some(1),
            to: Some(2),
        });
        assert!(stack
            .undo_checked_store_sparse(&mut store, 1)
            .unwrap()
            .is_some());
        assert_eq!(store.parent(10), Some(1));
        assert!(store.pending_patches().is_empty());
    }

    #[test]
    fn scene_store_undo_checked_discards_when_root_reparent_then_despawn() {
        // Regression: parent() of missing id is None, which equals t.to == None
        // and used to slip past stale → RejectedMissing restored → stack jam.
        let mut store = SceneStore::new();
        let mut stack = SlotUndoStack::new(8);
        spawn(&mut store, 1, 1, None);
        spawn(&mut store, 2, 10, Some(1));
        assert!(store
            .apply(CrdtOp::MoveNode {
                ts: OpTimestamp::new(3, 1),
                node: 10,
                new_parent: None,
            })
            .is_success());
        stack.push(SlotTransition {
            child: 10,
            from: Some(1),
            to: None,
        });
        assert!(store.remove_node_sparse(10));
        assert_eq!(
            stack.undo_checked_store(&mut store, 1),
            Err(UndoError::Stale)
        );
        assert_eq!(stack.undo_len(), 0);
        // Later entries still reachable.
        stack.push(SlotTransition {
            child: 99,
            from: None,
            to: Some(1),
        });
        assert_eq!(stack.undo_len(), 1);
    }

    #[test]
    fn discard_child_clears_undo_and_redo() {
        let mut g = SlotGraph::new();
        let mut stack = SlotUndoStack::new(8);
        g.reparent(10, Some(1)).unwrap();
        stack.push(SlotTransition {
            child: 10,
            from: None,
            to: Some(1),
        });
        // Undo → entry on redo. Do not `push` next (that would `redo.clear()`).
        assert!(stack.undo(&mut g).unwrap().is_some());
        assert_eq!(stack.undo_len(), 0);
        assert_eq!(stack.redo_len(), 1);
        stack.discard_child(10);
        assert_eq!(stack.redo_len(), 0);

        // Undo deque path.
        stack.push(SlotTransition {
            child: 11,
            from: None,
            to: Some(1),
        });
        assert_eq!(stack.undo_len(), 1);
        stack.discard_child(11);
        assert_eq!(stack.undo_len(), 0);
        assert_eq!(stack.redo_len(), 0);
    }
}
