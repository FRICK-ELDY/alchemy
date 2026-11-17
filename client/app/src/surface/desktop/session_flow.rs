//! Desktop auth / session / Worlds / host orchestration (x002e / x0080 / x00a1).
//!
//! Guarantees: Sign in / Register / Logout, Start / Join, Worlds Join · Host · Rescue ·
//! Takeover · Leave · Rejoin, the deferred room-bus bind (one painted "Binding…" frame
//! before `bind_room_after_join`), the Waiting host-session poll and the per-frame
//! Live / stall shell. HTTP runs on the auth worker (`super::job`); the UI thread only
//! applies outcomes. Leave precedes `set_room_id`; Logout / Leave bump the auth epoch
//! before touching Zenoh.
//! Non-guarantee: host claim / lease / ACL authority stays on world / auth servers; this
//! module only mirrors their results into `SystemUi`.

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

use crate::auth;
use crate::operator_prefs::{self, endpoints_for, OperatorKind};
use crate::room::install_host_inventory_token;
use crate::scene::SharedState;
use crate::system_ui::{DashTab, OnboardingShell, SystemUi};
use crate::world_api;
use crate::{bind_room_after_join, close_zenoh, join_room_with_access, leave_room};

use super::inventory_flow::{
    apply_inventory_job_outcome, shared_has_host_inventory_token, sync_room_save_ui,
};
use super::job::{
    arm_auth_job, discard_auth_job, send_enter_progress, spawn_auth_http, stop_host_if_job_current,
    AuthJobOutcome,
};
use super::support::{publish_inventory_mirror_from_ui, stop_supervised_host};

/// Success UI after a deferred [`bind_room_after_join`] (one frame of enter overlay).
enum PendingBindSuccess {
    StartGuestNotOwner {
        wanted: String,
    },
    Host {
        name: String,
        wanted: String,
    },
    Join {
        ok_status: String,
        ok_log: String,
        can_host_rescue: bool,
        enter_log_detail: String,
    },
}

pub(super) struct PendingRoomBind {
    stop_host_on_fail: bool,
    success: PendingBindSuccess,
}

/// Mut refs for deferred bind scheduling (keeps `handle_ui_action` under clippy arity).
pub(super) struct RoomBindDefer<'a> {
    pub(super) pending: &'a mut Option<PendingRoomBind>,
    pub(super) painted: &'a mut bool,
}

impl RoomBindDefer<'_> {
    pub(super) fn clear(&mut self) {
        *self.pending = None;
        *self.painted = false;
    }
}

/// Non-blocking poll of Auth worker (x002e). Safe to call every frame.
/// Drains `EnterProgress` without clearing the job; finals clear it.
pub(super) fn poll_auth_job(
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    auth_job: &mut Option<(u64, Receiver<AuthJobOutcome>)>,
    room_bind: RoomBindDefer<'_>,
) {
    let Some((epoch, _)) = auth_job.as_ref() else {
        return;
    };
    if !shared.auth_work_is_current(*epoch) {
        // Logout / Leave discarded this generation — drop without UI noise.
        *auth_job = None;
        return;
    }
    let epoch = *epoch;
    loop {
        let Some((_, rx)) = auth_job.as_ref() else {
            return;
        };
        let next = match rx.try_recv() {
            Ok(outcome) => Ok(outcome),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(()),
        };
        match next {
            Err(()) => {
                *auth_job = None;
                if ui.state.session_enter_active {
                    ui.state.end_session_enter();
                }
                ui.state.home_status = "Auth worker disconnected".into();
                ui.mark_dash_form_dirty();
                ui.rebuild();
                log::warn!("auth worker disconnected before result");
                return;
            }
            Ok(AuthJobOutcome::EnterProgress {
                progress,
                title,
                log,
            }) => {
                if !shared.auth_work_is_current(epoch) {
                    *auth_job = None;
                    return;
                }
                ui.state
                    .apply_session_enter_progress(progress, &title, &log);
                if !title.is_empty() {
                    ui.state.home_status = title.clone();
                    ui.state.worlds_status = title;
                }
                ui.mark_dash_chrome_dirty();
                ui.rebuild();
                // Keep draining progress / possible final in the same frame.
            }
            Ok(outcome) if outcome.is_inventory_outcome() => {
                if !shared.auth_work_is_current(epoch) {
                    *auth_job = None;
                    return;
                }
                let clear_job = !outcome.is_mid_flight_progress();
                apply_inventory_job_outcome(ui, shared, outcome);
                if clear_job {
                    *auth_job = None;
                    return;
                }
            }
            Ok(outcome) => {
                if !shared.auth_work_is_current(epoch) {
                    *auth_job = None;
                    return;
                }
                let refresh_main_hub = matches!(
                    &outcome,
                    AuthJobOutcome::LoginOk { .. } | AuthJobOutcome::RegisterOk { .. }
                );
                *auth_job = None;
                apply_auth_outcome(ui, shared, room_bind, outcome);
                if refresh_main_hub {
                    start_worlds_catalog_refresh(ui, shared, auth_job);
                }
                return;
            }
        }
    }
}

fn apply_auth_outcome(
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    room_bind: RoomBindDefer<'_>,
    outcome: AuthJobOutcome,
) {
    if outcome.is_mid_flight_progress() || outcome.is_inventory_outcome() {
        // EnterProgress: poll_auth_job. Inventory*: inventory_flow via poll_auth_job.
        return;
    }
    match outcome {
        AuthJobOutcome::EnterProgress { .. }
        | AuthJobOutcome::InventoryTakeOutProgress { .. }
        | AuthJobOutcome::InventoryTakeOutDone { .. } => {}
        AuthJobOutcome::LoginOk { username } => {
            persist_session_prefs(ui, shared, &username);
            ui.state.home_logged_in_as = Some(username);
            ui.state.home_password.clear();
            ui.state.home_create_open = false;
            ui.state.shell = OnboardingShell::MainHub;
            ui.state.main_hub_tab = crate::system_ui::MainHubTab::Participation;
            ui.state.home_status = "Signed in — join a public session".into();
            ui.mark_dash_logic_dirty();
            ui.rebuild();
            log::info!("auth login ok (no auto-join)");
        }
        AuthJobOutcome::RegisterOk { username } => {
            persist_session_prefs(ui, shared, &username);
            ui.state.home_logged_in_as = Some(username);
            ui.state.home_password.clear();
            ui.state.home_create_open = false;
            ui.state.shell = OnboardingShell::MainHub;
            ui.state.main_hub_tab = crate::system_ui::MainHubTab::Participation;
            ui.state.home_status = "Account created — join a public session".into();
            ui.mark_dash_logic_dirty();
            ui.rebuild();
            log::info!("auth register ok (no auto-join)");
        }
        AuthJobOutcome::LoginFailed(err) => {
            ui.state.home_status = format!("Login failed: {err}");
            ui.mark_dash_form_dirty();
            ui.rebuild();
            log::warn!("login failed: {err}");
        }
        AuthJobOutcome::RegisterFailed(err) => {
            ui.state.home_status = format!("Register failed: {err}");
            ui.mark_dash_form_dirty();
            ui.rebuild();
            log::warn!("register failed: {err}");
        }
        AuthJobOutcome::StartSession { visibility, join } => {
            apply_start_session_result(
                ui,
                shared,
                room_bind.pending,
                room_bind.painted,
                visibility,
                join,
            );
        }
        AuthJobOutcome::JoinRoom {
            result,
            enter_waiting_shell,
            can_host_rescue_on_ok,
        } => {
            apply_join_result(
                ui,
                room_bind.pending,
                room_bind.painted,
                result,
                JoinUiOpts {
                    ok_status: "Joined — waiting for host if space is still empty",
                    ok_log: "join room ok",
                    enter_waiting_shell,
                    can_host_rescue_on_ok,
                },
            );
        }
        AuthJobOutcome::WorldsList { result } => {
            // Stale completion after logout / account switch — drop payload.
            if ui.state.home_logged_in_as.is_none() {
                clear_worlds_catalog(ui);
                ui.mark_dash_logic_dirty();
                ui.rebuild();
                return;
            }
            match result {
                Ok(rooms) => {
                    ui.state.worlds_rooms = rooms
                        .into_iter()
                        .map(|r| (r.room_id, r.visibility, r.role_hint, r.owner_sub))
                        .collect();
                    if let Some(sel) = ui.state.worlds_selected.as_ref() {
                        if !ui.state.worlds_rooms.iter().any(|(id, ..)| id == sel) {
                            ui.state.worlds_selected = None;
                        }
                    }
                    ui.state.clamp_worlds_catalog_scroll();
                    ui.state.worlds_status =
                        format!("{} visible room(s)", ui.state.worlds_rooms.len());
                    ui.mark_dash_logic_dirty();
                    ui.rebuild();
                }
                Err(err) => {
                    ui.state.worlds_status = format!("Catalog failed: {err}");
                    ui.mark_dash_logic_dirty();
                    ui.rebuild();
                    log::warn!("worlds catalog failed: {err}");
                }
            }
        }
    }
}

fn persist_session_prefs(ui: &SystemUi, shared: &SharedState, username: &str) {
    let operator = ui.state.operator;
    let custom_host = ui.state.custom_host.clone();
    let remember_me = ui.state.home_remember_me;
    let refresh = shared
        .auth
        .lock()
        .ok()
        .and_then(|g| g.refresh_token.clone());
    let username = username.to_string();
    let vault = crate::refresh_vault::OsRefreshVault;
    let apply_endpoints = |prefs: &mut operator_prefs::OnboardingPrefs| {
        prefs.operator_chosen = true;
        prefs.operator = operator;
        prefs.custom_host = custom_host.clone();
        let ep = endpoints_for(prefs.operator, &prefs.custom_host);
        prefs.apply_endpoints(&ep);
    };
    if remember_me {
        match refresh.as_deref() {
            Some(rt) if !rt.is_empty() => {
                // Vault I/O outside prefs lock; disk RMW afterward.
                let _ = operator_prefs::persist_remember_me(&username, rt, &vault, apply_endpoints);
            }
            _ => {
                let key = operator_prefs::update_prefs(|prefs| {
                    apply_endpoints(prefs);
                    let key = operator_prefs::clear_remember_me_fields(prefs);
                    prefs.username = Some(username.clone());
                    key
                });
                operator_prefs::delete_refresh_vault_key(key, &vault);
            }
        }
    } else {
        let key = operator_prefs::update_prefs(|prefs| {
            apply_endpoints(prefs);
            let key = operator_prefs::clear_remember_me_fields(prefs);
            prefs.username = Some(username.clone());
            key
        });
        operator_prefs::delete_refresh_vault_key(key, &vault);
    }
}

/// Dash Sign in: `auth::login` on the auth worker (no auto-join).
pub(super) fn auth_login(
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    auth_job: &mut Option<(u64, Receiver<AuthJobOutcome>)>,
) {
    if auth_job.is_some() {
        ui.state.home_status = "Auth busy…".into();
        ui.mark_dash_form_dirty();
        ui.rebuild();
        return;
    }
    let identifier = ui.state.home_identifier.clone();
    let password = ui.state.home_password.clone();
    let remember = ui.state.home_remember_me;
    let auth_url = shared.get_auth_url();
    let shared_job = Arc::clone(shared);
    ui.state.home_status = "Signing in…".into();
    ui.mark_dash_form_dirty();
    ui.rebuild();
    let (tx, rx) = mpsc::channel();
    let epoch = arm_auth_job(shared, auth_job, rx);
    spawn_auth_http(move || {
        match auth::login(&auth_url, &identifier, &password, remember) {
            Ok(session) => {
                if !shared_job.auth_work_is_current(epoch) {
                    return;
                }
                let username = session.user.username.clone();
                let applied_access = session.access_token.clone();
                // Clone push payload under lock, then release before IPC retry (x009f).
                let push = {
                    let Ok(mut gate) = shared_job.auth.lock() else {
                        return;
                    };
                    if !shared_job.auth_work_is_current(epoch) {
                        return;
                    }
                    gate.apply_auth_session(session);
                    gate.access_token
                        .clone()
                        .zip(gate.access_expires_in_remaining())
                };
                // Leave/Logout keep supervised host — re-login must refresh child JWT.
                if let Some((tok, exp)) = push {
                    shared_job
                        .host_supervisor
                        .push_access_token_retry(&tok, exp);
                }
                if !shared_job.auth_work_is_current(epoch) {
                    if let Ok(mut gate) = shared_job.auth.lock() {
                        if gate.access_token.as_deref() == Some(applied_access.as_str()) {
                            gate.clear_session();
                        }
                    }
                    return;
                }
                let _ = tx.send(AuthJobOutcome::LoginOk { username });
            }
            Err(err) => {
                if shared_job.auth_work_is_current(epoch) {
                    let _ = tx.send(AuthJobOutcome::LoginFailed(format!("{err:#}")));
                }
            }
        }
    });
}

/// Dash Create account: `auth::register` on the auth worker (ToS gate first).
pub(super) fn auth_register(
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    auth_job: &mut Option<(u64, Receiver<AuthJobOutcome>)>,
) {
    if auth_job.is_some() {
        ui.state.home_status = "Auth busy…".into();
        ui.mark_dash_form_dirty();
        ui.rebuild();
        return;
    }
    let username = ui.state.home_username.clone();
    let email = ui.state.home_email.clone();
    let password = ui.state.home_password.clone();
    let birthday = ui.state.home_birthday.clone();
    let remember = ui.state.home_remember_me;
    let tos_agreed = ui.state.home_tos_agreed;
    if !tos_agreed {
        ui.state.home_status = "Agree to the Terms of Service to register".into();
        ui.mark_dash_form_dirty();
        ui.rebuild();
        return;
    }
    let auth_url = shared.get_auth_url();
    let shared_job = Arc::clone(shared);
    ui.state.home_status = "Creating account…".into();
    ui.mark_dash_form_dirty();
    ui.rebuild();
    let (tx, rx) = mpsc::channel();
    let epoch = arm_auth_job(shared, auth_job, rx);
    spawn_auth_http(move || {
        match auth::register(
            &auth_url, &username, &email, &password, &birthday, tos_agreed, remember,
        ) {
            Ok(session) => {
                if !shared_job.auth_work_is_current(epoch) {
                    return;
                }
                let username = session.user.username.clone();
                let applied_access = session.access_token.clone();
                // Clone push payload under lock, then release before IPC retry (x009f).
                let push = {
                    let Ok(mut gate) = shared_job.auth.lock() else {
                        return;
                    };
                    if !shared_job.auth_work_is_current(epoch) {
                        return;
                    }
                    gate.apply_auth_session(session);
                    gate.access_token
                        .clone()
                        .zip(gate.access_expires_in_remaining())
                };
                // Leave/Logout keep supervised host — register must refresh child JWT.
                if let Some((tok, exp)) = push {
                    shared_job
                        .host_supervisor
                        .push_access_token_retry(&tok, exp);
                }
                if !shared_job.auth_work_is_current(epoch) {
                    if let Ok(mut gate) = shared_job.auth.lock() {
                        if gate.access_token.as_deref() == Some(applied_access.as_str()) {
                            gate.clear_session();
                        }
                    }
                    return;
                }
                let _ = tx.send(AuthJobOutcome::RegisterOk { username });
            }
            Err(err) => {
                if shared_job.auth_work_is_current(epoch) {
                    let _ = tx.send(AuthJobOutcome::RegisterFailed(format!("{err:#}")));
                }
            }
        }
    });
}

/// Sign out: bump epoch, Leave, stop supervised host, clear session + refresh vault key.
pub(super) fn auth_logout(
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    zenoh: &mut Option<zenoh::Session>,
    auth_job: &mut Option<(u64, Receiver<AuthJobOutcome>)>,
    mut room_bind: RoomBindDefer<'_>,
) {
    let (access, refresh) = shared
        .auth
        .lock()
        .ok()
        .map(|g| (g.access_token.clone(), g.refresh_token.clone()))
        .unwrap_or((None, None));
    // Bump epoch first so in-flight join cannot commit past Logout.
    // HTTP logout is best-effort on a worker (does not gate UI).
    discard_auth_job(ui, shared, auth_job);
    room_bind.clear();
    leave_room(shared, zenoh);
    stop_supervised_host(shared);
    clear_worlds_session_shell(ui);
    clear_worlds_catalog(ui);
    if let Ok(mut gate) = shared.auth.lock() {
        gate.clear_session();
    }
    let key = operator_prefs::clear_session_rmw();
    operator_prefs::delete_refresh_vault_key(key, &crate::refresh_vault::OsRefreshVault);
    ui.state.home_logged_in_as = None;
    ui.state.home_create_open = false;
    ui.state.shell = OnboardingShell::Step1Operator;
    ui.state.home_status = "Signed out — select a server operator".into();
    ui.state.tab = DashTab::Home;
    ui.mark_dash_logic_dirty();
    ui.rebuild();
    log::info!("auth logout — Leave + local session cleared; back to operator select");
    if let Some(access) = access {
        let auth_url = shared.get_auth_url();
        spawn_auth_http(move || {
            let _ = auth::logout(&auth_url, &access, refresh.as_deref());
        });
    }
}

/// Home Create / Start: visibility ensure, local host when owner, then join (worker).
pub(super) fn start_session(
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    zenoh: &mut Option<zenoh::Session>,
    auth_job: &mut Option<(u64, Receiver<AuthJobOutcome>)>,
) {
    if !ui.state.server_operator {
        ui.state.home_status = "Enable Server Operator in Settings to start a host".into();
        ui.mark_dash_form_dirty();
        ui.rebuild();
        return;
    }
    if auth_job.is_some() {
        ui.state.home_status = "Auth busy…".into();
        ui.mark_dash_form_dirty();
        ui.rebuild();
        return;
    }
    // Create panel must not reuse an existing room (e.g. `local`) — that keeps
    // prior take-out/despawn scene. Mint once more if the field was never refreshed.
    if ui.state.home_create_open {
        let cur = ui.state.home_room_id.trim();
        if cur.is_empty() || cur.eq_ignore_ascii_case("local") {
            ui.state.home_room_id = crate::system_ui::fresh_create_room_id();
        }
    }
    let room = ui.state.home_room_id.trim().to_string();
    if room.is_empty() {
        ui.state.home_status =
            "Set room_id before Start (Settings Advanced or create panel)".into();
        ui.mark_dash_form_dirty();
        ui.rebuild();
        return;
    }
    // Auth gate before Leave (same order as Worlds Join/Host).
    let (access, access_expires_in) = shared
        .auth
        .lock()
        .ok()
        .map(|g| (g.access_token.clone(), g.access_expires_in_remaining()))
        .unwrap_or((None, None));
    let Some(access) = access else {
        ui.state.home_status = "Sign in before starting a session".into();
        ui.mark_dash_form_dirty();
        ui.rebuild();
        return;
    };
    // Contract: Leave before set_room_id.
    leave_room(shared, zenoh);
    clear_worlds_session_shell(ui);
    shared.set_room_id(Arc::<str>::from(room.as_str()));
    ui.state.home_room_id = room;
    let level = ui.state.home_access_level;
    let shared_job = Arc::clone(shared);
    begin_enter_ui(
        ui,
        &format!("Starting host + session ({})…", level.as_str()),
        "Ensuring room visibility…",
    );
    let (tx, rx) = mpsc::channel();
    let epoch = arm_auth_job(shared, auth_job, rx);
    spawn_auth_http(move || {
        if !shared_job.auth_work_is_current(epoch) {
            return;
        }
        send_enter_progress(
            &tx,
            &shared_job,
            epoch,
            0.15,
            "Ensuring room…",
            "PUT/create room visibility",
        );
        let world_url = shared_job.get_world_url();
        let visibility = world_api::ensure_room_visibility(
            &world_url,
            &shared_job.get_room_id(),
            &access,
            level,
        )
        .map_err(|e| format!("{e:#}"));
        if !shared_job.auth_work_is_current(epoch) {
            return;
        }
        // Spawn local host only when we own/create the room. Existing rooms → guest Join.
        let room_id = shared_job.get_room_id();
        let mut host_started = false;
        let join = match &visibility {
            Ok(world_api::VisibilityEnsure::Applied) => {
                send_enter_progress(
                    &tx,
                    &shared_job,
                    epoch,
                    0.40,
                    "Starting local host…",
                    "Spawning host-server (cargo run can take a while)",
                );
                match shared_job.host_supervisor.ensure_running_for_room(
                    room_id.as_ref(),
                    Some(access.as_str()),
                    access_expires_in,
                ) {
                    Ok(()) => {
                        host_started = true;
                        send_enter_progress(
                            &tx,
                            &shared_job,
                            epoch,
                            0.72,
                            "Joining room…",
                            "host-server ready — fetching Room Token",
                        );
                        if !shared_job.auth_work_is_current(epoch) {
                            // Stale: Leave keeps host; do not stop a possible re-Host.
                            Err("start aborted: signed out".into())
                        } else {
                            match join_room_with_access(&shared_job, &access, epoch) {
                                Ok(()) => {
                                    if let Err(err) =
                                        install_host_inventory_token(&shared_job, &access)
                                    {
                                        log::warn!(
                                                    "host inventory Token after StartSession host failed: {err:#}"
                                                );
                                    }
                                    Ok(())
                                }
                                Err(e) => Err(format!("{e:#}")),
                            }
                        }
                    }
                    Err(err) => Err(format!("local host-server start failed: {err:#}")),
                }
            }
            Ok(world_api::VisibilityEnsure::LeftExisting) => {
                send_enter_progress(
                    &tx,
                    &shared_job,
                    epoch,
                    0.55,
                    "Joining existing room…",
                    "Not owner — guest Join path",
                );
                if !shared_job.auth_work_is_current(epoch) {
                    Err("join aborted: signed out".into())
                } else {
                    join_room_with_access(&shared_job, &access, epoch).map_err(|e| format!("{e:#}"))
                }
            }
            Err(_) => Err("skipped join (visibility step failed)".into()),
        };
        if join.is_err() && host_started {
            stop_host_if_job_current(&shared_job, epoch);
        }
        if join.is_ok() {
            send_enter_progress(
                &tx,
                &shared_job,
                epoch,
                0.85,
                "Binding session…",
                "Room Token OK — opening Zenoh bus",
            );
        }
        if shared_job.auth_work_is_current(epoch) {
            let _ = tx.send(AuthJobOutcome::StartSession { visibility, join });
        }
        // Stale: do not leave_room / stop host — same access Leave→re-Host must stay intact.
    });
}

/// Home / Settings Advanced Join by `room_id` (guest Room Token on the worker).
pub(super) fn join_room(
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    zenoh: &mut Option<zenoh::Session>,
    auth_job: &mut Option<(u64, Receiver<AuthJobOutcome>)>,
) {
    if auth_job.is_some() {
        ui.state.home_status = "Auth busy…".into();
        ui.mark_dash_form_dirty();
        ui.rebuild();
        return;
    }
    let room = ui.state.home_room_id.trim().to_string();
    if room.is_empty() {
        ui.state.home_status = "Set room_id before Join (Settings Advanced)".into();
        ui.mark_dash_form_dirty();
        ui.rebuild();
        return;
    }
    // Auth gate before Leave (same order as Worlds Join/Host).
    let access = shared.auth.lock().ok().and_then(|g| g.access_token.clone());
    let Some(access) = access else {
        ui.state.home_status = "Sign in before joining".into();
        ui.mark_dash_form_dirty();
        ui.rebuild();
        return;
    };
    // Owner Rescue CTA only if catalog already marked this room as ours.
    let catalog_role = ui
        .state
        .worlds_rooms
        .iter()
        .find(|(id, _, _, _)| id == &room)
        .map(|(_, _, role, _)| role.as_str());
    let can_host_rescue_on_ok = catalog_role == Some("owner");
    // Tip only when ownership is unknown (room missing from loaded catalog).
    let ownership_unknown = catalog_role.is_none();
    // Contract: Leave before set_room_id.
    leave_room(shared, zenoh);
    clear_worlds_session_shell(ui);
    shared.set_room_id(Arc::<str>::from(room.as_str()));
    ui.state.home_room_id = room;
    let shared_job = Arc::clone(shared);
    let tip = if ownership_unknown {
        "Joining… (Refresh catalog if you need Host Rescue as owner)"
    } else {
        "Fetching guest Room Token…"
    };
    begin_enter_ui(ui, "Joining room…", tip);
    let (tx, rx) = mpsc::channel();
    let epoch = arm_auth_job(shared, auth_job, rx);
    spawn_auth_http(move || {
        if !shared_job.auth_work_is_current(epoch) {
            return;
        }
        send_enter_progress(
            &tx,
            &shared_job,
            epoch,
            0.35,
            "Joining room…",
            "Requesting guest Room Token + mTLS",
        );
        let result =
            join_room_with_access(&shared_job, &access, epoch).map_err(|e| format!("{e:#}"));
        if result.is_ok() {
            send_enter_progress(
                &tx,
                &shared_job,
                epoch,
                0.80,
                "Binding session…",
                "Ticket OK — opening Zenoh bus",
            );
        }
        if shared_job.auth_work_is_current(epoch) {
            // Align with Worlds Join: Waiting/Live shell after ticket OK.
            let _ = tx.send(AuthJobOutcome::JoinRoom {
                result,
                enter_waiting_shell: true,
                can_host_rescue_on_ok,
            });
        }
    });
}

/// Worlds catalog Join as guest for the selected room.
pub(super) fn worlds_join_guest(
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    zenoh: &mut Option<zenoh::Session>,
    auth_job: &mut Option<(u64, Receiver<AuthJobOutcome>)>,
) {
    if auth_job.is_some() {
        ui.state.worlds_status = "Auth busy…".into();
        ui.rebuild();
        return;
    }
    let Some(room_id) = ui.state.worlds_selected.clone() else {
        ui.state.worlds_status = "Select a room first".into();
        ui.rebuild();
        return;
    };
    let Some((_, _, role_hint, _)) = ui
        .state
        .worlds_rooms
        .iter()
        .find(|(id, ..)| id == &room_id)
        .cloned()
    else {
        ui.state.worlds_status = "Select a room first".into();
        ui.rebuild();
        return;
    };
    let access = shared.auth.lock().ok().and_then(|g| g.access_token.clone());
    let Some(access) = access else {
        ui.state.worlds_status = "Sign in before joining".into();
        ui.rebuild();
        return;
    };
    let can_host_rescue_on_ok = role_hint == "owner";
    leave_room(shared, zenoh);
    shared.set_room_id(Arc::<str>::from(room_id.as_str()));
    ui.state.home_room_id = room_id;
    // Only set can_host_rescue after successful join (see AuthJobOutcome::JoinRoom).
    clear_worlds_session_shell(ui);
    let shared_job = Arc::clone(shared);
    begin_enter_ui(ui, "Joining as guest…", "Fetching guest Room Token…");
    let (tx, rx) = mpsc::channel();
    let epoch = arm_auth_job(shared, auth_job, rx);
    spawn_auth_http(move || {
        if !shared_job.auth_work_is_current(epoch) {
            return;
        }
        send_enter_progress(
            &tx,
            &shared_job,
            epoch,
            0.35,
            "Joining as guest…",
            "Requesting guest Room Token + mTLS",
        );
        let result =
            join_room_with_access(&shared_job, &access, epoch).map_err(|e| format!("{e:#}"));
        if result.is_ok() {
            send_enter_progress(
                &tx,
                &shared_job,
                epoch,
                0.80,
                "Binding session…",
                "Ticket OK — opening Zenoh bus",
            );
        }
        if shared_job.auth_work_is_current(epoch) {
            let _ = tx.send(AuthJobOutcome::JoinRoom {
                result,
                enter_waiting_shell: true,
                can_host_rescue_on_ok,
            });
        }
    });
}

/// Worlds Host for an owned room (Server Operator only).
pub(super) fn worlds_host_room(
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    zenoh: &mut Option<zenoh::Session>,
    auth_job: &mut Option<(u64, Receiver<AuthJobOutcome>)>,
) {
    if !ui.state.server_operator {
        ui.state.worlds_status = "Enable Server Operator in Settings to host".into();
        ui.rebuild();
        return;
    }
    let room_id = match ui
        .state
        .worlds_selected
        .as_ref()
        .and_then(|sel| ui.state.worlds_rooms.iter().find(|(id, ..)| id == sel))
    {
        Some((id, _, role, _)) if role == "owner" => id.clone(),
        Some(_) => {
            ui.state.worlds_status = "Only the owner can host this room".into();
            ui.rebuild();
            return;
        }
        None => {
            ui.state.worlds_status = "Select a room to host".into();
            ui.rebuild();
            return;
        }
    };
    start_worlds_host(ui, shared, zenoh, auth_job, room_id, HostStartKind::Owner);
}

/// Host Rescue for the joined room (owner, or nominated handover candidate).
pub(super) fn worlds_host_rescue(
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    zenoh: &mut Option<zenoh::Session>,
    auth_job: &mut Option<(u64, Receiver<AuthJobOutcome>)>,
) {
    // Ignore catalog selection — always the actually joined room.
    if !ui.state.server_operator {
        ui.state.worlds_status = "Enable Server Operator in Settings to host this room".into();
        ui.rebuild();
        return;
    }
    let kind = if ui.state.worlds_can_host_rescue {
        HostStartKind::Owner
    } else if ui.state.worlds_nominated_host {
        HostStartKind::Nominated
    } else {
        ui.state.worlds_status = "Not allowed to host this room".into();
        ui.rebuild();
        return;
    };
    let room_id = shared.get_room_id().to_string();
    if room_id.is_empty() {
        ui.state.worlds_status = "Not in a room to host".into();
        ui.rebuild();
        return;
    }
    start_worlds_host(ui, shared, zenoh, auth_job, room_id, kind);
}

/// Take over a dormant room's compute host (x0080).
pub(super) fn worlds_takeover(
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    zenoh: &mut Option<zenoh::Session>,
    auth_job: &mut Option<(u64, Receiver<AuthJobOutcome>)>,
) {
    if !ui.state.server_operator {
        ui.state.worlds_status = "Enable Server Operator in Settings to take over host".into();
        ui.rebuild();
        return;
    }
    if !ui.state.worlds_can_takeover {
        ui.state.worlds_status = "Takeover unavailable (session still active)".into();
        ui.rebuild();
        return;
    }
    let room_id = shared.get_room_id().to_string();
    if room_id.is_empty() {
        ui.state.worlds_status = "Not in a room to take over".into();
        ui.rebuild();
        return;
    }
    start_worlds_host(
        ui,
        shared,
        zenoh,
        auth_job,
        room_id,
        HostStartKind::Takeover,
    );
}

/// Leave the room (handover first when this client runs the local host).
pub(super) fn worlds_leave(
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    zenoh: &mut Option<zenoh::Session>,
    auth_job: &mut Option<(u64, Receiver<AuthJobOutcome>)>,
    mut room_bind: RoomBindDefer<'_>,
) {
    // Invalidate in-flight Join/Host so Leave status is not overwritten by Auth busy.
    // Do not auto-refresh catalog — that would occupy auth_job and block immediate re-Join.
    let aborting_enter = ui.state.session_enter_active && !ui.state.worlds_in_room;
    let local_host = shared.host_supervisor.owns_local_host();
    discard_auth_job(ui, shared, auth_job);
    room_bind.clear();
    // x0080: local compute host Leave must handover (or dormant) then stop host.
    if local_host && !aborting_enter {
        maybe_handover_before_leave(shared);
    }
    leave_room(shared, zenoh);
    // Abort mid-Create/Join: stop in-flight local host.
    // Live Leave while local host: stop after handover (no longer keep host for re-Host).
    if aborting_enter || local_host {
        stop_supervised_host(shared);
    }
    clear_worlds_session_shell(ui);
    ui.state.worlds_status = if aborting_enter {
        "Enter cancelled — Refresh for catalog".into()
    } else if local_host {
        "Left as host — session handed over or dormant".into()
    } else {
        "Left room — Refresh for catalog".into()
    };
    ui.state.home_status = if aborting_enter {
        "Enter cancelled — back to Main hub".into()
    } else {
        "Left session — back to Main hub".into()
    };
    ui.state.tab = DashTab::Home;
    ui.state.shell = OnboardingShell::MainHub;
    ui.state.main_hub_tab = crate::system_ui::MainHubTab::Participation;
    ui.mark_dash_chrome_dirty();
    ui.rebuild();
}

/// KeyStale Rejoin: Leave + synchronous join, then deferred room bus bind.
pub(super) fn worlds_rejoin(
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    zenoh: &mut Option<zenoh::Session>,
    auth_job: &mut Option<(u64, Receiver<AuthJobOutcome>)>,
    mut room_bind: RoomBindDefer<'_>,
) {
    let room_id = shared.get_room_id().to_string();
    let (access, restore_host_inventory) = {
        let Ok(auth) = shared.auth.lock() else {
            ui.state.worlds_status = "Rejoin failed: auth lock".into();
            ui.rebuild();
            return;
        };
        (auth.access_token.clone(), auth.host_room_token.is_some())
    };
    discard_auth_job(ui, shared, auth_job);
    room_bind.clear();
    leave_room(shared, zenoh);
    clear_worlds_session_shell(ui);
    let Some(access) = access else {
        ui.state.worlds_status = "Rejoin failed: not signed in".into();
        ui.state.tab = DashTab::Home;
        ui.state.shell = OnboardingShell::MainHub;
        ui.state.main_hub_tab = crate::system_ui::MainHubTab::Participation;
        ui.rebuild();
        return;
    };
    if room_id.is_empty() {
        ui.state.worlds_status = "Rejoin failed: no room".into();
        ui.state.tab = DashTab::Home;
        ui.state.shell = OnboardingShell::MainHub;
        ui.state.main_hub_tab = crate::system_ui::MainHubTab::Participation;
        ui.rebuild();
        return;
    }
    shared.set_room_id(std::sync::Arc::<str>::from(room_id));
    let epoch = shared.auth_work_epoch();
    let join = match join_room_with_access(shared, &access, epoch) {
        Ok(()) => {
            if restore_host_inventory {
                if let Err(err) = install_host_inventory_token(shared, &access) {
                    log::warn!("host inventory Token after Rejoin failed: {err:#}");
                }
            }
            Ok(())
        }
        Err(e) => Err(format!("{e:#}")),
    };
    apply_join_result(
        ui,
        room_bind.pending,
        room_bind.painted,
        join,
        JoinUiOpts {
            ok_status: "Rejoined — waiting for host frames",
            ok_log: "KeyStale rejoin ok",
            enter_waiting_shell: true,
            can_host_rescue_on_ok: false,
        },
    );
    ui.state.worlds_key_stale = false;
    ui.mark_dash_chrome_dirty();
    ui.rebuild();
}

/// Per-frame Worlds session UI (RedrawRequested): KeyStale, enter-wait details, Room Save
/// gate, Live / stall edges, Waiting host-session poll and nominated auto-host.
pub(super) fn tick_worlds_session_ui(
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    zenoh: &mut Option<zenoh::Session>,
    auth_job: &mut Option<(u64, Receiver<AuthJobOutcome>)>,
    waiting_session_job: &mut Option<Receiver<WaitingSessionSnapshot>>,
) {
    let tick = shared.world.lock().ok().and_then(|w| w.last_frame_tick);
    let now = std::time::Instant::now();
    let mut dirty = false;
    // x00d5: pending grace → hard KeyStale even when no wire sample arrives.
    let _ = shared.poll_pending_room_aead_grace();
    let stale = shared.key_stale();
    if ui.state.worlds_key_stale != stale {
        ui.state.worlds_key_stale = stale;
        if stale {
            ui.state.worlds_status = "Room updated — reconnect required".into();
            ui.state.tab = DashTab::Home;
            ui.state.shell = OnboardingShell::Dash;
            ui.state.open = true;
        }
        dirty = true;
    }
    if poll_session_enter_wait_details(ui, shared) {
        dirty = true;
    }
    if sync_room_save_ui(ui, shared) {
        dirty = true;
    }
    if let Some(t) = tick {
        if ui.state.worlds_seen_frame_tick != Some(t) {
            ui.state.worlds_seen_frame_tick = Some(t);
            ui.state.worlds_last_frame_at = Some(now);
            if ui.state.worlds_waiting_host || ui.state.session_enter_active {
                ui.state.worlds_waiting_host = false;
                ui.state.worlds_host_disconnect = false;
                ui.state.worlds_status = "Live — host frames arriving".into();
                ui.state.home_status = "Live — host frames arriving".into();
                if ui.state.session_enter_active {
                    let waited = ui
                        .state
                        .session_enter_wait_since
                        .map(|t0| now.duration_since(t0).as_secs())
                        .unwrap_or(0);
                    ui.state.set_session_enter_progress(1.0);
                    ui.state.push_session_enter_log(format!(
                        "Live — first Frame tick={t} after {waited}s"
                    ));
                    ui.state.end_session_enter();
                    ui.state.open = false;
                }
                dirty = true;
            }
        }
    }
    // Live stall / recovery: host skips empty Frame deltas without advancing
    // net_tick (x0024). Presence/Manifest keepalive must count as alive.
    const FRAME_STALL: std::time::Duration = std::time::Duration::from_secs(10);
    if ui.state.worlds_waiting_host
        && ui.state.worlds_host_disconnect
        && shared
            .last_host_alive_at()
            .is_some_and(|at| now.duration_since(at) < FRAME_STALL)
    {
        ui.state.worlds_waiting_host = false;
        ui.state.worlds_host_disconnect = false;
        ui.state.worlds_status = "Live — host signals arriving".into();
        ui.state.home_status = "Live — host signals arriving".into();
        dirty = true;
    }
    if ui.state.worlds_in_room
        && !ui.state.worlds_waiting_host
        && ui.state.worlds_seen_frame_tick.is_some()
        && shared
            .last_host_alive_at()
            .is_some_and(|at| now.duration_since(at) > FRAME_STALL)
    {
        // One-shot recovery: Presence may be invisible if RX stalled; still
        // re-bind Intent + liveliness so a transient liveliness Delete can heal.
        shared.request_intent_rebind();
        ui.state.worlds_waiting_host = true;
        ui.state.worlds_host_disconnect = true;
        ui.state.worlds_status = "Host connection lost — wait for return or Leave".into();
        ui.state.home_status = "Host connection lost — wait for return or Leave".into();
        // Inventory is Live-only (x007f): stall → Waiting opens Room so
        // Leave / host CTA stay reachable (x00b6). Settings may stay.
        if matches!(
            ui.state.tab,
            DashTab::Inventory | DashTab::Home | DashTab::Worlds | DashTab::Room
        ) {
            ui.state.tab = DashTab::Room;
            ui.state.room_sub_tab = crate::system_ui::RoomSubTab::Settings;
        }
        ui.state.shell = OnboardingShell::Dash;
        ui.state.open = true;
        dirty = true;
    }
    kick_waiting_host_session_poll(ui, shared, waiting_session_job, now);
    let (session_dirty, auto_nominated) =
        apply_waiting_host_session_poll(ui, shared, waiting_session_job);
    if session_dirty {
        dirty = true;
    }
    if auto_nominated {
        let room_id = shared.get_room_id().to_string();
        start_worlds_host(
            ui,
            shared,
            zenoh,
            auth_job,
            room_id,
            HostStartKind::Nominated,
        );
        dirty = true;
    }
    if dirty {
        ui.mark_dash_logic_dirty();
        ui.rebuild();
    }
}

/// Show enter modal on the dash (Create / Join / Host until first frame).
fn begin_enter_ui(ui: &mut SystemUi, title: &str, first_log: &str) {
    ui.state.begin_session_enter(title);
    if !first_log.is_empty() {
        ui.state.push_session_enter_log(first_log);
    }
    ui.state.home_status = title.into();
    ui.state.worlds_status = title.into();
    ui.state.open = true;
    ui.state.tab = DashTab::Home;
    ui.state.shell = OnboardingShell::Dash;
    ui.mark_dash_chrome_dirty();
    ui.rebuild();
}

/// While enter overlay waits for first Frame: Manifest detection + elapsed heartbeats.
/// Returns true when UI was updated (caller should rebuild).
fn poll_session_enter_wait_details(ui: &mut SystemUi, shared: &SharedState) -> bool {
    if !ui.state.session_enter_active || !ui.state.worlds_in_room {
        return false;
    }
    let now = std::time::Instant::now();
    let mut dirty = false;

    if ui.state.session_enter_wait_since.is_none() {
        ui.state.session_enter_wait_since = Some(now);
    }

    // Manifest accepted before first Frame (host seal/publish often dominates wait).
    if !ui.state.session_enter_seen_manifest {
        if let Ok(guard) = shared.manifest_gen_rev.lock() {
            if let Some(gr) = *guard {
                ui.state.session_enter_seen_manifest = true;
                ui.state.apply_session_enter_progress(
                    0.96,
                    "Manifest OK — waiting for Frame…",
                    &format!(
                        "Manifest accepted gen={} rev={} — waiting for first Frame",
                        gr.host_generation, gr.revision
                    ),
                );
                ui.state.session_enter_last_detail_at = Some(now);
                ui.state.home_status = "Manifest received — waiting for Frame…".into();
                ui.state.worlds_status = ui.state.home_status.clone();
                dirty = true;
            }
        }
    }

    const HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(2);
    let due = ui
        .state
        .session_enter_last_detail_at
        .map(|t| now.duration_since(t) >= HEARTBEAT)
        .unwrap_or(true);
    if due {
        ui.state.session_enter_last_detail_at = Some(now);
        let elapsed = ui
            .state
            .session_enter_wait_since
            .map(|t| now.duration_since(t).as_secs())
            .unwrap_or(0);
        let bus = shared
            .room_bus_started
            .load(std::sync::atomic::Ordering::SeqCst);
        let host_key = shared.host_pub.lock().ok().is_some_and(|g| g.is_some());
        let phase = if ui.state.session_enter_seen_manifest {
            "host Frame not yet applied (tick)"
        } else if host_key {
            "host_pub set; Manifest watermark pending"
        } else if bus {
            "subscribed — waiting Manifest (host seal / Zenoh put)"
        } else {
            "room bus not started"
        };
        ui.state
            .push_session_enter_log(format!("{elapsed}s — {phase}"));
        if !ui.state.session_enter_seen_manifest {
            ui.state.set_session_enter_progress(0.92);
            ui.state.session_enter_title = "Waiting for Manifest…".into();
        } else {
            ui.state.set_session_enter_progress(0.97);
        }
        dirty = true;
    }
    dirty
}

/// Clear Worlds in-room / Waiting / stall / enter state (Leave, Logout, failed join/host).
pub(super) fn clear_worlds_session_shell(ui: &mut SystemUi) {
    ui.state.worlds_waiting_host = false;
    ui.state.worlds_host_disconnect = false;
    ui.state.worlds_can_host_rescue = false;
    ui.state.worlds_can_takeover = false;
    ui.state.worlds_nominated_host = false;
    ui.state.worlds_session_poll_at = None;
    ui.state.worlds_seen_frame_tick = None;
    ui.state.worlds_last_frame_at = None;
    ui.state.worlds_in_room = false;
    ui.state.worlds_key_stale = false;
    ui.state.room_save_enabled = false;
    ui.state.room_save_status.clear();
    ui.state.end_session_enter();
    // Drop peer bit so Leave/Logout can push in_session=false (XR Inventory unblocks).
    ui.state.in_session_peer = false;
    // Room tab is in-session only (x00b6).
    if ui.state.tab == DashTab::Room {
        ui.state.tab = DashTab::Home;
        ui.state.room_sub_tab = crate::system_ui::RoomSubTab::Settings;
    }
    ui.mark_dash_chrome_dirty();
}

/// Drop catalog + selection (logout / XR leave). Does not touch session shell flags.
pub(super) fn clear_worlds_catalog(ui: &mut SystemUi) {
    ui.state.worlds_rooms.clear();
    ui.state.worlds_selected = None;
    ui.state.worlds_catalog_scroll = 0;
    ui.state.worlds_catalog_scroll_accum = 0.0;
    ui.state.worlds_status.clear();
}

/// Failed Start / guest path: cancel in-flight spawn; allow a later ensure retry.
/// External / CI hosts are never touched.
/// Result of one background Waiting host-session GET (x0080).
pub(super) struct WaitingSessionSnapshot {
    nominated: bool,
    can_takeover: bool,
    /// Server Operator ON + nominated + not already hosting.
    auto_start: bool,
}

/// Kick a background GET when Waiting; apply via [`apply_waiting_host_session_poll`].
fn kick_waiting_host_session_poll(
    ui: &mut SystemUi,
    shared: &SharedState,
    job: &mut Option<Receiver<WaitingSessionSnapshot>>,
    now: std::time::Instant,
) {
    if !ui.state.worlds_in_room || !ui.state.worlds_waiting_host {
        *job = None;
        return;
    }
    if job.is_some() {
        return;
    }
    if shared.host_supervisor.owns_local_host() {
        return;
    }
    const PERIOD: std::time::Duration = std::time::Duration::from_secs(3);
    if ui
        .state
        .worlds_session_poll_at
        .is_some_and(|t| now.duration_since(t) < PERIOD)
    {
        return;
    }
    let access = shared.auth.lock().ok().and_then(|g| g.access_token.clone());
    let local_sub = shared.auth.lock().ok().and_then(|g| g.local_sub.clone());
    let (Some(access), Some(local_sub)) = (access, local_sub) else {
        return;
    };
    let room_id = shared.get_room_id();
    if room_id.is_empty() {
        return;
    }
    ui.state.worlds_session_poll_at = Some(now);
    let world_url = shared.get_world_url();
    let server_operator = ui.state.server_operator;
    let session_enter_active = ui.state.session_enter_active;
    let (tx, rx) = mpsc::channel();
    *job = Some(rx);
    std::thread::spawn(move || {
        let Ok(summary) = world_api::get_room(&world_url, &access, room_id.as_ref()) else {
            return;
        };
        // x00a1: nominated = candidate waiting to accept (dormant), not yet active_host.
        // Takeover CTA only when dormant with **no** pending candidate (else server 409).
        let nominated = summary
            .handover_candidate_sub
            .as_deref()
            .is_some_and(|s| s == local_sub)
            && summary.state == "dormant";
        let can_takeover = summary.state == "dormant" && summary.handover_candidate_sub.is_none();
        let auto_start = nominated && server_operator && !session_enter_active;
        let _ = tx.send(WaitingSessionSnapshot {
            nominated,
            can_takeover,
            auto_start,
        });
    });
}

/// Apply a finished Waiting poll. Returns `(ui_dirty, should_auto_start_nominated_host)`.
fn apply_waiting_host_session_poll(
    ui: &mut SystemUi,
    shared: &SharedState,
    job: &mut Option<Receiver<WaitingSessionSnapshot>>,
) -> (bool, bool) {
    let Some(rx) = job.as_ref() else {
        return (false, false);
    };
    match rx.try_recv() {
        Ok(snap) => {
            *job = None;
            if shared.host_supervisor.owns_local_host()
                || !ui.state.worlds_in_room
                || !ui.state.worlds_waiting_host
            {
                return (false, false);
            }
            let mut dirty = false;
            if ui.state.worlds_nominated_host != snap.nominated {
                ui.state.worlds_nominated_host = snap.nominated;
                dirty = true;
            }
            if ui.state.worlds_can_takeover != snap.can_takeover {
                ui.state.worlds_can_takeover = snap.can_takeover;
                dirty = true;
            }
            let auto_start =
                snap.auto_start && snap.nominated && !shared.host_supervisor.owns_local_host();
            if auto_start {
                ui.state.worlds_status = "Nominated as host — starting local host…".into();
                dirty = true;
            }
            (dirty, auto_start)
        }
        Err(TryRecvError::Empty) => (false, false),
        Err(TryRecvError::Disconnected) => {
            *job = None;
            (false, false)
        }
    }
}

/// x0080 / x00a1: nominate a Presence peer as **candidate** (no lease) or dormant.
/// Candidate must `accept_handover` before becoming active_host.
/// Runs ureq off this thread but waits up to 3s so stop-host does not race the CAS.
pub(super) fn maybe_handover_before_leave(shared: &SharedState) {
    let access = shared.auth.lock().ok().and_then(|g| g.access_token.clone());
    let Some(access) = access else {
        log::warn!("handover skipped: no access token");
        return;
    };
    let room_id = shared.get_room_id();
    if room_id.is_empty() {
        return;
    }
    // Still may propose a peer as candidate; lease is deferred until accept (x00a1).
    let target = shared.first_presence_peer_sub();
    let world_url = shared.get_world_url();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = world_api::handover(&world_url, &access, room_id.as_ref(), target.as_deref());
        let _ = tx.send(result);
    });
    match rx.recv_timeout(std::time::Duration::from_secs(3)) {
        Ok(Ok(summary)) => {
            log::info!(
                "handover ok room={} state={} active_host={:?}",
                summary.room_id,
                summary.state,
                summary.active_host_sub
            );
        }
        Ok(Err(err)) => log::warn!("handover before Leave failed: {err:#}"),
        Err(_) => log::warn!("handover before Leave timed out (3s)"),
    }
}

/// x0080: world 409 host claim/lease race — keep access JWT; stop local host / stay Waiting.
fn is_host_authority_conflict(err: &str) -> bool {
    err.contains("host_claim_conflict")
        || err.contains("host_lease_conflict")
        || err.contains("takeover_conflict")
}

fn apply_host_authority_conflict(ui: &mut SystemUi, shared: &SharedState, err: &str) {
    stop_supervised_host(shared);
    ui.state.worlds_nominated_host = false;
    // Stay in-room Waiting when already joined; otherwise clear enter shell only.
    // Do not invent Takeover CTA — pending candidate still yields server 409 until poll.
    if ui.state.worlds_in_room {
        ui.state.worlds_waiting_host = true;
        ui.state.worlds_can_takeover = false;
        ui.state.worlds_can_host_rescue = false;
        ui.state.session_enter_active = false;
        ui.state.home_status =
            "Host claim lost (409) — waiting; Takeover when dormant (no re-login)".into();
        ui.state.worlds_status = ui.state.home_status.clone();
    } else {
        clear_worlds_session_shell(ui);
        ui.state.home_status =
            "Host claim lost (409) — another host holds the room (no re-login)".into();
        ui.state.worlds_status = ui.state.home_status.clone();
    }
    log::warn!("host authority conflict (session kept): {err}");
}

/// Client Exit: sticky exit latch + stop (Start HTTP 中の Exit でも後続 ensure を拒否).
pub(super) fn exit_stop_supervised_host(shared: &SharedState) {
    if shared.host_supervisor.owns_local_host() {
        maybe_handover_before_leave(shared);
    }
    shared.host_supervisor.latch_exit_and_shutdown();
}

fn schedule_binding_room_bus(
    ui: &mut SystemUi,
    pending: &mut Option<PendingRoomBind>,
    painted: &mut bool,
    bind: PendingRoomBind,
) {
    ui.state.apply_session_enter_progress(
        0.86,
        "Binding room bus…",
        "Opening Zenoh (mTLS reopen / liveliness / subscribers)",
    );
    ui.state.open = true;
    ui.rebuild();
    *pending = Some(bind);
    // Always wait one successful paint for this pending (Rejoin must not inherit prior true).
    *painted = false;
}

pub(super) fn complete_pending_room_bind(
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    zenoh: &mut Option<zenoh::Session>,
    pending: PendingRoomBind,
) {
    match bind_room_after_join(shared, zenoh) {
        Ok(()) => match pending.success {
            PendingBindSuccess::StartGuestNotOwner { wanted } => {
                ui.state.home_create_open = false;
                ui.state.home_status =
                    format!("Joined as guest (not owner) — waiting for host (wanted {wanted})");
                ui.state.worlds_status = ui.state.home_status.clone();
                ui.state.worlds_waiting_host = true;
                ui.state.worlds_host_disconnect = false;
                ui.state.worlds_can_host_rescue = false;
                ui.state.worlds_can_takeover = false;
                ui.state.worlds_nominated_host = false;
                ui.state.worlds_seen_frame_tick = None;
                ui.state.worlds_last_frame_at = None;
                ui.state.worlds_in_room = true;
                ui.state.mark_session_enter_bus_bound();
                ui.state.push_session_enter_log(
                    "Joined existing room as guest — waiting Manifest then Frame",
                );
                ui.state.open = true;
                ui.state.tab = DashTab::Home;
                ui.state.shell = OnboardingShell::Dash;
                log::info!("start session joined existing as guest (wanted {wanted})");
            }
            PendingBindSuccess::Host { name, wanted } => {
                ui.state.home_create_open = false;
                let mut status = if name.is_empty() {
                    format!("Hosting — waiting for first frame ({wanted})")
                } else {
                    format!("Hosting: {name} — waiting for first frame ({wanted})")
                };
                // Soft-fail Host/Start only: install_host_inventory_token failed after join.
                // Owner take-out still works via guest+session / owner_sub (x0098); host Token
                // is only needed so a non-owner compute host can re-bind as role=host.
                if !shared_has_host_inventory_token(shared) {
                    status.push_str("; host Token missing (role=host Intent upgrade)");
                    ui.state.inventory_status = "host Token missing — owner take-out OK (guest session); non-owner host edit needs re-Host".into();
                    publish_inventory_mirror_from_ui(ui, shared);
                }
                ui.state.home_status = status.clone();
                ui.state.worlds_status = status;
                ui.state.worlds_waiting_host = true;
                ui.state.worlds_host_disconnect = false;
                ui.state.worlds_can_host_rescue = true;
                ui.state.worlds_seen_frame_tick = None;
                ui.state.worlds_last_frame_at = None;
                ui.state.worlds_in_room = true;
                ui.state.mark_session_enter_bus_bound();
                ui.state.push_session_enter_log(
                    "Local host joined — next: Manifest seal/publish, then Frame",
                );
                ui.state.open = true;
                ui.state.tab = DashTab::Home;
                ui.state.shell = OnboardingShell::Dash;
                log::info!("start/host session ok; local host up; wanted={wanted}");
            }
            PendingBindSuccess::Join {
                ok_status,
                ok_log,
                can_host_rescue,
                enter_log_detail,
            } => {
                ui.state.home_create_open = false;
                ui.state.home_status = ok_status.clone();
                ui.state.worlds_status = ok_status;
                ui.state.worlds_host_disconnect = false;
                ui.state.worlds_seen_frame_tick = None;
                ui.state.worlds_last_frame_at = None;
                ui.state.worlds_can_host_rescue = can_host_rescue;
                ui.state.worlds_in_room = true;
                ui.state.home_room_id = shared.get_room_id().to_string();
                ui.state.worlds_waiting_host = true;
                ui.state.tab = DashTab::Home;
                ui.state.shell = OnboardingShell::Dash;
                ui.state.open = true;
                ui.state.mark_session_enter_bus_bound();
                ui.state.push_session_enter_log(enter_log_detail);
                log::info!("{ok_log}");
            }
        },
        Err(err) => {
            if pending.stop_host_on_fail {
                stop_supervised_host(shared);
            }
            clear_worlds_session_shell(ui);
            ui.state.home_status = format!("Joined but room bus failed: {err:#}");
            ui.state.worlds_status = ui.state.home_status.clone();
            log::warn!("room bus after join/start failed: {err:#}");
        }
    }
    ui.mark_dash_chrome_dirty();
    ui.mark_dash_logic_dirty();
    ui.rebuild();
}

fn apply_start_session_result(
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    pending_room_bind: &mut Option<PendingRoomBind>,
    room_bind_progress_painted: &mut bool,
    visibility: Result<world_api::VisibilityEnsure, String>,
    join: Result<(), String>,
) {
    let wanted = ui.state.home_access_level.as_str();
    let name = ui
        .state
        .home_session_name
        .trim()
        .chars()
        .take(crate::system_ui::SESSION_NAME_MAX)
        .collect::<String>();
    match (&visibility, join) {
        (Ok(world_api::VisibilityEnsure::Applied), Ok(())) => {
            schedule_binding_room_bus(
                ui,
                pending_room_bind,
                room_bind_progress_painted,
                PendingRoomBind {
                    stop_host_on_fail: true,
                    success: PendingBindSuccess::Host {
                        name,
                        wanted: wanted.to_string(),
                    },
                },
            );
        }
        (Ok(world_api::VisibilityEnsure::LeftExisting), Ok(())) => {
            // Not owner (PUT 403/404) — never keep a local host, even if a job spawned by mistake.
            stop_supervised_host(shared);
            schedule_binding_room_bus(
                ui,
                pending_room_bind,
                room_bind_progress_painted,
                PendingRoomBind {
                    stop_host_on_fail: false,
                    success: PendingBindSuccess::StartGuestNotOwner {
                        wanted: wanted.to_string(),
                    },
                },
            );
        }
        (vis, Err(err)) => {
            // x00a1: accept/claim failure must drop nominated so Waiting does not auto-retry forever.
            ui.state.worlds_nominated_host = false;
            if is_host_authority_conflict(&err) {
                apply_host_authority_conflict(ui, shared, &err);
            } else {
                stop_supervised_host(shared);
                clear_worlds_session_shell(ui);
                ui.state.home_status = match vis {
                    Ok(world_api::VisibilityEnsure::Applied) => {
                        format!("Access level applied ({wanted}) but join failed: {err}")
                    }
                    Ok(world_api::VisibilityEnsure::LeftExisting) => {
                        format!("Visibility unchanged (not owner); join failed: {err}")
                    }
                    Err(vis_err) => {
                        format!("Start failed: visibility={vis_err}; join={err}")
                    }
                };
                ui.state.worlds_status = ui.state.home_status.clone();
                log::warn!("start session join failed: {err}");
            }
        }
        // StartSession worker always pairs visibility Err with join Err; keep arm for exhaustiveness.
        (Err(vis_err), Ok(())) => {
            stop_supervised_host(shared);
            clear_worlds_session_shell(ui);
            ui.state.home_status =
                format!("Joined — visibility step failed ({vis_err}); existing ACL");
            ui.state.worlds_status = ui.state.home_status.clone();
            log::warn!("unexpected visibility Err + join Ok: {vis_err}");
        }
    }
    ui.mark_dash_chrome_dirty();
    ui.mark_dash_logic_dirty();
    ui.rebuild();
}

struct JoinUiOpts<'a> {
    ok_status: &'a str,
    ok_log: &'a str,
    /// Worlds / Settings Advanced Join: log wording only (enter always waits for Frame).
    enter_waiting_shell: bool,
    can_host_rescue_on_ok: bool,
}

fn apply_join_result(
    ui: &mut SystemUi,
    pending_room_bind: &mut Option<PendingRoomBind>,
    room_bind_progress_painted: &mut bool,
    join: Result<(), String>,
    opts: JoinUiOpts<'_>,
) {
    match join {
        Ok(()) => {
            let enter_log_detail = if opts.enter_waiting_shell {
                "Ticket OK — waiting Manifest then Frame".to_string()
            } else {
                "Joined — waiting Manifest then Frame".to_string()
            };
            schedule_binding_room_bus(
                ui,
                pending_room_bind,
                room_bind_progress_painted,
                PendingRoomBind {
                    stop_host_on_fail: false,
                    success: PendingBindSuccess::Join {
                        ok_status: opts.ok_status.to_string(),
                        ok_log: opts.ok_log.to_string(),
                        can_host_rescue: opts.can_host_rescue_on_ok,
                        enter_log_detail,
                    },
                },
            );
        }
        Err(err) => {
            if is_host_authority_conflict(&err) {
                // Takeover race while Waiting — keep Waiting shell; access JWT stays.
                // CTA from next session poll only (pending candidate must not flash Takeover).
                ui.state.worlds_nominated_host = false;
                ui.state.worlds_can_takeover = false;
                ui.state.session_enter_active = false;
                ui.state.home_status =
                    "Takeover lost (409) — another host claimed; wait or retry".into();
                ui.state.worlds_status = ui.state.home_status.clone();
                log::warn!("takeover conflict (session kept): {err}");
            } else {
                clear_worlds_session_shell(ui);
                ui.state.home_status = format!("Join failed: {err}");
                ui.state.worlds_status = format!("Join failed: {err}");
                log::warn!("join failed (session kept): {err}");
            }
        }
    }
    ui.mark_dash_chrome_dirty();
    ui.mark_dash_logic_dirty();
    ui.rebuild();
}

/// How local host spawn claims compute-host authority (x0080).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostStartKind {
    /// Owner create/PUT path (Organizing / Host Rescue as owner).
    Owner,
    /// CAS takeover while dormant / lease-expired.
    Takeover,
    /// Handover candidate — must `accept_handover` before host Token (x00a1).
    Nominated,
}

/// Worlds Host / Host Rescue / Takeover path (leave → spawn → rejoin).
fn start_worlds_host(
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    zenoh: &mut Option<zenoh::Session>,
    auth_job: &mut Option<(u64, Receiver<AuthJobOutcome>)>,
    room_id: String,
    kind: HostStartKind,
) {
    if auth_job.is_some() {
        ui.state.worlds_status = "Auth busy…".into();
        ui.rebuild();
        return;
    }
    if room_id.is_empty() {
        ui.state.worlds_status = "Select a room to host".into();
        ui.rebuild();
        return;
    }
    let (access, access_expires_in) = shared
        .auth
        .lock()
        .ok()
        .map(|g| (g.access_token.clone(), g.access_expires_in_remaining()))
        .unwrap_or((None, None));
    let Some(access) = access else {
        ui.state.worlds_status = "Sign in before hosting".into();
        ui.rebuild();
        return;
    };
    let level = ui.state.home_access_level;
    // MVP vs slice: leave → spawn → rejoin (not stay-joined rescue).
    leave_room(shared, zenoh);
    shared.set_room_id(Arc::<str>::from(room_id.as_str()));
    ui.state.home_room_id = room_id;
    clear_worlds_session_shell(ui);
    let shared_job = Arc::clone(shared);
    begin_enter_ui(
        ui,
        &format!("Starting host ({})…", level.as_str()),
        "Ensuring room visibility…",
    );
    let (tx, rx) = mpsc::channel();
    let epoch = arm_auth_job(shared, auth_job, rx);
    spawn_auth_http(move || {
        if !shared_job.auth_work_is_current(epoch) {
            return;
        }
        send_enter_progress(
            &tx,
            &shared_job,
            epoch,
            0.15,
            "Ensuring room…",
            "PUT/create room visibility",
        );
        let world_url = shared_job.get_world_url();
        let room_id = shared_job.get_room_id();
        let visibility =
            world_api::ensure_room_visibility(&world_url, room_id.as_ref(), &access, level)
                .map_err(|e| format!("{e:#}"));
        if !shared_job.auth_work_is_current(epoch) {
            return;
        }
        let claim = match kind {
            HostStartKind::Owner => match &visibility {
                Ok(world_api::VisibilityEnsure::Applied) => Ok(()),
                Ok(world_api::VisibilityEnsure::LeftExisting) => {
                    Err("cannot host: not room owner (visibility unchanged)".into())
                }
                Err(e) => Err(e.clone()),
            },
            HostStartKind::Takeover => {
                send_enter_progress(
                    &tx,
                    &shared_job,
                    epoch,
                    0.28,
                    "Claiming host…",
                    "POST /rooms/:id/takeover",
                );
                world_api::takeover(&world_url, &access, room_id.as_ref())
                    .map(|_| ())
                    .map_err(|e| format!("{e:#}"))
            }
            HostStartKind::Nominated => {
                send_enter_progress(
                    &tx,
                    &shared_job,
                    epoch,
                    0.28,
                    "Accepting handover…",
                    "POST /rooms/:id/handover/accept",
                );
                world_api::accept_handover(&world_url, &access, room_id.as_ref())
                    .map(|_| ())
                    .map_err(|e| format!("{e:#}"))
            }
        };
        let mut host_started = false;
        let join = match claim {
            Ok(()) => {
                send_enter_progress(
                    &tx,
                    &shared_job,
                    epoch,
                    0.40,
                    "Starting local host…",
                    "Spawning host-server (cargo run can take a while)",
                );
                match shared_job.host_supervisor.ensure_running_for_room(
                    room_id.as_ref(),
                    Some(access.as_str()),
                    access_expires_in,
                ) {
                    Ok(()) => {
                        host_started = true;
                        send_enter_progress(
                            &tx,
                            &shared_job,
                            epoch,
                            0.72,
                            "Joining room…",
                            "host-server ready — fetching Room Token",
                        );
                        if !shared_job.auth_work_is_current(epoch) {
                            // Stale: Leave keeps host; do not stop a possible re-Host.
                            Err("host start aborted: signed out".into())
                        } else {
                            match join_room_with_access(&shared_job, &access, epoch) {
                                Ok(()) => {
                                    if let Err(err) =
                                        install_host_inventory_token(&shared_job, &access)
                                    {
                                        log::warn!(
                                            "host inventory Token after Host join failed: {err:#}"
                                        );
                                    }
                                    Ok(())
                                }
                                Err(e) => Err(format!("{e:#}")),
                            }
                        }
                    }
                    Err(err) => Err(format!("local host-server start failed: {err:#}")),
                }
            }
            Err(err) => Err(err),
        };
        if join.is_err() && host_started {
            stop_host_if_job_current(&shared_job, epoch);
        }
        if join.is_ok() {
            send_enter_progress(
                &tx,
                &shared_job,
                epoch,
                0.85,
                "Binding session…",
                "Room Token OK — opening Zenoh bus",
            );
        }
        // Non-owner Takeover/Nominated often get LeftExisting on visibility PUT; UI Host path
        // still needs Applied once claim+join succeeded (x0080).
        let visibility = match kind {
            HostStartKind::Owner => visibility,
            HostStartKind::Takeover | HostStartKind::Nominated if join.is_ok() => {
                Ok(world_api::VisibilityEnsure::Applied)
            }
            HostStartKind::Takeover | HostStartKind::Nominated => visibility,
        };
        if shared_job.auth_work_is_current(epoch) {
            let _ = tx.send(AuthJobOutcome::StartSession { visibility, join });
        }
        // Stale: Leave/Logout already bumped epoch and owns teardown — do not stop a newer host.
    });
}

/// Load Worlds catalog into `ui.state.worlds_rooms` (MainHub Participation / Dash Worlds).
pub(super) fn start_worlds_catalog_refresh(
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    auth_job: &mut Option<(u64, Receiver<AuthJobOutcome>)>,
) {
    let access = shared.auth.lock().ok().and_then(|g| g.access_token.clone());
    let Some(access) = access else {
        ui.state.worlds_status = "Sign in to load Worlds".into();
        ui.rebuild();
        return;
    };
    if auth_job.is_some() {
        ui.state.worlds_status = "Auth busy…".into();
        ui.rebuild();
        return;
    }
    // x0080: Participation = Public active sessions; Organizing = mine (incl. dormant).
    ui.state.worlds_mine_only = ui.state.main_hub_tab == crate::system_ui::MainHubTab::Organizing;
    let mine = ui.state.worlds_mine_only;
    let shared_job = Arc::clone(shared);
    ui.state.worlds_status = if mine {
        "Loading my worlds…".into()
    } else {
        "Loading public sessions…".into()
    };
    ui.rebuild();
    let (tx, rx) = mpsc::channel();
    let epoch = arm_auth_job(shared, auth_job, rx);
    spawn_auth_http(move || {
        let world_url = shared_job.get_world_url();
        let result = world_api::list_rooms(&world_url, &access, mine).map_err(|e| format!("{e:#}"));
        if shared_job.auth_work_is_current(epoch) {
            let _ = tx.send(AuthJobOutcome::WorldsList { result });
        }
    });
}

/// Step1 Apply: retarget endpoints + drop stale Zenoh so the next join uses the new cluster.
pub(super) fn apply_operator_endpoints(
    ui: &mut SystemUi,
    shared: &SharedState,
    zenoh: &mut Option<zenoh::Session>,
) {
    if ui.state.operator == OperatorKind::Federation {
        return;
    }
    let ep = endpoints_for(ui.state.operator, &ui.state.custom_host);
    shared.apply_endpoints(&ep);
    // Drop any pre-join / stale session so join opens the new endpoint.
    close_zenoh(zenoh);
    shared
        .room_bus_started
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let operator = ui.state.operator;
    let custom_host = ui.state.custom_host.clone();
    // RMW under prefs lock — must not resurrect a cleared refresh_vault_key.
    operator_prefs::update_prefs(|prefs| {
        prefs.operator_chosen = true;
        prefs.operator = operator;
        prefs.custom_host = custom_host;
        prefs.apply_endpoints(&ep);
    });
    log::info!(
        "operator endpoints applied: auth={} world={} zenoh={}",
        ep.auth_url,
        ep.world_url,
        ep.zenoh_connect
    );
}

/// x0068: Token refresh installed new mTLS PEMs → reopen Zenoh while joined.
pub(super) fn poll_mtls_reopen(shared: &Arc<SharedState>, zenoh: &mut Option<zenoh::Session>) {
    let joined = shared
        .auth
        .lock()
        .ok()
        .map(|g| g.is_joined())
        .unwrap_or(false);
    if !joined {
        return;
    }
    if !shared
        .mtls_reopen_pending
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return;
    }
    match bind_room_after_join(shared, zenoh) {
        Ok(()) => log::info!("mTLS refresh: Zenoh reopened + room bus rebound"),
        // pending restore is owned by bind_room_after_join (leave_room clears it).
        Err(err) => log::warn!("mTLS refresh reopen failed (will retry): {err:#}"),
    }
}
