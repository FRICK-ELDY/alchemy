//! Local host-server — x0022 liveliness↔jti + x0019 Leave + ROOM_ID keys / presence / Grab.
//!
//! Guarantees:
//! - Internal Rapier tick at 60Hz; network delta publish at 30Hz
//! - Input: Room Token verify + `is_finite` (NaN/Inf dropped before physics)
//! - SceneManifest with default Noclip LocomotionConfig (blake3 refs; no GLB bytes)
//! - Per-`sub` Intent/Grab → up to 4 user_roots (entity 3..6); first-come slot assign
//! - PresenceState: Leave by verified `sub`; liveliness Delete schedules jti-matched reclaim with grace (x0022)
//! - Zenoh keys from `ROOM_ID` via protocol `*_key` helpers (x0017)
//! - Grabbable cuboids entity 10..18; Grab hold kinematic follow; no Steal
//! - Slice E / x001e: Zenoh `host_control` off by default; `ALCHEMY_HOST_CONTROL=1` to enable
//!
//! Non-guarantees:
//! - Steal / rotation follow / Formula / polished Walk / signed Manifest / verifier
//! - Authenticated remote host shutdown (use launcher / `ALCHEMY_HOST_PID`)

mod finite;
mod host_control;
mod room_token;
mod sim;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use alchemy_protocol::{
    frame_key, host_control_key, input_buttons, input_key, liveliness_binding_from_key,
    liveliness_wildcard, manifest_key, presence_key, validate_room_id, Entity, InputEvent,
    LocomotionConfig, LocomotionMode, Message, PresenceAssignment, PresenceState, RenderFrame,
    SceneEntry, SceneManifest, PROTOCOL_PIN,
};
use anyhow::{Context, Result};
use finite::input_event_is_finite;
use room_token::{default_auth_url, fetch_auth_token, fetch_host_token, RoomTokenVerifier};
use sim::{
    box_color, user_root_color, LocoMode, PhysicsWorld, BOX_COUNT, BOX_ENTITY_FIRST,
    DEFAULT_MOVE_SPEED, USER_ROOT_ENTITY_FIRST, USER_ROOT_SLOTS,
};
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
/// Coin_PolygonalMind.glb (CC0) — same as client fixtures.
const COIN_BLAKE3_HEX: &str = "fa24848fa7b971718ce6e8dc9bd2d211808fd64e4ddf555feb66d4f82ee1e72a";
/// Pose change below this is treated as static (sleep / omit from upserts).
const MOVE_EPSILON: f64 = 1e-4;
/// Republish Manifest so late joiners still see blob refs (still ≪ frame rate).
const MANIFEST_REPUBLISH: Duration = Duration::from_secs(5);
/// Republish Presence for late joiners (same cadence as Manifest).
const PRESENCE_REPUBLISH: Duration = Duration::from_secs(5);
/// Intent silence before reclaiming a Presence slot (client INPUT_HZ=30).
const PRESENCE_SLOT_TTL: Duration = Duration::from_secs(3);

const ENTITY_MOVING: u32 = 1;
const ENTITY_STATIC: u32 = 2;

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
#[derive(Debug)]
struct PresenceIntents {
    by_slot: [PoseOverride; USER_ROOT_SLOTS],
    /// Occupied slots: Some(sub) when assigned.
    occupied_sub: [Option<String>; USER_ROOT_SLOTS],
    /// Last verified Room Token `jti` per occupied slot (x0022). None until first verify.
    session_jti: [Option<String>; USER_ROOT_SLOTS],
    /// Scheduled liveliness reclaim: `(deleted_jti, confirm_after)`. Cancelled if `session_jti` rotates.
    pending_liveliness: [Option<(String, Instant)>; USER_ROOT_SLOTS],
    /// Last accepted Intent time per slot (None when free).
    last_input: [Option<Instant>; USER_ROOT_SLOTS],
    dropped_full: u64,
    /// Bumped on assignment or reclaim (PresenceState.revision).
    revision: u64,
    /// Entity ids freed off the physics thread (Leave / liveliness); drained each tick.
    pending_removes: Vec<u32>,
}

impl Default for PresenceIntents {
    fn default() -> Self {
        Self {
            by_slot: [PoseOverride::default(); USER_ROOT_SLOTS],
            occupied_sub: std::array::from_fn(|_| None),
            session_jti: std::array::from_fn(|_| None),
            pending_liveliness: std::array::from_fn(|_| None),
            last_input: [None; USER_ROOT_SLOTS],
            dropped_full: 0,
            revision: 0,
            pending_removes: Vec::new(),
        }
    }
}

impl PresenceIntents {
    fn assign_or_lookup(&mut self, sub: &str, jti: &str, now: Instant) -> Option<usize> {
        for (i, occupied) in self.occupied_sub.iter().enumerate() {
            if occupied.as_deref() == Some(sub) {
                self.last_input[i] = Some(now);
                // Cancel pending Delete only on jti rotate (refresh). Same-jti Intent leaves
                // the schedule so crash/undeclare still confirms after grace.
                if self.session_jti[i].as_deref() != Some(jti) {
                    self.pending_liveliness[i] = None;
                }
                self.session_jti[i] = Some(jti.to_string());
                return Some(i);
            }
        }
        for (i, occupied) in self.occupied_sub.iter_mut().enumerate() {
            if occupied.is_none() {
                *occupied = Some(sub.to_string());
                self.session_jti[i] = Some(jti.to_string());
                self.pending_liveliness[i] = None;
                self.last_input[i] = Some(now);
                self.revision = self.revision.saturating_add(1);
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

    /// Free one occupied slot. Returns entity id.
    fn free_slot_at(&mut self, i: usize, reason: &str) -> u32 {
        let sub = self.occupied_sub[i].take();
        self.session_jti[i] = None;
        self.pending_liveliness[i] = None;
        self.last_input[i] = None;
        self.by_slot[i] = PoseOverride::default();
        self.revision = self.revision.saturating_add(1);
        let entity_id = PhysicsWorld::user_root_entity_id(i);
        log::info!(
            "presence: reclaimed slot={i} entity={entity_id} sub={} reason={reason} (revision={})",
            sub.as_deref().unwrap_or("?"),
            self.revision
        );
        entity_id
    }

    /// Immediate reclaim for verified Leave (queues `pending_removes`).
    ///
    /// Leave is authorized by JWT `sub` alone (any unexpired Room Token for that sub).
    /// Stale-token Leave kick is accepted; revocation feed is out of scope (→ P2-2 / auth).
    fn reclaim_sub(&mut self, sub: &str, reason: &str) -> Option<u32> {
        for i in 0..USER_ROOT_SLOTS {
            if self.occupied_sub[i].as_deref() == Some(sub) {
                let entity_id = self.free_slot_at(i, reason);
                self.pending_removes.push(entity_id);
                return Some(entity_id);
            }
        }
        None
    }

    /// Schedule reclaim when Delete `jti` matches the bound session (x0022).
    ///
    /// Does **not** free immediately: grace lets a refresh Intent with a new `jti` cancel the
    /// pending Delete when liveliness arrives before the Intent (no cross-key order).
    fn schedule_liveliness_reclaim(&mut self, sub: &str, jti: &str, now: Instant) -> bool {
        for i in 0..USER_ROOT_SLOTS {
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
    fn confirm_liveliness_reclaims(&mut self, now: Instant) -> Vec<u32> {
        let mut freed = Vec::new();
        for i in 0..USER_ROOT_SLOTS {
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

    /// Drop slots with no Intent for `ttl`. Returns freed user_root entity ids.
    fn reclaim_stale(&mut self, now: Instant, ttl: Duration) -> Vec<u32> {
        let mut freed = Vec::new();
        for i in 0..USER_ROOT_SLOTS {
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

    fn drain_reclaims(&mut self, now: Instant, ttl: Duration) -> Vec<u32> {
        let mut freed = self.reclaim_stale(now, ttl);
        freed.extend(self.confirm_liveliness_reclaims(now));
        freed.append(&mut self.pending_removes);
        freed
    }

    fn to_presence_state(&self) -> PresenceState {
        let mut assignments = Vec::with_capacity(USER_ROOT_SLOTS);
        for (i, occupied) in self.occupied_sub.iter().enumerate() {
            if let Some(sub) = occupied {
                assignments.push(PresenceAssignment {
                    sub: sub.clone(),
                    entity_id: PhysicsWorld::user_root_entity_id(i),
                });
            }
        }
        PresenceState {
            revision: self.revision,
            assignments,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Pose {
    x: f64,
    y: f64,
    z: f64,
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

fn zenoh_connect_endpoint() -> String {
    std::env::var("ZENOH_CONNECT").unwrap_or_else(|_| DEFAULT_ZENOH_CONNECT.to_string())
}

fn open_zenoh() -> Result<zenoh::Session> {
    zenoh_connect::open_client(&zenoh_connect_endpoint())
}

/// Hermetic E2E (x001f): signal only after input gate + frame publisher have put ≥1 frame.
/// Write failures are logged and ignored so a stray env cannot take down production host.
fn signal_host_ready_once(signaled: &mut bool) {
    if *signaled {
        return;
    }
    let Ok(path) = std::env::var("ALCHEMY_HOST_READY_FILE") else {
        *signaled = true;
        return;
    };
    if path.is_empty() {
        *signaled = true;
        return;
    }
    match std::fs::write(&path, b"ready\n") {
        Ok(()) => log::info!("host ready file written (after first frame put): {path}"),
        Err(err) => log::warn!("ALCHEMY_HOST_READY_FILE write failed ({path}): {err}"),
    }
    *signaled = true;
}

fn host_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn static_marker_pose() -> Pose {
    Pose {
        x: 3.0,
        y: 0.5,
        z: 0.0,
        r: 0.9,
        g: 0.3,
        b: 0.2,
        a: 1.0,
    }
}

fn moving_pose_from_sim(world: &PhysicsWorld) -> Pose {
    let (x, y, z) = world.moving_translation();
    Pose {
        x: f64::from(x),
        y: f64::from(y),
        z: f64::from(z),
        r: 0.2,
        g: 0.9,
        b: 0.4,
        a: 1.0,
    }
}

fn user_root_pose_from_sim(world: &PhysicsWorld, slot: usize) -> Pose {
    let (x, y, z) = world.user_root_translation(slot);
    let (r, g, b, a) = user_root_color(slot);
    Pose {
        x: f64::from(x),
        y: f64::from(y),
        z: f64::from(z),
        r,
        g,
        b,
        a,
    }
}

fn box_pose_from_sim(world: &PhysicsWorld, index: usize) -> Pose {
    let (x, y, z) = world.box_translation(index);
    let (r, g, b, a) = box_color(index);
    Pose {
        x: f64::from(x),
        y: f64::from(y),
        z: f64::from(z),
        r,
        g,
        b,
        a,
    }
}

fn to_entity(id: u32, pose: Pose) -> Entity {
    Entity {
        id,
        x: pose.x,
        y: pose.y,
        z: pose.z,
        r: pose.r,
        g: pose.g,
        b: pose.b,
        a: pose.a,
    }
}

fn pose_changed(prev: Pose, next: Pose) -> bool {
    (prev.x - next.x).abs() > MOVE_EPSILON
        || (prev.y - next.y).abs() > MOVE_EPSILON
        || (prev.z - next.z).abs() > MOVE_EPSILON
}

fn build_manifest(room_id: &str, blake3: &str) -> SceneManifest {
    SceneManifest {
        room_id: room_id.to_string(),
        revision: 1,
        entries: vec![SceneEntry {
            entity_id: ENTITY_MOVING,
            blake3: blake3.to_string(),
            x: 0.0,
            y: f64::from(sim::SPAWN_Y),
            z: 0.0,
        }],
        locomotion: Some(LocomotionConfig {
            mode: LocomotionMode::Noclip.into(),
            move_speed: DEFAULT_MOVE_SPEED,
        }),
    }
}

fn spawn_input_gate(
    session: zenoh::Session,
    input_key_expr: String,
    verifier: Arc<RoomTokenVerifier>,
    presence: Arc<Mutex<PresenceIntents>>,
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
                "subscribed to {input_key_expr} (Room Token + is_finite + Leave + per-sub gate)"
            );
            let mut accepted: u64 = 0;
            let mut leaves: u64 = 0;
            let mut dropped_token: u64 = 0;
            let mut dropped_finite: u64 = 0;
            let mut last_log = Instant::now();
            while let Ok(sample) = subscriber.recv() {
                let bytes: ZBytes = sample.payload().clone();
                let payload = bytes.to_bytes();
                match InputEvent::decode(payload.as_ref()) {
                    Ok(event) => match verifier.verify(&event.room_token) {
                        Ok(claims) => {
                            if claims.jti.is_empty() || claims.jti.contains('/') {
                                dropped_token += 1;
                                log::warn!("drop InputEvent: Room Token jti missing or contains '/'");
                            } else if event.leave {
                                if let Ok(mut guard) = presence.lock() {
                                    if guard.reclaim_sub(&claims.sub, "leave").is_some() {
                                        leaves += 1;
                                    }
                                }
                            } else if !input_event_is_finite(&event) {
                                dropped_finite += 1;
                                log::warn!(
                                    "drop InputEvent: non-finite move_*=({:.3},{:.3},{:.3})",
                                    event.move_x,
                                    event.move_y,
                                    event.move_z
                                );
                            } else if let Ok(mut guard) = presence.lock() {
                                match guard.assign_or_lookup(
                                    &claims.sub,
                                    &claims.jti,
                                    Instant::now(),
                                ) {
                                    Some(slot) => {
                                        let overlay = &mut guard.by_slot[slot];
                                        overlay.x = event.move_x;
                                        overlay.y = event.move_y;
                                        overlay.z = event.move_z;
                                        overlay.active = true;
                                        overlay.grab = (event.buttons & input_buttons::GRAB) != 0;
                                        overlay.grab_entity_id = event.grab_entity_id;
                                        overlay.aim_x = event.aim_x;
                                        overlay.aim_y = event.aim_y;
                                        overlay.aim_z = event.aim_z;
                                        accepted += 1;
                                    }
                                    None => {
                                        guard.dropped_full += 1;
                                        log::warn!(
                                            "drop InputEvent: presence full (max {USER_ROOT_SLOTS}); sub={}",
                                            claims.sub
                                        );
                                    }
                                }
                            }
                        }
                        Err(err) => {
                            dropped_token += 1;
                            log::warn!("drop InputEvent: Room Token invalid: {err:#}");
                        }
                    },
                    Err(err) => {
                        dropped_token += 1;
                        log::warn!("drop InputEvent: decode failed: {err}");
                    }
                }
                if last_log.elapsed() >= Duration::from_secs(5) {
                    let dropped_full = presence
                        .lock()
                        .map(|g| g.dropped_full)
                        .unwrap_or(0);
                    log::info!(
                        "input gate: accepted={accepted} leaves={leaves} dropped_token={dropped_token} dropped_non_finite={dropped_finite} dropped_full={dropped_full} (last ~5s)"
                    );
                    accepted = 0;
                    leaves = 0;
                    dropped_token = 0;
                    dropped_finite = 0;
                    last_log = Instant::now();
                }
            }
        })
        .context("spawn input thread")?;
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
                if let Ok(mut guard) = presence.lock() {
                    let _ = guard.schedule_liveliness_reclaim(sub, jti, Instant::now());
                }
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
    let blob_blake3 =
        std::env::var("ALCHEMY_BLOB_BLAKE3").unwrap_or_else(|_| COIN_BLAKE3_HEX.to_string());

    let manifest_key_expr = manifest_key(&room_id);
    let frame_key_expr = frame_key(&room_id);
    let input_key_expr = input_key(&room_id);
    let presence_key_expr = presence_key(&room_id);
    let host_control_key_expr = host_control_key(&room_id);
    let liveliness_key_expr = liveliness_wildcard(&room_id);
    let host_control_mode = if host_control::zenoh_control_enabled() {
        "zenoh-opt-in"
    } else {
        "disabled"
    };

    log::info!(
        "host-server x0022 starting; protocol_pin={PROTOCOL_PIN} physics_hz={PHYSICS_HZ} net_hz={NET_HZ} manifest={manifest_key_expr} frame={frame_key_expr} input={input_key_expr} presence={presence_key_expr} liveliness={liveliness_key_expr} host_control={host_control_key_expr}({host_control_mode}) world={world_url} auth={auth_url} room={room_id} user_roots={USER_ROOT_ENTITY_FIRST}..{} presence_ttl_secs={}",
        USER_ROOT_ENTITY_FIRST + USER_ROOT_SLOTS as u32 - 1,
        PRESENCE_SLOT_TTL.as_secs()
    );

    let auth_token = fetch_auth_token(&auth_url).context("fetch auth JWT")?;
    let host_token = fetch_host_token(&world_url, &room_id, &auth_token)
        .context("fetch host Room Token (is world-server up? auth Bearer required)")?;
    let verifier = Arc::new(RoomTokenVerifier::new(&world_url, &room_id));
    verifier.refresh_jwks().context("initial JWKS fetch")?;
    verifier.spawn_background_refresh();
    verifier
        .verify(&host_token)
        .context("host Room Token self-check")?;
    log::info!("auth JWT + host Room Token acquired and verified");

    let presence = Arc::new(Mutex::new(PresenceIntents::default()));

    let session = open_zenoh().context("open zenoh session")?;
    spawn_input_gate(
        session.clone(),
        input_key_expr,
        Arc::clone(&verifier),
        Arc::clone(&presence),
    )?;
    spawn_liveliness_watcher(session.clone(), &room_id, Arc::clone(&presence))?;
    host_control::spawn_listener(session.clone(), &room_id)?;

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

    let empty_presence = PresenceState {
        revision: 0,
        assignments: vec![],
    };
    presence_pub
        .put(empty_presence.encode_to_vec())
        .wait()
        .map_err(|e| anyhow::anyhow!("presence publish failed: {e}"))?;
    log::info!("published PresenceState revision=0 assignments=0");
    let mut last_presence_rev = 0u64;
    let mut last_presence = Instant::now();

    let manifest = build_manifest(&room_id, &blob_blake3);
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

    let manifest_bytes = manifest.encode_to_vec();
    manifest_pub
        .put(manifest_bytes.clone())
        .wait()
        .map_err(|e| anyhow::anyhow!("manifest publish failed: {e}"))?;
    log::info!(
        "published SceneManifest revision={} entries={} blake3={} loco={:?} speed={move_speed}",
        manifest.revision,
        manifest.entries.len(),
        blob_blake3,
        loco_mode
    );
    let mut last_manifest = Instant::now();

    let mut physics = PhysicsWorld::new(loco_mode, move_speed);
    let grabbable_n = physics.boxes.grabbable.iter().filter(|&&g| g).count();
    log::info!(
        "Rapier world ready (floor + coin entity {ENTITY_MOVING} + user_roots {USER_ROOT_ENTITY_FIRST}..{} + grabbable boxes {BOX_ENTITY_FIRST}..{} count={BOX_COUNT} grabbable={grabbable_n} half_extent={:.2} mode={:?})",
        USER_ROOT_ENTITY_FIRST + USER_ROOT_SLOTS as u32 - 1,
        BOX_ENTITY_FIRST + BOX_COUNT as u32 - 1,
        physics.boxes.half_extent,
        physics.loco_mode()
    );

    let physics_period = Duration::from_nanos(1_000_000_000 / PHYSICS_HZ);
    let dt = 1.0 / PHYSICS_HZ as f32;
    let mut next_physics = Instant::now();
    let mut physics_steps: u64 = 0;
    let mut net_tick: u64 = 0;
    let mut last_log = Instant::now();
    let mut publishes: u64 = 0;
    let mut upsert_entities: u64 = 0;
    let mut empty_delta_frames: u64 = 0;
    let mut physics_steps_logged: u64 = 0;
    let mut last_sent: HashMap<u32, Pose> = HashMap::new();
    // E2E ready = input gate + frame publisher live AND at least one frame put (x001f).
    let mut host_ready_signaled = false;

    loop {
        let now = Instant::now();
        if now < next_physics {
            std::thread::sleep(next_physics - now);
        }
        next_physics += physics_period;

        if last_manifest.elapsed() >= MANIFEST_REPUBLISH {
            manifest_pub
                .put(manifest_bytes.clone())
                .wait()
                .map_err(|e| anyhow::anyhow!("manifest republish failed: {e}"))?;
            last_manifest = Instant::now();
            log::info!(
                "republished SceneManifest revision={} (late-joiner assist)",
                manifest.revision
            );
        }

        let tick_now = Instant::now();
        let (by_slot, occupied, presence_rev, presence_state, reclaimed) = presence
            .lock()
            .map(|mut g| {
                let freed = g.drain_reclaims(tick_now, PRESENCE_SLOT_TTL);
                (
                    g.by_slot,
                    g.occupied_sub.clone(),
                    g.revision,
                    g.to_presence_state(),
                    freed,
                )
            })
            .unwrap_or_else(|_| {
                (
                    [PoseOverride::default(); USER_ROOT_SLOTS],
                    std::array::from_fn(|_| None),
                    0,
                    PresenceState {
                        revision: 0,
                        assignments: vec![],
                    },
                    Vec::new(),
                )
            });

        for &entity_id in &reclaimed {
            let slot = (entity_id - USER_ROOT_ENTITY_FIRST) as usize;
            physics.release_slot(slot);
            last_sent.remove(&entity_id);
        }

        if presence_rev != last_presence_rev || last_presence.elapsed() >= PRESENCE_REPUBLISH {
            let n = presence_state.assignments.len();
            let rev = presence_state.revision;
            presence_pub
                .put(presence_state.encode_to_vec())
                .wait()
                .map_err(|e| anyhow::anyhow!("presence publish failed: {e}"))?;
            if presence_rev != last_presence_rev {
                log::info!("published PresenceState revision={rev} assignments={n}");
            } else {
                log::info!(
                    "republished PresenceState revision={rev} assignments={n} (late-joiner assist)"
                );
            }
            last_presence_rev = presence_rev;
            last_presence = Instant::now();
        }

        for (slot, occupied_sub) in occupied.iter().enumerate() {
            if occupied_sub.is_none() {
                continue;
            }
            let overlay = by_slot[slot];
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

        if !physics_steps.is_multiple_of(PHYSICS_PER_NET) {
            // Still flush removes promptly when reclaim happens between net ticks.
            if !reclaimed.is_empty() {
                let frame = RenderFrame {
                    tick: net_tick,
                    host_time_ms: host_time_ms(),
                    upserts: vec![],
                    removes: reclaimed.clone(),
                };
                frame_pub
                    .put(frame.encode_to_vec())
                    .wait()
                    .map_err(|e| anyhow::anyhow!("frame publish failed: {e}"))?;
                net_tick = net_tick.wrapping_add(1);
                publishes += 1;
                signal_host_ready_once(&mut host_ready_signaled);
            }
            continue;
        }

        let mut poses: Vec<(u32, Pose)> = Vec::with_capacity(2 + USER_ROOT_SLOTS + BOX_COUNT);
        poses.push((ENTITY_MOVING, moving_pose_from_sim(&physics)));
        poses.push((ENTITY_STATIC, static_marker_pose()));
        for (slot, occupied_sub) in occupied.iter().enumerate() {
            if occupied_sub.is_some() {
                poses.push((
                    PhysicsWorld::user_root_entity_id(slot),
                    user_root_pose_from_sim(&physics, slot),
                ));
            }
        }
        for i in 0..BOX_COUNT {
            poses.push((physics.boxes.entity_ids[i], box_pose_from_sim(&physics, i)));
        }

        let mut upserts = Vec::new();
        for (id, next_pose) in poses {
            // Boxes always upsert: they sleep after settle, and frame is delta-only —
            // late joiners would otherwise never see entity 10..18 (x0010).
            let is_box = (BOX_ENTITY_FIRST..BOX_ENTITY_FIRST + BOX_COUNT as u32).contains(&id);
            let should_send = is_box
                || match last_sent.get(&id) {
                    None => true,
                    Some(prev) => pose_changed(*prev, next_pose),
                };
            if should_send {
                upserts.push(to_entity(id, next_pose));
                last_sent.insert(id, next_pose);
            }
        }

        let upsert_len = upserts.len() as u64;
        let removes = reclaimed;
        if upserts.is_empty() && removes.is_empty() {
            empty_delta_frames += 1;
            net_tick = net_tick.wrapping_add(1);
        } else {
            upsert_entities += upsert_len;
            let frame = RenderFrame {
                tick: net_tick,
                host_time_ms: host_time_ms(),
                upserts,
                removes,
            };
            frame_pub
                .put(frame.encode_to_vec())
                .wait()
                .map_err(|e| anyhow::anyhow!("frame publish failed: {e}"))?;
            net_tick = net_tick.wrapping_add(1);
            publishes += 1;
            signal_host_ready_once(&mut host_ready_signaled);
        }

        if last_log.elapsed() >= Duration::from_secs(1) {
            let (mx, my, mz) = physics.moving_translation();
            let mut roots = String::new();
            for (slot, occupied_sub) in occupied.iter().enumerate() {
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
            log::info!(
                "physics: steps={physics_steps_logged} coin=({mx:.3},{my:.3},{mz:.3}) {roots}; delta: frames={publishes} upsert_entities={upsert_entities} skipped_empty={empty_delta_frames} net_tick={net_tick}"
            );
            publishes = 0;
            upsert_entities = 0;
            empty_delta_frames = 0;
            physics_steps_logged = 0;
            last_log = Instant::now();
        }
    }
}

#[cfg(test)]
mod presence_reclaim_tests {
    use super::*;

    #[test]
    fn reclaim_stale_frees_slot_for_reuse() {
        let mut p = PresenceIntents::default();
        let t0 = Instant::now();
        let slot = p.assign_or_lookup("alice", "j1", t0).expect("assign");
        assert_eq!(slot, 0);
        assert_eq!(p.to_presence_state().assignments.len(), 1);

        // Still within TTL: no reclaim.
        let freed = p.reclaim_stale(t0 + Duration::from_secs(2), PRESENCE_SLOT_TTL);
        assert!(freed.is_empty());
        assert!(p.occupied_sub[0].is_some());

        let freed = p.reclaim_stale(t0 + PRESENCE_SLOT_TTL, PRESENCE_SLOT_TTL);
        assert_eq!(freed, vec![PhysicsWorld::user_root_entity_id(0)]);
        assert!(p.occupied_sub[0].is_none());
        assert_eq!(p.to_presence_state().assignments.len(), 0);
        assert!(p.revision >= 2);

        let slot2 = p
            .assign_or_lookup(
                "bob",
                "j2",
                t0 + PRESENCE_SLOT_TTL + Duration::from_millis(1),
            )
            .expect("reuse");
        assert_eq!(slot2, 0);
        assert_eq!(p.to_presence_state().assignments[0].sub, "bob".to_string());
    }

    #[test]
    fn same_sub_before_ttl_keeps_slot() {
        let mut p = PresenceIntents::default();
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
    fn reclaim_sub_frees_immediately() {
        let mut p = PresenceIntents::default();
        let t0 = Instant::now();
        assert_eq!(p.assign_or_lookup("alice", "j1", t0), Some(0));
        let entity = p.reclaim_sub("alice", "leave").expect("reclaim");
        assert_eq!(entity, PhysicsWorld::user_root_entity_id(0));
        assert!(p.occupied_sub[0].is_none());
        assert_eq!(p.pending_removes, vec![entity]);
        let drained = p.drain_reclaims(t0, PRESENCE_SLOT_TTL);
        assert_eq!(drained, vec![entity]);
        assert!(p.pending_removes.is_empty());
        assert_eq!(p.assign_or_lookup("bob", "j2", t0), Some(0));
    }

    #[test]
    fn schedule_liveliness_requires_matching_jti() {
        let mut p = PresenceIntents::default();
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
        assert_eq!(freed, vec![PhysicsWorld::user_root_entity_id(0)]);
        assert!(p.occupied_sub[0].is_none());
    }

    #[test]
    fn schedule_liveliness_ignores_unknown_sub() {
        let mut p = PresenceIntents::default();
        assert!(!p.schedule_liveliness_reclaim("nobody", "jti", Instant::now()));
    }

    #[test]
    fn refresh_intent_cancels_pending_liveliness_delete() {
        // Race: Delete(old_jti) arrives before Intent(new_jti). Grace + jti rotate must not reclaim.
        let mut p = PresenceIntents::default();
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
