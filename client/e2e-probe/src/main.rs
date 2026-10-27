//! x001f / x0023 hermetic E2E probe — no GUI.
//!
//! Guarantees: against a running auth/world/assets/zenohd (+ host-server), asserts
//! login → Room Token → signed InputEvent put → RenderFrame with upserts + user_root.
//! Non-guarantees: Token refresh, WAN, R2, GUI client, Frame signature check.

use std::env;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use alchemy_protocol::wire_crypto::{
    generate_signing_key, public_key_bytes, sign_input, SigningKey,
};
use alchemy_protocol::{
    frame_key, input_key, snapshot_key, user_root_entity_last, validate_room_id, InputEvent,
    Message, RenderFrame, PROTOCOL_PIN, USER_ROOT_ENTITY_FIRST,
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

fn fetch_room_token(world_url: &str, room_id: &str, access: &str, role: &str) -> Result<String> {
    let base = world_url.trim_end_matches('/');
    let authz = format!("Bearer {access}");
    // x0021: do not create rooms here — host-server owns create/owner_sub.
    let resp: TokenResponse = ureq::post(&format!("{base}/api/v1/rooms/{room_id}/tokens"))
        .set("content-type", "application/json")
        .set("authorization", &authz)
        .timeout(Duration::from_secs(15))
        .send_json(serde_json::json!({ "role": role }))
        .context("issue Room Token (is host-server up and room created?)")?
        .into_json()
        .context("parse Room Token")?;
    if resp.token.is_empty() {
        bail!("Room Token response empty");
    }
    Ok(resp.token)
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

fn put_input(session: &zenoh::Session, key: &str, event: &InputEvent) -> Result<()> {
    let bytes = event.encode_to_vec();
    session
        .put(key, bytes)
        .wait()
        .map_err(|e| anyhow!("zenoh put input: {e}"))?;
    Ok(())
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

    let room_token = fetch_room_token(&world_url, &room_id, &access, "guest")?;
    log::info!("Room Token ok (role=guest, len={})", room_token.len());

    let session = open_zenoh(&zenoh_connect).context("open zenoh (is zenohd up?)")?;
    let frame_k = frame_key(&room_id);
    let input_k = input_key(&room_id);

    let subscriber = session
        .declare_subscriber(frame_k.clone())
        .wait()
        .map_err(|e| anyhow!("declare_subscriber frame: {e}"))?;
    log::info!("subscribed {frame_k}");

    thread::sleep(Duration::from_millis(200));

    let session_key = generate_signing_key();
    let mut intent_seq = 1u64;
    put_input(
        &session,
        &input_k,
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
                let payload = bytes.to_bytes();
                match RenderFrame::decode(payload.as_ref()) {
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
    let snap_k = snapshot_key(&room_id);
    let mut saw_snapshot = false;
    match session.get(&snap_k).wait() {
        Ok(receiver) => {
            while let Ok(reply) = receiver.recv() {
                if let Ok(sample) = reply.result() {
                    let bytes: ZBytes = sample.payload().clone();
                    if let Ok(frame) = RenderFrame::decode(bytes.to_bytes().as_ref()) {
                        if frame.host_sig.len() == 64 && !frame.upserts.is_empty() {
                            saw_snapshot = true;
                            break;
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

    // Token without valid session sig must not kill the host.
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
    };
    put_input(&session, &input_k, &bad)?;
    log::info!("put invalid InputEvent (expect host drop)");

    let mut post_bad_frame = false;
    let remain = deadline.saturating_duration_since(Instant::now());
    let post_deadline = Instant::now() + remain.min(Duration::from_secs(10));
    while Instant::now() < post_deadline {
        match subscriber.try_recv() {
            Ok(Some(sample)) => {
                let bytes: ZBytes = sample.payload().clone();
                if RenderFrame::decode(bytes.to_bytes().as_ref()).is_ok() {
                    post_bad_frame = true;
                    break;
                }
            }
            Ok(None) | Err(_) => thread::sleep(Duration::from_millis(50)),
        }
    }
    if !post_bad_frame {
        bail!("host stopped publishing frames after invalid token input");
    }
    log::info!("host still publishing after invalid token ok");

    println!("e2e-probe: ok (login→token→input→frame+user_root)");
    Ok(())
}
