//! RenderFrame delta subscriber + snapshot convergence (x0023 / x0024 / x0029 / x00bf / x00c1 / x00c3 / x00c4 / x00c5 / x00c6).
//!
//! Guarantees:
//! - Frame finite / tick / `host_sig` verified before apply; snapshot requires non-empty `upserts`
//! - Tick gap beyond `FRAME_TICK_GAP_RESYNC` → delta not applied (world unchanged); `…/snapshot`
//!   replace via `fetch_snapshot`, rate-limited by `SNAPSHOT_GAP_RESYNC_COOLDOWN`
//! - Snapshot skips the tick gate so a racing delta cannot block convergence
//! - RenderFrame Entity carries quat + scale; client applies TRS (x0029)
//! - Class-specific decode byte/count limits and AEAD failure → drop (x003b)
//! - Default path is **cell mainline** (x00c5/x00c6): Room `…/frame` is **not** subscribed.
//!   `ALCHEMY_ROOM_FRAME=1` opts into Room `…/frame` only. Exactly one frame mode runs;
//!   both modes share apply crypto; tick gates differ (x00bf / x00c0 / x00c3): cell uses
//!   [`decide_cell_frame_tick`] (same-tick merge); Room uses [`decide_room_frame_tick`]
//! - Whichever mode is active may gap→Room `…/snapshot` (`gap_resync=true`); there is no
//!   cell-scoped snapshot
//! - **x00c6**: cell mainline subscribes Moore 3³ **per-key ≤27** (`frame_cell_key`), follows
//!   wire `local_user_root` with hysteresis (x00c1), diffs resub on center change. Interest
//!   **enter** (new Moore cell) → rate-limited Room snapshot that is **deferred, never
//!   dropped** (`enter_snapshot_pending`); a successful **gap→Interest snapshot clears
//!   pending** (enter need already satisfied). **exit** prune runs in the same world lock as
//!   snapshot replace. Fan-in uses `try_send` (full → drop + count) so snapshot `get` cannot
//!   stall Zenoh callbacks. `…/frame/cell/*` is not the default path (debug/ACL helper only)
//! - x00c2/x00c3: host may soft-put old-cell removes and new-cell upserts on the **same**
//!   `net_tick`. Client merges same-tick cell samples (cross-cell upsert guard; within-frame
//!   remove wins). Room same-tick override remains for the Room opt-out path / unit tests
//! - **x00c6c**: `NeedResync` / snapshot cooldown must not stop MoveIntent publish or owned
//!   prediction ticks (those run on other threads / input paths; this module only applies wire)
//!
//! Non-guarantees: retransmission of missed deltas, reliable-delivery ACK channel, Manifest trust
//! (see `bus::manifest`; `host_pub` is read, not established here); host dual-publish stop;
//! Presence-unset follow; recovery when host cell soft-put fails (default Room frame is not
//! subscribed to backfill); AOI-exterior continuous visibility; Moore `|` union delivery;
//! `FRAME_TICK_GAP_RESYNC` relaxation; **AOI-invariant far ghosts** when a host cell remove is
//! missed (no periodic prune — cleared on next enter / gap→snapshot only).
//! Hot-path note: receive budget is Moore-bounded (≤27 keys). Snapshot / gap / enter authority
//! remains Room `…/snapshot`.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use alchemy_protocol::decode_limits::{check_frame, check_payload_bytes, WireClass};
use alchemy_protocol::wire_aead::AeadKind;
use alchemy_protocol::wire_crypto::{frame_entities_finite, verify_frame};
use alchemy_protocol::{
    decide_cell_frame_tick, decide_room_frame_tick, frame_cell_key, moore_neighborhood_cells,
    snapshot_key, FrameTickDecision, Message, RenderFrame, FRAME_TICK_GAP_RESYNC,
    MOORE_NEIGHBORHOOD_LEN,
};
use alchemy_shared_sim::mvcc::{CellId, EntityDirectory};
use anyhow::{Context, Result};
use zenoh::bytes::ZBytes;
use zenoh::Wait;

use super::wire::open_zenoh_payload;
use super::GATE_LOG_INTERVAL;
use crate::scene::{self, EntityPose, SharedState, LOCAL_USER_ROOT_UNSET};

/// Outcome of [`apply_frame`] (x0023 / x0024).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApplyFrameOutcome {
    /// Entity table updated.
    Applied,
    /// Dropped (bad sig / stale / not ready); no resync.
    Dropped,
    /// Tick gap on a delta — do not apply; caller must `fetch_snapshot`.
    NeedResync,
}

/// Apply a tick-gated delta to a [`scene::WorldView`] (no crypto). Snapshot path skips the gate.
///
/// `from_room`: Room `…/frame` / snapshot use Room authority gate; cell interest uses
/// [`decide_cell_frame_tick`] (x00c3 same-tick merge).
/// On [`ApplyFrameOutcome::NeedResync`], the world is left unchanged.
fn apply_delta_entities(
    world: &mut scene::WorldView,
    tick: u64,
    upserts: &[alchemy_protocol::Entity],
    removes: &[u32],
    replace_entities: bool,
    from_room: bool,
) -> ApplyFrameOutcome {
    if !replace_entities {
        let decision = if from_room {
            decide_room_frame_tick(
                world.last_frame_tick,
                tick,
                FRAME_TICK_GAP_RESYNC,
                world.last_frame_from_room,
            )
        } else {
            decide_cell_frame_tick(
                world.last_frame_tick,
                tick,
                FRAME_TICK_GAP_RESYNC,
                world.last_frame_from_room,
            )
        };
        match decision {
            FrameTickDecision::DropStale => return ApplyFrameOutcome::Dropped,
            FrameTickDecision::ResyncSnapshot => return ApplyFrameOutcome::NeedResync,
            FrameTickDecision::Apply => {}
        }
    }

    let same_tick_cell_merge = !from_room
        && !replace_entities
        && world.last_frame_tick == Some(tick)
        && !world.last_frame_from_room;
    let tick_advanced = world.last_frame_tick != Some(tick);
    if from_room || replace_entities || tick_advanced {
        world.cell_tick_upsert_guard.clear();
    }

    if replace_entities {
        world.entities.clear();
    }
    for e in upserts {
        world.entities.insert(
            e.id,
            EntityPose {
                x: e.x as f32,
                y: e.y as f32,
                z: e.z as f32,
                qx: e.qx as f32,
                qy: e.qy as f32,
                qz: e.qz as f32,
                qw: e.qw as f32,
                sx: e.sx as f32,
                sy: e.sy as f32,
                sz: e.sz as f32,
                r: e.r,
                g: e.g,
                b: e.b,
                a: e.a,
            },
        );
    }
    // Per-id membership for merge removes (avoid O(|removes|·|upserts|) scans).
    let upsert_ids_this_frame: std::collections::HashSet<u32> = if same_tick_cell_merge {
        upserts.iter().map(|e| e.id).collect()
    } else {
        std::collections::HashSet::new()
    };
    for id in removes {
        // Priority (x00c3): within-frame upserts∩removes → remove wins (host 契約).
        // Cross-cell: skip remove of id only when that id has no upsert this frame and is guarded.
        if same_tick_cell_merge
            && !upsert_ids_this_frame.contains(id)
            && world.cell_tick_upsert_guard.contains(id)
        {
            continue;
        }
        world.entities.remove(id);
    }

    if !from_room && !replace_entities {
        for e in upserts {
            if world.entities.contains_key(&e.id) {
                world.cell_tick_upsert_guard.insert(e.id);
            } else {
                world.cell_tick_upsert_guard.remove(&e.id);
            }
        }
    }

    world.last_frame_tick = Some(tick);
    // Snapshot and Room commits are Room authority; cell leaves the flag false for override/merge.
    world.last_frame_from_room = from_room || replace_entities;
    ApplyFrameOutcome::Applied
}

/// Apply a verified frame. `replace_entities` clears the map first (snapshot convergence).
///
/// Snapshot contract (x0023): success requires **non-empty `upserts`**. Empty signed
/// frames (host not ready) must not set `snapshot_ok`. Snapshot also **skips** the
/// tick gate so a racing delta cannot block convergence.
///
/// Delta contract (x0024): tick advance beyond [`FRAME_TICK_GAP_RESYNC`] →
/// [`ApplyFrameOutcome::NeedResync`] (no partial apply).
fn apply_frame(
    shared: &SharedState,
    frame: RenderFrame,
    replace_entities: bool,
    from_room: bool,
    // When set on successful snapshot replace: prune Moore-exterior under the same lock
    // (x00c6 Interest exit — no Room-full ghost frame between replace and prune).
    interest_near: Option<&HashSet<[i32; 3]>>,
) -> ApplyFrameOutcome {
    let host_pub = shared.host_pub.lock().ok().and_then(|g| g.clone());
    let Some(host_pub) = host_pub else {
        log::warn!("drop RenderFrame: host_pub not yet known from Manifest");
        return ApplyFrameOutcome::Dropped;
    };
    if let Err(err) = verify_frame(&host_pub, &frame) {
        log::warn!("drop RenderFrame: bad host_sig: {err}");
        return ApplyFrameOutcome::Dropped;
    }
    if !frame_entities_finite(&frame) {
        log::warn!(
            "drop RenderFrame: non-finite entity fields (tick={})",
            frame.tick
        );
        return ApplyFrameOutcome::Dropped;
    }
    if replace_entities && frame.upserts.is_empty() {
        log::warn!(
            "drop snapshot: empty upserts (tick={}); host not ready — retry later",
            frame.tick
        );
        return ApplyFrameOutcome::Dropped;
    }
    let keep_root = shared.local_user_root();
    // Single lock for tick gate + entity mutate (+ optional Interest prune).
    let Ok(mut world) = shared.world.lock() else {
        log::warn!("drop RenderFrame: world lock poisoned/failed");
        return ApplyFrameOutcome::Dropped;
    };
    let prev_tick = world.last_frame_tick;
    let outcome = apply_delta_entities(
        &mut world,
        frame.tick,
        &frame.upserts,
        &frame.removes,
        replace_entities,
        from_room,
    );
    if outcome == ApplyFrameOutcome::Applied && replace_entities {
        if let Some(near) = interest_near {
            prune_entities_outside_moore(&mut world, near, keep_root);
        }
    }
    // TRS-only copy under world lock; mirror after drop (avoid world ↔ epoch lock order).
    let poses_for_epoch = if outcome == ApplyFrameOutcome::Applied {
        Some(
            world
                .entities
                .iter()
                .map(|(&id, p)| {
                    (
                        id,
                        crate::epoch_mirror::KinematicPose {
                            x: p.x,
                            y: p.y,
                            z: p.z,
                            qx: p.qx,
                            qy: p.qy,
                            qz: p.qz,
                            qw: p.qw,
                            sx: p.sx,
                            sy: p.sy,
                            sz: p.sz,
                        },
                    )
                })
                .collect::<std::collections::HashMap<_, _>>(),
        )
    } else {
        None
    };
    drop(world);
    match outcome {
        ApplyFrameOutcome::Dropped
            if !replace_entities && prev_tick.is_some_and(|p| frame.tick < p) =>
        {
            log::warn!(
                "drop RenderFrame: tick {} < last {}",
                frame.tick,
                prev_tick.unwrap_or(0)
            );
        }
        ApplyFrameOutcome::NeedResync => {
            log::warn!(
                "frame tick gap: got {} last {:?} (max_gap={FRAME_TICK_GAP_RESYNC}); defer delta → snapshot",
                frame.tick,
                prev_tick
            );
            // Verified host Frame still proves the host is alive (x007c stall / Inventory Live).
            shared.note_host_alive();
        }
        ApplyFrameOutcome::Applied => {
            shared.metrics.note_frame_accepted();
            shared.note_host_alive();
            if let Some(poses) = poses_for_epoch {
                // Phase 3 will add wire `sim_tick`; until then net `tick` is the sim clock.
                crate::epoch_mirror::ingest_applied_world(
                    &shared.epoch,
                    &poses,
                    frame.tick,
                    frame.host_time_ms,
                );
                // x00ab: reconcile held Grab on authority only (held id lookup inside).
                shared.reconcile_held_on_authority(&poses);
            }
        }
        _ => {}
    }
    outcome
}

/// Returns true when a non-empty snapshot was verified and applied (replace).
///
/// `interest_near`: when `Some`, Moore-exterior entities are pruned in the **same** world
/// lock as the replace (x00c6 Interest enter/exit). Pass `None` for Room-only / Manifest paths.
pub(crate) fn fetch_snapshot(
    session: &zenoh::Session,
    room_id: &str,
    wire_cap: &str,
    shared: &SharedState,
) -> bool {
    fetch_snapshot_interest(session, room_id, wire_cap, shared, None)
}

fn fetch_snapshot_interest(
    session: &zenoh::Session,
    room_id: &str,
    wire_cap: &str,
    shared: &SharedState,
    interest_near: Option<&HashSet<[i32; 3]>>,
) -> bool {
    let key = snapshot_key(room_id, wire_cap);
    let mut applied = false;
    let mut dropped_limits: u64 = 0;
    let mut dropped_aead: u64 = 0;
    match session.get(&key).wait() {
        Ok(receiver) => {
            while let Ok(reply) = receiver.recv() {
                match reply.result() {
                    Ok(sample) => {
                        let bytes: ZBytes = sample.payload().clone();
                        let payload = match open_zenoh_payload(shared, AeadKind::Snapshot, &bytes) {
                            Ok(pt) => pt,
                            Err(err) => {
                                dropped_aead += 1;
                                log::warn!("drop snapshot: AEAD: {err}");
                                continue;
                            }
                        };
                        if let Err(err) = check_payload_bytes(WireClass::Frame, payload.len()) {
                            dropped_limits += 1;
                            log::warn!("drop snapshot: {err}");
                            continue;
                        }
                        match RenderFrame::decode(payload.as_ref()) {
                            Ok(frame) => {
                                if let Err(err) = check_frame(&frame) {
                                    dropped_limits += 1;
                                    log::warn!("drop snapshot: {err}");
                                    continue;
                                }
                                let snap_scene_rev = frame.scene_rev;
                                if apply_frame(
                                    shared,
                                    frame,
                                    /* replace */ true,
                                    /* from_room */ true,
                                    interest_near,
                                ) == ApplyFrameOutcome::Applied
                                {
                                    log::info!("snapshot applied from {key}");
                                    applied = true;
                                    // x00b3: fold live tip so EditCommit gaps converge.
                                    if snap_scene_rev != 0 {
                                        if let Ok(mut slot) = shared.last_edit_scene_rev.lock() {
                                            let next = match *slot {
                                                Some(prev) => prev.max(snap_scene_rev),
                                                None => snap_scene_rev,
                                            };
                                            *slot = Some(next);
                                        }
                                    }
                                }
                            }
                            Err(err) => log::warn!("snapshot decode failed: {err}"),
                        }
                    }
                    Err(err) => log::warn!("snapshot reply err: {err}"),
                }
            }
        }
        Err(err) => log::warn!("snapshot get failed: {err}"),
    }
    if dropped_limits > 0 || dropped_aead > 0 {
        log::info!("snapshot gate: dropped_limits={dropped_limits} dropped_aead={dropped_aead}");
    }
    if !applied {
        log::warn!("snapshot not applied yet; will retry on next Manifest or frame gap");
    }
    applied
}

/// Minimum spacing between gap-triggered snapshot gets (x0024). Applied **before**
/// `fetch_snapshot` so success and failure share the same 500ms wait (load shed;
/// see `protocol/peer-delivery.md`). Manifest path can still retry independently.
const SNAPSHOT_GAP_RESYNC_COOLDOWN: Duration = Duration::from_millis(500);

/// Poll interval for wire-position interest center checks (x00c1).
const CELL_INTEREST_POLL: Duration = Duration::from_millis(250);

/// Wire xyz for interest when Presence has assigned `local_user_root` and the entity exists.
fn wire_user_root_xyz(shared: &SharedState) -> Option<[f64; 3]> {
    let id = shared.local_user_root();
    if id == LOCAL_USER_ROOT_UNSET {
        return None;
    }
    let world = shared.world.lock().ok()?;
    let p = world.entities.get(&id)?;
    Some([f64::from(p.x), f64::from(p.y), f64::from(p.z)])
}

/// Advance interest center only after hysteresis trips; then snap to world `CellId` (x00c1).
fn maybe_advance_interest_center(current: CellId, xyz: [f64; 3]) -> CellId {
    let origin = current.origin_f64();
    let local = [
        (xyz[0] - origin[0]) as f32,
        (xyz[1] - origin[1]) as f32,
        (xyz[2] - origin[2]) as f32,
    ];
    match EntityDirectory::should_cross_cell(current, local) {
        Some(_) => CellId::from_world_f64(xyz[0], xyz[1], xyz[2]),
        None => current,
    }
}

/// Moore neighborhood as a set (x00c6 Interest).
fn moore_cell_set(center: CellId) -> HashSet<[i32; 3]> {
    moore_neighborhood_cells(center.0[0], center.0[1], center.0[2])
        .into_iter()
        .collect()
}

/// Drop entity poses whose world cell is outside `near` (keep local user_root). x00c6 exit.
fn prune_entities_outside_moore(
    world: &mut scene::WorldView,
    near: &HashSet<[i32; 3]>,
    keep_id: u32,
) {
    world.entities.retain(|id, pose| {
        if keep_id != LOCAL_USER_ROOT_UNSET && *id == keep_id {
            return true;
        }
        let cell = CellId::from_world_f64(f64::from(pose.x), f64::from(pose.y), f64::from(pose.z));
        near.contains(&cell.0)
    });
}

fn prune_shared_outside_moore(shared: &SharedState, near: &HashSet<[i32; 3]>) {
    let keep = shared.local_user_root();
    if let Ok(mut world) = shared.world.lock() {
        prune_entities_outside_moore(&mut world, near, keep);
    }
}

/// Interest-enter snapshot scheduling (x00c6b): rate-limit delays fetch; never drops the need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnterSnapAction {
    Idle,
    WaitCooldown,
    FetchNow,
}

fn enter_snapshot_action(pending: bool, now: Instant, cooldown_until: Instant) -> EnterSnapAction {
    if !pending {
        EnterSnapAction::Idle
    } else if now < cooldown_until {
        EnterSnapAction::WaitCooldown
    } else {
        EnterSnapAction::FetchNow
    }
}

/// Fan-in Moore cell subscribers into one channel (per-key ≤27; not `|` union). x00c6a.
///
/// Callbacks use [`SyncSender::try_send`] so a blocked consumer (e.g. synchronous snapshot
/// `get`) cannot stall Zenoh; full channel → drop + `dropped_backpressure` count.
///
/// Tuning note: under `FRAME_TICK_GAP_RESYNC=1`, sustained Full drops can cascade into
/// NeedResync. Prefer raising capacity / tightening drain before widening the gap constant.
fn sync_moore_cell_subscribers(
    session: &zenoh::Session,
    room_id: &str,
    wire_cap: &str,
    center: CellId,
    tx: &SyncSender<ZBytes>,
    dropped_backpressure: &Arc<AtomicU64>,
    subs: &mut HashMap<[i32; 3], zenoh::pubsub::Subscriber<()>>,
) -> Result<()> {
    let wanted = moore_cell_set(center);
    debug_assert!(wanted.len() <= MOORE_NEIGHBORHOOD_LEN);
    // Declare missing first so a mid-failure still leaves a superset (safer than holes).
    for cell in &wanted {
        if subs.contains_key(cell) {
            continue;
        }
        let key = frame_cell_key(room_id, wire_cap, cell[0], cell[1], cell[2]);
        let tx = tx.clone();
        let dropped_backpressure = Arc::clone(dropped_backpressure);
        let sub = session
            .declare_subscriber(key.clone())
            .callback(move |sample| match tx.try_send(sample.payload().clone()) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    dropped_backpressure.fetch_add(1, Ordering::Relaxed);
                }
                Err(TrySendError::Disconnected(_)) => {}
            })
            .wait()
            .map_err(|e| anyhow::anyhow!("declare_subscriber cell {key}: {e}"))?;
        subs.insert(*cell, sub);
    }
    subs.retain(|cell, _| wanted.contains(cell));
    Ok(())
}

struct FrameGateCounters {
    dropped_limits: u64,
    dropped_aead: u64,
    applied: u64,
    dropped_tick: u64,
    need_resync: u64,
    dropped_backpressure: u64,
}

/// Options for [`handle_frame_sample`] gap→snapshot / Interest prune (keeps arity low).
struct FrameSampleOpts<'a> {
    from_room: bool,
    gap_resync: bool,
    /// When `Some` and gap→snapshot fires, prune Moore-exterior under the same lock.
    interest_near: Option<&'a HashSet<[i32; 3]>>,
    /// Cleared when an interest-scoped Room snapshot applies (enter need already satisfied).
    enter_snapshot_pending: Option<&'a mut bool>,
}

/// Handle one sealed frame sample (AEAD / decode / apply / optional Room gap→snapshot).
fn handle_frame_sample(
    shared: &SharedState,
    session: &zenoh::Session,
    bytes: &ZBytes,
    opts: FrameSampleOpts<'_>,
    resync_cooldown_until: &mut Instant,
    gates: &mut FrameGateCounters,
) {
    let FrameSampleOpts {
        from_room,
        gap_resync,
        interest_near,
        enter_snapshot_pending,
    } = opts;
    let payload = match open_zenoh_payload(shared, AeadKind::Frame, bytes) {
        Ok(pt) => pt,
        Err(err) => {
            gates.dropped_aead += 1;
            log::warn!("drop RenderFrame: AEAD: {err}");
            return;
        }
    };
    if let Err(err) = check_payload_bytes(WireClass::Frame, payload.len()) {
        gates.dropped_limits += 1;
        log::warn!("drop RenderFrame: {err}");
        return;
    }
    match RenderFrame::decode(payload.as_ref()) {
        Ok(frame) => {
            if let Err(err) = check_frame(&frame) {
                gates.dropped_limits += 1;
                log::warn!("drop RenderFrame: {err}");
                return;
            }
            match apply_frame(
                shared, frame, /* replace */ false, from_room, /* interest_near */ None,
            ) {
                ApplyFrameOutcome::Applied => {
                    gates.applied += 1;
                }
                ApplyFrameOutcome::Dropped => {
                    gates.dropped_tick += 1;
                }
                ApplyFrameOutcome::NeedResync => {
                    gates.need_resync += 1;
                    // x0024 / x00c4: active frame path (Room XOR cell) may fetch Room `…/snapshot`.
                    // Cooldown sheds bursty NeedResync (gap storms / snapshot get storms).
                    // x00c6c: this path must not pause Intent publish or owned prediction —
                    // those live on other threads / input surfaces; we only refresh wire.
                    let now = Instant::now();
                    if now < *resync_cooldown_until {
                        return;
                    }
                    *resync_cooldown_until = now + SNAPSHOT_GAP_RESYNC_COOLDOWN;
                    // Production callers always pass `gap_resync=true` (Room default + cell
                    // mainline). `false` is kept as an apply-only API for tests / future paths.
                    if !gap_resync {
                        log::warn!(
                            "frame NeedResync (tick gap) with gap_resync=false; skip snapshot get"
                        );
                        return;
                    }
                    if let Some(cap) = shared.wire_cap() {
                        let applied = fetch_snapshot_interest(
                            session,
                            shared.get_room_id().as_ref(),
                            &cap,
                            shared,
                            interest_near,
                        );
                        // Gap snapshot with Interest prune already converges enter need.
                        clear_enter_pending_after_interest_snapshot(
                            applied,
                            enter_snapshot_pending,
                        );
                    }
                }
            }
        }
        Err(err) => log::warn!("RenderFrame decode failed: {err}"),
    }
}

/// Clear Interest-enter pending after a successful interest-scoped Room snapshot (gap or enter).
fn clear_enter_pending_after_interest_snapshot(
    applied: bool,
    enter_snapshot_pending: Option<&mut bool>,
) {
    if applied {
        if let Some(pending) = enter_snapshot_pending {
            *pending = false;
        }
    }
}

fn maybe_log_frame_gate(
    thread_name: &str,
    last_gate_log: &mut Instant,
    gates: &mut FrameGateCounters,
) {
    if last_gate_log.elapsed() < GATE_LOG_INTERVAL {
        return;
    }
    let noisy = gates.dropped_limits > 0
        || gates.dropped_aead > 0
        || gates.need_resync > 0
        || gates.dropped_backpressure > 0;
    if noisy {
        log::info!(
            "frame gate ({thread_name}): applied={} dropped_tick={} need_resync={} dropped_limits={} dropped_aead={} dropped_backpressure={} (last ~5s)",
            gates.applied,
            gates.dropped_tick,
            gates.need_resync,
            gates.dropped_limits,
            gates.dropped_aead,
            gates.dropped_backpressure
        );
    } else if gates.applied > 0 || gates.dropped_tick > 0 {
        log::debug!(
            "frame gate ({thread_name}): applied={} dropped_tick={} (last ~5s)",
            gates.applied,
            gates.dropped_tick
        );
    }
    gates.dropped_limits = 0;
    gates.dropped_aead = 0;
    gates.applied = 0;
    gates.dropped_tick = 0;
    gates.need_resync = 0;
    gates.dropped_backpressure = 0;
    *last_gate_log = Instant::now();
}

/// Room `…/frame` — tick authority; gap → Room `…/snapshot`.
pub(crate) fn spawn_frame_subscriber(
    session: zenoh::Session,
    key: String,
    shared: Arc<SharedState>,
) -> Result<()> {
    let subscriber = session
        .declare_subscriber(key.clone())
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_subscriber frame failed: {e}"))?;

    const THREAD_NAME: &str = "zenoh-frame";
    thread::Builder::new()
        .name(THREAD_NAME.into())
        .spawn(move || {
            log::info!(
                "subscribed to {key} (signed delta upserts/removes; kind=Room; gap_resync=true)"
            );
            let mut resync_cooldown_until = Instant::now();
            let mut gates = FrameGateCounters {
                dropped_limits: 0,
                dropped_aead: 0,
                applied: 0,
                dropped_tick: 0,
                need_resync: 0,
                dropped_backpressure: 0,
            };
            let mut last_gate_log = Instant::now();
            while let Ok(sample) = subscriber.recv() {
                let bytes: ZBytes = sample.payload().clone();
                handle_frame_sample(
                    &shared,
                    &session,
                    &bytes,
                    FrameSampleOpts {
                        from_room: true,
                        gap_resync: true,
                        interest_near: None,
                        enter_snapshot_pending: None,
                    },
                    &mut resync_cooldown_until,
                    &mut gates,
                );
                maybe_log_frame_gate(THREAD_NAME, &mut last_gate_log, &mut gates);
            }
        })
        .context("spawn frame thread")?;
    Ok(())
}

/// Default cell mainline (x00c6): Moore per-key ≤27 + gap/enter→Room snapshot + exit prune.
pub(crate) fn spawn_cell_interest_subscriber(
    session: zenoh::Session,
    room_id: String,
    wire_cap: String,
    shared: Arc<SharedState>,
) -> Result<()> {
    let center = CellId([0, 0, 0]);
    // Bounded fan-in from ≤27 cell callbacks (no flume dependency; std mpsc).
    let (tx, rx) = mpsc::sync_channel::<ZBytes>(512);
    let dropped_backpressure = Arc::new(AtomicU64::new(0));
    let mut subs: HashMap<[i32; 3], zenoh::pubsub::Subscriber<()>> = HashMap::new();
    sync_moore_cell_subscribers(
        &session,
        &room_id,
        &wire_cap,
        center,
        &tx,
        &dropped_backpressure,
        &mut subs,
    )
    .context("declare initial Moore cell subscribers")?;

    const THREAD_NAME: &str = "zenoh-frame-cell";
    thread::Builder::new()
        .name(THREAD_NAME.into())
        .spawn(move || {
            // Keep a sender alive so callback clones do not disconnect the channel.
            let _tx_keep = tx;
            log::info!(
                "subscribed Moore per-key ≤{MOORE_NEIGHBORHOOD_LEN} cells around {:?} \
                 (kind=CellInterest; gap_resync=true; enter→Room snapshot deferred-not-dropped; \
                  exit prune; try_send fan-in; x00c6)",
                center.0
            );
            let mut center = center;
            let mut near_cells = moore_cell_set(center);
            let mut subs = subs;
            let mut resync_cooldown_until = Instant::now();
            let mut enter_snapshot_pending = false;
            let mut gates = FrameGateCounters {
                dropped_limits: 0,
                dropped_aead: 0,
                applied: 0,
                dropped_tick: 0,
                need_resync: 0,
                dropped_backpressure: 0,
            };
            let mut last_gate_log = Instant::now();
            // Deadline so busy cell traffic cannot starve interest follow (x00c1 / x00c6).
            let mut next_eval_at = Instant::now() + CELL_INTEREST_POLL;
            let mut primed_pose = false;

            // Drain backlog without blocking (critical after sync snapshot get).
            loop {
                let wait = next_eval_at.saturating_duration_since(Instant::now());
                match rx.recv_timeout(wait) {
                    Ok(bytes) => {
                        handle_frame_sample(
                            &shared,
                            &session,
                            &bytes,
                            FrameSampleOpts {
                                from_room: false,
                                gap_resync: true,
                                interest_near: Some(&near_cells),
                                enter_snapshot_pending: Some(&mut enter_snapshot_pending),
                            },
                            &mut resync_cooldown_until,
                            &mut gates,
                        );
                        while let Ok(more) = rx.try_recv() {
                            handle_frame_sample(
                                &shared,
                                &session,
                                &more,
                                FrameSampleOpts {
                                    from_room: false,
                                    gap_resync: true,
                                    interest_near: Some(&near_cells),
                                    enter_snapshot_pending: Some(&mut enter_snapshot_pending),
                                },
                                &mut resync_cooldown_until,
                                &mut gates,
                            );
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
                gates.dropped_backpressure += dropped_backpressure.swap(0, Ordering::Relaxed);
                maybe_log_frame_gate(THREAD_NAME, &mut last_gate_log, &mut gates);

                // Pending enter may become due while idle / under frame traffic.
                match enter_snapshot_action(
                    enter_snapshot_pending,
                    Instant::now(),
                    resync_cooldown_until,
                ) {
                    EnterSnapAction::Idle | EnterSnapAction::WaitCooldown => {}
                    EnterSnapAction::FetchNow => {
                        resync_cooldown_until = Instant::now() + SNAPSHOT_GAP_RESYNC_COOLDOWN;
                        let applied = if let Some(cap) = shared.wire_cap() {
                            fetch_snapshot_interest(
                                &session,
                                shared.get_room_id().as_ref(),
                                &cap,
                                &shared,
                                Some(&near_cells),
                            )
                        } else {
                            false
                        };
                        enter_snapshot_pending = !applied;
                        while let Ok(more) = rx.try_recv() {
                            handle_frame_sample(
                                &shared,
                                &session,
                                &more,
                                FrameSampleOpts {
                                    from_room: false,
                                    gap_resync: true,
                                    interest_near: Some(&near_cells),
                                    enter_snapshot_pending: Some(&mut enter_snapshot_pending),
                                },
                                &mut resync_cooldown_until,
                                &mut gates,
                            );
                        }
                    }
                }

                if Instant::now() < next_eval_at {
                    continue;
                }
                next_eval_at = Instant::now() + CELL_INTEREST_POLL;
                let Some(xyz) = wire_user_root_xyz(&shared) else {
                    continue;
                };

                // First wire pose: snap center (join snapshot may include far ghosts).
                if !primed_pose {
                    let first = CellId::from_world_f64(xyz[0], xyz[1], xyz[2]);
                    if first != center {
                        match sync_moore_cell_subscribers(
                            &session,
                            &room_id,
                            &wire_cap,
                            first,
                            &_tx_keep,
                            &dropped_backpressure,
                            &mut subs,
                        ) {
                            Ok(()) => {
                                log::info!(
                                    "cell interest prime center={:?} -> {:?} (subs={})",
                                    center.0,
                                    first.0,
                                    subs.len()
                                );
                                center = first;
                                near_cells = moore_cell_set(center);
                            }
                            Err(err) => {
                                log::warn!(
                                    "cell interest prime declare failed (will retry): {err}"
                                );
                                // Do not set primed_pose — retry next poll.
                                continue;
                            }
                        }
                    } else {
                        near_cells = moore_cell_set(center);
                    }
                    primed_pose = true;
                    // Enter-style converge once Presence has a pose (deferred if cooldown).
                    enter_snapshot_pending = true;
                    // Exit prune immediately so join far-ghosts do not linger until snapshot.
                    prune_shared_outside_moore(&shared, &near_cells);
                    continue;
                }

                let next = maybe_advance_interest_center(center, xyz);
                if next == center {
                    continue;
                }

                let new_set = moore_cell_set(next);
                let entered = new_set.difference(&near_cells).count();

                match sync_moore_cell_subscribers(
                    &session,
                    &room_id,
                    &wire_cap,
                    next,
                    &_tx_keep,
                    &dropped_backpressure,
                    &mut subs,
                ) {
                    Ok(()) => {
                        log::info!(
                            "cell interest resub center={:?} -> {:?} entered={entered} subs={}",
                            center.0,
                            next.0,
                            subs.len()
                        );
                        center = next;
                        near_cells = new_set;
                    }
                    Err(err) => {
                        log::warn!(
                            "cell interest resub declare failed (keeping {:?}): {err}",
                            center.0
                        );
                        continue;
                    }
                }

                // Interest enter (x00c6b): always schedule; top-of-loop flushes when due.
                if entered > 0 {
                    enter_snapshot_pending = true;
                }
                // Exit prune for cells we no longer subscribe (do not wait for snapshot).
                prune_shared_outside_moore(&shared, &near_cells);
            }
        })
        .context("spawn cell interest thread")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alchemy_protocol::Entity;
    use scene::WorldView;

    fn sample_entity(id: u32, x: f64) -> Entity {
        alchemy_protocol::entity_at(id, x, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0)
    }

    #[test]
    fn gap_delta_does_not_mutate_entities_and_signals_resync() {
        let mut world = WorldView {
            last_frame_tick: Some(10),
            last_frame_from_room: true,
            cell_tick_upsert_guard: Default::default(),
            entities: [(
                1,
                EntityPose {
                    x: 1.0,
                    ..Default::default()
                },
            )]
            .into(),
        };
        let before = world.entities.clone();

        let outcome =
            apply_delta_entities(&mut world, 12, &[sample_entity(1, 9.0)], &[], false, true);

        assert_eq!(outcome, ApplyFrameOutcome::NeedResync);
        assert_eq!(world.last_frame_tick, Some(10));
        assert_eq!(world.entities, before);
    }

    #[test]
    fn consecutive_delta_applies_and_advances_tick() {
        let mut world = WorldView {
            last_frame_tick: Some(10),
            last_frame_from_room: true,
            cell_tick_upsert_guard: Default::default(),
            entities: Default::default(),
        };

        let outcome =
            apply_delta_entities(&mut world, 11, &[sample_entity(7, 3.0)], &[], false, true);

        assert_eq!(outcome, ApplyFrameOutcome::Applied);
        assert_eq!(world.last_frame_tick, Some(11));
        assert!(world.last_frame_from_room);
        assert!((world.entities.get(&7).unwrap().x - 3.0).abs() < f32::EPSILON);
    }

    #[test]
    fn delta_applies_quat_and_scale() {
        let mut world = WorldView {
            last_frame_tick: Some(1),
            last_frame_from_room: true,
            cell_tick_upsert_guard: Default::default(),
            entities: Default::default(),
        };
        let e = alchemy_protocol::entity_trs(
            10,
            0.0,
            0.25,
            0.0,
            [0.0, 0.0, 0.70710677, 0.70710677],
            [2.0, 1.0, 0.5],
            1.0,
            1.0,
            1.0,
            1.0,
        );

        let outcome = apply_delta_entities(&mut world, 2, &[e], &[], false, true);

        assert_eq!(outcome, ApplyFrameOutcome::Applied);
        let p = world.entities.get(&10).unwrap();
        assert!((p.qw - 0.70710677).abs() < 1e-5);
        assert!((p.sx - 2.0).abs() < f32::EPSILON);
        assert!((p.sz - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn snapshot_replace_skips_tick_gate() {
        let mut world = WorldView {
            last_frame_tick: Some(100),
            last_frame_from_room: true,
            cell_tick_upsert_guard: Default::default(),
            entities: [(1, EntityPose::default())].into(),
        };

        let outcome =
            apply_delta_entities(&mut world, 5, &[sample_entity(2, 1.5)], &[], true, true);

        assert_eq!(outcome, ApplyFrameOutcome::Applied);
        assert_eq!(world.last_frame_tick, Some(5));
        assert!(world.last_frame_from_room);
        assert!(!world.entities.contains_key(&1));
        assert!(world.entities.contains_key(&2));
    }

    /// Host must stamp snapshot with *last published* tick. If it wrongly used the next
    /// unpublished `net_tick`, the following delta would share that tick and DropStale —
    /// entity updates stall forever (x0024 regression).
    #[test]
    fn after_snapshot_next_published_tick_applies_same_tick_is_stale() {
        let mut world = WorldView::default();
        let snap = apply_delta_entities(&mut world, 5, &[sample_entity(1, 1.0)], &[], true, true);
        assert_eq!(snap, ApplyFrameOutcome::Applied);
        assert_eq!(world.last_frame_tick, Some(5));
        assert!(world.last_frame_from_room);

        // Wrong host: snapshot.tick == next delta.tick → DropStale, pose stuck.
        let same = apply_delta_entities(&mut world, 5, &[sample_entity(1, 9.0)], &[], false, true);
        assert_eq!(same, ApplyFrameOutcome::Dropped);
        assert!((world.entities.get(&1).unwrap().x - 1.0).abs() < f32::EPSILON);

        // Correct host: next publish is last_published+1.
        let next = apply_delta_entities(&mut world, 6, &[sample_entity(1, 9.0)], &[], false, true);
        assert_eq!(next, ApplyFrameOutcome::Applied);
        assert_eq!(world.last_frame_tick, Some(6));
        assert!((world.entities.get(&1).unwrap().x - 9.0).abs() < f32::EPSILON);
    }

    /// x00c2: cell-scoped removes clear entities (host exit/despawn on old cell topic).
    #[test]
    fn cell_removes_clear_entities() {
        let mut world = WorldView::default();
        assert_eq!(
            apply_delta_entities(
                &mut world,
                1,
                &[sample_entity(1, 1.0)],
                &[],
                false,
                /* from_room */ false,
            ),
            ApplyFrameOutcome::Applied
        );
        assert!(world.entities.contains_key(&1));
        assert_eq!(
            apply_delta_entities(&mut world, 2, &[], &[1], false, false),
            ApplyFrameOutcome::Applied
        );
        assert!(!world.entities.contains_key(&1));
        assert!(!world.last_frame_from_room);
    }

    /// x00c3: same-tick new-cell upsert then old-cell remove → merge; entity stays (Room 無し).
    #[test]
    fn cell_exit_same_tick_upsert_then_remove_merges() {
        let mut world = WorldView::default();
        assert_eq!(
            apply_delta_entities(&mut world, 4, &[sample_entity(1, 1.0)], &[], false, false),
            ApplyFrameOutcome::Applied
        );

        assert_eq!(
            apply_delta_entities(&mut world, 5, &[sample_entity(1, 32.0)], &[], false, false),
            ApplyFrameOutcome::Applied
        );
        assert!((world.entities.get(&1).unwrap().x - 32.0).abs() < f32::EPSILON);
        assert!(world.cell_tick_upsert_guard.contains(&1));

        // Old cell remove same tick merges; guarded upsert wins.
        assert_eq!(
            apply_delta_entities(&mut world, 5, &[], &[1], false, false),
            ApplyFrameOutcome::Applied
        );
        assert!(!world.last_frame_from_room);
        assert!((world.entities.get(&1).unwrap().x - 32.0).abs() < f32::EPSILON);
    }

    /// x00c3: reverse arrival — old-cell remove then new-cell upsert merges (Room 無し).
    #[test]
    fn cell_exit_same_tick_remove_then_upsert_merges() {
        let mut world = WorldView::default();
        assert_eq!(
            apply_delta_entities(&mut world, 4, &[sample_entity(1, 1.0)], &[], false, false),
            ApplyFrameOutcome::Applied
        );

        assert_eq!(
            apply_delta_entities(&mut world, 5, &[], &[1], false, false),
            ApplyFrameOutcome::Applied
        );
        assert!(!world.entities.contains_key(&1));

        assert_eq!(
            apply_delta_entities(&mut world, 5, &[sample_entity(1, 32.0)], &[], false, false),
            ApplyFrameOutcome::Applied
        );
        assert!(!world.last_frame_from_room);
        assert!((world.entities.get(&1).unwrap().x - 32.0).abs() < f32::EPSILON);
        assert!(world.cell_tick_upsert_guard.contains(&1));
    }

    /// x00c3: within a single cell frame, upserts∩removes stays remove-wins (host 契約).
    #[test]
    fn cell_same_frame_upsert_remove_intersection_remove_wins() {
        let mut world = WorldView::default();
        assert_eq!(
            apply_delta_entities(&mut world, 1, &[sample_entity(1, 8.0)], &[1], false, false,),
            ApplyFrameOutcome::Applied
        );
        assert!(!world.entities.contains_key(&1));
        assert!(!world.cell_tick_upsert_guard.contains(&1));
    }

    /// x00c3: mid-merge intersection still remove-wins even if id was already guarded.
    #[test]
    fn cell_merge_same_frame_intersection_remove_wins_over_guard() {
        let mut world = WorldView::default();
        assert_eq!(
            apply_delta_entities(&mut world, 1, &[sample_entity(1, 1.0)], &[], false, false),
            ApplyFrameOutcome::Applied
        );
        assert!(world.cell_tick_upsert_guard.contains(&1));

        // Same tick, another cell frame with upsert∩remove for the guarded id.
        assert_eq!(
            apply_delta_entities(&mut world, 1, &[sample_entity(1, 9.0)], &[1], false, false,),
            ApplyFrameOutcome::Applied
        );
        assert!(!world.entities.contains_key(&1));
        assert!(!world.cell_tick_upsert_guard.contains(&1));
    }

    /// x00c3/x00c5: Room same-tick override API (Room opt-out / gate regression; not cell-mainline runtime).
    #[test]
    fn room_overrides_same_tick_after_multi_cell_merge() {
        let mut world = WorldView::default();
        assert_eq!(
            apply_delta_entities(&mut world, 4, &[sample_entity(1, 1.0)], &[], false, false),
            ApplyFrameOutcome::Applied
        );
        assert_eq!(
            apply_delta_entities(&mut world, 5, &[sample_entity(1, 32.0)], &[], false, false),
            ApplyFrameOutcome::Applied
        );
        assert_eq!(
            apply_delta_entities(&mut world, 5, &[], &[1], false, false),
            ApplyFrameOutcome::Applied
        );
        assert!(!world.last_frame_from_room);
        assert!(world.cell_tick_upsert_guard.contains(&1));
        assert!((world.entities.get(&1).unwrap().x - 32.0).abs() < f32::EPSILON);

        assert_eq!(
            apply_delta_entities(
                &mut world,
                5,
                &[sample_entity(1, 40.0), sample_entity(2, 2.0)],
                &[],
                false,
                /* from_room */ true,
            ),
            ApplyFrameOutcome::Applied
        );
        assert!(world.last_frame_from_room);
        assert!(world.cell_tick_upsert_guard.is_empty());
        assert!((world.entities.get(&1).unwrap().x - 40.0).abs() < f32::EPSILON);
        assert!(world.entities.contains_key(&2));

        // Late cell after Room stays DropStale.
        assert_eq!(
            apply_delta_entities(&mut world, 5, &[], &[2], false, false),
            ApplyFrameOutcome::Dropped
        );
        assert!(world.entities.contains_key(&2));
    }

    /// x00c0/x00c5: cell partial then same-tick Room full overrides (gate regression; Room opt-out path).
    #[test]
    fn room_overrides_same_tick_after_partial_cell() {
        let mut world = WorldView::default();
        let cell = apply_delta_entities(
            &mut world,
            3,
            &[sample_entity(1, 1.0)],
            &[],
            false,
            /* from_room */ false,
        );
        assert_eq!(cell, ApplyFrameOutcome::Applied);
        assert!(!world.last_frame_from_room);
        assert!((world.entities.get(&1).unwrap().x - 1.0).abs() < f32::EPSILON);

        let room = apply_delta_entities(
            &mut world,
            3,
            &[sample_entity(1, 9.0), sample_entity(2, 4.0)],
            &[99],
            false,
            /* from_room */ true,
        );
        assert_eq!(room, ApplyFrameOutcome::Applied);
        assert!(world.last_frame_from_room);
        assert!((world.entities.get(&1).unwrap().x - 9.0).abs() < f32::EPSILON);
        assert!(world.entities.contains_key(&2));

        // Second Room same tick stays DropStale (x0024).
        let dup = apply_delta_entities(&mut world, 3, &[sample_entity(1, 0.0)], &[], false, true);
        assert_eq!(dup, ApplyFrameOutcome::Dropped);

        // Peer cell same tick after Room stays DropStale.
        let late_cell =
            apply_delta_entities(&mut world, 3, &[sample_entity(1, 0.0)], &[], false, false);
        assert_eq!(late_cell, ApplyFrameOutcome::Dropped);
    }

    // Identity / presence pin / gen-rev unit tables live in alchemy-client-trust (x0070).

    /// x00c6: Moore per-key declares (≤27) must succeed on a local peer.
    #[test]
    fn moore_per_key_declare_subscriber_ok() {
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
        let (tx, _rx) = mpsc::sync_channel::<ZBytes>(8);
        let dropped = Arc::new(AtomicU64::new(0));
        let mut subs = HashMap::new();
        sync_moore_cell_subscribers(
            &session,
            "local",
            LOCAL_WIRE_CAP,
            CellId([0, 0, 0]),
            &tx,
            &dropped,
            &mut subs,
        )
        .expect("declare Moore per-key");
        assert_eq!(subs.len(), MOORE_NEIGHBORHOOD_LEN);
        // Diff resub to a neighbor center must stay ≤27.
        sync_moore_cell_subscribers(
            &session,
            "local",
            LOCAL_WIRE_CAP,
            CellId([1, 0, 0]),
            &tx,
            &dropped,
            &mut subs,
        )
        .expect("resub Moore per-key");
        assert_eq!(subs.len(), MOORE_NEIGHBORHOOD_LEN);
    }

    /// x00c1: stay inside hysteresis; cross only past 32+2 from current center origin.
    #[test]
    fn interest_center_hysteresis_holds_then_snaps() {
        let origin = CellId([0, 0, 0]);
        assert_eq!(
            maybe_advance_interest_center(origin, [31.9, 0.0, 0.0]),
            origin
        );
        assert_eq!(
            maybe_advance_interest_center(origin, [34.1, 0.0, 0.0]),
            CellId([1, 0, 0])
        );
        // Far teleport: once hysteresis trips, snap to true world cell (not crawl ±1).
        assert_eq!(
            maybe_advance_interest_center(origin, [200.0, 0.0, 0.0]),
            CellId::from_world_f64(200.0, 0.0, 0.0)
        );
        // From cell 1 (origin x=64): world 34.1 → local≈-29.9 stays; world 29.9 → local≈-34.1 snaps.
        let c1 = CellId([1, 0, 0]);
        assert_eq!(maybe_advance_interest_center(c1, [34.1, 0.0, 0.0]), c1);
        assert_eq!(
            maybe_advance_interest_center(c1, [29.9, 0.0, 0.0]),
            CellId([0, 0, 0])
        );
    }

    /// x00c6: Moore key set changes when interest center moves.
    #[test]
    fn moore_set_changes_with_center() {
        let a = moore_cell_set(CellId([0, 0, 0]));
        let b = moore_cell_set(CellId([1, 0, 0]));
        assert_eq!(a.len(), MOORE_NEIGHBORHOOD_LEN);
        assert_eq!(b.len(), MOORE_NEIGHBORHOOD_LEN);
        assert!(a.contains(&[0, 0, 0]));
        assert!(b.contains(&[1, 0, 0]));
        assert_ne!(a, b);
        assert!(b.difference(&a).count() > 0);
    }

    /// x00c6b: exit prune drops far poses; keeps local user_root even if outside.
    #[test]
    fn prune_outside_moore_keeps_near_and_local_root() {
        let mut world = WorldView {
            last_frame_tick: Some(1),
            last_frame_from_room: true,
            cell_tick_upsert_guard: Default::default(),
            entities: [
                (
                    1,
                    EntityPose {
                        x: 1.0,
                        ..Default::default()
                    },
                ),
                (
                    2,
                    EntityPose {
                        x: 500.0,
                        ..Default::default()
                    },
                ),
                (
                    99,
                    EntityPose {
                        x: 999.0,
                        ..Default::default()
                    },
                ),
            ]
            .into(),
        };
        let near = moore_cell_set(CellId([0, 0, 0]));
        prune_entities_outside_moore(&mut world, &near, 99);
        assert!(world.entities.contains_key(&1));
        assert!(!world.entities.contains_key(&2));
        assert!(world.entities.contains_key(&99));
    }

    /// x00c6b: enter snapshot is deferred under cooldown, never dropped.
    #[test]
    fn enter_snapshot_pending_waits_then_fetches() {
        let t0 = Instant::now();
        let cool = t0 + Duration::from_millis(500);
        assert_eq!(
            enter_snapshot_action(false, t0, cool),
            EnterSnapAction::Idle
        );
        assert_eq!(
            enter_snapshot_action(true, t0, cool),
            EnterSnapAction::WaitCooldown
        );
        assert_eq!(
            enter_snapshot_action(true, cool, cool),
            EnterSnapAction::FetchNow
        );
        assert_eq!(
            enter_snapshot_action(true, cool + Duration::from_millis(1), cool),
            EnterSnapAction::FetchNow
        );
    }

    /// x00c6b: successful interest-scoped snapshot clears enter pending (gap or enter path).
    #[test]
    fn gap_interest_snapshot_applied_clears_enter_pending() {
        let mut pending = true;
        clear_enter_pending_after_interest_snapshot(false, Some(&mut pending));
        assert!(pending, "failed snapshot must leave pending set");
        clear_enter_pending_after_interest_snapshot(true, Some(&mut pending));
        assert!(!pending, "applied interest snapshot satisfies enter need");
        clear_enter_pending_after_interest_snapshot(true, None);
    }

    /// x00c6c: NeedResync leaves world untouched — Intent/prediction are other threads
    /// (`spawn_input_publisher`, surface prediction) and are not locked here.
    #[test]
    fn need_resync_is_pure_wrt_world_and_has_no_intent_side_effect_api() {
        let mut world = WorldView {
            last_frame_tick: Some(10),
            last_frame_from_room: false,
            cell_tick_upsert_guard: Default::default(),
            entities: [(
                1,
                EntityPose {
                    x: 1.0,
                    ..Default::default()
                },
            )]
            .into(),
        };
        let before = world.entities.clone();
        let outcome =
            apply_delta_entities(&mut world, 12, &[sample_entity(1, 9.0)], &[], false, false);
        assert_eq!(outcome, ApplyFrameOutcome::NeedResync);
        assert_eq!(world.entities, before);
        assert_eq!(world.last_frame_tick, Some(10));
        // Contract anchor: this module never takes `shared.intent` / owned_prediction locks
        // on the NeedResync path (see handle_frame_sample); MoveIntent publish continues.
    }
}
