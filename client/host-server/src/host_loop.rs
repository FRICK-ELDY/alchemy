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
use host_server::room_token::{publish_allowed_at, HostPublishSnapshot, RoomScene};
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
            match inventory_spawn::apply_ready_spawn(
                ready,
                &self.shared_scene,
                &self.force_reseal,
                |entry| self.physics.spawn_mesh_entry(entry),
            ) {
                Ok(out) if out.scene_applied && out.physics_ok => log::info!(
                    "inventory spawn applied entity_id={} blake3={} (xpbd+scene+reseal)",
                    out.entity_id,
                    &blake3[..8.min(blake3.len())]
                ),
                Ok(out) if out.scene_applied => log::warn!(
                    "inventory spawn scene+reseal only entity_id={} blake3={} (xpbd refused; physics may need host restart)",
                    out.entity_id,
                    &blake3[..8.min(blake3.len())]
                ),
                Ok(out) => log::warn!(
                    "inventory spawn stale revision skipped entity_id={} blake3={}",
                    out.entity_id,
                    &blake3[..8.min(blake3.len())]
                ),
                Err(err) => log::warn!("inventory spawn apply failed: {err:#}"),
            }
        }

        if let Some(ready) = self.despawn_inbox.try_recv() {
            let entity_id = ready.entity_id;
            match inventory_despawn::apply_ready_despawn(
                ready,
                &self.shared_scene,
                &self.force_reseal,
                |id| self.physics.despawn_mesh_entity(id),
            ) {
                Ok(out) if out.scene_applied && out.physics_ok => log::info!(
                    "inventory despawn applied entity_id={} (xpbd+scene+reseal)",
                    out.entity_id
                ),
                Ok(out) if out.scene_applied => log::warn!(
                    "inventory despawn scene+reseal only entity_id={} (xpbd miss; physics may need host restart)",
                    out.entity_id
                ),
                Ok(out) => log::warn!(
                    "inventory despawn stale revision skipped entity_id={}",
                    out.entity_id
                ),
                Err(err) => {
                    log::warn!("inventory despawn apply failed entity_id={entity_id}: {err:#}")
                }
            }
        }

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
        self.physics.step_dt(self.dt);
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
            let (publishes, upsert_entities, empty_delta_frames, coalesce, net_tick) =
                self.publish_stats.snapshot_and_reset();
            log::info!(
                "physics: steps={} coin=({mx:.3},{my:.3},{mz:.3}) {roots}; xpbd barriers={} pubs={} rows={}; delta: frames={publishes} upsert_entities={upsert_entities} skipped_empty={empty_delta_frames} coalesce={coalesce} net_tick={net_tick}",
                self.physics_steps_logged,
                self.physics.barrier_ticks,
                self.physics.publishes,
                self.physics.entity_count()
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
