//! Alchemy client — x0029 Frame TRS + x0026 Manifest seal + x0025 attestation.
//!
//! Guarantees:
//! - Home tab login / register / logout; create-world access level + join (x0018 / x0037)
//! - Explicit Leave + Zenoh liveliness on join/logout (x0019); Intent TTL is host fallback
//! - Access + Room Token auto-refresh at ~80% TTL while joined (x0021 P1-1)
//! - Intent Ed25519 session (Token on bind only); Frame finite/tick/sig + snapshot (x0023)
//! - Frame tick gap → do not apply delta; `…/snapshot` replace (x0024 / peer-delivery)
//! - Manifest `world_seal` + `host_attestation` via world JWKS before adopting `host_pub`; Presence `host_sig` (x0026)
//! - Manifest entries drive mesh blake3 / Grab flags / box half-extents (x0027)
//! - RenderFrame Entity carries quat + scale; client applies TRS (x0029)
//! - PROTOCOL_PIN mismatch on Manifest/Presence rejects apply (x0023)
//! - CI fallback: `AUTH_USER` or `ALCHEMY_AUTH_TOKEN` skips UI and joins
//! - Subscribes to SceneManifest / PresenceState / RenderFrame; downloads blob; wgpu mesh
//! - Zenoh keys from `ROOM_ID` via protocol `*_key` helpers (x0017)
//! - Desktop WASD/Space/C → Intent; Grab / OpenXR paths unchanged when joined
//! - Camera follows Presence-assigned local user_root; remotes as colored boxes
//! - Surface boot: OpenXR when runtime present, else Desktop; ESC / Menu dash + Exit
//!
//! Non-guarantees: Steal, XR login keyboard, email verify UI, Quest standalone, WSL HMD,
//! reliable-delivery ACK channel

mod access_level;
mod auth;
mod host_attestation;
mod host_control;
mod intent_session;
mod jwt_sub;
mod manifest_seal;
mod mesh;
mod scene;
mod surface;
mod system_ui;
mod turn;
mod world_api;
mod world_hit;
mod world_jwks;

use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::AtomicU32;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use alchemy_protocol::wire_crypto::{
    frame_entities_finite, manifest_seal_blake3_hex, verify_frame, verify_presence,
};
use alchemy_protocol::{
    decide_frame_tick, frame_key, input_key, liveliness_key, manifest_key, presence_key,
    protocol_pin_matches, scene_entry_quat_xyzw, scene_entry_scale, snapshot_key, validate_room_id,
    FrameTickDecision, Message, PresenceState, RenderFrame, SceneManifest, FRAME_TICK_GAP_RESYNC,
    PROTOCOL_PIN,
};
use alchemy_shared_sim::{scene_entry_color_rgba, ManifestIndex};
use anyhow::{Context, Result};
use auth::AuthGate;
use host_attestation::HostAttestationVerifier;
use manifest_seal::ManifestSealVerifier;
use mesh::{blake3_hex, load_mesh_from_glb, verify_blake3, COIN_BLAKE3_HEX};
use scene::{EntityPose, SharedState, LOCAL_USER_ROOT_UNSET};
use serde::Deserialize;
use world_jwks::WorldJwks;
use zenoh::bytes::ZBytes;
use zenoh::Wait;

const DEFAULT_ZENOH_CONNECT: &str = "tcp/127.0.0.1:7447";
const DEFAULT_WORLD_URL: &str = "http://127.0.0.1:4003";
const DEFAULT_AUTH_URL: &str = "http://127.0.0.1:4002";
const DEFAULT_ASSETS_URL: &str = "http://127.0.0.1:4004";
const DEFAULT_ROOM_ID: &str = "local";
/// Match net frame rate so Grab hold aim stays smooth enough (x0011).
const INPUT_HZ: u64 = 30;
const MANIFEST_WAIT: Duration = Duration::from_millis(400);

#[derive(Debug, Deserialize)]
struct TokenResponse {
    token: String,
    /// Required from world-server; missing field fails deserialize (no silent `0`).
    expires_in: u64,
}

#[derive(Debug, Clone)]
pub struct GuestToken {
    pub token: String,
    pub expires_in: u64,
}

const TOKEN_REFRESH_POLL: Duration = Duration::from_secs(5);

fn zenoh_connect_endpoint() -> String {
    std::env::var("ZENOH_CONNECT").unwrap_or_else(|_| DEFAULT_ZENOH_CONNECT.to_string())
}

fn open_zenoh() -> Result<zenoh::Session> {
    zenoh_connect::open_client(&zenoh_connect_endpoint())
}

pub fn fetch_guest_token(world_url: &str, room_id: &str, auth_token: &str) -> Result<GuestToken> {
    // x0025: env bypass is debug-only (release fail-closed).
    if cfg!(debug_assertions) {
        if let Ok(token) = std::env::var("ALCHEMY_ROOM_TOKEN") {
            if !token.is_empty() {
                return Ok(GuestToken {
                    token,
                    // Env override: no TTL → skip auto-refresh.
                    expires_in: 0,
                });
            }
        }
    }

    // x0021: do not POST /rooms — owner is claimed only by host create.
    let base = world_url.trim_end_matches('/');
    let authz = format!("Bearer {auth_token}");
    let token_url = format!("{base}/api/v1/rooms/{room_id}/tokens");
    let resp: TokenResponse = ureq::post(&token_url)
        .set("content-type", "application/json")
        .set("authorization", &authz)
        .send_json(serde_json::json!({ "role": "guest" }))
        .context("issue guest Room Token (is host up and room created?)")?
        .into_json()
        .context("parse token response")?;
    if resp.expires_in == 0 {
        anyhow::bail!("guest Room Token response missing positive expires_in");
    }
    Ok(GuestToken {
        token: resp.token,
        expires_in: resp.expires_in,
    })
}

fn fetch_blob_bytes(assets_url: &str, blake3: &str, auth_token: &str) -> Result<Vec<u8>> {
    let base = assets_url.trim_end_matches('/');
    let url = format!("{base}/api/v1/blobs/{blake3}");
    let authz = format!("Bearer {auth_token}");
    // Follow 302 → R2 presigned GET (x001d). ureq strips Authorization on cross-host redirect.
    let agent = ureq::AgentBuilder::new().redirects(5).build();
    let resp = agent
        .get(&url)
        .set("authorization", &authz)
        .call()
        .context("assets blob download failed")?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .read_to_end(&mut bytes)
        .context("read blob body")?;
    Ok(bytes)
}

fn load_blob_bytes(assets_url: &str, blake3: &str, auth_token: &str) -> Result<Vec<u8>> {
    match fetch_blob_bytes(assets_url, blake3, auth_token) {
        Ok(bytes) => {
            log::info!(
                "downloaded blob via assets GET ({} bytes, blake3={}; may follow R2 redirect)",
                bytes.len(),
                blake3_hex(&bytes)
            );
            Ok(bytes)
        }
        Err(err) => {
            log::warn!("assets download failed ({err:#}); falling back to fixture");
            load_fixture_bytes()
        }
    }
}

fn load_fixture_bytes() -> Result<Vec<u8>> {
    let path = fixture_path()?;
    std::fs::read(&path).with_context(|| format!("read fixture {}", path.display()))
}

fn fixture_path() -> Result<PathBuf> {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("../fixtures/Coin_PolygonalMind.glb");
    Ok(path)
}

fn client_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn spawn_manifest_subscriber(
    session: zenoh::Session,
    key: String,
    shared: Arc<SharedState>,
    blake3_tx: mpsc::SyncSender<String>,
    seal: Arc<ManifestSealVerifier>,
    attest: Arc<HostAttestationVerifier>,
) -> Result<()> {
    let subscriber = session
        .declare_subscriber(key.clone())
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_subscriber manifest failed: {e}"))?;

    thread::Builder::new()
        .name("zenoh-manifest".into())
        .spawn(move || {
            log::info!("subscribed to {key}");
            let mut sent_blake3 = false;
            // `true` only after a successful snapshot apply for the current `host_pub`.
            let mut snapshot_ok = false;
            while let Ok(sample) = subscriber.recv() {
                let bytes: ZBytes = sample.payload().clone();
                let payload = bytes.to_bytes();
                match SceneManifest::decode(payload.as_ref()) {
                    Ok(manifest) => {
                        if !protocol_pin_matches(&manifest.protocol_pin) {
                            log::error!(
                                "SceneManifest protocol_pin mismatch: got {:?} want {PROTOCOL_PIN}; leaving room",
                                manifest.protocol_pin
                            );
                            shared.leave_room_local();
                            continue;
                        }
                        if manifest.room_id != shared.room_id.as_ref() {
                            log::error!(
                                "SceneManifest room_id mismatch: got {:?} want {}; leaving room",
                                manifest.room_id,
                                shared.room_id
                            );
                            shared.leave_room_local();
                            continue;
                        }
                        if manifest.host_pub.len() != 32 {
                            log::error!(
                                "SceneManifest host_pub length {} (want 32); leaving room",
                                manifest.host_pub.len()
                            );
                            shared.leave_room_local();
                            continue;
                        }
                        let body_hex = manifest_seal_blake3_hex(&manifest);
                        if let Err(err) =
                            seal.verify(&manifest.world_seal, manifest.revision, &body_hex)
                        {
                            log::error!(
                                "SceneManifest world_seal rejected: {err:#}; leaving room"
                            );
                            shared.leave_room_local();
                            continue;
                        }
                        let attested_pub = match attest
                            .verify_and_host_pub(&manifest.host_attestation, &manifest.host_pub)
                        {
                            Ok(pub_bytes) => pub_bytes,
                            Err(err) => {
                                log::error!(
                                    "SceneManifest host_attestation rejected: {err:#}; leaving room"
                                );
                                shared.leave_room_local();
                                continue;
                            }
                        };
                        // Only Some→different Some is a host generation change.
                        // None→Some is first install (do not renew IntentSession).
                        let generation_changed = match shared.host_pub.lock() {
                            Ok(mut slot) => match slot.as_ref() {
                                None => {
                                    *slot = Some(attested_pub.clone());
                                    false
                                }
                                Some(prev) if prev != &attested_pub => {
                                    *slot = Some(attested_pub.clone());
                                    true
                                }
                                Some(_) => false,
                            },
                            Err(err) => {
                                log::error!(
                                    "host_pub lock poisoned; skip generation reset: {err}"
                                );
                                false
                            }
                        };
                        if generation_changed {
                            log::info!(
                                "SceneManifest host_pub changed; reset world + renew Intent session for new host"
                            );
                            shared.reset_for_host_generation();
                            snapshot_ok = false;
                        }
                        if !snapshot_ok {
                            snapshot_ok =
                                fetch_snapshot(&session, shared.room_id.as_ref(), &shared);
                        }
                        log::info!(
                            "SceneManifest revision={} entries={} pin_ok attested snapshot_ok={snapshot_ok}",
                            manifest.revision,
                            manifest.entries.len()
                        );
                        // Classification: ManifestIndex (shared-sim / x0030; priority = x0027).
                        let index = ManifestIndex::from_entries(&manifest.entries);
                        let mesh_id = index.mesh_entity_id.unwrap_or(0);
                        let mut mesh_blake3: Option<String> = None;
                        if let Some(mid) = index.mesh_entity_id {
                            if let Some(entry) =
                                manifest.entries.iter().find(|e| e.entity_id == mid)
                            {
                                if entry.half_extent > 0.0 {
                                    log::warn!(
                                        "scene entry {} has blake3 + half_extent; mesh wins (box ignored)",
                                        entry.entity_id
                                    );
                                }
                                mesh_blake3 = Some(entry.blake3.clone());
                            }
                        }
                        if let Ok(mut world) = shared.world.lock() {
                            for entry in &manifest.entries {
                                let (r, g, b, a) = scene_entry_color_rgba(entry);
                                match world.entities.entry(entry.entity_id) {
                                    std::collections::hash_map::Entry::Vacant(v) => {
                                        let [qx, qy, qz, qw] = scene_entry_quat_xyzw(entry);
                                        let [sx, sy, sz] = scene_entry_scale(entry);
                                        v.insert(EntityPose {
                                            x: entry.x as f32,
                                            y: entry.y as f32,
                                            z: entry.z as f32,
                                            qx: qx as f32,
                                            qy: qy as f32,
                                            qz: qz as f32,
                                            qw: qw as f32,
                                            sx: sx as f32,
                                            sy: sy as f32,
                                            sz: sz as f32,
                                            r,
                                            g,
                                            b,
                                            a,
                                        });
                                    }
                                    std::collections::hash_map::Entry::Occupied(mut o) => {
                                        // Frame owns pose (quat/scale); Manifest revises tint only
                                        // — no TRS reseed on revision (x002c).
                                        if entry.a > 0.0 {
                                            let p = o.get_mut();
                                            p.r = r;
                                            p.g = g;
                                            p.b = b;
                                            p.a = a;
                                        }
                                    }
                                }
                            }
                        }
                        shared.apply_manifest_defs(
                            index.grabbable,
                            index.half_extents,
                            mesh_id,
                        );
                        if let Some(hex) = mesh_blake3 {
                            if !sent_blake3 {
                                let _ = blake3_tx.send(hex);
                                sent_blake3 = true;
                            } else {
                                // Boot download already consumed the channel; live mesh swap is out of scope.
                                log::info!(
                                    "Manifest mesh blake3 updated after boot download; remount not applied"
                                );
                            }
                        }
                    }
                    Err(err) => log::warn!("SceneManifest decode failed: {err}"),
                }
            }
        })
        .context("spawn manifest thread")?;
    Ok(())
}

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
/// On [`ApplyFrameOutcome::NeedResync`], the world is left unchanged.
fn apply_delta_entities(
    world: &mut scene::WorldView,
    tick: u64,
    upserts: &[alchemy_protocol::Entity],
    removes: &[u32],
    replace_entities: bool,
) -> ApplyFrameOutcome {
    if !replace_entities {
        match decide_frame_tick(world.last_frame_tick, tick, FRAME_TICK_GAP_RESYNC) {
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
    );
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
        }
        _ => {}
    }
    outcome
}

/// Returns true when a non-empty snapshot was verified and applied (replace).
fn fetch_snapshot(session: &zenoh::Session, room_id: &str, shared: &SharedState) -> bool {
    let key = snapshot_key(room_id);
    let mut applied = false;
    match session.get(&key).wait() {
        Ok(receiver) => {
            while let Ok(reply) = receiver.recv() {
                match reply.result() {
                    Ok(sample) => {
                        let bytes: ZBytes = sample.payload().clone();
                        match RenderFrame::decode(bytes.to_bytes().as_ref()) {
                            Ok(frame) => {
                                if apply_frame(shared, frame, /* replace */ true)
                                    == ApplyFrameOutcome::Applied
                                {
                                    log::info!("snapshot applied from {key}");
                                    applied = true;
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
    if !applied {
        log::warn!("snapshot not applied yet; will retry on next Manifest or frame gap");
    }
    applied
}

/// Minimum spacing between gap-triggered snapshot gets (x0024). Applied **before**
/// `fetch_snapshot` so success and failure share the same 500ms wait (load shed;
/// see `protocol/peer-delivery.md`). Manifest path can still retry independently.
const SNAPSHOT_GAP_RESYNC_COOLDOWN: Duration = Duration::from_millis(500);

fn spawn_frame_subscriber(
    session: zenoh::Session,
    key: String,
    shared: Arc<SharedState>,
) -> Result<()> {
    let subscriber = session
        .declare_subscriber(key.clone())
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_subscriber frame failed: {e}"))?;

    thread::Builder::new()
        .name("zenoh-frame".into())
        .spawn(move || {
            log::info!("subscribed to {key} (signed delta upserts/removes)");
            let mut resync_cooldown_until = Instant::now();
            while let Ok(sample) = subscriber.recv() {
                let bytes: ZBytes = sample.payload().clone();
                let payload = bytes.to_bytes();
                match RenderFrame::decode(payload.as_ref()) {
                    Ok(frame) => match apply_frame(&shared, frame, /* replace */ false) {
                        ApplyFrameOutcome::Applied | ApplyFrameOutcome::Dropped => {}
                        ApplyFrameOutcome::NeedResync => {
                            // x0024: missing deltas are not retransmitted — resync via snapshot.
                            let now = Instant::now();
                            if now < resync_cooldown_until {
                                continue;
                            }
                            resync_cooldown_until = now + SNAPSHOT_GAP_RESYNC_COOLDOWN;
                            let _ = fetch_snapshot(&session, shared.room_id.as_ref(), &shared);
                        }
                    },
                    Err(err) => log::warn!("RenderFrame decode failed: {err}"),
                }
            }
        })
        .context("spawn frame thread")?;
    Ok(())
}

fn spawn_presence_subscriber(
    session: zenoh::Session,
    key: String,
    shared: Arc<SharedState>,
) -> Result<()> {
    let subscriber = session
        .declare_subscriber(key.clone())
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_subscriber presence failed: {e}"))?;

    thread::Builder::new()
        .name("zenoh-presence".into())
        .spawn(move || {
            let _session = session;
            log::info!("subscribed to {key} (Presence; local_sub from AuthGate)");
            while let Ok(sample) = subscriber.recv() {
                let bytes: ZBytes = sample.payload().clone();
                let payload = bytes.to_bytes();
                match PresenceState::decode(payload.as_ref()) {
                    Ok(state) => {
                        if !protocol_pin_matches(&state.protocol_pin) {
                            log::error!(
                                "PresenceState protocol_pin mismatch: got {:?} want {PROTOCOL_PIN}; leaving room",
                                state.protocol_pin
                            );
                            shared.leave_room_local();
                            continue;
                        }
                        let host_pub = shared.host_pub.lock().ok().and_then(|g| g.clone());
                        let Some(host_pub) = host_pub else {
                            log::warn!("drop PresenceState: host_pub not yet attested");
                            continue;
                        };
                        if let Err(err) = verify_presence(&host_pub, &state) {
                            log::warn!("drop PresenceState: bad host_sig: {err}");
                            continue;
                        }
                        let local_sub = shared
                            .auth
                            .lock()
                            .ok()
                            .and_then(|g| g.local_sub.clone())
                            .unwrap_or_default();
                        if local_sub.is_empty() {
                            if shared.local_user_root() != LOCAL_USER_ROOT_UNSET {
                                shared.set_local_user_root(LOCAL_USER_ROOT_UNSET);
                            }
                            continue;
                        }
                        let mine = state
                            .assignments
                            .iter()
                            .find(|a| a.sub == local_sub)
                            .map(|a| a.entity_id);
                        match mine {
                            Some(entity_id) => {
                                let prev = shared.local_user_root();
                                if prev != entity_id {
                                    shared.set_local_user_root(entity_id);
                                    log::info!(
                                        "Presence: local sub={local_sub} → user_root entity {entity_id} (revision={})",
                                        state.revision
                                    );
                                }
                            }
                            None => {
                                // Rebind only when we *lost* a prior assignment (not first-join wait).
                                let had_slot =
                                    shared.local_user_root() != LOCAL_USER_ROOT_UNSET;
                                if had_slot {
                                    shared.set_local_user_root(LOCAL_USER_ROOT_UNSET);
                                    log::warn!(
                                        "Presence: local sub={local_sub} left assignments (revision={})",
                                        state.revision
                                    );
                                    let joined = shared
                                        .auth
                                        .lock()
                                        .ok()
                                        .map(|g| g.is_joined())
                                        .unwrap_or(false);
                                    if joined {
                                        shared.request_intent_rebind();
                                    }
                                }
                            }
                        }
                    }
                    Err(err) => log::warn!("PresenceState decode failed: {err}"),
                }
            }
        })
        .context("spawn presence thread")?;
    Ok(())
}

fn spawn_input_publisher(
    session: zenoh::Session,
    key: String,
    shared: Arc<SharedState>,
) -> Result<()> {
    let publisher = session
        .declare_publisher(key.clone())
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_publisher input failed: {e}"))?;

    thread::Builder::new()
        .name("zenoh-input-pub".into())
        .spawn(move || {
            log::info!(
                "publishing InputEvent intent to {key} @ {INPUT_HZ}Hz (session Ed25519; Token on bind)"
            );
            let period = Duration::from_nanos(1_000_000_000 / INPUT_HZ);
            let mut next = Instant::now();
            loop {
                let now = Instant::now();
                if now < next {
                    thread::sleep(next - now);
                }
                next += period;

                let room_token = shared.auth.lock().ok().and_then(|g| g.room_token.clone());
                let Some(room_token) = room_token else {
                    continue;
                };
                let Some(event) = (|| {
                    let mut slot = shared.intent_session.lock().ok()?;
                    let session = slot.as_mut()?;
                    let intent = shared.intent.lock().ok().map(|g| *g).unwrap_or_default();
                    let body = if intent.suppress {
                        intent_session::IntentBody {
                            client_time_ms: client_time_ms(),
                            ..Default::default()
                        }
                    } else {
                        intent_session::IntentBody {
                            client_time_ms: client_time_ms(),
                            move_x: intent.move_x,
                            move_y: intent.move_y,
                            move_z: intent.move_z,
                            buttons: intent.buttons,
                            grab_entity_id: intent.grab_entity_id,
                            aim_x: intent.aim_x,
                            aim_y: intent.aim_y,
                            aim_z: intent.aim_z,
                            leave: false,
                        }
                    };
                    Some(session.signed_intent(Some(&room_token), body))
                })() else {
                    continue;
                };
                let was_bind = !event.room_token.is_empty();
                if let Err(err) = publisher.put(event.encode_to_vec()).wait() {
                    log::warn!("input publish failed: {err}");
                    // Keep bind_pending so the next Intent still carries room_token.
                    continue;
                }
                if was_bind {
                    if let Ok(mut slot) = shared.intent_session.lock() {
                        if let Some(s) = slot.as_mut() {
                            s.note_bind_put_ok();
                        }
                    }
                }
                // put().wait() = router accept, not host verify. Liveliness rebind only after a
                // bind Intent put (Token on wire) so refresh without bind does not rotate jti early.
                let (need_rebind, need_retire_prev) = shared
                    .auth
                    .lock()
                    .ok()
                    .map(|g| (g.liveliness_rebind, g.liveliness_retire_prev))
                    .unwrap_or((false, false));
                if need_retire_prev {
                    if let Ok(mut prev) = shared.liveliness_prev.lock() {
                        *prev = None;
                    }
                    if let Ok(mut gate) = shared.auth.lock() {
                        gate.liveliness_retire_prev = false;
                    }
                }
                if need_rebind && was_bind {
                    match declare_room_alive(&shared, &session, /* retain_previous */ true) {
                        Ok(()) => {
                            if let Ok(mut gate) = shared.auth.lock() {
                                gate.liveliness_rebind = false;
                                gate.liveliness_retire_prev = true;
                            }
                        }
                        Err(err) => log::warn!("liveliness rebind after refresh failed: {err:#}"),
                    }
                }
            }
        })
        .context("spawn input publisher")?;
    Ok(())
}

/// Apply auth session + guest Room Token into SharedState (x0018 join).
pub fn join_room_with_access(shared: &SharedState, access_token: &str) -> Result<()> {
    let guest = fetch_guest_token(&shared.world_url, &shared.room_id, access_token)
        .context("fetch guest Room Token")?;
    let local_sub = jwt_sub::sub_unverified(&guest.token)
        .context("read Room Token sub (unverified payload)")?;
    if let Ok(mut gate) = shared.auth.lock() {
        gate.apply_room(guest.token, local_sub.clone(), guest.expires_in);
    }
    if let Ok(mut session) = shared.intent_session.lock() {
        *session = Some(intent_session::IntentSession::new());
    }
    log::info!(
        "joined room={} sub={local_sub} expires_in={}",
        shared.room_id,
        guest.expires_in
    );
    Ok(())
}

/// Background access + Room Token refresh at ~80% TTL (x0021 P1-1).
fn spawn_token_refresh(shared: Arc<SharedState>) -> Result<()> {
    thread::Builder::new()
        .name("token-refresh".into())
        .spawn(move || {
            log::info!("token refresh loop every {}s", TOKEN_REFRESH_POLL.as_secs());
            loop {
                thread::sleep(TOKEN_REFRESH_POLL);
                let now = Instant::now();

                let need_access = shared
                    .auth
                    .lock()
                    .ok()
                    .and_then(|g| g.access_deadline.map(|d| now >= d))
                    .unwrap_or(false);
                if need_access {
                    let refresh_token = shared
                        .auth
                        .lock()
                        .ok()
                        .and_then(|g| g.refresh_token.clone());
                    match refresh_token {
                        Some(rt) => match auth::refresh(&shared.auth_url, &rt) {
                            Ok(session) => {
                                if let Ok(mut gate) = shared.auth.lock() {
                                    gate.apply_auth_session(session);
                                }
                                log::info!("access token refreshed");
                            }
                            Err(err) => {
                                log::warn!("access token refresh failed: {err:#}");
                                if let Ok(mut gate) = shared.auth.lock() {
                                    // Back off one poll interval to avoid tight failure loops.
                                    gate.access_deadline = Some(now + TOKEN_REFRESH_POLL);
                                }
                            }
                        },
                        None => {
                            // No refresh_token (e.g. silent env JWT): stop trying access refresh.
                            if let Ok(mut gate) = shared.auth.lock() {
                                gate.access_deadline = None;
                            }
                        }
                    }
                }

                let need_room = shared
                    .auth
                    .lock()
                    .ok()
                    .map(|g| g.is_joined() && g.room_deadline.map(|d| now >= d).unwrap_or(false))
                    .unwrap_or(false);
                if need_room {
                    let access = shared.auth.lock().ok().and_then(|g| g.access_token.clone());
                    let Some(access) = access else {
                        if let Ok(mut gate) = shared.auth.lock() {
                            gate.room_deadline = Some(now + TOKEN_REFRESH_POLL);
                        }
                        continue;
                    };
                    match fetch_guest_token(&shared.world_url, &shared.room_id, &access) {
                        Ok(guest) => {
                            let local_sub = match jwt_sub::sub_unverified(&guest.token) {
                                Ok(s) => s,
                                Err(err) => {
                                    log::warn!("Room Token refresh: bad sub: {err:#}");
                                    if let Ok(mut gate) = shared.auth.lock() {
                                        gate.room_deadline = Some(now + TOKEN_REFRESH_POLL);
                                    }
                                    continue;
                                }
                            };
                            if let Ok(mut gate) = shared.auth.lock() {
                                if gate.is_joined() {
                                    gate.apply_room_refresh(
                                        guest.token,
                                        local_sub,
                                        guest.expires_in,
                                    );
                                    if let Ok(mut sess) = shared.intent_session.lock() {
                                        if let Some(s) = sess.as_mut() {
                                            s.mark_bind_pending();
                                        }
                                    }
                                    log::info!(
                                        "Room Token refreshed expires_in={} (session rebind + liveliness pending)",
                                        guest.expires_in
                                    );
                                }
                            }
                        }
                        Err(err) => {
                            log::warn!("Room Token refresh failed: {err:#}");
                            if let Ok(mut gate) = shared.auth.lock() {
                                gate.room_deadline = Some(now + TOKEN_REFRESH_POLL);
                            }
                        }
                    }
                }
            }
        })
        .context("spawn token refresh")?;
    Ok(())
}

/// Declare Zenoh liveliness for current Room Token `sub`+`jti` (x0022). No-op if not joined.
///
/// `retain_previous`: only for Room Token **refresh** rebind. Stashes the old token in
/// `liveliness_prev` until the next Intent put. Join / re-login pass `false` and drop any
/// previous key immediately (no dual-key leak until leave).
pub fn declare_room_alive(
    shared: &SharedState,
    session: &zenoh::Session,
    retain_previous: bool,
) -> Result<()> {
    let room_token = shared
        .auth
        .lock()
        .ok()
        .and_then(|g| g.room_token.clone())
        .context("declare liveliness: not joined")?;
    let (sub, jti) =
        jwt_sub::sub_jti_unverified(&room_token).context("declare liveliness: read sub/jti")?;
    let key = liveliness_key(shared.room_id.as_ref(), &sub, &jti);
    let token = session
        .liveliness()
        .declare_token(key.clone())
        .wait()
        .map_err(|e| anyhow::anyhow!("declare liveliness token failed: {e}"))?;
    if let Ok(mut slot) = shared.liveliness.lock() {
        let old = slot.replace(token);
        if let Some(old) = old {
            if retain_previous {
                if let Ok(mut prev) = shared.liveliness_prev.lock() {
                    *prev = Some(old);
                }
            }
            // else: drop `old` immediately (join / re-login)
        }
    }
    log::info!("liveliness declared key={key} retain_previous={retain_previous}");
    Ok(())
}

/// Explicit Leave put + liveliness undeclare + local clear (x0019 / x0023 session).
pub fn leave_room(shared: &SharedState, session: &zenoh::Session) {
    let room_token = shared.auth.lock().ok().and_then(|g| g.room_token.clone());
    let leave_event = {
        let mut slot = shared.intent_session.lock().ok();
        slot.as_mut().and_then(|opt| {
            let sess = opt.as_mut()?;
            Some(sess.signed_intent(
                room_token.as_deref(),
                intent_session::IntentBody {
                    client_time_ms: client_time_ms(),
                    leave: true,
                    ..Default::default()
                },
            ))
        })
    };
    if let Some(event) = leave_event {
        let key = input_key(shared.room_id.as_ref());
        match session.declare_publisher(key.clone()).wait() {
            Ok(publisher) => {
                if let Err(err) = publisher.put(event.encode_to_vec()).wait() {
                    log::warn!("Leave InputEvent publish failed: {err}");
                } else {
                    log::info!("Leave InputEvent published on {key}");
                }
            }
            Err(err) => log::warn!("Leave publisher declare failed: {err}"),
        }
    }
    // Drop liveliness before clearing auth so host sees Delete promptly.
    if let Ok(mut token) = shared.liveliness.lock() {
        *token = None;
    }
    if let Ok(mut prev) = shared.liveliness_prev.lock() {
        *prev = None;
    }
    shared.leave_room_local();
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let world_url = std::env::var("WORLD_URL").unwrap_or_else(|_| DEFAULT_WORLD_URL.to_string());
    let auth_url = std::env::var("AUTH_URL").unwrap_or_else(|_| DEFAULT_AUTH_URL.to_string());
    let assets_url = std::env::var("ASSETS_URL").unwrap_or_else(|_| DEFAULT_ASSETS_URL.to_string());
    let room_id_raw = std::env::var("ROOM_ID").unwrap_or_else(|_| DEFAULT_ROOM_ID.to_string());
    let room_id = validate_room_id(&room_id_raw)
        .map_err(|e| anyhow::anyhow!("invalid ROOM_ID={room_id_raw:?}: {e}"))?
        .to_string();
    let fallback_blake3 =
        std::env::var("ALCHEMY_BLOB_BLAKE3").unwrap_or_else(|_| COIN_BLAKE3_HEX.to_string());

    let manifest_key_expr = manifest_key(&room_id);
    let frame_key_expr = frame_key(&room_id);
    let input_key_expr = input_key(&room_id);
    let presence_key_expr = presence_key(&room_id);

    let silent = auth::try_silent_session(&auth_url).context("silent auth")?;
    let auth_mode = if silent.is_some() { "silent" } else { "ui" };
    log::info!(
        "client x002e starting; protocol_pin={PROTOCOL_PIN} room={room_id} auth_mode={auth_mode} manifest={manifest_key_expr} frame={frame_key_expr} input={input_key_expr} presence={presence_key_expr}"
    );

    let mut gate = AuthGate::default();
    if let Some(session) = silent {
        gate.apply_auth_session(session);
    }

    let shared = Arc::new(SharedState {
        local_user_root: AtomicU32::new(LOCAL_USER_ROOT_UNSET),
        room_id: Arc::<str>::from(room_id),
        auth_url: Arc::<str>::from(auth_url),
        world_url: Arc::<str>::from(world_url),
        assets_url: Arc::<str>::from(assets_url.clone()),
        auth: Mutex::new(gate),
        intent_session: Mutex::new(None),
        host_pub: Mutex::new(None),
        liveliness: Mutex::new(None),
        liveliness_prev: Mutex::new(None),
        world: Mutex::new(scene::WorldView::default()),
        box_defs: Mutex::new(Arc::new(scene::BoxDefs::default())),
        mesh_entity_id: AtomicU32::new(0),
        look: Mutex::new(Default::default()),
        intent: Mutex::new(Default::default()),
    });

    if shared
        .auth
        .lock()
        .ok()
        .and_then(|g| g.access_token.clone())
        .is_some()
    {
        let access = shared
            .auth
            .lock()
            .ok()
            .and_then(|g| g.access_token.clone())
            .expect("access just checked");
        join_room_with_access(&shared, &access).context("silent join room")?;
    }

    let (blake3_tx, blake3_rx) = mpsc::sync_channel::<String>(1);

    spawn_token_refresh(Arc::clone(&shared))?;

    let jwks = Arc::new(WorldJwks::new(shared.world_url.as_ref()));
    jwks.refresh()
        .context("initial world JWKS (ManifestSeal + HostAttestation)")?;
    let seal = Arc::new(ManifestSealVerifier::new(
        Arc::clone(&jwks),
        shared.room_id.as_ref(),
    ));
    let attest = Arc::new(HostAttestationVerifier::new(
        Arc::clone(&jwks),
        shared.room_id.as_ref(),
    ));

    let session = open_zenoh().context("open zenoh session")?;
    if shared
        .auth
        .lock()
        .ok()
        .map(|g| g.is_joined())
        .unwrap_or(false)
    {
        declare_room_alive(&shared, &session, false).context("silent liveliness")?;
    }
    spawn_manifest_subscriber(
        session.clone(),
        manifest_key_expr,
        Arc::clone(&shared),
        blake3_tx,
        Arc::clone(&seal),
        Arc::clone(&attest),
    )?;
    spawn_presence_subscriber(session.clone(), presence_key_expr, Arc::clone(&shared))?;
    spawn_frame_subscriber(session.clone(), frame_key_expr, Arc::clone(&shared))?;
    spawn_input_publisher(session.clone(), input_key_expr, Arc::clone(&shared))?;

    let blob_blake3 = match blake3_rx.recv_timeout(MANIFEST_WAIT) {
        Ok(hex) => {
            let mesh_id = shared.mesh_entity_id();
            log::info!("Manifest blake3 for mesh entity {mesh_id}: {hex}");
            hex
        }
        Err(_) => {
            log::warn!(
                "no SceneManifest within {:?}; using fallback blake3={fallback_blake3}",
                MANIFEST_WAIT
            );
            fallback_blake3
        }
    };

    let access_for_blob = shared
        .auth
        .lock()
        .ok()
        .and_then(|g| g.access_token.clone())
        .unwrap_or_default();
    let bytes = {
        let primary = if access_for_blob.is_empty() {
            log::info!("no auth yet — loading fixture mesh (login via Home tab)");
            load_fixture_bytes()?
        } else {
            load_blob_bytes(&assets_url, &blob_blake3, &access_for_blob)?
        };
        match verify_blake3(&primary, &blob_blake3) {
            Ok(()) => primary,
            Err(_) => {
                let fixture = load_fixture_bytes()?;
                verify_blake3(&fixture, COIN_BLAKE3_HEX).context("fixture blake3")?;
                fixture
            }
        }
    };
    let mesh = load_mesh_from_glb(&bytes).context("decode GLB")?;
    log::info!(
        "mesh ready: {} verts, {} indices, radius={:.3}",
        mesh.positions.len(),
        mesh.indices.len(),
        mesh.radius
    );

    let kind = surface::choose();
    surface::run(kind, mesh, shared, Some(session))?;
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

        let outcome = apply_delta_entities(&mut world, 12, &[sample_entity(1, 9.0)], &[], false);

        assert_eq!(outcome, ApplyFrameOutcome::NeedResync);
        assert_eq!(world.last_frame_tick, Some(10));
        assert_eq!(world.entities, before);
    }

    #[test]
    fn consecutive_delta_applies_and_advances_tick() {
        let mut world = WorldView {
            last_frame_tick: Some(10),
            entities: Default::default(),
        };

        let outcome = apply_delta_entities(&mut world, 11, &[sample_entity(7, 3.0)], &[], false);

        assert_eq!(outcome, ApplyFrameOutcome::Applied);
        assert_eq!(world.last_frame_tick, Some(11));
        assert!((world.entities.get(&7).unwrap().x - 3.0).abs() < f32::EPSILON);
    }

    #[test]
    fn delta_applies_quat_and_scale() {
        let mut world = WorldView {
            last_frame_tick: Some(1),
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

        let outcome = apply_delta_entities(&mut world, 2, &[e], &[], false);

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
            entities: [(1, EntityPose::default())].into(),
        };

        let outcome = apply_delta_entities(&mut world, 5, &[sample_entity(2, 1.5)], &[], true);

        assert_eq!(outcome, ApplyFrameOutcome::Applied);
        assert_eq!(world.last_frame_tick, Some(5));
        assert!(!world.entities.contains_key(&1));
        assert!(world.entities.contains_key(&2));
    }

    /// Host must stamp snapshot with *last published* tick. If it wrongly used the next
    /// unpublished `net_tick`, the following delta would share that tick and DropStale —
    /// entity updates stall forever (x0024 regression).
    #[test]
    fn after_snapshot_next_published_tick_applies_same_tick_is_stale() {
        let mut world = WorldView::default();
        let snap = apply_delta_entities(&mut world, 5, &[sample_entity(1, 1.0)], &[], true);
        assert_eq!(snap, ApplyFrameOutcome::Applied);
        assert_eq!(world.last_frame_tick, Some(5));

        // Wrong host: snapshot.tick == next delta.tick → DropStale, pose stuck.
        let same = apply_delta_entities(&mut world, 5, &[sample_entity(1, 9.0)], &[], false);
        assert_eq!(same, ApplyFrameOutcome::Dropped);
        assert!((world.entities.get(&1).unwrap().x - 1.0).abs() < f32::EPSILON);

        // Correct host: next publish is last_published+1.
        let next = apply_delta_entities(&mut world, 6, &[sample_entity(1, 9.0)], &[], false);
        assert_eq!(next, ApplyFrameOutcome::Applied);
        assert_eq!(world.last_frame_tick, Some(6));
        assert!((world.entities.get(&1).unwrap().x - 9.0).abs() < f32::EPSILON);
    }
}
