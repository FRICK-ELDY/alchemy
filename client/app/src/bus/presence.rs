//! PresenceState subscriber (x0026 `host_sig`, x003a gen/rev gate).
//!
//! Guarantees:
//! - PROTOCOL_PIN, `host_sig` (against Manifest-attested `host_pub`), and `(host_generation,
//!   revision)` monotonic gate checked before apply; regression → discard (x003a)
//! - Verify failure → discard (keep joined); leave only on explicit Leave (x0038)
//! - Class-specific decode byte/count limits and AEAD failure → drop (x003b)
//! - Camera follows Presence-assigned local `user_root`; lost assignment while joined requests
//!   Intent rebind
//! - Subscriber holds `WeakSession` only (session clone dropped) so `close()` ends `recv`
//!
//! Non-guarantees: establishing `host_pub` (see `bus::manifest`), Virtual Desktop presence parity,
//! reliable delivery of Presence puts.

use std::sync::Arc;
use std::thread;
use std::time::Instant;

use alchemy_client_trust::gate_presence_pin;
use alchemy_protocol::decode_limits::{check_payload_bytes, check_presence, WireClass};
use alchemy_protocol::generation_gate::{decide_presence_gen_rev, GenRev, GenRevDecision};
use alchemy_protocol::wire_aead::AeadKind;
use alchemy_protocol::wire_crypto::verify_presence;
use alchemy_protocol::{Message, PresenceState, PROTOCOL_PIN};
use anyhow::{Context, Result};
use zenoh::bytes::ZBytes;
use zenoh::Wait;

use super::wire::open_zenoh_payload;
use super::GATE_LOG_INTERVAL;
use crate::scene::{SharedState, LOCAL_USER_ROOT_UNSET};

pub(crate) fn spawn_presence_subscriber(
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
            let mut dropped_aead: u64 = 0;
            let mut last_gate_log = Instant::now();
            while let Ok(sample) = subscriber.recv() {
                'sample: {
                let bytes: ZBytes = sample.payload().clone();
                let payload = match open_zenoh_payload(&shared, AeadKind::Presence, &bytes) {
                    Ok(pt) => pt,
                    Err(err) => {
                        dropped_aead += 1;
                        log::warn!("drop PresenceState: AEAD: {err}");
                        break 'sample;
                    }
                };
                if let Err(err) = check_payload_bytes(WireClass::Presence, payload.len()) {
                    dropped_limits += 1;
                    log::warn!("drop PresenceState: {err}");
                    break 'sample;
                }
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
                        + dropped_aead
                        > 0
                    {
                        log::info!(
                            "presence gate: dropped_pin={dropped_pin} dropped_no_host={dropped_no_host} dropped_sig={dropped_sig} dropped_gen_rev={dropped_gen_rev} dropped_decode={dropped_decode} dropped_limits={dropped_limits} dropped_aead={dropped_aead} (last ~5s)"
                        );
                    }
                    dropped_pin = 0;
                    dropped_no_host = 0;
                    dropped_sig = 0;
                    dropped_gen_rev = 0;
                    dropped_limits = 0;
                    dropped_decode = 0;
                    dropped_aead = 0;
                    last_gate_log = Instant::now();
                }
            }
        })
        .context("spawn presence thread")?;
    Ok(())
}
