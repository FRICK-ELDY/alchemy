//! SoftLease visibility cache from EditCommit (x00f3 / F1-2).
//!
//! Guarantees: short-lived per-entity holder hints from Commit
//! (`lease_holder_actor_id` + SoftLease-gated ops); TTL ≈ host SoftLease at **60Hz**
//! (`SOFT_LEASE_TTL_TICKS` wall-clock). Despawn accept clears the entity hint.
//! Non-guarantees: full SoftLease table sync; Presence display names; physical Grab;
//! exact sync with host when physics Hz ≠ 60 (chrome may drop early / linger briefly).

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::time::{Duration, Instant};

use alchemy_protocol::edit_op::Op;
use alchemy_protocol::EditCommit;
use alchemy_shared_sim::SOFT_LEASE_TTL_TICKS;

/// Wall-clock TTL matching host SoftLease (~60 ticks @ **assumed 60Hz**).
///
/// Host `lease_tick` advances with physics Hz; if the host runs slower/faster than
/// 60Hz, chrome expiry drifts from the real SoftLease table (documented non-guarantee).
pub const SOFT_LEASE_HINT_TTL: Duration = Duration::from_millis(SOFT_LEASE_TTL_TICKS * 1000 / 60);

/// One SoftLease chrome hint for an entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoftLeaseHint {
    pub holder_actor_id: u64,
    pub expires_at: Instant,
}

impl SoftLeaseHint {
    #[inline]
    pub fn is_active(self, now: Instant) -> bool {
        self.expires_at > now && self.holder_actor_id != 0
    }
}

/// Local SoftLease visibility table (Published / Epoch 非汚染).
#[derive(Clone, Debug, Default)]
pub struct SoftLeaseHintTable {
    by_entity: HashMap<u32, SoftLeaseHint>,
}

impl SoftLeaseHintTable {
    /// Drop all expired entries (call on Commit fold / chrome refresh).
    pub fn purge_expired(&mut self, now: Instant) {
        self.by_entity.retain(|_, h| h.is_active(now));
    }

    /// Active holder for `entity_id`, if any (purges expired globally first).
    pub fn holder_of(&mut self, entity_id: u32, now: Instant) -> Option<u64> {
        self.purge_expired(now);
        self.by_entity
            .get(&entity_id)
            .map(|h| h.holder_actor_id)
            .filter(|&id| id != 0)
    }

    /// Fold an EditCommit into the hint table.
    pub fn note_commit(&mut self, commit: &EditCommit, now: Instant) {
        self.purge_expired(now);

        // Accepted Despawn: entity is gone — drop stale chrome before id reuse.
        if commit.accepted {
            if let Some(Op::Despawn(d)) = commit.ops.first().and_then(|o| o.op.as_ref()) {
                self.by_entity.remove(&d.entity_id);
                return;
            }
        }

        let holder = commit.lease_holder_actor_id;
        if holder == 0 {
            return;
        }
        let Some(entity_id) = soft_lease_entity_from_commit(commit) else {
            return;
        };
        // Accept of SoftLease-gated ops, or SOFT_LEASE reject — both name a holder.
        let soft_lease_reject =
            !commit.accepted && commit.reject_reason == alchemy_protocol::edit_reject::SOFT_LEASE;
        if !(commit.accepted || soft_lease_reject) {
            return;
        }
        self.by_entity.insert(
            entity_id,
            SoftLeaseHint {
                holder_actor_id: holder,
                expires_at: now + SOFT_LEASE_HINT_TTL,
            },
        );
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.by_entity.len()
    }
}

/// Entity SoftLease applies to (Reparent child / SetField target).
pub fn soft_lease_entity_from_commit(commit: &EditCommit) -> Option<u32> {
    let op = commit.ops.first()?.op.as_ref()?;
    match op {
        Op::Reparent(r) => Some(r.child_entity_id),
        Op::SetField(f) => Some(f.entity_id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alchemy_protocol::{EditDespawn, EditOp, EditSetField};

    fn set_field_commit(accepted: bool, reason: u32, holder: u64) -> EditCommit {
        EditCommit {
            scene_rev: 1,
            op_id: 1,
            author_session_pub: vec![],
            accepted,
            reject_reason: reason,
            ops: vec![EditOp {
                op: Some(Op::SetField(EditSetField {
                    entity_id: 42,
                    field_id: 1,
                    value: vec![],
                    expected_value_hash: vec![],
                })),
            }],
            host_sig: vec![],
            lease_holder_actor_id: holder,
        }
    }

    fn despawn_accept(entity_id: u32) -> EditCommit {
        EditCommit {
            scene_rev: 2,
            op_id: 2,
            author_session_pub: vec![],
            accepted: true,
            reject_reason: 0,
            ops: vec![EditOp {
                op: Some(Op::Despawn(EditDespawn { entity_id })),
            }],
            host_sig: vec![],
            lease_holder_actor_id: 0,
        }
    }

    #[test]
    fn soft_lease_reject_stores_holder() {
        let mut table = SoftLeaseHintTable::default();
        let now = Instant::now();
        table.note_commit(
            &set_field_commit(false, alchemy_protocol::edit_reject::SOFT_LEASE, 99),
            now,
        );
        assert_eq!(table.holder_of(42, now), Some(99));
        assert_eq!(table.holder_of(7, now), None);
    }

    #[test]
    fn accept_with_holder_stores_hint() {
        let mut table = SoftLeaseHintTable::default();
        let now = Instant::now();
        table.note_commit(&set_field_commit(true, 0, 7), now);
        assert_eq!(table.holder_of(42, now), Some(7));
    }

    #[test]
    fn zero_holder_ignored() {
        let mut table = SoftLeaseHintTable::default();
        let now = Instant::now();
        table.note_commit(
            &set_field_commit(false, alchemy_protocol::edit_reject::SOFT_LEASE, 0),
            now,
        );
        assert_eq!(table.len(), 0);
    }

    #[test]
    fn expired_hint_cleared() {
        let mut table = SoftLeaseHintTable::default();
        let now = Instant::now();
        table.note_commit(&set_field_commit(true, 0, 3), now);
        let later = now + SOFT_LEASE_HINT_TTL + Duration::from_millis(1);
        assert_eq!(table.holder_of(42, later), None);
        assert_eq!(
            table.len(),
            0,
            "purge drops expired without selecting that entity"
        );
    }

    #[test]
    fn despawn_accept_clears_hint() {
        let mut table = SoftLeaseHintTable::default();
        let now = Instant::now();
        table.note_commit(&set_field_commit(true, 0, 7), now);
        assert_eq!(table.holder_of(42, now), Some(7));
        table.note_commit(&despawn_accept(42), now);
        assert_eq!(table.holder_of(42, now), None);
    }

    #[test]
    fn purge_expired_on_unrelated_read() {
        let mut table = SoftLeaseHintTable::default();
        let now = Instant::now();
        table.note_commit(&set_field_commit(true, 0, 3), now);
        let later = now + SOFT_LEASE_HINT_TTL + Duration::from_millis(1);
        // Reading a different entity still purges the expired one.
        assert_eq!(table.holder_of(99, later), None);
        assert_eq!(table.len(), 0);
    }
}
