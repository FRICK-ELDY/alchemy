//! Local prediction for owned Grab entities (x00ab).
//!
//! Guarantees: Keep / Blend / Snap reconcile against host authority; Grab reject
//! forces Snap; under Keep the display pose stays within `keep_threshold_m` of the
//! predicted pose; [`display_override`] is consumed by hit ([`crate::scene::SharedState::grabbable_display_poses`])
//! and draw (`model_override` when pack would otherwise ignore CPU TRS).
//! Non-guarantees: full `acked_intent_seq` / `Entity.holder` Snap wiring (fields on
//! wire as of x00af; reject still uses authority-distance after a short grace);
//! user_root loco prediction (Epoch eye path); rewriting Epoch GPU storage rows.

/// How predicted pose is reconciled with host authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconcileMode {
    /// Prediction wins (error ≤ keep threshold).
    Keep,
    /// Soft pull toward authority (keep < error ≤ blend).
    Blend,
    /// Hard snap to authority (reject, large error, or explicit snap).
    Snap,
}

/// Default keep distance (meters) — held object stays with the hand.
pub const DEFAULT_KEEP_THRESHOLD_M: f32 = 0.05;
/// Beyond this, snap instead of blend. Also the post-grace host-accept radius.
pub const DEFAULT_BLEND_THRESHOLD_M: f32 = 0.35;
/// Blend factor toward authority when in [`ReconcileMode::Blend`].
pub const DEFAULT_BLEND_ALPHA: f32 = 0.35;
/// Optimistic Applied Frame count before reject distance is checked.
/// With `N`, frames `1..=N` stay accepted; reject is eligible from frame `N+1`
/// (`held_auth_frames <= N` → optimistic). Default 2 → reject from the 3rd Applied.
pub const DEFAULT_REJECT_GRACE_FRAMES: u32 = 2;

#[derive(Clone, Copy, Debug)]
pub struct PredictionPose {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub qx: f32,
    pub qy: f32,
    pub qz: f32,
    pub qw: f32,
}

impl PredictionPose {
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

    pub fn distance_xyz(self, other: Self) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        let dz = self.z - other.z;
        (dx * dx + dy * dy + dz * dz).sqrt()
    }

    /// Hemisphere-aware nlerp (same path for near-parallel quats; no dead slerp branch).
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
        let _ = dot; // hemisphere fix only; nlerp is stable for near-1 dots
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

/// Reconcile local prediction with host authority.
///
/// `authority_accepted == false` (Grab reject / authority NACK) always Snaps.
/// Grab hold path uses [`OwnedPrediction::reconcile_held`] (Keep on large lag while accepted).
#[cfg_attr(not(test), allow(dead_code))]
pub fn reconcile_owned(
    predicted: PredictionPose,
    authority: PredictionPose,
    authority_accepted: bool,
    keep_threshold_m: f32,
    blend_threshold_m: f32,
    blend_alpha: f32,
) -> (PredictionPose, ReconcileMode) {
    if !authority_accepted {
        return (authority, ReconcileMode::Snap);
    }
    let err = predicted.distance_xyz(authority);
    if err <= keep_threshold_m {
        (predicted, ReconcileMode::Keep)
    } else if err <= blend_threshold_m {
        (
            predicted.lerp_toward(authority, blend_alpha),
            ReconcileMode::Blend,
        )
    } else {
        (authority, ReconcileMode::Snap)
    }
}

/// Whether host appears to have accepted the Grab (distance heuristic).
///
/// Wire carries `acked_intent_seq` / `Entity.holder` as of x00af, but this helper
/// still uses authority distance after grace — callers have not switched yet.
/// `auth_frames_since_grab <= grace_frames` → optimistic accept (no distance check).
pub fn grab_accepted_heuristic(
    auth_frames_since_grab: u32,
    grace_frames: u32,
    predicted: PredictionPose,
    authority: PredictionPose,
    blend_threshold_m: f32,
) -> bool {
    if auth_frames_since_grab <= grace_frames {
        return true;
    }
    predicted.distance_xyz(authority) <= blend_threshold_m
}

/// Tracks predicted pose for the locally held Grab entity only.
///
/// `user_root` is intentionally **not** overridden here (loco stays Epoch eye path)
/// to avoid replacing interpolated display with authority-seeded poses.
#[derive(Debug)]
pub struct OwnedPrediction {
    pub held_entity: Option<u32>,
    pub held_pose: Option<PredictionPose>,
    /// Applied Frame count while the current hold has been active.
    pub held_auth_frames: u32,
    pub reject_grace_frames: u32,
    /// Set when reconcile Snaps due to reject; Desktop clears local `held_grab`.
    grab_rejected: bool,
    pub keep_threshold_m: f32,
    pub blend_threshold_m: f32,
    pub blend_alpha: f32,
}

impl Default for OwnedPrediction {
    fn default() -> Self {
        Self::new()
    }
}

impl OwnedPrediction {
    pub fn new() -> Self {
        Self {
            held_entity: None,
            held_pose: None,
            held_auth_frames: 0,
            reject_grace_frames: DEFAULT_REJECT_GRACE_FRAMES,
            grab_rejected: false,
            keep_threshold_m: DEFAULT_KEEP_THRESHOLD_M,
            blend_threshold_m: DEFAULT_BLEND_THRESHOLD_M,
            blend_alpha: DEFAULT_BLEND_ALPHA,
        }
    }

    /// Clear or replace the active hold. Does **not** clear [`Self::grab_rejected`] —
    /// only [`Self::take_grab_rejected`] does. Cannot begin a new hold while rejected.
    pub fn set_held(&mut self, id: Option<u32>) {
        if id.is_some() && self.grab_rejected {
            return;
        }
        if self.held_entity != id {
            self.held_pose = None;
            self.held_auth_frames = 0;
        }
        self.held_entity = id;
    }

    /// Begin a new local hold (Desktop Grab start).
    /// Returns `false` while `grab_rejected` is set (caller must not set local held state).
    pub fn begin_held(&mut self, entity_id: u32, pose: PredictionPose) -> bool {
        if self.grab_rejected {
            return false;
        }
        self.held_entity = Some(entity_id);
        self.held_pose = Some(pose);
        self.held_auth_frames = 0;
        true
    }

    /// Update pose for the **existing** hold only. Never starts a hold and never
    /// clears `grab_rejected` (avoids resurrecting after Applied-Frame reject).
    pub fn predict_held(&mut self, entity_id: u32, pose: PredictionPose) {
        if self.grab_rejected || self.held_entity != Some(entity_id) {
            return;
        }
        self.held_pose = Some(pose);
    }

    /// Missing authority entity (despawn / gap) → same as host reject.
    pub fn force_reject(&mut self) {
        if self.held_entity.is_none() && self.held_pose.is_none() {
            return;
        }
        self.grab_rejected = true;
        self.held_entity = None;
        self.held_pose = None;
        self.held_auth_frames = 0;
    }

    pub fn has_pending_reject(&self) -> bool {
        self.grab_rejected
    }

    /// Authority reconcile on Applied Frame only.
    ///
    /// While host appears to accept the Grab, large auth error **Keeps** the hand
    /// prediction (do not Snap the held object back to a distant rest pose). Snap +
    /// clear hold only when the reject heuristic fires (`!accepted`).
    pub fn reconcile_held(
        &mut self,
        authority: PredictionPose,
    ) -> Option<(PredictionPose, ReconcileMode)> {
        let predicted = self.held_pose?;
        self.held_auth_frames = self.held_auth_frames.saturating_add(1);
        let accepted = grab_accepted_heuristic(
            self.held_auth_frames,
            self.reject_grace_frames,
            predicted,
            authority,
            self.blend_threshold_m,
        );
        if !accepted {
            self.grab_rejected = true;
            self.held_entity = None;
            self.held_pose = None;
            self.held_auth_frames = 0;
            return Some((authority, ReconcileMode::Snap));
        }
        let err = predicted.distance_xyz(authority);
        let (out, mode) = if err <= self.keep_threshold_m {
            (predicted, ReconcileMode::Keep)
        } else if err <= self.blend_threshold_m {
            (
                predicted.lerp_toward(authority, self.blend_alpha),
                ReconcileMode::Blend,
            )
        } else {
            // Accepted but auth still catching up — keep hand attachment.
            (predicted, ReconcileMode::Keep)
        };
        self.held_pose.replace(out);
        Some((out, mode))
    }

    pub fn take_grab_rejected(&mut self) -> bool {
        let v = self.grab_rejected;
        self.grab_rejected = false;
        v
    }

    /// Display pose for the held entity, or [`None`] (→ Epoch/wire path).
    pub fn display_override(&self, entity_id: u32) -> Option<PredictionPose> {
        if Some(entity_id) == self.held_entity {
            self.held_pose
        } else {
            None
        }
    }

    pub fn has_override(&self, entity_id: u32) -> bool {
        self.display_override(entity_id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grab_reject_snaps_to_authority() {
        let pred = PredictionPose::xyz(1.0, 0.0, 0.0);
        let auth = PredictionPose::xyz(0.0, 0.0, 0.0);
        let (out, mode) = reconcile_owned(
            pred,
            auth,
            false,
            DEFAULT_KEEP_THRESHOLD_M,
            DEFAULT_BLEND_THRESHOLD_M,
            DEFAULT_BLEND_ALPHA,
        );
        assert_eq!(mode, ReconcileMode::Snap);
        assert!((out.x - 0.0).abs() < 1e-5);
    }

    #[test]
    fn keep_holds_within_threshold_under_jitter() {
        let pred = PredictionPose::xyz(0.02, 0.0, 0.0);
        let auth = PredictionPose::xyz(0.0, 0.0, 0.0);
        let (out, mode) = reconcile_owned(
            pred,
            auth,
            true,
            DEFAULT_KEEP_THRESHOLD_M,
            DEFAULT_BLEND_THRESHOLD_M,
            DEFAULT_BLEND_ALPHA,
        );
        assert_eq!(mode, ReconcileMode::Keep);
        assert!((out.x - 0.02).abs() < 1e-5);
        assert!(out.distance_xyz(pred) <= DEFAULT_KEEP_THRESHOLD_M);
    }

    #[test]
    fn large_error_snaps_when_accepted_flag_true() {
        let pred = PredictionPose::xyz(2.0, 0.0, 0.0);
        let auth = PredictionPose::xyz(0.0, 0.0, 0.0);
        let (_, mode) = reconcile_owned(
            pred,
            auth,
            true,
            DEFAULT_KEEP_THRESHOLD_M,
            DEFAULT_BLEND_THRESHOLD_M,
            DEFAULT_BLEND_ALPHA,
        );
        assert_eq!(mode, ReconcileMode::Snap);
    }

    #[test]
    fn blend_pulls_toward_authority() {
        let pred = PredictionPose::xyz(0.2, 0.0, 0.0);
        let auth = PredictionPose::xyz(0.0, 0.0, 0.0);
        let (out, mode) = reconcile_owned(
            pred,
            auth,
            true,
            DEFAULT_KEEP_THRESHOLD_M,
            DEFAULT_BLEND_THRESHOLD_M,
            DEFAULT_BLEND_ALPHA,
        );
        assert_eq!(mode, ReconcileMode::Blend);
        assert!(out.x < pred.x && out.x > auth.x);
    }

    #[test]
    fn predict_does_not_reconcile_away_from_hand() {
        let mut owned = OwnedPrediction::new();
        owned.begin_held(42, PredictionPose::xyz(1.0, 2.0, 3.0));
        owned.predict_held(42, PredictionPose::xyz(1.0, 2.0, 3.0));
        assert_eq!(owned.display_override(42).unwrap().x, 1.0);
    }

    #[test]
    fn predict_held_does_not_resurrect_after_reject() {
        let mut owned = OwnedPrediction::new();
        owned.begin_held(42, PredictionPose::xyz(1.0, 0.0, 0.0));
        owned.reject_grace_frames = 0;
        let _ = owned.reconcile_held(PredictionPose::xyz(0.0, 0.0, 0.0));
        assert!(owned.has_pending_reject());
        owned.predict_held(42, PredictionPose::xyz(1.0, 0.0, 0.0));
        owned.begin_held(42, PredictionPose::xyz(1.0, 0.0, 0.0));
        owned.set_held(Some(42));
        assert!(owned.held_entity.is_none());
        assert!(owned.has_pending_reject());
        assert!(owned.take_grab_rejected());
        owned.begin_held(42, PredictionPose::xyz(1.0, 0.0, 0.0));
        assert_eq!(owned.held_entity, Some(42));
    }

    #[test]
    fn set_held_none_does_not_clear_reject_flag() {
        let mut owned = OwnedPrediction::new();
        owned.begin_held(7, PredictionPose::xyz(0.0, 0.0, 0.0));
        owned.force_reject();
        owned.set_held(None);
        assert!(owned.has_pending_reject());
    }

    #[test]
    fn grace_keeps_hand_prediction_when_auth_still_far() {
        let mut owned = OwnedPrediction::new();
        owned.reject_grace_frames = 2;
        owned.begin_held(42, PredictionPose::xyz(1.0, 0.0, 0.0));
        let far = PredictionPose::xyz(0.0, 0.0, 0.0);
        let (out, mode) = owned.reconcile_held(far).unwrap();
        assert_eq!(mode, ReconcileMode::Keep);
        assert!((out.x - 1.0).abs() < 1e-5);
        assert!(!owned.take_grab_rejected());
        assert_eq!(owned.held_entity, Some(42));
    }

    #[test]
    fn after_grace_far_authority_is_reject() {
        let mut owned = OwnedPrediction::new();
        // grace=1 → frame 1 optimistic; frame 2 (held_auth_frames=2 > 1) may reject.
        owned.reject_grace_frames = 1;
        owned.begin_held(42, PredictionPose::xyz(1.0, 0.0, 0.0));
        let _ = owned.reconcile_held(PredictionPose::xyz(0.0, 0.0, 0.0));
        owned.predict_held(42, PredictionPose::xyz(1.0, 0.0, 0.0));
        let (_, mode) = owned
            .reconcile_held(PredictionPose::xyz(0.0, 0.0, 0.0))
            .unwrap();
        assert_eq!(mode, ReconcileMode::Snap);
        assert!(owned.take_grab_rejected());
        assert!(owned.held_entity.is_none());
        assert!(owned.display_override(42).is_none());
    }

    #[test]
    fn after_grace_near_authority_keeps() {
        let mut owned = OwnedPrediction::new();
        owned.reject_grace_frames = 1;
        owned.begin_held(42, PredictionPose::xyz(1.0, 0.0, 0.0));
        let _ = owned.reconcile_held(PredictionPose::xyz(1.0, 0.0, 0.0));
        owned.predict_held(42, PredictionPose::xyz(1.0, 0.0, 0.0));
        let (out, mode) = owned
            .reconcile_held(PredictionPose::xyz(1.02, 0.0, 0.0))
            .unwrap();
        assert_eq!(mode, ReconcileMode::Keep);
        assert!((out.x - 1.0).abs() < 1e-5);
        assert!(!owned.take_grab_rejected());
    }

    #[test]
    fn missing_authority_force_rejects() {
        let mut owned = OwnedPrediction::new();
        assert!(owned.begin_held(9, PredictionPose::xyz(1.0, 0.0, 0.0)));
        owned.force_reject();
        assert!(owned.take_grab_rejected());
        assert!(owned.held_entity.is_none());
    }

    #[test]
    fn begin_held_fails_while_rejected() {
        let mut owned = OwnedPrediction::new();
        assert!(owned.begin_held(1, PredictionPose::xyz(0.0, 0.0, 0.0)));
        owned.force_reject();
        assert!(!owned.begin_held(2, PredictionPose::xyz(1.0, 0.0, 0.0)));
        assert!(owned.held_entity.is_none());
        assert!(owned.has_pending_reject());
    }

    #[test]
    fn heuristic_optimistic_during_grace() {
        let pred = PredictionPose::xyz(1.0, 0.0, 0.0);
        let auth = PredictionPose::xyz(0.0, 0.0, 0.0);
        assert!(grab_accepted_heuristic(
            1,
            2,
            pred,
            auth,
            DEFAULT_BLEND_THRESHOLD_M
        ));
        assert!(!grab_accepted_heuristic(
            3,
            2,
            pred,
            auth,
            DEFAULT_BLEND_THRESHOLD_M
        ));
    }
}
