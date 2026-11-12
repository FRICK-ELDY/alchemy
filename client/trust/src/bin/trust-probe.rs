//! x0070 / x0087 headless trust consumer — same gates as GUI client.
//!
//! Guarantees (hermetic): login → Room Token → Accept host Manifest → adversarial
//! fake-seal / wrong-key Discard leaves [`TrustWatermarks`] unchanged.
//! Guarantees (WAN mode via `TRUST_PROBE_MODE=wan`, x0087): real host-server Manifest
//! Accept + Frame open; no client cert fails; guest sim put does not reach a peer
//! guest; old `room_key_epoch` AEAD Discard (local); wildcard other-room ciphertext
//! unreadable with own `room_key`.
//! Non-guarantees: GUI leave, valid old-seal JWT matrix (unit), L7 room isolation,
//! `put.wait` returning Err on ACL drop (Zenoh may return Ok).

use std::env;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use alchemy_client_trust::{
    open_manifest_aead, process_manifest_bytes, DiscardReason, HostAttestationVerifier,
    ManifestSealVerifier, TrustOutcome, TrustWatermarks, WorldJwks,
};
use alchemy_protocol::wire_aead::{self, AeadKind};
use alchemy_protocol::wire_crypto::{generate_signing_key, public_key_bytes};
use alchemy_protocol::{
    frame_key, input_key, manifest_key, validate_room_id, InputEvent, Message, RenderFrame,
    SceneManifest, PROTOCOL_PIN,
};
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use zenoh::bytes::ZBytes;
use zenoh::Wait;

const DEFAULT_AUTH_URL: &str = "http://127.0.0.1:4102";
const DEFAULT_WORLD_URL: &str = "http://127.0.0.1:4103";
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
    #[serde(default)]
    mtls: Option<MtlsResponse>,
}

#[derive(Debug, Deserialize)]
struct MtlsResponse {
    client_cert_pem: String,
}

fn env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

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
    let n = (u32::from(pub_bytes[30]) << 16) | (u32::from(pub_bytes[31]) << 8);
    out.push(T[((n >> 18) & 63) as usize] as char);
    out.push(T[((n >> 12) & 63) as usize] as char);
    out.push(T[((n >> 6) & 63) as usize] as char);
    Ok(out)
}

fn create_room(world_url: &str, access: &str, room_id: &str) -> Result<()> {
    let base = world_url.trim_end_matches('/');
    match ureq::post(&format!("{base}/api/v1/rooms"))
        .set("authorization", &format!("Bearer {access}"))
        .set("content-type", "application/json")
        .timeout(Duration::from_secs(15))
        .send_json(serde_json::json!({
            "room_id": room_id,
            "visibility": "public",
        })) {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(409, _)) => Ok(()),
        Err(err) => Err(err).context(format!("create room {room_id}")),
    }
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

struct RoomBundle {
    token: String,
    wire_cap: String,
    room_key: [u8; wire_aead::ROOM_KEY_LEN],
    mtls: Option<MtlsResponse>,
}

fn fetch_room_token(
    world_url: &str,
    room_id: &str,
    access: &str,
    role: &str,
    session_pub: &[u8],
    csr_pem: Option<&str>,
) -> Result<RoomBundle> {
    let base = world_url.trim_end_matches('/');
    let spk = session_pub_b64(session_pub)?;
    let mut body = serde_json::json!({
        "role": role,
        "session_pub": spk,
    });
    if let Some(csr) = csr_pem {
        body["csr_pem"] = serde_json::Value::String(csr.to_string());
    }
    let resp: TokenResponse = ureq::post(&format!("{base}/api/v1/rooms/{room_id}/tokens"))
        .set("authorization", &format!("Bearer {access}"))
        .set("content-type", "application/json")
        .timeout(Duration::from_secs(15))
        .send_json(body)
        .context("room token")?
        .into_json()
        .context("parse room token")?;
    if resp.token.is_empty() || resp.wire_cap.is_empty() || resp.room_key.is_empty() {
        bail!("room token missing fields");
    }
    Ok(RoomBundle {
        token: resp.token,
        wire_cap: resp.wire_cap,
        room_key: wire_aead::decode_room_key_b64(&resp.room_key).context("room_key")?,
        mtls: resp.mtls,
    })
}

/// Post-identity adversarial Manifest: valid pin/room/host_pub len, forged `world_seal`.
/// Hits Seal Discard (not Identity) so the hermetic assert matches x0070「偽 seal」.
fn fake_seal_manifest(room_id: &str) -> SceneManifest {
    SceneManifest {
        room_id: room_id.into(),
        revision: 0,
        entries: vec![],
        locomotion: None,
        protocol_pin: PROTOCOL_PIN.into(),
        host_pub: vec![0u8; 32],
        host_attestation: String::new(),
        world_seal: "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.e30.e30".into(),
        host_generation: 1,
    }
}

/// Temp PEM paths for a second mTLS identity; always unlink on drop (incl. bail).
struct TempPemPair {
    cert: std::path::PathBuf,
    key: std::path::PathBuf,
}

impl TempPemPair {
    fn write(cert_pem: &str, key_pem: &str) -> Result<Self> {
        let cert = std::env::temp_dir().join(format!(
            "alchemy-trust-probe-host-{}.cert.pem",
            std::process::id()
        ));
        let key = std::env::temp_dir().join(format!(
            "alchemy-trust-probe-host-{}.key.pem",
            std::process::id()
        ));
        std::fs::write(&cert, cert_pem).context("write host cert pem")?;
        std::fs::write(&key, key_pem).context("write host key pem")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600));
        }
        Ok(Self { cert, key })
    }
}

impl Drop for TempPemPair {
    fn drop(&mut self) {
        if let Err(err) = std::fs::remove_file(&self.cert) {
            log::warn!("temp cert cleanup {}: {err}", self.cert.display());
        }
        if let Err(err) = std::fs::remove_file(&self.key) {
            log::warn!("temp key cleanup {}: {err}", self.key.display());
        }
    }
}

fn seal_aead_manifest(room_id: &str, room_key: &[u8], manifest: &SceneManifest) -> Result<Vec<u8>> {
    let plain = manifest.encode_to_vec();
    wire_aead::seal(room_key, room_id, 0, AeadKind::Manifest, &plain)
        .map_err(|e| anyhow!("seal fake manifest: {e}"))
}

fn put_aead_manifest(
    session: &zenoh::Session,
    key: &str,
    room_id: &str,
    room_key: &[u8],
    manifest: &SceneManifest,
) -> Result<()> {
    let env = seal_aead_manifest(room_id, room_key, manifest)?;
    session
        .put(key, env)
        .wait()
        .map_err(|e| anyhow!("put fake manifest: {e}"))?;
    Ok(())
}

fn put_aead_manifest_pub(
    publisher: &zenoh::pubsub::Publisher<'_>,
    room_id: &str,
    room_key: &[u8],
    manifest: &SceneManifest,
) -> Result<()> {
    let env = seal_aead_manifest(room_id, room_key, manifest)?;
    publisher
        .put(env)
        .wait()
        .map_err(|e| anyhow!("publisher put fake manifest: {e}"))?;
    Ok(())
}

fn open_zenoh(endpoint: &str) -> Result<zenoh::Session> {
    zenoh_connect::open_client(endpoint)
}

fn run_hermetic() -> Result<()> {
    let auth_url = env_or("AUTH_URL", DEFAULT_AUTH_URL);
    let world_url = env_or("WORLD_URL", DEFAULT_WORLD_URL);
    let zenoh_connect = env_or("ZENOH_CONNECT", DEFAULT_ZENOH_CONNECT);
    let room_id = validate_room_id(&env_or("ROOM_ID", DEFAULT_ROOM_ID))
        .map_err(|e| anyhow!("ROOM_ID: {e}"))?
        .to_string();
    let user = env_or("AUTH_USER", DEFAULT_AUTH_USER);
    let password = env_or("AUTH_PASSWORD", DEFAULT_AUTH_PASSWORD);
    let timeout_secs: u64 = env_or("E2E_TIMEOUT_SECS", &DEFAULT_TIMEOUT_SECS.to_string())
        .parse()
        .context("E2E_TIMEOUT_SECS")?;
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);

    log::info!(
        "trust-probe hermetic; pin={PROTOCOL_PIN} auth={auth_url} world={world_url} zenoh={zenoh_connect} room={room_id}"
    );

    let access = login(&auth_url, &user, &password)?;
    let session_key = generate_signing_key();
    let room = fetch_room_token(
        &world_url,
        &room_id,
        &access,
        "guest",
        &public_key_bytes(&session_key),
        None,
    )?;

    let jwks = Arc::new(WorldJwks::new(&world_url));
    jwks.refresh().context("JWKS refresh")?;
    let seal = ManifestSealVerifier::new(Arc::clone(&jwks), &room_id);
    let attest = HostAttestationVerifier::new(Arc::clone(&jwks), &room_id);
    let mut watermarks = TrustWatermarks::default();

    let session = open_zenoh(&zenoh_connect)?;
    let manifest_k = manifest_key(&room_id, &room.wire_cap);
    let frame_k = frame_key(&room_id, &room.wire_cap);
    let input_k = input_key(&room_id, &room.wire_cap);

    let manifest_sub = session
        .declare_subscriber(manifest_k.clone())
        .wait()
        .map_err(|e| anyhow!("subscribe manifest: {e}"))?;
    let frame_sub = session
        .declare_subscriber(frame_k.clone())
        .wait()
        .map_err(|e| anyhow!("subscribe frame: {e}"))?;

    // Nudge host with a bind Intent so Manifest/Frame flow.
    let mut event = InputEvent {
        room_token: room.token.clone(),
        client_time_ms: Instant::now().elapsed().as_millis() as u64,
        move_x: 0.0,
        move_y: 0.0,
        move_z: 0.0,
        buttons: 0,
        grab_entity_id: 0,
        aim_x: 0.0,
        aim_y: 1.6,
        aim_z: 0.0,
        leave: false,
        session_pub: public_key_bytes(&session_key),
        intent_seq: 1,
        intent_sig: vec![],
    };
    alchemy_protocol::wire_crypto::sign_input(&session_key, &mut event);
    let env = wire_aead::seal(
        &room.room_key,
        &room_id,
        0,
        AeadKind::Input,
        &event.encode_to_vec(),
    )
    .map_err(|e| anyhow!("seal input: {e}"))?;
    // x0077: stamp so host bind rate keys off this session's ZenohId.
    session
        .put(&input_k, env)
        .timestamp(session.new_timestamp())
        .wait()
        .map_err(|e| anyhow!("put input: {e}"))?;

    let mut accepted = false;
    while Instant::now() < deadline {
        if let Ok(Some(sample)) = manifest_sub.try_recv() {
            let bytes: ZBytes = sample.payload().clone();
            match process_manifest_bytes(
                &room_id,
                &room.room_key,
                bytes.to_bytes().as_ref(),
                &room_id,
                &mut watermarks,
                &seal,
                &attest,
            ) {
                TrustOutcome::Accept { .. } => {
                    accepted = true;
                    log::info!(
                        "trust-probe: Accept host Manifest (gen_rev={:?})",
                        watermarks.manifest_gen_rev
                    );
                    break;
                }
                TrustOutcome::Discard { reason, detail } => {
                    log::debug!("pre-accept discard {reason:?}: {detail}");
                }
            }
        } else {
            thread::sleep(Duration::from_millis(50));
        }
    }
    if !accepted {
        bail!("timeout waiting for Accept host Manifest");
    }
    if watermarks.host_pub.is_none() || watermarks.last_world_seal.is_none() {
        bail!("Accept left watermarks incomplete");
    }
    // Concurrent host Manifest may Accept; process each sample on a scratch copy so
    // Accept never mutates joined watermarks (CI false negatives after e2e-probe).
    // Watermark freeze is asserted on `trial` after Discard (+ unit tests).
    let snapshot = watermarks.clone();

    // Wrong room_key envelope: drain → put → require sample openable with wrong_key
    // but not room_key (avoids false positive on unrelated AEAD fails).
    while let Ok(Some(_)) = manifest_sub.try_recv() {}
    let wrong_key = [0xABu8; wire_aead::ROOM_KEY_LEN];
    put_aead_manifest(
        &session,
        &manifest_k,
        &room_id,
        &wrong_key,
        &fake_seal_manifest(&room_id),
    )?;
    let step_deadline = Instant::now() + Duration::from_secs(8);
    let mut saw_wrong_key = false;
    while Instant::now() < step_deadline {
        if let Ok(Some(sample)) = manifest_sub.try_recv() {
            let bytes: ZBytes = sample.payload().clone();
            let envelope = bytes.to_bytes();
            let opens_with_wrong =
                open_manifest_aead(&room_id, &wrong_key, envelope.as_ref()).is_ok();
            let opens_with_room =
                open_manifest_aead(&room_id, &room.room_key, envelope.as_ref()).is_ok();
            if !opens_with_wrong || opens_with_room {
                // Host republish or unrelated noise — keep waiting for our put.
                continue;
            }
            let mut trial = snapshot.clone();
            match process_manifest_bytes(
                &room_id,
                &room.room_key,
                envelope.as_ref(),
                &room_id,
                &mut trial,
                &seal,
                &attest,
            ) {
                TrustOutcome::Discard {
                    reason: DiscardReason::Aead,
                    ..
                } => {
                    if trial != snapshot {
                        bail!("scratch watermarks mutated on wrong-key AEAD Discard");
                    }
                    saw_wrong_key = true;
                    break;
                }
                other => bail!("wrong-key sample: expected Aead Discard, got {other:?}"),
            }
        } else {
            thread::sleep(Duration::from_millis(40));
        }
    }
    if !saw_wrong_key {
        bail!("expected attributable wrong-key AEAD Discard");
    }
    log::info!("trust-probe: wrong-key AEAD Discard + scratch watermark stable ok");

    // Correct AEAD wrapping forged seal → Seal Discard; scratch stays at snapshot.
    while let Ok(Some(_)) = manifest_sub.try_recv() {}
    put_aead_manifest(
        &session,
        &manifest_k,
        &room_id,
        &room.room_key,
        &fake_seal_manifest(&room_id),
    )?;
    let step_deadline = Instant::now() + Duration::from_secs(8);
    let mut saw_seal = false;
    while Instant::now() < step_deadline {
        if let Ok(Some(sample)) = manifest_sub.try_recv() {
            let bytes: ZBytes = sample.payload().clone();
            let mut trial = snapshot.clone();
            match process_manifest_bytes(
                &room_id,
                &room.room_key,
                bytes.to_bytes().as_ref(),
                &room_id,
                &mut trial,
                &seal,
                &attest,
            ) {
                TrustOutcome::Discard {
                    reason: DiscardReason::Seal,
                    ..
                } => {
                    if trial != snapshot {
                        bail!("scratch watermarks mutated on fake-seal Discard");
                    }
                    saw_seal = true;
                    break;
                }
                TrustOutcome::Accept { .. } => {
                    // Concurrent host republish on scratch — ignore.
                }
                TrustOutcome::Discard { reason, .. } => {
                    log::debug!("other discard while waiting fake seal: {reason:?}");
                }
            }
        } else {
            thread::sleep(Duration::from_millis(40));
        }
    }
    if !saw_seal {
        bail!("expected Seal Discard for fake-seal Manifest");
    }
    log::info!("trust-probe: fake-seal Discard + scratch watermark stable ok");

    // Host still alive (optional Frame sample).
    let frame_deadline = Instant::now() + Duration::from_secs(5);
    let mut saw_frame = false;
    while Instant::now() < frame_deadline {
        if let Ok(Some(sample)) = frame_sub.try_recv() {
            let bytes: ZBytes = sample.payload().clone();
            if let Ok(pt) = wire_aead::open(
                &room.room_key,
                &room_id,
                0,
                AeadKind::Frame,
                bytes.to_bytes().as_ref(),
            ) {
                if RenderFrame::decode(pt.as_ref()).is_ok() {
                    saw_frame = true;
                    break;
                }
            }
        } else {
            thread::sleep(Duration::from_millis(40));
        }
    }
    if !saw_frame {
        log::warn!(
            "no Frame after adversarial (host may be slow); watermark asserts already passed"
        );
    }

    log::info!("trust-probe hermetic ok");
    Ok(())
}

fn run_wan() -> Result<()> {
    let world_url = env_or("WORLD_URL", DEFAULT_WORLD_URL);
    let auth_url = env_or("AUTH_URL", DEFAULT_AUTH_URL);
    let zenoh_connect = env_or("ZENOH_CONNECT", "tls/127.0.0.1:7547");
    let user = env_or("AUTH_USER", DEFAULT_AUTH_USER);
    let password = env_or("AUTH_PASSWORD", DEFAULT_AUTH_PASSWORD);
    let room_a = validate_room_id(&env_or("ROOM_ID", "wana"))
        .map_err(|e| anyhow!("ROOM_ID: {e}"))?
        .to_string();
    let room_b = validate_room_id(&env_or("ROOM_ID_B", "wanb"))
        .map_err(|e| anyhow!("ROOM_ID_B: {e}"))?
        .to_string();
    let timeout_secs: u64 = env_or("E2E_TIMEOUT_SECS", "90")
        .parse()
        .context("E2E_TIMEOUT_SECS")?;

    log::info!("trust-probe WAN (x0087); zenoh={zenoh_connect} room_a={room_a} room_b={room_b}");

    // 1) No client cert → open must fail.
    std::env::remove_var("ZENOH_TLS_CONNECT_CERT");
    std::env::remove_var("ZENOH_TLS_CONNECT_KEY");
    match open_zenoh(&zenoh_connect) {
        Ok(_) => bail!("WAN open without client cert must fail"),
        Err(err) => log::info!("no-cert reject ok: {err}"),
    }

    let access = login(&auth_url, &user, &password)?;
    create_room(&world_url, &access, &room_a)?;
    create_room(&world_url, &access, &room_b)?;

    let key_a = generate_signing_key();
    let eph = zenoh_connect::EphemeralMtlsIdentity::generate().context("CSR A")?;
    let bundle_a = fetch_room_token(
        &world_url,
        &room_a,
        &access,
        "guest",
        &public_key_bytes(&key_a),
        Some(&eph.csr_pem),
    )?;
    let mtls_a = bundle_a
        .mtls
        .as_ref()
        .ok_or_else(|| anyhow!("room A token missing mtls (is world MTLS_CA configured?)"))?;
    // Root CA comes from smoke `ZENOH_TLS_ROOT_CA` (same material as world signer).
    zenoh_connect::install_world_mtls_pems(
        &mtls_a.client_cert_pem,
        eph.private_key_pem(),
        &bundle_a.wire_cap,
        "guest",
        None,
    )
    .context("install guest A mTLS")?;

    let session = open_zenoh(&zenoh_connect).context("open with guest mTLS")?;
    let manifest_k = manifest_key(&room_a, &bundle_a.wire_cap);
    let frame_k = frame_key(&room_a, &bundle_a.wire_cap);
    let input_k = input_key(&room_a, &bundle_a.wire_cap);

    let jwks = Arc::new(WorldJwks::new(&world_url));
    jwks.refresh().context("JWKS refresh")?;
    let seal = ManifestSealVerifier::new(Arc::clone(&jwks), &room_a);
    let attest = HostAttestationVerifier::new(Arc::clone(&jwks), &room_a);
    let mut watermarks = TrustWatermarks::default();

    let manifest_sub = session
        .declare_subscriber(manifest_k.clone())
        .wait()
        .map_err(|e| anyhow!("subscribe manifest: {e}"))?;
    let frame_sub = session
        .declare_subscriber(frame_k.clone())
        .wait()
        .map_err(|e| anyhow!("subscribe frame: {e}"))?;

    // Nudge real host-server (smoke starts it) so Manifest/Frame flow.
    let mut event = InputEvent {
        room_token: bundle_a.token.clone(),
        client_time_ms: Instant::now().elapsed().as_millis() as u64,
        move_x: 0.0,
        move_y: 0.0,
        move_z: 0.0,
        buttons: 0,
        grab_entity_id: 0,
        aim_x: 0.0,
        aim_y: 1.6,
        aim_z: 0.0,
        leave: false,
        session_pub: public_key_bytes(&key_a),
        intent_seq: 1,
        intent_sig: vec![],
    };
    alchemy_protocol::wire_crypto::sign_input(&key_a, &mut event);
    let env = wire_aead::seal(
        &bundle_a.room_key,
        &room_a,
        0,
        AeadKind::Input,
        &event.encode_to_vec(),
    )
    .map_err(|e| anyhow!("seal input: {e}"))?;
    session
        .put(&input_k, env)
        .timestamp(session.new_timestamp())
        .wait()
        .map_err(|e| anyhow!("put input: {e}"))?;

    // 2) Accept Manifest from real host-server (CSR → credential worker → seal).
    let accept_deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut accepted = false;
    while Instant::now() < accept_deadline {
        if let Ok(Some(sample)) = manifest_sub.try_recv() {
            let bytes: ZBytes = sample.payload().clone();
            match process_manifest_bytes(
                &room_a,
                &bundle_a.room_key,
                bytes.to_bytes().as_ref(),
                &room_a,
                &mut watermarks,
                &seal,
                &attest,
            ) {
                TrustOutcome::Accept { .. } => {
                    accepted = true;
                    log::info!(
                        "WAN: Accept real host Manifest (gen_rev={:?})",
                        watermarks.manifest_gen_rev
                    );
                    break;
                }
                TrustOutcome::Discard { reason, detail } => {
                    log::debug!("pre-accept discard {reason:?}: {detail}");
                }
            }
        } else {
            thread::sleep(Duration::from_millis(50));
        }
    }
    if !accepted {
        bail!("timeout waiting for Accept from real host-server Manifest");
    }
    if watermarks.host_pub.is_none() || watermarks.last_world_seal.is_none() {
        bail!("Accept left watermarks incomplete");
    }
    let snapshot = watermarks.clone();

    // 3) Frame from real host (AEAD open + decode). Same budget as Manifest Accept.
    let frame_deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut saw_frame = false;
    while Instant::now() < frame_deadline {
        if let Ok(Some(sample)) = frame_sub.try_recv() {
            let bytes: ZBytes = sample.payload().clone();
            if let Ok(pt) = wire_aead::open(
                &bundle_a.room_key,
                &room_a,
                0,
                AeadKind::Frame,
                bytes.to_bytes().as_ref(),
            ) {
                if RenderFrame::decode(pt.as_ref()).is_ok() {
                    saw_frame = true;
                    log::info!("WAN: opened Frame from real host-server");
                    break;
                }
            }
        } else {
            thread::sleep(Duration::from_millis(50));
        }
    }
    if !saw_frame {
        bail!("timeout waiting for Frame from real host-server");
    }
    // Real host keeps putting Frame/Manifest after Accept. Leaving these declared but
    // unread saturates the session RX path and can starve a later wildcard subscriber
    // (seen after x0093 finite-pose Frame publish resumed). drop undeclares them.
    drop(frame_sub);
    drop(manifest_sub);

    // 4) Guest sim put ingress must not reach peer guests (x0067 / P1-4).
    // Zenoh put.wait() often returns Ok even when ACL drops ingress — assert non-delivery.
    // Observe window is a slice of E2E_TIMEOUT_SECS (not a fixed 3s).
    const GUEST_SIM_PUT_PROBE: &[u8] = b"guest-sim-put-probe";
    let observe_secs = timeout_secs.clamp(3, 15);
    let key_obs = generate_signing_key();
    let eph_obs = zenoh_connect::EphemeralMtlsIdentity::generate().context("CSR guest observer")?;
    let access_obs = login(&auth_url, &user, &password)?;
    let bundle_obs = fetch_room_token(
        &world_url,
        &room_a,
        &access_obs,
        "guest",
        &public_key_bytes(&key_obs),
        Some(&eph_obs.csr_pem),
    )?;
    let mtls_obs = bundle_obs
        .mtls
        .as_ref()
        .ok_or_else(|| anyhow!("observer guest token missing mtls"))?;
    let obs_pems = TempPemPair::write(&mtls_obs.client_cert_pem, eph_obs.private_key_pem())?;
    {
        let obs_session = zenoh_connect::open_client_mtls(
            &zenoh_connect,
            obs_pems.cert.to_str().context("obs cert path utf8")?,
            obs_pems.key.to_str().context("obs key path utf8")?,
        )
        .context("open guest observer mTLS")?;
        let obs_sub = obs_session
            .declare_subscriber(manifest_k.clone())
            .wait()
            .map_err(|e| anyhow!("observer subscribe manifest: {e}"))?;
        // Nudge host republish via presence join (ACL allows guest input put).
        let obs_input_k = input_key(&room_a, &bundle_obs.wire_cap);
        let mut obs_event = InputEvent {
            room_token: bundle_obs.token.clone(),
            client_time_ms: Instant::now().elapsed().as_millis() as u64,
            move_x: 0.0,
            move_y: 0.0,
            move_z: 0.0,
            buttons: 0,
            grab_entity_id: 0,
            aim_x: 0.0,
            aim_y: 1.6,
            aim_z: 0.0,
            leave: false,
            session_pub: public_key_bytes(&key_obs),
            intent_seq: 1,
            intent_sig: vec![],
        };
        alchemy_protocol::wire_crypto::sign_input(&key_obs, &mut obs_event);
        let obs_env = wire_aead::seal(
            &bundle_obs.room_key,
            &room_a,
            0,
            AeadKind::Input,
            &obs_event.encode_to_vec(),
        )
        .map_err(|e| anyhow!("seal observer input: {e}"))?;
        obs_session
            .put(&obs_input_k, obs_env)
            .timestamp(obs_session.new_timestamp())
            .wait()
            .map_err(|e| anyhow!("observer put input (liveness nudge): {e}"))?;
        // Positive liveness: observer must see at least one host Manifest before probe put.
        let alive_deadline = Instant::now() + Duration::from_secs(timeout_secs);
        let mut saw_host_sample = false;
        while Instant::now() < alive_deadline {
            if let Ok(Some(sample)) = obs_sub.try_recv() {
                let bytes: ZBytes = sample.payload().clone();
                if bytes.to_bytes().as_ref() != GUEST_SIM_PUT_PROBE {
                    saw_host_sample = true;
                    break;
                }
            } else {
                thread::sleep(Duration::from_millis(50));
            }
        }
        if !saw_host_sample {
            bail!(
                "guest observer saw no host Manifest within {timeout_secs}s; \
                 cannot trust sim-put non-delivery check"
            );
        }
        log::info!("guest observer live (received host Manifest)");
        match session.put(&manifest_k, GUEST_SIM_PUT_PROBE).wait() {
            Ok(()) => log::info!(
                "guest sim put returned Ok (Zenoh may still drop ACL ingress; checking peer)"
            ),
            Err(err) => {
                let msg = format!("{err:#}").to_lowercase();
                let deny_like = [
                    "unauthor",
                    "denied",
                    "forbidden",
                    "not allowed",
                    "acl",
                    "reject",
                    "insufficient",
                    "permission",
                ]
                .iter()
                .any(|needle| msg.contains(needle));
                if deny_like {
                    log::info!("guest sim put returned ACL-like Err: {err}");
                } else {
                    bail!(
                        "guest sim put failed unexpectedly (not ACL-like; got: {err:#}); \
                         session/router may be dead — refusing non-delivery green"
                    );
                }
            }
        }
        let observe_deadline = Instant::now() + Duration::from_secs(observe_secs);
        while Instant::now() < observe_deadline {
            if let Ok(Some(sample)) = obs_sub.try_recv() {
                let bytes: ZBytes = sample.payload().clone();
                if bytes.to_bytes().as_ref() == GUEST_SIM_PUT_PROBE {
                    bail!(
                        "guest put on sim/manifest was delivered to peer guest \
                         (WAN ACL ingress drop failed)"
                    );
                }
            } else {
                thread::sleep(Duration::from_millis(50));
            }
        }
        log::info!(
            "guest sim put not delivered to peer guest (ACL ingress drop ok; observe={observe_secs}s)"
        );
        // Close observer before TempPemPair drop (Windows may block unlink while open).
        drop(obs_sub);
        drop(obs_session);
    }

    // 5) Old room_key_epoch — **local** trust-gate check (no Zenoh put).
    // Wire-path old-epoch inject via host B is out of scope (slice やらない).
    let stale_plain = fake_seal_manifest(&room_a).encode_to_vec();
    let stale_env = wire_aead::seal(
        &bundle_a.room_key,
        &room_a,
        99,
        AeadKind::Manifest,
        &stale_plain,
    )
    .map_err(|e| anyhow!("seal old-epoch manifest: {e}"))?;
    let mut trial = snapshot.clone();
    match process_manifest_bytes(
        &room_a,
        &bundle_a.room_key,
        &stale_env,
        &room_a,
        &mut trial,
        &seal,
        &attest,
    ) {
        TrustOutcome::Discard {
            reason: DiscardReason::Aead,
            ..
        } => {
            if trial != snapshot {
                bail!("watermarks mutated on old-epoch AEAD Discard");
            }
            log::info!(
                "WAN: old-epoch AEAD Discard + watermark stable ok (local process_manifest_bytes)"
            );
        }
        other => bail!("old-epoch: expected Aead Discard, got {other:?}"),
    }

    // 6) Wildcard other-room ciphertext unreadable with room A key (x0070).
    // Zenoh puts are not retained: declare publisher + subscriber interest before put,
    // then re-put until observed (CI router can lag after compose).
    let key_b = generate_signing_key();
    let eph_host = zenoh_connect::EphemeralMtlsIdentity::generate().context("CSR host B")?;
    let access_host = login(&auth_url, &user, &password)?;
    let bundle_b = fetch_room_token(
        &world_url,
        &room_b,
        &access_host,
        "host",
        &public_key_bytes(&key_b),
        Some(&eph_host.csr_pem),
    )?;
    let mtls_b = bundle_b
        .mtls
        .as_ref()
        .ok_or_else(|| anyhow!("room B host token missing mtls"))?;
    let host_pems = TempPemPair::write(&mtls_b.client_cert_pem, eph_host.private_key_pem())?;
    let other_key = manifest_key(&room_b, &bundle_b.wire_cap);
    let other_manifest = SceneManifest {
        room_id: room_b.clone(),
        revision: 1,
        entries: vec![],
        locomotion: None,
        protocol_pin: PROTOCOL_PIN.into(),
        host_pub: vec![1u8; 32],
        host_attestation: String::new(),
        world_seal: "other-room-seal".into(),
        host_generation: 1,
    };
    let (saw_unreadable, puts, samples, room_b_samples) = {
        // Keep host B session open across re-puts; close before TempPemPair drop (Windows).
        let host_session = zenoh_connect::open_client_mtls(
            &zenoh_connect,
            host_pems.cert.to_str().context("host cert path utf8")?,
            host_pems.key.to_str().context("host key path utf8")?,
        )
        .context("open host B mTLS")?;
        // Match real host-server: interest attaches to a declared publisher (ACL egress).
        let host_pub = host_session
            .declare_publisher(other_key.clone())
            .wait()
            .map_err(|e| anyhow!("host B declare_publisher: {e}"))?;
        let wild = "alchemy/dev/room/*/*/manifest";
        let wild_sub = session
            .declare_subscriber(wild)
            .wait()
            .map_err(|e| anyhow!("wildcard subscribe: {e}"))?;
        log::info!("subscribed {wild}; host B publisher on {other_key}");
        thread::sleep(Duration::from_millis(500));

        // Same budget family as guest-sim observe / Accept (E2E_TIMEOUT_SECS).
        let wild_secs = timeout_secs.clamp(3, 15);
        let deadline = Instant::now() + Duration::from_secs(wild_secs);
        let mut saw = false;
        let mut puts: u32 = 0;
        let mut samples: u32 = 0;
        let mut room_b_samples: u32 = 0;
        while Instant::now() < deadline {
            put_aead_manifest_pub(&host_pub, &room_b, &bundle_b.room_key, &other_manifest)?;
            puts = puts.wrapping_add(1);
            if puts == 1 || puts.is_multiple_of(5) {
                log::info!("host B put AEAD Manifest on {other_key} (n={puts})");
            }
            let burst_deadline = Instant::now() + Duration::from_millis(400);
            while Instant::now() < burst_deadline {
                match wild_sub.try_recv() {
                    Ok(Some(sample)) => {
                        samples = samples.wrapping_add(1);
                        let key = sample.key_expr().as_str().to_string();
                        let bytes: ZBytes = sample.payload().clone();
                        // Exact key (not substring): room ids like wan/wana would false-match.
                        let is_room_b = key == other_key;
                        if is_room_b {
                            room_b_samples = room_b_samples.wrapping_add(1);
                        }
                        match wire_aead::open(
                            &bundle_a.room_key,
                            &room_a,
                            0,
                            AeadKind::Manifest,
                            bytes.to_bytes().as_ref(),
                        ) {
                            Err(_) if is_room_b => {
                                log::info!(
                                    "wildcard sample on {key} unreadable with room A key (ok; puts={puts} samples={samples})"
                                );
                                saw = true;
                                break;
                            }
                            Err(_) => {
                                log::debug!("AEAD fail on {key} (keep waiting for room B)");
                            }
                            Ok(_) => {
                                log::debug!("opened sample on {key} with room A key (own room?)");
                            }
                        }
                    }
                    Ok(None) => thread::sleep(Duration::from_millis(50)),
                    Err(_) => thread::sleep(Duration::from_millis(50)),
                }
            }
            if saw {
                break;
            }
        }
        (saw, puts, samples, room_b_samples)
    };
    if !saw_unreadable {
        bail!(
            "WAN wildcard: did not observe other-room ciphertext that fails open with room A key \
             (puts={puts} wild_samples={samples} room_b_samples={room_b_samples})"
        );
    }

    drop(host_pems);
    log::info!("trust-probe WAN ok (x0087 host round-trip + negatives)");
    Ok(())
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    match env_or("TRUST_PROBE_MODE", "hermetic").as_str() {
        "wan" => run_wan(),
        _ => run_hermetic(),
    }
}
