//! SoftLease — short-lived structural edit lock (x00e2).
//!
//! Guarantees: [`SoftLease`] holder + expiry; [`SoftLeaseTable::allows`] conflict
//! check without TTL refresh; [`SoftLeaseTable::try_acquire`] acquires/refreshes
//! only when the caller opts in (host refreshes **after** a successful sparse
//! apply); [`actor_id_from_sub`] is a stable numeric map of Room Token `sub`
//! (Dual-auth still uses the string).
//! Non-guarantees: physical grab (`holder_slot` / `STATE_IS_GRABBED`); JWT /
//! Dual-auth itself; TRASH / 3-layer IDs / `op_chain_hash`; FNV-1a collision
//! resistance (hash is **not** an auth substitute — colliding `sub`s share a
//! SoftLease holder id and weaken mutual exclusion).

use std::collections::HashMap;

use super::EntityId;

/// Default SoftLease lifetime in host ticks (~1s at 60Hz).
pub const SOFT_LEASE_TTL_TICKS: u64 = 60;

/// Structural edit lock (inspector / Reparent). Not a physical grab lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoftLease {
    pub holder_actor_id: u64,
    pub lease_expires_tick: u64,
}

impl SoftLease {
    #[inline]
    pub fn is_active(self, now_tick: u64) -> bool {
        self.lease_expires_tick > now_tick
    }

    /// `true` when an active lease is held by a different actor.
    #[inline]
    pub fn blocks(self, actor_id: u64, now_tick: u64) -> bool {
        self.is_active(now_tick) && self.holder_actor_id != actor_id
    }
}

/// Another actor still holds an unexpired SoftLease on the entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoftLeaseConflict;

/// Per-entity SoftLease table (Phase 1b key = wire [`EntityId`]).
#[derive(Clone, Debug, Default)]
pub struct SoftLeaseTable {
    leases: HashMap<EntityId, SoftLease>,
}

impl SoftLeaseTable {
    #[inline]
    pub fn get(&self, entity_id: EntityId) -> Option<SoftLease> {
        self.leases.get(&entity_id).copied()
    }

    /// Conflict check only — does **not** create or refresh a lease.
    ///
    /// Use before mutating the scene; call [`Self::try_acquire`] only after a
    /// successful apply so failed / invalid ops cannot extend TTL.
    pub fn allows(
        &self,
        entity_id: EntityId,
        actor_id: u64,
        now_tick: u64,
    ) -> Result<(), SoftLeaseConflict> {
        if let Some(lease) = self.leases.get(&entity_id) {
            if lease.blocks(actor_id, now_tick) {
                return Err(SoftLeaseConflict);
            }
        }
        Ok(())
    }

    /// Acquire or refresh SoftLease for `actor_id`.
    ///
    /// Expired leases and same-holder refreshes succeed. A different holder
    /// while still active → [`SoftLeaseConflict`].
    pub fn try_acquire(
        &mut self,
        entity_id: EntityId,
        actor_id: u64,
        now_tick: u64,
        ttl_ticks: u64,
    ) -> Result<(), SoftLeaseConflict> {
        self.allows(entity_id, actor_id, now_tick)?;
        self.leases.insert(
            entity_id,
            SoftLease {
                holder_actor_id: actor_id,
                lease_expires_tick: now_tick.saturating_add(ttl_ticks),
            },
        );
        Ok(())
    }

    #[inline]
    pub fn clear(&mut self, entity_id: EntityId) {
        self.leases.remove(&entity_id);
    }

    /// Drop expired entries (optional GC; acquire already treats expired as free).
    pub fn purge_expired(&mut self, now_tick: u64) {
        self.leases.retain(|_, lease| lease.is_active(now_tick));
    }
}

/// Stable host-local actor id from Room Token `sub` (FNV-1a 64).
///
/// Does **not** replace Dual-auth string `sub` checks — SoftLease holder keys only
/// (CRDT [`super::OpTimestamp::actor_id`] remains host-sequenced until a later
/// slice). **Non-guarantee:** FNV-1a collisions map distinct `sub`s to the same
/// `u64`; those actors will not SoftLease-exclude each other. Not cryptographic.
/// `0` is remapped to `1` so it never collides with a reserved host-sequenced id.
pub fn actor_id_from_sub(sub: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in sub.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    if hash == 0 {
        1
    } else {
        hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_actor_refreshes_other_actor_blocked_until_expiry() {
        let mut table = SoftLeaseTable::default();
        assert!(table.try_acquire(10, 1, 0, SOFT_LEASE_TTL_TICKS).is_ok());
        assert!(table.try_acquire(10, 1, 5, SOFT_LEASE_TTL_TICKS).is_ok());
        assert_eq!(
            table.try_acquire(10, 2, 5, SOFT_LEASE_TTL_TICKS),
            Err(SoftLeaseConflict)
        );
        // Refresh at tick 5 set expires = 5 + TTL; past that, other actor may take over.
        let after_expiry = 5 + SOFT_LEASE_TTL_TICKS;
        assert!(table
            .try_acquire(10, 2, after_expiry, SOFT_LEASE_TTL_TICKS)
            .is_ok());
        assert_eq!(table.get(10).unwrap().holder_actor_id, 2);
    }

    #[test]
    fn actor_id_from_sub_stable_and_nonzero() {
        let a = actor_id_from_sub("alice");
        let b = actor_id_from_sub("alice");
        let c = actor_id_from_sub("bob");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, 0);
        assert_ne!(actor_id_from_sub(""), 0);
    }

    #[test]
    fn allows_does_not_refresh_ttl() {
        let mut table = SoftLeaseTable::default();
        assert!(table.try_acquire(10, 1, 0, SOFT_LEASE_TTL_TICKS).is_ok());
        let expires = table.get(10).unwrap().lease_expires_tick;
        assert!(table.allows(10, 1, 5).is_ok());
        assert_eq!(table.get(10).unwrap().lease_expires_tick, expires);
        assert_eq!(table.allows(10, 2, 5), Err(SoftLeaseConflict));
        assert_eq!(table.get(10).unwrap().lease_expires_tick, expires);
    }
}
