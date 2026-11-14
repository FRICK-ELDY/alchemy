//! x001f / x0023 / x0038 / x003a / x0046 / x0069 hermetic E2E probe — no GUI.
//!
//! Guarantees: against a running auth/world/assets/zenohd (+ host-server), asserts
//! login → Room Token → signed InputEvent put → RenderFrame with upserts + user_root;
//! adversarial suite does not stop host Frame publish (x0038 / x003a / x0046 / x0069).
//!
//! Manifest dual-wall (x0069): probe **subscribes** `…/manifest` as a verifying guest and
//! asserts both gates — **Case A** AEAD discard (plaintext / wrong-room key / oversize
//! envelope) and **Case B** post-AEAD discard (correct `room_key`+AAD wrapping corrupt
//! protobuf or pin/empty-`world_seal` Manifest). Cryptographically valid old-seal JWT
//! replay / GUI leave remain unit-tested in alchemy-protocol / client.
//! Non-guarantees: Token refresh, WAN, R2, GUI leave path, Frame signature check.

use std::env;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use alchemy_protocol::decode_limits::check_manifest;
use alchemy_protocol::wire_aead::{self, AeadKind};
use alchemy_protocol::wire_crypto::{
    generate_signing_key, public_key_bytes, sign_input, SigningKey,
};
use alchemy_protocol::{
    frame_key, input_key, manifest_key, snapshot_key, user_root_entity_last, validate_room_id,
    InputEvent, Message, RenderFrame, SceneManifest, PROTOCOL_PIN, USER_ROOT_ENTITY_FIRST,
};
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use zenoh::bytes::ZBytes;
use zenoh::Wait;

/// Defaults match `docker-compose.e2e.yml` host ports (offset from day-to-day :4002 stack).
const DEFAULT_AUTH_URL: &str = "http://127.0.0.1:4102";
const DEFAULT_WORLD_URL: &str = "http://127.0.0.1:4103";
const DEFAULT_ASSETS_URL: &str = "http://127.0.0.1:4104";
const DEFAULT_ZENOH_CONNECT: &str = "tcp/127.0.0.1:7547";
const DEFAULT_ROOM_ID: &str = "local";
const DEFAULT_AUTH_USER: &str = "alice";
const DEFAULT_AUTH_PASSWORD: &str = "Password1";
const DEFAULT_TIMEOUT_SECS: u64 = 45;

#[derive(Debug, Deserialize)]
struct AuthSession {
    access_token: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    token: String,
    #[serde(default)]
    wire_cap: String,
    #[serde(default)]
    room_key: String,
}

fn env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn health_get(url: &str) -> Result<()> {
    let base = url.trim_end_matches('/');
    ureq::get(&format!("{base}/health"))
        .timeout(Duration::from_secs(5))
        .call()
        .with_context(|| format!("GET {base}/health"))?;
    Ok(())
}

fn login(auth_url: &str, user: &str, password: &str) -> Result<String> {
    let base = auth_url.trim_end_matches('/');
    let resp: AuthSession = ureq::post(&format!("{base}/api/v1/auth/login"))
        .set("content-type", "application/json")
        .timeout(Duration::from_secs(15))
        .send_json(serde_json::json!({
            "identifier": user,
            "password": password,
            "remember_me": false
        }))
        .context("auth login")?
        .into_json()
        .context("parse auth session")?;
    if resp.access_token.is_empty() {
        bail!("login returned empty access_token");
    }
    Ok(resp.access_token)
}

/// base64url (no pad) for **exactly** 32-byte Ed25519 pub.
/// Must match world `Base.url_encode64(..., padding: false)` / client `URL_SAFE_NO_PAD`.
/// Hand-rolled to avoid a new Hex/Cargo dep in e2e-probe; do not reuse for other lengths.
fn session_pub_b64(pub_bytes: &[u8]) -> Result<String> {
    if pub_bytes.len() != 32 {
        bail!("session_pub must be 32 bytes");
    }
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(43);
    let mut i = 0;
    while i + 3 <= pub_bytes.len() {
        let n = (u32::from(pub_bytes[i]) << 16)
            | (u32::from(pub_bytes[i + 1]) << 8)
            | u32::from(pub_bytes[i + 2]);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(T[((n >> 6) & 63) as usize] as char);
        out.push(T[(n & 63) as usize] as char);
        i += 3;
    }
    // 32 ≡ 2 (mod 3): one leftover pair → 3 chars, no pad.
    let n = (u32::from(pub_bytes[30]) << 16) | (u32::from(pub_bytes[31]) << 8);
    out.push(T[((n >> 18) & 63) as usize] as char);
    out.push(T[((n >> 12) & 63) as usize] as char);
    out.push(T[((n >> 6) & 63) as usize] as char);
    Ok(out)
}

struct RoomTokenBundle {
    token: String,
    wire_cap: String,
    room_key: [u8; wire_aead::ROOM_KEY_LEN],
}

fn fetch_room_token(
    world_url: &str,
    room_id: &str,
    access: &str,
    role: &str,
    session_pub: &[u8],
) -> Result<RoomTokenBundle> {
    let base = world_url.trim_end_matches('/');
    let authz = format!("Bearer {access}");
    let session_pub_b64 = session_pub_b64(session_pub)?;
    // x0021: do not create rooms here — host-server owns create/owner_sub.
    // x003e: session_pub required for cnf.spk.
    // x004e: wire_cap required for Zenoh keys.
    let resp: TokenResponse = ureq::post(&format!("{base}/api/v1/rooms/{room_id}/tokens"))
        .set("content-type", "application/json")
        .set("authorization", &authz)
        .timeout(Duration::from_secs(15))
        .send_json(serde_json::json!({
            "role": role,
            "session_pub": session_pub_b64,
        }))
        .context("issue Room Token (is host-server up and room created?)")?
        .into_json()
        .context("parse Room Token")?;
    if resp.token.is_empty() {
        bail!("Room Token response empty");
    }
    let wire_cap =
        zenoh_connect::require_wire_cap(Some(resp.wire_cap.as_str())).context("e2e wire_cap")?;
    let room_key = wire_aead::decode_room_key_b64(&resp.room_key).context("e2e room_key")?;
    Ok(RoomTokenBundle {
        token: resp.token,
        wire_cap,
        room_key,
    })
}

fn open_zenoh(endpoint: &str) -> Result<zenoh::Session> {
    zenoh_connect::open_client(endpoint)
}

fn client_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn put_input(
    session: &zenoh::Session,
    key: &str,
    room_id: &str,
    room_key: &[u8],
    event: &InputEvent,
) -> Result<()> {
    let env = wire_aead::seal(
        room_key,
        room_id,
        0,
        AeadKind::Input,
        &event.encode_to_vec(),
    )
    .map_err(|e| anyhow!("AEAD seal input: {e}"))?;
    // x0077: stamp so host bind rate keys off this session's ZenohId.
    session
        .put(key, env)
        .timestamp(session.new_timestamp())
        .wait()
        .map_err(|e| anyhow!("zenoh put input: {e}"))?;
    Ok(())
}

/// Ceiling on `ZBytes::len` → copy → AEAD open (single check site for Frame / Snapshot).
fn open_zenoh_sample(
    room_id: &str,
    room_key: &[u8],
    kind: AeadKind,
    bytes: &ZBytes,
) -> Result<Vec<u8>> {
    wire_aead::check_aead_envelope_bytes(kind, bytes.len())
        .map_err(|e| anyhow!("{kind:?} envelope: {e}"))?;
    let envelope = bytes.to_bytes();
    wire_aead::open(room_key, room_id, 0, kind, envelope.as_ref())
        .map_err(|e| anyhow!("AEAD open {kind:?}: {e}"))
}

fn put_raw(session: &zenoh::Session, key: &str, bytes: Vec<u8>) -> Result<()> {
    session
        .put(key, bytes)
        .wait()
        .map_err(|e| anyhow!("zenoh put raw on {key}: {e}"))?;
    Ok(())
}

/// Case A: plaintext Manifest bytes (no Room AEAD) — guest must fail at AEAD open.
fn put_manifest_plaintext(
    session: &zenoh::Session,
    key: &str,
    manifest: &SceneManifest,
) -> Result<()> {
    put_raw(session, key, manifest.encode_to_vec())
}

/// Case B / host-shaped: seal Manifest plaintext with the room's key + Manifest AAD.
fn put_manifest_aead(
    session: &zenoh::Session,
    key: &str,
    room_id: &str,
    room_key: &[u8],
    manifest: &SceneManifest,
) -> Result<()> {
    put_manifest_aead_bytes(session, key, room_id, room_key, &manifest.encode_to_vec())
}

fn put_manifest_aead_bytes(
    session: &zenoh::Session,
    key: &str,
    room_id: &str,
    room_key: &[u8],
    plaintext: &[u8],
) -> Result<()> {
    let env = wire_aead::seal(room_key, room_id, 0, AeadKind::Manifest, plaintext)
        .map_err(|e| anyhow!("AEAD seal manifest: {e}"))?;
    put_raw(session, key, env)
}

/// Per adversarial put: independent of happy-path `deadline` so a slow login/frame
/// phase does not starve later oversize asserts (x0046 review).
const ADVERSARIAL_STEP_WAIT: Duration = Duration::from_secs(10);

/// Drop Frames already queued so the next wait observes a **new** publish after put.
fn drain_pending_frames(try_recv: &mut impl FnMut() -> Option<ZBytes>) {
    while try_recv().is_some() {}
}

fn drain_pending_samples(try_recv: &mut impl FnMut() -> Option<ZBytes>) {
    while try_recv().is_some() {}
}

/// Wait for an AEAD-openable, decodable RenderFrame after a prior drain + put (x0069).
fn expect_new_frame(
    try_recv: &mut impl FnMut() -> Option<ZBytes>,
    room_id: &str,
    room_key: &[u8],
    context: &str,
) -> Result<()> {
    let post_deadline = Instant::now() + ADVERSARIAL_STEP_WAIT;
    while Instant::now() < post_deadline {
        if let Some(bytes) = try_recv() {
            if let Ok(payload) = open_zenoh_sample(room_id, room_key, AeadKind::Frame, &bytes) {
                if RenderFrame::decode(payload.as_ref()).is_ok() {
                    return Ok(());
                }
            }
        } else {
            thread::sleep(Duration::from_millis(50));
        }
    }
    bail!("{context}");
}

/// Guest-side classification of a Manifest sample (mirrors client dual-wall order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManifestGuestOutcome {
    /// Gate 1: envelope / tag / AAD / wrong key failed.
    AeadDiscard,
    /// Gate 2: AEAD opened, then protobuf / pin / empty `world_seal` rejected.
    PostAeadDiscard,
    /// Looks like host traffic (pin+room+host_pub ok, non-empty seal; JWT not checked here).
    HostShaped,
}

fn classify_manifest_guest(room_id: &str, room_key: &[u8], bytes: &ZBytes) -> ManifestGuestOutcome {
    let payload = match open_zenoh_sample(room_id, room_key, AeadKind::Manifest, bytes) {
        Ok(pt) => pt,
        Err(_) => return ManifestGuestOutcome::AeadDiscard,
    };
    match SceneManifest::decode(payload.as_ref()) {
        Err(_) => ManifestGuestOutcome::PostAeadDiscard,
        Ok(manifest) => {
            if check_manifest(&manifest).is_err() {
                return ManifestGuestOutcome::PostAeadDiscard;
            }
            if manifest.protocol_pin != PROTOCOL_PIN
                || manifest.room_id != room_id
                || manifest.host_pub.len() != 32
                || manifest.world_seal.is_empty()
            {
                return ManifestGuestOutcome::PostAeadDiscard;
            }
            ManifestGuestOutcome::HostShaped
        }
    }
}

/// Wait until a Manifest sample matches `want`, skipping concurrent host-shaped publishes.
fn expect_manifest_guest_outcome(
    try_recv: &mut impl FnMut() -> Option<ZBytes>,
    room_id: &str,
    room_key: &[u8],
    want: ManifestGuestOutcome,
    context: &str,
) -> Result<()> {
    debug_assert!(
        want != ManifestGuestOutcome::HostShaped,
        "e2e waits for discard outcomes only"
    );
    let post_deadline = Instant::now() + ADVERSARIAL_STEP_WAIT;
    while Instant::now() < post_deadline {
        if let Some(bytes) = try_recv() {
            match classify_manifest_guest(room_id, room_key, &bytes) {
                outcome if outcome == want => return Ok(()),
                ManifestGuestOutcome::HostShaped => {}
                other => {
                    log::debug!("manifest guest sample {other:?} (want {want:?}); keep waiting");
                }
            }
        } else {
            thread::sleep(Duration::from_millis(50));
        }
    }
    bail!("{context}");
}

/// Adversarial Manifest: wrong pin so any verifying peer must discard (x0038 / Case B).
fn fake_manifest(room_id: &str) -> SceneManifest {
    SceneManifest {
        room_id: room_id.into(),
        revision: 0,
        entries: vec![],
        locomotion: None,
        protocol_pin: "bogus-pin-x0038".into(),
        host_pub: vec![0u8; 32],
        host_attestation: String::new(),
        world_seal: String::new(),
        host_generation: 0,
    }
}

/// x003a / Case B: seal-less "stale revision" Manifest (empty `world_seal`).
///
/// Does **not** prove discard of a cryptographically valid old JWT seal (unit-tested).
/// Wrapped in correct Room AEAD so the guest reaches the ManifestSeal gate.
fn stale_revision_manifest(room_id: &str) -> SceneManifest {
    SceneManifest {
        room_id: room_id.into(),
        revision: 0,
        entries: vec![],
        locomotion: None,
        protocol_pin: PROTOCOL_PIN.into(),
        host_pub: vec![0u8; 32],
        host_attestation: String::new(),
        world_seal: String::new(),
        host_generation: 1,
    }
}

fn move_input(key: &SigningKey, room_token: Option<&str>, seq: u64) -> InputEvent {
    let mut event = InputEvent {
        room_token: room_token.unwrap_or("").to_string(),
        client_time_ms: client_time_ms(),
        move_x: 0.1,
        move_y: 0.0,
        move_z: 0.0,
        buttons: 0,
        grab_entity_id: 0,
        aim_x: 0.0,
        aim_y: 1.6,
        aim_z: 0.0,
        leave: false,
        session_pub: public_key_bytes(key),
        intent_seq: seq,
        intent_sig: vec![],
        ..Default::default()
    };
    sign_input(key, &mut event);
    event
}

fn frame_has_user_root(frame: &RenderFrame) -> bool {
    let last = user_root_entity_last();
    frame
        .upserts
        .iter()
        .any(|e| (USER_ROOT_ENTITY_FIRST..=last).contains(&e.id))
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let auth_url = env_or("AUTH_URL", DEFAULT_AUTH_URL);
    let world_url = env_or("WORLD_URL", DEFAULT_WORLD_URL);
    let assets_url = env_or("ASSETS_URL", DEFAULT_ASSETS_URL);
    let zenoh_connect = env_or("ZENOH_CONNECT", DEFAULT_ZENOH_CONNECT);
    let room_id_raw = env_or("ROOM_ID", DEFAULT_ROOM_ID);
    let room_id = validate_room_id(&room_id_raw)
        .map_err(|e| anyhow!("invalid ROOM_ID={room_id_raw:?}: {e}"))?
        .to_string();
    let user = env_or("AUTH_USER", DEFAULT_AUTH_USER);
    let password = env_or("AUTH_PASSWORD", DEFAULT_AUTH_PASSWORD);
    let timeout_secs: u64 = env_or("E2E_TIMEOUT_SECS", &DEFAULT_TIMEOUT_SECS.to_string())
        .parse()
        .context("E2E_TIMEOUT_SECS")?;
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);

    log::info!(
        "e2e-probe start; protocol_pin={PROTOCOL_PIN} auth={auth_url} world={world_url} assets={assets_url} zenoh={zenoh_connect} room={room_id} timeout_secs={timeout_secs}"
    );

    health_get(&auth_url).context("auth health (is compose up?)")?;
    health_get(&world_url).context("world health")?;
    health_get(&assets_url).context("assets health")?;
    log::info!("health ok (auth/world/assets)");

    let access = login(&auth_url, &user, &password)?;
    log::info!("login ok");

    // x003e: mint session key before Token so cnf.spk matches bind Intent.
    let session_key = generate_signing_key();
    let room = fetch_room_token(
        &world_url,
        &room_id,
        &access,
        "guest",
        &public_key_bytes(&session_key),
    )?;
    let room_token = room.token;
    let wire_cap = room.wire_cap;
    let room_key = room.room_key;
    log::info!(
        "Room Token ok (role=guest, len={}, wire_cap_len={})",
        room_token.len(),
        wire_cap.len()
    );

    let session = open_zenoh(&zenoh_connect).context("open zenoh (is zenohd up?)")?;
    let frame_k = frame_key(&room_id, &wire_cap);
    let input_k = input_key(&room_id, &wire_cap);

    let subscriber = session
        .declare_subscriber(frame_k.clone())
        .wait()
        .map_err(|e| anyhow!("declare_subscriber frame: {e}"))?;
    log::info!("subscribed {frame_k}");

    thread::sleep(Duration::from_millis(200));

    let mut intent_seq = 1u64;
    put_input(
        &session,
        &input_k,
        &room_id,
        &room_key,
        &move_input(&session_key, Some(&room_token), intent_seq),
    )?;
    log::info!("put valid signed InputEvent (bind) on {input_k}");

    let mut saw_upserts = false;
    let mut saw_user_root = false;
    let mut saw_host_sig = false;
    let mut frames = 0u32;

    while Instant::now() < deadline {
        match subscriber.try_recv() {
            Ok(Some(sample)) => {
                let bytes: ZBytes = sample.payload().clone();
                match open_zenoh_sample(&room_id, &room_key, AeadKind::Frame, &bytes) {
                    Ok(payload) => match RenderFrame::decode(payload.as_ref()) {
                        Ok(frame) => {
                            frames += 1;
                            if frame.host_sig.len() == 64 {
                                saw_host_sig = true;
                            }
                            if !frame.upserts.is_empty() {
                                saw_upserts = true;
                            }
                            if frame_has_user_root(&frame) {
                                saw_user_root = true;
                            }
                            if saw_upserts && saw_user_root && saw_host_sig {
                                break;
                            }
                        }
                        Err(err) => log::warn!("RenderFrame decode failed: {err}"),
                    },
                    Err(err) => log::warn!("RenderFrame AEAD open failed: {err}"),
                }
            }
            Ok(None) | Err(_) => {
                // Re-put while waiting so a missed first input still assigns presence.
                if !saw_user_root {
                    intent_seq += 1;
                    // Keep Token on retries until presence assigns (bind may have been missed).
                    let _ = put_input(
                        &session,
                        &input_k,
                        &room_id,
                        &room_key,
                        &move_input(&session_key, Some(&room_token), intent_seq),
                    );
                }
                thread::sleep(Duration::from_millis(50));
            }
        }
    }

    if !saw_upserts {
        bail!(
            "timeout waiting for RenderFrame with upserts (frames_seen={frames}; is host-server publishing?)"
        );
    }
    log::info!("RenderFrame with upserts ok (frames_seen={frames})");

    if !saw_host_sig {
        bail!("timeout waiting for RenderFrame.host_sig (64 bytes; x0023)");
    }
    log::info!("RenderFrame.host_sig present ok");

    if !saw_user_root {
        bail!(
            "timeout waiting for user_root entity ({}..{}) after valid input (frames_seen={frames})",
            USER_ROOT_ENTITY_FIRST,
            user_root_entity_last()
        );
    }
    log::info!("user_root upsert after valid input ok");

    // Snapshot queryable should return a signed full pose set (x0023).
    let snap_k = snapshot_key(&room_id, &wire_cap);
    let mut saw_snapshot = false;
    match session.get(&snap_k).wait() {
        Ok(receiver) => {
            while let Ok(reply) = receiver.recv() {
                if let Ok(sample) = reply.result() {
                    let bytes: ZBytes = sample.payload().clone();
                    if let Ok(payload) =
                        open_zenoh_sample(&room_id, &room_key, AeadKind::Snapshot, &bytes)
                    {
                        if let Ok(frame) = RenderFrame::decode(payload.as_ref()) {
                            if frame.host_sig.len() == 64 && !frame.upserts.is_empty() {
                                saw_snapshot = true;
                                break;
                            }
                        }
                    }
                }
            }
        }
        Err(err) => log::warn!("snapshot get failed: {err}"),
    }
    if !saw_snapshot {
        bail!("snapshot queryable on {snap_k} did not return signed upserts");
    }
    log::info!("snapshot queryable ok ({snap_k})");

    // --- adversarial suite (x0038 / x003a / x0046 / x0069) ---
    // Each step: drain → put → assert guest Manifest gate (when applicable) → new Frame.
    // Per-step wait is independent of happy-path `deadline`.
    let mut poll_frame = || match subscriber.try_recv() {
        Ok(Some(sample)) => Some(sample.payload().clone()),
        Ok(None) | Err(_) => None,
    };

    let manifest_k = manifest_key(&room_id, &wire_cap);
    let manifest_sub = session
        .declare_subscriber(manifest_k.clone())
        .wait()
        .map_err(|e| anyhow!("declare_subscriber manifest: {e}"))?;
    log::info!("subscribed {manifest_k} (guest dual-wall assert)");
    let mut poll_manifest = || match manifest_sub.try_recv() {
        Ok(Some(sample)) => Some(sample.payload().clone()),
        Ok(None) | Err(_) => None,
    };

    // Invalid Input must not kill the host.
    let bad = InputEvent {
        room_token: "not-a-valid-jwt".into(),
        client_time_ms: client_time_ms(),
        move_x: 1.0,
        move_y: 0.0,
        move_z: 0.0,
        buttons: 0,
        grab_entity_id: 0,
        aim_x: 0.0,
        aim_y: 0.0,
        aim_z: 0.0,
        leave: false,
        session_pub: vec![0u8; 32],
        intent_seq: 99,
        intent_sig: vec![0u8; 64],
        ..Default::default()
    };
    drain_pending_frames(&mut poll_frame);
    put_input(&session, &input_k, &room_id, &room_key, &bad)?;
    log::info!("put invalid InputEvent (expect host drop)");
    expect_new_frame(
        &mut poll_frame,
        &room_id,
        &room_key,
        "host stopped publishing frames after invalid token input",
    )?;
    log::info!("host still publishing after invalid token ok");

    // --- Manifest Case A (gate 1: AEAD) ---
    // A1: plaintext junk — AEAD open must fail before protobuf / ManifestSeal.
    drain_pending_samples(&mut poll_manifest);
    drain_pending_frames(&mut poll_frame);
    put_manifest_plaintext(&session, &manifest_k, &fake_manifest(&room_id))?;
    log::info!("Case A1: put plaintext fake SceneManifest (expect AEAD discard)");
    expect_manifest_guest_outcome(
        &mut poll_manifest,
        &room_id,
        &room_key,
        ManifestGuestOutcome::AeadDiscard,
        "guest did not AEAD-discard plaintext Manifest",
    )?;
    expect_new_frame(
        &mut poll_frame,
        &room_id,
        &room_key,
        "host stopped publishing frames after plaintext Manifest",
    )?;
    log::info!("Case A1 ok (AEAD discard + host Frame continues)");

    // A2: ciphertext sealed with a different room key (Untrusted Host / wrong room).
    let wrong_room_key = [0xA5u8; wire_aead::ROOM_KEY_LEN];
    drain_pending_samples(&mut poll_manifest);
    drain_pending_frames(&mut poll_frame);
    put_manifest_aead(
        &session,
        &manifest_k,
        &room_id,
        &wrong_room_key,
        &fake_manifest(&room_id),
    )?;
    log::info!("Case A2: put Manifest sealed with wrong room_key (expect AEAD discard)");
    expect_manifest_guest_outcome(
        &mut poll_manifest,
        &room_id,
        &room_key,
        ManifestGuestOutcome::AeadDiscard,
        "guest did not AEAD-discard wrong-key Manifest",
    )?;
    expect_new_frame(
        &mut poll_frame,
        &room_id,
        &room_key,
        "host stopped publishing frames after wrong-key Manifest",
    )?;
    log::info!("Case A2 ok (AEAD discard + host Frame continues)");

    // --- Manifest Case B (gate 2: post-AEAD protobuf / pin / ManifestSeal) ---
    // B1: correct room_key+AAD wrapping bogus-pin Manifest (same-room malice).
    drain_pending_samples(&mut poll_manifest);
    drain_pending_frames(&mut poll_frame);
    put_manifest_aead(
        &session,
        &manifest_k,
        &room_id,
        &room_key,
        &fake_manifest(&room_id),
    )?;
    log::info!("Case B1: put AEAD-wrapped fake SceneManifest (expect post-AEAD discard)");
    expect_manifest_guest_outcome(
        &mut poll_manifest,
        &room_id,
        &room_key,
        ManifestGuestOutcome::PostAeadDiscard,
        "guest did not post-AEAD-discard AEAD-wrapped fake Manifest",
    )?;
    expect_new_frame(
        &mut poll_frame,
        &room_id,
        &room_key,
        "host stopped publishing frames after AEAD-wrapped fake Manifest",
    )?;
    log::info!("Case B1 ok (post-AEAD discard + host Frame continues)");

    // B2: correct AEAD wrapping of non-protobuf junk → decode boundary discard.
    drain_pending_samples(&mut poll_manifest);
    drain_pending_frames(&mut poll_frame);
    put_manifest_aead_bytes(
        &session,
        &manifest_k,
        &room_id,
        &room_key,
        b"not-a-scene-manifest-protobuf",
    )?;
    log::info!("Case B2: put AEAD-wrapped corrupt Manifest bytes (expect decode discard)");
    expect_manifest_guest_outcome(
        &mut poll_manifest,
        &room_id,
        &room_key,
        ManifestGuestOutcome::PostAeadDiscard,
        "guest did not post-AEAD-discard corrupt Manifest protobuf",
    )?;
    expect_new_frame(
        &mut poll_frame,
        &room_id,
        &room_key,
        "host stopped publishing frames after AEAD-wrapped corrupt Manifest",
    )?;
    log::info!("Case B2 ok (post-AEAD discard + host Frame continues)");

    // B3: seal-less stale-rev under correct AEAD → empty world_seal gate (x003a).
    drain_pending_samples(&mut poll_manifest);
    drain_pending_frames(&mut poll_frame);
    put_manifest_aead(
        &session,
        &manifest_k,
        &room_id,
        &room_key,
        &stale_revision_manifest(&room_id),
    )?;
    log::info!(
        "Case B3: put AEAD-wrapped seal-less stale-rev Manifest (expect empty world_seal discard)"
    );
    expect_manifest_guest_outcome(
        &mut poll_manifest,
        &room_id,
        &room_key,
        ManifestGuestOutcome::PostAeadDiscard,
        "guest did not post-AEAD-discard seal-less stale-rev Manifest",
    )?;
    expect_new_frame(
        &mut poll_frame,
        &room_id,
        &room_key,
        "host stopped publishing frames after seal-less stale-rev Manifest",
    )?;
    log::info!("Case B3 ok (post-AEAD discard + host Frame continues)");

    // x0046 / x0069: oversize Input hits host AEAD envelope ceiling; host must keep publishing.
    let oversize = wire_aead::max_aead_envelope_bytes(AeadKind::Input) + 1;
    drain_pending_frames(&mut poll_frame);
    put_raw(&session, &input_k, vec![0u8; oversize])?;
    log::info!("put oversize Input payload ({oversize} bytes; expect host drop)");
    expect_new_frame(
        &mut poll_frame,
        &room_id,
        &room_key,
        "host stopped publishing frames after oversize Input",
    )?;
    log::info!("host still publishing after oversize Input ok");

    // Case A (oversize): envelope exceeds Manifest AEAD ceiling → guest AEAD discard;
    // also asserts host Frame survival after large wire put (host is publisher-only).
    let oversize_manifest = wire_aead::max_aead_envelope_bytes(AeadKind::Manifest) + 1;
    drain_pending_samples(&mut poll_manifest);
    drain_pending_frames(&mut poll_frame);
    put_raw(&session, &manifest_k, vec![0u8; oversize_manifest])?;
    log::info!(
        "Case A3: put oversize Manifest envelope ({oversize_manifest} bytes; expect AEAD discard)"
    );
    expect_manifest_guest_outcome(
        &mut poll_manifest,
        &room_id,
        &room_key,
        ManifestGuestOutcome::AeadDiscard,
        "guest did not AEAD-discard oversize Manifest envelope",
    )?;
    expect_new_frame(
        &mut poll_frame,
        &room_id,
        &room_key,
        "host stopped publishing frames after oversize Manifest",
    )?;
    log::info!("Case A3 ok (AEAD discard + host Frame continues)");

    println!(
        "e2e-probe: ok (login→token→input→frame+user_root; adversarial: Manifest Case A/B + oversize)"
    );
    Ok(())
}
