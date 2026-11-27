//! PendingLocalOps + Error Smoothing (x00e6) — Ring 3 display-only lag compensation.
//!
//! Guarantees: Keep / Blend / Snap against authority samples; `teleport=true` or
//! Snap **removes** the pending entry so display falls back to Epoch / GPU path;
//! never mutates Published / [`EpochSnapshotPair`] Arcs (no stored refs to sheets;
//! reconcile takes poses by value).
//! Non-guarantees: host authority loop changes; Arena `TELEPORT_THIS_TICK` → GPU
//! wiring; laser / physical-button live paths; replacing Grab [`crate::owned_prediction`];
//! rotation-only error metric (translation distance only — same as owned_prediction).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use alchemy_shared_sim::bridge::EpochSnapshotPair;
use alchemy_shared_sim::tape::WorldTransformf64;

/// How predicted pose is reconciled with host authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconcileMode {
    /// Prediction wins (error ≤ keep threshold).
    Keep,
    /// Soft pull toward authority (keep < error ≤ blend).
    Blend,
    /// Hard snap to authority (reject, large error, teleport, or explicit snap).
    Snap,
}

/// Kind of pending local prediction. Pose only in x00e6; Property etc. later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingKind {
    Pose,
}

/// Default keep distance (meters). Canonical source; Grab path re-exports via
/// [`crate::owned_prediction`].
pub const DEFAULT_KEEP_THRESHOLD_M: f32 = 0.05;
/// Beyond this, snap instead of blend.
pub const DEFAULT_BLEND_THRESHOLD_M: f32 = 0.35;
/// Blend factor toward authority when in [`ReconcileMode::Blend`].
pub const DEFAULT_BLEND_ALPHA: f32 = 0.35;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PendingPose {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub qx: f32,
    pub qy: f32,
    pub qz: f32,
    pub qw: f32,
}

impl PendingPose {
    pub fn xyz(x: f32, y: f32, z: f32) -> Self {
        Self {
            x,
            y,
            z,
            qx: 0.0,
            qy: 0.0,
            qz: 0.0,
            qw: 1.0,
        }
    }

    pub fn xyz_quat(x: f32, y: f32, z: f32, qx: f32, qy: f32, qz: f32, qw: f32) -> Self {
        Self {
            x,
            y,
            z,
            qx,
            qy,
            qz,
            qw,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn from_world(xf: &WorldTransformf64) -> Self {
        Self {
            x: xf.position[0] as f32,
            y: xf.position[1] as f32,
            z: xf.position[2] as f32,
            qx: xf.rotation[0],
            qy: xf.rotation[1],
            qz: xf.rotation[2],
            qw: xf.rotation[3],
        }
    }

    pub fn distance_xyz(self, other: Self) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        let dz = self.z - other.z;
        (dx * dx + dy * dy + dz * dz).sqrt()
    }

    /// Hemisphere-aware nlerp (same path for near-parallel quats).
    pub fn lerp_toward(self, authority: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let mut q_pred = [self.qx, self.qy, self.qz, self.qw];
        let q_auth = [authority.qx, authority.qy, authority.qz, authority.qw];
        let mut dot = q_pred[0] * q_auth[0]
            + q_pred[1] * q_auth[1]
            + q_pred[2] * q_auth[2]
            + q_pred[3] * q_auth[3];
        if dot < 0.0 {
            q_pred = [-q_pred[0], -q_pred[1], -q_pred[2], -q_pred[3]];
            dot = -dot;
        }
        let _ = dot;
        let qx = q_pred[0] + (q_auth[0] - q_pred[0]) * t;
        let qy = q_pred[1] + (q_auth[1] - q_pred[1]) * t;
        let qz = q_pred[2] + (q_auth[2] - q_pred[2]) * t;
        let qw = q_pred[3] + (q_auth[3] - q_pred[3]) * t;
        let inv = 1.0 / (qx * qx + qy * qy + qz * qz + qw * qw).sqrt().max(1e-6);
        Self {
            x: self.x + (authority.x - self.x) * t,
            y: self.y + (authority.y - self.y) * t,
            z: self.z + (authority.z - self.z) * t,
            qx: qx * inv,
            qy: qy * inv,
            qz: qz * inv,
            qw: qw * inv,
        }
    }
}

/// Thresholds for Keep / Blend / Snap (Grab-agnostic).
#[derive(Clone, Copy, Debug)]
pub struct ErrorSmoothing {
    pub keep_threshold_m: f32,
    pub blend_threshold_m: f32,
    pub blend_alpha: f32,
}

impl Default for ErrorSmoothing {
    fn default() -> Self {
        Self {
            keep_threshold_m: DEFAULT_KEEP_THRESHOLD_M,
            blend_threshold_m: DEFAULT_BLEND_THRESHOLD_M,
            blend_alpha: DEFAULT_BLEND_ALPHA,
        }
    }
}

impl ErrorSmoothing {
    /// Reconcile prediction with authority. `teleport` forces Snap (skip Blend).
    ///
    /// Error metric is translation-only (parity with owned_prediction); rotation
    /// divergence alone does not leave Keep.
    pub fn reconcile(
        &self,
        predicted: PendingPose,
        authority: PendingPose,
        teleport: bool,
    ) -> (PendingPose, ReconcileMode) {
        if teleport {
            return (authority, ReconcileMode::Snap);
        }
        let err = predicted.distance_xyz(authority);
        if err <= self.keep_threshold_m {
            (predicted, ReconcileMode::Keep)
        } else if err <= self.blend_threshold_m {
            (
                predicted.lerp_toward(authority, self.blend_alpha),
                ReconcileMode::Blend,
            )
        } else {
            (authority, ReconcileMode::Snap)
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct PendingEntry {
    kind: PendingKind,
    predicted: PendingPose,
}

/// Per-entity pending local predictions + error smoothing (display only).
///
/// Holds **no** references to Published / Epoch sheets.
#[derive(Debug)]
pub struct PendingLocalOps {
    entries: HashMap<u32, PendingEntry>,
    pub smoothing: ErrorSmoothing,
}

impl Default for PendingLocalOps {
    fn default() -> Self {
        Self::new()
    }
}

impl PendingLocalOps {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            smoothing: ErrorSmoothing::default(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Push or replace a pose prediction for `entity_id`.
    pub fn push_pose(&mut self, entity_id: u32, pose: PendingPose) {
        self.entries.insert(
            entity_id,
            PendingEntry {
                kind: PendingKind::Pose,
                predicted: pose,
            },
        );
    }

    pub fn clear(&mut self, entity_id: u32) {
        self.entries.remove(&entity_id);
    }

    /// Reconcile one entity against an authority sample.
    ///
    /// Keep / Blend → update predicted in place. Snap (incl. `teleport`) →
    /// **remove** the entry so Epoch / GPU path owns display again.
    /// Until Arena TELEPORT reaches GPU interp, that fallback may still lerp for
    /// a frame (slice non-guarantee). Never touches Published Arcs.
    pub fn reconcile(
        &mut self,
        entity_id: u32,
        authority: PendingPose,
        teleport: bool,
    ) -> Option<(PendingPose, ReconcileMode)> {
        let predicted = self.entries.get(&entity_id)?.predicted;
        let (out, mode) = self.smoothing.reconcile(predicted, authority, teleport);
        if mode == ReconcileMode::Snap {
            self.entries.remove(&entity_id);
        } else if let Some(entry) = self.entries.get_mut(&entity_id) {
            entry.predicted = out;
        }
        Some((out, mode))
    }

    /// Read-only authority from a published sheet Arc, then reconcile.
    ///
    /// Requires `curr.owner_scope[row_id] == entity_id` so a wrong row cannot
    /// supply authority. Shared-ref only — does not mutate Published.
    pub fn reconcile_from_published(
        &mut self,
        snapshots: &EpochSnapshotPair,
        entity_id: u32,
        row_id: u32,
        teleport: bool,
    ) -> Option<(PendingPose, ReconcileMode)> {
        let faces = snapshots.load_pair();
        let curr = &faces.curr;
        let idx = row_id as usize;
        if idx >= curr.world_transforms.len() || idx >= curr.owner_scope.len() {
            return None;
        }
        if curr.owner_scope[idx] != entity_id {
            return None;
        }
        let authority = PendingPose::from_world(&curr.world_transforms[idx]);
        self.reconcile(entity_id, authority, teleport)
    }

    /// Display pose override, or [`None`] → Epoch / other path.
    pub fn display_override(&self, entity_id: u32) -> Option<PendingPose> {
        self.entries.get(&entity_id).map(|e| {
            debug_assert_eq!(e.kind, PendingKind::Pose);
            e.predicted
        })
    }

    #[cfg(test)]
    pub fn has_override(&self, entity_id: u32) -> bool {
        self.entries.contains_key(&entity_id)
    }
}

/// Mutex + Release/Acquire emptiness gate — sole mutation path for pending ops.
///
/// Callers cannot touch the map without updating [`Self::any_pending`]. Used by
/// [`crate::scene::SharedState`] so the draw hot path can skip when empty.
#[derive(Debug)]
pub struct PendingLocalGate {
    inner: Mutex<PendingLocalOps>,
    any: AtomicBool,
}

impl Default for PendingLocalGate {
    fn default() -> Self {
        Self::new()
    }
}

impl PendingLocalGate {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(PendingLocalOps::new()),
            any: AtomicBool::new(false),
        }
    }

    /// `Acquire` — pairs with `Release` stores after map mutations.
    pub fn any_pending(&self) -> bool {
        self.any.load(Ordering::Acquire)
    }

    fn sync_any(&self, ops: &PendingLocalOps) {
        self.any.store(!ops.is_empty(), Ordering::Release);
    }

    pub fn push_pose(&self, entity_id: u32, pose: PendingPose) {
        let Ok(mut ops) = self.inner.lock() else {
            return;
        };
        ops.push_pose(entity_id, pose);
        self.sync_any(&ops);
    }

    pub fn clear(&self, entity_id: u32) {
        let Ok(mut ops) = self.inner.lock() else {
            return;
        };
        ops.clear(entity_id);
        self.sync_any(&ops);
    }

    pub fn reconcile(
        &self,
        entity_id: u32,
        authority: PendingPose,
        teleport: bool,
    ) -> Option<(PendingPose, ReconcileMode)> {
        let Ok(mut ops) = self.inner.lock() else {
            return None;
        };
        let out = ops.reconcile(entity_id, authority, teleport);
        self.sync_any(&ops);
        out
    }

    pub fn reconcile_from_published(
        &self,
        snapshots: &EpochSnapshotPair,
        entity_id: u32,
        row_id: u32,
        teleport: bool,
    ) -> Option<(PendingPose, ReconcileMode)> {
        let Ok(mut ops) = self.inner.lock() else {
            return None;
        };
        let out = ops.reconcile_from_published(snapshots, entity_id, row_id, teleport);
        self.sync_any(&ops);
        out
    }

    pub fn display_override(&self, entity_id: u32) -> Option<PendingPose> {
        let Ok(ops) = self.inner.lock() else {
            return None;
        };
        ops.display_override(entity_id)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use alchemy_shared_sim::tape::EpochSheet;

    use super::*;

    fn mid_error_pair() -> (PendingPose, PendingPose) {
        // ~0.2 m — inside blend band with defaults.
        (
            PendingPose::xyz(0.2, 0.0, 0.0),
            PendingPose::xyz(0.0, 0.0, 0.0),
        )
    }

    #[test]
    fn gate_release_acquire_push_snap_clear() {
        let gate = PendingLocalGate::new();
        assert!(!gate.any_pending());

        gate.push_pose(1, PendingPose::xyz(0.2, 0.0, 0.0));
        assert!(gate.any_pending());
        assert!(gate.display_override(1).is_some());

        let (_, mode) = gate
            .reconcile(1, PendingPose::xyz(0.0, 0.0, 0.0), true)
            .unwrap();
        assert_eq!(mode, ReconcileMode::Snap);
        assert!(
            !gate.any_pending(),
            "Snap must Release-store false so draw skips mutex"
        );
        assert!(gate.display_override(1).is_none());

        gate.push_pose(2, PendingPose::xyz(0.1, 0.0, 0.0));
        assert!(gate.any_pending());
        gate.clear(2);
        assert!(!gate.any_pending());
    }

    #[test]
    fn teleport_skips_blend_and_snaps() {
        let smoothing = ErrorSmoothing::default();
        let (pred, auth) = mid_error_pair();

        let (out_blend, mode_blend) = smoothing.reconcile(pred, auth, false);
        assert_eq!(mode_blend, ReconcileMode::Blend);
        assert!(out_blend.x < pred.x && out_blend.x > auth.x);

        let (out_tp, mode_tp) = smoothing.reconcile(pred, auth, true);
        assert_eq!(mode_tp, ReconcileMode::Snap);
        assert!((out_tp.x - auth.x).abs() < 1e-5);

        let mut ops = PendingLocalOps::new();
        ops.push_pose(7, pred);
        let (out, mode) = ops.reconcile(7, auth, true).unwrap();
        assert_eq!(mode, ReconcileMode::Snap);
        assert!((out.x - auth.x).abs() < 1e-5);
        assert!(
            ops.display_override(7).is_none(),
            "Snap/teleport removes entry so Epoch path owns display"
        );
        assert!(ops.is_empty());
    }

    #[test]
    fn large_error_snap_clears_entry() {
        let mut ops = PendingLocalOps::new();
        ops.push_pose(3, PendingPose::xyz(2.0, 0.0, 0.0));
        let (out, mode) = ops
            .reconcile(3, PendingPose::xyz(0.0, 0.0, 0.0), false)
            .unwrap();
        assert_eq!(mode, ReconcileMode::Snap);
        assert!((out.x - 0.0).abs() < 1e-5);
        assert!(!ops.has_override(3));
    }

    #[test]
    fn smoothing_does_not_mutate_published_arc() {
        let mut sheet = EpochSheet::with_capacity(4);
        let h = sheet.alloc_row();
        sheet
            .set_transform(h.row_id, WorldTransformf64::identity_at([3.0, 0.0, 0.0]))
            .unwrap();
        sheet.owner_scope[h.row_id as usize] = 42;

        let snaps = EpochSnapshotPair::new(sheet.clone(), sheet);
        let before = snaps.load_pair();
        let curr_ptr = Arc::as_ptr(&before.curr);
        let prev_ptr = Arc::as_ptr(&before.prev);
        let faces_ptr = Arc::as_ptr(&before);
        let pos_before = before.curr.world_transforms[h.row_id as usize].position;

        let mut ops = PendingLocalOps::new();
        ops.push_pose(42, PendingPose::xyz(3.2, 0.0, 0.0));
        let (out, mode) = ops
            .reconcile_from_published(&snaps, 42, h.row_id, false)
            .unwrap();
        assert_eq!(mode, ReconcileMode::Blend);
        assert!(out.x < 3.2);

        let after = snaps.load_pair();
        assert!(
            Arc::ptr_eq(&before, &after),
            "EpochFaces Arc must be unchanged"
        );
        assert_eq!(Arc::as_ptr(&after), faces_ptr);
        assert_eq!(Arc::as_ptr(&after.curr), curr_ptr);
        assert_eq!(Arc::as_ptr(&after.prev), prev_ptr);
        assert_eq!(
            after.curr.world_transforms[h.row_id as usize].position, pos_before,
            "sheet transform must not be rewritten"
        );
        assert!(ops.has_override(42));
    }

    #[test]
    fn reconcile_from_published_rejects_owner_mismatch() {
        let mut sheet = EpochSheet::with_capacity(4);
        let h = sheet.alloc_row();
        sheet
            .set_transform(h.row_id, WorldTransformf64::identity_at([1.0, 0.0, 0.0]))
            .unwrap();
        sheet.owner_scope[h.row_id as usize] = 99;

        let snaps = EpochSnapshotPair::new(sheet.clone(), sheet);
        let mut ops = PendingLocalOps::new();
        ops.push_pose(42, PendingPose::xyz(1.2, 0.0, 0.0));
        assert!(ops
            .reconcile_from_published(&snaps, 42, h.row_id, false)
            .is_none());
        assert!(ops.has_override(42), "mismatch must not clear pending");
    }

    #[test]
    fn keep_within_threshold() {
        let smoothing = ErrorSmoothing::default();
        let pred = PendingPose::xyz(0.02, 0.0, 0.0);
        let auth = PendingPose::xyz(0.0, 0.0, 0.0);
        let (out, mode) = smoothing.reconcile(pred, auth, false);
        assert_eq!(mode, ReconcileMode::Keep);
        assert!((out.x - 0.02).abs() < 1e-5);
    }

    #[test]
    fn large_error_snaps_without_teleport() {
        let smoothing = ErrorSmoothing::default();
        let pred = PendingPose::xyz(2.0, 0.0, 0.0);
        let auth = PendingPose::xyz(0.0, 0.0, 0.0);
        let (out, mode) = smoothing.reconcile(pred, auth, false);
        assert_eq!(mode, ReconcileMode::Snap);
        assert!((out.x - 0.0).abs() < 1e-5);
    }
}
