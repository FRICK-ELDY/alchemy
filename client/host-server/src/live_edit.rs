//! Host live edit facade over `shared-sim::scene_store` (x00e0…x00e5).
//!
//! Contract (x00b3 / x00b9 / x00e0 / x00e1 / x00e2 / x00e3 / x00e5):
//! - Sparse SSoT is [`SceneStore`] (parents / fields / entities) — **no** host
//!   HashMap second copy.
//! - Host keeps quarantine, `tip` (`scene_rev`) sequencing, pending spawn/despawn
//!   / freelist / reserved, edit + Save I/O.
//! - [`Self::op_chain_hash`] + [`Self::fold_confirmed_op`] are the Phase 1c
//!   chain-tip update口 (call **after** [`Self::bump_tip`]). host_loop does
//!   **not** auto-fold yet (wire / Verifier carriage is a later slice).
//! - Optimistic store mutates on live accept; **`tip` is NOT bumped** until
//!   FrameBridge queue **and** barrier succeed (Reparent/SetField).
//!   Spawn/Despawn tip bumps after inbox send / physics-ok.
//! - Accept `EditCommit` only after that success (never rewind tip).
//! - Queue / barrier / inbox failure → `rollback_mirrors` + reject `EditCommit`
//!   (`APPLY`); tip untouched.
//! - Per-operator [`SlotUndoStack`] **records** barrier-confirmed Reparents
//!   (`note_reparent_undo`) and optional despawn→trash (`note_despawn_undo`).
//!   Applying undo/redo as tip/`EditCommit`/FrameBridge ops is **not** wired yet
//!   (§3.3 Intent) — see `undo_reparent` / `redo_reparent` (`cfg(test)` peel only).
//! - Logical delete = MoveNode to [`TRASH_ENTITY_ID`] + [`TrashStore`] GC
//!   (count/TTL). Save body still projects **`pending_despawns`** (1b→1c bridge).
//!   Session spawn cancel → freelist hard remove (not trash). GC hard-remove →
//!   Undo/Redo of that child is No-op (`Ok(None)`). GC'd ids stay reserved for
//!   alloc via `pending_despawns` until [`Self::clear_save_pending`].
//! - Child 退避: direct children move under the live restore parent (or root);
//!   despawn Undo restores **only the trashed node**, not evacuated children.
//! - [`SoftLeaseTable`] gates concurrent structural edits (Reparent / SetField) at
//!   the edit entrance (`allows` before mutate, `try_acquire` only after sparse
//!   success). Barrier / APPLY rollback keeps SoftLease so the same actor can
//!   retry without another actor stealing mid-window; physical grab
//!   (`holder_slot`) is a separate SSoT.
//!
//! Facade structural ops use [`SceneStore::apply_sparse`] (no `SimPatch` enqueue):
//! FrameBridge already owns dense Reparent/SetField until Arena drain is wired
//! from this ledger. Barrier / queue failure rolls back **LIFO** (reverse apply order).
//!
//! Non-guarantees: crash durability of unsaved pending; Spatial VCS flush;
//! Undo Intent / tip bump / EditCommit path; **production despawn does not call
//! [`Self::note_despawn_undo`]** (test peel only until Intent wires operator +
//! `from_parent`); HostLoop despawn [`NoteLiveDespawn::Failed`] rejects tip but
//! does **not** restore already-applied scene/physics rows (sparse may stay live
//! → rare SSoT skew until operator retry / Leave); leave-time `undo_stacks`
//! eviction; Arena SimPatch drain wiring; SoftLease ↔ physical grab integration;
//! StableUuid full path (→ x00e4); host_loop tip-path auto-fold of
//! `op_chain_hash` (→ later Reliable Op / Verifier wiring).

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use alchemy_protocol::wire_aead::AeadKind;
use alchemy_protocol::wire_crypto::{sign_edit_commit, SigningKey};
use alchemy_protocol::{
    edit_field_id, scene_entry_is_user_root, user_root_entity_last, EditCommit, EditProposal,
    EditReparent, EditSetField, Message, SceneEntry,
};
use alchemy_shared_sim::{
    fold_op_chain, is_trash_entity_id, ApplyOutcome, CrdtOp, OpTimestamp, PropValue, SceneStore,
    SequencedOp, SlotTransition, SlotUndoStack, SoftLease, SoftLeaseConflict, SoftLeaseTable,
    TrashStore, SOFT_LEASE_TTL_TICKS,
};
#[cfg(test)]
use alchemy_shared_sim::{UndoError, TRASH_ENTITY_ID};
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

/// Outcome of [`LiveEditLedger::note_live_despawn`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteLiveDespawn {
    /// Canonical id moved to trash (or already trashed; pending kept).
    ///
    /// `restore_parent`: on **first** trash, `None` = was world root. On
    /// **idempotent re-entry**, always `None` and **must not** be used as Undo
    /// `from_parent` — keep the first-trash value separately.
    Trashed { restore_parent: Option<u32> },
    /// Unsaved session spawn cancelled → freelist hard remove.
    SessionRecycled,
    /// Sparse still live; Save projection not claimed.
    Failed,
}

/// Shared live working-tree tip + scene_store facade (edit lane + physics).
#[derive(Debug, Default)]
pub struct LiveEditLedger {
    /// Monotonic EditCommit tip (advanced only after barrier success).
    pub tip: u64,
    /// Host-sequenced op chain tip (x00e5 枠). Updated only via
    /// [`Self::fold_confirmed_op`] after tip bump — not on the wire yet.
    // x00e5 枠: host_loop tip-path fold not wired yet (bin dead_code; used in tests).
    #[cfg_attr(not(test), allow(dead_code))]
    pub op_chain_hash: [u8; 32],
    /// Session sparse SSoT (parents / fields / entities).
    store: SceneStore,
    /// Unsaved session spawns (Save body `spawns`); ticket kept until Save or local despawn.
    pub pending_spawns: HashMap<u32, PendingSpawn>,
    /// Save-body `despawns` projection for canonical entities moved to trash
    /// (1b→1c bridge; live SSoT is TRASH MoveNode + [`TrashStore`]).
    pub pending_despawns: HashSet<u32>,
    /// Recycled ids from cancelled session spawns only (not canonical pending_despawns).
    pub freelist: Vec<u32>,
    /// Allocated ids not yet in the store / scene (concurrent spawn race).
    pub reserved: HashSet<u32>,
    /// Per-operator session undo (key = `session_pub` bytes). Populated on
    /// barrier-confirmed Reparent / despawn→trash; not persisted across leave.
    /// Leave/Save do not clear this map (non-guarantee). Trash GC → Undo No-op.
    undo_stacks: HashMap<Vec<u8>, SlotUndoStack>,
    /// SoftLease table (entity → holder). Advanced by [`Self::advance_lease_tick`].
    soft_leases: SoftLeaseTable,
    /// Logical-delete index (FIFO count/TTL GC). Advanced with SoftLease tick.
    trash: TrashStore,
    /// Host tick clock for SoftLease / Trash TTL (not `tip` / scene_rev).
    lease_tick: u64,
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

    /// Fold a confirmed op into [`Self::op_chain_hash`] and return the matching
    /// [`SequencedOp`] (hash + payload are the same fold).
    ///
    /// Call **after** [`Self::bump_tip`] so `room_seq` matches the accepted
    /// `scene_rev`. Does not publish to Zenoh / EditCommit (枠 only).
    ///
    /// Edit* → [`CrdtOp`] rebuild map (host_loop tip wiring):
    /// - Spawn → [`CrdtOp::SpawnNode`]
    /// - Despawn / [`NoteLiveDespawn::Trashed`] → [`CrdtOp::MoveNode`] to
    ///   trash (`TRASH_ENTITY_ID`)
    /// - Despawn / [`NoteLiveDespawn::SessionRecycled`] → hard remove + freelist
    ///   (unsaved session cancel). **Not** MoveNode(TRASH). `bump_tip` may
    ///   still run; do **not** fold a TRASH MoveNode for this path (dedicated
    ///   chain expression undefined — skip fold until one exists).
    /// - Reparent → [`CrdtOp::MoveNode`]
    /// - SetField → [`CrdtOp::SetProperty`]
    // x00e5 枠: host_loop tip-path fold not wired yet (bin dead_code; used in tests).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn fold_confirmed_op(&mut self, host_epoch: u64, op: CrdtOp) -> SequencedOp {
        debug_assert!(
            self.tip > 0,
            "fold_confirmed_op requires bump_tip first (room_seq must be > 0)"
        );
        self.op_chain_hash = fold_op_chain(&self.op_chain_hash, host_epoch, self.tip, &op);
        SequencedOp {
            host_epoch,
            room_seq: self.tip,
            op_chain_hash: self.op_chain_hash,
            op,
        }
    }

    /// Advance SoftLease + Trash GC clock (call once per host physics tick).
    pub fn advance_lease_tick(&mut self) {
        self.lease_tick = self.lease_tick.saturating_add(1);
        self.soft_leases.purge_expired(self.lease_tick);
        self.gc_trash();
    }

    /// Hard-remove TrashStore GC candidates (`remove_node_sparse`).
    ///
    /// [`Self::pending_despawns`] is **kept** until Save folds them. Undo of a
    /// GC'd child is No-op (`Ok(None)`).
    fn gc_trash(&mut self) {
        for entity_id in self.trash.gc(self.lease_tick) {
            let _ = self.store.remove_node_sparse(entity_id);
            self.soft_leases.clear(entity_id);
        }
    }

    /// Test helper: force Trash GC with custom TTL/cap (does not advance tick).
    #[cfg(test)]
    fn gc_trash_with_limits(&mut self, ttl_ticks: u64, max_entries: usize) {
        for entity_id in self
            .trash
            .gc_with_limits(self.lease_tick, ttl_ticks, max_entries)
        {
            let _ = self.store.remove_node_sparse(entity_id);
            self.soft_leases.clear(entity_id);
        }
    }

    /// SoftLease conflict check (no TTL refresh). Call before sparse mutate.
    pub fn soft_lease_allows(
        &self,
        entity_id: u32,
        actor_id: u64,
    ) -> Result<(), SoftLeaseConflict> {
        self.soft_leases
            .allows(entity_id, actor_id, self.lease_tick)
    }

    /// SoftLease acquire/refresh after a **successful** sparse apply.
    ///
    /// Failed decode / CYCLE / etc. must not call this (avoids TTL extension
    /// without a scene change). Barrier / APPLY rollback intentionally leaves
    /// the lease so the same actor can retry.
    pub fn try_acquire_soft_lease(
        &mut self,
        entity_id: u32,
        actor_id: u64,
    ) -> Result<(), SoftLeaseConflict> {
        self.soft_leases
            .try_acquire(entity_id, actor_id, self.lease_tick, SOFT_LEASE_TTL_TICKS)
    }

    /// Active SoftLease for `entity_id` (x00f3 visibility / SoftLease reject fill).
    #[inline]
    pub fn soft_lease_of(&self, entity_id: u32) -> Option<SoftLease> {
        self.soft_leases
            .get(entity_id)
            .filter(|l| l.is_active(self.lease_tick))
    }

    /// Test helper: drop SoftLease without despawn (same-tick race with accept Commit).
    #[cfg(test)]
    pub(crate) fn clear_soft_lease_for_test(&mut self, entity_id: u32) {
        self.soft_leases.clear(entity_id);
    }

    #[inline]
    fn next_ts(&self) -> OpTimestamp {
        OpTimestamp::new(self.store.max_lamport().saturating_add(1), HOST_ACTOR_ID)
    }

    /// World-root parent is `0` on the wire / Save body.
    pub fn parent_of(&self, child: u32) -> u32 {
        self.store.parent(child).unwrap_or(0)
    }

    /// Live (editable) entity — present and **not** under trash.
    #[inline]
    pub fn contains_entity(&self, entity_id: u32) -> bool {
        self.store.is_live(entity_id)
    }

    /// Dense sparse ids (**includes trashed** rows still awaiting GC / Save clear).
    ///
    /// For editable / Save live diffs use [`Self::contains_entity`] (`is_live`)
    /// as a filter — this slice keeps one packed walk; a dedicated live-only
    /// API is deferred.
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
    /// `new_parent == TRASH_ENTITY_ID` / non-live ids are rejected — use
    /// [`Self::note_live_despawn`] for logical delete.
    pub fn apply_reparent(&mut self, child: u32, new_parent: u32) -> ApplyOutcome {
        if is_trash_entity_id(new_parent) || is_trash_entity_id(child) {
            return ApplyOutcome::RejectedInvalidId;
        }
        if !self.store.is_live(child) {
            return ApplyOutcome::RejectedMissing;
        }
        if new_parent != 0 && !self.store.is_live(new_parent) {
            return ApplyOutcome::RejectedMissing;
        }
        self.store.apply_sparse(CrdtOp::MoveNode {
            ts: self.next_ts(),
            node: child,
            new_parent: wire_parent(new_parent),
        })
    }

    /// Apply quarantined SetField into scene_store (optimistic; tip unchanged).
    ///
    /// Non-live / trashed targets → [`ApplyOutcome::RejectedMissing`].
    pub fn apply_set_field(
        &mut self,
        entity_id: u32,
        field_id: u32,
        value: PropValue,
    ) -> ApplyOutcome {
        if is_trash_entity_id(entity_id) || !self.store.is_live(entity_id) {
            return ApplyOutcome::RejectedMissing;
        }
        self.store.apply_sparse(CrdtOp::SetProperty {
            ts: self.next_ts(),
            node: entity_id,
            prop_key: field_id,
            value,
        })
    }

    /// Drop trash index + Save projection (and hard-remove if still under trash)
    /// so [`SceneStore::seed_node`] can insert cleanly (no silent un-trash /
    /// no stale `pending_despawns` after GC).
    fn clear_trashed_for_reseed(&mut self, entity_id: u32) {
        let under_trash = self.store.is_trashed(entity_id);
        let pending = self.pending_despawns.contains(&entity_id);
        if !under_trash && !pending {
            return;
        }
        self.trash.forget(entity_id);
        self.pending_despawns.remove(&entity_id);
        self.soft_leases.clear(entity_id);
        if under_trash {
            let _ = self.store.remove_node_sparse(entity_id);
        }
    }

    /// Record a newly spawned entity. Preserves an existing live parent if already set
    /// (`seed` with root only when absent — does not clobber a prior Reparent on re-note).
    /// Returns `false` if seed or props apply fails.
    ///
    /// Rollback on props failure: **fresh** insert → `remove_node_sparse` (no half-write).
    /// **Re-note** (`!fresh`): any props already applied in this call are **left
    /// as-is** (no snapshot undo); caller sees `false` and must not treat the note
    /// as fully successful. Same contract as [`Self::note_spawn_with_parent`].
    /// Trashed / pending-despawn ids are cleared (forget + optional hard remove)
    /// before seed — including GC'd rows that only linger in `pending_despawns`.
    #[must_use = "seed/props failure; do not ignore"]
    pub fn note_spawn(
        &mut self,
        entity_id: u32,
        flags: u32,
        color: [f32; 4],
        blake3: String,
    ) -> bool {
        self.clear_trashed_for_reseed(entity_id);
        let fresh = !self.store.contains(entity_id);
        if fresh && !self.store.seed_node(entity_id, None) {
            return false;
        }
        if !self.write_spawn_props(entity_id, flags, color, blake3) {
            if fresh {
                let _ = self.store.remove_node_sparse(entity_id);
            }
            return false;
        }
        true
    }

    /// Seed or overwrite including creative parent (x00b8 boot / scene install).
    /// Returns `false` if seed or props apply fails.
    ///
    /// Rollback on props failure: **fresh** insert → `remove_node_sparse` (no half-write).
    /// **Re-seed** (`was_live`): parent link from `seed_node` and any props already
    /// applied in this call are **left as-is** (no snapshot undo); caller sees
    /// `false` and must not treat the note as fully successful.
    /// Trashed / pending-despawn ids are cleared before seed (see [`Self::note_spawn`]).
    #[must_use = "seed/props failure; do not ignore"]
    pub fn note_spawn_with_parent(
        &mut self,
        entity_id: u32,
        parent_entity_id: u32,
        flags: u32,
        color: [f32; 4],
        blake3: String,
    ) -> bool {
        self.clear_trashed_for_reseed(entity_id);
        let was_live = self.store.is_live(entity_id);
        if !self
            .store
            .seed_node(entity_id, wire_parent(parent_entity_id))
        {
            return false;
        }
        if !self.write_spawn_props(entity_id, flags, color, blake3) {
            if !was_live {
                let _ = self.store.remove_node_sparse(entity_id);
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

    /// Logical delete: MoveNode to trash (no FreeSlot; FrameBridge owns dense free).
    ///
    /// On **first** success returns the restore parent for Undo (`None` = world
    /// root) and records [`TrashStore`]. Does **not** prune undo (use
    /// [`Self::note_despawn_undo`]). Caller owns [`Self::pending_despawns`].
    ///
    /// **Idempotent re-entry** (already trashed): returns `Some(None)` — this is
    /// **not** a restore parent. Intent wiring must keep `from_parent` from the
    /// first trash (or `note_despawn_undo`), never from a later re-entry.
    ///
    /// On failure the store is left unchanged (child 退避 is atomic). Missing /
    /// invalid ids only clear SoftLease / trash index — **no** hard remove of a
    /// still-present node after a failed evacuate.
    #[must_use = "prev_parent needed for note_despawn_undo (first trash only)"]
    pub fn note_despawn(&mut self, entity_id: u32) -> Option<Option<u32>> {
        if is_trash_entity_id(entity_id) || entity_id == u32::MAX {
            return None;
        }
        // Idempotent: already trashed — keep index / SoftLease clear; no Ok(TRASH).
        // `Some(None)` ≠ "was world-rooted"; do not feed Undo from_parent.
        if self.store.is_trashed(entity_id) {
            self.trash.note(entity_id, self.lease_tick);
            self.soft_leases.clear(entity_id);
            return Some(None);
        }
        let ts = self.next_ts();
        match self.store.move_to_trash_sparse(entity_id, ts) {
            Ok(restore_parent) => {
                self.trash.note(entity_id, self.lease_tick);
                self.soft_leases.clear(entity_id);
                Some(restore_parent)
            }
            Err(ApplyOutcome::RejectedMissing) | Err(ApplyOutcome::RejectedInvalidId) => {
                if !self.store.contains(entity_id) {
                    self.trash.forget(entity_id);
                    self.soft_leases.clear(entity_id);
                }
                None
            }
            Err(_) => None,
        }
    }

    /// Record a barrier-confirmed Reparent for the operator's undo stack (x00e1).
    ///
    /// `from_parent` / `to_parent` are wire parents (`0` = world root).
    /// Recording only — applying inverse still requires Intent / edit lane
    /// (tip + EditCommit + FrameBridge) before production use.
    ///
    /// No-op when `child` is no longer live (e.g. trash / GC raced ahead of
    /// barrier Ok) so missing ids do not re-enter the stack.
    pub fn note_reparent_undo(
        &mut self,
        operator: &[u8],
        child: u32,
        from_parent: u32,
        to_parent: u32,
    ) {
        if !self.store.is_live(child) {
            return;
        }
        if is_trash_entity_id(to_parent) {
            // Despawn undo uses [`Self::note_despawn_undo`].
            return;
        }
        let t = SlotTransition {
            child,
            from: wire_parent(from_parent),
            to: wire_parent(to_parent),
        };
        if let Some(stack) = self.undo_stacks.get_mut(operator) {
            stack.push(t);
        } else {
            let mut stack = SlotUndoStack::default();
            stack.push(t);
            self.undo_stacks.insert(operator.to_vec(), stack);
        }
    }

    /// Record logical delete for undo (MoveNode → [`TRASH_ENTITY_ID`]).
    ///
    /// Call after a successful [`Self::note_despawn`] / [`Self::note_live_despawn`]
    /// while the node is still under trash (before GC). `from_parent` is the wire
    /// parent before trash (`0` = world root).
    ///
    /// **Test / Intent peel only** — HostLoop does not call this yet; drop
    /// `cfg(test)` when Undo Intent wires operator + first-trash `from_parent`.
    #[cfg(test)]
    pub fn note_despawn_undo(&mut self, operator: &[u8], child: u32, from_parent: u32) {
        if !self.store.is_trashed(child) {
            return;
        }
        let t = SlotTransition {
            child,
            from: wire_parent(from_parent),
            to: Some(TRASH_ENTITY_ID),
        };
        if let Some(stack) = self.undo_stacks.get_mut(operator) {
            stack.push(t);
        } else {
            let mut stack = SlotUndoStack::default();
            stack.push(t);
            self.undo_stacks.insert(operator.to_vec(), stack);
        }
    }

    /// Drop undo/redo entries whose **`child`** is `entity_id` (session spawn cancel).
    ///
    /// Does **not** strip entries that only mention the id as `from` / `to`
    /// (parent role). Canonical trash deletes keep stack entries for restore /
    /// GC No-op.
    fn prune_undo_for_entity(&mut self, entity_id: u32) {
        self.undo_stacks.retain(|_, stack| {
            stack.discard_child(entity_id);
            stack.undo_len() > 0 || stack.redo_len() > 0
        });
    }

    /// **Test / library peel only** — sparse Move without tip / EditCommit /
    /// FrameBridge. Production must not call this until Undo Intent is wired
    /// (§3.3). Uses host-sequenced `actor_id` (0); operator key is `session_pub`.
    #[cfg(test)]
    pub fn undo_reparent(&mut self, operator: &[u8]) -> Result<Option<SlotTransition>, UndoError> {
        self.undo_reparent_sparse_peel(operator, HOST_ACTOR_ID)
    }

    /// **Test / library peel only** — see [`Self::undo_reparent`].
    #[cfg(test)]
    pub fn redo_reparent(&mut self, operator: &[u8]) -> Result<Option<SlotTransition>, UndoError> {
        self.redo_reparent_sparse_peel(operator, HOST_ACTOR_ID)
    }

    #[cfg(test)]
    fn undo_reparent_sparse_peel(
        &mut self,
        operator: &[u8],
        actor_id: u64,
    ) -> Result<Option<SlotTransition>, UndoError> {
        let Some(mut stack) = self.undo_stacks.remove(operator) else {
            return Ok(None);
        };
        let result = stack.undo_checked_store_sparse(&mut self.store, actor_id);
        if let Ok(Some(inv)) = &result {
            // Restore-from-trash: drop Save projection + trash index.
            if self.store.is_live(inv.child) {
                self.trash.forget(inv.child);
                self.pending_despawns.remove(&inv.child);
            }
        }
        if stack.undo_len() > 0 || stack.redo_len() > 0 {
            self.undo_stacks.insert(operator.to_vec(), stack);
        }
        result
    }

    #[cfg(test)]
    fn redo_reparent_sparse_peel(
        &mut self,
        operator: &[u8],
        actor_id: u64,
    ) -> Result<Option<SlotTransition>, UndoError> {
        let Some(mut stack) = self.undo_stacks.remove(operator) else {
            return Ok(None);
        };
        let result = stack.redo_store_sparse(&mut self.store, actor_id);
        if stack.undo_len() > 0 || stack.redo_len() > 0 {
            self.undo_stacks.insert(operator.to_vec(), stack);
        }
        result
    }

    #[cfg(test)]
    fn undo_len_for(&self, operator: &[u8]) -> usize {
        self.undo_stacks
            .get(operator)
            .map(|s| s.undo_len())
            .unwrap_or(0)
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

    /// Live despawn: session spawn cancel → freelist hard remove; canonical →
    /// trash MoveNode + Save `pending_despawns` projection.
    ///
    /// Already-trashed re-entry keeps `pending_despawns` (idempotent
    /// [`NoteLiveDespawn::Trashed`]). Production HostLoop does **not** record
    /// despawn Undo yet (non-guarantee; Intent).
    #[must_use = "outcome distinguishes trash / recycle / failure for tip Accept"]
    pub fn note_live_despawn(&mut self, entity_id: u32) -> NoteLiveDespawn {
        self.reserved.remove(&entity_id);
        if self.pending_spawns.remove(&entity_id).is_some() {
            self.freelist.push(entity_id);
            let _ = self.store.remove_node_sparse(entity_id);
            self.trash.forget(entity_id);
            self.soft_leases.clear(entity_id);
            self.prune_undo_for_entity(entity_id);
            NoteLiveDespawn::SessionRecycled
        } else {
            self.pending_despawns.insert(entity_id);
            match self.note_despawn(entity_id) {
                Some(restore_parent) => NoteLiveDespawn::Trashed { restore_parent },
                None => {
                    if self.store.is_trashed(entity_id) {
                        // Race: still trashed — keep Save projection.
                        NoteLiveDespawn::Trashed {
                            restore_parent: None,
                        }
                    } else {
                        // Never left the live tree — do not claim a Save delete.
                        self.pending_despawns.remove(&entity_id);
                        NoteLiveDespawn::Failed
                    }
                }
            }
        }
    }

    /// Clear Save-folded pending after successful world Save.
    ///
    /// Hard-removes any still-trashed ids that were projected in `despawns`
    /// (Save already deleted them from positive world).
    pub fn clear_save_pending(&mut self) {
        self.pending_spawns.clear();
        for entity_id in self.pending_despawns.drain() {
            let _ = self.store.remove_node_sparse(entity_id);
            self.trash.forget(entity_id);
            self.soft_leases.clear(entity_id);
        }
    }

    /// Host-local entity id (scene max+1 / freelist; skip user_root + pending_despawns).
    /// Never returns `u32::MAX` (sparse ABSENT) or [`TRASH_ENTITY_ID`].
    ///
    /// Id-space exhaustion panics (host abort). A `Result` / edit reject path is a
    /// possible later refinement; not in this slice.
    pub fn alloc_entity_id(&mut self, scene_entries: &[SceneEntry]) -> u32 {
        while let Some(id) = self.freelist.pop() {
            if Self::is_allocable_id(id)
                && !self.id_in_use(id, scene_entries)
                && !scene_entry_is_user_root(id)
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
            .filter(|id| !scene_entry_is_user_root(*id) && Self::is_allocable_id(*id))
            .max()
            .unwrap_or(0);
        let mut candidate = max_content.saturating_add(1).max(100);
        let start = candidate;
        loop {
            if !Self::is_allocable_id(candidate) {
                // Skip ABSENT + TRASH sentinels; wrap after ABSENT.
                candidate = if candidate == u32::MAX {
                    100
                } else {
                    candidate.saturating_add(1)
                };
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

    #[inline]
    fn is_allocable_id(id: u32) -> bool {
        id != u32::MAX && !is_trash_entity_id(id)
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
                // x00f2 POS: SoftLease-only accept path leaves `prev_value` empty
                // (no sparse write) — rollback is a no-op for the store.
                if alchemy_protocol::is_pos_field_id(op.field_id) && prev_value.is_empty() {
                    true
                } else {
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
        // POS_* is SoftLease-only (no sparse write / no rollback blob · x00f2).
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
        /// Acquiring actor fixed on Ready (x00f3 accept Commit; not table re-lookup).
        lease_holder_actor_id: u64,
    },
    SetField {
        proposal: EditProposal,
        op: EditSetField,
        prev_value: Vec<u8>,
        /// Acquiring actor fixed on Ready (x00f3 accept Commit; not table re-lookup).
        lease_holder_actor_id: u64,
    },
}

impl ReadyLiveEdit {
    pub fn proposal(&self) -> &EditProposal {
        match self {
            Self::Reparent { proposal, .. } | Self::SetField { proposal, .. } => proposal,
        }
    }

    /// Acquiring actor fixed when Ready is built (barrier accept; not SoftLease table re-lookup).
    #[inline]
    pub fn lease_holder_actor_id(&self) -> u64 {
        match self {
            Self::Reparent {
                lease_holder_actor_id,
                ..
            }
            | Self::SetField {
                lease_holder_actor_id,
                ..
            } => *lease_holder_actor_id,
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
///
/// POS_* is not a sparse property (Epoch is pose SSoT · x00f2) — returns `None`.
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

    pub fn publish_accept(
        &self,
        proposal: &EditProposal,
        scene_rev: u64,
        lease_holder_actor_id: u64,
    ) {
        self.publish(
            proposal,
            scene_rev,
            true,
            alchemy_protocol::edit_reject::NONE,
            lease_holder_actor_id,
        );
    }

    /// Queue / barrier / inbox failure NACK (tip unchanged).
    pub fn publish_reject(&self, proposal: &EditProposal, reason: u32, scene_rev: u64) {
        self.publish(proposal, scene_rev, false, reason, 0);
    }

    fn publish(
        &self,
        proposal: &EditProposal,
        scene_rev: u64,
        accepted: bool,
        reject_reason: u32,
        lease_holder_actor_id: u64,
    ) {
        let mut commit = EditCommit {
            scene_rev,
            op_id: proposal.op_id,
            author_session_pub: proposal.session_pub.clone(),
            accepted,
            reject_reason,
            ops: proposal.ops.clone(),
            host_sig: vec![],
            lease_holder_actor_id,
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
            lease_holder_actor_id: 1,
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
            lease_holder_actor_id: 1,
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
            lease_holder_actor_id: 1,
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
            lease_holder_actor_id: 1,
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
            lease_holder_actor_id: 1,
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
            lease_holder_actor_id: 1,
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

    #[test]
    fn facade_reparent_undo_redo_restores_parent() {
        let op_a = b"operator-a";
        let mut led = LiveEditLedger::default();
        assert!(led.note_spawn_with_parent(1, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.note_spawn_with_parent(2, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.note_spawn_with_parent(10, 1, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.apply_reparent(10, 2).is_success());
        led.note_reparent_undo(op_a, 10, 1, 2);
        assert_eq!(led.parent_of(10), 2);
        assert_eq!(led.undo_len_for(op_a), 1);

        let applied = led.undo_reparent(op_a).unwrap().unwrap();
        assert_eq!(applied.to, Some(1));
        assert_eq!(led.parent_of(10), 1);
        assert!(led.store().pending_patches().is_empty());

        let applied = led.redo_reparent(op_a).unwrap().unwrap();
        assert_eq!(applied.to, Some(2));
        assert_eq!(led.parent_of(10), 2);
    }

    #[test]
    fn facade_undo_stale_after_foreign_reparent() {
        let op_a = b"operator-a";
        let mut led = LiveEditLedger::default();
        assert!(led.note_spawn_with_parent(1, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.note_spawn_with_parent(2, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.note_spawn_with_parent(3, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.note_spawn_with_parent(10, 1, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.apply_reparent(10, 2).is_success());
        led.note_reparent_undo(op_a, 10, 1, 2);
        // Foreign operator wins parent.
        assert!(led.apply_reparent(10, 3).is_success());
        assert_eq!(led.undo_reparent(op_a), Err(UndoError::Stale));
        assert_eq!(led.parent_of(10), 3);
        assert_eq!(led.undo_len_for(op_a), 0);
    }

    #[test]
    fn facade_despawn_moves_to_trash_and_keeps_pending_despawns() {
        let mut led = LiveEditLedger::default();
        assert!(led.note_spawn_with_parent(100, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        // Canonical id (not in pending_spawns) → trash + pending_despawns for Save.
        assert_eq!(
            led.note_live_despawn(100),
            NoteLiveDespawn::Trashed {
                restore_parent: None
            }
        );
        assert!(!led.contains_entity(100));
        assert!(led.store().is_trashed(100));
        assert!(led.pending_despawns.contains(&100));
        assert!(led.store().pending_patches().is_empty());
        // Idempotent re-despawn keeps Save projection.
        assert_eq!(
            led.note_live_despawn(100),
            NoteLiveDespawn::Trashed {
                restore_parent: None
            }
        );
        assert!(led.pending_despawns.contains(&100));
        assert!(led.store().is_trashed(100));
        // Session spawn cancel recycles freelist instead of trash / pending_despawns.
        assert!(led.note_spawn_with_parent(200, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        led.note_pending_spawn(PendingSpawn {
            entry: SceneEntry {
                entity_id: 200,
                blake3: String::new(),
                x: 0.0,
                y: 0.0,
                z: 0.0,
                half_extent: 0.0,
                flags: 0,
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
                qx: 0.0,
                qy: 0.0,
                qz: 0.0,
                qw: 1.0,
                sx: 1.0,
                sy: 1.0,
                sz: 1.0,
                parent_entity_id: 0,
            },
            spawn_ticket: "t".into(),
        });
        assert_eq!(led.note_live_despawn(200), NoteLiveDespawn::SessionRecycled);
        assert!(!led.pending_despawns.contains(&200));
        assert!(led.freelist.contains(&200));
        assert!(!led.store().contains(200));
    }

    #[test]
    fn facade_despawn_undo_restores_then_gc_is_noop() {
        let op_a = b"operator-a";
        let mut led = LiveEditLedger::default();
        assert!(led.note_spawn_with_parent(1, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.note_spawn_with_parent(10, 1, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        let NoteLiveDespawn::Trashed {
            restore_parent: prev,
        } = led.note_live_despawn(10)
        else {
            panic!("expected trash");
        };
        assert_eq!(prev, Some(1));
        led.note_despawn_undo(op_a, 10, prev.unwrap_or(0));
        assert_eq!(led.undo_len_for(op_a), 1);
        let restored = led.undo_reparent(op_a).expect("undo");
        assert!(restored.is_some());
        assert!(led.contains_entity(10));
        assert_eq!(led.parent_of(10), 1);
        assert!(!led.pending_despawns.contains(&10));
        // Re-trash, record undo, force GC → No-op.
        let NoteLiveDespawn::Trashed {
            restore_parent: prev2,
        } = led.note_live_despawn(10)
        else {
            panic!("expected re-trash");
        };
        led.note_despawn_undo(op_a, 10, prev2.unwrap_or(0));
        led.gc_trash_with_limits(0, 0);
        assert!(!led.store().contains(10));
        assert_eq!(led.undo_reparent(op_a), Ok(None));
        assert_eq!(led.undo_len_for(op_a), 0);
    }

    #[test]
    fn facade_reseed_after_trash_clears_pending_and_trash_index() {
        let mut led = LiveEditLedger::default();
        assert!(led.note_spawn_with_parent(10, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(matches!(
            led.note_live_despawn(10),
            NoteLiveDespawn::Trashed { .. }
        ));
        assert!(led.pending_despawns.contains(&10));
        assert!(led.store().is_trashed(10));
        // Re-seed must not leave a live node that GC / clear_save would delete.
        assert!(led.note_spawn_with_parent(10, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.contains_entity(10));
        assert!(!led.store().is_trashed(10));
        assert!(!led.pending_despawns.contains(&10));
        led.gc_trash_with_limits(0, 0);
        assert!(led.contains_entity(10), "GC must not remove revived id");
    }

    #[test]
    fn facade_reseed_after_gc_clears_stale_pending_despawns() {
        let mut led = LiveEditLedger::default();
        assert!(led.note_spawn_with_parent(10, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(matches!(
            led.note_live_despawn(10),
            NoteLiveDespawn::Trashed { .. }
        ));
        led.gc_trash_with_limits(0, 0);
        assert!(!led.store().contains(10));
        assert!(
            led.pending_despawns.contains(&10),
            "Save projection until clear"
        );
        assert!(led.note_spawn_with_parent(10, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.contains_entity(10));
        assert!(
            !led.pending_despawns.contains(&10),
            "re-seed must drop GC'd pending projection"
        );
    }

    #[test]
    fn note_reparent_undo_skips_trashed_child_after_despawn_race() {
        let op_a = b"operator-a";
        let mut led = LiveEditLedger::default();
        assert!(led.note_spawn_with_parent(1, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.note_spawn_with_parent(10, 1, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(matches!(
            led.note_live_despawn(10),
            NoteLiveDespawn::Trashed { .. }
        ));
        // Barrier Ok after trash must not re-seed the stack.
        led.note_reparent_undo(op_a, 10, 1, 0);
        assert_eq!(led.undo_len_for(op_a), 0);
        assert!(led.undo_stacks.is_empty());
    }

    #[test]
    fn facade_cycle_reject_leaves_parent_unchanged() {
        let mut led = LiveEditLedger::default();
        assert!(led.note_spawn_with_parent(1, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.note_spawn_with_parent(2, 1, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.note_spawn_with_parent(3, 2, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        // 1 → 2 → 3; moving 1 under 3 would cycle.
        let outcome = led.apply_reparent(1, 3);
        assert_eq!(outcome, ApplyOutcome::RejectedCycle);
        assert_eq!(
            edit_reject_for_apply(outcome),
            alchemy_protocol::edit_reject::CYCLE
        );
        assert_eq!(led.parent_of(1), 0);
        assert_eq!(led.parent_of(2), 1);
        assert_eq!(led.parent_of(3), 2);
    }

    #[test]
    fn soft_lease_blocks_other_actor_until_expiry() {
        let mut led = LiveEditLedger::default();
        assert!(led.note_spawn_with_parent(10, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.try_acquire_soft_lease(10, 1).is_ok());
        assert!(
            led.try_acquire_soft_lease(10, 1).is_ok(),
            "same actor refresh"
        );
        assert_eq!(
            led.try_acquire_soft_lease(10, 2),
            Err(SoftLeaseConflict),
            "other actor blocked while lease active"
        );
        for _ in 0..=SOFT_LEASE_TTL_TICKS {
            led.advance_lease_tick();
        }
        assert!(
            led.try_acquire_soft_lease(10, 2).is_ok(),
            "other actor may acquire after expiry"
        );
    }

    #[test]
    fn soft_lease_cleared_on_despawn() {
        let mut led = LiveEditLedger::default();
        assert!(led.note_spawn_with_parent(10, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.try_acquire_soft_lease(10, 1).is_ok());
        assert!(matches!(
            led.note_live_despawn(10),
            NoteLiveDespawn::Trashed { .. }
        ));
        // Entity trashed; lease row cleared — re-seed clears trash/pending first.
        assert!(led.note_spawn_with_parent(10, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(!led.pending_despawns.contains(&10));
        assert!(led.try_acquire_soft_lease(10, 2).is_ok());
    }

    #[test]
    fn soft_lease_allows_does_not_refresh_ttl() {
        let mut led = LiveEditLedger::default();
        assert!(led.note_spawn_with_parent(10, 0, 0, [1.0, 1.0, 1.0, 1.0], String::new()));
        assert!(led.try_acquire_soft_lease(10, 1).is_ok());
        let expires = led.soft_lease_of(10).unwrap().lease_expires_tick;
        led.advance_lease_tick();
        assert!(led.soft_lease_allows(10, 1).is_ok());
        assert_eq!(
            led.soft_lease_of(10).unwrap().lease_expires_tick,
            expires,
            "allows must not extend TTL"
        );
    }

    #[test]
    fn fold_confirmed_op_after_bump_updates_chain_tip() {
        let mut led = LiveEditLedger::default();
        assert_eq!(led.op_chain_hash, [0u8; 32]);
        let tip = led.bump_tip();
        assert_eq!(tip, 1);
        let op = CrdtOp::SpawnNode {
            ts: OpTimestamp {
                lamport: 1,
                actor_id: HOST_ACTOR_ID,
            },
            node: 10,
            parent: None,
        };
        let seq1 = led.fold_confirmed_op(42, op.clone());
        assert_ne!(seq1.op_chain_hash, [0u8; 32]);
        assert_eq!(led.op_chain_hash, seq1.op_chain_hash);
        assert_eq!(seq1.host_epoch, 42);
        assert_eq!(seq1.room_seq, 1);
        assert_eq!(seq1.op, op);

        let tip2 = led.bump_tip();
        assert_eq!(tip2, 2);
        let op2 = CrdtOp::MoveNode {
            ts: OpTimestamp {
                lamport: 2,
                actor_id: HOST_ACTOR_ID,
            },
            node: 10,
            new_parent: Some(1),
        };
        let seq2 = led.fold_confirmed_op(42, op2.clone());
        assert_ne!(seq2.op_chain_hash, seq1.op_chain_hash);
        assert_eq!(seq2.room_seq, 2);
        assert_eq!(seq2.op, op2);
        assert_eq!(led.op_chain_hash, seq2.op_chain_hash);

        // Same inputs → same fold (deterministic).
        let mut led2 = LiveEditLedger::default();
        let _ = led2.bump_tip();
        assert_eq!(
            led2.fold_confirmed_op(42, op).op_chain_hash,
            seq1.op_chain_hash
        );
    }
}
