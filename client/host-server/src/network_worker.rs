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
//! `put` failure on Room `…/frame` is fail-closed via `process::exit(78)` (same
//! severity as former main-loop `?`, without returning to physics).
//! x00c0: after a successful Room frame put, dirty upserts are bucketed by
//! [`alchemy_shared_sim::sim_pages::CellId`] and each non-empty cell is soft-put as a
//! **separately sealed** `…/frame/cell/{cx}_{cy}_{cz}` frame (same `net_tick`).
//! x00c2: `last_wire_cell` tracks the last published delivery cell per entity; on
//! cell-boundary cross or Room `removes` / `removes_only`, the **old cell** also
//! receives a soft-put with cell-scoped `removes` (remove-only cells included).
//! Room still carries full `removes` + `acked_intent_seq` (fail-closed). Cell
//! seal/put failure and LRU eviction are logged + counted and do **not** kill the
//! Room path.
//! x00c7b: after Room success, occupied `user_root` poses from `last_sent` are
//! bucketed by LodCell (`cell_to_lod_cell` / stride=3) and soft-put to
//! `…/frame/lod/{lx}_{ly}_{lz}` at ≤5 Hz (wall ≈200 ms) as an **occupied roster**
//! plus **exit removes** vs `last_lod_ids` (empty LodCell → remove-only). Rate gate
//! (`last_lod_put_at`) and `last_lod_ids` advance only when at least one lod put
//! reaches the wire. Lod seal/put / LRU are isolated from cell stats and never
//! fail-close Room.
//! Non-guarantees: rkyv/bytemuck wire; Prometheus; backpressure that stalls physics;
//! draining pending jobs / joining the worker on `stop()` (process exit may drop
//! in-flight puts); Manifest logical (non-bytes) jobs — sealed bytes pass through;
//! zero-alloc encode/AEAD/Zenoh; pooling of `removes` / upsert `Entity` vecs;
//! zero-clone cell upsert packing (cell path still clones `Entity` for sign/seal);
//! cell / lod put reachability (x00c5 client default is cell mainline); same-tick multi-cell merge;
//! lod snapshot; world mesh / Grab on lod keys.

use alchemy_protocol::frame_cell_key;
use alchemy_protocol::frame_lod_key;
use alchemy_protocol::wire_aead::AeadKind;
use alchemy_protocol::wire_crypto::{sign_frame, sign_presence, SigningKey};
use alchemy_protocol::{
    cell_to_lod_cell, scene_entry_is_user_root, Entity, Message, PresenceState, RenderFrame,
};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use zenoh::Wait;

use alchemy_shared_sim::sim_pages::{CellId, BITMASK_WORDS};

/// Max cached cell-frame publishers (x00c0). Overflow: declare the new cell, then **LRU-evict**.
pub const CELL_PUBLISHER_CACHE_MAX: usize = 64;

/// Max cached lod-frame publishers (x00c7). Isolated from [`CELL_PUBLISHER_CACHE_MAX`].
pub const LOD_PUBLISHER_CACHE_MAX: usize = 64;

/// Minimum wall interval between lod soft-put batches (≤5 Hz · x00c7).
pub const LOD_MIN_INTERVAL: Duration = Duration::from_millis(200);

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
    /// When true, `poses` is the dirty set (x00b1): skip `last_sent` pose_changed filter.
    pub dirty_authored: bool,
    /// Bits detached from acc at pack (x00b1): restore on empty/fail, discard on success.
    pub dirty_inflight_mask: Option<[u64; BITMASK_WORDS]>,
    /// Host live_scene_rev tip (x00b3); stamped onto `RenderFrame.scene_rev`.
    pub scene_rev: u64,
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
/// - **dirty↔dirty poses** (x00b1): entity_id union with last-write-wins (`next` wins);
///   `dirty_inflight_mask` is bitwise OR so empty/fail restore covers both packs.
/// - **dirty→full**: `next` poses win, but prev inflight mask is OR'd onto next so a
///   successful full put discards both; empty/fail restores both.
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
    let keep_prev_poses = next.removes_only && next.poses.is_empty();
    if keep_prev_poses {
        // Keep prior net-tick poses when only flushing more removes.
        pool.release(next.poses);
        let _ = next.acked_intent_seq;
        let _ = next.sim_tick;
        // removes_only next should not carry inflight; OR any stray bits onto prev.
        let dirty_inflight_mask =
            or_dirty_masks(prev.dirty_inflight_mask, next.dirty_inflight_mask);
        return FrameSample {
            poses: prev.poses,
            removes,
            resume_catchup,
            publish_allowed: prev.publish_allowed || next.publish_allowed,
            removes_only,
            host_time_ms: prev.host_time_ms.max(next.host_time_ms),
            sim_tick: prev.sim_tick,
            acked_intent_seq: prev.acked_intent_seq,
            dirty_authored: prev.dirty_authored,
            dirty_inflight_mask,
            scene_rev: prev.scene_rev.max(next.scene_rev),
        };
    }

    if prev.dirty_authored && next.dirty_authored {
        let dirty_inflight_mask =
            or_dirty_masks(prev.dirty_inflight_mask, next.dirty_inflight_mask);
        let poses = union_dirty_poses_lww(prev.poses, next.poses, pool);
        return FrameSample {
            poses,
            removes,
            resume_catchup,
            publish_allowed: next.publish_allowed,
            removes_only,
            host_time_ms: next.host_time_ms,
            sim_tick: next.sim_tick,
            acked_intent_seq: next.acked_intent_seq,
            dirty_authored: true,
            dirty_inflight_mask,
            scene_rev: prev.scene_rev.max(next.scene_rev),
        };
    }

    // Full / catchup wins poses; keep prior dirty inflight so success discards / fail restores.
    pool.release(prev.poses);
    let dirty_inflight_mask = or_dirty_masks(prev.dirty_inflight_mask, next.dirty_inflight_mask);
    FrameSample {
        poses: next.poses,
        removes,
        resume_catchup,
        publish_allowed: next.publish_allowed,
        removes_only,
        host_time_ms: next.host_time_ms,
        sim_tick: next.sim_tick,
        acked_intent_seq: next.acked_intent_seq,
        dirty_authored: next.dirty_authored,
        dirty_inflight_mask,
        scene_rev: prev.scene_rev.max(next.scene_rev),
    }
}

fn or_dirty_masks(
    a: Option<[u64; BITMASK_WORDS]>,
    b: Option<[u64; BITMASK_WORDS]>,
) -> Option<[u64; BITMASK_WORDS]> {
    match (a, b) {
        (None, None) => None,
        (Some(x), None) | (None, Some(x)) => Some(x),
        (Some(mut x), Some(y)) => {
            for (dst, src) in x.iter_mut().zip(y.iter()) {
                *dst |= *src;
            }
            Some(x)
        }
    }
}

/// Last-write-wins by `entity_id` (`next` overwrites `prev`); recycles `next` buffer.
///
/// O(n·m) scan is intentional while dirty cardinality stays scene-small (full-mesh
/// Phase 3). Larger sets belong with Spatial Interest — swap to a scratch map then.
fn union_dirty_poses_lww(
    mut prev: Vec<(u32, Pose)>,
    mut next: Vec<(u32, Pose)>,
    pool: &PoseScratchPool,
) -> Vec<(u32, Pose)> {
    for (id, pose) in next.drain(..) {
        if let Some((_, slot)) = prev.iter_mut().find(|(i, _)| *i == id) {
            *slot = pose;
        } else {
            prev.push((id, pose));
        }
    }
    pool.release(next);
    prev
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
    /// Successful soft-puts to `…/frame/cell/…` (x00c0 multi-cell).
    pub cell_publishes: AtomicU64,
    /// Soft-fail count for cell put (Room path still advanced).
    pub cell_publish_failures: AtomicU64,
    /// LRU evictions when publisher cache is at [`CELL_PUBLISHER_CACHE_MAX`] (x00c0).
    pub cell_publish_skips: AtomicU64,
    /// Successful soft-puts to `…/frame/lod/…` (x00c7).
    pub lod_publishes: AtomicU64,
    /// Soft-fail count for lod put / declare (Room path still advanced).
    pub lod_publish_failures: AtomicU64,
    /// LRU evictions when lod publisher cache is at [`LOD_PUBLISHER_CACHE_MAX`] (x00c7).
    pub lod_publish_skips: AtomicU64,
    /// In-flight dirty masks to OR back on the physics thread after empty/fail put (x00b1).
    pub wire_dirty_restores: Mutex<Vec<[u64; BITMASK_WORDS]>>,
}

/// 1s window snapshot from [`PublishStats::snapshot_and_reset`] (x00c0 multi-cell · x00c7 lod).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PublishStatsWindow {
    pub publishes: u64,
    pub upsert_entities: u64,
    pub empty_delta_frames: u64,
    pub coalesce_frames: u64,
    /// Cumulative net tick (not window-reset).
    pub net_tick: u64,
    pub cell_ok: u64,
    pub cell_fail: u64,
    pub cell_skip: u64,
    pub lod_ok: u64,
    pub lod_fail: u64,
    pub lod_skip: u64,
}

impl PublishStats {
    pub fn snapshot_and_reset(&self) -> PublishStatsWindow {
        PublishStatsWindow {
            publishes: self.publishes.swap(0, Ordering::Relaxed),
            upsert_entities: self.upsert_entities.swap(0, Ordering::Relaxed),
            empty_delta_frames: self.empty_delta_frames.swap(0, Ordering::Relaxed),
            coalesce_frames: self.coalesce_frames.swap(0, Ordering::Relaxed),
            net_tick: self.net_tick.load(Ordering::Relaxed),
            cell_ok: self.cell_publishes.swap(0, Ordering::Relaxed),
            cell_fail: self.cell_publish_failures.swap(0, Ordering::Relaxed),
            cell_skip: self.cell_publish_skips.swap(0, Ordering::Relaxed),
            lod_ok: self.lod_publishes.swap(0, Ordering::Relaxed),
            lod_fail: self.lod_publish_failures.swap(0, Ordering::Relaxed),
            lod_skip: self.lod_publish_skips.swap(0, Ordering::Relaxed),
        }
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
    /// Zenoh session for on-demand cell publishers (x00c0).
    pub session: zenoh::Session,
    /// Room wire_cap for [`frame_cell_key`].
    pub wire_cap: String,
    pub snapshot_cache: Arc<Mutex<RenderFrame>>,
    pub host_key: SigningKey,
    /// Room id for AEAD AAD (x0069).
    pub room_id: String,
    /// Shared Room AEAD keys (current + prev grace · x007e).
    pub room_aead: host_server::room_aead::SharedRoomAead,
}

/// One cached cell publisher + last-use stamp for clock LRU (x00c0).
struct CellPublisherEntry {
    pub_: zenoh::pubsub::Publisher<'static>,
    last_used: u64,
}

/// Pure LRU pick used by [`CellPublisherCache::evict_lru`] (Zenoh-free · x00c0).
fn pick_lru_victim<'a>(entries: impl IntoIterator<Item = (&'a [i32; 3], u64)>) -> Option<[i32; 3]> {
    entries
        .into_iter()
        .min_by_key(|(_, last_used)| *last_used)
        .map(|(cell, _)| *cell)
}

/// Evicts needed after a **successful** declare before insert (`len` → at most `cap - 1`).
fn lru_evicts_before_insert(current_len: usize, cap: usize) -> usize {
    if cap == 0 {
        return current_len;
    }
    current_len.saturating_sub(cap - 1)
}

/// Cached cell-frame publishers with LRU eviction at [`CELL_PUBLISHER_CACHE_MAX`] (x00c0).
///
/// Cap is **concurrent active cell publishers**, not lifetime unique cells. Declare
/// succeeds first; only then is an LRU victim dropped if at capacity (declare failure
/// never shrinks the cache). Touch is O(1) via a monotonic clock stamp.
struct CellPublisherCache {
    session: zenoh::Session,
    room_id: String,
    wire_cap: String,
    pubs: HashMap<[i32; 3], CellPublisherEntry>,
    /// Monotonic stamp; bumped on every successful touch / insert.
    use_clock: u64,
    /// Concurrent publisher cap (production: [`CELL_PUBLISHER_CACHE_MAX`]).
    cap: usize,
}

impl CellPublisherCache {
    fn new(session: zenoh::Session, room_id: String, wire_cap: String) -> Self {
        Self::with_cap(session, room_id, wire_cap, CELL_PUBLISHER_CACHE_MAX)
    }

    fn with_cap(session: zenoh::Session, room_id: String, wire_cap: String, cap: usize) -> Self {
        Self {
            session,
            room_id,
            wire_cap,
            pubs: HashMap::new(),
            use_clock: 0,
            cap: cap.max(1),
        }
    }

    fn touch(&mut self, cell: [i32; 3]) {
        self.use_clock = self.use_clock.wrapping_add(1);
        if let Some(entry) = self.pubs.get_mut(&cell) {
            entry.last_used = self.use_clock;
        }
    }

    /// Drop the least-recently used entry (O(n), n ≤ cap; rare path).
    fn evict_lru(&mut self, stats: &PublishStats) {
        let Some(victim) = pick_lru_victim(self.pubs.iter().map(|(k, e)| (k, e.last_used))) else {
            return;
        };
        self.pubs.remove(&victim);
        stats.cell_publish_skips.fetch_add(1, Ordering::Relaxed);
    }

    /// Ready publisher, or `None` on declare failure (caller counts fail).
    /// At capacity: declare first, then LRU-evict to make room, then insert.
    fn publisher_for(
        &mut self,
        cell: [i32; 3],
        stats: &PublishStats,
    ) -> Option<&zenoh::pubsub::Publisher<'static>> {
        if self.pubs.contains_key(&cell) {
            self.touch(cell);
            return self.pubs.get(&cell).map(|e| &e.pub_);
        }
        let key = frame_cell_key(&self.room_id, &self.wire_cap, cell[0], cell[1], cell[2]);
        let pub_ = match self.session.declare_publisher(key).wait() {
            Ok(p) => p,
            Err(e) => {
                log::warn!(
                    "frame_cell declare_publisher failed for {}_{}_{}: {e}",
                    cell[0],
                    cell[1],
                    cell[2]
                );
                stats.cell_publish_failures.fetch_add(1, Ordering::Relaxed);
                return None;
            }
        };
        for _ in 0..lru_evicts_before_insert(self.pubs.len(), self.cap) {
            self.evict_lru(stats);
        }
        self.use_clock = self.use_clock.wrapping_add(1);
        self.pubs.insert(
            cell,
            CellPublisherEntry {
                pub_,
                last_used: self.use_clock,
            },
        );
        self.pubs.get(&cell).map(|e| &e.pub_)
    }
}

/// Cached lod-frame publishers with LRU at [`LOD_PUBLISHER_CACHE_MAX`] (x00c7).
///
/// Isolated from [`CellPublisherCache`] so cell cap pressure cannot starve mid-band keys.
struct LodPublisherCache {
    session: zenoh::Session,
    room_id: String,
    wire_cap: String,
    pubs: HashMap<[i32; 3], CellPublisherEntry>,
    use_clock: u64,
    cap: usize,
}

impl LodPublisherCache {
    fn new(session: zenoh::Session, room_id: String, wire_cap: String) -> Self {
        Self::with_cap(session, room_id, wire_cap, LOD_PUBLISHER_CACHE_MAX)
    }

    fn with_cap(session: zenoh::Session, room_id: String, wire_cap: String, cap: usize) -> Self {
        Self {
            session,
            room_id,
            wire_cap,
            pubs: HashMap::new(),
            use_clock: 0,
            cap: cap.max(1),
        }
    }

    fn touch(&mut self, cell: [i32; 3]) {
        self.use_clock = self.use_clock.wrapping_add(1);
        if let Some(entry) = self.pubs.get_mut(&cell) {
            entry.last_used = self.use_clock;
        }
    }

    fn evict_lru(&mut self, stats: &PublishStats) {
        let Some(victim) = pick_lru_victim(self.pubs.iter().map(|(k, e)| (k, e.last_used))) else {
            return;
        };
        self.pubs.remove(&victim);
        stats.lod_publish_skips.fetch_add(1, Ordering::Relaxed);
    }

    fn publisher_for(
        &mut self,
        lod_cell: [i32; 3],
        stats: &PublishStats,
    ) -> Option<&zenoh::pubsub::Publisher<'static>> {
        if self.pubs.contains_key(&lod_cell) {
            self.touch(lod_cell);
            return self.pubs.get(&lod_cell).map(|e| &e.pub_);
        }
        let key = frame_lod_key(
            &self.room_id,
            &self.wire_cap,
            lod_cell[0],
            lod_cell[1],
            lod_cell[2],
        );
        let pub_ = match self.session.declare_publisher(key).wait() {
            Ok(p) => p,
            Err(e) => {
                log::warn!(
                    "frame_lod declare_publisher failed for {}_{}_{}: {e}",
                    lod_cell[0],
                    lod_cell[1],
                    lod_cell[2]
                );
                stats.lod_publish_failures.fetch_add(1, Ordering::Relaxed);
                return None;
            }
        };
        for _ in 0..lru_evicts_before_insert(self.pubs.len(), self.cap) {
            self.evict_lru(stats);
        }
        self.use_clock = self.use_clock.wrapping_add(1);
        self.pubs.insert(
            lod_cell,
            CellPublisherEntry {
                pub_,
                last_used: self.use_clock,
            },
        );
        self.pubs.get(&lod_cell).map(|e| &e.pub_)
    }
}

/// Whether a lod soft-put batch may run given the last lod put time (x00c7 · ≤5 Hz).
pub fn lod_rate_gate_open(last_lod_put_at: Option<Instant>, now: Instant) -> bool {
    match last_lod_put_at {
        None => true,
        Some(t) => now.saturating_duration_since(t) >= LOD_MIN_INTERVAL,
    }
}

/// Bucket `user_root` entities by LodCell for mid-band soft-put (x00c7).
///
/// Guarantees: non-user_root ids are dropped; LodCell uses fine [`CellId`] then
/// [`cell_to_lod_cell`]. Non-guarantees: stable map iteration order.
///
/// Contract: lod is an **occupied roster per LodCell** at ≤5 Hz (not a dirty delta).
pub fn bucket_user_roots_by_lod_cell(entities: &[Entity]) -> HashMap<[i32; 3], Vec<Entity>> {
    let mut buckets: HashMap<[i32; 3], Vec<Entity>> = HashMap::new();
    for e in entities {
        if !scene_entry_is_user_root(e.id) {
            continue;
        }
        let fine = CellId::from_world_f64(e.x, e.y, e.z).0;
        let lod = cell_to_lod_cell(fine[0], fine[1], fine[2]);
        buckets.entry(lod).or_default().push(*e);
    }
    buckets
}

/// Collect occupied user_root poses from `last_sent`, excluding Room `removes` (x00c7).
fn collect_lod_user_root_entities(last_sent: &HashMap<u32, Pose>, removes: &[u32]) -> Vec<Entity> {
    let removed: HashSet<u32> = removes.iter().copied().collect();
    let mut out = Vec::new();
    for (&id, pose) in last_sent {
        if removed.contains(&id) || !scene_entry_is_user_root(id) {
            continue;
        }
        out.push(to_entity(id, *pose));
    }
    out
}

/// Plan lod soft-puts: current occupied roster + exit removes vs last published set (x00c7).
///
/// Guarantees: LodCells that become empty emit remove-only puts; ids that left a LodCell
/// appear in that cell's `removes`. Non-guarantees: stable output order.
pub fn plan_lod_soft_puts(
    current_entities: &[Entity],
    last_lod_ids: &HashMap<[i32; 3], HashSet<u32>>,
) -> Vec<([i32; 3], Vec<Entity>, Vec<u32>)> {
    let current_buckets = bucket_user_roots_by_lod_cell(current_entities);
    let mut current_ids: HashMap<[i32; 3], HashSet<u32>> = HashMap::new();
    for (cell, ents) in &current_buckets {
        current_ids.insert(*cell, ents.iter().map(|e| e.id).collect());
    }
    let mut cells: HashSet<[i32; 3]> = current_buckets.keys().copied().collect();
    cells.extend(last_lod_ids.keys().copied());
    let mut out = Vec::with_capacity(cells.len());
    for cell in cells {
        let upserts = current_buckets.get(&cell).cloned().unwrap_or_default();
        let curr = current_ids.get(&cell);
        let mut removes = Vec::new();
        if let Some(prev) = last_lod_ids.get(&cell) {
            for &id in prev {
                if !curr.is_some_and(|c| c.contains(&id)) {
                    removes.push(id);
                }
            }
        }
        removes.sort_unstable();
        if upserts.is_empty() && removes.is_empty() {
            continue;
        }
        out.push((cell, upserts, removes));
    }
    out
}

/// Apply a successful lod soft-put into the last-published id map (x00c7).
///
/// `upsert_ids` is the full occupied roster for this LodCell (empty → cell cleared).
pub fn apply_last_lod_ids(
    last_lod_ids: &mut HashMap<[i32; 3], HashSet<u32>>,
    lod_cell: [i32; 3],
    upsert_ids: &[u32],
) {
    if upsert_ids.is_empty() {
        last_lod_ids.remove(&lod_cell);
        return;
    }
    last_lod_ids.insert(lod_cell, upsert_ids.iter().copied().collect());
}

/// Index dirty upserts by delivery cell (`CellId` / `CELL_SIZE_M=64` center origin · x00c0).
///
/// Guarantees: empty input → empty map; indices refer into the caller's slice (no Entity clone);
/// ids in `exclude` are skipped (Room `removes` win over same-tick upserts · x00c2).
/// Non-guarantees: stable iteration order.
pub fn bucket_upsert_indices_by_cell(
    upserts: &[Entity],
    exclude: &HashSet<u32>,
) -> HashMap<[i32; 3], Vec<usize>> {
    let mut buckets: HashMap<[i32; 3], Vec<usize>> = HashMap::new();
    for (i, e) in upserts.iter().enumerate() {
        if exclude.contains(&e.id) {
            continue;
        }
        let cell = CellId::from_world_f64(e.x, e.y, e.z).0;
        buckets.entry(cell).or_default().push(i);
    }
    buckets
}

/// Bucket cell-scoped exit / despawn removes for Spatial Interest (x00c2).
///
/// Guarantees: first-seen upsert (no prior cell) yields no remove; same-cell upsert
/// yields no remove; `old != new` pushes `id` onto the **old** cell; Room `removes`
/// push onto the last published cell when known; ids in `removed` skip the cross path;
/// each bucket is sorted unique.
/// Non-guarantees: stable map iteration order; intermediate cells on long teleports.
pub fn bucket_cell_exit_removes(
    upserts: &[Entity],
    removes: &[u32],
    last_wire_cell: &HashMap<u32, [i32; 3]>,
    removed: &HashSet<u32>,
) -> HashMap<[i32; 3], Vec<u32>> {
    let mut buckets: HashMap<[i32; 3], Vec<u32>> = HashMap::new();
    for &id in removes {
        if let Some(&old) = last_wire_cell.get(&id) {
            buckets.entry(old).or_default().push(id);
        }
    }
    for e in upserts {
        if removed.contains(&e.id) {
            continue;
        }
        let new_cell = CellId::from_world_f64(e.x, e.y, e.z).0;
        if let Some(&old) = last_wire_cell.get(&e.id) {
            if old != new_cell {
                buckets.entry(old).or_default().push(e.id);
            }
        }
    }
    for ids in buckets.values_mut() {
        ids.sort_unstable();
        ids.dedup();
    }
    buckets
}

/// Upsert-index buckets + exit-remove id buckets for cell soft-puts (x00c2).
type CellFrameBuckets = (HashMap<[i32; 3], Vec<usize>>, HashMap<[i32; 3], Vec<u32>>);

/// Plan per-cell soft-put contents after a successful Room frame (x00c0 / x00c2).
///
/// Guarantees: ids in Room `removes` are **not** cell-upserted; boundary crosses and
/// despawns appear as old-cell removes; remove-only cells are included when the remove
/// bucket is non-empty and the upsert bucket is empty.
/// Returns `(upsert_index_buckets, remove_id_buckets)` into the caller's `upserts` slice.
pub fn plan_cell_frame_buckets(
    upserts: &[Entity],
    removes: &[u32],
    last_wire_cell: &HashMap<u32, [i32; 3]>,
) -> CellFrameBuckets {
    let removed: HashSet<u32> = removes.iter().copied().collect();
    let upsert_buckets = bucket_upsert_indices_by_cell(upserts, &removed);
    let remove_buckets = bucket_cell_exit_removes(upserts, removes, last_wire_cell, &removed);
    (upsert_buckets, remove_buckets)
}

/// Materialize planned cell soft-puts in the same consume order as [`WorkerState::put_signed_frame`]
/// (x00c2): upsert cells first (owning paired removes by move), then leftover remove-only cells.
///
/// Guarantees: every non-empty planned bucket appears exactly once; remove-only cells are
/// preserved when they have no upsert counterpart. Non-guarantees: stable output order.
pub fn materialize_cell_soft_puts(
    upserts: &[Entity],
    upsert_buckets: HashMap<[i32; 3], Vec<usize>>,
    mut remove_buckets: HashMap<[i32; 3], Vec<u32>>,
) -> Vec<([i32; 3], Vec<Entity>, Vec<u32>)> {
    let mut out = Vec::with_capacity(upsert_buckets.len() + remove_buckets.len());
    for (cell, idxs) in upsert_buckets {
        let cell_removes = remove_buckets.remove(&cell).unwrap_or_default();
        let cell_upserts: Vec<Entity> = idxs.iter().map(|&i| upserts[i]).collect();
        if cell_upserts.is_empty() && cell_removes.is_empty() {
            continue;
        }
        out.push((cell, cell_upserts, cell_removes));
    }
    for (cell, cell_removes) in remove_buckets {
        if cell_removes.is_empty() {
            continue;
        }
        out.push((cell, Vec::new(), cell_removes));
    }
    out
}

/// Update `last_wire_cell` after a successful Room frame put (x00c2).
///
/// Order matches client `apply_frame` and host `last_sent` post-put cleanup: upserts
/// first, then removes (**remove wins** when an id is in both — coalesce of net poses
/// with a later `removes_only` can produce that intersection).
fn apply_last_wire_cell(
    last_wire_cell: &mut HashMap<u32, [i32; 3]>,
    upserts: &[Entity],
    removes: &[u32],
) {
    for e in upserts {
        let cell = CellId::from_world_f64(e.x, e.y, e.z).0;
        last_wire_cell.insert(e.id, cell);
    }
    for &id in removes {
        last_wire_cell.remove(&id);
    }
}

/// Test helper: materialize [`bucket_upsert_indices_by_cell`] into owned Entities.
#[cfg(test)]
fn bucket_upserts_by_cell(upserts: &[Entity]) -> HashMap<[i32; 3], Vec<Entity>> {
    let empty = HashSet::new();
    let indices = bucket_upsert_indices_by_cell(upserts, &empty);
    indices
        .into_iter()
        .map(|(cell, idxs)| {
            let ents = idxs.into_iter().map(|i| upserts[i]).collect();
            (cell, ents)
        })
        .collect()
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

/// Soft put for Spatial Interest cell frame (x00be/x00c0). Does not exit the process.
fn put_soft(label: &str, pub_: &zenoh::pubsub::Publisher<'_>, bytes: Vec<u8>) -> bool {
    if let Err(e) = pub_.put(bytes).wait() {
        log::warn!("{label} soft-put failed (Room path kept): {e}");
        return false;
    }
    true
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

/// Soft seal for cell frames after Room put succeeded (x00c0). Never exits the process.
fn seal_soft(
    kind: AeadKind,
    room_id: &str,
    room_aead: &host_server::room_aead::SharedRoomAead,
    plaintext: Vec<u8>,
) -> Option<Vec<u8>> {
    match host_server::room_aead::seal_shared(room_aead, room_id, kind, &plaintext) {
        Ok(env) => Some(env),
        Err(e) => {
            log::warn!("AEAD seal {kind:?} soft-failed (Room path kept): {e}");
            None
        }
    }
}

struct WorkerState {
    last_sent: HashMap<u32, Pose>,
    /// Last successfully published delivery cell per entity (x00c2 cell-exit removes).
    last_wire_cell: HashMap<u32, [i32; 3]>,
    net_tick: u64,
    last_published_tick: Option<u64>,
    /// Wall clock of last **successful** lod soft-put batch (x00c7 ≤5 Hz).
    last_lod_put_at: Option<Instant>,
    /// Last successfully published user_root ids per LodCell (roster / exit removes · x00c7).
    last_lod_ids: HashMap<[i32; 3], HashSet<u32>>,
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

    #[allow(clippy::too_many_arguments)]
    fn soft_put_cell_frame(
        cell_pubs: &mut CellPublisherCache,
        key: &SigningKey,
        room_id: &str,
        room_aead: &host_server::room_aead::SharedRoomAead,
        stats: &PublishStats,
        cell: [i32; 3],
        tick: u64,
        host_time_ms: u64,
        sim_tick: u64,
        scene_rev: u64,
        cell_upserts: Vec<Entity>,
        cell_removes: Vec<u32>,
    ) {
        if cell_upserts.is_empty() && cell_removes.is_empty() {
            return;
        }
        let Some(cell_pub) = cell_pubs.publisher_for(cell, stats) else {
            return;
        };
        let mut cell_frame = RenderFrame {
            tick,
            host_time_ms,
            upserts: cell_upserts,
            removes: cell_removes,
            host_sig: vec![],
            sim_tick,
            redundancy: 0,
            acked_intent_seq: vec![],
            scene_rev,
        };
        sign_frame(key, &mut cell_frame);
        let Some(cell_env) = seal_soft(
            AeadKind::Frame,
            room_id,
            room_aead,
            cell_frame.encode_to_vec(),
        ) else {
            stats.cell_publish_failures.fetch_add(1, Ordering::Relaxed);
            return;
        };
        if put_soft("frame_cell", cell_pub, cell_env) {
            stats.cell_publishes.fetch_add(1, Ordering::Relaxed);
        } else {
            stats.cell_publish_failures.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn soft_put_lod_frame(
        lod_pubs: &mut LodPublisherCache,
        key: &SigningKey,
        room_id: &str,
        room_aead: &host_server::room_aead::SharedRoomAead,
        stats: &PublishStats,
        lod_cell: [i32; 3],
        tick: u64,
        host_time_ms: u64,
        sim_tick: u64,
        scene_rev: u64,
        lod_upserts: Vec<Entity>,
        lod_removes: Vec<u32>,
    ) -> bool {
        if lod_upserts.is_empty() && lod_removes.is_empty() {
            return false;
        }
        let Some(lod_pub) = lod_pubs.publisher_for(lod_cell, stats) else {
            return false;
        };
        let mut lod_frame = RenderFrame {
            tick,
            host_time_ms,
            upserts: lod_upserts,
            removes: lod_removes,
            host_sig: vec![],
            sim_tick,
            redundancy: 0,
            acked_intent_seq: vec![],
            scene_rev,
        };
        sign_frame(key, &mut lod_frame);
        let Some(lod_env) = seal_soft(
            AeadKind::Frame,
            room_id,
            room_aead,
            lod_frame.encode_to_vec(),
        ) else {
            stats.lod_publish_failures.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        if put_soft("frame_lod", lod_pub, lod_env) {
            stats.lod_publishes.fetch_add(1, Ordering::Relaxed);
            true
        } else {
            stats.lod_publish_failures.fetch_add(1, Ordering::Relaxed);
            false
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
        scene_rev: u64,
    ) {
        let mut cache = cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.tick = tick;
        cache.host_time_ms = host_time_ms;
        cache.sim_tick = sim_tick;
        cache.redundancy = 0;
        cache.acked_intent_seq = acked_intent_seq.to_vec();
        cache.scene_rev = scene_rev;
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
        cell_pubs: &mut CellPublisherCache,
        lod_pubs: &mut LodPublisherCache,
        key: &SigningKey,
        room_id: &str,
        room_aead: &host_server::room_aead::SharedRoomAead,
        stats: &PublishStats,
        upserts: Vec<Entity>,
        removes: Vec<u32>,
        host_time_ms: u64,
        sim_tick: u64,
        acked_intent_seq: Vec<u64>,
        scene_rev: u64,
    ) {
        let upsert_len = upserts.len() as u64;
        // Plan cell soft-puts before moving into the Room frame (x00c0 / x00c2).
        let (cell_upsert_buckets, cell_remove_buckets) =
            plan_cell_frame_buckets(&upserts, &removes, &self.last_wire_cell);
        let cell_soft_puts =
            materialize_cell_soft_puts(&upserts, cell_upsert_buckets, cell_remove_buckets);
        // Lod uses last_sent (updated before this call) minus Room removes (x00c7).
        let lod_entities = collect_lod_user_root_entities(&self.last_sent, &removes);
        let tick = self.net_tick;
        let mut frame = RenderFrame {
            tick,
            host_time_ms,
            upserts,
            removes,
            host_sig: vec![],
            sim_tick,
            redundancy: 0,
            acked_intent_seq,
            scene_rev,
        };
        sign_frame(key, &mut frame);
        let env = seal_or_die(AeadKind::Frame, room_id, room_aead, frame.encode_to_vec());
        // Room frame is tick authority: fail-closed before cell soft-puts (x00be/x00c0).
        put_or_die("frame", frame_pub, env);

        // Cell path after Room success (x00c2): same order as `materialize_cell_soft_puts`.
        // `acked_intent_seq` stays on Room only.
        for (cell, cell_upserts, cell_removes) in cell_soft_puts {
            Self::soft_put_cell_frame(
                cell_pubs,
                key,
                room_id,
                room_aead,
                stats,
                cell,
                tick,
                host_time_ms,
                sim_tick,
                scene_rev,
                cell_upserts,
                cell_removes,
            );
        }

        // Mid-band lod after Room success (x00c7): occupied roster ≤5 Hz + exit removes.
        let now = Instant::now();
        if lod_rate_gate_open(self.last_lod_put_at, now) {
            let lod_puts = plan_lod_soft_puts(&lod_entities, &self.last_lod_ids);
            if !lod_puts.is_empty() {
                let mut any_ok = false;
                for (lod_cell, lod_upserts, lod_removes) in lod_puts {
                    let upsert_ids: Vec<u32> = lod_upserts.iter().map(|e| e.id).collect();
                    if Self::soft_put_lod_frame(
                        lod_pubs,
                        key,
                        room_id,
                        room_aead,
                        stats,
                        lod_cell,
                        tick,
                        host_time_ms,
                        sim_tick,
                        scene_rev,
                        lod_upserts,
                        lod_removes,
                    ) {
                        apply_last_lod_ids(&mut self.last_lod_ids, lod_cell, &upsert_ids);
                        any_ok = true;
                    }
                }
                // Only advance the rate gate when at least one lod put reached the wire.
                if any_ok {
                    self.last_lod_put_at = Some(now);
                }
            }
        }

        apply_last_wire_cell(&mut self.last_wire_cell, &frame.upserts, &frame.removes);

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
        cell_pubs: &mut CellPublisherCache,
        lod_pubs: &mut LodPublisherCache,
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
            dirty_authored,
            dirty_inflight_mask,
            scene_rev,
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
                c.scene_rev = scene_rev;
                c.host_sig.clear();
            }
            let remove_ids = removes.clone();
            self.put_signed_frame(
                frame_pub,
                cell_pubs,
                lod_pubs,
                key,
                room_id,
                room_aead,
                stats,
                vec![],
                remove_ids,
                host_time_ms,
                sim_tick,
                acked_intent_seq,
                scene_rev,
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
            queue_dirty_inflight_restore(stats, dirty_inflight_mask);
            if let Some(tick) = wire_tick::snapshot_tick_on_empty_hold(self.last_published_tick) {
                Self::refresh_snapshot_cache(
                    cache,
                    tick,
                    host_time_ms,
                    sim_tick,
                    &acked_intent_seq,
                    poses,
                    scene_rev,
                );
            } else {
                // No published tick yet — warm poses only; leave tick untouched.
                let mut c = cache.lock().unwrap_or_else(|p| p.into_inner());
                c.host_time_ms = host_time_ms;
                c.sim_tick = sim_tick;
                c.acked_intent_seq = acked_intent_seq;
                c.scene_rev = scene_rev;
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
        } else if dirty_authored {
            // x00b1: dirty mask already selected the set — do not re-filter vs last_sent.
            for &(id, next_pose) in poses {
                upserts.push(to_entity(id, next_pose));
                self.last_sent.insert(id, next_pose);
            }
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
            // Bits were detached at pack — restore so a later net tick can send them.
            queue_dirty_inflight_restore(stats, dirty_inflight_mask);
            if let Some(tick) = wire_tick::snapshot_tick_on_empty_hold(self.last_published_tick) {
                Self::refresh_snapshot_cache(
                    cache,
                    tick,
                    host_time_ms,
                    sim_tick,
                    &acked_intent_seq,
                    poses,
                    scene_rev,
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
            scene_rev,
        );
        let remove_ids = removes.clone();
        self.put_signed_frame(
            frame_pub,
            cell_pubs,
            lod_pubs,
            key,
            room_id,
            room_aead,
            stats,
            upserts,
            remove_ids,
            host_time_ms,
            sim_tick,
            acked_intent_seq,
            scene_rev,
        );
        // Success: drop inflight mask without restore — bits stay detached from acc.
        // Re-dirties absorbed after pack remain in acc (detach only removed the packed set).
        let _ = (dirty_authored, dirty_inflight_mask);
        if !resume_catchup {
            for id in &removes {
                self.last_sent.remove(id);
            }
        }
    }
}

fn queue_dirty_inflight_restore(stats: &PublishStats, mask: Option<[u64; BITMASK_WORDS]>) {
    let Some(mask) = mask else {
        return;
    };
    let mut q = stats
        .wire_dirty_restores
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    q.push(mask);
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
                session,
                wire_cap,
                snapshot_cache,
                host_key,
                room_id,
                room_aead,
            } = config;
            let mut cell_pubs =
                CellPublisherCache::new(session.clone(), room_id.clone(), wire_cap.clone());
            let mut lod_pubs = LodPublisherCache::new(session, room_id.clone(), wire_cap);
            let mut state = WorkerState {
                last_sent: HashMap::new(),
                last_wire_cell: HashMap::new(),
                net_tick: 0,
                last_published_tick: None,
                last_lod_put_at: None,
                last_lod_ids: HashMap::new(),
                deferred_removes: Vec::new(),
                deferred_catchup: false,
            };
            log::info!(
                "network worker ready (Zenoh put off physics thread; AEAD on; cell dirty filter x00c0; cell exit remove x00c2; lod roster+exit soft-put x00c7)"
            );
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
                        &mut cell_pubs,
                        &mut lod_pubs,
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

    /// x00c0: 1s window swaps cell soft-put / skip counters with Room delta stats.
    #[test]
    fn snapshot_and_reset_includes_cell_window() {
        let stats = PublishStats::default();
        stats.publishes.store(3, Ordering::Relaxed);
        stats.cell_publishes.store(2, Ordering::Relaxed);
        stats.cell_publish_failures.store(1, Ordering::Relaxed);
        stats.cell_publish_skips.store(4, Ordering::Relaxed);
        stats.net_tick.store(9, Ordering::Relaxed);
        let w = stats.snapshot_and_reset();
        assert_eq!(
            w,
            PublishStatsWindow {
                publishes: 3,
                upsert_entities: 0,
                empty_delta_frames: 0,
                coalesce_frames: 0,
                net_tick: 9,
                cell_ok: 2,
                cell_fail: 1,
                cell_skip: 4,
                lod_ok: 0,
                lod_fail: 0,
                lod_skip: 0,
            }
        );
        assert_eq!(stats.publishes.load(Ordering::Relaxed), 0);
        assert_eq!(stats.cell_publishes.load(Ordering::Relaxed), 0);
        assert_eq!(stats.cell_publish_failures.load(Ordering::Relaxed), 0);
        assert_eq!(stats.cell_publish_skips.load(Ordering::Relaxed), 0);
        assert_eq!(stats.net_tick.load(Ordering::Relaxed), 9);
    }

    #[test]
    fn snapshot_and_reset_includes_lod_window() {
        let stats = PublishStats::default();
        stats.lod_publishes.store(5, Ordering::Relaxed);
        stats.lod_publish_failures.store(2, Ordering::Relaxed);
        stats.lod_publish_skips.store(1, Ordering::Relaxed);
        let w = stats.snapshot_and_reset();
        assert_eq!(w.lod_ok, 5);
        assert_eq!(w.lod_fail, 2);
        assert_eq!(w.lod_skip, 1);
        assert_eq!(stats.lod_publishes.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn lod_rate_gate_respects_min_interval() {
        let t0 = Instant::now();
        assert!(lod_rate_gate_open(None, t0));
        assert!(!lod_rate_gate_open(Some(t0), t0));
        assert!(!lod_rate_gate_open(
            Some(t0),
            t0 + LOD_MIN_INTERVAL - Duration::from_millis(1)
        ));
        assert!(lod_rate_gate_open(Some(t0), t0 + LOD_MIN_INTERVAL));
    }

    #[test]
    fn plan_lod_soft_puts_emits_exit_removes_and_empty_cells() {
        use alchemy_protocol::USER_ROOT_ENTITY_FIRST;
        let a = alchemy_protocol::entity_trs(
            USER_ROOT_ENTITY_FIRST,
            0.0,
            0.0,
            0.0,
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            1.0,
            0.0,
            0.0,
            1.0,
        );
        let mut last = HashMap::new();
        last.insert([0, 0, 0], HashSet::from([USER_ROOT_ENTITY_FIRST]));
        last.insert([1, 0, 0], HashSet::from([USER_ROOT_ENTITY_FIRST + 1]));
        // Only A remains at origin; B despawned → LodCell [1,0,0] remove-only.
        let puts = plan_lod_soft_puts(&[a], &last);
        let origin = puts.iter().find(|(c, _, _)| *c == [0, 0, 0]).unwrap();
        assert_eq!(origin.1.len(), 1);
        assert!(origin.2.is_empty());
        let far = puts.iter().find(|(c, _, _)| *c == [1, 0, 0]).unwrap();
        assert!(far.1.is_empty());
        assert_eq!(far.2, vec![USER_ROOT_ENTITY_FIRST + 1]);
    }

    #[test]
    fn bucket_user_roots_by_lod_cell_filters_and_strides() {
        use alchemy_protocol::USER_ROOT_ENTITY_FIRST;
        let root = alchemy_protocol::entity_trs(
            USER_ROOT_ENTITY_FIRST,
            0.0,
            0.0,
            0.0,
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            1.0,
            0.0,
            0.0,
            1.0,
        );
        // Fine cell [3,0,0] → LodCell [1,0,0] (stride 3); world x = 192 → cell 3.
        let root_far = alchemy_protocol::entity_trs(
            USER_ROOT_ENTITY_FIRST + 1,
            192.0,
            0.0,
            0.0,
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            1.0,
            0.0,
            0.0,
            1.0,
        );
        let world = alchemy_protocol::entity_trs(
            42,
            0.0,
            0.0,
            0.0,
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            1.0,
            0.0,
            0.0,
            1.0,
        );
        let buckets = bucket_user_roots_by_lod_cell(&[root, root_far, world]);
        assert_eq!(buckets.get(&[0, 0, 0]).map(|v| v.len()), Some(1));
        assert_eq!(buckets[&[0, 0, 0]][0].id, USER_ROOT_ENTITY_FIRST);
        assert_eq!(buckets.get(&[1, 0, 0]).map(|v| v.len()), Some(1));
        assert_eq!(buckets[&[1, 0, 0]][0].id, USER_ROOT_ENTITY_FIRST + 1);
        assert_eq!(buckets.values().map(|v| v.len()).sum::<usize>(), 2);
    }

    #[test]
    fn pick_lru_victim_selects_oldest_stamp() {
        let stamps = [([0, 0, 0], 1u64), ([1, 0, 0], 5u64), ([2, 0, 0], 3u64)];
        assert_eq!(
            pick_lru_victim(stamps.iter().map(|(c, t)| (c, *t))),
            Some([0, 0, 0])
        );
        assert_eq!(pick_lru_victim(std::iter::empty()), None);
    }

    #[test]
    fn lru_evicts_before_insert_matches_declare_then_evict() {
        assert_eq!(lru_evicts_before_insert(0, 64), 0);
        assert_eq!(lru_evicts_before_insert(63, 64), 0);
        assert_eq!(lru_evicts_before_insert(64, 64), 1);
        assert_eq!(lru_evicts_before_insert(65, 64), 2);
        // Failed declare never reaches this helper (no shrink).
    }

    /// Exercises [`CellPublisherCache::evict_lru`] via declare-then-evict with cap=2.
    #[test]
    fn cell_publisher_cache_declare_then_evicts_lru() {
        use alchemy_protocol::LOCAL_WIRE_CAP;
        use zenoh::Config;
        use zenoh::Wait;

        let mut config = Config::default();
        config.insert_json5("mode", r#""peer""#).expect("mode");
        config
            .insert_json5("listen/endpoints", r#"["tcp/127.0.0.1:0"]"#)
            .expect("listen");
        config
            .insert_json5("scouting/multicast/enabled", "false")
            .expect("scouting");
        let session = zenoh::open(config).wait().expect("zenoh open");
        let stats = PublishStats::default();
        let mut cache =
            CellPublisherCache::with_cap(session, "local".into(), LOCAL_WIRE_CAP.into(), 2);

        assert!(cache.publisher_for([0, 0, 0], &stats).is_some());
        assert!(cache.publisher_for([1, 0, 0], &stats).is_some());
        assert_eq!(cache.pubs.len(), 2);
        assert_eq!(stats.cell_publish_skips.load(Ordering::Relaxed), 0);

        // Touch [1,0,0] so [0,0,0] is LRU; third declare must evict [0,0,0] after success.
        cache.touch([1, 0, 0]);
        assert!(cache.publisher_for([2, 0, 0], &stats).is_some());
        assert_eq!(cache.pubs.len(), 2);
        assert!(!cache.pubs.contains_key(&[0, 0, 0]));
        assert!(cache.pubs.contains_key(&[1, 0, 0]));
        assert!(cache.pubs.contains_key(&[2, 0, 0]));
        assert_eq!(stats.cell_publish_skips.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn bucket_upsert_indices_match_entity_buckets() {
        let a = alchemy_protocol::entity_trs(
            1,
            0.0,
            0.0,
            0.0,
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            1.0,
            1.0,
            1.0,
            1.0,
        );
        let b = alchemy_protocol::entity_trs(
            2,
            64.0,
            0.0,
            0.0,
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            1.0,
            1.0,
            1.0,
            1.0,
        );
        let upserts = [a, b];
        let empty = HashSet::new();
        let idxs = bucket_upsert_indices_by_cell(&upserts, &empty);
        assert_eq!(idxs.get(&[0, 0, 0]).map(Vec::as_slice), Some(&[0][..]));
        assert_eq!(idxs.get(&[1, 0, 0]).map(Vec::as_slice), Some(&[1][..]));
        assert!(bucket_upsert_indices_by_cell(&[], &empty).is_empty());
    }

    /// x00c0: center-origin cell boundaries match shared-sim `CellId`.
    #[test]
    fn bucket_upserts_by_cell_respects_boundaries() {
        let origin = alchemy_protocol::entity_trs(
            1,
            31.9,
            0.0,
            0.0,
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            1.0,
            1.0,
            1.0,
            1.0,
        );
        let next = alchemy_protocol::entity_trs(
            2,
            32.0,
            0.0,
            0.0,
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            1.0,
            1.0,
            1.0,
            1.0,
        );
        let neg = alchemy_protocol::entity_trs(
            3,
            -32.1,
            0.0,
            0.0,
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            1.0,
            1.0,
            1.0,
            1.0,
        );
        let buckets = bucket_upserts_by_cell(&[origin, next, neg]);
        assert_eq!(buckets.len(), 3);
        assert_eq!(buckets.get(&[0, 0, 0]).map(|v| v.len()), Some(1));
        assert_eq!(buckets.get(&[0, 0, 0]).map(|v| v[0].id), Some(1));
        assert_eq!(buckets.get(&[1, 0, 0]).map(|v| v[0].id), Some(2));
        assert_eq!(buckets.get(&[-1, 0, 0]).map(|v| v[0].id), Some(3));
        assert!(bucket_upserts_by_cell(&[]).is_empty());
    }

    /// x00c0: Room path keeps the full upsert list; bucketing is a separate view.
    #[test]
    fn bucket_upserts_by_cell_does_not_drop_room_entities() {
        let a = alchemy_protocol::entity_trs(
            10,
            0.0,
            0.0,
            0.0,
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            1.0,
            1.0,
            1.0,
            1.0,
        );
        let b = alchemy_protocol::entity_trs(
            11,
            100.0,
            0.0,
            0.0,
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            1.0,
            1.0,
            1.0,
            1.0,
        );
        let room = vec![a, b];
        let buckets = bucket_upserts_by_cell(&room);
        let bucketed: usize = buckets.values().map(|v| v.len()).sum();
        assert_eq!(room.len(), 2);
        assert_eq!(bucketed, 2);
        assert_eq!(buckets.len(), 2);
    }

    fn entity_at(id: u32, x: f64, y: f64, z: f64) -> Entity {
        alchemy_protocol::entity_trs(
            id,
            x,
            y,
            z,
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            1.0,
            1.0,
            1.0,
            1.0,
        )
    }

    /// x00c2: crossing 31.9 → 32.0 removes from old cell `0_0_0`.
    #[test]
    fn bucket_cell_exit_removes_on_boundary_cross() {
        let mut last = HashMap::new();
        last.insert(1, [0, 0, 0]);
        let upserts = [entity_at(1, 32.0, 0.0, 0.0)];
        let (upsert_buckets, remove_buckets) = plan_cell_frame_buckets(&upserts, &[], &last);
        assert_eq!(
            remove_buckets.get(&[0, 0, 0]).map(Vec::as_slice),
            Some(&[1][..])
        );
        assert_eq!(remove_buckets.len(), 1);
        assert_eq!(
            upsert_buckets.get(&[1, 0, 0]).map(Vec::as_slice),
            Some(&[0][..])
        );
    }

    /// x00c2: same-cell dirty upsert yields no exit remove.
    #[test]
    fn bucket_cell_exit_removes_empty_on_same_cell() {
        let mut last = HashMap::new();
        last.insert(1, [0, 0, 0]);
        let upserts = [entity_at(1, 10.0, 0.0, 0.0)];
        let (_u, r) = plan_cell_frame_buckets(&upserts, &[], &last);
        assert!(r.is_empty());
    }

    /// x00c2: first upsert (no prior cell) yields no exit remove.
    #[test]
    fn bucket_cell_exit_removes_empty_on_first_upsert() {
        let last = HashMap::new();
        let upserts = [entity_at(1, 32.0, 0.0, 0.0)];
        let (_u, r) = plan_cell_frame_buckets(&upserts, &[], &last);
        assert!(r.is_empty());
    }

    /// x00c2: Room removes / removes_only map to the last published cell.
    #[test]
    fn bucket_cell_exit_removes_on_despawn() {
        let mut last = HashMap::new();
        last.insert(7, [1, 0, 0]);
        last.insert(8, [0, 0, 0]);
        let (upsert_buckets, remove_buckets) = plan_cell_frame_buckets(&[], &[7], &last);
        assert!(upsert_buckets.is_empty());
        assert_eq!(
            remove_buckets.get(&[1, 0, 0]).map(Vec::as_slice),
            Some(&[7][..])
        );
        assert!(!remove_buckets.contains_key(&[0, 0, 0]));
    }

    /// x00c2: upserts∩removes → no cell upsert; old cell gets remove-only; map remove-wins.
    #[test]
    fn plan_cell_frame_intersection_remove_wins() {
        let mut last = HashMap::new();
        last.insert(1, [0, 0, 0]);
        // Coalesce can leave the same id in both lists (net poses + later removes_only).
        let upserts = [entity_at(1, 32.0, 0.0, 0.0)];
        let removes = [1u32];
        let (upsert_buckets, remove_buckets) = plan_cell_frame_buckets(&upserts, &removes, &last);
        assert!(
            upsert_buckets.is_empty(),
            "despawned id must not soft-put as cell upsert"
        );
        assert_eq!(
            remove_buckets.get(&[0, 0, 0]).map(Vec::as_slice),
            Some(&[1][..])
        );

        apply_last_wire_cell(&mut last, &upserts, &removes);
        assert!(
            !last.contains_key(&1),
            "last_wire_cell must match last_sent: remove wins"
        );
    }

    /// x00c2 DoD: after Room success, boundary cross materializes old remove + new upsert.
    #[test]
    fn materialize_cell_soft_puts_boundary_cross_after_room_plan() {
        let mut last = HashMap::new();
        last.insert(1, [0, 0, 0]);
        let upserts = [entity_at(1, 32.0, 0.0, 0.0)];
        let (u, r) = plan_cell_frame_buckets(&upserts, &[], &last);
        let puts = materialize_cell_soft_puts(&upserts, u, r);
        let old = puts
            .iter()
            .find(|(c, _, _)| *c == [0, 0, 0])
            .expect("old cell");
        let new = puts
            .iter()
            .find(|(c, _, _)| *c == [1, 0, 0])
            .expect("new cell");
        assert!(old.1.is_empty(), "old cell is remove-only");
        assert_eq!(old.2.as_slice(), &[1]);
        assert_eq!(new.1.len(), 1);
        assert_eq!(new.1[0].id, 1);
        assert!(new.2.is_empty());
        assert_eq!(puts.len(), 2);
    }

    /// x00c2 DoD: removes_only / despawn materializes last-cell remove-only soft-put.
    #[test]
    fn materialize_cell_soft_puts_despawn_remove_only() {
        let mut last = HashMap::new();
        last.insert(7, [1, 0, 0]);
        let (u, r) = plan_cell_frame_buckets(&[], &[7], &last);
        let puts = materialize_cell_soft_puts(&[], u, r);
        assert_eq!(puts.len(), 1);
        assert_eq!(puts[0].0, [1, 0, 0]);
        assert!(puts[0].1.is_empty());
        assert_eq!(puts[0].2.as_slice(), &[7]);
    }

    /// x00c2: apply_last_wire_cell tracks crosses and clears despawns.
    #[test]
    fn apply_last_wire_cell_tracks_cross_and_despawn() {
        let mut last = HashMap::new();
        apply_last_wire_cell(&mut last, &[entity_at(1, 31.9, 0.0, 0.0)], &[]);
        assert_eq!(last.get(&1), Some(&[0, 0, 0]));

        apply_last_wire_cell(&mut last, &[entity_at(1, 32.0, 0.0, 0.0)], &[]);
        assert_eq!(last.get(&1), Some(&[1, 0, 0]));

        apply_last_wire_cell(&mut last, &[], &[1]);
        assert!(!last.contains_key(&1));
    }

    /// x00c2: catch-up style full live upsert set + removes aligns the map.
    #[test]
    fn apply_last_wire_cell_catchup_rewrites_map() {
        let mut last = HashMap::new();
        last.insert(1, [0, 0, 0]);
        last.insert(2, [1, 0, 0]);
        last.insert(3, [-1, 0, 0]);
        // Catch-up upserts = all live; removes = departed (mirrors catchup_after_publish_gate).
        apply_last_wire_cell(
            &mut last,
            &[
                entity_at(1, 100.0, 0.0, 0.0),
                entity_at(2, 64.0, 0.0, 0.0),
                entity_at(4, 0.0, 0.0, 0.0),
            ],
            &[3],
        );
        assert_eq!(last.get(&1), Some(&[2, 0, 0])); // floor((100+32)/64) = 2
        assert_eq!(last.get(&2), Some(&[1, 0, 0]));
        assert!(!last.contains_key(&3));
        assert_eq!(last.get(&4), Some(&[0, 0, 0]));
    }

    /// x00be: Room tick authority key and origin cell dual-publish key stay distinct.
    #[test]
    fn origin_cell_frame_key_is_distinct_from_room_frame() {
        use alchemy_protocol::{
            frame_cell_key, frame_key, FRAME_CELL_KEY_LOCAL_ORIGIN, LOCAL_WIRE_CAP,
        };
        let room = frame_key("local", LOCAL_WIRE_CAP);
        let cell = frame_cell_key("local", LOCAL_WIRE_CAP, 0, 0, 0);
        assert_eq!(cell, FRAME_CELL_KEY_LOCAL_ORIGIN);
        assert_ne!(room, cell);
        assert!(cell.ends_with("/frame/cell/0_0_0"));
        assert!(!cell.contains("/frame/**"));
    }

    /// x00be: WAN ACL must list `frame/cell/*` explicitly on the four interest paths
    /// (never widen Room `…/frame` to `…/frame/**`).
    #[test]
    fn wan_acl_lists_explicit_frame_cell_keys() {
        let roots = [
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../relay-server/config.wan.json5"
            ),
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../relay-server/config.wan.e2e.json5"
            ),
        ];
        let cell = "alchemy/dev/room/*/*/frame/cell/*";
        let forbidden = "alchemy/dev/room/*/*/frame/**";
        let required_rules = [
            "host-put-sim",
            "host-egress-declare",
            "guest-sub-sim",
            "guest-egress-sim",
        ];
        for path in roots {
            let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
            assert!(
                !text.contains(forbidden),
                "{path} must not widen frame to frame/**"
            );
            let cell_count = text.matches(cell).count();
            assert!(
                cell_count >= required_rules.len(),
                "{path}: expected ≥{} `{cell}` rows, got {cell_count}",
                required_rules.len()
            );
            for rule in required_rules {
                assert!(text.contains(rule), "{path}: missing ACL rule id {rule}");
            }
        }
    }

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
            dirty_authored: false,
            dirty_inflight_mask: None,
            scene_rev: 0,
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
            dirty_authored: false,
            dirty_inflight_mask: None,
            scene_rev: 0,
        }));
        let mut g = hub.pending.lock().unwrap();
        let f = g.frame.take().unwrap();
        assert_eq!(f.host_time_ms, 2);
        assert!((f.poses[0].1.x - 9.0).abs() < f64::EPSILON);
        drop(g);
        hub.release_poses(f.poses);
    }

    #[test]
    fn coalesce_dirty_authored_unions_poses_and_or_masks() {
        // x00b1: delayed worker must not drop the earlier dirty pack when coalescing.
        let pool = PoseScratchPool::new(8);
        let mut mask_a = [0u64; BITMASK_WORDS];
        mask_a[0] = 1 << 1;
        let mut mask_b = [0u64; BITMASK_WORDS];
        mask_b[0] = 1 << 3;
        let prev = FrameSample {
            poses: scratch_one(
                &pool,
                11,
                Pose {
                    x: 1.0,
                    ..Pose::default()
                },
            ),
            removes: vec![],
            resume_catchup: false,
            publish_allowed: true,
            removes_only: false,
            host_time_ms: 1,
            sim_tick: 2,
            acked_intent_seq: vec![],
            dirty_authored: true,
            dirty_inflight_mask: Some(mask_a),
            scene_rev: 0,
        };
        let next = FrameSample {
            poses: scratch_one(
                &pool,
                33,
                Pose {
                    x: 3.0,
                    ..Pose::default()
                },
            ),
            removes: vec![],
            resume_catchup: false,
            publish_allowed: true,
            removes_only: false,
            host_time_ms: 2,
            sim_tick: 4,
            acked_intent_seq: vec![],
            dirty_authored: true,
            dirty_inflight_mask: Some(mask_b),
            scene_rev: 0,
        };
        let merged = coalesce_frame(prev, next, &pool);
        assert!(merged.dirty_authored);
        assert_eq!(merged.poses.len(), 2);
        assert!(merged.poses.iter().any(|(id, p)| *id == 11 && p.x == 1.0));
        assert!(merged.poses.iter().any(|(id, p)| *id == 33 && p.x == 3.0));
        let mask = merged.dirty_inflight_mask.expect("mask");
        assert_eq!(mask[0] & (1 << 1), 1 << 1);
        assert_eq!(mask[0] & (1 << 3), 1 << 3);
        pool.release(merged.poses);
    }

    #[test]
    fn coalesce_dirty_same_entity_lww_next_wins() {
        let pool = PoseScratchPool::new(8);
        let prev = FrameSample {
            poses: scratch_one(
                &pool,
                7,
                Pose {
                    x: 1.0,
                    ..Pose::default()
                },
            ),
            removes: vec![],
            resume_catchup: false,
            publish_allowed: true,
            removes_only: false,
            host_time_ms: 1,
            sim_tick: 1,
            acked_intent_seq: vec![],
            dirty_authored: true,
            dirty_inflight_mask: Some([1u64 << 0; BITMASK_WORDS]),
            scene_rev: 0,
        };
        let next = FrameSample {
            poses: scratch_one(
                &pool,
                7,
                Pose {
                    x: 9.0,
                    ..Pose::default()
                },
            ),
            removes: vec![],
            resume_catchup: false,
            publish_allowed: true,
            removes_only: false,
            host_time_ms: 2,
            sim_tick: 2,
            acked_intent_seq: vec![],
            dirty_authored: true,
            dirty_inflight_mask: Some([1u64 << 1; BITMASK_WORDS]),
            scene_rev: 0,
        };
        let merged = coalesce_frame(prev, next, &pool);
        assert_eq!(merged.poses.len(), 1);
        assert_eq!(merged.poses[0].0, 7);
        assert!((merged.poses[0].1.x - 9.0).abs() < f64::EPSILON);
        pool.release(merged.poses);
    }

    #[test]
    fn coalesce_dirty_into_full_keeps_prev_inflight_mask() {
        let pool = PoseScratchPool::new(8);
        let mut mask = [0u64; BITMASK_WORDS];
        mask[0] = 1 << 5;
        let prev = FrameSample {
            poses: scratch_one(&pool, 5, Pose::default()),
            removes: vec![],
            resume_catchup: false,
            publish_allowed: true,
            removes_only: false,
            host_time_ms: 1,
            sim_tick: 1,
            acked_intent_seq: vec![],
            dirty_authored: true,
            dirty_inflight_mask: Some(mask),
            scene_rev: 0,
        };
        let next = FrameSample {
            poses: scratch_one(
                &pool,
                99,
                Pose {
                    x: 2.0,
                    ..Pose::default()
                },
            ),
            removes: vec![],
            resume_catchup: true,
            publish_allowed: true,
            removes_only: false,
            host_time_ms: 2,
            sim_tick: 2,
            acked_intent_seq: vec![],
            dirty_authored: false,
            dirty_inflight_mask: None,
            scene_rev: 0,
        };
        let merged = coalesce_frame(prev, next, &pool);
        assert!(!merged.dirty_authored);
        assert_eq!(merged.poses.len(), 1);
        assert_eq!(merged.poses[0].0, 99);
        let kept = merged.dirty_inflight_mask.expect("prev inflight kept");
        assert_eq!(kept[0] & (1 << 5), 1 << 5);
        pool.release(merged.poses);
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
            dirty_authored: false,
            dirty_inflight_mask: None,
            scene_rev: 0,
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
            dirty_authored: false,
            dirty_inflight_mask: None,
            scene_rev: 0,
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
            dirty_authored: false,
            dirty_inflight_mask: None,
            scene_rev: 0,
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
            dirty_authored: false,
            dirty_inflight_mask: None,
            scene_rev: 0,
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
            last_wire_cell: HashMap::new(),
            net_tick: 0,
            last_published_tick: None,
            last_lod_put_at: None,
            last_lod_ids: HashMap::new(),
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
            dirty_authored: false,
            dirty_inflight_mask: None,
            scene_rev: 0,
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
            dirty_authored: false,
            dirty_inflight_mask: None,
            scene_rev: 0,
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
            dirty_authored: false,
            dirty_inflight_mask: None,
            scene_rev: 0,
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
            dirty_authored: false,
            dirty_inflight_mask: None,
            scene_rev: 0,
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
            dirty_authored: false,
            dirty_inflight_mask: None,
            scene_rev: 0,
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
            dirty_authored: false,
            dirty_inflight_mask: None,
            scene_rev: 0,
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
            dirty_authored: false,
            dirty_inflight_mask: None,
            scene_rev: 0,
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
            dirty_authored: false,
            dirty_inflight_mask: None,
            scene_rev: 0,
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
            dirty_authored: false,
            dirty_inflight_mask: None,
            scene_rev: 0,
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
            dirty_authored: false,
            dirty_inflight_mask: None,
            scene_rev: 0,
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
            dirty_authored: false,
            dirty_inflight_mask: None,
            scene_rev: 0,
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
            dirty_authored: false,
            dirty_inflight_mask: None,
            scene_rev: 0,
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
            dirty_authored: false,
            dirty_inflight_mask: None,
            scene_rev: 0,
        };
        let m = coalesce_frame(a, b, &pool);
        assert_eq!(m.sim_tick, 12);
        assert_eq!(m.acked_intent_seq, vec![9]);
        assert!((m.poses[0].1.x - 2.0).abs() < f64::EPSILON);
        pool.release(m.poses);
    }
}
