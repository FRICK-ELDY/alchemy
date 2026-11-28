//! Physics tick owned by [`HostLoop`] (x0099 / P2-6).
//!
//! Guarantees: one physics tick's side effects live here (spawn/despawn → gate/manifest →
//! reclaim/Presence → intent → `step_dt` → mid-net/net submit → 1Hz log). Sleep / lateness
//! stay in `main`.
//! Non-guarantees: Zenoh put; credential HTTP; input gate; PROTOCOL_PIN / spatial UI.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use host_server::intent_rate::IntentRateLimiter;
use host_server::room_token::{
    publish_allowed_at, remove_local_scene_entry, HostPublishSnapshot, RoomScene,
};
use host_server::session::SessionTable;

use alchemy_protocol::SimState;

use crate::inventory_despawn::{self, DespawnInbox};
use crate::inventory_spawn::{self, SpawnInbox};
use crate::network_worker::{
    FrameSample, ManifestJob, ManifestReason, OutboundHub, Pose, PresenceJob, PublishStats,
};
use crate::presence::{lock_or_fatal, PresenceIntents, PRESENCE_SLOT_TTL};
use crate::sim::PhysicsWorld;

/// Shared with `main` sleep / startup log so net_hz and publish cadence stay linked.
pub(crate) const PHYSICS_HZ: u64 = 60;
pub(crate) const NET_HZ: u64 = 30;
/// Publish a network frame every N physics steps (`PHYSICS_HZ / NET_HZ`).
const PHYSICS_PER_NET: u64 = PHYSICS_HZ / NET_HZ;
const _: () = assert!(PHYSICS_HZ.is_multiple_of(NET_HZ));
const MANIFEST_REPUBLISH: Duration = Duration::from_secs(5);
const PRESENCE_REPUBLISH: Duration = Duration::from_secs(5);

/// Physics-facing gate fields (no `manifest_bytes` clone).
#[derive(Clone, Copy, Debug)]
pub(crate) struct PublishGateView {
    pub publish_allowed: bool,
    pub host_generation: u64,
}

pub(crate) fn read_publish_gate(
    snapshot: &RwLock<HostPublishSnapshot>,
    now: Instant,
) -> PublishGateView {
    match snapshot.read() {
        Ok(guard) => PublishGateView {
            // Physics evaluates hard expiry itself so HTTP-bound worker cannot lag the gate.
            publish_allowed: publish_allowed_at(now, guard.hard_expire_at, guard.publish_allowed),
            host_generation: guard.host_generation,
        },
        Err(_) => {
            log::error!("publish snapshot RwLock poisoned; aborting host-server");
            std::process::exit(78);
        }
    }
}

/// One locked read: `(epoch, revision, bytes)` when worker advanced past `current_epoch`.
fn take_manifest_if_newer(
    snapshot: &RwLock<HostPublishSnapshot>,
    current_epoch: u64,
) -> Option<(u64, u64, Vec<u8>)> {
    match snapshot.read() {
        Ok(guard) => {
            if guard.manifest_epoch != current_epoch {
                Some((
                    guard.manifest_epoch,
                    guard.manifest_revision,
                    guard.manifest_bytes.clone(),
                ))
            } else {
                None
            }
        }
        Err(_) => {
            log::error!("publish snapshot RwLock poisoned; aborting host-server");
            std::process::exit(78);
        }
    }
}

fn host_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn pose_with_trs(
    translation: (f32, f32, f32),
    quat_xyzw: [f32; 4],
    scale: [f32; 3],
    rgba: (f32, f32, f32, f32),
) -> Pose {
    let (x, y, z) = translation;
    let (r, g, b, a) = rgba;
    Pose {
        x: f64::from(x),
        y: f64::from(y),
        z: f64::from(z),
        qx: f64::from(quat_xyzw[0]),
        qy: f64::from(quat_xyzw[1]),
        qz: f64::from(quat_xyzw[2]),
        qw: f64::from(quat_xyzw[3]),
        sx: f64::from(scale[0]),
        sy: f64::from(scale[1]),
        sz: f64::from(scale[2]),
        r,
        g,
        b,
        a,
        holder: alchemy_protocol::ENTITY_HOLDER_NONE,
    }
}

fn pose_trs_finite(pose: &Pose) -> bool {
    pose.x.is_finite()
        && pose.y.is_finite()
        && pose.z.is_finite()
        && pose.qx.is_finite()
        && pose.qy.is_finite()
        && pose.qz.is_finite()
        && pose.qw.is_finite()
        && pose.sx.is_finite()
        && pose.sy.is_finite()
        && pose.sz.is_finite()
}

fn append_mesh_poses_from_sim(world: &PhysicsWorld, out: &mut Vec<(u32, Pose)>) {
    for id in world.mesh_entity_ids() {
        let Some((t, q)) = world.mesh_pose_for(id) else {
            continue;
        };
        let pose = pose_with_trs(t, q, world.scale_for(id), world.color_for(id));
        if pose_trs_finite(&pose) {
            out.push((id, pose));
        }
    }
}

fn fill_wire_poses_from_sim(
    world: &PhysicsWorld,
    occupied: &[Option<String>],
    slot_count: usize,
    holders: &mut Vec<u32>,
    out: &mut Vec<(u32, Pose)>,
) {
    out.clear();
    append_mesh_poses_from_sim(world, out);
    for (slot, occupied_sub) in occupied.iter().enumerate().take(slot_count) {
        if occupied_sub.is_some() {
            out.push((
                PhysicsWorld::user_root_entity_id(slot),
                user_root_pose_from_sim(world, slot),
            ));
        }
    }
    // O(slots): grab → box index → wire holder (avoid O(boxes×slots)).
    holders.clear();
    holders.resize(world.boxes.len(), alchemy_protocol::ENTITY_HOLDER_NONE);
    for slot in 0..slot_count {
        if let Some(box_idx) = world.grabbed_box_index(slot) {
            if box_idx < holders.len() {
                holders[box_idx] = alchemy_protocol::entity_holder_from_slot(slot as u8);
            }
        }
    }
    for (i, (&entity_id, &holder)) in world
        .boxes
        .entity_ids
        .iter()
        .zip(holders.iter())
        .enumerate()
    {
        let mut pose = box_pose_from_sim(world, i);
        pose.holder = holder;
        out.push((entity_id, pose));
    }
}

/// Frame `acked_intent_seq` from the same Presence snapshot as loco/grab overlays.
fn fill_acked_intent_seq_from_overlays(
    by_slot: &[crate::presence::PoseOverride],
    occupied: &[Option<String>],
    slot_count: usize,
    out: &mut Vec<u64>,
) {
    out.clear();
    out.resize(slot_count, 0);
    for (slot, occupied_sub) in occupied.iter().enumerate().take(slot_count) {
        if occupied_sub.is_some() {
            out[slot] = by_slot[slot].applied_intent_seq;
        }
    }
}

fn user_root_pose_from_sim(world: &PhysicsWorld, slot: usize) -> Pose {
    let id = PhysicsWorld::user_root_entity_id(slot);
    let (t, q) = world.user_root_pose(slot);
    pose_with_trs(t, q, world.scale_for(id), world.color_for(id))
}

fn box_pose_from_sim(world: &PhysicsWorld, index: usize) -> Pose {
    let id = world.boxes.entity_ids[index];
    let (t, q) = world.box_pose(index);
    pose_with_trs(t, q, world.scale_for(id), world.color_for(id))
}

/// Owns one physics tick's mutable state (x0099).
///
/// Latch / epoch fields stay private; construct via [`HostLoop::new`].
pub(crate) struct HostLoop {
    physics: PhysicsWorld,
    outbound: Arc<OutboundHub>,
    publish_stats: Arc<PublishStats>,
    presence: Arc<Mutex<PresenceIntents>>,
    sessions: Arc<Mutex<SessionTable>>,
    intent_rates: Arc<Mutex<IntentRateLimiter>>,
    publish_snapshot: Arc<RwLock<HostPublishSnapshot>>,
    spawn_inbox: SpawnInbox,
    despawn_inbox: DespawnInbox,
    live_inbox: crate::live_edit::LiveEditInbox,
    live_edit: crate::live_edit::SharedLiveEdit,
    /// Queued this tick; tip/Commit after barrier Ok (None in unit tests).
    live_commit: Option<crate::live_edit::LiveCommitPublisher>,
    live_awaiting_barrier: Vec<crate::live_edit::ReadyLiveEdit>,
    shared_scene: Arc<RwLock<RoomScene>>,
    force_reseal: Arc<AtomicBool>,
    dt: f32,
    physics_steps: u64,
    physics_steps_logged: u64,
    /// Previous tick was allowed to publish (resume catch-up latch).
    was_publishing: bool,
    manifest_bytes: Vec<u8>,
    manifest_revision: u64,
    manifest_epoch: u64,
    last_manifest: Instant,
    last_presence: Instant,
    last_presence_rev: u64,
    last_publish_denied_log: Instant,
    last_log: Instant,
    /// Reused holder fill scratch (x00af · avoid per-tick `Vec` alloc).
    holder_scratch: Vec<u32>,
    /// Reused ack fill scratch (cleared/filled each tick; swapped into Frame / last_wire).
    acked_scratch: Vec<u64>,
    /// Last ack published with a pose sample (mid-net removes must not advance alone).
    last_wire_acked: Vec<u64>,
    /// `sim_tick` paired with [`Self::last_wire_acked`] / last pose sample.
    last_wire_sim_tick: u64,
    /// Room id for `…/sim_state` export (x00b0).
    room_id: String,
    /// Shared with sim_state queryable; refreshed each physics tick.
    sim_state_cache: Arc<Mutex<Option<SimState>>>,
}

/// Wiring inputs for [`HostLoop::new`] (boot-owned arcs / inboxes / initial manifest).
pub(crate) struct HostLoopParts {
    pub physics: PhysicsWorld,
    pub outbound: Arc<OutboundHub>,
    pub publish_stats: Arc<PublishStats>,
    pub presence: Arc<Mutex<PresenceIntents>>,
    pub sessions: Arc<Mutex<SessionTable>>,
    pub intent_rates: Arc<Mutex<IntentRateLimiter>>,
    pub publish_snapshot: Arc<RwLock<HostPublishSnapshot>>,
    pub spawn_inbox: SpawnInbox,
    pub despawn_inbox: DespawnInbox,
    pub live_inbox: crate::live_edit::LiveEditInbox,
    pub live_edit: crate::live_edit::SharedLiveEdit,
    /// Production: `Some(publisher)`. Unit tests may leave `None` (no reject/accept put).
    pub live_commit: Option<crate::live_edit::LiveCommitPublisher>,
    pub shared_scene: Arc<RwLock<RoomScene>>,
    pub force_reseal: Arc<AtomicBool>,
    pub dt: f32,
    pub manifest_bytes: Vec<u8>,
    pub manifest_revision: u64,
    pub manifest_epoch: u64,
    pub last_manifest: Instant,
    pub last_presence: Instant,
    pub last_presence_rev: u64,
    pub last_publish_denied_log: Instant,
    /// Resume physics step counter after planned handover (x00b0); default 0.
    pub resume_sim_tick: u64,
    pub room_id: String,
    pub sim_state_cache: Arc<Mutex<Option<SimState>>>,
}

impl HostLoop {
    pub(crate) fn new(parts: HostLoopParts) -> Self {
        Self {
            physics: parts.physics,
            outbound: parts.outbound,
            publish_stats: parts.publish_stats,
            presence: parts.presence,
            sessions: parts.sessions,
            intent_rates: parts.intent_rates,
            publish_snapshot: parts.publish_snapshot,
            spawn_inbox: parts.spawn_inbox,
            despawn_inbox: parts.despawn_inbox,
            live_inbox: parts.live_inbox,
            live_edit: parts.live_edit,
            live_commit: parts.live_commit,
            live_awaiting_barrier: Vec::new(),
            shared_scene: parts.shared_scene,
            force_reseal: parts.force_reseal,
            dt: parts.dt,
            physics_steps: parts.resume_sim_tick,
            physics_steps_logged: 0,
            was_publishing: true,
            manifest_bytes: parts.manifest_bytes,
            manifest_revision: parts.manifest_revision,
            manifest_epoch: parts.manifest_epoch,
            last_manifest: parts.last_manifest,
            last_presence: parts.last_presence,
            last_presence_rev: parts.last_presence_rev,
            last_publish_denied_log: parts.last_publish_denied_log,
            last_log: Instant::now(),
            holder_scratch: Vec::new(),
            acked_scratch: Vec::new(),
            last_wire_acked: Vec::new(),
            last_wire_sim_tick: parts.resume_sim_tick,
            room_id: parts.room_id,
            sim_state_cache: parts.sim_state_cache,
        }
    }

    /// Run one physics tick. Caller owns sleep / lateness scheduling.
    ///
    /// `now` is the **publish-gate clock**. Pass the pre-sleep sample (legacy contract) so
    /// hard-expiry does not advance by up to one physics period relative to pre-x0099.
    pub(crate) fn tick(&mut self, now: Instant) {
        // SoftLease clock (x00e2); independent of tip / scene_rev.
        match self.live_edit.lock() {
            Ok(mut led) => led.advance_lease_tick(),
            Err(_) => {
                log::error!("LiveEditLedger poisoned on SoftLease tick; aborting host");
                std::process::exit(78);
            }
        }
        // x00b1: restore in-flight bits from empty/fail puts before this tick's absorb.
        {
            let mut q = self
                .publish_stats
                .wire_dirty_restores
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if !q.is_empty() {
                let masks: Vec<_> = q.drain(..).collect();
                drop(q);
                self.physics.restore_wire_dirty_inflight_bits(masks);
            }
        }
        if let Some(ready) = self.spawn_inbox.try_recv() {
            let blake3 = ready.entry.blake3.clone();
            let entry_flags = ready.entry.flags;
            let entry_color = [
                ready.entry.r,
                ready.entry.g,
                ready.entry.b,
                if ready.entry.a == 0.0 {
                    1.0
                } else {
                    ready.entry.a
                },
            ];
            let entry_id = ready.entry.entity_id;
            let entry_blake3 = ready.entry.blake3.clone();
            let pending_entry = ready.entry.clone();
            let spawn_ticket = ready.spawn_ticket.clone();
            let edit_proposal = ready.edit_proposal.clone();
            match inventory_spawn::apply_ready_spawn(
                ready,
                &self.shared_scene,
                &self.force_reseal,
                |entry| self.physics.spawn_mesh_entry(entry),
            ) {
                Ok(out) if out.scene_applied && out.physics_ok => {
                    // Shared spawn inbox (inventory + edit-lane): ledger + pending after physics.
                    let mut led = match self.live_edit.lock() {
                        Ok(g) => g,
                        Err(_) => {
                            log::error!(
                                "LiveEditLedger poisoned after spawn apply Ok; aborting host"
                            );
                            std::process::exit(78);
                        }
                    };
                    if !led.note_spawn(entry_id, entry_flags, entry_color, entry_blake3) {
                        // Fail-closed: physics/scene succeeded but SSoT seed refused
                        // (e.g. entity_id == u32::MAX). Do not tip-bump / accept.
                        // Same shape as the xpbd-refuse path below (despawn + scene
                        // remove + reject); `force_reseal` was already set by
                        // `apply_ready_spawn` and is left as-is.
                        drop(led);
                        let _ = self.physics.despawn_mesh_entity(entry_id);
                        let _ = remove_local_scene_entry(&self.shared_scene, entry_id);
                        self.reject_spawn_apply(entry_id, edit_proposal.as_ref());
                        log::warn!(
                            "spawn rolled back entity_id={} blake3={} (ledger note_spawn failed; reject APPLY if edit)",
                            out.entity_id,
                            &blake3[..8.min(blake3.len())]
                        );
                    } else {
                        led.note_pending_spawn(crate::live_edit::PendingSpawn {
                            entry: pending_entry,
                            spawn_ticket,
                        });
                        // Tip⇔Commit 一体。publisher 欠落は miswire → fail-closed abort.
                        if let Some(proposal) = edit_proposal.as_ref() {
                            let tip = led.bump_tip();
                            self.require_live_commit().publish_accept(proposal, tip, 0);
                        }
                        drop(led);
                        log::info!(
                            "spawn applied entity_id={} blake3={} (xpbd+local scene+reseal; x00b9)",
                            out.entity_id,
                            &blake3[..8.min(blake3.len())]
                        );
                    }
                }
                Ok(out) if out.scene_applied => {
                    let _ = remove_local_scene_entry(&self.shared_scene, entry_id);
                    self.reject_spawn_apply(entry_id, edit_proposal.as_ref());
                    log::warn!(
                        "spawn rolled back entity_id={} blake3={} (xpbd refused; reject APPLY if edit)",
                        out.entity_id,
                        &blake3[..8.min(blake3.len())]
                    );
                }
                Ok(out) => {
                    self.reject_spawn_apply(entry_id, edit_proposal.as_ref());
                    log::warn!(
                        "spawn local scene rejected entity_id={} blake3={} (reject APPLY if edit)",
                        out.entity_id,
                        &blake3[..8.min(blake3.len())]
                    );
                }
                Err(err) => {
                    self.reject_spawn_apply(entry_id, edit_proposal.as_ref());
                    log::warn!("spawn apply failed: {err:#}");
                }
            }
        }

        if let Some(ready) = self.despawn_inbox.try_recv() {
            let entity_id = ready.entity_id;
            let edit_proposal = ready.edit_proposal.clone();
            match inventory_despawn::apply_ready_despawn(
                ready,
                &self.shared_scene,
                &self.force_reseal,
                |id| self.physics.despawn_mesh_entity(id),
            ) {
                Ok(out) if out.scene_applied => {
                    // Local scene already dropped the row — fold into pending even if XPBD miss.
                    let mut led = match self.live_edit.lock() {
                        Ok(g) => g,
                        Err(_) => {
                            log::error!(
                                "LiveEditLedger poisoned after despawn apply Ok; aborting host"
                            );
                            std::process::exit(78);
                        }
                    };
                    // Logical trash + pending_despawns. Despawn Undo
                    // (`note_despawn_undo`) is Intent-era non-guarantee — not
                    // recorded here (no operator / barrier pair yet).
                    let despawn_out = led.note_live_despawn(entity_id);
                    match despawn_out {
                        crate::live_edit::NoteLiveDespawn::Trashed { .. }
                        | crate::live_edit::NoteLiveDespawn::SessionRecycled => {
                            // Tip⇔Commit 一体。publisher 欠落は miswire → fail-closed abort.
                            if let Some(proposal) = edit_proposal.as_ref() {
                                let tip = led.bump_tip();
                                self.require_live_commit().publish_accept(proposal, tip, 0);
                            }
                            drop(led);
                            if out.physics_ok {
                                log::info!(
                                    "despawn applied entity_id={} (xpbd+local scene+reseal; x00b9)",
                                    out.entity_id
                                );
                            } else {
                                log::warn!(
                                    "despawn entity_id={} local scene ok but xpbd miss; pending recorded",
                                    out.entity_id
                                );
                            }
                        }
                        crate::live_edit::NoteLiveDespawn::Failed => {
                            // Tip reject only — does **not** restore scene/physics
                            // rows already dropped (non-guarantee SSoT skew; see
                            // live_edit module docs). Sparse may remain live.
                            drop(led);
                            self.reject_edit_apply(edit_proposal.as_ref());
                            log::error!(
                                "despawn scene ok but live_edit trash failed entity_id={}; reject APPLY (no scene/physics compensate)",
                                out.entity_id
                            );
                        }
                    }
                }
                Ok(out) => {
                    self.reject_edit_apply(edit_proposal.as_ref());
                    log::warn!(
                        "despawn local scene miss entity_id={} (reject APPLY if edit)",
                        out.entity_id
                    );
                }
                Err(err) => {
                    self.reject_edit_apply(edit_proposal.as_ref());
                    log::warn!("despawn apply failed entity_id={entity_id}: {err:#}");
                }
            }
        }

        // x00b3: drain live Ready → FrameBridge. Tip/Commit only after barrier Ok.
        // On queue failure: LIFO-rollback + reject the failed op **and** the rest of
        // this drain (edit thread already applied them to the ledger; continuing
        // would desync SSoT vs FrameBridge on dependent Reparents).
        const LIVE_EDIT_DRAIN_MAX: usize = 8;
        debug_assert!(self.live_awaiting_barrier.is_empty());
        self.live_awaiting_barrier.clear();
        let mut batch = self.live_inbox.try_recv_n(LIVE_EDIT_DRAIN_MAX);
        let mut fail_at: Option<(usize, String, u32)> = None;
        for (i, ready) in batch.iter().enumerate() {
            let queue_ok = match ready {
                crate::live_edit::ReadyLiveEdit::Reparent { op, .. } => self
                    .physics
                    .queue_live_reparent(op.child_entity_id, op.new_parent_entity_id),
                crate::live_edit::ReadyLiveEdit::SetField { op, .. } => {
                    self.physics.queue_live_set_field(
                        op.entity_id,
                        op.field_id,
                        &op.value,
                        &op.expected_value_hash,
                    )
                }
            };
            if let Err(e) = queue_ok {
                // POS Epoch hash miss → STALE (same vocabulary as FLAGS expected_hash).
                let reason = if e == "stale pose" {
                    alchemy_protocol::edit_reject::STALE_PRECONDITION
                } else {
                    alchemy_protocol::edit_reject::APPLY
                };
                fail_at = Some((i, e.to_string(), reason));
                break;
            }
        }
        if let Some((fail_idx, err, reject_reason)) = fail_at {
            let reject_tail: Vec<_> = batch.split_off(fail_idx);
            let reject_n = reject_tail.len();
            let queued_n = batch.len();
            let tip = match self.live_edit.lock() {
                Ok(mut led) => {
                    // Policy: tip unchanged + reject even if some rollbacks fail
                    // (rollback_mirrors already logs; count surfaced here).
                    let mut rb_fail = 0usize;
                    for ready in reject_tail.iter().rev() {
                        if !led.rollback_mirrors(ready) {
                            rb_fail += 1;
                        }
                    }
                    if rb_fail > 0 {
                        log::warn!(
                            "live edit queue-fail rollback: {rb_fail}/{reject_n} compensating apply(s) failed; tip unchanged, reject still published"
                        );
                    }
                    led.tip
                }
                Err(_) => {
                    log::error!("LiveEditLedger poisoned on queue fail; aborting host");
                    std::process::exit(78);
                }
            };
            let pubber = self.require_live_commit();
            // Only the failing head keeps the specific reason (e.g. STALE); the
            // rest of the abort tail were cancelled, not independently stale.
            for (j, ready) in reject_tail.iter().enumerate() {
                let reason = if j == 0 {
                    reject_reason
                } else {
                    alchemy_protocol::edit_reject::APPLY
                };
                pubber.publish_reject(ready.proposal(), reason, tip);
            }
            log::warn!(
                "live edit queue failed ({err}); LIFO rolled back {reject_n} drain op(s) + head reject reason={reject_reason} (tip={tip}; kept {queued_n} queued)"
            );
        }
        self.live_awaiting_barrier = batch;

        // x003d: credentials / seal live on the worker; physics reads gate + hard expiry.
        let pub_snap = read_publish_gate(&self.publish_snapshot, now);
        if !pub_snap.publish_allowed {
            // Keep `last_sent` as last wire view so resume can synthesize removes.
            self.was_publishing = false;
            if self.last_publish_denied_log.elapsed() >= Duration::from_secs(5) {
                log::warn!(
                    "publish paused (credential hard expiry or gate); physics continues, waiting for refresh"
                );
                self.last_publish_denied_log = Instant::now();
            }
        } else if let Some((epoch, rev, bytes)) =
            take_manifest_if_newer(&self.publish_snapshot, self.manifest_epoch)
        {
            self.manifest_bytes = bytes;
            self.manifest_revision = rev;
            self.manifest_epoch = epoch;
            let _ = self.outbound.submit_manifest(ManifestJob {
                bytes: self.manifest_bytes.clone(),
                revision: self.manifest_revision,
                epoch: self.manifest_epoch,
                reason: ManifestReason::Epoch,
            });
            self.last_manifest = Instant::now();
        } else if self.last_manifest.elapsed() >= MANIFEST_REPUBLISH {
            let _ = self.outbound.submit_manifest(ManifestJob {
                bytes: self.manifest_bytes.clone(),
                revision: self.manifest_revision,
                epoch: self.manifest_epoch,
                reason: ManifestReason::Republish,
            });
            self.last_manifest = Instant::now();
        }
        // Stays true across non-net ticks until catch-up runs on a net tick
        // (`was_publishing` is not cleared/set on mid-net).
        let resume_catchup = pub_snap.publish_allowed && !self.was_publishing;

        let tick_now = Instant::now();
        let host_generation = pub_snap.host_generation;
        let (by_slot, occupied, slot_count, presence_rev, presence_state, reclaimed, join_assist) = {
            let mut g = lock_or_fatal(&self.presence, "presence");
            let freed = g.drain_reclaims(tick_now, PRESENCE_SLOT_TTL);
            let join_assist = g.take_manifest_join_assist();
            (
                g.by_slot,
                g.occupied_sub.clone(),
                g.slot_count(),
                g.revision,
                g.to_presence_state(host_generation),
                freed,
                join_assist,
            )
        };
        // Same snapshot as loco/grab apply — fill scratch now; Frame uses this on net ticks.
        fill_acked_intent_seq_from_overlays(
            &by_slot,
            &occupied,
            slot_count,
            &mut self.acked_scratch,
        );

        for (entity_id, sub) in &reclaimed {
            let slot = (entity_id - crate::sim::USER_ROOT_ENTITY_FIRST) as usize;
            self.physics.release_slot(slot);
            // Do not mutate `last_sent` here: it is the last wire view. Removals are
            // applied when removes are published, or wholesale on resume catch-up.
            if !sub.is_empty() {
                let removed = lock_or_fatal(&self.sessions, "sessions").remove_by_sub(sub);
                let mut rates = lock_or_fatal(&self.intent_rates, "intent_rate");
                rates.forget_sub(sub);
                rates.forget_session_pubs(&removed);
            }
        }

        if pub_snap.publish_allowed
            && (presence_rev != self.last_presence_rev
                || self.last_presence.elapsed() >= PRESENCE_REPUBLISH)
        {
            let republish = presence_rev == self.last_presence_rev;
            let _ = self.outbound.submit_presence(PresenceJob {
                state: presence_state,
                republish,
            });
            self.last_presence_rev = presence_rev;
            self.last_presence = Instant::now();
        }

        if pub_snap.publish_allowed && join_assist {
            let _ = self.outbound.submit_manifest(ManifestJob {
                bytes: self.manifest_bytes.clone(),
                revision: self.manifest_revision,
                epoch: self.manifest_epoch,
                reason: ManifestReason::JoinAssist,
            });
            self.last_manifest = Instant::now();
        }

        for (slot, (occupied_sub, &overlay)) in occupied
            .iter()
            .zip(by_slot.iter())
            .take(slot_count)
            .enumerate()
        {
            if occupied_sub.is_none() {
                continue;
            }
            if overlay.active {
                self.physics
                    .apply_user_intent(slot, overlay.x, overlay.y, overlay.z, self.dt);
            }
            self.physics.apply_grab(
                slot,
                overlay.grab,
                overlay.grab_entity_id,
                (overlay.aim_x, overlay.aim_y, overlay.aim_z),
            );
        }
        let barrier_ok = self.physics.step_dt(self.dt).is_ok();
        let awaiting = std::mem::take(&mut self.live_awaiting_barrier);
        if barrier_ok {
            if !awaiting.is_empty() {
                let mut led = match self.live_edit.lock() {
                    Ok(g) => g,
                    Err(_) => {
                        log::error!("LiveEditLedger poisoned after live barrier Ok; aborting host");
                        std::process::exit(78);
                    }
                };
                // Tip⇔Commit 一体。publisher は Commit を出すときだけ必須。
                // Confirmed Reparent also feeds per-operator SlotUndoStack (x00e1).
                let pubber = self.require_live_commit();
                for ready in awaiting {
                    if let crate::live_edit::ReadyLiveEdit::Reparent {
                        proposal,
                        op,
                        prev_parent,
                        ..
                    } = &ready
                    {
                        led.note_reparent_undo(
                            &proposal.session_pub,
                            op.child_entity_id,
                            *prev_parent,
                            op.new_parent_entity_id,
                        );
                    }
                    let tip = led.bump_tip();
                    // SoftLease acquiring actor fixed on Ready (x00f3); not table re-lookup.
                    pubber.publish_accept(ready.proposal(), tip, ready.lease_holder_actor_id());
                }
            }
        } else if !awaiting.is_empty() {
            let awaiting_n = awaiting.len();
            let tip = match self.live_edit.lock() {
                Ok(mut led) => {
                    // LIFO: same-child multi Reparent in one tick must unwind newest first.
                    // Tip stays put and reject is published even if compensating apply fails.
                    let mut rb_fail = 0usize;
                    for ready in awaiting.iter().rev() {
                        if !led.rollback_mirrors(ready) {
                            rb_fail += 1;
                        }
                    }
                    if rb_fail > 0 {
                        log::warn!(
                            "live edit barrier-fail rollback: {rb_fail}/{awaiting_n} compensating apply(s) failed; tip unchanged, reject still published"
                        );
                    }
                    led.tip
                }
                Err(_) => {
                    log::error!("LiveEditLedger poisoned on barrier fail; aborting host");
                    std::process::exit(78);
                }
            };
            let pubber = self.require_live_commit();
            for ready in &awaiting {
                pubber.publish_reject(ready.proposal(), alchemy_protocol::edit_reject::APPLY, tip);
            }
            log::warn!(
                "live edit barrier failed; rolled back {awaiting_n} mirror(s) + reject APPLY (tip={tip})"
            );
        }
        self.physics_steps = self.physics_steps.wrapping_add(1);
        self.physics_steps_logged += 1;

        let remove_ids: Vec<u32> = reclaimed.iter().map(|(id, _)| *id).collect();

        if !self.physics_steps.is_multiple_of(PHYSICS_PER_NET) {
            // Still flush removes promptly when reclaim happens between net ticks.
            // Resume catch-up (with synthesized removes) waits for a net tick below.
            // sim_tick/ack stay pose-paired: last net-tick values (do not advance mid-net).
            if !remove_ids.is_empty()
                && pub_snap.publish_allowed
                && !resume_catchup
                && self.outbound.submit_frame(FrameSample {
                    poses: vec![],
                    removes: remove_ids,
                    resume_catchup: false,
                    publish_allowed: true,
                    removes_only: true,
                    host_time_ms: host_time_ms(),
                    sim_tick: self.last_wire_sim_tick,
                    acked_intent_seq: self.last_wire_acked.clone(),
                    dirty_authored: false,
                    dirty_inflight_mask: None,
                    scene_rev: self.live_edit.lock().map(|g| g.tip).unwrap_or(0),
                })
            {
                self.publish_stats
                    .coalesce_frames
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            return;
        }

        // Pose sample only on net ticks (delta/encode/put live on zenoh-publish).
        // x0095: recycled scratch — worker / coalesce returns the Vec to the pool.
        // x00b1: mvcc + publish_allowed + !catchup → dirty-authored sample (else full scan).
        let mut all_poses = self.outbound.acquire_poses();
        let dirty_authored =
            pub_snap.publish_allowed && !resume_catchup && self.physics.uses_wire_dirty();
        let dirty_inflight_mask = if dirty_authored {
            self.physics
                .fill_wire_dirty_poses(&occupied, slot_count, &mut all_poses)
        } else {
            fill_wire_poses_from_sim(
                &self.physics,
                &occupied,
                slot_count,
                &mut self.holder_scratch,
                &mut all_poses,
            );
            None
        };
        // Swap filled scratch into `last_wire_acked`; clone once for the Frame sample.
        std::mem::swap(&mut self.last_wire_acked, &mut self.acked_scratch);
        self.last_wire_sim_tick = self.physics_steps;
        let acked_intent_seq = self.last_wire_acked.clone();

        let live_tip = self.live_edit.lock().map(|g| g.tip).unwrap_or(0);
        if self.outbound.submit_frame(FrameSample {
            poses: all_poses,
            removes: remove_ids,
            resume_catchup,
            publish_allowed: pub_snap.publish_allowed,
            removes_only: false,
            host_time_ms: host_time_ms(),
            sim_tick: self.physics_steps,
            acked_intent_seq,
            dirty_authored,
            dirty_inflight_mask,
            scene_rev: live_tip,
        }) {
            self.publish_stats
                .coalesce_frames
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        // sim_state cache: every other net tick when publishing (~15Hz); avoid 60Hz rebuild.
        if pub_snap.publish_allowed && (self.physics_steps / PHYSICS_PER_NET).is_multiple_of(2) {
            self.refresh_sim_state_cache(pub_snap.host_generation);
        }
        // Catch-up flag is sticky in `coalesce_frame`; clearing local gate latch here is
        // safe even if this sample merges with a later `resume_catchup: false` tick.
        if resume_catchup && pub_snap.publish_allowed {
            self.was_publishing = true;
        }

        if self.last_log.elapsed() >= Duration::from_secs(1) {
            let (mx, my, mz) = self.physics.moving_translation();
            let mut roots = String::new();
            for (slot, occupied_sub) in occupied.iter().enumerate().take(slot_count) {
                if occupied_sub.is_none() {
                    continue;
                }
                let (ux, uy, uz) = self.physics.user_root_translation(slot);
                let eid = PhysicsWorld::user_root_entity_id(slot);
                let grab = self
                    .physics
                    .grab_entity_id(slot)
                    .map(|id| format!("grab={id}"))
                    .unwrap_or_else(|| "grab=-".into());
                if !roots.is_empty() {
                    roots.push(' ');
                }
                roots.push_str(&format!("root{eid}=({ux:.3},{uy:.3},{uz:.3}) {grab}"));
            }
            if roots.is_empty() {
                roots.push_str("user_roots=-");
            }
            let w = self.publish_stats.snapshot_and_reset();
            log::info!(
                "physics: steps={} coin=({mx:.3},{my:.3},{mz:.3}) {roots}; xpbd barriers={} pubs={} rows={}; delta: frames={} upsert_entities={} skipped_empty={} coalesce={} net_tick={} cell_ok={} cell_fail={} cell_skip={} lod_ok={} lod_fail={} lod_skip={}",
                self.physics_steps_logged,
                self.physics.barrier_ticks,
                self.physics.publishes,
                self.physics.entity_count(),
                w.publishes,
                w.upsert_entities,
                w.empty_delta_frames,
                w.coalesce_frames,
                w.net_tick,
                w.cell_ok,
                w.cell_fail,
                w.cell_skip,
                w.lod_ok,
                w.lod_fail,
                w.lod_skip,
            );
            self.physics_steps_logged = 0;
            self.last_log = Instant::now();
        }
    }

    fn refresh_sim_state_cache(&self, host_generation: u64) {
        let net_tick = self.physics_steps / PHYSICS_PER_NET;
        let state = self.physics.export_sim_state(
            &self.room_id,
            host_generation,
            self.manifest_revision,
            self.physics_steps,
            net_tick,
        );
        match self.sim_state_cache.lock() {
            Ok(mut guard) => *guard = Some(state),
            Err(_) => {
                log::error!("sim_state_cache Mutex poisoned; aborting host-server");
                std::process::exit(78);
            }
        }
    }

    /// Production `main` always wires this; missing = miswire → fail-closed abort.
    fn require_live_commit(&self) -> &crate::live_edit::LiveCommitPublisher {
        match self.live_commit.as_ref() {
            Some(pubber) => pubber,
            None => {
                log::error!("live_commit publisher missing (miswire); aborting host");
                std::process::exit(78);
            }
        }
    }

    /// Spawn apply failed: free reserved id + reject edit Commit if this was `…/edit`.
    fn reject_spawn_apply(
        &self,
        entity_id: u32,
        proposal: Option<&alchemy_protocol::EditProposal>,
    ) {
        let tip = match self.live_edit.lock() {
            Ok(mut led) => {
                led.release_reserved(entity_id);
                led.tip
            }
            Err(_) => {
                log::error!("LiveEditLedger poisoned on spawn reject; aborting host");
                std::process::exit(78);
            }
        };
        if let Some(proposal) = proposal {
            self.require_live_commit().publish_reject(
                proposal,
                alchemy_protocol::edit_reject::APPLY,
                tip,
            );
        }
    }

    /// Despawn apply failed: reject edit Commit if this was `…/edit` (tip unchanged).
    fn reject_edit_apply(&self, proposal: Option<&alchemy_protocol::EditProposal>) {
        let tip = match self.live_edit.lock() {
            Ok(led) => led.tip,
            Err(_) => {
                log::error!("LiveEditLedger poisoned on edit reject; aborting host");
                std::process::exit(78);
            }
        };
        if let Some(proposal) = proposal {
            self.require_live_commit().publish_reject(
                proposal,
                alchemy_protocol::edit_reject::APPLY,
                tip,
            );
        }
    }
}

#[cfg(test)]
impl HostLoop {
    fn presence_for_test(&self) -> &Arc<Mutex<PresenceIntents>> {
        &self.presence
    }

    fn outbound_for_test(&self) -> &Arc<OutboundHub> {
        &self.outbound
    }

    fn publish_snapshot_for_test(&self) -> &Arc<RwLock<HostPublishSnapshot>> {
        &self.publish_snapshot
    }

    fn physics_steps_for_test(&self) -> u64 {
        self.physics_steps
    }

    fn was_publishing_for_test(&self) -> bool {
        self.was_publishing
    }

    fn set_was_publishing_for_test(&mut self, v: bool) {
        self.was_publishing = v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{demo_entries_for_test, LocoMode};
    use alchemy_protocol::DEFAULT_MOVE_SPEED;

    fn test_loop(publish_allowed: bool) -> HostLoop {
        let entries = demo_entries_for_test();
        let physics = PhysicsWorld::from_entries(&entries, LocoMode::Noclip, DEFAULT_MOVE_SPEED);
        let slots = physics.user_root_slot_count();
        let now = Instant::now();
        HostLoop::new(HostLoopParts {
            physics,
            outbound: Arc::new(OutboundHub::new()),
            publish_stats: Arc::new(PublishStats::default()),
            presence: Arc::new(Mutex::new(PresenceIntents::with_slots(slots))),
            sessions: Arc::new(Mutex::new(SessionTable::default())),
            intent_rates: Arc::new(Mutex::new(IntentRateLimiter::new())),
            publish_snapshot: Arc::new(RwLock::new(HostPublishSnapshot {
                publish_allowed,
                hard_expire_at: None,
                host_generation: 1,
                manifest_bytes: vec![1, 2, 3],
                manifest_revision: 1,
                manifest_epoch: 0,
            })),
            spawn_inbox: SpawnInbox::empty_for_test(),
            despawn_inbox: DespawnInbox::empty_for_test(),
            live_inbox: crate::live_edit::LiveEditInbox::empty_for_test(),
            live_edit: crate::live_edit::new_shared_live_edit(),
            live_commit: None,
            shared_scene: Arc::new(RwLock::new(RoomScene {
                revision: 1,
                entries,
                locomotion: None,
                owner_sub: None,
            })),
            force_reseal: Arc::new(AtomicBool::new(false)),
            dt: 1.0 / PHYSICS_HZ as f32,
            manifest_bytes: vec![1, 2, 3],
            manifest_revision: 1,
            manifest_epoch: 0,
            last_manifest: now,
            last_presence: now,
            last_presence_rev: 0,
            last_publish_denied_log: now - Duration::from_secs(60),
            resume_sim_tick: 0,
            room_id: "local".into(),
            sim_state_cache: Arc::new(Mutex::new(None)),
        })
    }

    #[test]
    fn mid_net_reclaim_submits_removes_only() {
        let mut host = test_loop(true);
        let t0 = Instant::now();
        {
            let mut p = host.presence_for_test().lock().unwrap();
            assert_eq!(p.assign_or_lookup("alice", "j1", t0), Some(0));
            let entity = p.reclaim_sub("alice", "leave").expect("reclaim");
            assert_eq!(entity.0, PhysicsWorld::user_root_entity_id(0));
        }
        // First tick after boot is step 1 → mid-net (not multiple of PHYSICS_PER_NET).
        host.tick(t0);
        assert_eq!(host.physics_steps_for_test(), 1);
        let sample = host
            .outbound_for_test()
            .take_pending_frame_for_test()
            .expect("mid-net frame");
        assert!(sample.removes_only);
        assert!(sample.poses.is_empty());
        assert_eq!(sample.removes, vec![PhysicsWorld::user_root_entity_id(0)]);
        assert!(!sample.resume_catchup);
    }

    #[test]
    fn resume_catchup_sets_on_gate_restore() {
        let mut host = test_loop(false);
        host.set_was_publishing_for_test(false);
        let t0 = Instant::now();
        // Mid-net while gated: no frame (publish_allowed false).
        host.tick(t0);
        assert!(host
            .outbound_for_test()
            .take_pending_frame_for_test()
            .is_none());
        assert!(!host.was_publishing_for_test());

        {
            let mut snap = host.publish_snapshot_for_test().write().unwrap();
            snap.publish_allowed = true;
        }
        // Net tick (step 2): resume_catchup must be sticky on the sample, then latch clear.
        host.tick(t0 + Duration::from_millis(16));
        assert_eq!(host.physics_steps_for_test(), 2);
        let sample = host
            .outbound_for_test()
            .take_pending_frame_for_test()
            .expect("net frame");
        assert!(!sample.removes_only);
        assert!(sample.resume_catchup);
        assert!(sample.publish_allowed);
        assert!(host.was_publishing_for_test());
        assert!(!sample.poses.is_empty(), "net-tick must fill poses");
    }

    #[test]
    fn net_tick_acquire_fill_submit() {
        let mut host = test_loop(true);
        let t0 = Instant::now();
        host.tick(t0); // mid-net
        assert!(host
            .outbound_for_test()
            .take_pending_frame_for_test()
            .is_none());
        host.tick(t0 + Duration::from_millis(16)); // net
        let sample = host
            .outbound_for_test()
            .take_pending_frame_for_test()
            .expect("net frame");
        assert!(!sample.removes_only);
        assert!(!sample.resume_catchup);
        assert!(sample.publish_allowed);
        assert!(
            !sample.poses.is_empty(),
            "demo scene must produce mesh/box poses"
        );
    }
}
