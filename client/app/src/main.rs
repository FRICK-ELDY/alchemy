//! Alchemy client — x0029 Frame TRS + x0026 Manifest seal + x0025 attestation.
//!
//! Guarantees:
//! - Home tab login / register / logout; create-world access level + join (x0018 / x0037)
//! - Explicit Leave + Zenoh liveliness on join/logout (x0019); Intent TTL is host fallback
//! - Access + Room Token auto-refresh at ~80% TTL while joined (x0021 P1-1)
//! - Intent Ed25519 session (Token on bind only); Frame finite/tick/sig + snapshot (x0023)
//! - Frame tick gap → do not apply delta; `…/snapshot` replace (x0024 / peer-delivery)
//! - Manifest `world_seal` + `host_attestation` via world JWKS before adopting `host_pub`; Presence `host_sig` (x0026)
//! - Manifest / Presence `(host_generation, revision)` monotonic gate; regression → discard (x003a)
//! - Manifest / Presence verify failure → discard (keep joined); leave only on explicit Leave (x0038)
//! - Class-specific Zenoh decode byte/count limits → drop (x003b)
//! - Manifest entries drive mesh blake3 / Grab flags / box half-extents (x0027)
//! - RenderFrame Entity carries quat + scale; client applies TRS (x0029)
//! - PROTOCOL_PIN mismatch on Manifest/Presence rejects apply (x0023 / x0038 discard)
//! - CI fallback: `AUTH_USER` or `ALCHEMY_AUTH_TOKEN` skips UI and joins
//! - Subscribes to SceneManifest / PresenceState / RenderFrame; downloads blob; wgpu mesh
//! - Authenticated assets GET: connect/read timeout; failure → error placeholder mesh + dash
//!   status (not Coin fixture). Fixture fallback only with debug + `ALCHEMY_ALLOW_FIXTURE_FALLBACK`
//!   (x0048 / P3-3)
//! - Zenoh keys from `ROOM_ID` via protocol `*_key` helpers (x0017)
//! - Desktop WASD/Space/C → Intent; Grab / OpenXR paths unchanged when joined
//! - Camera follows Presence-assigned local user_root; remotes as colored boxes
//! - Surface boot: OpenXR when runtime present, else Desktop; ESC / Menu dash + Exit
//!
//! Non-guarantees: Steal, XR login keyboard, email verify UI, Quest standalone, WSL HMD,
//! reliable-delivery ACK channel, live mesh swap / cancel mid-stream beyond HTTP timeout

mod access_level;
mod auth;
mod host_attestation;
mod host_control;
mod intent_session;
mod jwt_sub;
mod manifest_seal;
mod mesh;
mod metrics;
mod scene;
mod surface;
mod system_ui;
mod turn;
mod world_api;
mod world_hit;
mod world_jwks;
mod world_token;

use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use alchemy_protocol::decode_limits::{
    check_frame, check_manifest, check_payload_bytes, check_presence, WireClass,
};
use alchemy_protocol::generation_gate::{
    decide_gen_rev, decide_presence_gen_rev, GenRev, GenRevDecision,
};
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
use mesh::{
    blake3_hex, load_mesh_from_glb, placeholder_error_mesh, verify_blake3, CpuMesh, COIN_BLAKE3_HEX,
};
use scene::{EntityPose, SharedState, LOCAL_USER_ROOT_UNSET};
use world_jwks::WorldJwks;
use zenoh::bytes::ZBytes;
use zenoh::Wait;

const DEFAULT_ROOM_ID: &str = "local";
/// Match net frame rate so Grab hold aim stays smooth enough (x0011).
const INPUT_HZ: u64 = 30;
const MANIFEST_WAIT: Duration = Duration::from_millis(400);
/// Auth join: cover host presence-join Manifest put (Intent → assign/jti-rotate → put).
/// Blake3 is sent before snapshot GET; this wait is not snapshot-bound.
const MANIFEST_WAIT_AUTH: Duration = Duration::from_secs(2);

const TOKEN_REFRESH_POLL: Duration = Duration::from_secs(5);
const GATE_LOG_INTERVAL: Duration = Duration::from_secs(5);
/// Assets blob GET: connect bound (hung TCP / dead assets).
const ASSETS_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Assets blob GET: read bound (cancels stalled body / R2 transfer — x0048).
const ASSETS_HTTP_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// x0038: Manifest identity checks. Failure → discard sample (never leave).
fn gate_manifest_identity(
    protocol_pin: &str,
    room_id: &str,
    host_pub_len: usize,
    expected_room: &str,
) -> Result<(), &'static str> {
    if !protocol_pin_matches(protocol_pin) {
        return Err("protocol_pin mismatch");
    }
    if room_id != expected_room {
        return Err("room_id mismatch");
    }
    if host_pub_len != 32 {
        return Err("host_pub length");
    }
    Ok(())
}

/// x0038: Presence pin check. Failure → discard sample (never leave).
fn gate_presence_pin(protocol_pin: &str) -> Result<(), &'static str> {
    if !protocol_pin_matches(protocol_pin) {
        return Err("protocol_pin mismatch");
    }
    Ok(())
}

/// Persist + apply home-node endpoints while unjoined (x0066).
///
/// Order: validate → probe JWKS → reopen Zenoh → commit SharedState → `save_file` →
/// optional session clear. Probe / SharedState / persist failures roll JWKS / Zenoh /
/// SharedState back to the pre-Apply snapshot (best effort). Session clear runs only
/// after durable persist so a failed Apply never clears login. Does **not** call
/// `apply_to_env`.
///
/// Returns whether the auth base changed (UI must logout). When true, session tokens
/// are cleared after persist (poisoned auth lock → `into_inner`, still clear).
pub fn apply_connect_profile(
    shared: &SharedState,
    zenoh: &mut Option<zenoh::Session>,
    profile: &alchemy_connect_config::ConnectProfile,
) -> Result<bool> {
    // Fail-closed: do not Apply against a poisoned AuthGate (may be mid-panic residue).
    let joined = shared
        .auth
        .lock()
        .map_err(|_| anyhow::anyhow!("auth lock poisoned; refusing Connect Apply"))?
        .is_joined();
    if joined {
        anyhow::bail!("leave the room before applying Connect endpoints");
    }

    let mut profile = profile.clone();
    profile.sanitize();
    profile.sync_empty_allocate_hints();
    alchemy_connect_config::validate_for_apply(&profile)?;

    let prev = ConnectEndpointSnap {
        auth: shared.auth_url_snap(),
        world: shared.world_url_snap(),
        assets: shared.assets_url_snap(),
        zenoh: shared.zenoh_connect_snap(),
        allocate: shared.relay_allocate_snap(),
    };
    let auth_changed = profile.auth_url.as_str() != prev.auth.as_ref();

    // --- Probes (rollback JWKS / Zenoh on failure; SharedState untouched) ---
    shared.world_jwks.retarget_world(&profile.world_url);
    if let Err(err) = shared.world_jwks.refresh() {
        restore_jwks(shared, prev.world.as_ref());
        return Err(err).context("refresh world JWKS after Connect Apply");
    }

    close_zenoh(zenoh);
    match open_zenoh(profile.selected_zenoh()) {
        Ok(session) => *zenoh = Some(session),
        Err(err) => {
            restore_jwks(shared, prev.world.as_ref());
            restore_zenoh(zenoh, prev.zenoh.as_ref());
            return Err(err).context("reopen zenoh after Connect Apply");
        }
    }
    shared.mtls_reopen_pending.store(false, Ordering::SeqCst);

    // --- Commit SharedState (all locks held before any write) ---
    if let Err(err) = commit_endpoint_slots(shared, &profile) {
        restore_jwks(shared, prev.world.as_ref());
        restore_zenoh(zenoh, prev.zenoh.as_ref());
        return Err(err);
    }

    // --- Persist; on failure roll SharedState + probes back (session untouched) ---
    let path = alchemy_connect_config::default_config_path();
    if let Err(err) = alchemy_connect_config::save_file(&path, &profile) {
        if let Err(restore_err) = commit_endpoint_snap(shared, &prev) {
            log::warn!(
                "Connect Apply SharedState rollback failed after persist error: {restore_err:#}"
            );
            restore_jwks(shared, prev.world.as_ref());
            restore_zenoh(zenoh, prev.zenoh.as_ref());
            return Err(err).context(format!(
                "persist connect.json after Connect Apply; SharedState rollback also failed: {restore_err:#}"
            ));
        }
        restore_jwks(shared, prev.world.as_ref());
        restore_zenoh(zenoh, prev.zenoh.as_ref());
        return Err(err).context("persist connect.json after Connect Apply");
    }

    // After durable commit only: always clear tokens when auth base moved (poison → into_inner).
    if auth_changed {
        shared
            .auth
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear_session();
    }

    log::info!(
        "Connect applied auth={} assets={} world={} zenoh={} allocate={:?} path={} auth_changed={auth_changed}",
        profile.auth_url,
        shared.assets_url_snap(),
        profile.world_url,
        profile.selected_zenoh(),
        profile.selected_allocate_url(),
        path.display()
    );
    Ok(auth_changed)
}

struct ConnectEndpointSnap {
    auth: Arc<str>,
    world: Arc<str>,
    assets: Arc<str>,
    zenoh: Arc<str>,
    allocate: Option<Arc<str>>,
}

fn restore_jwks(shared: &SharedState, world_url: &str) {
    shared.world_jwks.retarget_world(world_url);
    if let Err(err) = shared.world_jwks.refresh() {
        log::warn!("Connect Apply JWKS restore failed: {err:#}");
    }
}

fn restore_zenoh(zenoh: &mut Option<zenoh::Session>, endpoint: &str) {
    close_zenoh(zenoh);
    match open_zenoh(endpoint) {
        Ok(session) => *zenoh = Some(session),
        Err(err) => {
            log::warn!("Connect Apply zenoh restore failed: {err:#}");
            *zenoh = None;
        }
    }
}

fn commit_endpoint_slots(
    shared: &SharedState,
    profile: &alchemy_connect_config::ConnectProfile,
) -> Result<()> {
    let mut auth = shared
        .auth_url
        .lock()
        .map_err(|_| anyhow::anyhow!("auth_url lock poisoned"))?;
    let mut world = shared
        .world_url
        .lock()
        .map_err(|_| anyhow::anyhow!("world_url lock poisoned"))?;
    let mut assets = shared
        .assets_url
        .lock()
        .map_err(|_| anyhow::anyhow!("assets_url lock poisoned"))?;
    let mut zenoh_ep = shared
        .zenoh_connect
        .lock()
        .map_err(|_| anyhow::anyhow!("zenoh_connect lock poisoned"))?;
    let mut allocate = shared
        .relay_allocate_url
        .lock()
        .map_err(|_| anyhow::anyhow!("relay_allocate_url lock poisoned"))?;
    *auth = Arc::<str>::from(profile.auth_url.as_str());
    *world = Arc::<str>::from(profile.world_url.as_str());
    *assets = Arc::<str>::from(profile.assets_url.as_str());
    *zenoh_ep = Arc::<str>::from(profile.selected_zenoh());
    *allocate = profile.selected_allocate_url().map(Arc::<str>::from);
    Ok(())
}

fn commit_endpoint_snap(shared: &SharedState, snap: &ConnectEndpointSnap) -> Result<()> {
    let mut auth = shared
        .auth_url
        .lock()
        .map_err(|_| anyhow::anyhow!("auth_url lock poisoned"))?;
    let mut world = shared
        .world_url
        .lock()
        .map_err(|_| anyhow::anyhow!("world_url lock poisoned"))?;
    let mut assets = shared
        .assets_url
        .lock()
        .map_err(|_| anyhow::anyhow!("assets_url lock poisoned"))?;
    let mut zenoh_ep = shared
        .zenoh_connect
        .lock()
        .map_err(|_| anyhow::anyhow!("zenoh_connect lock poisoned"))?;
    let mut allocate = shared
        .relay_allocate_url
        .lock()
        .map_err(|_| anyhow::anyhow!("relay_allocate_url lock poisoned"))?;
    *auth = Arc::clone(&snap.auth);
    *world = Arc::clone(&snap.world);
    *assets = Arc::clone(&snap.assets);
    *zenoh_ep = Arc::clone(&snap.zenoh);
    *allocate = snap.allocate.clone();
    Ok(())
}

/// Explicit close so bus threads stop even when another strong `Session` clone still exists.
///
/// Zenoh 1.x only auto-`close`s on `Drop` when `strong_counter == 1`. Leaving App's slot as
/// `None` alone is not enough if a spawn_* thread retained a clone (see presence historically).
fn close_zenoh(zenoh: &mut Option<zenoh::Session>) {
    if let Some(session) = zenoh.take() {
        if let Err(err) = session.close().wait() {
            log::warn!("zenoh session close failed: {err}");
        }
    }
}

fn open_zenoh(endpoint: &str) -> Result<zenoh::Session> {
    zenoh_connect::open_client(endpoint)
}

fn allocate_for_shared(
    shared: &SharedState,
    allocation_token: &str,
) -> Result<Option<zenoh_connect::AllocatedIdentity>> {
    match shared.relay_allocate_snap() {
        Some(url) if !url.is_empty() => {
            zenoh_connect::allocate(url.as_ref(), allocation_token).map(Some)
        }
        _ => Ok(None),
    }
}

fn assets_http_agent() -> ureq::Agent {
    // Follow 302 → R2 presigned GET (x001d). ureq strips Authorization on cross-host redirect.
    // `timeout_read` is per-read / idle stall (not a wall-clock cap on chunked transfer).
    // Hung connect or stalled body is cancelled; cooperative mid-stream cancel is non-goal.
    ureq::AgentBuilder::new()
        .timeout_connect(ASSETS_HTTP_CONNECT_TIMEOUT)
        .timeout_read(ASSETS_HTTP_READ_TIMEOUT)
        .redirects(5)
        .build()
}

/// Short Home-facing line (top-level `Display` only — full chain stays in logs).
fn asset_status_line(kind: &str, err: &anyhow::Error) -> String {
    format!("{kind} — placeholder ({err})")
}

fn try_coin_fixture_mesh() -> Result<CpuMesh> {
    let fixture = load_fixture_bytes()?;
    verify_blake3(&fixture, COIN_BLAKE3_HEX).context("fixture blake3")?;
    load_mesh_from_glb(&fixture).context("decode fixture GLB")
}

fn placeholder_from_err(kind: &str, err: &anyhow::Error) -> AuthMeshLoad {
    let status = asset_status_line(kind, err);
    log::error!("{status}; detail={err:#}");
    AuthMeshLoad::Placeholder {
        mesh: placeholder_error_mesh(),
        status,
    }
}

fn fetch_blob_bytes(assets_url: &str, blake3: &str, auth_token: &str) -> Result<Vec<u8>> {
    let base = assets_url.trim_end_matches('/');
    let url = format!("{base}/api/v1/blobs/{blake3}");
    let authz = format!("Bearer {auth_token}");
    let resp = assets_http_agent()
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

/// Debug-only Coin fixture escape hatch (x003c / x0048). Release never returns true.
fn fixture_fallback_allowed() -> bool {
    cfg!(debug_assertions) && std::env::var_os("ALCHEMY_ALLOW_FIXTURE_FALLBACK").is_some()
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
            // x003c / x0048: authenticated path requires assets. Fixture is debug-only escape hatch.
            if fixture_fallback_allowed() {
                log::warn!(
                    "assets download failed ({err:#}); ALCHEMY_ALLOW_FIXTURE_FALLBACK → fixture"
                );
                return load_fixture_bytes();
            }
            Err(err).context(
                "assets blob download required (upload demo coin / ensure CAS; debug: ALCHEMY_ALLOW_FIXTURE_FALLBACK=1)",
            )
        }
    }
}

/// Authenticated mesh resolution: assets GET success, debug fixture, or error placeholder.
enum AuthMeshLoad {
    Ready(CpuMesh),
    /// Distinct cube + dash status — never Coin fixture (x0048).
    Placeholder {
        mesh: CpuMesh,
        status: String,
    },
}

fn debug_fixture_or_placeholder(kind: &str, err: &anyhow::Error) -> AuthMeshLoad {
    if fixture_fallback_allowed() {
        log::warn!("{kind} ({err:#}); ALCHEMY_ALLOW_FIXTURE_FALLBACK → fixture");
        match try_coin_fixture_mesh() {
            Ok(mesh) => AuthMeshLoad::Ready(mesh),
            Err(fix_err) => {
                placeholder_from_err(&format!("{kind}; fixture fallback failed"), &fix_err)
            }
        }
    } else {
        placeholder_from_err(kind, err)
    }
}

fn load_auth_mesh(assets_url: &str, blake3: &str, auth_token: &str) -> AuthMeshLoad {
    let primary = match load_blob_bytes(assets_url, blake3, auth_token) {
        Ok(bytes) => bytes,
        Err(err) => {
            // load_blob_bytes already applied debug fixture escape on download failure.
            return placeholder_from_err("Asset download failed", &err);
        }
    };
    match verify_blake3(&primary, blake3) {
        Ok(()) => match load_mesh_from_glb(&primary) {
            Ok(mesh) => AuthMeshLoad::Ready(mesh),
            Err(err) => debug_fixture_or_placeholder("Asset GLB decode failed", &err),
        },
        Err(err) => debug_fixture_or_placeholder("Asset blake3 mismatch", &err),
    }
}

/// Unauthenticated Home: Coin fixture is the intentional mesh (not Manifest blake3).
/// Missing/corrupt fixture → placeholder (debug env cannot invent bytes).
fn load_unauth_home_mesh() -> AuthMeshLoad {
    match try_coin_fixture_mesh() {
        Ok(mesh) => AuthMeshLoad::Ready(mesh),
        Err(err) => placeholder_from_err("Home fixture unavailable", &err),
    }
}

fn load_fixture_bytes() -> Result<Vec<u8>> {
    let path = fixture_path()?;
    std::fs::read(&path).with_context(|| format!("read fixture {}", path.display()))
}

fn fixture_path() -> Result<PathBuf> {
    // Used for: (1) ensure-demo-coin seed source beside ZIP, (2) unauthenticated Home UI
    // placeholder before login, (3) debug `ALCHEMY_ALLOW_FIXTURE_FALLBACK`.
    // Not the authenticated release mesh path (that is Manifest → assets GET).
    const REL: &str = "fixtures/Coin_PolygonalMind.glb";
    if let Ok(p) = std::env::var("ALCHEMY_FIXTURE_GLB") {
        return Ok(PathBuf::from(p));
    }
    if let Ok(mut exe) = std::env::current_exe() {
        exe.pop();
        let beside_exe = exe.join(REL);
        if beside_exe.is_file() {
            return Ok(beside_exe);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let beside_cwd = cwd.join(REL);
        if beside_cwd.is_file() {
            return Ok(beside_cwd);
        }
    }
    // Dev tree: client/app → ../fixtures
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
    // Ok(hex) = mesh blake3; Err(reason) = sealed Manifest applied but no mesh blake3.
    blake3_tx: mpsc::SyncSender<Result<String, String>>,
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
            // x0038: drop counters (host input-gate style); never leave on verify fail.
            let mut dropped_identity: u64 = 0;
            let mut dropped_seal: u64 = 0;
            let mut dropped_attest: u64 = 0;
            let mut dropped_gen_rev: u64 = 0;
            let mut dropped_decode: u64 = 0;
            let mut dropped_limits: u64 = 0;
            let mut last_gate_log = Instant::now();
            while let Ok(sample) = subscriber.recv() {
                // x003b: early drops must still reach the interval aggregate log below.
                'sample: {
                let bytes: ZBytes = sample.payload().clone();
                // x003b: length before contiguous copy (`to_bytes`).
                if let Err(err) = check_payload_bytes(WireClass::Manifest, bytes.len()) {
                    dropped_limits += 1;
                    log::warn!("drop SceneManifest: {err}");
                    break 'sample;
                }
                let payload = bytes.to_bytes();
                match SceneManifest::decode(payload.as_ref()) {
                    Ok(manifest) => {
                        if let Err(err) = check_manifest(&manifest) {
                            dropped_limits += 1;
                            log::warn!("drop SceneManifest: {err}");
                            break 'sample;
                        }
                        if let Err(reason) = gate_manifest_identity(
                            &manifest.protocol_pin,
                            &manifest.room_id,
                            manifest.host_pub.len(),
                            shared.room_id.as_ref(),
                        ) {
                            dropped_identity += 1;
                            log::warn!(
                                "drop SceneManifest: {reason} (pin={:?} room={:?} host_pub_len={} want_pin={PROTOCOL_PIN} want_room={})",
                                manifest.protocol_pin,
                                manifest.room_id,
                                manifest.host_pub.len(),
                                shared.room_id
                            );
                            break 'sample;
                        }
                        let body_hex = manifest_seal_blake3_hex(&manifest);
                        if let Err(err) = seal.verify(
                            &manifest.world_seal,
                            manifest.revision,
                            manifest.host_generation,
                            &body_hex,
                        ) {
                            dropped_seal += 1;
                            log::warn!("drop SceneManifest: world_seal rejected: {err:#}");
                            break 'sample;
                        }
                        let (attested_pub, attested_gen) = match attest.verify_and_host_pub(
                            &manifest.host_attestation,
                            &manifest.host_pub,
                            manifest.host_generation,
                        ) {
                            Ok(pair) => pair,
                            Err(err) => {
                                dropped_attest += 1;
                                log::warn!(
                                    "drop SceneManifest: host_attestation rejected: {err:#}"
                                );
                                break 'sample;
                            }
                        };
                        let incoming = GenRev {
                            host_generation: attested_gen,
                            revision: manifest.revision,
                        };
                        // Hold both locks while gating + applying so poison / same-gen
                        // host_pub mismatch cannot leave host_pub updated alone (x003a).
                        let apply_outcome = match (
                            shared.host_pub.lock(),
                            shared.manifest_gen_rev.lock(),
                        ) {
                            (Ok(mut host_slot), Ok(mut gen_slot)) => {
                                let known = *gen_slot;
                                let decision = decide_gen_rev(known, incoming);
                                match decision {
                                    GenRevDecision::RejectRegression
                                    | GenRevDecision::RejectInvalidGeneration => {
                                        Err((
                                            "gen/rev gate",
                                            format!(
                                                "{decision:?} (incoming gen={} rev={} known={known:?})",
                                                incoming.host_generation, incoming.revision
                                            ),
                                        ))
                                    }
                                    GenRevDecision::AcceptSameOrForward => {
                                        // Same generation ⇒ host_pub is frozen (no swap, no
                                        // reinstall if leave cleared the key mid-flight).
                                        match host_slot.as_ref() {
                                            Some(prev) if prev == &attested_pub => {
                                                *gen_slot = Some(incoming);
                                                Ok(false)
                                            }
                                            Some(_) => Err((
                                                "same-generation host_pub mismatch",
                                                format!(
                                                    "gen={} (Frame key must not swap without generation bump)",
                                                    incoming.host_generation
                                                ),
                                            )),
                                            None => Err((
                                                "same-generation host_pub missing",
                                                format!(
                                                    "gen={} (watermark set but key cleared; reject reinstall)",
                                                    incoming.host_generation
                                                ),
                                            )),
                                        }
                                    }
                                    GenRevDecision::AcceptFirst => {
                                        *host_slot = Some(attested_pub.clone());
                                        *gen_slot = Some(incoming);
                                        Ok(false)
                                    }
                                    GenRevDecision::AcceptNewGeneration => {
                                        *host_slot = Some(attested_pub.clone());
                                        *gen_slot = Some(incoming);
                                        Ok(true)
                                    }
                                }
                            }
                            _ => {
                                log::error!(
                                    "host_pub/manifest_gen_rev lock poisoned; skip Manifest apply"
                                );
                                break 'sample;
                            }
                        };
                        let generation_changed = match apply_outcome {
                            Ok(changed) => changed,
                            Err((reason, detail)) => {
                                dropped_gen_rev += 1;
                                log::warn!("drop SceneManifest: {reason}: {detail}");
                                break 'sample;
                            }
                        };
                        if generation_changed {
                            log::info!(
                                "SceneManifest host_generation advanced to {}; reset world + renew Intent session",
                                incoming.host_generation
                            );
                            shared.reset_for_host_generation();
                            snapshot_ok = false;
                        }
                        // Classification: ManifestIndex (shared-sim / x0030; priority = x0027).
                        // x003c: send mesh blake3 BEFORE snapshot GET — boot wait must not
                        // depend on snapshot latency (auth path bails on Manifest timeout).
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
                        if !sent_blake3 {
                            let msg = match mesh_blake3 {
                                Some(hex) => Ok(hex),
                                None => Err(
                                    "SceneManifest has no mesh entity blake3".to_string(),
                                ),
                            };
                            let _ = blake3_tx.send(msg);
                            sent_blake3 = true;
                        } else if mesh_blake3.is_some() {
                            // Boot download already consumed the channel; live mesh swap is out of scope.
                            log::info!(
                                "Manifest mesh blake3 updated after boot download; remount not applied"
                            );
                        }
                        if !snapshot_ok {
                            if let Some(cap) = shared.wire_cap() {
                                snapshot_ok = fetch_snapshot(
                                    &session,
                                    shared.room_id.as_ref(),
                                    &cap,
                                    &shared,
                                );
                            }
                        }
                        log::info!(
                            "SceneManifest generation={} revision={} entries={} pin_ok attested snapshot_ok={snapshot_ok}",
                            manifest.host_generation,
                            manifest.revision,
                            manifest.entries.len()
                        );
                    }
                    Err(err) => {
                        dropped_decode += 1;
                        log::warn!("drop SceneManifest: decode failed: {err}");
                    }
                }
                } // 'sample
                if last_gate_log.elapsed() >= GATE_LOG_INTERVAL {
                    if dropped_identity
                        + dropped_seal
                        + dropped_attest
                        + dropped_gen_rev
                        + dropped_decode
                        + dropped_limits
                        > 0
                    {
                        log::info!(
                            "manifest gate: dropped_identity={dropped_identity} dropped_seal={dropped_seal} dropped_attest={dropped_attest} dropped_gen_rev={dropped_gen_rev} dropped_decode={dropped_decode} dropped_limits={dropped_limits} (last ~5s)"
                        );
                    }
                    dropped_identity = 0;
                    dropped_seal = 0;
                    dropped_attest = 0;
                    dropped_gen_rev = 0;
                    dropped_decode = 0;
                    dropped_limits = 0;
                    last_gate_log = Instant::now();
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
        ApplyFrameOutcome::Applied => {
            shared.metrics.note_frame_accepted();
        }
        _ => {}
    }
    outcome
}

/// Returns true when a non-empty snapshot was verified and applied (replace).
fn fetch_snapshot(
    session: &zenoh::Session,
    room_id: &str,
    wire_cap: &str,
    shared: &SharedState,
) -> bool {
    let key = snapshot_key(room_id, wire_cap);
    let mut applied = false;
    let mut dropped_limits: u64 = 0;
    match session.get(&key).wait() {
        Ok(receiver) => {
            while let Ok(reply) = receiver.recv() {
                match reply.result() {
                    Ok(sample) => {
                        let bytes: ZBytes = sample.payload().clone();
                        if let Err(err) = check_payload_bytes(WireClass::Frame, bytes.len()) {
                            dropped_limits += 1;
                            log::warn!("drop snapshot: {err}");
                            continue;
                        }
                        let payload = bytes.to_bytes();
                        match RenderFrame::decode(payload.as_ref()) {
                            Ok(frame) => {
                                if let Err(err) = check_frame(&frame) {
                                    dropped_limits += 1;
                                    log::warn!("drop snapshot: {err}");
                                    continue;
                                }
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
    if dropped_limits > 0 {
        log::info!("snapshot gate: dropped_limits={dropped_limits}");
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
            let mut dropped_limits: u64 = 0;
            let mut last_gate_log = Instant::now();
            while let Ok(sample) = subscriber.recv() {
                'sample: {
                    let bytes: ZBytes = sample.payload().clone();
                    if let Err(err) = check_payload_bytes(WireClass::Frame, bytes.len()) {
                        dropped_limits += 1;
                        log::warn!("drop RenderFrame: {err}");
                        break 'sample;
                    }
                    let payload = bytes.to_bytes();
                    match RenderFrame::decode(payload.as_ref()) {
                        Ok(frame) => {
                            if let Err(err) = check_frame(&frame) {
                                dropped_limits += 1;
                                log::warn!("drop RenderFrame: {err}");
                                break 'sample;
                            }
                            match apply_frame(&shared, frame, /* replace */ false) {
                                ApplyFrameOutcome::Applied | ApplyFrameOutcome::Dropped => {}
                                ApplyFrameOutcome::NeedResync => {
                                    // x0024: missing deltas are not retransmitted — resync via snapshot.
                                    let now = Instant::now();
                                    if now < resync_cooldown_until {
                                        break 'sample;
                                    }
                                    resync_cooldown_until = now + SNAPSHOT_GAP_RESYNC_COOLDOWN;
                                    if let Some(cap) = shared.wire_cap() {
                                        let _ = fetch_snapshot(
                                            &session,
                                            shared.room_id.as_ref(),
                                            &cap,
                                            &shared,
                                        );
                                    }
                                }
                            }
                        }
                        Err(err) => log::warn!("RenderFrame decode failed: {err}"),
                    }
                } // 'sample
                if last_gate_log.elapsed() >= GATE_LOG_INTERVAL {
                    if dropped_limits > 0 {
                        log::info!("frame gate: dropped_limits={dropped_limits} (last ~5s)");
                    }
                    dropped_limits = 0;
                    last_gate_log = Instant::now();
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

    // Drop owned clone here — Subscriber holds WeakSession; Leave uses close() to end recv.
    // Do not move `Session` into the thread (strong retain blocks Drop-auto-close).
    drop(session);

    thread::Builder::new()
        .name("zenoh-presence".into())
        .spawn(move || {
            log::info!("subscribed to {key} (Presence; local_sub from AuthGate)");
            let mut dropped_pin: u64 = 0;
            let mut dropped_no_host: u64 = 0;
            let mut dropped_sig: u64 = 0;
            let mut dropped_gen_rev: u64 = 0;
            let mut dropped_decode: u64 = 0;
            let mut dropped_limits: u64 = 0;
            let mut last_gate_log = Instant::now();
            while let Ok(sample) = subscriber.recv() {
                'sample: {
                let bytes: ZBytes = sample.payload().clone();
                if let Err(err) = check_payload_bytes(WireClass::Presence, bytes.len()) {
                    dropped_limits += 1;
                    log::warn!("drop PresenceState: {err}");
                    break 'sample;
                }
                let payload = bytes.to_bytes();
                match PresenceState::decode(payload.as_ref()) {
                    Ok(state) => {
                        if let Err(err) = check_presence(&state) {
                            dropped_limits += 1;
                            log::warn!("drop PresenceState: {err}");
                            break 'sample;
                        }
                        if let Err(reason) = gate_presence_pin(&state.protocol_pin) {
                            dropped_pin += 1;
                            log::warn!(
                                "drop PresenceState: {reason} (pin={:?} want={PROTOCOL_PIN})",
                                state.protocol_pin
                            );
                            break 'sample;
                        }
                        let host_pub = shared.host_pub.lock().ok().and_then(|g| g.clone());
                        let Some(host_pub) = host_pub else {
                            dropped_no_host += 1;
                            log::warn!("drop PresenceState: host_pub not yet attested");
                            break 'sample;
                        };
                        if let Err(err) = verify_presence(&host_pub, &state) {
                            dropped_sig += 1;
                            log::warn!("drop PresenceState: bad host_sig: {err}");
                            break 'sample;
                        }
                        let manifest_gen = shared
                            .manifest_gen_rev
                            .lock()
                            .ok()
                            .and_then(|g| g.map(|gr| gr.host_generation));
                        let known_presence = shared.presence_gen_rev.lock().ok().and_then(|g| *g);
                        let incoming = GenRev {
                            host_generation: state.host_generation,
                            revision: state.revision,
                        };
                        match decide_presence_gen_rev(manifest_gen, known_presence, incoming) {
                            GenRevDecision::AcceptFirst
                            | GenRevDecision::AcceptSameOrForward => {
                                if let Ok(mut slot) = shared.presence_gen_rev.lock() {
                                    *slot = Some(incoming);
                                }
                            }
                            other => {
                                dropped_gen_rev += 1;
                                log::warn!(
                                    "drop PresenceState: gen/rev gate {:?} (incoming gen={} rev={} manifest_gen={:?} known={:?})",
                                    other,
                                    incoming.host_generation,
                                    incoming.revision,
                                    manifest_gen,
                                    known_presence
                                );
                                break 'sample;
                            }
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
                            break 'sample;
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
                    Err(err) => {
                        dropped_decode += 1;
                        log::warn!("drop PresenceState: decode failed: {err}");
                    }
                }
                } // 'sample
                if last_gate_log.elapsed() >= GATE_LOG_INTERVAL {
                    if dropped_pin
                        + dropped_no_host
                        + dropped_sig
                        + dropped_gen_rev
                        + dropped_decode
                        + dropped_limits
                        > 0
                    {
                        log::info!(
                            "presence gate: dropped_pin={dropped_pin} dropped_no_host={dropped_no_host} dropped_sig={dropped_sig} dropped_gen_rev={dropped_gen_rev} dropped_decode={dropped_decode} dropped_limits={dropped_limits} (last ~5s)"
                        );
                    }
                    dropped_pin = 0;
                    dropped_no_host = 0;
                    dropped_sig = 0;
                    dropped_gen_rev = 0;
                    dropped_limits = 0;
                    dropped_decode = 0;
                    last_gate_log = Instant::now();
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
/// x003e: mint Intent session first so Token `cnf.spk` matches bind `session_pub`.
pub fn join_room_with_access(shared: &SharedState, access_token: &str) -> Result<()> {
    let session = intent_session::IntentSession::new();
    let session_pub_b64 = session.session_pub_b64();
    let guest = world_token::fetch_guest_token(
        &shared.world_url_snap(),
        &shared.room_id,
        access_token,
        &session_pub_b64,
    )
    .context("fetch guest Room Token")?;
    let local_sub = jwt_sub::sub_unverified(&guest.token)
        .context("read Room Token sub (unverified payload)")?;
    if let Ok(mut cap) = shared.wire_cap.lock() {
        *cap = Some(Arc::<str>::from(guest.wire_cap.as_str()));
    }
    // mTLS material for the *next* `open_zenoh` / one-shot `bind_room_after_join` reopen (x004e).
    if !guest.allocation_token.is_empty() {
        if let Some(id) = allocate_for_shared(shared, &guest.allocation_token)
            .context("relay allocate (guest)")?
        {
            std::env::set_var("ZENOH_TLS_CONNECT_CERT", id.cert_path.display().to_string());
            std::env::set_var("ZENOH_TLS_CONNECT_KEY", id.key_path.display().to_string());
            shared.mtls_reopen_pending.store(true, Ordering::SeqCst);
            log::info!(
                "relay allocate ok cn={} role={} (mtls_reopen_pending)",
                id.cn,
                id.role
            );
        }
    }
    // Lock order: auth → intent_session → pending (same as generation commit).
    if let Ok(mut gate) = shared.auth.lock() {
        gate.apply_room(guest.token, local_sub.clone(), guest.expires_in);
        if let Ok(mut slot) = shared.intent_session.lock() {
            *slot = Some(session);
            if let Ok(mut pending) = shared.pending_intent_session.lock() {
                *pending = None;
            }
        }
        if let Ok(mut retry) = shared.pending_retry.lock() {
            *retry = None;
        }
    }
    log::info!(
        "joined room={} sub={local_sub} expires_in={} wire_cap_len={}",
        shared.room_id,
        guest.expires_in,
        guest.wire_cap.len()
    );
    Ok(())
}

/// Start Zenoh room subscribers once `wire_cap` is known (silent boot or Home→Join).
///
/// Idempotent via `room_bus_started`. On spawn failure the flag is cleared so retry works.
/// Pass `Some(blake3_tx)` for boot mesh wait; `None` drains blake3 on a background thread
/// (late Home→Join mesh remount is non-goal).
pub fn ensure_room_bus(
    shared: &Arc<SharedState>,
    session: &zenoh::Session,
    blake3_tx: Option<mpsc::SyncSender<Result<String, String>>>,
) -> Result<bool> {
    if shared
        .room_bus_started
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Ok(false);
    }
    let started = (|| -> Result<()> {
        let wire_cap = shared
            .wire_cap()
            .ok_or_else(|| anyhow::anyhow!("ensure_room_bus: wire_cap missing"))?;
        let room_id = shared.room_id.as_ref();
        let manifest_key_expr = manifest_key(room_id, &wire_cap);
        let frame_key_expr = frame_key(room_id, &wire_cap);
        let input_key_expr = input_key(room_id, &wire_cap);
        let presence_key_expr = presence_key(room_id, &wire_cap);
        let blake3_tx = match blake3_tx {
            Some(tx) => tx,
            None => {
                let (tx, rx) = mpsc::sync_channel::<Result<String, String>>(1);
                thread::Builder::new()
                    .name("blake3-drain".into())
                    .spawn(move || while rx.recv().is_ok() {})
                    .ok();
                tx
            }
        };
        spawn_manifest_subscriber(
            session.clone(),
            manifest_key_expr,
            Arc::clone(shared),
            blake3_tx,
            Arc::clone(&shared.manifest_seal),
            Arc::clone(&shared.host_attest),
        )?;
        spawn_presence_subscriber(session.clone(), presence_key_expr, Arc::clone(shared))?;
        spawn_frame_subscriber(session.clone(), frame_key_expr, Arc::clone(shared))?;
        spawn_input_publisher(session.clone(), input_key_expr, Arc::clone(shared))?;
        log::info!("room Zenoh bus started (wire_cap_len={})", wire_cap.len());
        Ok(())
    })();
    if let Err(err) = started {
        shared.room_bus_started.store(false, Ordering::SeqCst);
        return Err(err);
    }
    Ok(true)
}

/// After Home→Join / StartSession: apply mTLS via reopen if needed, then start room bus + liveliness.
///
/// Reopen when `mtls_reopen_pending` (fresh allocate) **or** session was dropped on Leave.
/// Persisted `ZENOH_TLS_*` env alone must not reopen — that orphans an already-attached bus.
pub fn bind_room_after_join(
    shared: &Arc<SharedState>,
    zenoh: &mut Option<zenoh::Session>,
) -> Result<()> {
    let mtls_pending = shared.mtls_reopen_pending.swap(false, Ordering::SeqCst);
    if mtls_pending || zenoh.is_none() {
        // Match Leave / orphan path: always close before replace (not only Drop-on-overwrite).
        close_zenoh(zenoh);
        match open_zenoh(shared.zenoh_connect_snap().as_ref())
            .context("zenoh open/reopen after join")
        {
            Ok(session) => {
                *zenoh = Some(session);
                shared.room_bus_started.store(false, Ordering::SeqCst);
                log::info!(
                    "zenoh session {} after join (mtls_pending={mtls_pending})",
                    if mtls_pending { "reopened" } else { "opened" }
                );
            }
            Err(err) => {
                if mtls_pending {
                    shared.mtls_reopen_pending.store(true, Ordering::SeqCst);
                }
                return Err(err);
            }
        }
    }
    let bus = {
        let session = zenoh
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("bind_room_after_join: no zenoh session"))?;
        ensure_room_bus(shared, session, None)
    };
    if let Err(err) = bus {
        // Partial spawn_* may leave threads on this session; close() ends Weak recv/put.
        log::warn!("room bus spawn failed ({err:#}); closing session to clear orphans");
        close_zenoh(zenoh);
        shared.room_bus_started.store(false, Ordering::SeqCst);
        *zenoh = Some(
            open_zenoh(shared.zenoh_connect_snap().as_ref())
                .context("zenoh reopen after bus spawn failure")?,
        );
        let retry = {
            let session = zenoh
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("bind_room_after_join: no zenoh after reopen"))?;
            ensure_room_bus(shared, session, None)
        };
        if let Err(retry_err) = retry {
            close_zenoh(zenoh);
            return Err(retry_err).context("room bus retry after reopen");
        }
    }
    let session = zenoh
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("bind_room_after_join: no zenoh session"))?;
    declare_room_alive(shared, session, false).context("liveliness after join")?;
    Ok(())
}

/// Sleep until the next access/room deadline, capped at [`TOKEN_REFRESH_POLL`].
/// Joined+pending wakes at `pending_retry` (backoff) or soon if unset.
fn token_refresh_wait(shared: &SharedState) -> Duration {
    let now = Instant::now();
    if shared.has_pending_intent_session_joined() {
        return match shared.pending_retry_not_before() {
            Some(t) if t > now => t
                .saturating_duration_since(now)
                .min(TOKEN_REFRESH_POLL)
                .max(Duration::from_millis(50)),
            _ => Duration::from_millis(50),
        };
    }
    let Ok(gate) = shared.auth.lock() else {
        return TOKEN_REFRESH_POLL;
    };
    let mut wait = TOKEN_REFRESH_POLL;
    for deadline in [gate.access_deadline, gate.room_deadline]
        .into_iter()
        .flatten()
    {
        if deadline <= now {
            return Duration::from_millis(50);
        }
        wait = wait.min(deadline.saturating_duration_since(now));
    }
    wait.max(Duration::from_millis(50))
}

/// Background access + Room Token refresh at ~80% TTL (x0021 P1-1).
fn spawn_token_refresh(shared: Arc<SharedState>) -> Result<()> {
    thread::Builder::new()
        .name("token-refresh".into())
        .spawn(move || {
            log::info!(
                "token refresh loop (poll cap {}s; due deadlines wake sooner)",
                TOKEN_REFRESH_POLL.as_secs()
            );
            loop {
                thread::sleep(token_refresh_wait(&shared));
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
                        Some(rt) => match auth::refresh(&shared.auth_url_snap(), &rt) {
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

                shared.clear_pending_if_not_joined();

                let joined = shared
                    .auth
                    .lock()
                    .ok()
                    .map(|g| g.is_joined())
                    .unwrap_or(false);
                let pending_rotate = joined && shared.has_pending_intent_session_joined();
                let pending_due = pending_rotate
                    && shared
                        .pending_retry_not_before()
                        .map(|t| now >= t)
                        .unwrap_or(true);
                let room_due = joined
                    && shared
                        .auth
                        .lock()
                        .ok()
                        .map(|g| g.room_deadline.map(|d| now >= d).unwrap_or(false))
                        .unwrap_or(false);
                // pending backoff must not be bypassed by TTL; during backoff only live refresh.
                let need_room = pending_due || room_due;
                if need_room {
                    let access = shared.auth.lock().ok().and_then(|g| {
                        if g.is_joined() {
                            g.access_token.clone()
                        } else {
                            None
                        }
                    });
                    let Some(access) = access else {
                        if pending_due {
                            shared.note_pending_token_failure();
                        } else if let Ok(mut gate) = shared.auth.lock() {
                            gate.room_deadline = Some(now + TOKEN_REFRESH_POLL);
                        }
                        continue;
                    };
                    // Lock order for pub reads: auth → intent → pending (helpers).
                    // pending_due → pending key; else (TTL during backoff) → live key only.
                    let (session_pub_b64, for_pending) = if pending_due {
                        if let Some(b64) = shared.pending_session_pub_b64_if_joined() {
                            (b64, true)
                        } else if let Some(b64) = shared.live_session_pub_b64_if_joined() {
                            (b64, false)
                        } else {
                            if let Ok(mut gate) = shared.auth.lock() {
                                gate.room_deadline = Some(now + TOKEN_REFRESH_POLL);
                            }
                            continue;
                        }
                    } else if let Some(b64) = shared.live_session_pub_b64_if_joined() {
                        (b64, false)
                    } else {
                        if let Ok(mut gate) = shared.auth.lock() {
                            gate.room_deadline = Some(now + TOKEN_REFRESH_POLL);
                        }
                        continue;
                    };
                    // Guest refresh updates Room Token only — no re-allocate / session reopen.
                    // mTLS NotAfter ≈ 1d (openssl); mid-flight cert refresh is host-side / non-goal.
                    match world_token::fetch_guest_token(
                        &shared.world_url_snap(),
                        &shared.room_id,
                        &access,
                        &session_pub_b64,
                    ) {
                        Ok(guest) => {
                            let local_sub = match jwt_sub::sub_unverified(&guest.token) {
                                Ok(s) => s,
                                Err(err) => {
                                    log::warn!("Room Token refresh: bad sub: {err:#}");
                                    if for_pending {
                                        shared.note_pending_token_failure();
                                    } else if let Ok(mut gate) = shared.auth.lock() {
                                        gate.room_deadline = Some(now + TOKEN_REFRESH_POLL);
                                    }
                                    continue;
                                }
                            };
                            let applied = if for_pending {
                                shared.commit_pending_session_with_token(
                                    &session_pub_b64,
                                    guest,
                                    local_sub,
                                )
                            } else {
                                shared.apply_room_token_if_session_matches(
                                    &session_pub_b64,
                                    guest,
                                    local_sub,
                                )
                            };
                            if applied {
                                log::info!(
                                    "Room Token {} (pending_rotate={for_pending})",
                                    if for_pending {
                                        "committed for host generation"
                                    } else {
                                        "refreshed; session rebind + liveliness pending"
                                    }
                                );
                            } else {
                                log::info!(
                                    "discard Room Token fetch (session_pub no longer expected; pending={for_pending})"
                                );
                                shared.clear_pending_if_not_joined();
                                if for_pending && shared.has_pending_intent_session_joined() {
                                    shared.note_pending_token_failure();
                                } else if !for_pending {
                                    shared.force_room_token_refresh_soon();
                                }
                            }
                        }
                        Err(err) => {
                            log::warn!("Room Token refresh failed: {err:#}");
                            if for_pending {
                                shared.note_pending_token_failure();
                            } else if let Ok(mut gate) = shared.auth.lock() {
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
    let wire_cap = shared
        .wire_cap()
        .ok_or_else(|| anyhow::anyhow!("liveliness requires wire_cap (join first; x004e)"))?;
    let key = liveliness_key(shared.room_id.as_ref(), &wire_cap, &sub, &jti);
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

/// Explicit Leave put + liveliness undeclare + **`session.close()`** + local clear (x0019 / x0023).
///
/// Zenoh 1.x: `Drop` closes only when `strong_counter == 1`. Setting `*zenoh = None` alone does
/// not stop bus threads that retained a strong clone. `close().wait()` closes the peer; Subscriber
/// / Publisher use `WeakSession`, so `recv` ends and input put stops (Leave→Join must not double Intent).
pub fn leave_room(shared: &SharedState, zenoh: &mut Option<zenoh::Session>) {
    if let Some(session) = zenoh.as_ref() {
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
            if let Some(wire_cap) = shared.wire_cap() {
                let key = input_key(shared.room_id.as_ref(), &wire_cap);
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
            } else {
                log::warn!("Leave InputEvent skipped: no wire_cap");
            }
        }
    }
    // Drop liveliness before session close so host sees Delete promptly.
    if let Ok(mut token) = shared.liveliness.lock() {
        *token = None;
    }
    if let Ok(mut prev) = shared.liveliness_prev.lock() {
        *prev = None;
    }
    close_zenoh(zenoh);
    shared.leave_room_local();
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // File fills gaps; process env (CI / Start.cmd) still wins (x0066).
    if let Err(err) = alchemy_connect_config::apply_file_defaults_to_env() {
        log::warn!("connect.json defaults: {err:#}");
    }
    let endpoints = alchemy_connect_config::resolve().context("resolve Connect endpoints")?;
    alchemy_connect_config::apply_to_env(&endpoints);
    let auth_url = endpoints.auth_url.clone();
    let world_url = endpoints.world_url.clone();
    let assets_url = endpoints.assets_url.clone();

    let room_id_raw = std::env::var("ROOM_ID").unwrap_or_else(|_| DEFAULT_ROOM_ID.to_string());
    let room_id = validate_room_id(&room_id_raw)
        .map_err(|e| anyhow::anyhow!("invalid ROOM_ID={room_id_raw:?}: {e}"))?
        .to_string();
    let fallback_blake3 =
        std::env::var("ALCHEMY_BLOB_BLAKE3").unwrap_or_else(|_| COIN_BLAKE3_HEX.to_string());

    let silent = auth::try_silent_session(&auth_url).context("silent auth")?;
    let auth_mode = if silent.is_some() { "silent" } else { "ui" };

    let mut gate = AuthGate::default();
    if let Some(session) = silent {
        gate.apply_auth_session(session);
    }

    let client_metrics = Arc::new(metrics::ClientMetrics::default());
    metrics::maybe_spawn_from_env(Arc::clone(&client_metrics));

    let jwks = Arc::new(WorldJwks::new(&world_url));
    jwks.set_metrics(Arc::clone(&client_metrics));
    jwks.refresh()
        .context("initial world JWKS (ManifestSeal + HostAttestation)")?;
    jwks.spawn_background_refresh();
    let seal = Arc::new(ManifestSealVerifier::new(Arc::clone(&jwks), &room_id));
    let attest = Arc::new(HostAttestationVerifier::new(Arc::clone(&jwks), &room_id));

    let shared = Arc::new(SharedState {
        local_user_root: AtomicU32::new(LOCAL_USER_ROOT_UNSET),
        room_id: Arc::<str>::from(room_id.clone()),
        wire_cap: Mutex::new(None),
        room_bus_started: AtomicBool::new(false),
        mtls_reopen_pending: AtomicBool::new(false),
        manifest_seal: Arc::clone(&seal),
        host_attest: Arc::clone(&attest),
        world_jwks: Arc::clone(&jwks),
        auth_url: Mutex::new(Arc::<str>::from(auth_url)),
        world_url: Mutex::new(Arc::<str>::from(world_url)),
        assets_url: Mutex::new(Arc::<str>::from(assets_url.clone())),
        zenoh_connect: Mutex::new(Arc::<str>::from(endpoints.selected_zenoh())),
        relay_allocate_url: Mutex::new(endpoints.selected_allocate_url().map(Arc::<str>::from)),
        auth: Mutex::new(gate),
        intent_session: Mutex::new(None),
        pending_intent_session: Mutex::new(None),
        pending_retry: Mutex::new(None),
        host_pub: Mutex::new(None),
        manifest_gen_rev: Mutex::new(None),
        presence_gen_rev: Mutex::new(None),
        liveliness: Mutex::new(None),
        liveliness_prev: Mutex::new(None),
        world: Mutex::new(scene::WorldView::default()),
        metrics: Arc::clone(&client_metrics),
        box_defs: Mutex::new(Arc::new(scene::BoxDefs::default())),
        mesh_entity_id: AtomicU32::new(0),
        asset_status: Mutex::new(None),
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

    let wire_cap = shared.wire_cap();
    log::info!(
        "client x004e starting; protocol_pin={PROTOCOL_PIN} room={room_id} auth_mode={auth_mode} wire_cap={}",
        wire_cap
            .as_ref()
            .map(|c| format!("len={}", c.len()))
            .unwrap_or_else(|| "pending".into())
    );

    spawn_token_refresh(Arc::clone(&shared))?;

    // Allocate (if any) already set ZENOH_TLS_* during join — open picks them up.
    let session =
        Some(open_zenoh(shared.zenoh_connect_snap().as_ref()).context("open zenoh session")?);
    // Boot open consumed the cert; do not leave a stale one-shot for a later bind.
    shared.mtls_reopen_pending.store(false, Ordering::SeqCst);
    let joined_with_cap = wire_cap.is_some()
        && shared
            .auth
            .lock()
            .ok()
            .map(|g| g.is_joined())
            .unwrap_or(false);

    let (blake3_tx, blake3_rx) = mpsc::sync_channel::<Result<String, String>>(1);
    if joined_with_cap {
        let z = session.as_ref().expect("session just opened");
        ensure_room_bus(&shared, z, Some(blake3_tx))?;
        declare_room_alive(&shared, z, false).context("silent liveliness")?;
    } else {
        log::info!("room Zenoh bus deferred until Home→Join (x004e); local GUI join starts bus");
        drop(blake3_tx);
    }

    let access_for_blob = shared
        .auth
        .lock()
        .ok()
        .and_then(|g| g.access_token.clone())
        .unwrap_or_default();
    // x003c: silent/auth join must take SceneManifest blake3 → assets GET (no coin fallback).
    let blob_blake3 = if !joined_with_cap {
        log::info!("no room bus yet — Coin/Home fixture until join provides wire_cap");
        fallback_blake3
    } else {
        let manifest_wait = if access_for_blob.is_empty() {
            MANIFEST_WAIT
        } else {
            MANIFEST_WAIT_AUTH
        };
        match blake3_rx.recv_timeout(manifest_wait) {
            Ok(Ok(hex)) => {
                let mesh_id = shared.mesh_entity_id();
                log::info!("Manifest blake3 for mesh entity {mesh_id}: {hex}");
                hex
            }
            Ok(Err(reason)) if access_for_blob.is_empty() => {
                log::warn!("{reason}; unauthenticated — fallback blake3={fallback_blake3}");
                fallback_blake3
            }
            Ok(Err(reason)) => {
                anyhow::bail!("{reason} (auth join requires mesh blake3 for assets GET)");
            }
            Err(_) if access_for_blob.is_empty() => {
                log::warn!(
                    "no SceneManifest within {:?}; unauthenticated — fallback blake3={fallback_blake3}",
                    manifest_wait
                );
                fallback_blake3
            }
            Err(_) => {
                anyhow::bail!(
                    "no SceneManifest within {:?} after auth join (need Manifest blake3 for assets mesh; host presence-join republish should cover ZIP/launcher)",
                    manifest_wait
                );
            }
        }
    };

    let mesh = {
        let outcome = if access_for_blob.is_empty() {
            // Unauthenticated Home always intends Coin fixture; do not verify against
            // Manifest blake3 (may differ before login). x0048 review.
            log::info!("no auth yet — loading Coin fixture for Home (login via Home tab)");
            load_unauth_home_mesh()
        } else {
            load_auth_mesh(&assets_url, &blob_blake3, &access_for_blob)
        };
        match outcome {
            AuthMeshLoad::Ready(mesh) => mesh,
            AuthMeshLoad::Placeholder { mesh, status } => {
                shared.store_asset_status(status);
                mesh
            }
        }
    };
    log::info!(
        "mesh ready: {} verts, {} indices, radius={:.3}",
        mesh.positions.len(),
        mesh.indices.len(),
        mesh.radius
    );

    let kind = surface::choose();
    surface::run(kind, mesh, shared, session)?;
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

    /// x0038: identity / pin failures are discard reasons (Err), never a leave signal.
    #[test]
    fn manifest_identity_gate_rejects_bad_pin_room_and_host_pub_len() {
        assert!(gate_manifest_identity(PROTOCOL_PIN, "local", 32, "local").is_ok());
        assert_eq!(
            gate_manifest_identity("bogus-pin", "local", 32, "local"),
            Err("protocol_pin mismatch")
        );
        assert_eq!(
            gate_manifest_identity(PROTOCOL_PIN, "other", 32, "local"),
            Err("room_id mismatch")
        );
        assert_eq!(
            gate_manifest_identity(PROTOCOL_PIN, "local", 16, "local"),
            Err("host_pub length")
        );
    }

    #[test]
    fn presence_pin_gate_rejects_mismatch() {
        assert!(gate_presence_pin(PROTOCOL_PIN).is_ok());
        assert_eq!(gate_presence_pin("bogus-pin"), Err("protocol_pin mismatch"));
    }

    /// x003a: old signed Manifest replay must not roll back accepted watermark.
    #[test]
    fn manifest_gen_rev_gate_rejects_revision_regression() {
        let known = Some(GenRev {
            host_generation: 1,
            revision: 5,
        });
        let older = GenRev {
            host_generation: 1,
            revision: 3,
        };
        assert_eq!(
            decide_gen_rev(known, older),
            GenRevDecision::RejectRegression
        );
        let handoff = GenRev {
            host_generation: 2,
            revision: 0,
        };
        assert_eq!(
            decide_gen_rev(known, handoff),
            GenRevDecision::AcceptNewGeneration
        );
    }

    /// Same generation keeps AcceptSameOrForward; caller must still reject host_pub swap.
    #[test]
    fn same_generation_forward_does_not_imply_host_pub_handoff() {
        let known = Some(GenRev {
            host_generation: 1,
            revision: 1,
        });
        let forward = GenRev {
            host_generation: 1,
            revision: 2,
        };
        assert_eq!(
            decide_gen_rev(known, forward),
            GenRevDecision::AcceptSameOrForward
        );
    }
}
