//! Alchemy client — boot entry (x0029 Frame TRS + x0026 Manifest seal + x0025 attestation).
//!
//! Guarantees (boot scope):
//! - Zenoh keys from `ROOM_ID` via protocol `*_key` helpers (x0017); `ROOM_ID` validated at boot
//! - Silent / CI: `AUTH_USER` or `ALCHEMY_AUTH_TOKEN` authenticates only (no auto room join);
//!   MainHub then Join a host / Host a room
//! - Boot mesh is empty shell (no Coin / floor draw; x007d). Manifest blake3 → assets GET →
//!   GPU remount after Join/Host (`room` + `SceneGpu::replace_world_mesh`)
//! - Surface boot: OpenXR when runtime present, else Desktop; ESC / Menu dash + Exit;
//!   XR boot keeps a Desktop companion; F8 / HMD presence hotswitch (x0053)
//!
//! Per-unit guarantees live in the owning module:
//! - `room` — join / leave / liveliness / token refresh / bus start / mesh remount worker
//! - `bus::{wire, manifest, frame, presence, input, liveliness}` — Zenoh payload gates and threads
//! - `mesh::empty_shell_mesh` — MainHub boot / Leave; `assets_fetch` — authenticated GET on remount
//!
//! Non-guarantees: Steal; XR login keyboard; email verify UI; Quest standalone; WSL HMD;
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

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64};
use std::sync::{Arc, Mutex};

use alchemy_client_trust::{HostAttestationVerifier, ManifestSealVerifier, WorldJwks};
use alchemy_protocol::{validate_room_id, PROTOCOL_PIN};
use anyhow::{Context, Result};
use auth::AuthGate;
use mesh::empty_shell_mesh;
use scene::{SharedState, LOCAL_USER_ROOT_UNSET};

// Room public API for `surface/desktop.rs` (and any future surface).
use room::spawn_token_refresh;
pub use room::{bind_room_after_join, close_zenoh, join_room_with_access, leave_room};

const DEFAULT_ROOM_ID: &str = "local";

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let prefs = operator_prefs::load();
    let boot_ep = operator_prefs::boot_endpoints(&prefs);
    let auth_url = boot_ep.auth_url.clone();
    let world_url = boot_ep.world_url.clone();
    let zenoh_connect = boot_ep.zenoh_connect.clone();
    let room_id_raw = std::env::var("ROOM_ID").unwrap_or_else(|_| DEFAULT_ROOM_ID.to_string());
    let room_id = validate_room_id(&room_id_raw)
        .map_err(|e| anyhow::anyhow!("invalid ROOM_ID={room_id_raw:?}: {e}"))?
        .to_string();

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
    // Boot may not have world up yet (silent MainHub or UI onboarding); soft-fail and retry
    // on Join/Host / operator select.
    if let Err(err) = jwks.refresh() {
        log::warn!("initial world JWKS deferred ({auth_mode}): {err:#}");
    }
    jwks.spawn_background_refresh();
    let seal = Arc::new(ManifestSealVerifier::new(Arc::clone(&jwks), &room_id));
    let attest = Arc::new(HostAttestationVerifier::new(Arc::clone(&jwks), &room_id));

    let shared = Arc::new(SharedState {
        local_user_root: AtomicU32::new(LOCAL_USER_ROOT_UNSET),
        room_id: Mutex::new(Arc::<str>::from(room_id.clone())),
        wire_cap: Mutex::new(None),
        room_key: Mutex::new(None),
        room_key_epoch: AtomicU64::new(0),
        key_stale: AtomicBool::new(false),
        room_key_epoch_probe_at: Mutex::new(None),
        aead_key_refresh_nudge_at: Mutex::new(None),
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
        pending_world_mesh: Mutex::new(None),
        pending_mesh_epoch: AtomicU64::new(0),
        remounted_blake3: Mutex::new(None),
        remount_generation: AtomicU64::new(0),
        auth_work_epoch: AtomicU64::new(0),
        look: Mutex::new(Default::default()),
        intent: Mutex::new(Default::default()),
        presentation: crate::surface::PresentationGate::default(),
        dash: Mutex::new(crate::system_ui::SharedDashStore::default()),
        inventory_peer_actions: Mutex::new(Vec::new()),
        inventory_mirror: Mutex::new(crate::system_ui::InventoryMirror::default()),
        host_supervisor: Arc::new(crate::host_supervisor::HostSupervisor::default()),
    });
    // Sync AUTH/WORLD/ASSETS/ZENOH into process env for supervised host spawn.
    // Same world_url as the JWKS fetch above → apply_endpoints skips a second refresh.
    shared.apply_endpoints(&boot_ep);
    log::info!(
        "boot endpoints: auth={} world={} zenoh={}",
        boot_ep.auth_url,
        boot_ep.world_url,
        boot_ep.zenoh_connect
    );

    // Boot never joins a room. Silent / remember-me authenticate only; Worlds Join/Host
    // opens Zenoh and remounts Manifest mesh (x007d).
    log::info!(
        "client x004e starting; protocol_pin={PROTOCOL_PIN} room={room_id} auth_mode={auth_mode} wire_cap=pending"
    );

    spawn_token_refresh(Arc::clone(&shared))?;

    log::info!("zenoh open deferred until Home→Join (operator endpoints may still change)");
    log::info!("room Zenoh bus deferred until Home→Join (x004e); local GUI join starts bus");
    log::info!("boot mesh = empty shell (MainHub; Manifest remount after Join/Host)");

    let mesh = empty_shell_mesh();
    log::info!("mesh ready: empty shell");

    let kind = surface::choose();
    // Zenoh session is opened later on Worlds Join/Host (None at boot).
    surface::run(kind, mesh, shared, None)?;
    Ok(())
}
