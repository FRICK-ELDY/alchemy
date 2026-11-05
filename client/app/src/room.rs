//! Room lifecycle: Zenoh open/close, join, bus start, token refresh, Leave.
//!
//! Guarantees:
//! - Explicit Leave + Zenoh liveliness on join/logout (x0019); Intent TTL is host fallback
//! - `close_zenoh` uses `session.close()` (not Drop alone) so bus threads holding clones stop
//! - Access + Room Token auto-refresh at ~80% TTL while joined (x0021 P1-1); pending Intent
//!   session commit respects backoff
//! - Intent Ed25519 session minted before guest Room Token fetch so `cnf.spk` matches bind
//!   `session_pub` (x003e)
//! - `ensure_room_bus` idempotent via `room_bus_started`; spawn failure clears the flag
//! - Reopen after join only when `mtls_reopen_pending` or session was dropped (x0068)
//!
//! Non-guarantees: wire decode/verify (see `bus::*`), assets / mesh (see `assets_fetch`), Steal,
//! email verify UI, Quest standalone.

use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use alchemy_protocol::wire_aead::AeadKind;
use alchemy_protocol::{frame_key, input_key, manifest_key, presence_key, Message};
use anyhow::{Context, Result};
use zenoh::Wait;

use crate::auth;
use crate::bus::frame::spawn_frame_subscriber;
use crate::bus::input::spawn_input_publisher;
use crate::bus::liveliness::declare_room_alive;
use crate::bus::manifest::spawn_manifest_subscriber;
use crate::bus::presence::spawn_presence_subscriber;
use crate::bus::wire::{client_time_ms, seal_zenoh_payload};
use crate::intent_session;
use crate::jwt_sub;
use crate::scene::SharedState;
use crate::world_token;

const TOKEN_REFRESH_POLL: Duration = Duration::from_secs(5);

/// Keep process env in sync for zenoh-connect helpers that still read `ZENOH_CONNECT`.
fn sync_zenoh_connect_env(endpoint: &str) {
    std::env::set_var("ZENOH_CONNECT", endpoint);
}

/// Explicit close so bus threads stop even when another strong `Session` clone still exists.
///
/// Zenoh 1.x only auto-`close`s on `Drop` when `strong_counter == 1`. Leaving App's slot as
/// `None` alone is not enough if a spawn_* thread retained a clone (see presence historically).
pub fn close_zenoh(zenoh: &mut Option<zenoh::Session>) {
    if let Some(session) = zenoh.take() {
        if let Err(err) = session.close().wait() {
            log::warn!("zenoh session close failed: {err}");
        }
    }
}

/// Open using SharedState endpoint (not a stale boot-time env snapshot).
pub(crate) fn open_zenoh_for(shared: &SharedState) -> Result<zenoh::Session> {
    let endpoint = shared.get_zenoh_connect();
    sync_zenoh_connect_env(&endpoint);
    zenoh_connect::open_client(&endpoint)
}

/// Apply auth session + guest Room Token into SharedState (x0018 join).
/// x003e: mint Intent session first so Token `cnf.spk` matches bind `session_pub`.
pub fn join_room_with_access(shared: &SharedState, access_token: &str) -> Result<()> {
    let session = intent_session::IntentSession::new();
    let session_pub_b64 = session.session_pub_b64();
    let (csr_pem, eph_key) = if zenoh_connect::should_request_world_mtls() {
        let eph = zenoh_connect::EphemeralMtlsIdentity::generate().context("ephemeral mTLS CSR")?;
        (
            Some(eph.csr_pem.clone()),
            Some(eph.private_key_pem().to_string()),
        )
    } else {
        (None, None)
    };
    let world_url = shared.get_world_url();
    let guest = world_token::fetch_guest_token(
        &world_url,
        &shared.room_id,
        access_token,
        &session_pub_b64,
        csr_pem.as_deref(),
    )
    .context("fetch guest Room Token")?;
    let local_sub = jwt_sub::sub_unverified(&guest.token)
        .context("read Room Token sub (unverified payload)")?;
    if let Ok(mut cap) = shared.wire_cap.lock() {
        *cap = Some(Arc::<str>::from(guest.wire_cap.as_str()));
    }
    if let Ok(mut key) = shared.room_key.lock() {
        *key = Some(Arc::new(guest.room_key));
    }
    // mTLS: world-issued preferred; else Relay allocate (x0066 / x004e).
    if let Some(id) = zenoh_connect::apply_world_mtls_or_allocate(
        guest.mtls.as_ref().map(|m| m.client_cert_pem.as_str()),
        eph_key.as_deref(),
        &guest.wire_cap,
        "guest",
        &guest.allocation_token,
        guest
            .mtls
            .as_ref()
            .map(|m| m.expires_at.as_str())
            .filter(|s| !s.is_empty()),
    )
    .context("guest mTLS (world or relay allocate)")?
    {
        shared.mtls_reopen_pending.store(true, Ordering::SeqCst);
        log::info!(
            "guest mTLS ready cn={} role={} soft_ttl={}s (mtls_reopen_pending)",
            id.cn,
            id.role,
            id.expires_in
        );
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

/// Start Zenoh room subscribers once `wire_cap` **and** `room_key` are known
/// (silent boot or Home→Join; x004e / x0069).
///
/// Idempotent via `room_bus_started`. On spawn failure the flag is cleared so retry works.
/// Pass `Some(blake3_tx)` for boot mesh wait; `None` drains blake3 on a background thread
/// (late Home→Join mesh remount is non-goal).
pub(crate) fn ensure_room_bus(
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
        if shared.room_key().is_none() {
            anyhow::bail!("ensure_room_bus: room_key missing");
        }
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
    // Consume pending up front; restore on *any* Err so poll_mtls_reopen can retry (x0068).
    let mtls_pending = shared.mtls_reopen_pending.swap(false, Ordering::SeqCst);
    match bind_room_after_join_inner(shared, zenoh, mtls_pending) {
        Ok(()) => Ok(()),
        Err(err) => {
            if mtls_pending {
                shared.mtls_reopen_pending.store(true, Ordering::SeqCst);
            }
            Err(err)
        }
    }
}

fn bind_room_after_join_inner(
    shared: &Arc<SharedState>,
    zenoh: &mut Option<zenoh::Session>,
    mtls_pending: bool,
) -> Result<()> {
    if mtls_pending || zenoh.is_none() {
        // Match Leave / orphan path: always close before replace (not only Drop-on-overwrite).
        close_zenoh(zenoh);
        let session = open_zenoh_for(shared).context("zenoh open/reopen after join")?;
        *zenoh = Some(session);
        shared.room_bus_started.store(false, Ordering::SeqCst);
        log::info!(
            "zenoh session {} after join (mtls_pending={mtls_pending}) endpoint={}",
            if mtls_pending { "reopened" } else { "opened" },
            shared.get_zenoh_connect()
        );
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
        *zenoh = Some(open_zenoh_for(shared).context("zenoh reopen after bus spawn failure")?);
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
pub(crate) fn spawn_token_refresh(shared: Arc<SharedState>) -> Result<()> {
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
                        Some(rt) => match auth::refresh(&shared.get_auth_url(), &rt) {
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
                    // Guest refresh: re-issue Room Token; when WAN, new CSR → world mtls env.
                    // x0068: set mtls_reopen_pending so desktop/XR reapplies via bind_room_after_join.
                    let (csr_pem, eph_key) = if zenoh_connect::should_request_world_mtls() {
                        match zenoh_connect::EphemeralMtlsIdentity::generate() {
                            Ok(eph) => (
                                Some(eph.csr_pem.clone()),
                                Some(eph.private_key_pem().to_string()),
                            ),
                            Err(err) => {
                                log::warn!("guest refresh CSR failed: {err:#}");
                                (None, None)
                            }
                        }
                    } else {
                        (None, None)
                    };
                    let world_url = shared.get_world_url();
                    match world_token::fetch_guest_token(
                        &world_url,
                        &shared.room_id,
                        &access,
                        &session_pub_b64,
                        csr_pem.as_deref(),
                    ) {
                        Ok(guest) => {
                            match zenoh_connect::apply_world_mtls_or_allocate(
                                guest.mtls.as_ref().map(|m| m.client_cert_pem.as_str()),
                                eph_key.as_deref(),
                                &guest.wire_cap,
                                "guest",
                                &guest.allocation_token,
                                guest
                                    .mtls
                                    .as_ref()
                                    .map(|m| m.expires_at.as_str())
                                    .filter(|s| !s.is_empty()),
                            ) {
                                Ok(Some(id)) => {
                                    shared.mtls_reopen_pending.store(true, Ordering::SeqCst);
                                    log::info!(
                                        "guest refresh mTLS cn={} soft_ttl={}s (mtls_reopen_pending)",
                                        id.cn,
                                        id.expires_in
                                    );
                                }
                                Ok(None) => {}
                                Err(err) => {
                                    log::warn!("guest refresh mTLS update failed: {err:#}");
                                }
                            }
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
                        match seal_zenoh_payload(shared, AeadKind::Input, &event.encode_to_vec()) {
                            Ok(env) => {
                                if let Err(err) = publisher.put(env).wait() {
                                    log::warn!("Leave InputEvent publish failed: {err}");
                                } else {
                                    log::info!("Leave InputEvent published on {key}");
                                }
                            }
                            Err(err) => log::warn!("Leave InputEvent AEAD seal failed: {err}"),
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
