//! Zenoh publish worker (x0042 / P1-3 remainder · x0095 pose scratch).
//!
//! Guarantees: Frame / Manifest / Presence `put().wait()` run only on `zenoh-publish`;
//! Frame channel payload is wire-agnostic poses (+ removes / gate flags); encode+sign
//! stay inside this module (protobuf today); Frame slot coalesce keeps **latest poses**
//! but **unions `removes`** and **sticky-OR `resume_catchup`** (mid-net removes and
//! gate catch-up are not dropped); gated samples **defer** coalesced removes /
//! sticky catch-up until the next allowed Frame (does not rely on physics alone);
//! empty-hold / catch-up / 30Hz tick match pre-x0042;
//! net-tick pose `Vec`s recycle via [`PoseScratchPool`] (acquire → submit → coalesce/worker
//! release) so the steady path does not `Vec::with_capacity` each net tick.
//! `put` failure is fail-closed via `process::exit(78)` (same severity as former
//! main-loop `?`, without returning to physics).
//! Non-guarantees: rkyv/bytemuck wire; Prometheus; backpressure that stalls physics;
//! draining pending jobs / joining the worker on `stop()` (process exit may drop
//! in-flight puts); Manifest logical (non-bytes) jobs — sealed bytes pass through;
//! zero-alloc encode/AEAD/Zenoh; pooling of `removes` / upsert `Entity` vecs.

use alchemy_protocol::wire_aead::AeadKind;
use alchemy_protocol::wire_crypto::{sign_frame, sign_presence, SigningKey};
use alchemy_protocol::{Entity, Message, PresenceState, RenderFrame};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use zenoh::Wait;

use crate::pose_delta;
use crate::wire_tick;

/// Host pose sample shared across physics → network (not a wire type).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pose {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub qx: f64,
    pub qy: f64,
    pub qz: f64,
    pub qw: f64,
    pub sx: f64,
    pub sy: f64,
    pub sz: f64,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
    /// Wire `Entity.holder` (0 = none, 1..=32 = slot+1 · x00af).
    pub holder: u32,
}

#[derive(Clone, Copy, Debug)]
pub enum ManifestReason {
    Epoch,
    Republish,
    JoinAssist,
}

#[derive(Clone, Debug)]
pub struct ManifestJob {
    pub bytes: Vec<u8>,
    pub revision: u64,
    pub epoch: u64,
    pub reason: ManifestReason,
}

#[derive(Clone, Debug)]
pub struct PresenceJob {
    pub state: PresenceState,
    /// True when this is a late-joiner republish (same revision cadence).
    pub republish: bool,
}

/// Wire-agnostic Frame sample produced by physics.
///
/// Not `Clone`: `poses` may be a pooled scratch (x0095). Cloning would bypass the
/// recycle contract and allocate a duplicate `Vec`.
#[derive(Debug, Default)]
pub struct FrameSample {
    /// Full live pose set (empty for mid-net remove-only flush; not pooled).
    pub poses: Vec<(u32, Pose)>,
    pub removes: Vec<u32>,
    pub resume_catchup: bool,
    pub publish_allowed: bool,
    /// Mid-net reclaim flush (no pose delta this physics step).
    pub removes_only: bool,
    pub host_time_ms: u64,
    /// Physics tick at sample time (`RenderFrame.sim_tick` · x00af).
    pub sim_tick: u64,
    /// Per user_root slot last applied Intent seq (`RenderFrame.acked_intent_seq`).
    pub acked_intent_seq: Vec<u64>,
}

/// In-flight pose scratches: physics fill + hub pending + worker handle_frame.
const POSE_SCRATCH_POOLED: usize = 3;
/// Pre-size for mesh + user_root band + grabbable boxes (grows if scene is larger).
const POSE_SCRATCH_CAPACITY: usize = 64;

/// Recycled pose buffers for physics → network (x0095).
///
/// Guarantees: at most [`POSE_SCRATCH_POOLED`] buffers stay in the free list; acquire never
/// blocks (allocates on miss using the pool's configured capacity). With 3 pooled
/// scratches, the steady path covering fill+pending+worker does not call
/// `Vec::with_capacity`. Non-guarantees: capacity shrink; pooling empty `vec![]`;
/// pathological backlog beyond 3 still allocates then drops on release.
pub struct PoseScratchPool {
    free: Mutex<Vec<Vec<(u32, Pose)>>>,
    /// Capacity used for startup buffers and acquire-on-miss.
    buf_capacity: usize,
}

impl PoseScratchPool {
    pub fn new(capacity: usize) -> Self {
        let mut free = Vec::with_capacity(POSE_SCRATCH_POOLED);
        for _ in 0..POSE_SCRATCH_POOLED {
            free.push(Vec::with_capacity(capacity));
        }
        Self {
            free: Mutex::new(free),
            buf_capacity: capacity,
        }
    }

    pub fn acquire(&self) -> Vec<(u32, Pose)> {
        let mut g = self.free.lock().unwrap_or_else(|p| p.into_inner());
        g.pop()
            .unwrap_or_else(|| Vec::with_capacity(self.buf_capacity))
    }

    pub fn release(&self, mut buf: Vec<(u32, Pose)>) {
        buf.clear();
        // Mid-net `poses: vec![]` must not occupy a pooled slot.
        if buf.capacity() == 0 {
            return;
        }
        let mut g = self.free.lock().unwrap_or_else(|p| p.into_inner());
        if g.len() < POSE_SCRATCH_POOLED {
            g.push(buf);
        }
    }

    #[cfg(test)]
    fn free_len(&self) -> usize {
        self.free.lock().unwrap_or_else(|p| p.into_inner()).len()
    }
}

/// Merge two pending Frame samples (x0042 coalesce contract · x0095 recycle).
///
/// - **poses / host_time / publish_allowed**: latest (`next`) wins, except a
///   `removes_only` next must not wipe a prior net-tick pose set.
/// - **removes**: sorted unique union (mid-net reclaim must survive net-tick coalesce).
/// - **resume_catchup**: sticky OR (gate catch-up must not be cleared by a later tick).
/// - **removes_only**: true only if **both** sides are remove-only (else net path).
/// - **sim_tick** / **acked_intent_seq**: when keeping prior poses (removes-only `next`),
///   always `prev` so both stay paired with the pose sample (mid-net must not advance alone).
/// - Discarded pose buffers are returned to `pool`.
pub fn coalesce_frame(prev: FrameSample, next: FrameSample, pool: &PoseScratchPool) -> FrameSample {
    let removes = union_sorted_ids(prev.removes, next.removes);
    let resume_catchup = prev.resume_catchup || next.resume_catchup;
    let removes_only = prev.removes_only && next.removes_only;
    let (poses, host_time_ms, publish_allowed, sim_tick, acked_intent_seq) =
        if next.removes_only && next.poses.is_empty() {
            // Keep prior net-tick poses when only flushing more removes.
            pool.release(next.poses);
            let _ = next.acked_intent_seq; // drop; poses↔sim_tick↔ack stay prev
            let _ = next.sim_tick;
            (
                prev.poses,
                prev.host_time_ms.max(next.host_time_ms),
                prev.publish_allowed || next.publish_allowed,
                prev.sim_tick,
                prev.acked_intent_seq,
            )
        } else {
            pool.release(prev.poses);
            (
                next.poses,
                next.host_time_ms,
                next.publish_allowed,
                next.sim_tick,
                next.acked_intent_seq,
            )
        };
    FrameSample {
        poses,
        removes,
        resume_catchup,
        publish_allowed,
        removes_only,
        host_time_ms,
        sim_tick,
        acked_intent_seq,
    }
}

/// Merge two id lists into a sorted unique vec (O(n+m) when inputs are sorted).
fn union_sorted_ids(mut a: Vec<u32>, mut b: Vec<u32>) -> Vec<u32> {
    if a.is_empty() {
        b.sort_unstable();
        b.dedup();
        return b;
    }
    if b.is_empty() {
        a.sort_unstable();
        a.dedup();
        return a;
    }
    a.sort_unstable();
    b.sort_unstable();
    let mut out = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => {
                out.push(a[i]);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                out.push(b[j]);
                j += 1;
            }
            std::cmp::Ordering::Equal => {
                out.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    out.extend_from_slice(&a[i..]);
    out.extend_from_slice(&b[j..]);
    out
}

#[derive(Default)]
struct Pending {
    frame: Option<FrameSample>,
    presence: Option<PresenceJob>,
    manifest: Option<ManifestJob>,
}

/// Bounded coalesce slots + wake (`OutboundSlots` in the x0042 slice). Physics never blocks on Zenoh.
pub struct OutboundHub {
    pending: Mutex<Pending>,
    wake: Condvar,
    /// Set by physics on shutdown so the worker can exit.
    stop: AtomicBool,
    pose_pool: PoseScratchPool,
}

impl OutboundHub {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(Pending::default()),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
            pose_pool: PoseScratchPool::new(POSE_SCRATCH_CAPACITY),
        }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.wake.notify_one();
    }

    /// Take a recycled pose scratch (x0095).
    /// Empty when returned from the pool (`release` clears); capacity preserved when pooled.
    pub fn acquire_poses(&self) -> Vec<(u32, Pose)> {
        self.pose_pool.acquire()
    }

    /// Return a pose scratch after encode (or coalesce drop).
    pub fn release_poses(&self, buf: Vec<(u32, Pose)>) {
        self.pose_pool.release(buf);
    }

    pub fn submit_frame(&self, sample: FrameSample) -> bool {
        let mut g = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        let coalesced = g.frame.is_some();
        g.frame = Some(match g.frame.take() {
            Some(prev) => coalesce_frame(prev, sample, &self.pose_pool),
            None => sample,
        });
        self.wake.notify_one();
        coalesced
    }

    /// Test helper: take the coalesced pending frame without running the worker (x0099).
    #[cfg(test)]
    pub fn take_pending_frame_for_test(&self) -> Option<FrameSample> {
        let mut g = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        g.frame.take()
    }

    pub fn submit_presence(&self, job: PresenceJob) -> bool {
        let mut g = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        let coalesced = g.presence.is_some();
        g.presence = Some(match g.presence.take() {
            // Latest state wins; republish log only if *all* coalesced jobs were republish.
            Some(prev) => PresenceJob {
                state: job.state,
                republish: prev.republish && job.republish,
            },
            None => job,
        });
        self.wake.notify_one();
        coalesced
    }

    pub fn submit_manifest(&self, job: ManifestJob) -> bool {
        let mut g = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        let coalesced = g.manifest.is_some();
        g.manifest = Some(job);
        self.wake.notify_one();
        coalesced
    }

    fn take_batch(
        &self,
    ) -> Option<(
        Option<FrameSample>,
        Option<PresenceJob>,
        Option<ManifestJob>,
    )> {
        let mut g = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            if self.stop.load(Ordering::SeqCst) {
                return None;
            }
            let has = g.frame.is_some() || g.presence.is_some() || g.manifest.is_some();
            if has {
                let frame = g.frame.take();
                let presence = g.presence.take();
                let manifest = g.manifest.take();
                return Some((frame, presence, manifest));
            }
            g = self.wake.wait(g).unwrap_or_else(|p| p.into_inner());
        }
    }
}

/// Counters shared with the physics 1s log.
#[derive(Default)]
pub struct PublishStats {
    pub publishes: AtomicU64,
    pub upsert_entities: AtomicU64,
    pub empty_delta_frames: AtomicU64,
    pub coalesce_frames: AtomicU64,
    pub net_tick: AtomicU64,
}

impl PublishStats {
    pub fn snapshot_and_reset(&self) -> (u64, u64, u64, u64, u64) {
        (
            self.publishes.swap(0, Ordering::Relaxed),
            self.upsert_entities.swap(0, Ordering::Relaxed),
            self.empty_delta_frames.swap(0, Ordering::Relaxed),
            self.coalesce_frames.swap(0, Ordering::Relaxed),
            self.net_tick.load(Ordering::Relaxed),
        )
    }
}

pub struct NetworkWorkerHandles {
    pub hub: Arc<OutboundHub>,
    pub stats: Arc<PublishStats>,
}

pub struct NetworkWorkerConfig {
    pub manifest_pub: zenoh::pubsub::Publisher<'static>,
    pub presence_pub: zenoh::pubsub::Publisher<'static>,
    pub frame_pub: zenoh::pubsub::Publisher<'static>,
    pub snapshot_cache: Arc<Mutex<RenderFrame>>,
    pub host_key: SigningKey,
    /// Room id for AEAD AAD (x0069).
    pub room_id: String,
    /// Shared Room AEAD keys (current + prev grace · x007e).
    pub room_aead: host_server::room_aead::SharedRoomAead,
}

fn to_entity(id: u32, pose: Pose) -> Entity {
    let mut e = alchemy_protocol::entity_trs(
        id,
        pose.x,
        pose.y,
        pose.z,
        [pose.qx, pose.qy, pose.qz, pose.qw],
        [pose.sx, pose.sy, pose.sz],
        pose.r,
        pose.g,
        pose.b,
        pose.a,
    );
    e.holder = pose.holder;
    e
}

fn pose_changed(prev: Pose, next: Pose) -> bool {
    pose_delta::translation_changed([prev.x, prev.y, prev.z], [next.x, next.y, next.z])
        || pose_delta::orientation_changed(
            [prev.qx, prev.qy, prev.qz, prev.qw],
            [next.qx, next.qy, next.qz, next.qw],
        )
        || pose_delta::scale_changed([prev.sx, prev.sy, prev.sz], [next.sx, next.sy, next.sz])
        || prev.holder != next.holder
}

/// Wire catch-up after publish gate: removes for ids last sent but no longer live,
/// plus full upserts of current poses. Updates `last_sent` to the live set.
pub fn catchup_after_publish_gate(
    last_sent: &mut HashMap<u32, Pose>,
    all_poses: &[(u32, Pose)],
    remove_ids: &mut Vec<u32>,
) -> Vec<Entity> {
    let live: HashSet<u32> = all_poses.iter().map(|(id, _)| *id).collect();
    let mut synthesized: Vec<u32> = last_sent
        .keys()
        .copied()
        .filter(|id| !live.contains(id) && !remove_ids.contains(id))
        .collect();
    synthesized.sort_unstable();
    remove_ids.extend(synthesized);
    let upserts: Vec<_> = all_poses
        .iter()
        .map(|(id, pose)| to_entity(*id, *pose))
        .collect();
    last_sent.clear();
    for &(id, pose) in all_poses {
        last_sent.insert(id, pose);
    }
    upserts
}

fn put_or_die(label: &str, pub_: &zenoh::pubsub::Publisher<'_>, bytes: Vec<u8>) {
    if let Err(e) = pub_.put(bytes).wait() {
        log::error!("{label} publish failed: {e}");
        std::process::exit(78);
    }
}

fn seal_or_die(
    kind: AeadKind,
    room_id: &str,
    room_aead: &host_server::room_aead::SharedRoomAead,
    plaintext: Vec<u8>,
) -> Vec<u8> {
    match host_server::room_aead::seal_shared(room_aead, room_id, kind, &plaintext) {
        Ok(env) => env,
        Err(e) => {
            log::error!("AEAD seal {kind:?} failed: {e}");
            std::process::exit(78);
        }
    }
}

struct WorkerState {
    last_sent: HashMap<u32, Pose>,
    net_tick: u64,
    last_published_tick: Option<u64>,
    /// Removes / catch-up deferred while `publish_allowed == false` (or mid-net
    /// during an in-flight catch-up). Applied on the next allowed Frame.
    deferred_removes: Vec<u32>,
    deferred_catchup: bool,
}

impl WorkerState {
    fn defer_until_allowed(&mut self, removes: Vec<u32>, resume_catchup: bool) {
        self.deferred_removes =
            union_sorted_ids(std::mem::take(&mut self.deferred_removes), removes);
        self.deferred_catchup |= resume_catchup;
    }

    fn take_deferred(&mut self, removes: &mut Vec<u32>, resume_catchup: &mut bool) {
        if !self.deferred_removes.is_empty() {
            *removes =
                union_sorted_ids(std::mem::take(&mut self.deferred_removes), removes.clone());
        }
        if self.deferred_catchup {
            *resume_catchup = true;
            self.deferred_catchup = false;
        }
    }

    fn handle_manifest(
        &mut self,
        pub_: &zenoh::pubsub::Publisher<'_>,
        room_id: &str,
        room_aead: &host_server::room_aead::SharedRoomAead,
        job: ManifestJob,
    ) {
        let env = seal_or_die(AeadKind::Manifest, room_id, room_aead, job.bytes);
        put_or_die("manifest", pub_, env);
        match job.reason {
            ManifestReason::Epoch => log::info!(
                "published SceneManifest revision={} (credential worker epoch={})",
                job.revision,
                job.epoch
            ),
            ManifestReason::Republish => log::info!(
                "republished SceneManifest revision={} (late-joiner assist)",
                job.revision
            ),
            ManifestReason::JoinAssist => log::info!(
                "republished SceneManifest revision={} (presence join assist)",
                job.revision
            ),
        }
    }

    fn handle_presence(
        &mut self,
        pub_: &zenoh::pubsub::Publisher<'_>,
        key: &SigningKey,
        room_id: &str,
        room_aead: &host_server::room_aead::SharedRoomAead,
        mut job: PresenceJob,
    ) {
        let n = job.state.assignments.len();
        let rev = job.state.revision;
        sign_presence(key, &mut job.state);
        let env = seal_or_die(
            AeadKind::Presence,
            room_id,
            room_aead,
            job.state.encode_to_vec(),
        );
        put_or_die("presence", pub_, env);
        if job.republish {
            log::info!(
                "republished PresenceState revision={rev} assignments={n} (late-joiner assist)"
            );
        } else {
            log::info!("published PresenceState revision={rev} assignments={n}");
        }
    }

    fn refresh_snapshot_cache(
        cache: &Mutex<RenderFrame>,
        tick: u64,
        host_time_ms: u64,
        sim_tick: u64,
        acked_intent_seq: &[u64],
        poses: &[(u32, Pose)],
    ) {
        let mut cache = cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.tick = tick;
        cache.host_time_ms = host_time_ms;
        cache.sim_tick = sim_tick;
        cache.redundancy = 0;
        cache.acked_intent_seq = acked_intent_seq.to_vec();
        cache.upserts = poses
            .iter()
            .map(|(id, pose)| to_entity(*id, *pose))
            .collect();
        cache.removes.clear();
        cache.host_sig.clear();
    }

    #[allow(clippy::too_many_arguments)]
    fn put_signed_frame(
        &mut self,
        frame_pub: &zenoh::pubsub::Publisher<'_>,
        key: &SigningKey,
        room_id: &str,
        room_aead: &host_server::room_aead::SharedRoomAead,
        stats: &PublishStats,
        upserts: Vec<Entity>,
        removes: Vec<u32>,
        host_time_ms: u64,
        sim_tick: u64,
        acked_intent_seq: Vec<u64>,
    ) {
        let upsert_len = upserts.len() as u64;
        let mut frame = RenderFrame {
            tick: self.net_tick,
            host_time_ms,
            upserts,
            removes,
            host_sig: vec![],
            sim_tick,
            redundancy: 0,
            acked_intent_seq,
        };
        sign_frame(key, &mut frame);
        let env = seal_or_die(AeadKind::Frame, room_id, room_aead, frame.encode_to_vec());
        put_or_die("frame", frame_pub, env);
        stats
            .upsert_entities
            .fetch_add(upsert_len, Ordering::Relaxed);
        stats.publishes.fetch_add(1, Ordering::Relaxed);
        self.last_published_tick = Some(self.net_tick);
        self.net_tick = self.net_tick.wrapping_add(1);
        stats.net_tick.store(self.net_tick, Ordering::Relaxed);
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_frame(
        &mut self,
        frame_pub: &zenoh::pubsub::Publisher<'_>,
        key: &SigningKey,
        room_id: &str,
        room_aead: &host_server::room_aead::SharedRoomAead,
        cache: &Mutex<RenderFrame>,
        stats: &PublishStats,
        sample: FrameSample,
        hub: &OutboundHub,
    ) {
        let FrameSample {
            poses,
            mut removes,
            mut resume_catchup,
            publish_allowed,
            removes_only,
            host_time_ms,
            sim_tick,
            acked_intent_seq,
        } = sample;
        // Drop returns the scratch even on early return (x0095).
        let pose_return = PoseScratchReturn { hub, poses };
        let poses = pose_return.as_slice();

        if removes_only {
            if removes.is_empty() {
                return;
            }
            // Gated, or mid-net while catch-up is pending: do not drop removes.
            if !publish_allowed || resume_catchup {
                self.defer_until_allowed(removes, resume_catchup);
                return;
            }
            self.take_deferred(&mut removes, &mut resume_catchup);
            if resume_catchup {
                // Catch-up owns wire repair; keep removes for the net-tick catch-up path.
                self.defer_until_allowed(removes, true);
                return;
            }
            {
                let mut c = cache.lock().unwrap_or_else(|p| p.into_inner());
                c.upserts.retain(|e| !removes.contains(&e.id));
                c.tick = self.net_tick;
                c.host_time_ms = host_time_ms;
                c.sim_tick = sim_tick;
                c.acked_intent_seq = acked_intent_seq.clone();
                c.host_sig.clear();
            }
            let remove_ids = removes.clone();
            self.put_signed_frame(
                frame_pub,
                key,
                room_id,
                room_aead,
                stats,
                vec![],
                remove_ids,
                host_time_ms,
                sim_tick,
                acked_intent_seq,
            );
            for id in &removes {
                self.last_sent.remove(id);
            }
            return;
        }

        // Net-tick path.
        if !publish_allowed {
            // Hold coalesced removes / sticky catch-up until the gate opens again.
            // (Physics also sets was_publishing=false; worker defer is the hard guarantee.)
            self.defer_until_allowed(removes, resume_catchup);
            if let Some(tick) = wire_tick::snapshot_tick_on_empty_hold(self.last_published_tick) {
                Self::refresh_snapshot_cache(
                    cache,
                    tick,
                    host_time_ms,
                    sim_tick,
                    &acked_intent_seq,
                    poses,
                );
            } else {
                // No published tick yet — warm poses only; leave tick untouched.
                let mut c = cache.lock().unwrap_or_else(|p| p.into_inner());
                c.host_time_ms = host_time_ms;
                c.sim_tick = sim_tick;
                c.acked_intent_seq = acked_intent_seq;
                c.upserts = poses
                    .iter()
                    .map(|(id, pose)| to_entity(*id, *pose))
                    .collect();
                c.removes.clear();
                c.host_sig.clear();
            }
            return;
        }

        self.take_deferred(&mut removes, &mut resume_catchup);

        let mut upserts = Vec::new();
        if resume_catchup {
            log::info!("publish resumed; synthesizing catch-up upserts+removes");
            upserts = catchup_after_publish_gate(&mut self.last_sent, poses, &mut removes);
        } else {
            for &(id, next_pose) in poses {
                let should_send = match self.last_sent.get(&id) {
                    None => true,
                    Some(prev) => pose_changed(*prev, next_pose),
                };
                if should_send {
                    upserts.push(to_entity(id, next_pose));
                    self.last_sent.insert(id, next_pose);
                }
            }
        }

        if upserts.is_empty() && removes.is_empty() {
            stats.empty_delta_frames.fetch_add(1, Ordering::Relaxed);
            if let Some(tick) = wire_tick::snapshot_tick_on_empty_hold(self.last_published_tick) {
                Self::refresh_snapshot_cache(
                    cache,
                    tick,
                    host_time_ms,
                    sim_tick,
                    &acked_intent_seq,
                    poses,
                );
            }
            return;
        }

        Self::refresh_snapshot_cache(
            cache,
            self.net_tick,
            host_time_ms,
            sim_tick,
            &acked_intent_seq,
            poses,
        );
        let remove_ids = removes.clone();
        self.put_signed_frame(
            frame_pub,
            key,
            room_id,
            room_aead,
            stats,
            upserts,
            remove_ids,
            host_time_ms,
            sim_tick,
            acked_intent_seq,
        );
        if !resume_catchup {
            for id in &removes {
                self.last_sent.remove(id);
            }
        }
    }
}

/// Returns pose scratch to the pool when `handle_frame` exits (any path).
struct PoseScratchReturn<'a> {
    hub: &'a OutboundHub,
    poses: Vec<(u32, Pose)>,
}

impl PoseScratchReturn<'_> {
    fn as_slice(&self) -> &[(u32, Pose)] {
        &self.poses
    }
}

impl Drop for PoseScratchReturn<'_> {
    fn drop(&mut self) {
        self.hub.release_poses(std::mem::take(&mut self.poses));
    }
}

/// Spawn `zenoh-publish`. Returns hub/stats for the physics loop.
pub fn spawn_network_worker(config: NetworkWorkerConfig) -> Result<NetworkWorkerHandles> {
    let hub = Arc::new(OutboundHub::new());
    let stats = Arc::new(PublishStats::default());

    let hub_w = Arc::clone(&hub);
    let stats_w = Arc::clone(&stats);

    thread::Builder::new()
        .name("zenoh-publish".into())
        .spawn(move || {
            let NetworkWorkerConfig {
                manifest_pub,
                presence_pub,
                frame_pub,
                snapshot_cache,
                host_key,
                room_id,
                room_aead,
            } = config;
            let mut state = WorkerState {
                last_sent: HashMap::new(),
                net_tick: 0,
                last_published_tick: None,
                deferred_removes: Vec::new(),
                deferred_catchup: false,
            };
            log::info!("network worker ready (Zenoh put off physics thread; AEAD on)");
            while let Some((frame, presence, manifest)) = hub_w.take_batch() {
                if let Some(job) = manifest {
                    state.handle_manifest(&manifest_pub, &room_id, &room_aead, job);
                }
                if let Some(job) = presence {
                    state.handle_presence(&presence_pub, &host_key, &room_id, &room_aead, job);
                }
                if let Some(sample) = frame {
                    state.handle_frame(
                        &frame_pub,
                        &host_key,
                        &room_id,
                        &room_aead,
                        &snapshot_cache,
                        &stats_w,
                        sample,
                        &hub_w,
                    );
                }
            }
            log::info!("network worker stopped");
        })
        .context("spawn zenoh-publish")?;

    Ok(NetworkWorkerHandles { hub, stats })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catchup_synthesizes_removes_and_full_upserts() {
        let mut last_sent = HashMap::new();
        last_sent.insert(3, Pose::default());
        last_sent.insert(10, Pose::default());
        let live = vec![
            (
                10,
                Pose {
                    x: 1.0,
                    ..Pose::default()
                },
            ),
            (11, Pose::default()),
        ];
        let mut remove_ids = vec![];
        let upserts = catchup_after_publish_gate(&mut last_sent, &live, &mut remove_ids);
        assert_eq!(remove_ids, vec![3]);
        assert_eq!(upserts.len(), 2);
        assert!(last_sent.contains_key(&10));
        assert!(last_sent.contains_key(&11));
        assert!(!last_sent.contains_key(&3));
        assert!((last_sent[&10].x - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn submit_frame_coalesces_latest_poses() {
        let hub = OutboundHub::new();
        let mut a_poses = hub.acquire_poses();
        a_poses.push((1, Pose::default()));
        let mut b_poses = hub.acquire_poses();
        b_poses.push((
            1,
            Pose {
                x: 9.0,
                ..Pose::default()
            },
        ));
        assert!(!hub.submit_frame(FrameSample {
            poses: a_poses,
            removes: vec![],
            resume_catchup: false,
            publish_allowed: true,
            removes_only: false,
            host_time_ms: 1,
            sim_tick: 0,
            acked_intent_seq: vec![],
        }));
        assert!(hub.submit_frame(FrameSample {
            poses: b_poses,
            removes: vec![],
            resume_catchup: false,
            publish_allowed: true,
            removes_only: false,
            host_time_ms: 2,
            sim_tick: 0,
            acked_intent_seq: vec![],
        }));
        let mut g = hub.pending.lock().unwrap();
        let f = g.frame.take().unwrap();
        assert_eq!(f.host_time_ms, 2);
        assert!((f.poses[0].1.x - 9.0).abs() < f64::EPSILON);
        drop(g);
        hub.release_poses(f.poses);
    }

    fn scratch_one(pool: &PoseScratchPool, id: u32, pose: Pose) -> Vec<(u32, Pose)> {
        let mut poses = pool.acquire();
        poses.push((id, pose));
        poses
    }

    #[test]
    fn coalesce_unions_removes_across_mid_net_and_net_tick() {
        let pool = PoseScratchPool::new(8);
        let mid = FrameSample {
            poses: vec![],
            removes: vec![3, 5],
            resume_catchup: false,
            publish_allowed: true,
            removes_only: true,
            host_time_ms: 1,
            sim_tick: 0,
            acked_intent_seq: vec![],
        };
        let net = FrameSample {
            poses: scratch_one(
                &pool,
                10,
                Pose {
                    qw: 1.0,
                    ..Pose::default()
                },
            ),
            removes: vec![5, 7],
            resume_catchup: false,
            publish_allowed: true,
            removes_only: false,
            host_time_ms: 2,
            sim_tick: 0,
            acked_intent_seq: vec![],
        };
        let m = coalesce_frame(mid, net, &pool);
        assert_eq!(m.removes, vec![3, 5, 7]);
        assert!(!m.removes_only);
        assert_eq!(m.poses.len(), 1);
        assert_eq!(m.host_time_ms, 2);
        pool.release(m.poses);
    }

    #[test]
    fn coalesce_net_then_removes_only_keeps_poses_and_unions_removes() {
        let pool = PoseScratchPool::new(8);
        let net = FrameSample {
            poses: scratch_one(
                &pool,
                10,
                Pose {
                    qw: 1.0,
                    ..Pose::default()
                },
            ),
            removes: vec![1],
            resume_catchup: false,
            publish_allowed: true,
            removes_only: false,
            host_time_ms: 1,
            sim_tick: 0,
            acked_intent_seq: vec![],
        };
        let mid = FrameSample {
            poses: vec![],
            removes: vec![2],
            resume_catchup: false,
            publish_allowed: true,
            removes_only: true,
            host_time_ms: 2,
            sim_tick: 0,
            acked_intent_seq: vec![],
        };
        let m = coalesce_frame(net, mid, &pool);
        assert!(!m.removes_only);
        assert_eq!(m.removes, vec![1, 2]);
        assert_eq!(m.poses.len(), 1);
        assert_eq!(m.poses[0].0, 10);
        assert_eq!(m.host_time_ms, 2);
        pool.release(m.poses);
    }

    #[test]
    fn gated_defer_preserves_removes_and_catchup_for_next_allowed() {
        let mut state = WorkerState {
            last_sent: HashMap::new(),
            net_tick: 0,
            last_published_tick: None,
            deferred_removes: Vec::new(),
            deferred_catchup: false,
        };
        state.defer_until_allowed(vec![3, 5], true);
        let mut removes = vec![7];
        let mut catchup = false;
        state.take_deferred(&mut removes, &mut catchup);
        assert_eq!(removes, vec![3, 5, 7]);
        assert!(catchup);
        assert!(state.deferred_removes.is_empty());
        assert!(!state.deferred_catchup);
    }

    #[test]
    fn presence_coalesce_prefers_first_publish_log_flag() {
        let hub = OutboundHub::new();
        assert!(!hub.submit_presence(PresenceJob {
            state: PresenceState {
                revision: 1,
                ..Default::default()
            },
            republish: false,
        }));
        assert!(hub.submit_presence(PresenceJob {
            state: PresenceState {
                revision: 2,
                ..Default::default()
            },
            republish: true,
        }));
        let mut g = hub.pending.lock().unwrap();
        let p = g.presence.take().unwrap();
        assert_eq!(p.state.revision, 2);
        assert!(!p.republish);
    }

    #[test]
    fn coalesce_two_removes_only_keeps_union() {
        let pool = PoseScratchPool::new(8);
        let a = FrameSample {
            poses: vec![],
            removes: vec![1],
            resume_catchup: false,
            publish_allowed: true,
            removes_only: true,
            host_time_ms: 1,
            sim_tick: 0,
            acked_intent_seq: vec![],
        };
        let b = FrameSample {
            poses: vec![],
            removes: vec![2],
            resume_catchup: false,
            publish_allowed: true,
            removes_only: true,
            host_time_ms: 2,
            sim_tick: 0,
            acked_intent_seq: vec![],
        };
        let m = coalesce_frame(a, b, &pool);
        assert!(m.removes_only);
        assert_eq!(m.removes, vec![1, 2]);
        assert!(m.poses.is_empty());
        pool.release(m.poses);
    }

    #[test]
    fn coalesce_keeps_resume_catchup_sticky() {
        let pool = PoseScratchPool::new(8);
        let catchup = FrameSample {
            poses: scratch_one(
                &pool,
                10,
                Pose {
                    qw: 1.0,
                    ..Pose::default()
                },
            ),
            removes: vec![],
            resume_catchup: true,
            publish_allowed: true,
            removes_only: false,
            host_time_ms: 1,
            sim_tick: 0,
            acked_intent_seq: vec![],
        };
        let later = FrameSample {
            poses: scratch_one(
                &pool,
                10,
                Pose {
                    x: 1.0,
                    qw: 1.0,
                    ..Pose::default()
                },
            ),
            removes: vec![],
            resume_catchup: false,
            publish_allowed: true,
            removes_only: false,
            host_time_ms: 2,
            sim_tick: 0,
            acked_intent_seq: vec![],
        };
        let m = coalesce_frame(catchup, later, &pool);
        assert!(
            m.resume_catchup,
            "gate catch-up must survive later net tick"
        );
        assert!((m.poses[0].1.x - 1.0).abs() < f64::EPSILON);
        pool.release(m.poses);
    }

    #[test]
    fn submit_frame_coalesce_recycles_discarded_pose_scratch() {
        let hub = OutboundHub::new();
        assert_eq!(hub.pose_pool.free_len(), POSE_SCRATCH_POOLED);
        let mut a_poses = hub.acquire_poses();
        assert_eq!(hub.pose_pool.free_len(), POSE_SCRATCH_POOLED - 1);
        let a_cap = a_poses.capacity();
        assert!(a_cap >= POSE_SCRATCH_CAPACITY);
        a_poses.push((1, Pose::default()));
        assert!(!hub.submit_frame(FrameSample {
            poses: a_poses,
            removes: vec![],
            resume_catchup: false,
            publish_allowed: true,
            removes_only: false,
            host_time_ms: 1,
            sim_tick: 0,
            acked_intent_seq: vec![],
        }));
        // One scratch still pending; free list keeps the other startup scratches (POOLED-1).
        assert_eq!(hub.pose_pool.free_len(), POSE_SCRATCH_POOLED - 1);

        let mut b_poses = hub.acquire_poses();
        b_poses.push((
            1,
            Pose {
                x: 9.0,
                ..Pose::default()
            },
        ));
        assert!(hub.submit_frame(FrameSample {
            poses: b_poses,
            removes: vec![],
            resume_catchup: false,
            publish_allowed: true,
            removes_only: false,
            host_time_ms: 2,
            sim_tick: 0,
            acked_intent_seq: vec![],
        }));
        // Coalesce released `a` into free; pending still holds `b` → free_len == POOLED-1.
        assert_eq!(hub.pose_pool.free_len(), POSE_SCRATCH_POOLED - 1);

        let recycled = hub.acquire_poses();
        assert_eq!(
            recycled.capacity(),
            a_cap,
            "coalesce must return the discarded scratch with capacity"
        );
        assert!(recycled.is_empty());
        hub.release_poses(recycled);

        let mut g = hub.pending.lock().unwrap();
        let f = g.frame.take().unwrap();
        assert!((f.poses[0].1.x - 9.0).abs() < f64::EPSILON);
        let pending_cap = f.poses.capacity();
        drop(g);
        hub.release_poses(f.poses);
        assert_eq!(hub.pose_pool.free_len(), POSE_SCRATCH_POOLED);
        let again = hub.acquire_poses();
        assert_eq!(again.capacity(), pending_cap);
        hub.release_poses(again);
    }

    #[test]
    fn pose_scratch_release_preserves_capacity_roundtrip() {
        let pool = PoseScratchPool::new(POSE_SCRATCH_CAPACITY);
        let mut buf = pool.acquire();
        let cap = buf.capacity();
        buf.push((7, Pose::default()));
        pool.release(buf);
        let again = pool.acquire();
        assert!(again.is_empty());
        assert_eq!(again.capacity(), cap);
        pool.release(again);
    }

    #[test]
    fn pose_scratch_ignores_empty_literal_on_release() {
        let pool = PoseScratchPool::new(8);
        assert_eq!(pool.free_len(), POSE_SCRATCH_POOLED);
        pool.release(Vec::new());
        assert_eq!(pool.free_len(), POSE_SCRATCH_POOLED);
    }

    #[test]
    fn pose_scratch_miss_uses_configured_capacity() {
        let pool = PoseScratchPool::new(8);
        let mut held = Vec::new();
        for _ in 0..POSE_SCRATCH_POOLED {
            held.push(pool.acquire());
        }
        assert_eq!(pool.free_len(), 0);
        let miss = pool.acquire();
        assert_eq!(miss.capacity(), 8);
        pool.release(miss);
        for buf in held {
            pool.release(buf);
        }
    }

    #[test]
    fn mid_net_removes_only_does_not_touch_pose_pool() {
        let hub = OutboundHub::new();
        let before = hub.pose_pool.free_len();
        assert!(!hub.submit_frame(FrameSample {
            poses: vec![],
            removes: vec![3],
            resume_catchup: false,
            publish_allowed: true,
            removes_only: true,
            host_time_ms: 1,
            sim_tick: 0,
            acked_intent_seq: vec![],
        }));
        assert_eq!(hub.pose_pool.free_len(), before);
        let mut g = hub.pending.lock().unwrap();
        let f = g.frame.take().unwrap();
        assert!(f.removes_only);
        assert!(f.poses.is_empty());
        assert_eq!(f.poses.capacity(), 0);
        drop(g);
        // Empty literal must still not enter the free list.
        hub.release_poses(f.poses);
        assert_eq!(hub.pose_pool.free_len(), before);
    }

    #[test]
    fn submit_frame_coalesce_preserves_removes_and_catchup() {
        let hub = OutboundHub::new();
        assert!(!hub.submit_frame(FrameSample {
            poses: vec![],
            removes: vec![3],
            resume_catchup: false,
            publish_allowed: true,
            removes_only: true,
            host_time_ms: 1,
            sim_tick: 0,
            acked_intent_seq: vec![],
        }));
        let mut poses = hub.acquire_poses();
        poses.push((
            10,
            Pose {
                qw: 1.0,
                ..Pose::default()
            },
        ));
        assert!(hub.submit_frame(FrameSample {
            poses,
            removes: vec![],
            resume_catchup: true,
            publish_allowed: true,
            removes_only: false,
            host_time_ms: 2,
            sim_tick: 0,
            acked_intent_seq: vec![],
        }));
        let mut g = hub.pending.lock().unwrap();
        let f = g.frame.take().unwrap();
        assert_eq!(f.removes, vec![3]);
        assert!(f.resume_catchup);
        assert!(!f.removes_only);
        drop(g);
        hub.release_poses(f.poses);
    }

    #[test]
    fn pose_changed_detects_translation() {
        // Identity quat — Pose::default leaves qw=0 which forces orientation change.
        let a = Pose {
            qw: 1.0,
            ..Pose::default()
        };
        let b = Pose {
            x: 1.0,
            qw: 1.0,
            ..Pose::default()
        };
        assert!(pose_changed(a, b));
        assert!(!pose_changed(a, a));
    }

    #[test]
    fn pose_changed_detects_holder() {
        let a = Pose {
            qw: 1.0,
            ..Pose::default()
        };
        let b = Pose {
            qw: 1.0,
            holder: alchemy_protocol::entity_holder_from_slot(0),
            ..Pose::default()
        };
        assert!(pose_changed(a, b));
    }

    #[test]
    fn coalesce_removes_only_keeps_prev_acked_even_if_next_nonempty() {
        let pool = PoseScratchPool::new(8);
        let net = FrameSample {
            poses: scratch_one(
                &pool,
                10,
                Pose {
                    qw: 1.0,
                    ..Pose::default()
                },
            ),
            removes: vec![],
            resume_catchup: false,
            publish_allowed: true,
            removes_only: false,
            host_time_ms: 1,
            sim_tick: 40,
            acked_intent_seq: vec![7, 0, 3],
        };
        let mid = FrameSample {
            poses: vec![],
            removes: vec![2],
            resume_catchup: false,
            publish_allowed: true,
            removes_only: true,
            host_time_ms: 2,
            sim_tick: 41,
            // Deliberately mismatched next ack/sim_tick: coalesce must keep prev with poses.
            acked_intent_seq: vec![9, 1, 4],
        };
        let m = coalesce_frame(net, mid, &pool);
        assert_eq!(m.sim_tick, 40, "sim_tick stays with prior pose sample");
        assert_eq!(
            m.acked_intent_seq,
            vec![7, 0, 3],
            "acked stays with prior pose sample"
        );
        assert_eq!(m.poses.len(), 1);
        pool.release(m.poses);
    }

    #[test]
    fn coalesce_net_tick_takes_next_acked_and_sim_tick() {
        let pool = PoseScratchPool::new(8);
        let a = FrameSample {
            poses: scratch_one(&pool, 1, Pose::default()),
            removes: vec![],
            resume_catchup: false,
            publish_allowed: true,
            removes_only: false,
            host_time_ms: 1,
            sim_tick: 10,
            acked_intent_seq: vec![1],
        };
        let b = FrameSample {
            poses: scratch_one(
                &pool,
                1,
                Pose {
                    x: 2.0,
                    ..Pose::default()
                },
            ),
            removes: vec![],
            resume_catchup: false,
            publish_allowed: true,
            removes_only: false,
            host_time_ms: 2,
            sim_tick: 12,
            acked_intent_seq: vec![9],
        };
        let m = coalesce_frame(a, b, &pool);
        assert_eq!(m.sim_tick, 12);
        assert_eq!(m.acked_intent_seq, vec![9]);
        assert!((m.poses[0].1.x - 2.0).abs() < f64::EPSILON);
        pool.release(m.poses);
    }
}
