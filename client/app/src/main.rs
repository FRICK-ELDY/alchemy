//! Alchemy client — boot entry (x0029 Frame TRS + x0026 Manifest seal + x0025 attestation).
//!
//! Guarantees (boot scope):
//! - Zenoh keys from `ROOM_ID` via protocol `*_key` helpers (x0017); `ROOM_ID` validated at boot
//! - CI fallback: `AUTH_USER` or `ALCHEMY_AUTH_TOKEN` skips UI and joins
//! - Silent join requires Manifest `blake3` before assets GET (no Coin fallback, x003c); UI boot
//!   defers Zenoh open / room bus until Home→Join
//! - Surface boot: OpenXR when runtime present, else Desktop; ESC / Menu dash + Exit;
//!   XR boot keeps a Desktop companion; F8 / HMD presence hotswitch (x0053)
//!
//! Per-unit guarantees live in the owning module:
//! - `room` — join / leave / liveliness / token refresh / bus start
//! - `bus::{wire, manifest, frame, presence, input, liveliness}` — Zenoh payload gates and threads
//! - `assets_fetch` — assets GET / mesh boot resolution
//!
//! Non-guarantees: Steal, XR login keyboard, email verify UI, Quest standalone, WSL HMD,
//! Quest/WebXR hotswitch; Virtual Desktop presence parity

mod access_level;
mod assets_fetch;
mod auth;
mod bus;
mod host_supervisor;
mod intent_session;
mod inventory;
mod jwt_sub;
mod mesh;
mod metrics;
mod operator_prefs;
mod place_read;
mod room;
mod scene;
mod surface;
mod system_ui;
mod turn;
mod world_api;
mod world_hit;
mod world_token;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use alchemy_client_trust::{HostAttestationVerifier, ManifestSealVerifier, WorldJwks};
use alchemy_protocol::{validate_room_id, PROTOCOL_PIN};
use anyhow::{Context, Result};
use assets_fetch::{load_auth_mesh, load_unauth_home_mesh, try_place_read_for_mesh, AuthMeshLoad};
use auth::AuthGate;
use bus::liveliness::declare_room_alive;
use mesh::COIN_BLAKE3_HEX;
use scene::{SharedState, LOCAL_USER_ROOT_UNSET};

// Room public API for `surface/desktop.rs` (and any future surface).
pub use room::{bind_room_after_join, close_zenoh, join_room_with_access, leave_room};
use room::{ensure_room_bus, open_zenoh_for, spawn_token_refresh};

const DEFAULT_ROOM_ID: &str = "local";
const MANIFEST_WAIT: Duration = Duration::from_millis(400);
/// Auth join: cover host presence-join Manifest put (Intent → assign/jti-rotate → put).
/// Blake3 is sent before snapshot GET; this wait is not snapshot-bound.
const MANIFEST_WAIT_AUTH: Duration = Duration::from_secs(2);

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let prefs = operator_prefs::load();
    let boot_ep = operator_prefs::boot_endpoints(&prefs);
    let auth_url = boot_ep.auth_url.clone();
    let world_url = boot_ep.world_url.clone();
    let zenoh_connect = boot_ep.zenoh_connect.clone();
    // SharedState is the source of truth; sync env only when we actually open Zenoh
    // (open_zenoh_for) or when ApplyOperatorEndpoints updates endpoints.
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
    } else if prefs.operator_chosen
        && prefs.session_still_valid(operator_prefs::now_unix_secs())
        && prefs.refresh_token.is_some()
    {
        // Restore remember-me session (≤7 days); failure falls through to UI Step2.
        if let Some(rt) = prefs.refresh_token.clone() {
            match auth::refresh(&auth_url, &rt) {
                Ok(session) => {
                    log::info!("onboarding: restored session via refresh_token");
                    let mut prefs = prefs.clone();
                    prefs.refresh_token = session.refresh_token.clone().or(Some(rt));
                    prefs.username = Some(session.user.username.clone());
                    prefs.last_login_at = Some(operator_prefs::now_unix_secs());
                    operator_prefs::save(&prefs);
                    gate.apply_auth_session(session);
                }
                Err(err) => {
                    log::info!("onboarding: refresh restore failed ({err:#}) — UI login");
                    let mut prefs = prefs.clone();
                    prefs.clear_session();
                    operator_prefs::save(&prefs);
                }
            }
        }
    }

    let client_metrics = Arc::new(metrics::ClientMetrics::default());
    metrics::maybe_spawn_from_env(Arc::clone(&client_metrics));

    let jwks = Arc::new(WorldJwks::new(&world_url));
    jwks.set_metrics(Arc::clone(&client_metrics) as Arc<dyn alchemy_client_trust::JwksMetrics>);
    // UI-first boot may not have world up yet; soft-fail and retry after operator select.
    if let Err(err) = jwks.refresh() {
        if auth_mode == "silent" {
            return Err(err).context("initial world JWKS (ManifestSeal + HostAttestation)");
        }
        log::warn!("initial world JWKS deferred (UI onboarding): {err:#}");
    }
    jwks.spawn_background_refresh();
    let seal = Arc::new(ManifestSealVerifier::new(Arc::clone(&jwks), &room_id));
    let attest = Arc::new(HostAttestationVerifier::new(Arc::clone(&jwks), &room_id));

    let shared = Arc::new(SharedState {
        local_user_root: AtomicU32::new(LOCAL_USER_ROOT_UNSET),
        room_id: Arc::<str>::from(room_id.clone()),
        wire_cap: Mutex::new(None),
        room_key: Mutex::new(None),
        room_bus_started: AtomicBool::new(false),
        mtls_reopen_pending: AtomicBool::new(false),
        world_jwks: Arc::clone(&jwks),
        manifest_seal: Arc::clone(&seal),
        host_attest: Arc::clone(&attest),
        auth_url: Mutex::new(auth_url),
        world_url: Mutex::new(world_url),
        assets_url: Mutex::new(boot_ep.assets_url.clone()),
        zenoh_connect: Mutex::new(zenoh_connect),
        auth: Mutex::new(gate),
        intent_session: Mutex::new(None),
        pending_intent_session: Mutex::new(None),
        pending_retry: Mutex::new(None),
        host_pub: Mutex::new(None),
        manifest_gen_rev: Mutex::new(None),
        last_world_seal: Mutex::new(None),
        presence_gen_rev: Mutex::new(None),
        liveliness: Mutex::new(None),
        liveliness_prev: Mutex::new(None),
        world: Mutex::new(scene::WorldView::default()),
        metrics: Arc::clone(&client_metrics),
        box_defs: Mutex::new(Arc::new(scene::BoxDefs::default())),
        mesh_entity_id: AtomicU32::new(0),
        scene_meshes: Mutex::new(Vec::new()),
        asset_status: Mutex::new(None),
        look: Mutex::new(Default::default()),
        intent: Mutex::new(Default::default()),
        presentation: crate::surface::PresentationGate::default(),
        dash: Mutex::new(crate::system_ui::SharedDashStore::default()),
        host_supervisor: Arc::new(crate::host_supervisor::HostSupervisor::default()),
    });

    // Auto-join + Zenoh open only for silent/CI. Remember-me restore authenticates only
    // and lands on MainHub; SOLO/Join then opens the right endpoint (incl. mTLS).
    if auth_mode == "silent" {
        if let Some(access) = shared.auth.lock().ok().and_then(|g| g.access_token.clone()) {
            join_room_with_access(&shared, &access).context("silent join room")?;
        }
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

    let joined_with_cap = wire_cap.is_some()
        && shared
            .auth
            .lock()
            .ok()
            .map(|g| g.is_joined())
            .unwrap_or(false);

    // UI / pre-join: defer Zenoh open until bind_room_after_join so Step1 Official/Custom
    // can retarget endpoints without leaving a localhost session glued open.
    // Silent join: allocate may have set ZENOH_TLS_* — open picks them up now.
    let session = if joined_with_cap {
        let s = open_zenoh_for(&shared).context("open zenoh session")?;
        shared.mtls_reopen_pending.store(false, Ordering::SeqCst);
        Some(s)
    } else {
        log::info!("zenoh open deferred until Home→Join (operator endpoints may still change)");
        None
    };

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
        // Remember-me may have access_token without join — still use Coin fixture until
        // room bus / Manifest blake3 exist (avoid Official assets GET on Coin hash).
        let outcome = if !joined_with_cap {
            log::info!("not joined — loading Coin fixture for Home / MainHub");
            load_unauth_home_mesh()
        } else if access_for_blob.is_empty() {
            log::info!("joined without access token — Coin fixture");
            load_unauth_home_mesh()
        } else {
            let place_read = try_place_read_for_mesh(&shared);
            load_auth_mesh(
                &shared.get_assets_url(),
                &blob_blake3,
                &access_for_blob,
                place_read.as_deref(),
            )
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
