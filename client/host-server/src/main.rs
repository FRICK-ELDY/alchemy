//! Local host-server — x0025 host_pub attestation + x0024 wire-tick + x0023 session.
//!
//! Guarantees:
//! - Internal Rapier tick at 60Hz; network delta publish at 30Hz
//! - Input: Ed25519 session bind (Token on bind only) + `is_finite` (x0023 / x0021)
//! - SceneManifest from world DemoScene + attestation + seal; Frames / Presence signed (x0027)
//! - x003d: credential / ManifestSeal HTTP on a background worker; physics evaluates
//!   hard expiry each tick. Transient refresh failure keeps last good creds; hard expiry
//!   stops Manifest/Presence/Frame/**snapshot queryable** without exiting the process.
//!   Resume synthesizes wire `removes` + full upserts from the last `last_sent` view
//! - x0042: Zenoh Frame/Manifest/Presence `put().wait()` on `zenoh-publish`; physics
//!   submits wire-agnostic pose samples / jobs (encode+sign stay in the worker)
//! - `…/snapshot` queryable for late join (refused while gated); pure pose deltas
//! - Empty deltas: **no publish and no `net_tick` advance**; snapshot tick = last published (x0024)
//! - Per-`sub` Intent/Grab → Manifest user_root slots (≤ `USER_ROOT_SLOTS_MAX`; x004d);
//!   Presence Leave / liveliness jti grace (x0022)
//! - x0040: Intent rate after Ed25519 (steady: after seq eval, before `last_seq` commit,
//!   Leave-exempt; bind: atomic global 20/10s + per-pub 5/2s before JWT, Leave **not**
//!   exempt; failed bind drops unbound per-pub; rebind GC skips the new pub).
//!   JWKS hard stale on Room Token verify
//! - Zenoh keys from `ROOM_ID` via protocol `*_key` helpers (x0017)
//! - Slice E / x001e: Zenoh `host_control` off by default; `ALCHEMY_HOST_CONTROL=1` to enable
//! - x0032: localhost IPC shutdown when `ALCHEMY_HOST_IPC_*` set (alchemy-launcher)
//!
//! Non-guarantees:
//! - Steal / rotation follow / Formula / polished Walk / verifier
//! - Authenticated remote (WAN) host shutdown
//! - Re-fetch of world `GET …/scene` after boot (room redefinition requires host restart)
//! - rkyv / bytemuck wire (encode is protobuf inside network worker today)
//! - Intent temporary ban / Zenoh peer ACL
//! - Unlimited Ed25519 cost on unbound steady floods (no sub bucket until bind)

mod finite;
mod host_control;
mod host_ipc;
mod network_worker;
mod pose_delta;
mod sim;
mod wire_tick;

use host_server::{intent_rate, metrics, room_token, session};

use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Mutex poison must not degrade into empty Presence / silent skip (x002e / P3-8).
fn lock_or_fatal<'a, T>(mutex: &'a Mutex<T>, label: &str) -> MutexGuard<'a, T> {
    mutex.lock().unwrap_or_else(|poisoned| {
        log::error!("{label} mutex poisoned: {poisoned}; aborting host-server");
        std::process::exit(78);
    })
}

use alchemy_protocol::decode_limits::{check_input, check_payload_bytes, WireClass};
use alchemy_protocol::wire_crypto::{
    generate_signing_key, public_key_bytes, sign_frame, sign_presence, SigningKey,
};
use alchemy_protocol::{
    frame_key, host_control_key, input_buttons, input_key, liveliness_binding_from_key,
    liveliness_wildcard, manifest_key, presence_key, snapshot_key, user_root_slots_from_entries,
    validate_room_id, InputEvent, LocomotionConfig, LocomotionMode, Message, PresenceAssignment,
    PresenceState, RenderFrame, DEFAULT_MOVE_SPEED, PROTOCOL_PIN, USER_ROOT_SLOTS_MAX,
};
use anyhow::{Context, Result};
use finite::input_event_is_finite;
use intent_rate::IntentRateLimiter;
use network_worker::{
    spawn_network_worker, FrameSample, ManifestJob, ManifestReason, NetworkWorkerConfig, Pose,
    PresenceJob,
};
use room_token::{
    default_auth_url, earliest_hard_expire, ensure_sealed_manifest, fetch_room_scene,
    publish_allowed_at, spawn_credential_worker, CredentialWorkerConfig, HostCredentialChain,
    HostPublishSnapshot, ManifestSealCache, RoomTokenVerifier,
};
use session::SessionTable;
use sim::{LocoMode, PhysicsWorld, USER_ROOT_ENTITY_FIRST};
use zenoh::bytes::ZBytes;
use zenoh::sample::SampleKind;
use zenoh::Wait;

const PHYSICS_HZ: u64 = 60;
const NET_HZ: u64 = 30;
/// Publish a network frame every N physics steps (60/30 = 2).
const PHYSICS_PER_NET: u64 = PHYSICS_HZ / NET_HZ;
const DEFAULT_ZENOH_CONNECT: &str = "tcp/127.0.0.1:7447";
const DEFAULT_WORLD_URL: &str = "http://127.0.0.1:4003";
const DEFAULT_ROOM_ID: &str = "local";
/// Pose change below this is treated as static (sleep / omit from upserts).
/// Republish Manifest so late joiners still see blob refs (still ≪ frame rate).
const MANIFEST_REPUBLISH: Duration = Duration::from_secs(5);
/// Republish Presence for late joiners (same cadence as Manifest).
const PRESENCE_REPUBLISH: Duration = Duration::from_secs(5);
/// Intent silence before reclaiming a Presence slot (client INPUT_HZ=30).
const PRESENCE_SLOT_TTL: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, Default)]
struct PoseOverride {
    x: f32,
    y: f32,
    z: f32,
    active: bool,
    grab: bool,
    grab_entity_id: u32,
    aim_x: f32,
    aim_y: f32,
    aim_z: f32,
}

/// Grace before confirming liveliness Delete reclaim (x0022).
///
/// Covers Intent↔Delete reorder after Room Token refresh (distinct Zenoh keys).
/// Limits: only cancels when `session_jti` **rotates**; same-jti Intent during grace does not
/// clear a pending Delete (legitimate disconnect stays ~+1.5s). If Intent(new) arrives **after**
/// this window, Delete(old) can still reclaim (extreme delay / partition — out of guarantee).
const LIVELINESS_RECLAIM_GRACE: Duration = Duration::from_millis(1_500);

/// Per-sub input slots (x0015) + Leave / jti-bound liveliness / Intent TTL reclaim (x0022).
///
/// Storage is `[T; USER_ROOT_SLOTS_MAX]` + [`slot_count`] (Manifest-derived `n`, x004d).
/// `by_slot` stays `Copy` so the physics tick can snapshot without a heap alloc.
#[derive(Debug)]
struct PresenceIntents {
    by_slot: [PoseOverride; USER_ROOT_SLOTS_MAX],
    /// Occupied slots: Some(sub) when assigned. Only indices `0..slot_count` are live.
    occupied_sub: [Option<String>; USER_ROOT_SLOTS_MAX],
    /// Last verified Room Token `jti` per occupied slot (x0022). None until first verify.
    session_jti: [Option<String>; USER_ROOT_SLOTS_MAX],
    /// Scheduled liveliness reclaim: `(deleted_jti, confirm_after)`. Cancelled if `session_jti` rotates.
    pending_liveliness: [Option<(String, Instant)>; USER_ROOT_SLOTS_MAX],
    /// Last accepted Intent time per slot (None when free).
    last_input: [Option<Instant>; USER_ROOT_SLOTS_MAX],
    /// Manifest-derived live capacity (`1..=USER_ROOT_SLOTS_MAX`).
    slot_count: usize,
    dropped_full: u64,
    /// Bumped on assignment or reclaim (PresenceState.revision).
    revision: u64,
    /// `(entity_id, sub)` freed off the physics thread (Leave / liveliness); drained each tick.
    pending_removes: Vec<(u32, String)>,
    /// Set on new slot assign or session `jti` rotate; main loop takes for Manifest join assist (x003c).
    /// Not set on reclaim/leave (those bump `revision` but are not joiners).
    manifest_join_assist: bool,
}

impl PresenceIntents {
    fn with_slots(n: usize) -> Self {
        assert!(
            (1..=USER_ROOT_SLOTS_MAX).contains(&n),
            "Presence slots must be 1..={USER_ROOT_SLOTS_MAX}, got {n}"
        );
        Self {
            by_slot: [PoseOverride::default(); USER_ROOT_SLOTS_MAX],
            occupied_sub: std::array::from_fn(|_| None),
            session_jti: std::array::from_fn(|_| None),
            pending_liveliness: std::array::from_fn(|_| None),
            last_input: [None; USER_ROOT_SLOTS_MAX],
            slot_count: n,
            dropped_full: 0,
            revision: 0,
            pending_removes: Vec::new(),
            manifest_join_assist: false,
        }
    }

    fn slot_count(&self) -> usize {
        self.slot_count
    }

    fn take_manifest_join_assist(&mut self) -> bool {
        let v = self.manifest_join_assist;
        self.manifest_join_assist = false;
        v
    }

    fn assign_or_lookup(&mut self, sub: &str, jti: &str, now: Instant) -> Option<usize> {
        let n = self.slot_count;
        for i in 0..n {
            if self.occupied_sub[i].as_deref() == Some(sub) {
                self.last_input[i] = Some(now);
                // Cancel pending Delete only on jti rotate (refresh). Same-jti Intent leaves
                // the schedule so crash/undeclare still confirms after grace.
                let jti_rotated = self.session_jti[i].as_deref() != Some(jti);
                if jti_rotated {
                    self.pending_liveliness[i] = None;
                    // Reconnect / Token refresh: same sub, new jti — late subscribe needs Manifest.
                    self.manifest_join_assist = true;
                }
                self.session_jti[i] = Some(jti.to_string());
                return Some(i);
            }
        }
        for i in 0..n {
            if self.occupied_sub[i].is_none() {
                self.occupied_sub[i] = Some(sub.to_string());
                self.session_jti[i] = Some(jti.to_string());
                self.pending_liveliness[i] = None;
                self.last_input[i] = Some(now);
                self.revision = self.revision.saturating_add(1);
                self.manifest_join_assist = true;
                log::info!(
                    "presence: assigned sub={sub} → user_root entity {} (revision={})",
                    PhysicsWorld::user_root_entity_id(i),
                    self.revision
                );
                return Some(i);
            }
        }
        None
    }

    /// Free one occupied slot. Returns `(entity_id, sub)`.
    fn free_slot_at(&mut self, i: usize, reason: &str) -> (u32, String) {
        let sub = self.occupied_sub[i].take().unwrap_or_default();
        self.session_jti[i] = None;
        self.pending_liveliness[i] = None;
        self.last_input[i] = None;
        self.by_slot[i] = PoseOverride::default();
        self.revision = self.revision.saturating_add(1);
        let entity_id = PhysicsWorld::user_root_entity_id(i);
        log::info!(
            "presence: reclaimed slot={i} entity={entity_id} sub={sub} reason={reason} (revision={})",
            self.revision
        );
        (entity_id, sub)
    }

    /// Immediate reclaim for verified Leave / session (queues `pending_removes`).
    fn reclaim_sub(&mut self, sub: &str, reason: &str) -> Option<(u32, String)> {
        for i in 0..self.slot_count() {
            if self.occupied_sub[i].as_deref() == Some(sub) {
                let freed = self.free_slot_at(i, reason);
                self.pending_removes.push(freed.clone());
                return Some(freed);
            }
        }
        None
    }

    /// Schedule reclaim when Delete `jti` matches the bound session (x0022).
    ///
    /// Does **not** free immediately: grace lets a refresh Intent with a new `jti` cancel the
    /// pending Delete when liveliness arrives before the Intent (no cross-key order).
    fn schedule_liveliness_reclaim(&mut self, sub: &str, jti: &str, now: Instant) -> bool {
        for i in 0..self.slot_count() {
            if self.occupied_sub[i].as_deref() != Some(sub) {
                continue;
            }
            match self.session_jti[i].as_deref() {
                Some(bound) if bound == jti => {
                    let confirm_at = now + LIVELINESS_RECLAIM_GRACE;
                    self.pending_liveliness[i] = Some((jti.to_string(), confirm_at));
                    log::info!(
                        "liveliness Delete scheduled: sub={sub} jti={jti} confirm_in_ms={}",
                        LIVELINESS_RECLAIM_GRACE.as_millis()
                    );
                    return true;
                }
                Some(bound) => {
                    log::warn!(
                        "liveliness Delete ignored: sub={sub} jti={jti} bound_jti={bound} (mismatch)"
                    );
                    return false;
                }
                None => {
                    log::warn!(
                        "liveliness Delete ignored: sub={sub} jti={jti} (no verified session yet)"
                    );
                    return false;
                }
            }
        }
        false
    }

    /// Confirm scheduled liveliness reclaim if grace elapsed and `jti` still bound.
    fn confirm_liveliness_reclaims(&mut self, now: Instant) -> Vec<(u32, String)> {
        let mut freed = Vec::new();
        for i in 0..self.slot_count() {
            let Some((jti, confirm_at)) = self.pending_liveliness[i].clone() else {
                continue;
            };
            if now < confirm_at {
                continue;
            }
            self.pending_liveliness[i] = None;
            match self.session_jti[i].as_deref() {
                Some(bound) if bound == jti && self.occupied_sub[i].is_some() => {
                    freed.push(self.free_slot_at(i, "liveliness"));
                }
                Some(bound) => {
                    log::info!(
                        "liveliness Delete cancelled after grace: deleted_jti={jti} bound_jti={bound}"
                    );
                }
                None => {
                    log::info!(
                        "liveliness Delete cancelled after grace: deleted_jti={jti} (slot free)"
                    );
                }
            }
        }
        freed
    }

    /// Drop slots with no Intent for `ttl`. Returns freed `(entity_id, sub)`.
    fn reclaim_stale(&mut self, now: Instant, ttl: Duration) -> Vec<(u32, String)> {
        let mut freed = Vec::new();
        for i in 0..self.slot_count() {
            if self.occupied_sub[i].is_none() {
                continue;
            }
            let stale = match self.last_input[i] {
                Some(t) => now.saturating_duration_since(t) >= ttl,
                None => true,
            };
            if !stale {
                continue;
            }
            freed.push(self.free_slot_at(i, "ttl"));
        }
        freed
    }

    fn drain_reclaims(&mut self, now: Instant, ttl: Duration) -> Vec<(u32, String)> {
        let mut freed = self.reclaim_stale(now, ttl);
        freed.extend(self.confirm_liveliness_reclaims(now));
        freed.append(&mut self.pending_removes);
        freed
    }

    fn to_presence_state(&self, host_generation: u64) -> PresenceState {
        let mut assignments = Vec::with_capacity(self.slot_count);
        for i in 0..self.slot_count {
            if let Some(sub) = &self.occupied_sub[i] {
                assignments.push(PresenceAssignment {
                    sub: sub.clone(),
                    entity_id: PhysicsWorld::user_root_entity_id(i),
                });
            }
        }
        PresenceState {
            revision: self.revision,
            assignments,
            protocol_pin: PROTOCOL_PIN.to_string(),
            host_sig: vec![],
            host_generation,
        }
    }
}

fn zenoh_connect_endpoint() -> String {
    std::env::var("ZENOH_CONNECT").unwrap_or_else(|_| DEFAULT_ZENOH_CONNECT.to_string())
}

fn open_zenoh() -> Result<zenoh::Session> {
    zenoh_connect::open_client(&zenoh_connect_endpoint())
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
    }
}

fn moving_pose_from_sim(world: &PhysicsWorld) -> Option<(u32, Pose)> {
    let id = world.mesh_entity_id()?;
    let (t, q) = world.moving_pose();
    Some((
        id,
        pose_with_trs(t, q, world.scale_for(id), world.color_for(id)),
    ))
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

/// Physics-facing gate fields (no `manifest_bytes` clone).
#[derive(Clone, Copy, Debug)]
struct PublishGateView {
    publish_allowed: bool,
    host_generation: u64,
}

fn read_publish_gate(snapshot: &RwLock<HostPublishSnapshot>, now: Instant) -> PublishGateView {
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

fn spawn_input_gate(
    session: zenoh::Session,
    input_key_expr: String,
    verifier: Arc<RoomTokenVerifier>,
    presence: Arc<Mutex<PresenceIntents>>,
    sessions: Arc<Mutex<SessionTable>>,
    rate_limiter: Arc<Mutex<IntentRateLimiter>>,
) -> Result<()> {
    let subscriber = session
        .declare_subscriber(input_key_expr.clone())
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_subscriber input failed: {e}"))?;

    thread::Builder::new()
        .name("zenoh-input".into())
        .spawn(move || {
            let _session = session;
            log::info!(
                "subscribed to {input_key_expr} (session Ed25519 + optional Room Token bind + is_finite)"
            );
            let mut accepted: u64 = 0;
            let mut leaves: u64 = 0;
            let mut dropped_auth: u64 = 0;
            let mut dropped_finite: u64 = 0;
            let mut dropped_limits: u64 = 0;
            let mut dropped_rate: u64 = 0;
            let mut last_log = Instant::now();
            while let Ok(sample) = subscriber.recv() {
                // x003b: early drops must still reach the interval aggregate log below.
                'sample: {
                let bytes: ZBytes = sample.payload().clone();
                // x003b: length before contiguous copy (`to_bytes`).
                if let Err(err) = check_payload_bytes(WireClass::Input, bytes.len()) {
                    dropped_limits += 1;
                    log::warn!("drop InputEvent: {err}");
                    break 'sample;
                }
                let payload = bytes.to_bytes();
                match InputEvent::decode(payload.as_ref()) {
                    Ok(event) => {
                        if let Err(err) = check_input(&event) {
                            dropped_limits += 1;
                            log::warn!("drop InputEvent: {err}");
                            break 'sample;
                        }
                        let now = Instant::now();
                        let is_bind = !event.room_token.is_empty();
                        // x0040: Ed25519 first; rate never runs on forged sig.
                        // Bind: rates alone → JWT/sessions → rates again for forget.
                        // Steady: sessions (evaluate) → rates → sessions (commit).
                        // Leave/reclaim later: sessions then rates (never rates→sessions).
                        if let Err(err) = SessionTable::verify_intent_sig(&event) {
                            dropped_auth += 1;
                            log::warn!("drop InputEvent: session/auth: {err:#}");
                            break 'sample;
                        }
                        let auth = if is_bind {
                            // rates-only: atomic global+per-pub (Leave NOT exempt).
                            {
                                let mut rates = lock_or_fatal(&rate_limiter, "intent_rate");
                                if !rates.allow_bind(&event.session_pub, now) {
                                    dropped_rate += 1;
                                    break 'sample;
                                }
                            }
                            let evaluated = {
                                let table = lock_or_fatal(&sessions, "sessions");
                                table.evaluate_bind(&event, verifier.as_ref())
                            };
                            match evaluated {
                                Ok(pending) => {
                                    let (sub, jti, caps, removed) = {
                                        let mut table = lock_or_fatal(&sessions, "sessions");
                                        // `removed` excludes the pub just bound.
                                        table.commit_bind(&event, pending)
                                    };
                                    // sessions then rates (GC other pubs only).
                                    let mut rates = lock_or_fatal(&rate_limiter, "intent_rate");
                                    rates.forget_session_pubs(&removed);
                                    Ok((sub, jti, caps))
                                }
                                Err(err) => {
                                    let mut rates = lock_or_fatal(&rate_limiter, "intent_rate");
                                    rates.forget_session_pub(&event.session_pub);
                                    Err(err)
                                }
                            }
                        } else {
                            // sessions → rates → sessions (rate drop skips commit).
                            let evaluated = {
                                let table = lock_or_fatal(&sessions, "sessions");
                                table.evaluate_steady(&event)
                            };
                            match evaluated {
                                Ok(pending) => {
                                    {
                                        let mut rates =
                                            lock_or_fatal(&rate_limiter, "intent_rate");
                                        if !rates.allow_steady(event.leave, &pending.sub, now)
                                        {
                                            dropped_rate += 1;
                                            break 'sample;
                                        }
                                    }
                                    let mut table = lock_or_fatal(&sessions, "sessions");
                                    table.commit_steady(&event, pending)
                                }
                                Err(err) => Err(err),
                            }
                        };
                        match auth {
                            Ok((sub, jti, caps)) => {
                                let mut event = event;
                                alchemy_protocol::sim_capability::mask_intent(caps, &mut event);
                                if event.leave {
                                    let mut guard = lock_or_fatal(&presence, "presence");
                                    if guard.reclaim_sub(&sub, "leave").is_some() {
                                        leaves += 1;
                                    }
                                    drop(guard);
                                    let removed = {
                                        let mut table = lock_or_fatal(&sessions, "sessions");
                                        table.remove_by_sub(&sub)
                                    };
                                    let mut rates = lock_or_fatal(&rate_limiter, "intent_rate");
                                    rates.forget_sub(&sub);
                                    rates.forget_session_pubs(&removed);
                                } else if !input_event_is_finite(&event) {
                                    dropped_finite += 1;
                                    log::warn!(
                                        "drop InputEvent: non-finite move_*=({:.3},{:.3},{:.3})",
                                        event.move_x,
                                        event.move_y,
                                        event.move_z
                                    );
                                } else {
                                    let mut guard = lock_or_fatal(&presence, "presence");
                                    match guard.assign_or_lookup(&sub, &jti, Instant::now()) {
                                        Some(slot) => {
                                            let overlay = &mut guard.by_slot[slot];
                                            overlay.x = event.move_x;
                                            overlay.y = event.move_y;
                                            overlay.z = event.move_z;
                                            overlay.active = true;
                                            overlay.grab =
                                                (event.buttons & input_buttons::GRAB) != 0;
                                            overlay.grab_entity_id = event.grab_entity_id;
                                            overlay.aim_x = event.aim_x;
                                            overlay.aim_y = event.aim_y;
                                            overlay.aim_z = event.aim_z;
                                            accepted += 1;
                                        }
                                        None => {
                                            guard.dropped_full += 1;
                                            let max = guard.slot_count();
                                            log::warn!(
                                                "drop InputEvent: presence full (max {max}); sub={sub}"
                                            );
                                        }
                                    }
                                }
                            }
                            Err(err) => {
                                dropped_auth += 1;
                                log::warn!("drop InputEvent: session/auth: {err:#}");
                            }
                        }
                    }
                    Err(err) => {
                        dropped_auth += 1;
                        log::warn!("drop InputEvent: decode failed: {err}");
                    }
                }
                } // 'sample
                if last_log.elapsed() >= Duration::from_secs(5) {
                    let dropped_full = lock_or_fatal(&presence, "presence").dropped_full;
                    log::info!(
                        "input gate: accepted={accepted} leaves={leaves} dropped_auth={dropped_auth} dropped_non_finite={dropped_finite} dropped_limits={dropped_limits} dropped_rate={dropped_rate} dropped_full={dropped_full} (last ~5s)"
                    );
                    accepted = 0;
                    leaves = 0;
                    dropped_auth = 0;
                    dropped_finite = 0;
                    dropped_limits = 0;
                    dropped_rate = 0;
                    last_log = Instant::now();
                }
            }
        })
        .context("spawn input thread")?;
    Ok(())
}

fn spawn_snapshot_queryable(
    session: zenoh::Session,
    snapshot_key_expr: String,
    snapshot: Arc<Mutex<RenderFrame>>,
    publish_snapshot: Arc<RwLock<HostPublishSnapshot>>,
    host_key: SigningKey,
) -> Result<()> {
    let queryable = session
        .declare_queryable(snapshot_key_expr.clone())
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_queryable snapshot failed: {e}"))?;

    thread::Builder::new()
        .name("zenoh-snapshot".into())
        .spawn(move || {
            log::info!("queryable ready on {snapshot_key_expr}");
            while let Ok(query) = queryable.recv() {
                // x003d: hard-expiry fail-closed includes snapshot (same as Frame).
                let gate = read_publish_gate(&publish_snapshot, Instant::now());
                if !gate.publish_allowed {
                    if let Err(err) = query
                        .reply_err("snapshot unavailable (credential hard expiry)")
                        .wait()
                    {
                        log::warn!("snapshot reply_err failed: {err}");
                    }
                    continue;
                }
                let mut frame = lock_or_fatal(&snapshot, "snapshot").clone();
                // Contract: empty = not ready. Do not sign an empty frame as success.
                if frame.upserts.is_empty() {
                    if let Err(err) = query.reply_err("snapshot not ready (no poses yet)").wait() {
                        log::warn!("snapshot reply_err failed: {err}");
                    }
                    continue;
                }
                sign_frame(&host_key, &mut frame);
                let key = snapshot_key_expr.clone();
                if let Err(err) = query.reply(key, frame.encode_to_vec()).wait() {
                    log::warn!("snapshot reply failed: {err}");
                }
            }
        })
        .context("spawn snapshot thread")?;
    Ok(())
}

fn spawn_liveliness_watcher(
    session: zenoh::Session,
    room_id: &str,
    presence: Arc<Mutex<PresenceIntents>>,
) -> Result<()> {
    let key = liveliness_wildcard(room_id);
    let subscriber = session
        .liveliness()
        .declare_subscriber(key.clone())
        .history(true)
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_subscriber liveliness failed: {e}"))?;

    thread::Builder::new()
        .name("zenoh-liveliness".into())
        .spawn(move || {
            let _session = session;
            log::info!("subscribed to liveliness {key}");
            while let Ok(sample) = subscriber.recv() {
                if sample.kind() != SampleKind::Delete {
                    continue;
                }
                let key_str = sample.key_expr().as_str();
                let Some((sub, jti)) = liveliness_binding_from_key(key_str) else {
                    log::warn!("liveliness Delete: unparseable key={key_str}");
                    continue;
                };
                let mut guard = lock_or_fatal(&presence, "presence");
                let _ = guard.schedule_liveliness_reclaim(sub, jti, Instant::now());
            }
        })
        .context("spawn liveliness thread")?;
    Ok(())
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let world_url = std::env::var("WORLD_URL").unwrap_or_else(|_| DEFAULT_WORLD_URL.to_string());
    let room_id_raw = std::env::var("ROOM_ID").unwrap_or_else(|_| DEFAULT_ROOM_ID.to_string());
    let room_id = validate_room_id(&room_id_raw)
        .map_err(|e| anyhow::anyhow!("invalid ROOM_ID={room_id_raw:?}: {e}"))?
        .to_string();
    let auth_url = default_auth_url();
    let manifest_key_expr = manifest_key(&room_id);
    let frame_key_expr = frame_key(&room_id);
    let input_key_expr = input_key(&room_id);
    let presence_key_expr = presence_key(&room_id);
    let snapshot_key_expr = snapshot_key(&room_id);
    let host_control_key_expr = host_control_key(&room_id);
    let liveliness_key_expr = liveliness_wildcard(&room_id);
    let host_control_mode = if host_control::zenoh_control_enabled() {
        "zenoh-opt-in"
    } else {
        "disabled"
    };

    let host_signing_key = generate_signing_key();
    let host_pub = public_key_bytes(&host_signing_key);

    log::info!(
        "host-server x004d starting; protocol_pin={PROTOCOL_PIN} physics_hz={PHYSICS_HZ} net_hz={NET_HZ} manifest={manifest_key_expr} frame={frame_key_expr} snapshot={snapshot_key_expr} input={input_key_expr} presence={presence_key_expr} liveliness={liveliness_key_expr} host_control={host_control_key_expr}({host_control_mode}) world={world_url} auth={auth_url} room={room_id} user_root_band={USER_ROOT_ENTITY_FIRST}..{} presence_ttl_secs={}",
        USER_ROOT_ENTITY_FIRST + USER_ROOT_SLOTS_MAX as u32 - 1,
        PRESENCE_SLOT_TTL.as_secs()
    );

    let host_metrics = Arc::new(metrics::HostMetrics::default());
    metrics::maybe_spawn_from_env(Arc::clone(&host_metrics));
    let verifier = Arc::new(RoomTokenVerifier::new(&world_url, &room_id));
    verifier.set_metrics(Arc::clone(&host_metrics));
    verifier.refresh_jwks().context("initial JWKS fetch")?;
    verifier.spawn_background_refresh();
    let creds = HostCredentialChain::bootstrap(
        &auth_url,
        &world_url,
        &room_id,
        &host_pub,
        verifier.as_ref(),
    )
    .context("bootstrap auth → host Room Token → HostAttestation")?;
    log::info!(
        "auth JWT + host Room Token (role=host) + HostAttestation acquired; credential worker refresh enabled (fail-closed on hard expiry)"
    );

    // Scene is fetched once at boot (x0027 Non-guarantee: no re-fetch on world redefinition).
    let room_scene = fetch_room_scene(&world_url, &room_id, &creds.host_token)
        .context("fetch world DemoScene (GET …/scene)")?;
    // x0039: scene body is sealed against world DB — local blake3 override cannot get a seal.
    if let Ok(override_blake3) = std::env::var("ALCHEMY_BLOB_BLAKE3") {
        if !override_blake3.is_empty() {
            anyhow::bail!(
                "ALCHEMY_BLOB_BLAKE3 is incompatible with canonical ManifestSeal (x0039); unset it or change the world DemoScene"
            );
        }
    }
    let user_root_slots = user_root_slots_from_entries(&room_scene.entries)
        .context("invalid user_root entries in world scene (x004d)")?;
    log::info!(
        "world scene loaded revision={} entries={} user_root_slots={user_root_slots} (band max={USER_ROOT_SLOTS_MAX})",
        room_scene.revision,
        room_scene.entries.len()
    );

    let presence = Arc::new(Mutex::new(PresenceIntents::with_slots(user_root_slots)));
    let sessions = Arc::new(Mutex::new(SessionTable::default()));
    let intent_rates = Arc::new(Mutex::new(IntentRateLimiter::new()));
    let snapshot_cache = Arc::new(Mutex::new(RenderFrame {
        tick: 0,
        host_time_ms: 0,
        upserts: vec![],
        removes: vec![],
        host_sig: vec![],
    }));

    let session = open_zenoh().context("open zenoh session")?;
    spawn_input_gate(
        session.clone(),
        input_key_expr,
        Arc::clone(&verifier),
        Arc::clone(&presence),
        Arc::clone(&sessions),
        Arc::clone(&intent_rates),
    )?;
    spawn_liveliness_watcher(session.clone(), &room_id, Arc::clone(&presence))?;
    host_control::spawn_listener(session.clone(), &room_id)?;
    host_ipc::spawn_listener().context("launcher IPC listener")?;

    let manifest_pub = session
        .declare_publisher(manifest_key_expr)
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_publisher manifest failed: {e}"))?;
    let presence_pub = session
        .declare_publisher(presence_key_expr)
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_publisher presence failed: {e}"))?;
    let frame_pub = session
        .declare_publisher(frame_key_expr)
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_publisher frame failed: {e}"))?;

    let mut empty_presence = PresenceState {
        revision: 0,
        assignments: vec![],
        protocol_pin: PROTOCOL_PIN.to_string(),
        host_sig: vec![],
        host_generation: creds.host_generation,
    };
    sign_presence(&host_signing_key, &mut empty_presence);
    presence_pub
        .put(empty_presence.encode_to_vec())
        .wait()
        .map_err(|e| anyhow::anyhow!("presence publish failed: {e}"))?;
    log::info!(
        "published PresenceState revision=0 assignments=0 generation={} (signed)",
        creds.host_generation
    );
    let mut last_presence_rev = 0u64;
    let mut last_presence = Instant::now();

    let mut seal_cache = ManifestSealCache::empty();
    let manifest = ensure_sealed_manifest(
        &room_id,
        &room_scene,
        &host_pub,
        &creds,
        &world_url,
        &mut seal_cache,
    )
    .context("initial ManifestSeal")?;
    let loco = manifest.locomotion.unwrap_or(LocomotionConfig {
        mode: LocomotionMode::Noclip.into(),
        move_speed: DEFAULT_MOVE_SPEED,
    });
    let loco_mode = LocoMode::from_proto(loco.mode);
    let move_speed = if loco.move_speed > 0.0 {
        loco.move_speed
    } else {
        DEFAULT_MOVE_SPEED
    };

    let hard_expire =
        earliest_hard_expire(&[creds.hard_expire_at(), Some(seal_cache.hard_expire_at)]);
    let publish_snapshot = Arc::new(RwLock::new(HostPublishSnapshot::from_sealed(
        &manifest,
        hard_expire,
        0,
    )));
    spawn_snapshot_queryable(
        session.clone(),
        snapshot_key_expr,
        Arc::clone(&snapshot_cache),
        Arc::clone(&publish_snapshot),
        host_signing_key.clone(),
    )?;
    spawn_credential_worker(
        Arc::clone(&publish_snapshot),
        creds,
        seal_cache,
        Arc::clone(&verifier),
        CredentialWorkerConfig {
            auth_url: auth_url.clone(),
            world_url: world_url.clone(),
            room_id: room_id.clone(),
            host_pub: host_pub.clone(),
            scene: room_scene.clone(),
        },
    );
    log::info!("credential worker spawned (HTTP refresh off physics thread)");

    let mut manifest_bytes = manifest.encode_to_vec();
    let mut manifest_revision = manifest.revision;
    let mut manifest_epoch = 0u64;
    manifest_pub
        .put(manifest_bytes.clone())
        .wait()
        .map_err(|e| anyhow::anyhow!("manifest publish failed: {e}"))?;
    log::info!(
        "published SceneManifest revision={} entries={} loco={:?} speed={move_speed} (attested+sealed)",
        manifest_revision,
        manifest.entries.len(),
        loco_mode
    );
    let mut last_manifest = Instant::now();
    let mut last_publish_denied_log = Instant::now() - Duration::from_secs(60);

    // x0042: move publishers to network worker (boot puts above stay on main).
    let net = spawn_network_worker(NetworkWorkerConfig {
        manifest_pub,
        presence_pub,
        frame_pub,
        snapshot_cache: Arc::clone(&snapshot_cache),
        host_key: host_signing_key.clone(),
    })?;
    let outbound = net.hub;
    let publish_stats = net.stats;

    let mut physics = PhysicsWorld::from_entries(&room_scene.entries, loco_mode, move_speed);
    debug_assert_eq!(physics.user_root_slot_count(), user_root_slots);
    let grabbable_n = physics.boxes.grabbable.iter().filter(|&&g| g).count();
    let last_root = USER_ROOT_ENTITY_FIRST + physics.user_root_slot_count() as u32 - 1;
    log::info!(
        "Rapier world ready (mesh={:?} + user_roots {USER_ROOT_ENTITY_FIRST}..{last_root} + boxes={} grabbable={grabbable_n} mode={:?})",
        physics.mesh_entity_id(),
        physics.boxes.len(),
        physics.loco_mode()
    );

    let physics_period = Duration::from_nanos(1_000_000_000 / PHYSICS_HZ);
    let dt = 1.0 / PHYSICS_HZ as f32;
    let mut next_physics = Instant::now();
    let mut physics_steps: u64 = 0;
    let mut last_log = Instant::now();
    let mut physics_steps_logged: u64 = 0;
    // Whether the previous tick was allowed to publish (for resume catch-up).
    let mut was_publishing = true;

    loop {
        if host_ipc::shutdown_requested() {
            log::info!("launcher IPC shutdown requested — leaving main loop (exit 0)");
            break;
        }
        let scheduled = next_physics;
        let now = Instant::now();
        if now < scheduled {
            std::thread::sleep(scheduled - now);
        }
        let started = Instant::now();
        let late_ms = started.saturating_duration_since(scheduled).as_millis() as u64;
        host_metrics.set_tick_lateness_ms(late_ms);
        next_physics = scheduled + physics_period;

        // x003d: credentials / seal live on the worker; physics reads gate + hard expiry.
        let pub_snap = read_publish_gate(&publish_snapshot, now);
        if !pub_snap.publish_allowed {
            // Keep `last_sent` as last wire view so resume can synthesize removes.
            was_publishing = false;
            if last_publish_denied_log.elapsed() >= Duration::from_secs(5) {
                log::warn!(
                    "publish paused (credential hard expiry or gate); physics continues, waiting for refresh"
                );
                last_publish_denied_log = Instant::now();
            }
        } else if let Some((epoch, rev, bytes)) =
            take_manifest_if_newer(&publish_snapshot, manifest_epoch)
        {
            manifest_bytes = bytes;
            manifest_revision = rev;
            manifest_epoch = epoch;
            let _ = outbound.submit_manifest(ManifestJob {
                bytes: manifest_bytes.clone(),
                revision: manifest_revision,
                epoch: manifest_epoch,
                reason: ManifestReason::Epoch,
            });
            last_manifest = Instant::now();
        } else if last_manifest.elapsed() >= MANIFEST_REPUBLISH {
            let _ = outbound.submit_manifest(ManifestJob {
                bytes: manifest_bytes.clone(),
                revision: manifest_revision,
                epoch: manifest_epoch,
                reason: ManifestReason::Republish,
            });
            last_manifest = Instant::now();
        }
        // Stays true across non-net ticks until catch-up runs on a net tick.
        let resume_catchup = pub_snap.publish_allowed && !was_publishing;

        let tick_now = Instant::now();
        let host_generation = pub_snap.host_generation;
        let (by_slot, occupied, slot_count, presence_rev, presence_state, reclaimed, join_assist) = {
            let mut g = lock_or_fatal(&presence, "presence");
            let freed = g.drain_reclaims(tick_now, PRESENCE_SLOT_TTL);
            let join_assist = g.take_manifest_join_assist();
            (
                g.by_slot, // Copy — avoid per-tick Vec heap alloc (x004d review)
                g.occupied_sub.clone(),
                g.slot_count,
                g.revision,
                g.to_presence_state(host_generation),
                freed,
                join_assist,
            )
        };

        for (entity_id, sub) in &reclaimed {
            let slot = (entity_id - USER_ROOT_ENTITY_FIRST) as usize;
            physics.release_slot(slot);
            // Do not mutate `last_sent` here: it is the last wire view. Removals are
            // applied when removes are published, or wholesale on resume catch-up.
            if !sub.is_empty() {
                let removed = lock_or_fatal(&sessions, "sessions").remove_by_sub(sub);
                let mut rates = lock_or_fatal(&intent_rates, "intent_rate");
                rates.forget_sub(sub);
                rates.forget_session_pubs(&removed);
            }
        }

        if pub_snap.publish_allowed
            && (presence_rev != last_presence_rev || last_presence.elapsed() >= PRESENCE_REPUBLISH)
        {
            let republish = presence_rev == last_presence_rev;
            let _ = outbound.submit_presence(PresenceJob {
                state: presence_state,
                republish,
            });
            last_presence_rev = presence_rev;
            last_presence = Instant::now();
        }

        // New assign or session jti rotate (reconnect) — not reclaim/leave (x003c).
        if pub_snap.publish_allowed && join_assist {
            let _ = outbound.submit_manifest(ManifestJob {
                bytes: manifest_bytes.clone(),
                revision: manifest_revision,
                epoch: manifest_epoch,
                reason: ManifestReason::JoinAssist,
            });
            last_manifest = Instant::now();
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
                physics.apply_user_intent(slot, overlay.x, overlay.y, overlay.z, dt);
            }
            physics.apply_grab(
                slot,
                overlay.grab,
                overlay.grab_entity_id,
                (overlay.aim_x, overlay.aim_y, overlay.aim_z),
            );
        }
        physics.step();
        physics_steps = physics_steps.wrapping_add(1);
        physics_steps_logged += 1;

        let remove_ids: Vec<u32> = reclaimed.iter().map(|(id, _)| *id).collect();

        if !physics_steps.is_multiple_of(PHYSICS_PER_NET) {
            // Still flush removes promptly when reclaim happens between net ticks.
            // Resume catch-up (with synthesized removes) waits for a net tick below.
            if !remove_ids.is_empty()
                && pub_snap.publish_allowed
                && !resume_catchup
                && outbound.submit_frame(FrameSample {
                    poses: vec![],
                    removes: remove_ids,
                    resume_catchup: false,
                    publish_allowed: true,
                    removes_only: true,
                    host_time_ms: host_time_ms(),
                })
            {
                publish_stats
                    .coalesce_frames
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            continue;
        }

        // Pose sample only on net ticks (delta/encode/put live on zenoh-publish).
        let mut all_poses: Vec<(u32, Pose)> =
            Vec::with_capacity(1 + physics.user_root_slot_count() + physics.boxes.len());
        if let Some((id, pose)) = moving_pose_from_sim(&physics) {
            all_poses.push((id, pose));
        }
        for (slot, occupied_sub) in occupied.iter().enumerate().take(slot_count) {
            if occupied_sub.is_some() {
                all_poses.push((
                    PhysicsWorld::user_root_entity_id(slot),
                    user_root_pose_from_sim(&physics, slot),
                ));
            }
        }
        for i in 0..physics.boxes.len() {
            all_poses.push((physics.boxes.entity_ids[i], box_pose_from_sim(&physics, i)));
        }

        if outbound.submit_frame(FrameSample {
            poses: all_poses,
            removes: remove_ids,
            resume_catchup,
            publish_allowed: pub_snap.publish_allowed,
            removes_only: false,
            host_time_ms: host_time_ms(),
        }) {
            publish_stats
                .coalesce_frames
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        // Catch-up flag is sticky in `coalesce_frame`; clearing local gate latch here is
        // safe even if this sample merges with a later `resume_catchup: false` tick.
        if resume_catchup && pub_snap.publish_allowed {
            was_publishing = true;
        }

        if last_log.elapsed() >= Duration::from_secs(1) {
            let (mx, my, mz) = physics.moving_translation();
            let mut roots = String::new();
            for (slot, occupied_sub) in occupied.iter().enumerate().take(slot_count) {
                if occupied_sub.is_none() {
                    continue;
                }
                let (ux, uy, uz) = physics.user_root_translation(slot);
                let eid = PhysicsWorld::user_root_entity_id(slot);
                let grab = physics
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
                publish_stats.snapshot_and_reset();
            log::info!(
                "physics: steps={physics_steps_logged} coin=({mx:.3},{my:.3},{mz:.3}) {roots}; delta: frames={publishes} upsert_entities={upsert_entities} skipped_empty={empty_delta_frames} coalesce={coalesce} net_tick={net_tick}"
            );
            physics_steps_logged = 0;
            last_log = Instant::now();
        }
    }
    outbound.stop();
    Ok(())
}

#[cfg(test)]
mod presence_reclaim_tests {
    use super::*;

    #[test]
    fn reclaim_stale_frees_slot_for_reuse() {
        let mut p = PresenceIntents::with_slots(USER_ROOT_SLOTS_MAX);
        let t0 = Instant::now();
        let slot = p.assign_or_lookup("alice", "j1", t0).expect("assign");
        assert_eq!(slot, 0);
        assert_eq!(p.to_presence_state(1).assignments.len(), 1);
        assert_eq!(p.to_presence_state(1).host_generation, 1);

        // Still within TTL: no reclaim.
        let freed = p.reclaim_stale(t0 + Duration::from_secs(2), PRESENCE_SLOT_TTL);
        assert!(freed.is_empty());
        assert!(p.occupied_sub[0].is_some());

        let freed = p.reclaim_stale(t0 + PRESENCE_SLOT_TTL, PRESENCE_SLOT_TTL);
        assert_eq!(
            freed,
            vec![(PhysicsWorld::user_root_entity_id(0), "alice".into())]
        );
        assert!(p.occupied_sub[0].is_none());
        assert_eq!(p.to_presence_state(1).assignments.len(), 0);
        assert!(p.revision >= 2);

        let slot2 = p
            .assign_or_lookup(
                "bob",
                "j2",
                t0 + PRESENCE_SLOT_TTL + Duration::from_millis(1),
            )
            .expect("reuse");
        assert_eq!(slot2, 0);
        assert_eq!(p.to_presence_state(1).assignments[0].sub, "bob".to_string());
    }

    #[test]
    fn same_sub_before_ttl_keeps_slot() {
        let mut p = PresenceIntents::with_slots(USER_ROOT_SLOTS_MAX);
        let t0 = Instant::now();
        assert_eq!(p.assign_or_lookup("alice", "j1", t0), Some(0));
        assert_eq!(
            p.assign_or_lookup("alice", "j1", t0 + Duration::from_secs(2)),
            Some(0)
        );
        let freed = p.reclaim_stale(
            t0 + Duration::from_secs(2) + Duration::from_secs(2),
            PRESENCE_SLOT_TTL,
        );
        // last_input refreshed at +2s; at +4s still within 3s TTL from last touch.
        assert!(freed.is_empty());
    }

    #[test]
    fn manifest_join_assist_on_assign_and_jti_rotate_not_same_jti_or_reclaim() {
        let mut p = PresenceIntents::with_slots(USER_ROOT_SLOTS_MAX);
        let t0 = Instant::now();
        assert!(!p.take_manifest_join_assist());
        assert_eq!(p.assign_or_lookup("alice", "j1", t0), Some(0));
        assert!(p.take_manifest_join_assist());
        assert!(!p.take_manifest_join_assist());
        assert_eq!(p.assign_or_lookup("alice", "j1", t0), Some(0));
        assert!(!p.take_manifest_join_assist());
        assert_eq!(p.assign_or_lookup("alice", "j2", t0), Some(0));
        assert!(p.take_manifest_join_assist());
        let _ = p.reclaim_sub("alice", "leave").expect("leave");
        assert!(!p.take_manifest_join_assist());
        assert!(p.revision >= 2);
    }

    #[test]
    fn reclaim_sub_frees_immediately() {
        let mut p = PresenceIntents::with_slots(USER_ROOT_SLOTS_MAX);
        let t0 = Instant::now();
        assert_eq!(p.assign_or_lookup("alice", "j1", t0), Some(0));
        let entity = p.reclaim_sub("alice", "leave").expect("reclaim");
        assert_eq!(
            entity,
            (PhysicsWorld::user_root_entity_id(0), "alice".into())
        );
        assert!(p.occupied_sub[0].is_none());
        assert_eq!(p.pending_removes, vec![entity.clone()]);
        let drained = p.drain_reclaims(t0, PRESENCE_SLOT_TTL);
        assert_eq!(drained, vec![entity]);
        assert!(p.pending_removes.is_empty());
        assert_eq!(p.assign_or_lookup("bob", "j2", t0), Some(0));
    }

    #[test]
    fn schedule_liveliness_requires_matching_jti() {
        let mut p = PresenceIntents::with_slots(USER_ROOT_SLOTS_MAX);
        let t0 = Instant::now();
        assert_eq!(p.assign_or_lookup("alice", "jti-real", t0), Some(0));

        assert!(!p.schedule_liveliness_reclaim("alice", "jti-fake", t0));
        assert!(p.occupied_sub[0].is_some());
        assert!(p.pending_liveliness[0].is_none());

        assert!(p.schedule_liveliness_reclaim("alice", "jti-real", t0));
        assert!(p.occupied_sub[0].is_some());
        assert!(p.pending_liveliness[0].is_some());

        // Before grace: still occupied.
        let early = p.confirm_liveliness_reclaims(t0 + Duration::from_millis(100));
        assert!(early.is_empty());
        assert!(p.occupied_sub[0].is_some());

        let freed = p.confirm_liveliness_reclaims(t0 + LIVELINESS_RECLAIM_GRACE);
        assert_eq!(
            freed,
            vec![(PhysicsWorld::user_root_entity_id(0), "alice".into())]
        );
        assert!(p.occupied_sub[0].is_none());
    }

    #[test]
    fn schedule_liveliness_ignores_unknown_sub() {
        let mut p = PresenceIntents::with_slots(USER_ROOT_SLOTS_MAX);
        assert!(!p.schedule_liveliness_reclaim("nobody", "jti", Instant::now()));
    }

    #[test]
    fn narrowed_slot_count_rejects_overflow() {
        let mut p = PresenceIntents::with_slots(2);
        let t0 = Instant::now();
        assert_eq!(p.assign_or_lookup("alice", "j1", t0), Some(0));
        assert_eq!(p.assign_or_lookup("bob", "j2", t0), Some(1));
        assert_eq!(p.assign_or_lookup("carol", "j3", t0), None);
        assert_eq!(p.slot_count(), 2);
        // Slot indices beyond Manifest capacity must stay unused even though the
        // backing array is USER_ROOT_SLOTS_MAX wide.
        assert!(p.occupied_sub[2].is_none());
        assert!(p.occupied_sub[3].is_none());
    }

    #[test]
    fn refresh_intent_cancels_pending_liveliness_delete() {
        // Race: Delete(old_jti) arrives before Intent(new_jti). Grace + jti rotate must not reclaim.
        let mut p = PresenceIntents::with_slots(USER_ROOT_SLOTS_MAX);
        let t0 = Instant::now();
        assert_eq!(p.assign_or_lookup("alice", "jti-old", t0), Some(0));
        assert!(p.schedule_liveliness_reclaim("alice", "jti-old", t0));

        // New Room Token Intent arrives within grace.
        assert_eq!(
            p.assign_or_lookup("alice", "jti-new", t0 + Duration::from_millis(50)),
            Some(0)
        );
        assert!(
            p.pending_liveliness[0].is_none(),
            "pending Delete must clear on jti rotate"
        );

        let freed = p.confirm_liveliness_reclaims(t0 + LIVELINESS_RECLAIM_GRACE);
        assert!(freed.is_empty());
        assert_eq!(p.occupied_sub[0].as_deref(), Some("alice"));
        assert_eq!(p.session_jti[0].as_deref(), Some("jti-new"));
    }
}
