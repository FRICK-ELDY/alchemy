//! Intent (InputEvent) publisher thread (x0011 / x003e).
//!
//! Guarantees:
//! - Publishes at `INPUT_HZ` using the Ed25519 Intent session; Room Token only on bind (x003e)
//! - Payload sealed with AEAD (`AeadKind::Input`); seal failure → skip tick
//! - Each put carries `session.new_timestamp()` (x0077 host bind source key)
//! - Bind put failure keeps `bind_pending` so the next Intent still carries the Room Token
//! - Liveliness rebind after Room Token refresh only after a bind Intent put succeeds
//!
//! Non-guarantees: `put().wait()` is router accept, not host verify; no ACK / retransmit; Leave
//! Intent is published by `room::leave_room`, not here. Session-closed stop prefers
//! `Session::is_closed` + `zenoh::session::SessionClosedError`; Display substring is fallback only.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use alchemy_protocol::wire_aead::AeadKind;
use alchemy_protocol::Message;
use anyhow::{Context, Result};
use zenoh::Wait;

use super::liveliness::declare_room_alive;
use super::wire::{client_time_ms, seal_zenoh_payload};
use crate::intent_session;
use crate::scene::SharedState;

/// Match net frame rate so Grab hold aim stays smooth enough (x0011).
const INPUT_HZ: u64 = 30;

pub(crate) fn spawn_input_publisher(
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
                let env = match seal_zenoh_payload(&shared, AeadKind::Input, &event.encode_to_vec())
                {
                    Ok(env) => env,
                    Err(err) => {
                        log::warn!("input AEAD seal failed: {err}");
                        continue;
                    }
                };
                // x0077: stamp with session ZenohId so host bind rate is per-source.
                if let Err(err) = publisher
                    .put(env)
                    .timestamp(session.new_timestamp())
                    .wait()
                {
                    // Session closed/reopened: this publisher is orphaned — exit so a new
                    // bus thread can take over (avoid 30Hz warn flood + stuck Presence).
                    if session_put_means_closed(&session, &err) {
                        log::warn!(
                            "input publisher stopping (session closed); wait for bus rebind"
                        );
                        break;
                    }
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

/// Prefer typed / API signals; Display substring is last-resort for wrapped Zenoh errors.
fn session_put_means_closed(session: &zenoh::Session, err: &zenoh::Error) -> bool {
    if session.is_closed() {
        return true;
    }
    // `zenoh::Error` is `Box<dyn Error + Send + Sync>`; walk the chain for SessionClosedError.
    if error_chain_has_session_closed(err.as_ref()) {
        return true;
    }
    let msg = format!("{err}").to_ascii_lowercase();
    msg.contains("session closed")
}

fn error_chain_has_session_closed(err: &(dyn std::error::Error + 'static)) -> bool {
    use zenoh::session::SessionClosedError;
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = cur {
        if e.is::<SessionClosedError>() {
            return true;
        }
        cur = e.source();
    }
    false
}
