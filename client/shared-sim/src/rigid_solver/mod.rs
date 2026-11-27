//! RigidSolver — Ring 2 Step 2 rigid-body boundary only (x00e9).
//!
//! Guarantees: object-safe [`RigidSolver`] trait + [`NoopRigidSolver`];
//! reserved [`COMP_RIGID_BODY`] (`1 << 9`, roadmap §6); document that a future
//! solver swaps in at Step 2 without deleting Native XPBD / inequality contact
//! in this slice.
//! Non-guarantees: rapier3d / AVBD (unadopted — human approval for new deps);
//! host 60Hz wiring; Sim Arena / SparseSet consumption of `COMP_RIGID_BODY`;
//! Scale / CCU KPI; replacing [`crate::xpbd`].
//!
//! # Ring 2 placement
//!
//! Tick loop calls **[`step2_rigid`]** (canonical Step 2 entry).
//! [`RigidSolver::step`] is the implementor surface only. This slice leaves
//! host / client tick **unwired** — unit tests exercise the Noop path only.
//! Live physics remains [`crate::xpbd::step`] until a chosen backend is wired
//! separately. Stable contract: state injected into `Self`; see
//! [`RigidSolver`].

#![forbid(unsafe_code)]

mod solver;

pub use solver::{step2_rigid, NoopRigidSolver, RigidSolver, COMP_RIGID_BODY};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_step_is_callable() {
        let mut noop = NoopRigidSolver;
        // Canonical tick entry (not RigidSolver::step directly).
        step2_rigid(&mut noop, 1.0 / 60.0);
    }

    #[test]
    fn comp_rigid_body_matches_roadmap_bit() {
        assert_eq!(COMP_RIGID_BODY, 1 << 9);
    }

    #[test]
    fn dyn_rigid_solver_accepts_noop() {
        let mut boxed: Box<dyn RigidSolver> = Box::new(NoopRigidSolver);
        step2_rigid(boxed.as_mut(), 0.016);
    }
}
