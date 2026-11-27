//! Host live edit facade over `shared-sim::scene_store` (x00e0).
//!
//! Contract (x00b3 / x00b9 / x00e0):
//! - Sparse SSoT is [`SceneStore`] (parents / fields / entities) — **no** host
//!   HashMap second copy.
//! - Host keeps quarantine, `tip` (`scene_rev`) sequencing, pending spawn/despawn
//!   / freelist / reserved, edit + Save I/O.
//! - Optimistic store mutates on live accept; **`tip` is NOT bumped** until
//!   FrameBridge queue **and** barrier succeed (Reparent/SetField).
//!   Spawn/Despawn tip bumps after inbox send / physics-ok.
//! - Accept `EditCommit` only after that success (never rewind tip).
//! - Queue / barrier / inbox failure → `rollback_mirrors` + reject `EditCommit`
//!   (`APPLY`); tip untouched.
//!
//! Facade structural ops use [`SceneStore::apply_sparse`] (no `SimPatch` enqueue):
//! FrameBridge already owns dense Reparent/SetField until Arena drain is wired
//! from this ledger. Barrier / queue failure rolls back **LIFO** (reverse apply order).
//!
//! Non-guarantees: crash durability of unsaved pending; Spatial VCS flush;
//! TRASH_UUID / SoftLease / SlotUndoStack (→ x00e1); Arena SimPatch drain wiring.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use alchemy_protocol::wire_aead::AeadKind;
use alchemy_protocol::wire_crypto::{sign_edit_commit, SigningKey};
use alchemy_protocol::{
    edit_field_id, scene_entry_is_user_root, user_root_entity_last, EditCommit, EditProposal,
    EditReparent, EditSetField, Message, SceneEntry,
};
use alchemy_shared_sim::{ApplyOutcome, CrdtOp, OpTimestamp, PropValue, SceneStore};
use zenoh::Wait;

use crate::room_aead::{self, SharedRoomAead};

/// Host-sequenced local actor id for facade-applied ops (JWT `sub` → actor is host gate).
const HOST_ACTOR_ID: u64 = 0;

/// Session-local spawn awaiting explicit Save (x00b9); holds SpawnTicket for world verify.
#[derive(Clone, Debug)]
pub struct PendingSpawn {
    pub entry: SceneEntry,
    pub spawn_ticket: String,
}

/// Shared live working-tree tip + scene_store facade (edit lane + physics).
#[derive(Debug, Default)]
pub struct LiveEditLedger {
    /// Monotonic EditCommit tip (advanced only after barrier success).
    pub tip: u64,
    /// Session sparse SSoT (parents / fields / entities).
    store: SceneStore,
    /// Unsaved session spawns (Save body `spawns`); ticket kept until Save or local despawn.
    pub pending_spawns: HashMap<u32, PendingSpawn>,
    /// Canonical entities hidden live; Save body `despawns` deletes from positive.
    pub pending_despawns: HashSet<u32>,
    /// Recycled ids from cancelled session spawns only (not canonical pending_despawns).
    pub freelist: Vec<u32>,
    /// Allocated ids not yet in the store / scene (concurrent spawn race).
    pub reserved: HashSet<u32>,
}

pub type SharedLiveEdit = Arc<Mutex<LiveEditLedger>>;

pub fn new_shared_live_edit() -> SharedLiveEdit {
    Arc::new(Mutex::new(LiveEditLedger::default()))
}

impl LiveEditLedger {
    #[cfg(test)]
    fn store(&self) -> &SceneStore {
        &self.store
    }

    #[cfg(test)]
    fn store_mut(&mut self) -> &mut SceneStore {
        &mut self.store
    }

    pub fn bump_tip(&mut self) -> u64 {
        self.tip = self.tip.saturating_add(1);
        self.tip
    }

    #[inline]
    fn next_ts(&self) -> OpTimestamp {
        OpTimestamp::new(self.store.max_lamport().saturating_add(1), HOST_ACTOR_ID)
    }

    /// World-root parent is `0` on the wire / Save body.
    pub fn parent_of(&self, child: u32) -> u32 {
        self.store.parent(child).unwrap_or(0)
    }

    #[inline]
    pub fn contains_entity(&self, entity_id: u32) -> bool {
        self.store.contains(entity_id)
    }

    /// Dense live entity ids (save diff walk).
    #[inline]
    pub fn entity_ids(&self) -> &[u32] {
        self.store.entity_ids()
    }

    pub fn flags_of(&self, entity_id: u32) -> Option<u32> {
        match self.store.property(entity_id, edit_field_id::FLAGS)? {
            PropValue::U32(v) => Some(*v),
            _ => None,
        }
    }

    pub fn color_of(&self, entity_id: u32) -> Option<[f32; 4]> {
        match self.store.property(entity_id, edit_field_id::COLOR_RGBA)? {
            PropValue::F32x4(v) => Some(*v),
            _ => None,
        }
    }

    /// UTF-8 blake3 hex bytes in the store (no alloc).
    pub fn blake3_bytes(&self, entity_id: u32) -> Option<&[u8]> {
        match self.store.property(entity_id, edit_field_id::BLAKE3)? {
            PropValue::Bytes(b) => Some(b.as_slice()),
            _ => None,
        }
    }

    /// Apply quarantined Reparent into scene_store (optimistic; tip unchanged).
    ///
    /// Uses [`SceneStore::apply_sparse`] so Move does not enqueue `SimPatch`
    /// (FrameBridge dense path). Cycle / depth / stale / missing come from the store.
    pub fn apply_reparent(&mut self, child: u32, new_parent: u32) -> ApplyOutcome {
        self.store.apply_sparse(CrdtOp::MoveNode {
            ts: self.next_ts(),
            node: child,
            new_parent: wire_parent(new_parent),
        })
    }

    /// Apply quarantined SetField into scene_store (optimistic; tip unchanged).
    pub fn apply_set_field(
        &mut self,
        entity_id: u32,
        field_id: u32,
        value: PropValue,
    ) -> ApplyOutcome {
        self.store.apply_sparse(CrdtOp::SetProperty {
            ts: self.next_ts(),
            node: entity_id,
            prop_key: field_id,
            value,
        })
    }

    /// Record a newly spawned entity. Preserves an existing live parent if already set
    /// (`seed` with root only when absent — does not clobber a prior Reparent on re-note).
    /// Returns `false` if seed or props apply fails.
    ///
    /// Rollback on props failure: **fresh** insert → `remove_node` (no half-write).
    /// **Re-note** (`!fresh`): any props already applied in this call are **left
    /// as-is** (no snapshot undo); caller sees `false` and must not treat the note
    /// as fully successful. Same contract as [`Self::note_spawn_with_parent`].
    #[must_use = "seed/props failure; do not ignore"]
    pub fn note_spawn(
        &mut self,
        entity_id: u32,
        flags: u32,
        color: [f32; 4],
        blake3: String,
    ) -> bool {
        let fresh = !self.store.contains(entity_id);
        if fresh && !self.store.seed_node(entity_id, None) {
            return false;
        }
        if !self.write_spawn_props(entity_id, flags, color, blake3) {
            if fresh {
                let _ = self.store.remove_node(entity_id);
            }
            return false;
        }
        true
    }

    /// Seed or overwrite including creative parent (x00b8 boot / scene install).
    /// Returns `false` if seed or props apply fails.
    ///
    /// Rollback on props failure: **fresh** insert → `remove_node` (no half-write).
    /// **Re-seed** (`was_live`): parent link from `seed_node` and any props already
    /// applied in this call are **left as-is** (no snapshot undo); caller sees
    /// `false` and must not treat the note as fully successful.
    #[must_use = "seed/props failure; do not ignore"]
    pub fn note_spawn_with_parent(
        &mut self,
        entity_id: u32,
        parent_entity_id: u32,
        flags: u32,
        color: [f32; 4],
        blake3: String,
    ) -> bool {
        let was_live = self.store.contains(entity_id);
        if !self
            .store
            .seed_node(entity_id, wire_parent(parent_entity_id))
        {
            return false;
        }
        if !self.write_spawn_props(entity_id, flags, color, blake3) {
            if !was_live {
                let _ = self.store.remove_node(entity_id);
            }
            return false;
        }
        true
    }

    /// Write FLAGS / COLOR / BLAKE3. Returns `false` on the first SetProperty failure
    /// (does not continue applying later fields — limits lamport advance / partial write).
    fn write_spawn_props(
        &mut self,
        entity_id: u32,
        flags: u32,
        color: [f32; 4],
        blake3: String,
    ) -> bool {
        if !self
            .apply_set_field(entity_id, edit_field_id::FLAGS, PropValue::U32(flags))
            .is_success()
        {
            return false;
        }
        if !self
            .apply_set_field(
                entity_id,
                edit_field_id::COLOR_RGBA,
                PropValue::F32x4(color),
            )
            .is_success()
        {
            return false;
        }
        if !blake3.is_empty()
            && !self
                .apply_set_field(
                    entity_id,
                    edit_field_id::BLAKE3,
                    PropValue::Bytes(blake3.into_bytes()),
                )
                .is_success()
        {
            return false;
        }
        true
    }

    pub fn note_despawn(&mut self, entity_id: u32) {
        let _ = self.store.remove_node(entity_id);
    }

    /// Record a physics-ok session spawn for later Save (x00b9).
    pub fn note_pending_spawn(&mut self, pending: PendingSpawn) {
        let eid = pending.entry.entity_id;
        self.reserved.remove(&eid);
        self.pending_despawns.remove(&eid);
        self.pending_spawns.insert(eid, pending);
    }

    /// Drop a reserved id when spawn never reaches physics-ok (x00b9); return to freelist.
    pub fn release_reserved(&mut self, entity_id: u32) {
        if self.reserved.remove(&entity_id) {
            self.freelist.push(entity_id);
        }
    }

    /// Live despawn: drop unsaved spawn, or mark canonical id for Save `despawns`.
    pub fn note_live_despawn(&mut self, entity_id: u32) {
        self.reserved.remove(&entity_id);
        if self.pending_spawns.remove(&entity_id).is_some() {
            self.freelist.push(entity_id);
        } else {
            self.pending_despawns.insert(entity_id);
        }
        self.note_despawn(entity_id);
    }

    /// Clear Save-folded pending after successful world Save.
    pub fn clear_save_pending(&mut self) {
        self.pending_spawns.clear();
        self.pending_despawns.clear();
    }

    /// Host-local entity id (scene max+1 / freelist; skip user_root + pending_despawns).
    /// Never returns `u32::MAX` (scene_store sparse ABSENT sentinel).
    ///
    /// Id-space exhaustion panics (host abort). A `Result` / edit reject path is a
    /// possible later refinement; not in this slice.
    pub fn alloc_entity_id(&mut self, scene_entries: &[SceneEntry]) -> u32 {
        while let Some(id) = self.freelist.pop() {
            if id != u32::MAX && !self.id_in_use(id, scene_entries) && !scene_entry_is_user_root(id)
            {
                self.reserved.insert(id);
                return id;
            }
        }
        let max_content = scene_entries
            .iter()
            .map(|e| e.entity_id)
            .chain(self.store.entity_ids().iter().copied())
            .chain(self.pending_despawns.iter().copied())
            .chain(self.reserved.iter().copied())
            .filter(|id| !scene_entry_is_user_root(*id) && *id != u32::MAX)
            .max()
            .unwrap_or(0);
        let mut candidate = max_content.saturating_add(1).max(100);
        let start = candidate;
        loop {
            if candidate == u32::MAX {
                candidate = 100;
            } else if scene_entry_is_user_root(candidate) {
                candidate = user_root_entity_last().saturating_add(1);
            } else if self.id_in_use(candidate, scene_entries) {
                candidate = candidate.saturating_add(1);
            } else {
                self.reserved.insert(candidate);
                return candidate;
            }
            // Full scan of the non-sentinel id space with no free slot.
            if candidate == start {
                panic!(
                    "LiveEditLedger::alloc_entity_id: no free entity id (start={start}; store_len={})",
                    self.store.len()
                );
            }
        }
    }

    fn id_in_use(&self, id: u32, scene_entries: &[SceneEntry]) -> bool {
        self.store.contains(id)
            || self.pending_despawns.contains(&id)
            || self.reserved.contains(&id)
            || scene_entries.iter().any(|e| e.entity_id == id)
    }

    /// Restore store after a live op that never completed barrier (tip unchanged).
    ///
    /// Returns `false` if the compensating apply failed (SSoT may remain optimistic).
    /// Callers that rolled forward several ops must invoke this in **reverse** order.
    #[must_use = "rollback failure leaves optimistic SSoT; callers must not ignore"]
    pub fn rollback_mirrors(&mut self, ready: &ReadyLiveEdit) -> bool {
        let ok = match ready {
            ReadyLiveEdit::Reparent {
                op, prev_parent, ..
            } => self
                .apply_reparent(op.child_entity_id, *prev_parent)
                .is_success(),
            ReadyLiveEdit::SetField { op, prev_value, .. } => {
                let Some(value) = prop_from_field_bytes(op.field_id, prev_value) else {
                    log::error!(
                        "live edit rollback: cannot decode prev_value field_id={} entity={}",
                        op.field_id,
                        op.entity_id
                    );
                    return false;
                };
                self.apply_set_field(op.entity_id, op.field_id, value)
                    .is_success()
            }
        };
        if !ok {
            log::error!(
                "live edit rollback failed (tip unchanged; SSoT may stay optimistic): {:?}",
                ready.proposal().op_id
            );
        }
        ok
    }
}

/// Map [`ApplyOutcome`] → wire `edit_reject` (facade quarantine already covered
/// expected_parent / presence; this is the store residual).
pub fn edit_reject_for_apply(outcome: ApplyOutcome) -> u32 {
    use alchemy_protocol::edit_reject;
    match outcome {
        ApplyOutcome::Applied | ApplyOutcome::AppliedAsReparent => edit_reject::NONE,
        ApplyOutcome::RejectedCycle => edit_reject::CYCLE,
        ApplyOutcome::RejectedStale => edit_reject::STALE_PRECONDITION,
        ApplyOutcome::RejectedMissing => edit_reject::NOT_FOUND,
        // `u32::MAX` / corrupt id: edit quarantine usually blocks earlier; still
        // map if a residual reaches apply_sparse (defense-in-depth → PAYLOAD).
        ApplyOutcome::RejectedInvalidId => edit_reject::PAYLOAD,
        // Depth / clock are store gates, not graph CYCLE.
        ApplyOutcome::RejectedDepthExceeded | ApplyOutcome::RejectedClockDrift => {
            edit_reject::APPLY
        }
    }
}

#[inline]
fn wire_parent(parent_entity_id: u32) -> Option<u32> {
    if parent_entity_id == 0 {
        None
    } else {
        Some(parent_entity_id)
    }
}

fn prop_from_field_bytes(field_id: u32, bytes: &[u8]) -> Option<PropValue> {
    match field_id {
        edit_field_id::FLAGS => alchemy_protocol::decode_flags(bytes).map(PropValue::U32),
        edit_field_id::COLOR_RGBA => {
            alchemy_protocol::decode_color_rgba(bytes).map(PropValue::F32x4)
        }
        edit_field_id::BLAKE3 => {
            let hex = std::str::from_utf8(bytes).ok()?.to_ascii_lowercase();
            Some(PropValue::Bytes(hex.into_bytes()))
        }
        _ => None,
    }
}

/// Accepted live op awaiting FrameBridge queue + barrier before tip/Commit.
#[derive(Clone, Debug)]
pub enum ReadyLiveEdit {
    Reparent {
        proposal: EditProposal,
        op: EditReparent,
        prev_parent: u32,
    },
    SetField {
        proposal: EditProposal,
        op: EditSetField,
        prev_value: Vec<u8>,
    },
}

impl ReadyLiveEdit {
    pub fn proposal(&self) -> &EditProposal {
        match self {
            Self::Reparent { proposal, .. } | Self::SetField { proposal, .. } => proposal,
        }
    }
}

pub struct LiveEditInbox {
    rx: Mutex<Receiver<ReadyLiveEdit>>,
}

impl LiveEditInbox {
    pub fn pair() -> (Sender<ReadyLiveEdit>, Self) {
        let (tx, rx) = mpsc::channel();
        (tx, Self { rx: Mutex::new(rx) })
    }

    pub fn try_recv(&self) -> Option<ReadyLiveEdit> {
        self.rx.lock().ok()?.try_recv().ok()
    }

    pub fn try_recv_n(&self, max: usize) -> Vec<ReadyLiveEdit> {
        let mut out = Vec::new();
        for _ in 0..max {
            match self.try_recv() {
                Some(r) => out.push(r),
                None => break,
            }
        }
        out
    }

    #[cfg(test)]
    pub fn empty_for_test() -> Self {
        let (_tx, rx) = mpsc::channel();
        Self { rx: Mutex::new(rx) }
    }
}

/// Field bytes currently stored for `field_id` (for hash compare).
pub fn current_field_value(
    ledger: &LiveEditLedger,
    entity_id: u32,
    field_id: u32,
) -> Option<Vec<u8>> {
    if !ledger.contains_entity(entity_id) {
        return None;
    }
    match field_id {
        edit_field_id::FLAGS => {
            let f = ledger.flags_of(entity_id).unwrap_or(0);
            Some(alchemy_protocol::encode_flags(f).to_vec())
        }
        edit_field_id::COLOR_RGBA => {
            let c = ledger.color_of(entity_id).unwrap_or([1.0, 1.0, 1.0, 1.0]);
            Some(alchemy_protocol::encode_color_rgba(c).to_vec())
        }
        edit_field_id::BLAKE3 => {
            let hex = ledger.blake3_bytes(entity_id)?;
            Some(hex.to_vec())
        }
        _ => None,
    }
}

/// Publishes accept `EditCommit` after barrier (host physics tick).
pub struct LiveCommitPublisher {
    session: zenoh::Session,
    edit_key: String,
    room_id: String,
    room_aead: SharedRoomAead,
    host_key: SigningKey,
}

impl LiveCommitPublisher {
    pub fn new(
        session: zenoh::Session,
        edit_key: String,
        room_id: String,
        room_aead: SharedRoomAead,
        host_key: SigningKey,
    ) -> Self {
        Self {
            session,
            edit_key,
            room_id,
            room_aead,
            host_key,
        }
    }

    pub fn publish_accept(&self, proposal: &EditProposal, scene_rev: u64) {
        self.publish(
            proposal,
            scene_rev,
            true,
            alchemy_protocol::edit_reject::NONE,
        );
    }

    /// Queue / barrier / inbox failure NACK (tip unchanged).
    pub fn publish_reject(&self, proposal: &EditProposal, reason: u32, scene_rev: u64) {
        self.publish(proposal, scene_rev, false, reason);
    }

    fn publish(&self, proposal: &EditProposal, scene_rev: u64, accepted: bool, reject_reason: u32) {
        let mut commit = EditCommit {
            scene_rev,
            op_id: proposal.op_id,
            author_session_pub: proposal.session_pub.clone(),
            accepted,
            reject_reason,
            ops: proposal.ops.clone(),
            host_sig: vec![],
        };
        sign_edit_commit(&self.host_key, &mut commit);
        let plain = commit.encode_to_vec();
        match room_aead::seal_shared(&self.room_aead, &self.room_id, AeadKind::EditCommit, &plain) {
            Ok(env) => {
                if let Err(e) = self.session.put(&self.edit_key, env).wait() {
                    log::warn!("live EditCommit put failed: {e}");
                }
            }
            Err(e) => log::warn!("live EditCommit seal failed: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alchemy_protocol::wire_crypto::{
        generate_signing_key, public_key_bytes, sign_edit_proposal,
    };

    fn proposal() -> EditProposal {
        let key = generate_signing_key();
        let mut p = EditProposal {
            session_pub: public_key_bytes(&key),
            control_seq: 1,
            base_scene_rev: 0,
            op_id: 1,
            ops: vec![],
            sig: vec![],
            room_id: "local".into(),
        };
        sign_edit_proposal(&key, &mut p);
        p
    }

    #[test]
    fn rollback_mirrors_does_not_touch_tip() {
        let mut led = LiveEditLedger::default();
        assert!(led.note_spawn_with_parent(1, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.note_spawn_with_parent(10, 1, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        led.tip = 7;
        let ready = ReadyLiveEdit::Reparent {
            proposal: proposal(),
            op: EditReparent {
                child_entity_id: 10,
                expected_parent_entity_id: 1,
                new_parent_entity_id: 2,
            },
            prev_parent: 1,
        };
        assert!(led.note_spawn_with_parent(2, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.apply_reparent(10, 2).is_success());
        assert!(led.store().pending_patches().is_empty());
        assert!(led.rollback_mirrors(&ready));
        assert_eq!(led.tip, 7);
        assert_eq!(led.parent_of(10), 1);
        assert!(led.store().pending_patches().is_empty());
    }

    #[test]
    fn rollback_set_field_restores_flags() {
        use alchemy_protocol::encode_flags;
        let mut led = LiveEditLedger::default();
        assert!(led.note_spawn_with_parent(10, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        led.tip = 3;
        let prev = encode_flags(0).to_vec();
        assert!(led
            .apply_set_field(10, edit_field_id::FLAGS, PropValue::U32(7))
            .is_success());
        let ready = ReadyLiveEdit::SetField {
            proposal: proposal(),
            op: EditSetField {
                entity_id: 10,
                field_id: edit_field_id::FLAGS,
                value: encode_flags(7).to_vec(),
                expected_value_hash: vec![],
            },
            prev_value: prev,
        };
        assert!(led.rollback_mirrors(&ready));
        assert_eq!(led.tip, 3);
        assert_eq!(led.flags_of(10), Some(0));
    }

    #[test]
    fn queue_fail_policy_lifo_rolls_failed_and_drain_tail() {
        // Mirrors host_loop: on queue fail at index i, LIFO-rollback batch[i..]
        // while keeping earlier successfully-queued ops' ledger state.
        let mut led = LiveEditLedger::default();
        assert!(led.note_spawn_with_parent(1, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.note_spawn_with_parent(2, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.note_spawn_with_parent(3, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.note_spawn_with_parent(10, 1, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.apply_reparent(10, 2).is_success());
        let r1 = ReadyLiveEdit::Reparent {
            proposal: proposal(),
            op: EditReparent {
                child_entity_id: 10,
                expected_parent_entity_id: 1,
                new_parent_entity_id: 2,
            },
            prev_parent: 1,
        };
        assert!(led.apply_reparent(10, 3).is_success());
        let r2 = ReadyLiveEdit::Reparent {
            proposal: proposal(),
            op: EditReparent {
                child_entity_id: 10,
                expected_parent_entity_id: 2,
                new_parent_entity_id: 3,
            },
            prev_parent: 2,
        };
        // Simulate: r1 queued OK; r2 queue fails → rollback tail [r2] only.
        let batch = [r1, r2];
        let fail_idx = 1;
        for ready in batch[fail_idx..].iter().rev() {
            assert!(led.rollback_mirrors(ready));
        }
        assert_eq!(led.parent_of(10), 2);
        // If fail were at 0, LIFO [r2,r1] restores root parent 1.
        assert!(led.apply_reparent(10, 3).is_success());
        for ready in batch[0..].iter().rev() {
            assert!(led.rollback_mirrors(ready));
        }
        assert_eq!(led.parent_of(10), 1);
    }

    #[test]
    fn rollback_mirrors_lifo_restores_final_parent() {
        let mut led = LiveEditLedger::default();
        assert!(led.note_spawn_with_parent(1, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.note_spawn_with_parent(2, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.note_spawn_with_parent(3, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.note_spawn_with_parent(10, 1, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        // 10: 1 → 2 → 3
        assert!(led.apply_reparent(10, 2).is_success());
        let r1 = ReadyLiveEdit::Reparent {
            proposal: proposal(),
            op: EditReparent {
                child_entity_id: 10,
                expected_parent_entity_id: 1,
                new_parent_entity_id: 2,
            },
            prev_parent: 1,
        };
        assert!(led.apply_reparent(10, 3).is_success());
        let r2 = ReadyLiveEdit::Reparent {
            proposal: proposal(),
            op: EditReparent {
                child_entity_id: 10,
                expected_parent_entity_id: 2,
                new_parent_entity_id: 3,
            },
            prev_parent: 2,
        };
        assert_eq!(led.parent_of(10), 3);
        // FIFO would leave parent=2; LIFO restores 1.
        let awaiting = [r1, r2];
        for ready in awaiting.iter().rev() {
            assert!(led.rollback_mirrors(ready));
        }
        assert_eq!(led.parent_of(10), 1);
    }

    #[test]
    fn depth_exceeded_is_apply_not_cycle() {
        use alchemy_shared_sim::MAX_PARENT_HOPS;
        let mut led = LiveEditLedger::default();
        // Same topology as scene_store::move_under_deeper_than_max_parent_hops_rejects_depth:
        // chain 0→1→…→MAX (MAX edges leaf→root); attach outsider under leaf.
        let mut lamport = 1u64;
        let spawn =
            |led: &mut LiveEditLedger, lamport: &mut u64, node: u32, parent: Option<u32>| {
                let outcome = led.store_mut().apply_sparse(CrdtOp::SpawnNode {
                    ts: OpTimestamp::new(*lamport, HOST_ACTOR_ID),
                    node,
                    parent,
                });
                assert!(outcome.is_success(), "spawn {node}: {outcome:?}");
                *lamport += 1;
            };
        spawn(&mut led, &mut lamport, 0, None);
        for i in 1..=MAX_PARENT_HOPS as u32 {
            spawn(&mut led, &mut lamport, i, Some(i - 1));
        }
        let leaf = MAX_PARENT_HOPS as u32;
        let outsider = leaf + 1;
        spawn(&mut led, &mut lamport, outsider, None);
        let outcome = led.apply_reparent(outsider, leaf);
        assert_eq!(outcome, ApplyOutcome::RejectedDepthExceeded);
        assert_eq!(
            edit_reject_for_apply(outcome),
            alchemy_protocol::edit_reject::APPLY
        );
        assert_ne!(
            edit_reject_for_apply(outcome),
            alchemy_protocol::edit_reject::CYCLE
        );
    }

    #[test]
    fn facade_has_no_hashmap_mirrors() {
        let mut led = LiveEditLedger::default();
        assert!(led.note_spawn_with_parent(5, 0, 3, [0.1, 0.2, 0.3, 1.0], "ab".repeat(32)));
        assert!(led.contains_entity(5));
        assert_eq!(led.flags_of(5), Some(3));
        assert_eq!(led.color_of(5), Some([0.1, 0.2, 0.3, 1.0]));
        let hex = "ab".repeat(32);
        assert_eq!(
            led.blake3_bytes(5)
                .and_then(|b| std::str::from_utf8(b).ok()),
            Some(hex.as_str())
        );
        assert_eq!(led.parent_of(5), 0);
        // SSoT is the store only; facade Move/Set leave no SimPatch backlog.
        assert_eq!(led.store().len(), 1);
        assert!(led.store().pending_patches().is_empty());
    }
}
