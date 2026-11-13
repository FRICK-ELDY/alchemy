//! Room lifecycle: Zenoh open/close, join, bus start, token refresh, Leave.
//!
//! Guarantees:
//! - Explicit Leave + Zenoh liveliness on join/logout (x0019); Intent TTL is host fallback
//! - `close_zenoh` uses `session.close()` (not Drop alone) so bus threads holding clones stop
//! - Access + Room Token auto-refresh at ~80% TTL while joined (x0021 P1-1); pending Intent
//!   session commit respects backoff; guest ≈15s `room_key_epoch` probe + AEAD-fail nudge
//!   (x007e; host seal cadence)
//! - Intent Ed25519 session minted before guest Room Token fetch so `cnf.spk` matches bind
//!   `session_pub` (x003e)
//! - `ensure_room_bus` idempotent via `room_bus_started`; spawn failure clears the flag
//! - Reopen after join only when `mtls_reopen_pending` or session was dropped (x0068)
//! - While `room_bus_started`, soft refresh skips CSR and disk PEM rewrite (Token / epoch
//!   only). PEMs apply on next open / Leave→Join / `mtls_reopen` — mid-flight reopen is
//!   not done (full close orphans Intent → Presence TTL / liveliness Delete). NotAfter hang
//!   across a long live session is **not** guaranteed
//! - Epoch probe (~15s) only rotates Room Token / liveliness when `room_key_epoch` advances
//!   or soft-TTL is due; same-epoch probes must not churn `jti` (Presence reclaim).
//!   Probe still POSTs `/tokens` today (world has no epoch-only API) — issued jti is
//!   discarded on unchanged epoch (load/log noise; not a Presence hazard)
//! - Join starts a mesh-remount worker: Manifest blake3 → assets GET →
//!   `try_apply_world_remount` (x007d)
//! - In-flight remount after Leave / host-generation reset is discarded via
//!   `try_apply_world_remount` (generation + `wire_cap` + remounted/status under pending lock);
//!   Leave empty shell is not overwritten; stale `remounted_blake3` write-back cannot skip re-Join
//!
//! Non-guarantees: wire decode/verify (see `bus::*`), GPU buffer upload (surfaces poll
//! `pending_world_mesh`), mid-GET cancel of the HTTP request itself, GET failure retry for the
//! same blake3 within a session (sticky until Leave / re-Join), Steal, email verify UI,
//! Quest standalone.

use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use alchemy_protocol::wire_aead::AeadKind;
use alchemy_protocol::{frame_key, input_key, manifest_key, presence_key, Message};
use anyhow::{Context, Result};
use zenoh::Wait;

use crate::assets_fetch::{load_auth_mesh, try_place_read_for_mesh, AuthMeshLoad};
use crate::auth;
use crate::bus::frame::spawn_frame_subscriber;
use crate::bus::input::spawn_input_publisher;
use crate::bus::liveliness::declare_room_alive;
use crate::bus::manifest::spawn_manifest_subscriber;
use crate::bus::presence::spawn_presence_subscriber;
use crate::bus::wire::{client_time_ms, seal_zenoh_payload};
use crate::intent_session;
use crate::jwt_sub;
use crate::mesh::placeholder_error_mesh;
use crate::scene::{AssetStatusUpdate, RemountedBlake3Update, SharedState};
use crate::world_token;

const TOKEN_REFRESH_POLL: Duration = Duration::from_secs(5);

/// Keep process env in sync for zenoh-connect helpers that still read `ZENOH_CONNECT`.
fn sync_zenoh_connect_env(endpoint: &str) {
    std::env::set_var("ZENOH_CONNECT", endpoint);
}

/// Spawn remount worker + room bus with `Some(blake3_tx)` (x007d).
/// Worker is started only when the bus actually starts (`Ok(true)`).
fn start_room_bus_with_remount(
    shared: &Arc<SharedState>,
    session: &zenoh::Session,
) -> Result<bool> {
    let (blake3_tx, blake3_rx) = mpsc::sync_channel::<Result<String, String>>(4);
    let started = ensure_room_bus(shared, session, Some(blake3_tx))?;
    if started {
        let shared_worker = Arc::clone(shared);
        // If spawn fails after bus start, Manifest still holds `blake3_tx` with no consumer
        // until Leave `close_zenoh` / `room_bus_started` clear + bind retry recreates the bus.
        // Err propagates so bind does not pretend remount is live.
        thread::Builder::new()
            .name("mesh-remount".into())
            .spawn(move || mesh_remount_loop(shared_worker, blake3_rx))
            .context("spawn mesh-remount")?;
    }
    Ok(started)
}

/// Manifest blake3 → PlaceRead + assets GET → [`SharedState::try_apply_world_remount`].
///
/// Snapshot [`SharedState::remount_generation`] when a job starts; apply mesh + remounted/status
/// only through the generation-gated helper (atomic vs Leave / host-generation).
/// Placeholder / missing access-token paths do **not** set `remounted_blake3`, and Manifest
/// skips re-sending the same hex — GET failure for that blake3 stays sticky until Leave / re-Join
/// (x007d non-guarantee).
fn mesh_remount_loop(shared: Arc<SharedState>, blake3_rx: mpsc::Receiver<Result<String, String>>) {
    while let Ok(msg) = blake3_rx.recv() {
        let job_gen = shared.remount_generation.load(Ordering::SeqCst);
        match msg {
            Ok(hex) => {
                if shared.remounted_blake3().as_deref() == Some(hex.as_str()) {
                    log::debug!(
                        "mesh remount skip: already remounted blake3={}",
                        &hex[..8.min(hex.len())]
                    );
                    continue;
                }
                let access = shared
                    .auth
                    .lock()
                    .ok()
                    .and_then(|g| g.access_token.clone())
                    .unwrap_or_default();
                if access.is_empty() {
                    log::warn!("mesh remount: no access token; placeholder (sticky until Leave)");
                    if !shared.try_apply_world_remount(
                        placeholder_error_mesh(),
                        job_gen,
                        RemountedBlake3Update::Unchanged,
                        AssetStatusUpdate::Set("Mesh remount needs access token".into()),
                    ) {
                        log::debug!("mesh remount discard: left during no-token placeholder");
                    }
                    continue;
                }
                // Cheap early-out before HTTP; authoritative gate is try_apply after GET.
                if !shared.remount_apply_allowed(job_gen) {
                    log::debug!(
                        "mesh remount skip GET: generation/wire_cap stale blake3={}",
                        &hex[..8.min(hex.len())]
                    );
                    continue;
                }
                let place = try_place_read_for_mesh(&shared);
                let outcome =
                    load_auth_mesh(&shared.get_assets_url(), &hex, &access, place.as_deref());
                match outcome {
                    AuthMeshLoad::Ready(mesh) => {
                        let verts = mesh.positions.len();
                        if shared.try_apply_world_remount(
                            mesh,
                            job_gen,
                            RemountedBlake3Update::Set(Some(hex.clone())),
                            AssetStatusUpdate::Clear,
                        ) {
                            log::info!(
                                "mesh remount ready: blake3={} verts={}",
                                &hex[..8.min(hex.len())],
                                verts
                            );
                        } else {
                            log::info!(
                                "mesh remount discard: generation/wire_cap changed during GET blake3={}",
                                &hex[..8.min(hex.len())]
                            );
                        }
                    }
                    AuthMeshLoad::Placeholder { mesh, status } => {
                        // Do not set remounted_blake3 — Manifest will not re-send this hex;
                        // recovery is Leave / re-Join (documented non-guarantee).
                        if !shared.try_apply_world_remount(
                            mesh,
                            job_gen,
                            RemountedBlake3Update::Unchanged,
                            AssetStatusUpdate::Set(status),
                        ) {
                            log::info!(
                                "mesh remount discard: stale after placeholder GET blake3={}",
                                &hex[..8.min(hex.len())]
                            );
                        }
                    }
                }
            }
            Err(reason) => {
                let warn = reason.clone();
                if shared.try_apply_world_remount(
                    placeholder_error_mesh(),
                    job_gen,
                    RemountedBlake3Update::Set(None),
                    AssetStatusUpdate::Set(reason),
                ) {
                    log::warn!("mesh remount: {warn}");
                } else {
                    log::debug!("mesh remount discard: left during no-mesh Manifest");
                }
            }
        }
    }
    log::info!("mesh-remount worker exit (channel closed)");
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

/// Apply auth session + Room Token into SharedState (x0018 join).
/// x003e: mint Intent session first so Token `cnf.spk` matches bind `session_pub`.
///
/// Always joins as **guest** Room Token + guest mTLS CN (`alchemy-guest`). WAN ACL
/// only allows guest CN to put Intent / inventory keys; host CN is the host-server
/// process. Owner Inventory take-out uses guest+session; host accepts `sub ==
/// owner_sub` (x0098). [`install_host_inventory_token`] remains for compute-host
/// Intent `role=host` re-bind (non-owner takeover path).
///
/// `work_epoch` must match [`SharedState::auth_work_epoch`] at commit time so a
/// discarded Logout/Leave worker cannot install room state onto a newer session.
pub fn join_room_with_access(
    shared: &SharedState,
    access_token: &str,
    work_epoch: u64,
) -> Result<()> {
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
    let guest = world_token::fetch_room_token(
        &world_url,
        shared.get_room_id().as_ref(),
        access_token,
        &session_pub_b64,
        csr_pem.as_deref(),
        "guest",
    )
    .context("fetch guest Room Token")?;
    let local_sub = jwt_sub::sub_unverified(&guest.token)
        .context("read Room Token sub (unverified payload)")?;
    // Skip mTLS env rewrite if already superseded. Allocate still runs outside the
    // auth lock (HTTP); Leave→re-Join can race env PEMs — known; re-Join recovers.
    anyhow::ensure!(
        shared.auth_work_is_current(work_epoch),
        "join aborted: auth work superseded before mTLS"
    );
    let mtls_ready = zenoh_connect::apply_world_mtls_or_allocate(
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
    .context("room mTLS (world or relay allocate)")?;
    // Allocate may have rewritten env; refuse auth commit if superseded mid-flight.
    anyhow::ensure!(
        shared.auth_work_is_current(work_epoch),
        "join aborted: auth work superseded after mTLS"
    );
    // Access check + apply_room + AEAD/wire under one auth lock so Logout cannot
    // interleave a clear between the check and commit.
    {
        let mut gate = shared
            .auth
            .lock()
            .map_err(|_| anyhow::anyhow!("auth lock poisoned"))?;
        anyhow::ensure!(
            gate.access_token.as_deref() == Some(access_token),
            "join aborted: access session no longer active"
        );
        anyhow::ensure!(
            shared.auth_work_is_current(work_epoch),
            "join aborted: auth work superseded"
        );
        gate.apply_room(guest.token, local_sub.clone(), guest.expires_in);
        // Lock order: auth → intent_session → pending (same as generation commit).
        if let Ok(mut slot) = shared.intent_session.lock() {
            *slot = Some(session);
            if let Ok(mut pending) = shared.pending_intent_session.lock() {
                *pending = None;
            }
        }
        if let Ok(mut retry) = shared.pending_retry.lock() {
            *retry = None;
        }
        if let Ok(mut cap) = shared.wire_cap.lock() {
            *cap = Some(Arc::<str>::from(guest.wire_cap.as_str()));
        }
        // Fresh Join / Rejoin: commit live AEAD (clears KeyStale + pending).
        shared.commit_room_aead(guest.room_key, guest.room_key_epoch);
        if let Some(id) = mtls_ready.as_ref() {
            shared.mtls_reopen_pending.store(true, Ordering::SeqCst);
            log::info!(
                "guest mTLS ready cn={} role={} soft_ttl={}s (mtls_reopen_pending)",
                id.cn,
                id.role,
                id.expires_in
            );
        }
    }
    log::info!(
        "joined room={} sub={local_sub} expires_in={} wire_cap_len={}",
        shared.get_room_id(),
        guest.expires_in,
        guest.wire_cap.len()
    );
    Ok(())
}

/// Fetch compute-host `role=host` JWT into [`crate::auth::AuthGate::host_room_token`].
///
/// Does **not** change live guest Token / mTLS (no CSR). Used so Intent can re-bind
/// as `role=host` for non-owner compute hosts (x0081 Dual path). Owner take-out no
/// longer requires this slot (x0098). Call after a successful Host / Start / Rescue join.
pub fn install_host_inventory_token(shared: &SharedState, access_token: &str) -> Result<()> {
    let session_pub_b64 = {
        let Ok(slot) = shared.intent_session.lock() else {
            anyhow::bail!("intent_session lock poisoned");
        };
        let Some(session) = slot.as_ref() else {
            anyhow::bail!("host inventory Token requires live Intent session");
        };
        session.session_pub_b64()
    };
    let world_url = shared.get_world_url();
    let host = world_token::fetch_room_token(
        &world_url,
        shared.get_room_id().as_ref(),
        access_token,
        &session_pub_b64,
        /* csr_pem */ None,
        "host",
    )
    .context("fetch host Room Token for inventory")?;
    if crate::inventory::room_token_role_unverified(&host.token).as_deref() != Some("host") {
        anyhow::bail!("inventory host Token missing role=host");
    }
    let mut gate = shared
        .auth
        .lock()
        .map_err(|_| anyhow::anyhow!("auth lock poisoned"))?;
    anyhow::ensure!(
        gate.access_token.as_deref() == Some(access_token),
        "host inventory Token aborted: access session no longer active"
    );
    anyhow::ensure!(
        gate.room_token.is_some(),
        "host inventory Token aborted: not joined"
    );
    gate.set_host_room_token(Some(host.token));
    drop(gate);
    // Re-bind Intent with host JWT so SessionTable role=host (control plane x0081).
    if let Ok(mut slot) = shared.intent_session.lock() {
        if let Some(session) = slot.as_mut() {
            session.mark_bind_pending();
        }
    }
    log::info!(
        "installed host Room Token for inventory take-out / despawn (Intent re-bind pending)"
    );
    Ok(())
}

/// Start Zenoh room subscribers once `wire_cap` **and** `room_key` are known
/// (Worlds Join/Host via [`bind_room_after_join`]; x004e / x0069).
///
/// Idempotent via `room_bus_started`. On spawn failure the flag is cleared so retry works.
///
/// `blake3_tx`: remount worker sender (`Some` from bind). `None` still drains on a background
/// thread (tests / callers that do not remount).
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
        let room_id = shared.get_room_id();
        let manifest_key_expr = manifest_key(room_id.as_ref(), &wire_cap);
        let frame_key_expr = frame_key(room_id.as_ref(), &wire_cap);
        let input_key_expr = input_key(room_id.as_ref(), &wire_cap);
        let presence_key_expr = presence_key(room_id.as_ref(), &wire_cap);
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
        start_room_bus_with_remount(shared, session)
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
            start_room_bus_with_remount(shared, session)
        };
        if let Err(retry_err) = retry {
            close_zenoh(zenoh);
            return Err(retry_err).context("room bus retry after reopen");
        }
    }
    let session = zenoh
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("bind_room_after_join: no zenoh session"))?;
    // Join / post-reopen declare: never retain. Session close already undeclared any prior
    // wire key; retain_previous=true here only dual-declared the same jti and risked Delete.
    // Room Token refresh rebind (input publisher) is the sole retain_previous=true path.
    declare_room_alive(shared, session, false).context("liveliness after join")?;
    // Manifest / Presence need JWKS; boot soft-fail can leave the cache empty for tens of
    // seconds (Frames dropped, Presence reclaim invisible → no Intent rebind).
    if let Err(err) = shared.world_jwks.refresh() {
        log::warn!("JWKS refresh after room bind: {err:#}");
    }
    Ok(())
}

/// Sleep until the next access/room/epoch-probe deadline, capped at [`TOKEN_REFRESH_POLL`].
/// Joined+pending wakes at `pending_retry` (backoff) or soon if unset — epoch probe is
/// ignored while pending so a past-due `probe_at` cannot pin the loop at 50ms.
fn token_refresh_wait(shared: &SharedState) -> Duration {
    let now = Instant::now();
    // pending_rotate suppresses `epoch_probe_due`; keep wait on the same gate.
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
    drop(gate);
    if let Ok(slot) = shared.room_key_epoch_probe_at.lock() {
        if let Some(t) = *slot {
            if t <= now {
                return Duration::from_millis(50);
            }
            wait = wait.min(t.saturating_duration_since(now));
        }
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
                                // Release auth lock before IPC retry (timeouts can block seconds).
                                let push = if let Ok(mut gate) = shared.auth.lock() {
                                    gate.apply_auth_session(session);
                                    gate.access_token
                                        .clone()
                                        .zip(gate.access_expires_in_remaining())
                                } else {
                                    None
                                };
                                if let Some((tok, exp)) = push {
                                    shared.host_supervisor.push_access_token_retry(&tok, exp);
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
                // Keep probe out of the past while pending so wait stays on pending_retry only.
                if pending_rotate && shared.room_key_epoch_probe_due(now) {
                    shared.schedule_room_key_epoch_probe(crate::scene::ROOM_KEY_EPOCH_PROBE);
                }
                let room_due = joined
                    && shared
                        .auth
                        .lock()
                        .ok()
                        .map(|g| g.room_deadline.map(|d| now >= d).unwrap_or(false))
                        .unwrap_or(false);
                // Epoch probe must not bypass pending Intent backoff (same as soft TTL).
                let epoch_probe_due =
                    joined && !pending_rotate && shared.room_key_epoch_probe_due(now);
                // pending backoff must not be bypassed by TTL; during backoff only live refresh.
                let need_room = pending_due || room_due || epoch_probe_due;
                if need_room {
                    // Past-due probe left scheduled → token_refresh_wait sticks at 50ms.
                    let rearm_epoch_probe_if_due = || {
                        if epoch_probe_due {
                            shared.schedule_room_key_epoch_probe(TOKEN_REFRESH_POLL);
                        }
                    };
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
                        rearm_epoch_probe_if_due();
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
                            rearm_epoch_probe_if_due();
                            continue;
                        }
                    } else if let Some(b64) = shared.live_session_pub_b64_if_joined() {
                        (b64, false)
                    } else {
                        if let Ok(mut gate) = shared.auth.lock() {
                            gate.room_deadline = Some(now + TOKEN_REFRESH_POLL);
                        }
                        rearm_epoch_probe_if_due();
                        continue;
                    };
                    // Guest refresh: re-issue Room Token. New CSR only when the bus is down —
                    // mid-flight PEM rewrite is unnecessary for epoch probes and can churn
                    // liveliness; soft-TTL PEMs apply on next open / Leave→Join (x0068).
                    let bus_up = shared.room_bus_started.load(Ordering::SeqCst);
                    let (csr_pem, eph_key) = if !bus_up && zenoh_connect::should_request_world_mtls()
                    {
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
                    let refresh_host_inventory = shared
                        .auth
                        .lock()
                        .ok()
                        .is_some_and(|g| g.host_room_token.is_some());
                    match world_token::fetch_room_token(
                        &world_url,
                        shared.get_room_id().as_ref(),
                        &access,
                        &session_pub_b64,
                        csr_pem.as_deref(),
                        "guest",
                    ) {
                        Ok(guest) => {
                            // Epoch probe only: same epoch → do not rotate Room Token jti /
                            // liveliness (was reclaiming Presence every ~15s).
                            let probe_only = epoch_probe_due && !room_due && !pending_due;
                            if probe_only
                                && guest.room_key_epoch == shared.room_key_epoch()
                                && !shared.key_stale()
                            {
                                let _ = shared.note_room_aead_from_refresh(
                                    guest.room_key,
                                    guest.room_key_epoch,
                                );
                                shared.schedule_room_key_epoch_probe(
                                    crate::scene::ROOM_KEY_EPOCH_PROBE,
                                );
                                log::debug!(
                                    "room_key_epoch probe: unchanged epoch={}; skip Token rotate",
                                    guest.room_key_epoch
                                );
                                continue;
                            }
                            // Live bus: skip disk PEM rewrite (CSR already omitted when bus_up).
                            // Soft/hard TTL PEMs apply on next open / Leave→Join / mtls_reopen.
                            if !bus_up {
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
                                        shared
                                            .mtls_reopen_pending
                                            .store(true, Ordering::SeqCst);
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
                                    rearm_epoch_probe_if_due();
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
                                if refresh_host_inventory {
                                    if let Err(err) =
                                        install_host_inventory_token(&shared, &access)
                                    {
                                        log::warn!(
                                            "host inventory Token refresh failed: {err:#}"
                                        );
                                        if let Ok(mut gate) = shared.auth.lock() {
                                            gate.set_host_room_token(None);
                                        }
                                    }
                                }
                            } else {
                                log::info!(
                                    "discard Room Token fetch (session_pub no longer expected; pending={for_pending})"
                                );
                                shared.clear_pending_if_not_joined();
                                if for_pending && shared.has_pending_intent_session_joined() {
                                    shared.note_pending_token_failure();
                                } else if !for_pending {
                                    shared.force_room_token_refresh_soon();
                                    rearm_epoch_probe_if_due();
                                }
                            }
                        }
                        Err(err) => {
                            log::warn!("Room Token refresh failed: {err:#}");
                            if for_pending {
                                shared.note_pending_token_failure();
                            } else {
                                if let Ok(mut gate) = shared.auth.lock() {
                                    gate.room_deadline = Some(now + TOKEN_REFRESH_POLL);
                                }
                                rearm_epoch_probe_if_due();
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
        let room_token = shared
            .auth
            .lock()
            .ok()
            .and_then(|g| g.host_room_token.clone().or_else(|| g.room_token.clone()));
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
                let key = input_key(shared.get_room_id().as_ref(), &wire_cap);
                match session.declare_publisher(key.clone()).wait() {
                    Ok(publisher) => {
                        match seal_zenoh_payload(shared, AeadKind::Input, &event.encode_to_vec()) {
                            Ok(env) => {
                                // x0077: Leave+token also consumes bind source bucket.
                                if let Err(err) =
                                    publisher.put(env).timestamp(session.new_timestamp()).wait()
                                {
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
