//! RigidSolver trait + Noop (x00e9 Phase 3 boundary).
//!
//! Guarantees: object-safe [`RigidSolver`] step hook; [`NoopRigidSolver`];
//! [`COMP_RIGID_BODY`] bit reserved per roadmap §6.
//! Non-guarantees: Arena / SparseSet consumption of the bit; any live solver body.

#![forbid(unsafe_code)]

/// Component mask bit reserved for rigid bodies (roadmap §6 Phase 3 枠).
///
/// **Placement policy:** lives here until a shared §6 `COMP_*` / `STATE_*`
/// mask module (or `sim_arena`) adopts the full family. Do not redefine this
/// bit elsewhere — migrate / `pub use` this constant when centralizing.
/// Not wired into [`crate::sim_arena`] (`COMP_*` intentional defer there).
pub const COMP_RIGID_BODY: u64 = 1 << 9;

/// Ring 2 Step 2 rigid-body plug-in point.
///
/// **Stable contract (x00e9):** implementors own Work / sheet / collider
/// handles inside `Self` (ctor or setter injection). The trait surface is
/// `step(dt)` only ([`Self::step`]) — call-site face slices are **not** part
/// of this boundary. A later slice that needs shared borrow-at-call would
/// introduce a new trait / context type rather than silently widening this one.
pub trait RigidSolver {
    /// Advance rigid bodies for one tick (see trait contract above).
    ///
    /// [`NoopRigidSolver`] leaves the live Native XPBD path unchanged.
    /// Tick callers should use [`step2_rigid`] (canonical Ring 2 entry).
    fn step(&mut self, dt: f64);
}

/// Default Step 2 plug-in: no-op (host tick remains unwired / Native XPBD).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoopRigidSolver;

impl RigidSolver for NoopRigidSolver {
    fn step(&mut self, _dt: f64) {}
}

/// Canonical Ring 2 **Step 2** call-site entry (named hook for host wiring).
///
/// Forwards to [`RigidSolver::step`]. Prefer this over calling `step` directly
/// from the tick loop so the Step 2 seam stays searchable. Host / client tick
/// does **not** call this yet (x00e9 boundary only).
#[inline]
pub fn step2_rigid<S: RigidSolver + ?Sized>(solver: &mut S, dt: f64) {
    solver.step(dt);
}
