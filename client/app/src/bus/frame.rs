//! RenderFrame delta subscriber + snapshot convergence (x0023 / x0024 / x0029 / x00bf / x00c1).
//!
//! Guarantees:
//! - Frame finite / tick / `host_sig` verified before apply; snapshot requires non-empty `upserts`
//! - Tick gap beyond `FRAME_TICK_GAP_RESYNC` → delta not applied (world unchanged); `…/snapshot`
//!   replace via `fetch_snapshot`, rate-limited by `SNAPSHOT_GAP_RESYNC_COOLDOWN`
//! - Snapshot skips the tick gate so a racing delta cannot block convergence
//! - RenderFrame Entity carries quat + scale; client applies TRS (x0029)
//! - Class-specific decode byte/count limits and AEAD failure → drop (x003b)
//! - Room and opt-in cell interest share apply crypto; tick gates differ (x00bf / x00c0):
//!   cell uses strict [`decide_frame_tick`]; Room uses [`decide_room_frame_tick`] so a late
//!   Room full frame overrides a same-tick partial cell under 併読
//! - Cell interest threads do **not** gap→snapshot (Room path owns resync; avoids dual get)
//! - Opt-in cell interest follows wire `local_user_root` `CellId` with hysteresis and
//!   resubscribes Moore neighborhood on center change only (x00c1); Room sub stays put.
//!   Center is re-evaluated on a ≤250ms deadline even while cell samples keep arriving
//!
//! Non-guarantees: retransmission of missed deltas, reliable-delivery ACK channel, Manifest trust
//! (see `bus::manifest`; `host_pub` is read, not established here); cell-only delivery; AOI
//! bandwidth reduction (Room frame remains default / tick authority); Presence-unset follow;
//! cell gap convergence during resub.
//! Hot-path note (x00bf opt-in / x00c0 / x00c1): Room + near-cell 併読 may AEAD-open / sig-verify /
//! decode twice per tick; cell payloads are cell-filtered (Room remains tick authority).

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use alchemy_protocol::decode_limits::{check_frame, check_payload_bytes, WireClass};
use alchemy_protocol::wire_aead::AeadKind;
use alchemy_protocol::wire_crypto::{frame_entities_finite, verify_frame};
use alchemy_protocol::{
    decide_frame_tick, decide_room_frame_tick, frame_cell_interest_keyexpr,
    moore_neighborhood_cells, snapshot_key, FrameTickDecision, Message, RenderFrame,
    FRAME_TICK_GAP_RESYNC,
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
/// `from_room`: Room `…/frame` / snapshot use Room authority gate; cell interest uses strict.
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
            decide_frame_tick(world.last_frame_tick, tick, FRAME_TICK_GAP_RESYNC)
        };
        match decision {
            FrameTickDecision::DropStale => return ApplyFrameOutcome::Dropped,
            FrameTickDecision::ResyncSnapshot => return ApplyFrameOutcome::NeedResync,
            FrameTickDecision::Apply => {}
        }
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
    for id in removes {
        world.entities.remove(id);
    }
    world.last_frame_tick = Some(tick);
    // Snapshot and Room commits are Room authority; cell leaves the flag false for override.
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
    // Single lock for tick gate + entity mutate (no snapshot/delta interleave).
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
pub(crate) fn fetch_snapshot(
    session: &zenoh::Session,
    room_id: &str,
    wire_cap: &str,
    shared: &SharedState,
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
                                    shared, frame, /* replace */ true,
                                    /* from_room */ true,
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

fn cell_interest_key_for(room_id: &str, wire_cap: &str, center: CellId) -> String {
    let near = moore_neighborhood_cells(center.0[0], center.0[1], center.0[2]);
    frame_cell_interest_keyexpr(room_id, wire_cap, &near)
}

struct FrameGateCounters {
    dropped_limits: u64,
    dropped_aead: u64,
}

/// Handle one sealed frame sample (AEAD / decode / apply / optional Room gap→snapshot).
fn handle_frame_sample(
    shared: &SharedState,
    session: &zenoh::Session,
    bytes: &ZBytes,
    from_room: bool,
    gap_resync: bool,
    resync_cooldown_until: &mut Instant,
    gates: &mut FrameGateCounters,
) {
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
            match apply_frame(shared, frame, /* replace */ false, from_room) {
                ApplyFrameOutcome::Applied | ApplyFrameOutcome::Dropped => {}
                ApplyFrameOutcome::NeedResync => {
                    // x0024 / x00bf: Room fetches snapshot; cell only logs
                    // (same cooldown as Room so dual-publish gaps don't spam).
                    let now = Instant::now();
                    if now < *resync_cooldown_until {
                        return;
                    }
                    *resync_cooldown_until = now + SNAPSHOT_GAP_RESYNC_COOLDOWN;
                    if !gap_resync {
                        log::warn!(
                            "cell frame NeedResync (tick gap); defer to Room path / Room snapshot (no cell get)"
                        );
                        return;
                    }
                    if let Some(cap) = shared.wire_cap() {
                        let _ =
                            fetch_snapshot(session, shared.get_room_id().as_ref(), &cap, shared);
                    }
                }
            }
        }
        Err(err) => log::warn!("RenderFrame decode failed: {err}"),
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
    if gates.dropped_limits > 0 || gates.dropped_aead > 0 {
        log::info!(
            "frame gate ({thread_name}): dropped_limits={} dropped_aead={} (last ~5s)",
            gates.dropped_limits,
            gates.dropped_aead
        );
    }
    gates.dropped_limits = 0;
    gates.dropped_aead = 0;
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
            };
            let mut last_gate_log = Instant::now();
            while let Ok(sample) = subscriber.recv() {
                let bytes: ZBytes = sample.payload().clone();
                handle_frame_sample(
                    &shared,
                    &session,
                    &bytes,
                    /* from_room */ true,
                    /* gap_resync */ true,
                    &mut resync_cooldown_until,
                    &mut gates,
                );
                maybe_log_frame_gate(THREAD_NAME, &mut last_gate_log, &mut gates);
            }
        })
        .context("spawn frame thread")?;
    Ok(())
}

/// Opt-in cell interest: apply only; follow wire pose center with Moore resub (x00bf / x00c1).
pub(crate) fn spawn_cell_interest_subscriber(
    session: zenoh::Session,
    room_id: String,
    wire_cap: String,
    shared: Arc<SharedState>,
) -> Result<()> {
    let mut center = CellId([0, 0, 0]);
    let initial_key = cell_interest_key_for(&room_id, &wire_cap, center);
    let mut subscriber = session
        .declare_subscriber(initial_key.clone())
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_subscriber cell interest failed: {e}"))?;

    const THREAD_NAME: &str = "zenoh-frame-cell";
    thread::Builder::new()
        .name(THREAD_NAME.into())
        .spawn(move || {
            log::info!(
                "subscribed to {initial_key} (signed delta upserts/removes; kind=CellInterest; gap_resync=false; follow=wire_user_root)"
            );
            let mut resync_cooldown_until = Instant::now();
            let mut gates = FrameGateCounters {
                dropped_limits: 0,
                dropped_aead: 0,
            };
            let mut last_gate_log = Instant::now();
            // Deadline so busy cell traffic cannot starve interest follow (x00c1).
            let mut next_eval_at = Instant::now() + CELL_INTEREST_POLL;
            loop {
                let wait = next_eval_at.saturating_duration_since(Instant::now());
                match subscriber.recv_timeout(wait) {
                    Ok(Some(sample)) => {
                        let bytes: ZBytes = sample.payload().clone();
                        handle_frame_sample(
                            &shared,
                            &session,
                            &bytes,
                            /* from_room */ false,
                            /* gap_resync */ false,
                            &mut resync_cooldown_until,
                            &mut gates,
                        );
                        maybe_log_frame_gate(THREAD_NAME, &mut last_gate_log, &mut gates);
                    }
                    Ok(None) => {}
                    Err(_) => {
                        // Channel closed (session close / undeclare).
                        break;
                    }
                }
                if Instant::now() < next_eval_at {
                    continue;
                }
                next_eval_at = Instant::now() + CELL_INTEREST_POLL;
                let Some(xyz) = wire_user_root_xyz(&shared) else {
                    continue;
                };
                let next = maybe_advance_interest_center(center, xyz);
                if next == center {
                    continue;
                }
                let next_key = cell_interest_key_for(&room_id, &wire_cap, next);
                match session.declare_subscriber(next_key.clone()).wait() {
                    Ok(next_sub) => {
                        log::info!(
                            "cell interest resub center={:?} -> {:?} key={next_key}",
                            center.0,
                            next.0
                        );
                        drop(subscriber);
                        subscriber = next_sub;
                        center = next;
                    }
                    Err(err) => {
                        log::warn!(
                            "cell interest resub declare failed (keeping {:?}): {err}",
                            center.0
                        );
                    }
                }
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

    /// x00c0: cell partial then same-tick Room full must override (併読 race).
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

    /// x00bf: Moore interest `|` keyexpr must be acceptable to Zenoh `declare_subscriber`.
    #[test]
    fn cell_interest_keyexpr_declare_subscriber_ok() {
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
        let ke = cell_interest_key_for("local", LOCAL_WIRE_CAP, CellId([0, 0, 0]));
        assert!(ke.contains('|'));
        session
            .declare_subscriber(ke)
            .wait()
            .expect("declare_subscriber interest keyexpr");
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

    /// x00c1: Moore keyexpr changes when interest center moves.
    #[test]
    fn interest_keyexpr_changes_with_center() {
        use alchemy_protocol::LOCAL_WIRE_CAP;

        let k0 = cell_interest_key_for("local", LOCAL_WIRE_CAP, CellId([0, 0, 0]));
        let k1 = cell_interest_key_for("local", LOCAL_WIRE_CAP, CellId([1, 0, 0]));
        assert_ne!(k0, k1);
        assert!(k0.contains("0_0_0"));
        assert!(k1.contains("1_0_0"));
        assert!(!k0.contains("/frame/cell/2_0_0"));
        assert!(k1.contains("/frame/cell/2_0_0"));
    }
}
