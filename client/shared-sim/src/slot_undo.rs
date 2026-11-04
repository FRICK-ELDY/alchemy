//! Per-operator session undo/redo for Slot reparent transitions (x005b).
//!
//! Guarantees: reverse/reapply last reparent recorded for an operator stack;
//! stack cap with drop-oldest on overflow.
//! Non-guarantees: persistence across leave; undoing another operator's moves;
//! physics velocity restore; UI wiring.

use crate::slot_graph::{ReparentError, SlotGraph};

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
    undo: Vec<SlotTransition>,
    redo: Vec<SlotTransition>,
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
            undo: Vec::new(),
            redo: Vec::new(),
            cap: cap.max(1),
        }
    }

    pub fn push(&mut self, t: SlotTransition) {
        if self.undo.len() >= self.cap {
            self.undo.remove(0);
        }
        self.undo.push(t);
        self.redo.clear();
    }

    pub fn undo(&mut self, graph: &mut SlotGraph) -> Result<Option<SlotTransition>, ReparentError> {
        let Some(t) = self.undo.pop() else {
            return Ok(None);
        };
        let inv = t.inverse();
        graph.reparent(inv.child, inv.to)?;
        self.redo.push(t);
        Ok(Some(inv))
    }

    pub fn redo(&mut self, graph: &mut SlotGraph) -> Result<Option<SlotTransition>, ReparentError> {
        let Some(t) = self.redo.pop() else {
            return Ok(None);
        };
        graph.reparent(t.child, t.to)?;
        self.undo.push(t);
        Ok(Some(t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo_pass_style_reparent() {
        let mut g = SlotGraph::new();
        let mut stack = SlotUndoStack::new(8);
        // A(1) holds 10 → pass to B(2)
        g.attach_held(1, 10);
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
}
