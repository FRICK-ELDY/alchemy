//! Presence slots + Intent overlays (x0015 / x0022 / x004d).
//!
//! Guarantees: Manifest-derived `slot_count`; Leave / liveliness grace / Intent TTL reclaim;
//! `by_slot` stays `Copy` for physics tick snapshots.
//! Non-guarantees: WAN peer ACL; authenticated remote reclaim.

use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use alchemy_protocol::{PresenceAssignment, PresenceState, PROTOCOL_PIN, USER_ROOT_SLOTS_MAX};

use crate::sim::PhysicsWorld;

/// Intent silence before reclaiming a Presence slot (client INPUT_HZ=30).
pub(crate) const PRESENCE_SLOT_TTL: Duration = Duration::from_secs(3);

/// Grace before confirming liveliness Delete reclaim (x0022).
///
/// Covers Intent↔Delete reorder after Room Token refresh (distinct Zenoh keys).
/// Cancels when: `session_jti` rotates, same-jti liveliness Put, or Intent stays fresh
/// (`last_input` within [`PRESENCE_SLOT_TTL`]) — spurious WAN Delete must not kick a live client.
/// Silent crash (no Intent) still confirms after grace.
pub(crate) const LIVELINESS_RECLAIM_GRACE: Duration = Duration::from_millis(1_500);

/// Mutex poison must not degrade into empty Presence / silent skip (x002e / P3-8).
pub(crate) fn lock_or_fatal<'a, T>(mutex: &'a Mutex<T>, label: &str) -> MutexGuard<'a, T> {
    mutex.lock().unwrap_or_else(|poisoned| {
        log::error!("{label} mutex poisoned: {poisoned}; aborting host-server");
        std::process::exit(78);
    })
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PoseOverride {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub active: bool,
    pub grab: bool,
    pub grab_entity_id: u32,
    pub aim_x: f32,
    pub aim_y: f32,
    pub aim_z: f32,
}

/// Per-sub input slots (x0015) + Leave / jti-bound liveliness / Intent TTL reclaim (x0022).
///
/// Storage is `[T; USER_ROOT_SLOTS_MAX]` + [`slot_count`] (Manifest-derived `n`, x004d / x009b).
/// `by_slot` stays `Copy` so the physics tick can snapshot without a heap alloc.
#[derive(Debug)]
pub(crate) struct PresenceIntents {
    pub by_slot: [PoseOverride; USER_ROOT_SLOTS_MAX],
    /// Occupied slots: Some(sub) when assigned. Only indices `0..slot_count` are live.
    pub occupied_sub: [Option<String>; USER_ROOT_SLOTS_MAX],
    /// Last verified Room Token `jti` per occupied slot (x0022). None until first verify.
    pub session_jti: [Option<String>; USER_ROOT_SLOTS_MAX],
    /// Scheduled liveliness reclaim: `(deleted_jti, confirm_after)`. Cancelled if `session_jti` rotates.
    pub pending_liveliness: [Option<(String, Instant)>; USER_ROOT_SLOTS_MAX],
    /// Last accepted Intent time per slot (None when free).
    pub last_input: [Option<Instant>; USER_ROOT_SLOTS_MAX],
    /// Manifest-derived live capacity (`1..=USER_ROOT_SLOTS_MAX`).
    slot_count: usize,
    pub dropped_full: u64,
    /// Bumped on assignment or reclaim (PresenceState.revision).
    pub revision: u64,
    /// `(entity_id, sub)` freed off the physics thread (Leave / liveliness); drained each tick.
    pub pending_removes: Vec<(u32, String)>,
    /// Set on new slot assign or session `jti` rotate; [`crate::host_loop::HostLoop::tick`]
    /// takes for Manifest join assist (x003c).
    /// Not set on reclaim/leave (those bump `revision` but are not joiners).
    manifest_join_assist: bool,
}

impl PresenceIntents {
    pub(crate) fn with_slots(n: usize) -> Self {
        assert!(
            (1..=USER_ROOT_SLOTS_MAX).contains(&n),
            "Presence slots must be 1..={USER_ROOT_SLOTS_MAX}, got {n}"
        );
        Self {
            by_slot: [PoseOverride::default(); USER_ROOT_SLOTS_MAX],
            occupied_sub: std::array::from_fn(|_| None),
            session_jti: std::array::from_fn(|_| None),
            pending_liveliness: std::array::from_fn(|_| None),
            last_input: [None; USER_ROOT_SLOTS_MAX],
            slot_count: n,
            dropped_full: 0,
            revision: 0,
            pending_removes: Vec::new(),
            manifest_join_assist: false,
        }
    }

    pub(crate) fn slot_count(&self) -> usize {
        self.slot_count
    }

    pub(crate) fn take_manifest_join_assist(&mut self) -> bool {
        let v = self.manifest_join_assist;
        self.manifest_join_assist = false;
        v
    }

    pub(crate) fn assign_or_lookup(&mut self, sub: &str, jti: &str, now: Instant) -> Option<usize> {
        let n = self.slot_count;
        for i in 0..n {
            if self.occupied_sub[i].as_deref() == Some(sub) {
                self.last_input[i] = Some(now);
                // Cancel pending Delete only on jti rotate (refresh). Same-jti Intent leaves
                // the schedule so crash/undeclare still confirms after grace.
                let jti_rotated = self.session_jti[i].as_deref() != Some(jti);
                if jti_rotated {
                    self.pending_liveliness[i] = None;
                    // Reconnect / Token refresh: same sub, new jti — late subscribe needs Manifest.
                    self.manifest_join_assist = true;
                }
                self.session_jti[i] = Some(jti.to_string());
                return Some(i);
            }
        }
        for i in 0..n {
            if self.occupied_sub[i].is_none() {
                self.occupied_sub[i] = Some(sub.to_string());
                self.session_jti[i] = Some(jti.to_string());
                self.pending_liveliness[i] = None;
                self.last_input[i] = Some(now);
                self.revision = self.revision.saturating_add(1);
                self.manifest_join_assist = true;
                log::info!(
                    "presence: assigned sub={sub} → user_root entity {} (revision={})",
                    PhysicsWorld::user_root_entity_id(i),
                    self.revision
                );
                return Some(i);
            }
        }
        None
    }

    /// Free one occupied slot. Returns `(entity_id, sub)`.
    fn free_slot_at(&mut self, i: usize, reason: &str) -> (u32, String) {
        let sub = self.occupied_sub[i].take().unwrap_or_default();
        self.session_jti[i] = None;
        self.pending_liveliness[i] = None;
        self.last_input[i] = None;
        self.by_slot[i] = PoseOverride::default();
        self.revision = self.revision.saturating_add(1);
        let entity_id = PhysicsWorld::user_root_entity_id(i);
        log::info!(
            "presence: reclaimed slot={i} entity={entity_id} sub={sub} reason={reason} (revision={})",
            self.revision
        );
        (entity_id, sub)
    }

    /// Immediate reclaim for verified Leave / session (queues `pending_removes`).
    pub(crate) fn reclaim_sub(&mut self, sub: &str, reason: &str) -> Option<(u32, String)> {
        for i in 0..self.slot_count() {
            if self.occupied_sub[i].as_deref() == Some(sub) {
                let freed = self.free_slot_at(i, reason);
                self.pending_removes.push(freed.clone());
                return Some(freed);
            }
        }
        None
    }

    /// True when this slot saw a verified Intent within [`PRESENCE_SLOT_TTL`].
    /// Spurious liveliness Delete while the client is still putting Intent must not reclaim
    /// (Intent path is the liveness proof; liveliness is for crash when Intent also stops).
    fn intent_fresh_at(&self, slot: usize, now: Instant) -> bool {
        self.last_input[slot].is_some_and(|t| now.duration_since(t) < PRESENCE_SLOT_TTL)
    }

    /// Schedule reclaim when Delete `jti` matches the bound session (x0022).
    ///
    /// Does **not** free immediately: grace lets a refresh Intent with a new `jti` cancel the
    /// pending Delete when liveliness arrives before the Intent (no cross-key order).
    /// Same-jti Put during grace also cancels (spurious Delete while token still held).
    /// Fresh Intent at schedule time ignores Delete entirely (WAN / router glitch).
    pub(crate) fn schedule_liveliness_reclaim(
        &mut self,
        sub: &str,
        jti: &str,
        now: Instant,
    ) -> bool {
        for i in 0..self.slot_count() {
            if self.occupied_sub[i].as_deref() != Some(sub) {
                continue;
            }
            match self.session_jti[i].as_deref() {
                Some(bound) if bound == jti => {
                    if self.intent_fresh_at(i, now) {
                        log::warn!(
                            "liveliness Delete ignored: sub={sub} jti={jti} (Intent still fresh)"
                        );
                        return false;
                    }
                    let confirm_at = now + LIVELINESS_RECLAIM_GRACE;
                    self.pending_liveliness[i] = Some((jti.to_string(), confirm_at));
                    log::info!(
                        "liveliness Delete scheduled: sub={sub} jti={jti} confirm_in_ms={}",
                        LIVELINESS_RECLAIM_GRACE.as_millis()
                    );
                    return true;
                }
                Some(bound) => {
                    log::warn!(
                        "liveliness Delete ignored: sub={sub} jti={jti} bound_jti={bound} (mismatch)"
                    );
                    return false;
                }
                None => {
                    log::warn!(
                        "liveliness Delete ignored: sub={sub} jti={jti} (no verified session yet)"
                    );
                    return false;
                }
            }
        }
        false
    }

    /// Cancel a pending same-jti Delete when liveliness Put reappears during grace.
    pub(crate) fn cancel_liveliness_reclaim_on_put(&mut self, sub: &str, jti: &str) -> bool {
        for i in 0..self.slot_count() {
            if self.occupied_sub[i].as_deref() != Some(sub) {
                continue;
            }
            match &self.pending_liveliness[i] {
                Some((pending_jti, _)) if pending_jti == jti => {
                    self.pending_liveliness[i] = None;
                    log::info!(
                        "liveliness Delete cancelled: sub={sub} jti={jti} (Put while grace)"
                    );
                    return true;
                }
                _ => return false,
            }
        }
        false
    }

    /// Confirm scheduled liveliness reclaim if grace elapsed and `jti` still bound.
    /// Intent that arrived during grace also cancels (same proof as schedule-time ignore).
    pub(crate) fn confirm_liveliness_reclaims(&mut self, now: Instant) -> Vec<(u32, String)> {
        let mut freed = Vec::new();
        for i in 0..self.slot_count() {
            let Some((jti, confirm_at)) = self.pending_liveliness[i].clone() else {
                continue;
            };
            if now < confirm_at {
                continue;
            }
            self.pending_liveliness[i] = None;
            match self.session_jti[i].as_deref() {
                Some(bound) if bound == jti && self.occupied_sub[i].is_some() => {
                    if self.intent_fresh_at(i, now) {
                        log::info!(
                            "liveliness Delete cancelled after grace: deleted_jti={jti} (Intent still fresh)"
                        );
                        continue;
                    }
                    freed.push(self.free_slot_at(i, "liveliness"));
                }
                Some(bound) => {
                    log::info!(
                        "liveliness Delete cancelled after grace: deleted_jti={jti} bound_jti={bound}"
                    );
                }
                None => {
                    log::info!(
                        "liveliness Delete cancelled after grace: deleted_jti={jti} (slot free)"
                    );
                }
            }
        }
        freed
    }

    /// Drop slots with no Intent for `ttl`. Returns freed `(entity_id, sub)`.
    pub(crate) fn reclaim_stale(&mut self, now: Instant, ttl: Duration) -> Vec<(u32, String)> {
        let mut freed = Vec::new();
        for i in 0..self.slot_count() {
            if self.occupied_sub[i].is_none() {
                continue;
            }
            let stale = match self.last_input[i] {
                Some(t) => now.saturating_duration_since(t) >= ttl,
                None => true,
            };
            if !stale {
                continue;
            }
            freed.push(self.free_slot_at(i, "ttl"));
        }
        freed
    }

    pub(crate) fn drain_reclaims(&mut self, now: Instant, ttl: Duration) -> Vec<(u32, String)> {
        let mut freed = self.reclaim_stale(now, ttl);
        freed.extend(self.confirm_liveliness_reclaims(now));
        freed.append(&mut self.pending_removes);
        freed
    }

    pub(crate) fn to_presence_state(&self, host_generation: u64) -> PresenceState {
        let mut assignments = Vec::with_capacity(self.slot_count);
        for i in 0..self.slot_count {
            if let Some(sub) = &self.occupied_sub[i] {
                assignments.push(PresenceAssignment {
                    sub: sub.clone(),
                    entity_id: PhysicsWorld::user_root_entity_id(i),
                });
            }
        }
        PresenceState {
            revision: self.revision,
            assignments,
            protocol_pin: PROTOCOL_PIN.to_string(),
            host_sig: vec![],
            host_generation,
        }
    }
}

#[cfg(test)]
mod presence_reclaim_tests {
    use super::*;
    use alchemy_protocol::USER_ROOT_SLOTS_MAX;

    #[test]
    fn reclaim_stale_frees_slot_for_reuse() {
        let mut p = PresenceIntents::with_slots(USER_ROOT_SLOTS_MAX);
        let t0 = Instant::now();
        let slot = p.assign_or_lookup("alice", "j1", t0).expect("assign");
        assert_eq!(slot, 0);
        assert_eq!(p.to_presence_state(1).assignments.len(), 1);
        assert_eq!(p.to_presence_state(1).host_generation, 1);

        let freed = p.reclaim_stale(t0 + Duration::from_secs(2), PRESENCE_SLOT_TTL);
        assert!(freed.is_empty());
        assert!(p.occupied_sub[0].is_some());

        let freed = p.reclaim_stale(t0 + PRESENCE_SLOT_TTL, PRESENCE_SLOT_TTL);
        assert_eq!(
            freed,
            vec![(PhysicsWorld::user_root_entity_id(0), "alice".into())]
        );
        assert!(p.occupied_sub[0].is_none());
        assert_eq!(p.to_presence_state(1).assignments.len(), 0);
        assert!(p.revision >= 2);

        let slot2 = p
            .assign_or_lookup(
                "bob",
                "j2",
                t0 + PRESENCE_SLOT_TTL + Duration::from_millis(1),
            )
            .expect("reuse");
        assert_eq!(slot2, 0);
        assert_eq!(p.to_presence_state(1).assignments[0].sub, "bob".to_string());
    }

    #[test]
    fn same_sub_before_ttl_keeps_slot() {
        let mut p = PresenceIntents::with_slots(USER_ROOT_SLOTS_MAX);
        let t0 = Instant::now();
        assert_eq!(p.assign_or_lookup("alice", "j1", t0), Some(0));
        assert_eq!(
            p.assign_or_lookup("alice", "j1", t0 + Duration::from_secs(2)),
            Some(0)
        );
        let freed = p.reclaim_stale(
            t0 + Duration::from_secs(2) + Duration::from_secs(2),
            PRESENCE_SLOT_TTL,
        );
        assert!(freed.is_empty());
    }

    #[test]
    fn manifest_join_assist_on_assign_and_jti_rotate_not_same_jti_or_reclaim() {
        let mut p = PresenceIntents::with_slots(USER_ROOT_SLOTS_MAX);
        let t0 = Instant::now();
        assert!(!p.take_manifest_join_assist());
        assert_eq!(p.assign_or_lookup("alice", "j1", t0), Some(0));
        assert!(p.take_manifest_join_assist());
        assert!(!p.take_manifest_join_assist());
        assert_eq!(p.assign_or_lookup("alice", "j1", t0), Some(0));
        assert!(!p.take_manifest_join_assist());
        assert_eq!(p.assign_or_lookup("alice", "j2", t0), Some(0));
        assert!(p.take_manifest_join_assist());
        let _ = p.reclaim_sub("alice", "leave").expect("leave");
        assert!(!p.take_manifest_join_assist());
        assert!(p.revision >= 2);
    }

    #[test]
    fn reclaim_sub_frees_immediately() {
        let mut p = PresenceIntents::with_slots(USER_ROOT_SLOTS_MAX);
        let t0 = Instant::now();
        assert_eq!(p.assign_or_lookup("alice", "j1", t0), Some(0));
        let entity = p.reclaim_sub("alice", "leave").expect("reclaim");
        assert_eq!(
            entity,
            (PhysicsWorld::user_root_entity_id(0), "alice".into())
        );
        assert!(p.occupied_sub[0].is_none());
        assert_eq!(p.pending_removes, vec![entity.clone()]);
        let drained = p.drain_reclaims(t0, PRESENCE_SLOT_TTL);
        assert_eq!(drained, vec![entity]);
        assert!(p.pending_removes.is_empty());
        assert_eq!(p.assign_or_lookup("bob", "j2", t0), Some(0));
    }

    #[test]
    fn schedule_liveliness_requires_matching_jti() {
        let mut p = PresenceIntents::with_slots(USER_ROOT_SLOTS_MAX);
        let t0 = Instant::now();
        assert_eq!(p.assign_or_lookup("alice", "jti-real", t0), Some(0));
        p.last_input[0] = Some(t0 - PRESENCE_SLOT_TTL - Duration::from_millis(1));

        assert!(!p.schedule_liveliness_reclaim("alice", "jti-fake", t0));
        assert!(p.occupied_sub[0].is_some());
        assert!(p.pending_liveliness[0].is_none());

        assert!(p.schedule_liveliness_reclaim("alice", "jti-real", t0));
        assert!(p.occupied_sub[0].is_some());
        assert!(p.pending_liveliness[0].is_some());

        let early = p.confirm_liveliness_reclaims(t0 + Duration::from_millis(100));
        assert!(early.is_empty());
        assert!(p.occupied_sub[0].is_some());

        let freed = p.confirm_liveliness_reclaims(t0 + LIVELINESS_RECLAIM_GRACE);
        assert_eq!(
            freed,
            vec![(PhysicsWorld::user_root_entity_id(0), "alice".into())]
        );
        assert!(p.occupied_sub[0].is_none());
    }

    #[test]
    fn schedule_liveliness_ignores_unknown_sub() {
        let mut p = PresenceIntents::with_slots(USER_ROOT_SLOTS_MAX);
        assert!(!p.schedule_liveliness_reclaim("nobody", "jti", Instant::now()));
    }

    #[test]
    fn narrowed_slot_count_rejects_overflow() {
        let mut p = PresenceIntents::with_slots(2);
        let t0 = Instant::now();
        assert_eq!(p.assign_or_lookup("alice", "j1", t0), Some(0));
        assert_eq!(p.assign_or_lookup("bob", "j2", t0), Some(1));
        assert_eq!(p.assign_or_lookup("carol", "j3", t0), None);
        assert_eq!(p.slot_count(), 2);
        assert!(p.occupied_sub[2].is_none());
        assert!(p.occupied_sub[3].is_none());
    }

    #[test]
    fn refresh_intent_cancels_pending_liveliness_delete() {
        let mut p = PresenceIntents::with_slots(USER_ROOT_SLOTS_MAX);
        let t0 = Instant::now();
        assert_eq!(p.assign_or_lookup("alice", "jti-old", t0), Some(0));
        p.last_input[0] = Some(t0 - PRESENCE_SLOT_TTL - Duration::from_millis(1));
        assert!(p.schedule_liveliness_reclaim("alice", "jti-old", t0));

        assert_eq!(
            p.assign_or_lookup("alice", "jti-new", t0 + Duration::from_millis(50)),
            Some(0)
        );
        assert!(
            p.pending_liveliness[0].is_none(),
            "pending Delete must clear on jti rotate"
        );

        let freed = p.confirm_liveliness_reclaims(t0 + LIVELINESS_RECLAIM_GRACE);
        assert!(freed.is_empty());
        assert_eq!(p.occupied_sub[0].as_deref(), Some("alice"));
        assert_eq!(p.session_jti[0].as_deref(), Some("jti-new"));
    }

    #[test]
    fn liveliness_put_cancels_same_jti_pending_delete() {
        let mut p = PresenceIntents::with_slots(USER_ROOT_SLOTS_MAX);
        let t0 = Instant::now();
        assert_eq!(p.assign_or_lookup("alice", "jti-real", t0), Some(0));
        p.last_input[0] = Some(t0 - PRESENCE_SLOT_TTL - Duration::from_millis(1));
        assert!(p.schedule_liveliness_reclaim("alice", "jti-real", t0));
        assert!(p.cancel_liveliness_reclaim_on_put("alice", "jti-real"));
        assert!(p.pending_liveliness[0].is_none());
        let freed = p.confirm_liveliness_reclaims(t0 + LIVELINESS_RECLAIM_GRACE);
        assert!(freed.is_empty());
        assert_eq!(p.occupied_sub[0].as_deref(), Some("alice"));
    }

    #[test]
    fn liveliness_delete_ignored_while_intent_fresh() {
        let mut p = PresenceIntents::with_slots(USER_ROOT_SLOTS_MAX);
        let t0 = Instant::now();
        assert_eq!(p.assign_or_lookup("alice", "jti-real", t0), Some(0));
        assert!(!p.schedule_liveliness_reclaim("alice", "jti-real", t0));
        assert!(p.pending_liveliness[0].is_none());
        assert!(p.occupied_sub[0].is_some());
    }

    #[test]
    fn liveliness_delete_cancelled_if_intent_arrives_during_grace() {
        let mut p = PresenceIntents::with_slots(USER_ROOT_SLOTS_MAX);
        let t0 = Instant::now();
        assert_eq!(p.assign_or_lookup("alice", "jti-real", t0), Some(0));
        p.last_input[0] = Some(t0 - PRESENCE_SLOT_TTL - Duration::from_millis(1));
        assert!(p.schedule_liveliness_reclaim("alice", "jti-real", t0));
        assert_eq!(
            p.assign_or_lookup("alice", "jti-real", t0 + Duration::from_millis(100)),
            Some(0)
        );
        let freed = p.confirm_liveliness_reclaims(t0 + LIVELINESS_RECLAIM_GRACE);
        assert!(freed.is_empty());
        assert_eq!(p.occupied_sub[0].as_deref(), Some("alice"));
    }
}
