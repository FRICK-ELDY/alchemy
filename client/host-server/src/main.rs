//! Local host-server — x0025 host_pub attestation + x0024 wire-tick + x0023 session.
//!
//! Guarantees:
//! - Internal XPBD tick at 60Hz (x0093); network delta publish at 30Hz
//! - Wire Frame TRS sampled from Epoch Curr (Rapier removed; no kinematic mirror)
//! - Input: Ed25519 session bind (Token on bind only) + `is_finite` (x0023 / x0021)
//! - SceneManifest from world DemoScene + attestation + seal; Frames / Presence signed (x0027)
//! - x003d: credential / ManifestSeal HTTP on a background worker; physics evaluates
//!   hard expiry each tick. Transient refresh failure keeps last good creds; hard expiry
//!   stops Manifest/Presence/Frame/**snapshot queryable** without exiting the process.
//!   Resume synthesizes wire `removes` + full upserts from the last `last_sent` view
//! - x0042: Zenoh Frame/Manifest/Presence `put().wait()` on `zenoh-publish`; physics
//!   submits wire-agnostic pose samples / jobs (encode+sign stay in the worker)
//! - `…/snapshot` queryable for late join (refused while gated); pure pose deltas
//! - `…/sim_state` queryable for planned handover (x00ag; Room AEAD + host sig; refused while gated)
//! - `ALCHEMY_HOST_BOOT_MODE=cold|planned_handover|crash_promote` (default cold)
//! - Empty deltas: **no publish and no `net_tick` advance**; snapshot tick = last published (x0024)
//! - Per-`sub` Intent/Grab → Manifest user_root slots (≤ `USER_ROOT_SLOTS_MAX` DoS; x004d / x009b);
//!   Presence Leave / liveliness jti grace (x0022)
//! - x0040 / x0077 / x009c: Intent rate after Ed25519 (steady: after seq eval, before
//!   `last_seq` commit, Leave-exempt; bind: soft-global 20/10s + per-`session_pub` 5/2s
//!   before JWT, Leave **not** exempt; successful JWT **refunds** global; key is
//!   Ed25519 `session_pub` not ZenohId; `by_session_pub` ≤ `MAX_BIND_PUBS` with idle GC
//!   on last **successful** consume; failed JWT drops per-pub **only if unbound**;
//!   rebind GC skips the new pub). JWKS hard stale on Room Token verify
//! - Zenoh keys from `ROOM_ID` via protocol `*_key` helpers (x0017)
//! - Slice E / x001e: Zenoh `host_control` off by default; `ALCHEMY_HOST_CONTROL=1` to enable
//! - x0032 / x009f: localhost IPC shutdown + parent AUTH_PUSH when `ALCHEMY_HOST_IPC_*` set
//!
//! Non-guarantees:
//! - Steal / rotation follow / Dual-AST Kernel / polished Walk / verifier
//! - Rapier bit-exact parity; capsule Walk extrude; production default `mvcc_xpbd`
//! - Authenticated remote (WAN) host shutdown
//! - Full UGC scene editor (x006d only appends one mesh entry via world HTTP)
//! - rkyv / bytemuck wire (encode is protobuf inside network worker today)
//! - Intent temporary ban / Zenoh peer ACL
//! - Unlimited Ed25519 cost on unbound steady floods (no sub bucket until bind)
//! - Join denial while soft-global is drained by failed-JWT flood; new-pub join while
//!   bind-pub map is full and every row stays hot within `BIND_PUB_IDLE`
//! - App→host last-good IPC; automatic prev `host_pub` discovery; `drot` restore; `…/edit`

#![forbid(unsafe_code)]

mod finite;
mod host_control;
mod host_ipc;
mod host_loop;
mod inventory_despawn;
mod inventory_spawn;
mod network_worker;
mod pose_delta;
mod presence;
mod sim;
mod sim_backend;
mod sim_state;
mod wire_tick;

use host_server::{intent_rate, metrics, room_aead, room_token, session};

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use alchemy_protocol::decode_limits::{
    check_frame, check_input, check_payload_bytes, check_sim_state, WireClass,
};
use alchemy_protocol::wire_aead::{self, AeadKind};
use alchemy_protocol::wire_crypto::{
    generate_signing_key, public_key_bytes, sign_frame, sign_presence, sign_sim_state,
    sim_state_entities_finite, verify_frame, verify_sim_state, SigningKey,
};
use alchemy_protocol::{
    frame_key, host_control_key, input_buttons, input_key, liveliness_binding_from_key,
    liveliness_wildcard, manifest_key, presence_key, protocol_pin_matches, sim_state_key,
    snapshot_key, user_root_slots_from_entries, validate_room_id, InputEvent, LocomotionConfig,
    LocomotionMode, Message, PresenceState, RenderFrame, SimEntity, SimState, DEFAULT_MOVE_SPEED,
    PROTOCOL_PIN, USER_ROOT_SLOTS_MAX,
};
use anyhow::{Context, Result};
use finite::{input_event_is_finite, sanitize_release_velocity};
use host_loop::{read_publish_gate, HostLoop, HostLoopParts, NET_HZ, PHYSICS_HZ};
use intent_rate::IntentRateLimiter;
use network_worker::{spawn_network_worker, NetworkWorkerConfig};
use presence::{lock_or_fatal, PresenceIntents, PRESENCE_SLOT_TTL};
use room_token::{
    default_auth_url, earliest_hard_expire, ensure_sealed_manifest, fetch_room_scene,
    spawn_credential_worker, AuthBrokerInbox, CredentialWorkerConfig, HostCredentialChain,
    HostPublishSnapshot, ManifestSealCache, RoomTokenVerifier, SharedHostHttpAuth,
};
use session::SessionTable;
use sim::{LocoMode, PhysicsWorld, USER_ROOT_ENTITY_FIRST};
use sim_state::{decide_boot, BootSource, HostBootMode};
use zenoh::bytes::ZBytes;
use zenoh::sample::SampleKind;
use zenoh::Wait;

const DEFAULT_ZENOH_CONNECT: &str = "tcp/127.0.0.1:7447";
const DEFAULT_WORLD_URL: &str = "http://127.0.0.1:4003";
const DEFAULT_ROOM_ID: &str = "local";

fn zenoh_connect_endpoint() -> String {
    std::env::var("ZENOH_CONNECT").unwrap_or_else(|_| DEFAULT_ZENOH_CONNECT.to_string())
}

fn open_zenoh() -> Result<zenoh::Session> {
    zenoh_connect::open_client(&zenoh_connect_endpoint())
}

#[allow(clippy::too_many_arguments)]
fn spawn_input_gate(
    session: zenoh::Session,
    input_key_expr: String,
    room_id: String,
    room_aead: room_aead::SharedRoomAead,
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
                "subscribed to {input_key_expr} (AEAD + session Ed25519 + optional Room Token bind + is_finite)"
            );
            let mut accepted: u64 = 0;
            let mut leaves: u64 = 0;
            let mut dropped_auth: u64 = 0;
            let mut dropped_finite: u64 = 0;
            let mut dropped_limits: u64 = 0;
            let mut dropped_rate: u64 = 0;
            let mut dropped_aead: u64 = 0;
            let mut last_log = Instant::now();
            while let Ok(sample) = subscriber.recv() {
                // x003b: early drops must still reach the interval aggregate log below.
                'sample: {
                let bytes: ZBytes = sample.payload().clone();
                // x0069: ciphertext ceiling before open; plaintext limits after.
                if let Err(err) = wire_aead::check_aead_envelope_bytes(AeadKind::Input, bytes.len())
                {
                    dropped_limits += 1;
                    log::warn!("drop InputEvent: {err}");
                    break 'sample;
                }
                let envelope = bytes.to_bytes();
                let payload = match room_aead::open_shared(
                    &room_aead, &room_id, AeadKind::Input, envelope.as_ref(),
                ) {
                    Ok(pt) => pt,
                    Err(err) => {
                        dropped_aead += 1;
                        log::warn!("drop InputEvent: AEAD: {err}");
                        break 'sample;
                    }
                };
                if let Err(err) = check_payload_bytes(WireClass::Input, payload.len()) {
                    dropped_limits += 1;
                    log::warn!("drop InputEvent: {err}");
                    break 'sample;
                }
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
                        // Bind: rates alone → JWT/sessions → rates (ok: refund);
                        //   fail: nested sessions→rates for is_bound+forget (x009c).
                        // Steady: sessions (evaluate) → rates → sessions (commit).
                        // Leave/reclaim later: sessions then rates (never rates→sessions).
                        if let Err(err) = SessionTable::verify_intent_sig(&event) {
                            dropped_auth += 1;
                            log::warn!("drop InputEvent: session/auth: {err:#}");
                            break 'sample;
                        }
                        let auth = if is_bind {
                            // rates-only: atomic soft-global + per-session_pub (Leave NOT exempt).
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
                                    // x007a: Room Token jti correlation (same key as world/relay).
                                    log::info!("bind ok: sub={sub} room_jti={jti}");
                                    // sessions then rates: refund global + GC other pubs.
                                    let mut rates = lock_or_fatal(&rate_limiter, "intent_rate");
                                    rates.refund_bind_global(now);
                                    rates.forget_session_pubs(&removed);
                                    Ok((sub, jti, caps))
                                }
                                Err(err) => {
                                    // Nested lock order: sessions → rates. Hold sessions
                                    // across is_bound + after_bind_jwt_fail so a concurrent
                                    // commit_bind cannot race the forget (x009c).
                                    {
                                        let table = lock_or_fatal(&sessions, "sessions");
                                        let is_bound = table.is_bound(&event.session_pub);
                                        let mut rates =
                                            lock_or_fatal(&rate_limiter, "intent_rate");
                                        rates.after_bind_jwt_fail(
                                            &event.session_pub,
                                            is_bound,
                                        );
                                    }
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
                                    sanitize_release_velocity(&mut event);
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
                                            // Wire ack = applied overlay seq (not SessionBinding.last_seq).
                                            overlay.applied_intent_seq = event.intent_seq;
                                            overlay.release_vx = event.release_vx;
                                            overlay.release_vy = event.release_vy;
                                            overlay.release_vz = event.release_vz;
                                            overlay.release_wx = event.release_wx;
                                            overlay.release_wy = event.release_wy;
                                            overlay.release_wz = event.release_wz;
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
                        "input gate: accepted={accepted} leaves={leaves} dropped_auth={dropped_auth} dropped_non_finite={dropped_finite} dropped_limits={dropped_limits} dropped_aead={dropped_aead} dropped_rate={dropped_rate} dropped_full={dropped_full} (last ~5s)"
                    );
                    accepted = 0;
                    leaves = 0;
                    dropped_auth = 0;
                    dropped_finite = 0;
                    dropped_limits = 0;
                    dropped_aead = 0;
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
    room_id: String,
    room_aead: room_aead::SharedRoomAead,
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
                let env = match room_aead::seal_shared(
                    &room_aead,
                    &room_id,
                    AeadKind::Snapshot,
                    &frame.encode_to_vec(),
                ) {
                    Ok(env) => env,
                    Err(err) => {
                        log::warn!("snapshot AEAD seal failed: {err}");
                        if let Err(reply_err) = query.reply_err("snapshot AEAD seal failed").wait()
                        {
                            log::warn!("snapshot reply_err failed: {reply_err}");
                        }
                        continue;
                    }
                };
                if let Err(err) = query.reply(key, env).wait() {
                    log::warn!("snapshot reply failed: {err}");
                }
            }
        })
        .context("spawn snapshot thread")?;
    Ok(())
}

fn spawn_sim_state_queryable(
    session: zenoh::Session,
    sim_state_key_expr: String,
    sim_state_cache: Arc<Mutex<Option<SimState>>>,
    publish_snapshot: Arc<RwLock<HostPublishSnapshot>>,
    host_key: SigningKey,
    room_id: String,
    room_aead: room_aead::SharedRoomAead,
) -> Result<()> {
    let queryable = session
        .declare_queryable(sim_state_key_expr.clone())
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_queryable sim_state failed: {e}"))?;

    thread::Builder::new()
        .name("zenoh-sim-state".into())
        .spawn(move || {
            log::info!("queryable ready on {sim_state_key_expr}");
            while let Ok(query) = queryable.recv() {
                let gate = read_publish_gate(&publish_snapshot, Instant::now());
                if !gate.publish_allowed {
                    if let Err(err) = query
                        .reply_err("sim_state unavailable (credential hard expiry)")
                        .wait()
                    {
                        log::warn!("sim_state reply_err failed: {err}");
                    }
                    continue;
                }
                let mut state = match lock_or_fatal(&sim_state_cache, "sim_state").clone() {
                    Some(s) if !s.entities.is_empty() => s,
                    _ => {
                        if let Err(err) = query
                            .reply_err("sim_state not ready (no entities yet)")
                            .wait()
                        {
                            log::warn!("sim_state reply_err failed: {err}");
                        }
                        continue;
                    }
                };
                sign_sim_state(&host_key, &mut state);
                let key = sim_state_key_expr.clone();
                let env = match room_aead::seal_shared(
                    &room_aead,
                    &room_id,
                    AeadKind::SimState,
                    &state.encode_to_vec(),
                ) {
                    Ok(env) => env,
                    Err(err) => {
                        log::warn!("sim_state AEAD seal failed: {err}");
                        if let Err(reply_err) = query.reply_err("sim_state AEAD seal failed").wait()
                        {
                            log::warn!("sim_state reply_err failed: {reply_err}");
                        }
                        continue;
                    }
                };
                if let Err(err) = query.reply(key, env).wait() {
                    log::warn!("sim_state reply failed: {err}");
                }
            }
        })
        .context("spawn sim_state thread")?;
    Ok(())
}

/// Fetch planned-handover SimState from the previous host (x00ag).
fn fetch_sim_state(
    session: &zenoh::Session,
    room_id: &str,
    wire_cap: &str,
    room_aead: &room_aead::SharedRoomAead,
    expected_host_pub: &[u8],
) -> Result<SimState> {
    let key = sim_state_key(room_id, wire_cap);
    let receiver = session
        .get(&key)
        .wait()
        .map_err(|e| anyhow::anyhow!("sim_state get failed: {e}"))?;
    let mut last_err = anyhow::anyhow!("sim_state: no usable reply");
    while let Ok(reply) = receiver.recv() {
        match reply.result() {
            Ok(sample) => {
                let bytes: ZBytes = sample.payload().clone();
                let envelope = bytes.to_bytes();
                let plain = match room_aead::open_shared(
                    room_aead,
                    room_id,
                    AeadKind::SimState,
                    envelope.as_ref(),
                ) {
                    Ok(pt) => pt,
                    Err(err) => {
                        last_err = anyhow::anyhow!("sim_state AEAD: {err}");
                        continue;
                    }
                };
                if let Err(err) = check_payload_bytes(WireClass::SimState, plain.len()) {
                    last_err = anyhow::anyhow!("sim_state limits: {err}");
                    continue;
                }
                let state = match SimState::decode(plain.as_ref()) {
                    Ok(s) => s,
                    Err(err) => {
                        last_err = anyhow::anyhow!("sim_state decode: {err}");
                        continue;
                    }
                };
                if let Err(err) = check_sim_state(&state) {
                    last_err = anyhow::anyhow!("sim_state check: {err}");
                    continue;
                }
                if !protocol_pin_matches(&state.protocol_pin) {
                    last_err = anyhow::anyhow!(
                        "sim_state protocol_pin mismatch: got {}",
                        state.protocol_pin
                    );
                    continue;
                }
                if state.room_id != room_id {
                    last_err = anyhow::anyhow!(
                        "sim_state room_id mismatch: got {} expected {room_id}",
                        state.room_id
                    );
                    continue;
                }
                if !sim_state_entities_finite(&state) {
                    last_err = anyhow::anyhow!("sim_state non-finite entities");
                    continue;
                }
                if let Err(err) = verify_sim_state(expected_host_pub, &state) {
                    last_err = anyhow::anyhow!("sim_state signature: {err}");
                    continue;
                }
                return Ok(state);
            }
            Err(err) => {
                last_err = anyhow::anyhow!("sim_state reply err: {err}");
            }
        }
    }
    Err(last_err)
}

/// Crash seed: map a signed `…/snapshot` into SimState with **pose only / velocity 0**.
/// `flags` stays 0 so import keeps Manifest/cold readiness (does not wipe ALLOCATED/DYNAMIC).
fn fetch_snapshot_as_sim_state(
    session: &zenoh::Session,
    room_id: &str,
    wire_cap: &str,
    room_aead: &room_aead::SharedRoomAead,
    expected_host_pub: &[u8],
    scene_rev: u64,
) -> Result<SimState> {
    let key = snapshot_key(room_id, wire_cap);
    let receiver = session
        .get(&key)
        .wait()
        .map_err(|e| anyhow::anyhow!("snapshot get failed: {e}"))?;
    let mut last_err = anyhow::anyhow!("snapshot: no usable reply");
    while let Ok(reply) = receiver.recv() {
        match reply.result() {
            Ok(sample) => {
                let bytes: ZBytes = sample.payload().clone();
                let envelope = bytes.to_bytes();
                let plain = match room_aead::open_shared(
                    room_aead,
                    room_id,
                    AeadKind::Snapshot,
                    envelope.as_ref(),
                ) {
                    Ok(pt) => pt,
                    Err(err) => {
                        last_err = anyhow::anyhow!("snapshot AEAD: {err}");
                        continue;
                    }
                };
                if let Err(err) = check_payload_bytes(WireClass::Frame, plain.len()) {
                    last_err = anyhow::anyhow!("snapshot limits: {err}");
                    continue;
                }
                let frame = match RenderFrame::decode(plain.as_ref()) {
                    Ok(f) => f,
                    Err(err) => {
                        last_err = anyhow::anyhow!("snapshot decode: {err}");
                        continue;
                    }
                };
                if let Err(err) = check_frame(&frame) {
                    last_err = anyhow::anyhow!("snapshot check: {err}");
                    continue;
                }
                if frame.upserts.is_empty() {
                    last_err = anyhow::anyhow!("snapshot empty upserts");
                    continue;
                }
                if let Err(err) = verify_frame(expected_host_pub, &frame) {
                    last_err = anyhow::anyhow!("snapshot signature: {err}");
                    continue;
                }
                let entities = frame
                    .upserts
                    .iter()
                    .map(|e| SimEntity {
                        id: e.id,
                        x: e.x,
                        y: e.y,
                        z: e.z,
                        qx: e.qx as f32,
                        qy: e.qy as f32,
                        qz: e.qz as f32,
                        qw: e.qw as f32,
                        dx: 0.0,
                        dy: 0.0,
                        dz: 0.0,
                        drot_x: 0.0,
                        drot_y: 0.0,
                        drot_z: 0.0,
                        holder: e.holder,
                        flags: 0,
                    })
                    .collect();
                return Ok(SimState {
                    protocol_pin: PROTOCOL_PIN.into(),
                    room_id: room_id.into(),
                    host_generation: 0,
                    scene_rev,
                    sim_tick: frame.sim_tick,
                    net_tick: frame.tick,
                    entities,
                    host_sig: Vec::new(),
                });
            }
            Err(err) => {
                last_err = anyhow::anyhow!("snapshot reply err: {err}");
            }
        }
    }
    Err(last_err)
}

fn parse_prev_host_pub_hex() -> Result<Vec<u8>> {
    let raw = std::env::var("ALCHEMY_PREV_HOST_PUB")
        .context("ALCHEMY_PREV_HOST_PUB required for planned_handover (32-byte hex)")?;
    let hex = raw.trim();
    if hex.len() != 64 {
        anyhow::bail!(
            "ALCHEMY_PREV_HOST_PUB must be 64 hex chars, got {}",
            hex.len()
        );
    }
    let mut out = vec![0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|e| anyhow::anyhow!("ALCHEMY_PREV_HOST_PUB hex: {e}"))?;
    }
    Ok(out)
}

fn spawn_liveliness_watcher(
    session: zenoh::Session,
    room_id: &str,
    wire_cap: &str,
    presence: Arc<Mutex<PresenceIntents>>,
) -> Result<()> {
    let key = liveliness_wildcard(room_id, wire_cap);
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
                let key_str = sample.key_expr().as_str();
                let Some((sub, jti)) = liveliness_binding_from_key(key_str) else {
                    if sample.kind() == SampleKind::Delete {
                        log::warn!("liveliness Delete: unparseable key={key_str}");
                    }
                    continue;
                };
                let mut guard = lock_or_fatal(&presence, "presence");
                match sample.kind() {
                    SampleKind::Delete => {
                        let _ = guard.schedule_liveliness_reclaim(sub, jti, Instant::now());
                    }
                    SampleKind::Put => {
                        let _ = guard.cancel_liveliness_reclaim_on_put(sub, jti);
                    }
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
    let host_control_mode = if host_control::zenoh_control_enabled() {
        "zenoh-opt-in"
    } else {
        "disabled"
    };

    let host_signing_key = generate_signing_key();
    let host_pub = public_key_bytes(&host_signing_key);

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

    let wire_cap = creds.wire_cap.clone();
    let room_aead = room_aead::shared_room_aead(creds.room_key, creds.room_key_epoch);
    if let Some((prev_key, prev_epoch)) = creds.room_key_prev {
        let grace = creds
            .host_hard_expire_at
            .checked_duration_since(Instant::now())
            .unwrap_or(room_aead::DEFAULT_PREV_GRACE);
        if let Ok(mut keys) = room_aead.write() {
            let _ = keys.apply_token_key(
                creds.room_key,
                creds.room_key_epoch,
                grace,
                Some((prev_key, prev_epoch)),
            );
        }
    }
    let manifest_key_expr = manifest_key(&room_id, &wire_cap);
    let frame_key_expr = frame_key(&room_id, &wire_cap);
    let input_key_expr = input_key(&room_id, &wire_cap);
    let presence_key_expr = presence_key(&room_id, &wire_cap);
    let snapshot_key_expr = snapshot_key(&room_id, &wire_cap);
    let sim_state_key_expr = sim_state_key(&room_id, &wire_cap);
    let host_control_key_expr = host_control_key(&room_id, &wire_cap);
    let liveliness_key_expr = liveliness_wildcard(&room_id, &wire_cap);
    let boot_mode = HostBootMode::from_env().map_err(anyhow::Error::msg)?;

    log::info!(
        "host-server x004e starting; protocol_pin={PROTOCOL_PIN} physics_hz={PHYSICS_HZ} net_hz={NET_HZ} boot_mode={boot_mode:?} manifest={manifest_key_expr} frame={frame_key_expr} snapshot={snapshot_key_expr} sim_state={sim_state_key_expr} input={input_key_expr} presence={presence_key_expr} liveliness={liveliness_key_expr} host_control={host_control_key_expr}({host_control_mode}) world={world_url} auth={auth_url} room={room_id} wire_cap_len={} user_root_band={USER_ROOT_ENTITY_FIRST}..{} presence_ttl_secs={}",
        wire_cap.len(),
        USER_ROOT_ENTITY_FIRST + USER_ROOT_SLOTS_MAX as u32 - 1,
        PRESENCE_SLOT_TTL.as_secs()
    );

    // Scene is fetched at boot; x006d take-out may append mesh entries via world HTTP.
    let room_scene = fetch_room_scene(&world_url, &room_id, &creds.host_token, &creds.pop_key)
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
    let shared_scene = Arc::new(RwLock::new(room_scene.clone()));
    let force_reseal = Arc::new(AtomicBool::new(false));

    let presence = Arc::new(Mutex::new(PresenceIntents::with_slots(user_root_slots)));
    let sessions = Arc::new(Mutex::new(SessionTable::default()));
    let intent_rates = Arc::new(Mutex::new(IntentRateLimiter::new()));
    let snapshot_cache = Arc::new(Mutex::new(RenderFrame {
        tick: 0,
        host_time_ms: 0,
        upserts: vec![],
        removes: vec![],
        host_sig: vec![],
        ..Default::default()
    }));

    // Bootstrap already applied world mtls or allocate into env when configured (x0066/x004e).
    // Re-run allocate only if env still empty and allocation_token is present
    // (`allocate_if_configured` sets ZENOH_TLS_CONNECT_* itself).
    if !creds.allocation_token.is_empty() && !zenoh_connect::mtls_env_ready() {
        if let Some(id) = zenoh_connect::allocate_if_configured(&creds.allocation_token)
            .context("relay allocate (host)")?
        {
            log::info!("relay allocate ok cn={} role={}", id.cn, id.role);
        }
    }
    let session = open_zenoh().context("open zenoh session")?;
    spawn_input_gate(
        session.clone(),
        input_key_expr,
        room_id.clone(),
        Arc::clone(&room_aead),
        Arc::clone(&verifier),
        Arc::clone(&presence),
        Arc::clone(&sessions),
        Arc::clone(&intent_rates),
    )?;
    spawn_liveliness_watcher(session.clone(), &room_id, &wire_cap, Arc::clone(&presence))?;
    host_control::spawn_listener(session.clone(), &room_id, &wire_cap)?;
    let auth_broker = AuthBrokerInbox::new();
    host_ipc::spawn_listener(auth_broker.clone()).context("host IPC listener")?;
    let http_auth: SharedHostHttpAuth = creds.http_auth.clone();
    let spawn_inbox = inventory_spawn::spawn_listener(
        session.clone(),
        &room_id,
        &wire_cap,
        Arc::clone(&room_aead),
        world_url.clone(),
        Arc::clone(&sessions),
        http_auth.clone(),
        Arc::clone(&shared_scene),
    )
    .context("inventory spawn listener")?;
    let despawn_inbox = inventory_despawn::despawn_listener(
        session.clone(),
        &room_id,
        &wire_cap,
        Arc::clone(&room_aead),
        world_url.clone(),
        Arc::clone(&sessions),
        http_auth,
        Arc::clone(&shared_scene),
    )
    .context("inventory despawn listener")?;

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
    let empty_presence_env = room_aead::seal_shared(
        &room_aead,
        &room_id,
        AeadKind::Presence,
        &empty_presence.encode_to_vec(),
    )
    .context("AEAD seal initial presence")?;
    presence_pub
        .put(empty_presence_env)
        .wait()
        .map_err(|e| anyhow::anyhow!("presence publish failed: {e}"))?;
    log::info!(
        "published PresenceState revision=0 assignments=0 generation={} (signed)",
        creds.host_generation
    );
    let last_presence_rev = 0u64;
    let last_presence = Instant::now();

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
    // room_scene moved into shared_scene above; keep local clone for physics boot only.
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
    // Snapshot + sim_state queryables are declared AFTER boot fetch (below) so
    // planned/crash get cannot race our own not-ready replies against the old host.
    // Ops: until that declare, late-join / concurrent promote gets no local answer
    // (window is intentional; only the previous host should serve during fetch).
    let sim_state_cache = Arc::new(Mutex::new(None));
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
            scene: Arc::clone(&shared_scene),
            force_reseal: Arc::clone(&force_reseal),
            room_aead: Arc::clone(&room_aead),
            auth_broker,
        },
    );
    log::info!("credential worker spawned (HTTP refresh off physics thread)");

    let manifest_bytes = manifest.encode_to_vec();
    let manifest_revision = manifest.revision;
    let manifest_epoch = 0u64;
    let initial_manifest_env =
        room_aead::seal_shared(&room_aead, &room_id, AeadKind::Manifest, &manifest_bytes)
            .context("AEAD seal initial manifest")?;
    manifest_pub
        .put(initial_manifest_env)
        .wait()
        .map_err(|e| anyhow::anyhow!("manifest publish failed: {e}"))?;
    log::info!(
        "published SceneManifest revision={} entries={} loco={:?} speed={move_speed} (attested+sealed)",
        manifest_revision,
        manifest.entries.len(),
        loco_mode
    );
    let last_manifest = Instant::now();
    let last_publish_denied_log = Instant::now() - Duration::from_secs(60);

    // x0042: move publishers to network worker (boot puts above stay on main).
    let net = spawn_network_worker(NetworkWorkerConfig {
        manifest_pub,
        presence_pub,
        frame_pub,
        snapshot_cache: Arc::clone(&snapshot_cache),
        host_key: host_signing_key.clone(),
        room_id: room_id.clone(),
        room_aead: Arc::clone(&room_aead),
    })?;
    let outbound = net.hub;
    let publish_stats = net.stats;

    let sim_backend = sim::SimBackendKind::from_env().map_err(anyhow::Error::msg)?;
    let boot_source = match boot_mode {
        HostBootMode::ColdStart => BootSource::Cold,
        HostBootMode::PlannedHandover => {
            // Fetch while our own sim_state queryable is NOT declared (old host only).
            let prev_pub = parse_prev_host_pub_hex()?;
            let state = fetch_sim_state(&session, &room_id, &wire_cap, &room_aead, &prev_pub)
                .context("planned_handover: fetch …/sim_state")?;
            if state.scene_rev != 0 && state.scene_rev != room_scene.revision {
                log::warn!(
                    "planned_handover scene_rev={} != local Manifest revision={}; continuing (topology from local scene)",
                    state.scene_rev,
                    room_scene.revision
                );
            }
            BootSource::Planned(state)
        }
        HostBootMode::CrashPromote => {
            // last-good IPC is non-guarantee; try old-host snapshot while it still answers.
            let prev_pub = parse_prev_host_pub_hex().ok();
            let snapshot = match prev_pub.as_deref() {
                Some(pub_bytes) => fetch_snapshot_as_sim_state(
                    &session,
                    &room_id,
                    &wire_cap,
                    &room_aead,
                    pub_bytes,
                    room_scene.revision,
                )
                .map_err(|err| {
                    log::warn!("crash_promote: snapshot seed unavailable: {err:#}");
                    err
                })
                .ok(),
                None => {
                    log::warn!(
                        "crash_promote: ALCHEMY_PREV_HOST_PUB unset; cannot verify snapshot seed"
                    );
                    None
                }
            };
            BootSource::Crash {
                last_good: vec![],
                snapshot,
            }
        }
    };
    let decision = decide_boot(boot_source);
    let (physics, resume_sim_tick) = match decision {
        sim_state::BootDecision::FailClosed { reason } => {
            anyhow::bail!("host boot fail-closed (x00ag §4.8): {reason}");
        }
        sim_state::BootDecision::Start {
            state: None,
            cold_manifest: true,
            ..
        } => {
            let physics = PhysicsWorld::from_entries_with_backend(
                &room_scene.entries,
                loco_mode,
                move_speed,
                sim_backend,
            )
            .map_err(anyhow::Error::msg)?;
            (physics, 0u64)
        }
        sim_state::BootDecision::Start {
            state: Some(state),
            cold_manifest: false,
            import: Some(import),
        } => {
            let resume = state.sim_tick;
            let physics = PhysicsWorld::from_sim_state_with_backend(
                &room_scene.entries,
                loco_mode,
                move_speed,
                sim_backend,
                &state,
                import,
            )
            .map_err(anyhow::Error::msg)?;
            (physics, resume)
        }
        sim_state::BootDecision::Start { .. } => {
            anyhow::bail!("invalid boot decision combination");
        }
    };
    debug_assert_eq!(physics.user_root_slot_count(), user_root_slots);
    let grabbable_n = physics.boxes.grabbable.iter().filter(|&&g| g).count();
    let last_root = USER_ROOT_ENTITY_FIRST + physics.user_root_slot_count() as u32 - 1;
    log::info!(
        "XPBD world ready (backend={} resume_sim_tick={resume_sim_tick} mesh={:?} + user_roots {USER_ROOT_ENTITY_FIRST}..{last_root} + boxes={} grabbable={grabbable_n} mode={:?}); rows={}",
        physics.backend().as_str(),
        physics.mesh_entity_id(),
        physics.boxes.len(),
        physics.loco_mode(),
        physics.entity_count()
    );

    // Seed cache + declare queryables only after boot fetch completed (x00ag review).
    {
        let host_generation = read_publish_gate(&publish_snapshot, Instant::now()).host_generation;
        let seeded = physics.export_sim_state(
            &room_id,
            host_generation,
            manifest_revision,
            resume_sim_tick,
            resume_sim_tick / (PHYSICS_HZ / NET_HZ),
        );
        *lock_or_fatal(&sim_state_cache, "sim_state") = Some(seeded);
    }
    spawn_snapshot_queryable(
        session.clone(),
        snapshot_key_expr,
        Arc::clone(&snapshot_cache),
        Arc::clone(&publish_snapshot),
        host_signing_key.clone(),
        room_id.clone(),
        Arc::clone(&room_aead),
    )?;
    spawn_sim_state_queryable(
        session.clone(),
        sim_state_key_expr,
        Arc::clone(&sim_state_cache),
        Arc::clone(&publish_snapshot),
        host_signing_key.clone(),
        room_id.clone(),
        Arc::clone(&room_aead),
    )?;

    let physics_period = Duration::from_nanos(1_000_000_000 / PHYSICS_HZ);
    let dt = 1.0 / PHYSICS_HZ as f32;
    let mut next_physics = Instant::now();
    let mut host = HostLoop::new(HostLoopParts {
        physics,
        outbound: Arc::clone(&outbound),
        publish_stats,
        presence: Arc::clone(&presence),
        sessions: Arc::clone(&sessions),
        intent_rates: Arc::clone(&intent_rates),
        publish_snapshot: Arc::clone(&publish_snapshot),
        spawn_inbox,
        despawn_inbox,
        shared_scene: Arc::clone(&shared_scene),
        force_reseal: Arc::clone(&force_reseal),
        dt,
        manifest_bytes,
        manifest_revision,
        manifest_epoch,
        last_manifest,
        last_presence,
        last_presence_rev,
        last_publish_denied_log,
        resume_sim_tick,
        room_id: room_id.clone(),
        sim_state_cache: Arc::clone(&sim_state_cache),
    });

    loop {
        if host_ipc::shutdown_requested() {
            log::info!("launcher IPC shutdown requested — leaving main loop (exit 0)");
            break;
        }
        let scheduled = next_physics;
        // Gate clock = pre-sleep `now` (pre-x0099 contract; see HostLoop::tick).
        let now = Instant::now();
        if now < scheduled {
            std::thread::sleep(scheduled - now);
        }
        let started = Instant::now();
        let late_ms = started.saturating_duration_since(scheduled).as_millis() as u64;
        host_metrics.set_tick_lateness_ms(late_ms);
        next_physics = scheduled + physics_period;
        host.tick(now);
    }
    outbound.stop();
    Ok(())
}
