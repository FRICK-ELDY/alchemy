//! Desktop winit + wgpu surface (ESC dash + Noclip + mouse look + Grab).
//!
//! Guarantees: canvas-pixel pointer → system_ui `on_pointer` (hover/press) + click;
//! dash-closed LMB → cursor grab (`Confined` then `Locked`); mouse Δ → client-local
//! `CameraLook`; WASD projected from current yaw; look-capture + aim-down then
//! LMB/RMB/E hold → Grab via look-forward ray (x0013). Floor boxes need pitch down.
//! Look is never on Zenoh.
//! Auth login/register/logout and room start/join HTTP run on a worker thread; the UI
//! thread only polls the outcome (x002e / P3-9 / x0037).
//! x0053 Companion: F8 toggles VR↔Desktop present; Intent/Grab only while
//! `DesktopRole::owns_input`; present handoff zeros shared `MoveIntent`;
//! exit mirrors XR `presentation.exit_requested`; Zenoh bind/leave/mTLS are
//! owned here only. Companion still redraws while `vr_active` (best-effort
//! mirror — extra GPU cost, not a hotswitch guarantee).
//!
//! Session-enter overlay + deferred `bind_room_after_join` (Binding progress) are
//! **Desktop-only**; XR companion does not mirror that wait UX (non-guarantee).
//!
//! Linux/WSL: prefer Confined + `CursorMoved` (recenter). `Locked` often succeeds but
//! yields no `DeviceEvent::MouseMotion`, so look appears dead.

use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;

use alchemy_protocol::input_buttons;
use anyhow::{Context, Result};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{DeviceEvent, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Cursor, CursorGrabMode, CustomCursor, Window, WindowId};

use crate::auth;
use crate::host_supervisor;
use crate::mesh::CpuMesh;
use crate::operator_prefs::{self, endpoints_for, OperatorKind};
use crate::room::install_host_inventory_token;
use crate::scene::{
    create_depth_view, create_wgpu_for_window, CameraLook, EntityPose, MoveIntent, SceneGpu,
    SharedState,
};
use crate::surface::hotswitch::DesktopRole;
use crate::system_ui::{
    rematch_scene_selection, DashTab, InventoryPeerRequest, OnboardingShell, SystemUi, UiAction,
};
use crate::world_api;
use crate::world_hit::{desktop_look_ray, pick_grabbable_desktop};
use crate::{bind_room_after_join, close_zenoh, join_room_with_access, leave_room};

/// Radians per pixel (fixed; Settings Facet deferred).
const LOOK_SENSITIVITY: f32 = 0.0025;
/// Match `SceneGpu::desktop_view_proj` pitch clamp (~±88°).
const PITCH_LIMIT: f32 = 1.54;

struct DesktopGpu {
    surface: wgpu::Surface<'static>,
    scene: SceneGpu,
    config: wgpu::SurfaceConfiguration,
    depth_view: wgpu::TextureView,
}

impl DesktopGpu {
    async fn new(window: Arc<Window>, mesh: &CpuMesh, shared: Arc<SharedState>) -> Result<Self> {
        let (_instance, surface, _adapter, device, queue, config, format) =
            create_wgpu_for_window(Arc::clone(&window)).await?;
        let depth_view = create_depth_view(&device, config.width, config.height);
        let scene = SceneGpu::from_device(device, queue, format, mesh, shared)?;
        Ok(Self {
            surface,
            scene,
            config,
            depth_view,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.scene.device, &self.config);
        self.depth_view = create_depth_view(&self.scene.device, width, height);
    }

    fn render(&mut self, ui: &mut SystemUi, highlight_entity: Option<u32>) -> Result<()> {
        self.scene.maybe_log_pose();
        let eye = self.scene.user_root_pose();
        let look = self.scene.look();
        let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
        let (view, proj) = SceneGpu::desktop_view_proj(eye, look, aspect);

        ui.prepare_frame(&self.scene.queue);

        let frame = self
            .surface
            .get_current_texture()
            .context("get_current_texture")?;
        let frame_view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder =
            self.scene
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("desktop-encoder"),
                });

        // Once per frame (not inside draw_world — XR shares that path per-eye).
        self.scene.sync_epoch_gpu(&mut encoder);
        self.scene.draw_world(
            &mut encoder,
            &frame_view,
            &self.depth_view,
            view,
            proj,
            wgpu::LoadOp::Clear(wgpu::Color {
                r: 0.05,
                g: 0.06,
                b: 0.09,
                a: 1.0,
            }),
            highlight_entity,
        );

        if ui.state.open {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ui-overlay"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            ui.draw(&mut pass);
        }

        self.scene.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }
}

struct App {
    mesh: CpuMesh,
    shared: Arc<SharedState>,
    window: Option<Arc<Window>>,
    gpu: Option<DesktopGpu>,
    ui: SystemUi,
    zenoh: Option<zenoh::Session>,
    role: DesktopRole,
    cursor: (f64, f64),
    primary_down: bool,
    keys_down: HashSet<KeyCode>,
    /// True while cursor grab is active.
    look_captured: bool,
    /// Once raw device motion arrives, skip CursorMoved look (avoid double sensitivity).
    device_look_active: bool,
    /// Ignore one CursorMoved after warping to window center.
    skip_cursor_warp: bool,
    last_cursor: Option<(f64, f64)>,
    /// 1×1 transparent cursor (WSL often ignores `set_cursor_visible(false)` alone).
    blank_cursor: Option<CustomCursor>,
    /// Entity held by Desktop Grab (cleared on release or dash open).
    held_grab: Option<u32>,
    /// RMB held.
    grab_down: bool,
    /// After look capture, require one LMB release before LMB can Grab (avoid capture-click grab).
    lmb_grab_armed: bool,
    /// Rate-limit "grab miss" logs.
    last_grab_miss_log: Option<std::time::Instant>,
    /// In-flight Auth HTTP (+ join) worker: `(auth_work_epoch, rx)` (x002e / x007c).
    auth_job: Option<(u64, Receiver<AuthJobOutcome>)>,
    /// Waiting-shell host-session poll (x0080); off UI thread so ureq never blocks paint.
    waiting_session_job: Option<Receiver<WaitingSessionSnapshot>>,
    /// Last observed Desktop input ownership (READY / vr_active edges).
    last_owns_input: bool,
    /// Last applied [`SharedState::pending_mesh_epoch`] (x007d remount).
    mesh_epoch: u64,
    /// After Join/Host ticket OK: show "Binding room bus…" for one paint, then bind.
    pending_room_bind: Option<PendingRoomBind>,
    /// Set after "Binding…" has been drawn once; [`Self::poll_pending_room_bind`] then runs bind.
    room_bind_progress_painted: bool,
}

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

struct PendingRoomBind {
    stop_host_on_fail: bool,
    success: PendingBindSuccess,
}

/// Mut refs for deferred bind scheduling (keeps `handle_ui_action` under clippy arity).
struct RoomBindDefer<'a> {
    pending: &'a mut Option<PendingRoomBind>,
    painted: &'a mut bool,
}

impl RoomBindDefer<'_> {
    fn clear(&mut self) {
        *self.pending = None;
        *self.painted = false;
    }
}

/// Outcome of Auth / world HTTP work off the UI thread (x002e / P3-9 / x0037).
enum AuthJobOutcome {
    /// Session only (no auto-join).
    LoginOk {
        username: String,
    },
    RegisterOk {
        username: String,
    },
    LoginFailed(String),
    RegisterFailed(String),
    /// Mid-flight Create/Join/Host progress (does not clear `auth_job`).
    EnterProgress {
        progress: f32,
        title: String,
        log: String,
    },
    /// visibility ensure + guest join (`visibility` says whether the chosen level was applied).
    StartSession {
        visibility: Result<world_api::VisibilityEnsure, String>,
        join: Result<(), String>,
    },
    JoinRoom {
        result: Result<(), String>,
        /// Worlds catalog Join → Waiting shell; Home Join keeps prior UX.
        enter_waiting_shell: bool,
        /// Set `worlds_can_host_rescue` only after successful join (owner guest path).
        can_host_rescue_on_ok: bool,
    },
    WorldsList {
        result: Result<Vec<world_api::RoomSummary>, String>,
    },
    /// Mid-flight paced inventory spawn (does not clear `auth_job`).
    InventoryTakeOutProgress {
        status: String,
    },
    /// Prefab/blob take-out HTTP + paced Zenoh puts finished.
    InventoryTakeOutDone {
        status: String,
    },
}

fn send_enter_progress(
    tx: &std::sync::mpsc::Sender<AuthJobOutcome>,
    shared: &SharedState,
    epoch: u64,
    progress: f32,
    title: &str,
    log: &str,
) {
    if shared.auth_work_is_current(epoch) {
        let _ = tx.send(AuthJobOutcome::EnterProgress {
            progress,
            title: title.into(),
            log: log.into(),
        });
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

fn spawn_auth_http<F>(f: F)
where
    F: FnOnce() + Send + 'static,
{
    if let Err(err) = thread::Builder::new().name("auth-http".into()).spawn(f) {
        log::error!("spawn auth-http failed: {err}");
    }
}

/// Clear Worlds in-room / Waiting / stall / enter state (Leave, Logout, failed join/host).
fn clear_worlds_session_shell(ui: &mut SystemUi) {
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
    ui.state.end_session_enter();
    // Drop peer bit so Leave/Logout can push in_session=false (XR Inventory unblocks).
    ui.state.in_session_peer = false;
    ui.mark_dash_chrome_dirty();
}

/// Drop catalog + selection (logout / XR leave). Does not touch session shell flags.
fn clear_worlds_catalog(ui: &mut SystemUi) {
    ui.state.worlds_rooms.clear();
    ui.state.worlds_selected = None;
    ui.state.worlds_catalog_scroll = 0;
    ui.state.worlds_catalog_scroll_accum = 0.0;
    ui.state.worlds_status.clear();
}

/// Discard in-flight auth/catalog/inventory worker and bump epoch so the thread cannot commit.
/// Clears sticky take_out / spawn-put status so Leave/Logout does not leave Mirror mid-flight.
fn discard_auth_job(
    ui: &mut SystemUi,
    shared: &SharedState,
    auth_job: &mut Option<(u64, Receiver<AuthJobOutcome>)>,
) {
    shared.bump_auth_work_epoch();
    *auth_job = None;
    let sticky = ui.state.inventory_status.starts_with("take_out")
        || ui.state.inventory_status.starts_with("spawn put");
    if sticky {
        ui.state.inventory_status = "take_out cancelled".into();
        ui.state.inventory_take_out_override = None;
        publish_inventory_mirror_from_ui(ui, shared);
    }
}

/// Arm a new worker against the current epoch (call only when `auth_job` is empty).
fn arm_auth_job(
    shared: &SharedState,
    auth_job: &mut Option<(u64, Receiver<AuthJobOutcome>)>,
    rx: Receiver<AuthJobOutcome>,
) -> u64 {
    let epoch = shared.auth_work_epoch();
    *auth_job = Some((epoch, rx));
    epoch
}

/// Stop local host only while this job's epoch is still current.
/// Stale workers must not touch host — Leave keeps access and a newer Host may already run.
fn stop_host_if_job_current(shared: &SharedState, epoch: u64) {
    if shared.auth_work_is_current(epoch) {
        stop_supervised_host(shared);
    }
}

/// Failed Start / guest path: cancel in-flight spawn; allow a later ensure retry.
/// External / CI hosts are never touched.
/// Result of one background Waiting host-session GET (x0080).
struct WaitingSessionSnapshot {
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
        let nominated = summary
            .active_host_sub
            .as_deref()
            .is_some_and(|s| s == local_sub)
            && summary.state == "active";
        let can_takeover = summary.state == "dormant";
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

/// x0080: nominate a Presence peer or dormant the session before stopping local host.
/// Runs ureq off this thread but waits up to 3s so stop-host does not race the CAS.
fn maybe_handover_before_leave(shared: &SharedState) {
    let access = shared.auth.lock().ok().and_then(|g| g.access_token.clone());
    let Some(access) = access else {
        log::warn!("handover skipped: no access token");
        return;
    };
    let room_id = shared.get_room_id();
    if room_id.is_empty() {
        return;
    }
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
    if ui.state.worlds_in_room {
        ui.state.worlds_waiting_host = true;
        ui.state.worlds_can_takeover = true;
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

fn stop_supervised_host(shared: &SharedState) {
    // Even when Idle: sets cancel_spawn so an in-flight ensure_running aborts.
    shared.host_supervisor.shutdown_graceful();
}

/// Client Exit: sticky exit latch + stop (Start HTTP 中の Exit でも後続 ensure を拒否).
fn exit_stop_supervised_host(shared: &SharedState) {
    if shared.host_supervisor.owns_local_host() {
        maybe_handover_before_leave(shared);
    }
    shared.host_supervisor.latch_exit_and_shutdown();
}

impl App {
    fn owns_input(&self) -> bool {
        self.role.owns_input(&self.shared.presentation)
    }

    fn window_title(&self) -> &'static str {
        match self.role {
            DesktopRole::Solo => "Alchemy Client — Desktop",
            DesktopRole::Companion if !self.shared.presentation.xr_session_live() => {
                // XR init fail / exit → Solo-like.
                "Alchemy Client — Desktop"
            }
            DesktopRole::Companion if !self.shared.presentation.xr_session_started() => {
                "Alchemy Client — Desktop (XR starting…)"
            }
            DesktopRole::Companion if self.shared.presentation.vr_active() => {
                "Alchemy Client — VR active (F8 → Desktop)"
            }
            DesktopRole::Companion => "Alchemy Client — Desktop (F8 → VR)",
        }
    }

    fn refresh_window_title(&self) {
        if let Some(window) = self.window.as_ref() {
            window.set_title(self.window_title());
        }
    }

    fn sync_hotswitch_ui(&mut self) {
        let auto = self
            .shared
            .presentation
            .hotswitch_auto
            .load(std::sync::atomic::Ordering::SeqCst);
        let suppressed = self
            .shared
            .presentation
            .auto_suppressed
            .load(std::sync::atomic::Ordering::SeqCst);
        let prev_auto = self.ui.state.hotswitch_auto;
        let prev_suppressed = self.ui.state.hotswitch_suppressed;
        let prev_available = self.ui.state.hotswitch_available;
        let prev_label = self.ui.state.hotswitch_label.clone();
        // Hotswitch controls only after READY (same gate as F8). Local gate only.
        // PresentationGate is SoT for auto/suppressed — not SharedDashLogic.
        self.ui.state.hotswitch_available =
            self.role == DesktopRole::Companion && self.shared.presentation.xr_session_started();
        self.ui.state.hotswitch_auto = auto;
        self.ui.state.hotswitch_suppressed = suppressed;
        self.ui.state.refresh_hotswitch_label();
        // Title depends on live|started|vr, not only owns_input — refresh with UI sync.
        self.refresh_window_title();
        let view_changed = prev_auto != auto
            || prev_suppressed != suppressed
            || prev_available != self.ui.state.hotswitch_available
            || self.ui.state.hotswitch_label != prev_label;
        if view_changed && self.ui.state.open {
            self.ui.rebuild();
        }
    }

    fn apply_look_delta(&mut self, dx: f64, dy: f64) {
        if !self.owns_input() || !self.look_captured || self.ui.state.open {
            return;
        }
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        if let Ok(mut look) = self.shared.look.lock() {
            *look = apply_mouse_look(*look, dx, dy, LOOK_SENSITIVITY, PITCH_LIMIT);
        }
    }

    /// Fires when Desktop gains/loses Intent ownership (READY edge or vr_active).
    fn on_input_ownership_edge(&mut self) {
        let owns = self.owns_input();
        if owns == self.last_owns_input {
            return;
        }
        self.last_owns_input = owns;
        self.refresh_window_title();
        // Ownership edge: drop look / Grab and zero shared Intent (no ghost WASD/stick).
        self.set_look_capture(false);
        self.held_grab = None;
        self.grab_down = false;
        self.keys_down.clear();
        self.ui.clear_pointer();
        self.shared.clear_move_intent();
        if owns {
            log::info!("hotswitch: Desktop took input ownership");
        } else {
            log::info!("hotswitch: Desktop cleared Intent / yielded to VR");
        }
    }

    /// XR-requested leave / host shutdown (Desktop is the Zenoh owner).
    fn poll_presentation_requests(&mut self) {
        if self.shared.presentation.take_leave_requested() {
            let had_zenoh = self.zenoh.is_some();
            if !had_zenoh {
                log::warn!(
                    "hotswitch: leave requested but Zenoh is None — clearing auth without Leave put"
                );
            }
            // Bump epoch before Leave so in-flight join cannot commit past this point.
            discard_auth_job(&mut self.ui, &self.shared, &mut self.auth_job);
            if self.shared.host_supervisor.owns_local_host() {
                maybe_handover_before_leave(&self.shared);
            }
            leave_room(&self.shared, &mut self.zenoh);
            stop_supervised_host(&self.shared);
            clear_worlds_session_shell(&mut self.ui);
            clear_worlds_catalog(&mut self.ui);
            if let Ok(mut gate) = self.shared.auth.lock() {
                gate.clear_session();
            }
            let mut prefs = operator_prefs::load();
            prefs.clear_session();
            operator_prefs::save(&prefs);
            self.ui.state.home_logged_in_as = None;
            self.ui.state.home_create_open = false;
            self.ui.state.shell = OnboardingShell::Step1Operator;
            self.ui.state.home_status = if had_zenoh {
                "Signed out — Leave sent; select a server operator".into()
            } else {
                "Signed out — local only; select a server operator".into()
            };
            self.ui.state.open = true;
            self.ui.state.tab = DashTab::Home;
            self.ui.mark_dash_logic_dirty();
            self.ui.rebuild();
            log::info!("hotswitch: Desktop performed leave_room for XR AuthLogout");
        }
        if self.shared.presentation.take_host_shutdown_requested() {
            exit_stop_supervised_host(&self.shared);
            log::info!("hotswitch: Desktop graceful host stop for XR Exit");
            self.shared.presentation.request_exit();
        }
    }

    fn toggle_hotswitch_manual(&mut self) {
        if self.role != DesktopRole::Companion {
            return;
        }
        // READY only — avoids flipping into a preferred-VR state before XR can write Intent.
        if !self.shared.presentation.xr_session_started() {
            log::info!("hotswitch: F8 ignored (XR not READY yet)");
            return;
        }
        let now_vr = self.shared.presentation.toggle_vr_active();
        log::info!("hotswitch: F8 → {}", if now_vr { "VR" } else { "Desktop" });
        self.on_input_ownership_edge();
    }

    /// x0068: Token refresh installed new mTLS PEMs → reopen Zenoh while joined.
    fn poll_mtls_reopen(&mut self) {
        let joined = self
            .shared
            .auth
            .lock()
            .ok()
            .map(|g| g.is_joined())
            .unwrap_or(false);
        if !joined {
            return;
        }
        if !self
            .shared
            .mtls_reopen_pending
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return;
        }
        match bind_room_after_join(&self.shared, &mut self.zenoh) {
            Ok(()) => log::info!("mTLS refresh: Zenoh reopened + room bus rebound"),
            // pending restore is owned by bind_room_after_join (leave_room clears it).
            Err(err) => log::warn!("mTLS refresh reopen failed (will retry): {err:#}"),
        }
    }

    /// XR Inventory ops queued on SharedState (x007f). Desktop owns HTTP/Zenoh.
    fn drain_inventory_peer_actions(&mut self, event_loop: &ActiveEventLoop) {
        let pending = match self.shared.inventory_peer_actions.lock() {
            Ok(mut q) => std::mem::take(&mut *q),
            Err(_) => {
                log::warn!("inventory_peer_actions lock poisoned — XR queue dropped");
                Vec::new()
            }
        };
        for req in pending {
            let action = match apply_inventory_peer_request(&mut self.ui, &self.shared, &req) {
                Some(a) => a,
                None => continue,
            };
            handle_ui_action(
                action,
                &mut self.ui,
                &self.shared,
                &mut self.zenoh,
                &mut self.auth_job,
                RoomBindDefer {
                    pending: &mut self.pending_room_bind,
                    painted: &mut self.room_bind_progress_painted,
                },
                event_loop,
            );
        }
    }

    /// Non-blocking poll of Auth worker (x002e). Safe to call every frame.
    /// Drains `EnterProgress` without clearing the job; finals clear it.
    fn poll_auth_job(&mut self) {
        let Some((epoch, _)) = self.auth_job.as_ref() else {
            return;
        };
        if !self.shared.auth_work_is_current(*epoch) {
            // Logout / Leave discarded this generation — drop without UI noise.
            self.auth_job = None;
            return;
        }
        let epoch = *epoch;
        loop {
            let Some((_, rx)) = self.auth_job.as_ref() else {
                return;
            };
            let next = match rx.try_recv() {
                Ok(outcome) => Ok(outcome),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => Err(()),
            };
            match next {
                Err(()) => {
                    self.auth_job = None;
                    if self.ui.state.session_enter_active {
                        self.ui.state.end_session_enter();
                    }
                    self.ui.state.home_status = "Auth worker disconnected".into();
                    self.ui.mark_dash_form_dirty();
                    self.ui.rebuild();
                    log::warn!("auth worker disconnected before result");
                    return;
                }
                Ok(AuthJobOutcome::EnterProgress {
                    progress,
                    title,
                    log,
                }) => {
                    if !self.shared.auth_work_is_current(epoch) {
                        self.auth_job = None;
                        return;
                    }
                    self.ui
                        .state
                        .apply_session_enter_progress(progress, &title, &log);
                    if !title.is_empty() {
                        self.ui.state.home_status = title.clone();
                        self.ui.state.worlds_status = title;
                    }
                    self.ui.mark_dash_chrome_dirty();
                    self.ui.rebuild();
                    // Keep draining progress / possible final in the same frame.
                }
                Ok(AuthJobOutcome::InventoryTakeOutProgress { status }) => {
                    if !self.shared.auth_work_is_current(epoch) {
                        self.auth_job = None;
                        return;
                    }
                    self.ui.state.inventory_status = status;
                    publish_inventory_mirror_from_ui(&self.ui, &self.shared);
                    self.ui.rebuild();
                }
                Ok(outcome) => {
                    if !self.shared.auth_work_is_current(epoch) {
                        self.auth_job = None;
                        return;
                    }
                    let refresh_main_hub = matches!(
                        &outcome,
                        AuthJobOutcome::LoginOk { .. } | AuthJobOutcome::RegisterOk { .. }
                    );
                    self.auth_job = None;
                    self.apply_auth_outcome(outcome);
                    if refresh_main_hub {
                        start_worlds_catalog_refresh(
                            &mut self.ui,
                            &self.shared,
                            &mut self.auth_job,
                        );
                    }
                    return;
                }
            }
        }
    }

    fn persist_session_prefs(&self, username: &str) {
        let mut prefs = operator_prefs::load();
        prefs.operator_chosen = true;
        prefs.operator = self.ui.state.operator;
        prefs.custom_host = self.ui.state.custom_host.clone();
        let ep = endpoints_for(prefs.operator, &prefs.custom_host);
        prefs.apply_endpoints(&ep);
        prefs.username = Some(username.to_string());
        if self.ui.state.home_remember_me {
            prefs.refresh_token = self
                .shared
                .auth
                .lock()
                .ok()
                .and_then(|g| g.refresh_token.clone());
            prefs.last_login_at = Some(operator_prefs::now_unix_secs());
        } else {
            prefs.clear_session();
            prefs.username = Some(username.to_string());
        }
        operator_prefs::save(&prefs);
    }

    fn apply_auth_outcome(&mut self, outcome: AuthJobOutcome) {
        match outcome {
            AuthJobOutcome::EnterProgress { .. } => {
                // Handled in poll_auth_job (keep auth_job armed).
            }
            AuthJobOutcome::LoginOk { username } => {
                self.persist_session_prefs(&username);
                self.ui.state.home_logged_in_as = Some(username);
                self.ui.state.home_password.clear();
                self.ui.state.home_create_open = false;
                self.ui.state.shell = OnboardingShell::MainHub;
                self.ui.state.main_hub_tab = crate::system_ui::MainHubTab::Participation;
                self.ui.state.home_status = "Signed in — join a public session".into();
                self.ui.mark_dash_logic_dirty();
                self.ui.rebuild();
                log::info!("auth login ok (no auto-join)");
            }
            AuthJobOutcome::RegisterOk { username } => {
                self.persist_session_prefs(&username);
                self.ui.state.home_logged_in_as = Some(username);
                self.ui.state.home_password.clear();
                self.ui.state.home_create_open = false;
                self.ui.state.shell = OnboardingShell::MainHub;
                self.ui.state.main_hub_tab = crate::system_ui::MainHubTab::Participation;
                self.ui.state.home_status = "Account created — join a public session".into();
                self.ui.mark_dash_logic_dirty();
                self.ui.rebuild();
                log::info!("auth register ok (no auto-join)");
            }
            AuthJobOutcome::LoginFailed(err) => {
                self.ui.state.home_status = format!("Login failed: {err}");
                self.ui.mark_dash_form_dirty();
                self.ui.rebuild();
                log::warn!("login failed: {err}");
            }
            AuthJobOutcome::RegisterFailed(err) => {
                self.ui.state.home_status = format!("Register failed: {err}");
                self.ui.mark_dash_form_dirty();
                self.ui.rebuild();
                log::warn!("register failed: {err}");
            }
            AuthJobOutcome::StartSession { visibility, join } => {
                apply_start_session_result(
                    &mut self.ui,
                    &self.shared,
                    &mut self.pending_room_bind,
                    &mut self.room_bind_progress_painted,
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
                    &mut self.ui,
                    &mut self.pending_room_bind,
                    &mut self.room_bind_progress_painted,
                    result,
                    JoinUiOpts {
                        ok_status: "Joined — waiting for host if space is still empty",
                        ok_log: "join room ok",
                        enter_waiting_shell,
                        can_host_rescue_on_ok,
                    },
                );
            }
            AuthJobOutcome::InventoryTakeOutProgress { .. } => {
                // Handled in poll_auth_job (keep auth_job armed).
            }
            AuthJobOutcome::InventoryTakeOutDone { status } => {
                self.ui.state.inventory_status = status;
                publish_inventory_mirror_from_ui(&self.ui, &self.shared);
                self.ui.rebuild();
            }
            AuthJobOutcome::WorldsList { result } => {
                // Stale completion after logout / account switch — drop payload.
                if self.ui.state.home_logged_in_as.is_none() {
                    clear_worlds_catalog(&mut self.ui);
                    self.ui.mark_dash_logic_dirty();
                    self.ui.rebuild();
                    return;
                }
                match result {
                    Ok(rooms) => {
                        self.ui.state.worlds_rooms = rooms
                            .into_iter()
                            .map(|r| (r.room_id, r.visibility, r.role_hint, r.owner_sub))
                            .collect();
                        if let Some(sel) = self.ui.state.worlds_selected.as_ref() {
                            if !self.ui.state.worlds_rooms.iter().any(|(id, ..)| id == sel) {
                                self.ui.state.worlds_selected = None;
                            }
                        }
                        self.ui.state.clamp_worlds_catalog_scroll();
                        self.ui.state.worlds_status =
                            format!("{} visible room(s)", self.ui.state.worlds_rooms.len());
                        self.ui.mark_dash_logic_dirty();
                        self.ui.rebuild();
                    }
                    Err(err) => {
                        self.ui.state.worlds_status = format!("Catalog failed: {err}");
                        self.ui.mark_dash_logic_dirty();
                        self.ui.rebuild();
                        log::warn!("worlds catalog failed: {err}");
                    }
                }
            }
        }
    }

    fn apply_hidden_cursor(&self, window: &Window) {
        window.set_cursor_visible(false);
        if let Some(blank) = self.blank_cursor.as_ref() {
            window.set_cursor(Cursor::Custom(blank.clone()));
        }
    }

    fn apply_default_cursor(&self, window: &Window) {
        window.set_cursor(Cursor::Icon(winit::window::CursorIcon::Default));
        window.set_cursor_visible(true);
    }

    fn warp_cursor_center(&mut self, mark_skip: bool) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let size = window.inner_size();
        if size.width < 2 || size.height < 2 {
            return;
        }
        let cx = size.width as f64 * 0.5;
        let cy = size.height as f64 * 0.5;
        if window
            .set_cursor_position(PhysicalPosition::new(cx, cy))
            .is_ok()
        {
            if mark_skip {
                self.skip_cursor_warp = true;
            }
            self.last_cursor = Some((cx, cy));
            self.cursor = (cx, cy);
        }
    }

    /// Re-assert hide + soft-confine. WSL Confined often reports Ok but still leaks.
    fn maintain_look_capture(&mut self) {
        if !self.look_captured || self.ui.state.open {
            return;
        }
        let Some(window) = self.window.clone() else {
            return;
        };
        let _ = window
            .set_cursor_grab(CursorGrabMode::Confined)
            .or_else(|_| window.set_cursor_grab(CursorGrabMode::Locked));
        self.apply_hidden_cursor(&window);
        // Pull back only if drifted; avoid perpetual skip_cursor_warp (kills look).
        let size = window.inner_size();
        let cx = size.width as f64 * 0.5;
        let cy = size.height as f64 * 0.5;
        let (x, y) = self.cursor;
        let dist_sq = (x - cx) * (x - cx) + (y - cy) * (y - cy);
        if dist_sq > 36.0 {
            self.warp_cursor_center(true);
        }
    }

    fn set_look_capture(&mut self, capture: bool) {
        let Some(window) = self.window.clone() else {
            return;
        };
        if capture {
            // Confined first: keeps CursorMoved on Linux/WSL. Locked often swallows motion
            // without DeviceEvent::MouseMotion (look appears frozen after "capture on").
            let mode = match window.set_cursor_grab(CursorGrabMode::Confined) {
                Ok(()) => CursorGrabMode::Confined,
                Err(err) => {
                    log::warn!("cursor Confined failed ({err}); trying Locked");
                    match window.set_cursor_grab(CursorGrabMode::Locked) {
                        Ok(()) => CursorGrabMode::Locked,
                        Err(err2) => {
                            log::warn!("cursor Locked failed: {err2}");
                            self.look_captured = false;
                            self.apply_default_cursor(&window);
                            return;
                        }
                    }
                }
            };
            self.apply_hidden_cursor(&window);
            self.look_captured = true;
            self.lmb_grab_armed = false;
            self.device_look_active = false;
            self.skip_cursor_warp = false;
            self.warp_cursor_center(true);
            log::info!(
                "mouse look capture on ({mode:?}); aim at a box (look down) then hold LMB/RMB/E"
            );
        } else if self.look_captured {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            self.apply_default_cursor(&window);
            self.look_captured = false;
            self.lmb_grab_armed = false;
            self.device_look_active = false;
            self.skip_cursor_warp = false;
            self.last_cursor = None;
            log::info!("mouse look capture off");
        }
    }

    fn grab_button_held(&self) -> bool {
        self.grab_down
            || (self.primary_down && self.lmb_grab_armed)
            || self.keys_down.contains(&KeyCode::KeyE)
    }

    fn aim_highlight(&self) -> Option<u32> {
        if let Some(id) = self.held_grab {
            return Some(id);
        }
        if !self.look_captured || self.ui.state.open {
            return None;
        }
        let (eye, look) = self.eye_and_look();
        let ray = desktop_look_ray(eye, look);
        let (boxes, defs) = self.grabbable_boxes();
        pick_grabbable_desktop(ray, &boxes, &defs.half_extents).map(|h| h.entity_id)
    }

    fn on_cursor_moved_look(&mut self, x: f64, y: f64) {
        if !self.look_captured || self.ui.state.open || self.device_look_active {
            self.cursor = (x, y);
            return;
        }
        if self.skip_cursor_warp {
            self.skip_cursor_warp = false;
            self.last_cursor = Some((x, y));
            self.cursor = (x, y);
            return;
        }
        if let Some((lx, ly)) = self.last_cursor {
            self.apply_look_delta(x - lx, y - ly);
        }
        self.last_cursor = Some((x, y));
        self.cursor = (x, y);
        self.warp_cursor_center(true);
    }

    fn eye_and_look(&self) -> (EntityPose, CameraLook) {
        // Display snap (x0092): Redraw path uses begin_display_frame cache; mid-frame
        // rebuild_intent reuses that cache (or ensure freeze if cache was cleared).
        let eye = self.shared.user_root_pose();
        let look = self.shared.look.lock().ok().map(|g| *g).unwrap_or_default();
        (eye, look)
    }

    fn grabbable_boxes(
        &self,
    ) -> (
        Vec<(u32, EntityPose)>,
        std::sync::Arc<crate::scene::BoxDefs>,
    ) {
        // Display TRS (epoch α twin) so OBB matches mesh/box draw (x0091).
        self.shared.grabbable_display_poses()
    }

    /// Update held Grab from LMB/RMB/E + look-forward pick (x0013).
    fn update_grab_hold(&mut self) {
        if self.ui.state.open || !self.look_captured {
            if self.held_grab.take().is_some() {
                log::info!("desktop grab released (dash/look)");
            }
            return;
        }
        if self.grab_button_held() {
            if self.held_grab.is_none() {
                let (eye, look) = self.eye_and_look();
                let ray = desktop_look_ray(eye, look);
                let (boxes, defs) = self.grabbable_boxes();
                if let Some(hit) = pick_grabbable_desktop(ray, &boxes, &defs.half_extents) {
                    self.held_grab = Some(hit.entity_id);
                    log::info!("desktop grab begin entity={}", hit.entity_id);
                } else {
                    let now = std::time::Instant::now();
                    let should_log = self
                        .last_grab_miss_log
                        .map(|t| now.duration_since(t).as_secs_f32() > 1.0)
                        .unwrap_or(true);
                    if should_log {
                        self.last_grab_miss_log = Some(now);
                        log::info!(
                            "desktop grab miss — look down at a floor box until it brightens, then hold LMB/RMB/E"
                        );
                    }
                }
            }
        } else if self.held_grab.take().is_some() {
            log::info!("desktop grab released");
        }
    }

    fn rebuild_intent(&mut self) {
        if !self.owns_input() {
            // XR owns Intent while VR-present. Handoff zeros Intent in
            // `on_input_ownership_edge` / XR edge — do not write here.
            return;
        }
        self.update_grab_hold();

        let dash_open = self.ui.state.open;
        let (eye, look) = self.eye_and_look();
        let yaw = look.yaw;
        let forward_x = yaw.sin();
        let forward_z = -yaw.cos();
        let right_x = yaw.cos();
        let right_z = yaw.sin();

        let mut intent = MoveIntent {
            suppress: dash_open,
            ..Default::default()
        };

        if !dash_open {
            let mut local_x = 0.0_f32;
            let mut local_z = 0.0_f32;
            let mut local_y = 0.0_f32;

            if self.keys_down.contains(&KeyCode::KeyW) {
                local_z += 1.0;
            }
            if self.keys_down.contains(&KeyCode::KeyS) {
                local_z -= 1.0;
            }
            if self.keys_down.contains(&KeyCode::KeyD) {
                local_x += 1.0;
            }
            if self.keys_down.contains(&KeyCode::KeyA) {
                local_x -= 1.0;
            }
            if self.keys_down.contains(&KeyCode::Space) {
                local_y += 1.0;
            }
            if self.keys_down.contains(&KeyCode::KeyC) {
                local_y -= 1.0;
            }

            intent.move_x = local_x * right_x + local_z * forward_x;
            intent.move_z = local_x * right_z + local_z * forward_z;
            intent.move_y = local_y;

            let horiz = (intent.move_x * intent.move_x + intent.move_z * intent.move_z).sqrt();
            if horiz > 1.0 {
                intent.move_x /= horiz;
                intent.move_z /= horiz;
            }

            let mut buttons = 0u32;
            if self.keys_down.contains(&KeyCode::KeyY) {
                buttons |= input_buttons::PRIMARY;
            }
            if self.keys_down.contains(&KeyCode::KeyT) {
                buttons |= input_buttons::CONTEXT;
            }
            if self.keys_down.contains(&KeyCode::KeyR) {
                buttons |= input_buttons::SECONDARY;
            }
            if self.keys_down.contains(&KeyCode::Space) {
                buttons |= input_buttons::JUMP;
            }
            if let Some(id) = self.held_grab {
                buttons |= input_buttons::GRAB;
                intent.grab_entity_id = id;
                intent.aim_x = eye.x;
                intent.aim_y = eye.y;
                intent.aim_z = eye.z;
            }
            intent.buttons = buttons;
        }

        if let Ok(mut guard) = self.shared.intent.lock() {
            *guard = intent;
        }
    }
}

/// Apply raw mouse delta to look. `dx` right / `dy` down in pixels.
/// Positive `dx` increases yaw (turn right toward +X when looking down −Z).
pub(crate) fn apply_mouse_look(
    look: CameraLook,
    dx: f64,
    dy: f64,
    sensitivity: f32,
    pitch_limit: f32,
) -> CameraLook {
    CameraLook {
        yaw: look.yaw + (dx as f32) * sensitivity,
        pitch: (look.pitch - (dy as f32) * sensitivity).clamp(-pitch_limit, pitch_limit),
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        if self.blank_cursor.is_none() {
            match CustomCursor::from_rgba(vec![0, 0, 0, 0], 1, 1, 0, 0) {
                Ok(source) => {
                    self.blank_cursor = Some(event_loop.create_custom_cursor(source));
                }
                Err(err) => log::warn!("blank cursor rgba failed: {err}"),
            }
        }
        let attrs = Window::default_attributes()
            .with_title(self.window_title())
            .with_inner_size(LogicalSize::new(960.0, 540.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let gpu = pollster::block_on(DesktopGpu::new(
            Arc::clone(&window),
            &self.mesh,
            Arc::clone(&self.shared),
        ))
        .expect("init gpu");
        self.ui
            .init_gpu(
                &gpu.scene.device,
                &gpu.scene.queue,
                gpu.scene.surface_format,
                gpu.config.width,
                gpu.config.height,
            )
            .expect("init system_ui gpu");
        self.window = Some(window);
        self.gpu = Some(gpu);
        self.sync_hotswitch_ui();
        self.ui.rebuild();
        log::info!(
            "system_ui ready — Esc dash; F8 hotswitch (XR companion); click to look; Grab LMB/RMB/E; WASD+Space+C"
        );
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        if let DeviceEvent::MouseMotion { delta } = event {
            if self.look_captured && !self.ui.state.open {
                self.device_look_active = true;
                self.apply_look_delta(delta.0, delta.1);
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.shared.presentation.request_exit();
                event_loop.exit();
            }
            WindowEvent::Focused(false) => {
                self.set_look_capture(false);
            }
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(size.width, size.height);
                    self.ui.resize(&gpu.scene.device, size.width, size.height);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if self.ui.state.open {
                    self.cursor = (position.x, position.y);
                    self.ui.on_pointer(
                        self.cursor.0 as f32,
                        self.cursor.1 as f32,
                        self.primary_down,
                    );
                } else if self.look_captured {
                    self.on_cursor_moved_look(position.x, position.y);
                } else {
                    self.cursor = (position.x, position.y);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if self.ui.state.open {
                    let dy = match delta {
                        MouseScrollDelta::LineDelta(_, y) => -y * 48.0,
                        MouseScrollDelta::PixelDelta(p) => -p.y as f32,
                    };
                    self.ui
                        .on_scroll(self.cursor.0 as f32, self.cursor.1 as f32, dy);
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let pressed = state == ElementState::Pressed;
                self.primary_down = pressed;
                if self.ui.state.open {
                    self.ui.on_pointer(
                        self.cursor.0 as f32,
                        self.cursor.1 as f32,
                        self.primary_down,
                    );
                    if self.primary_down {
                        let action = self.ui.on_click(self.cursor.0 as f32, self.cursor.1 as f32);
                        handle_ui_action(
                            action,
                            &mut self.ui,
                            &self.shared,
                            &mut self.zenoh,
                            &mut self.auth_job,
                            RoomBindDefer {
                                pending: &mut self.pending_room_bind,
                                painted: &mut self.room_bind_progress_painted,
                            },
                            event_loop,
                        );
                    }
                } else if pressed && !self.look_captured && self.owns_input() {
                    self.set_look_capture(true);
                } else if !pressed && self.look_captured {
                    // Arm Grab on LMB only after the capture click is released.
                    self.lmb_grab_armed = true;
                }
                self.rebuild_intent();
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Right,
                ..
            } => {
                if self.owns_input() {
                    self.grab_down = state == ElementState::Pressed;
                }
                self.rebuild_intent();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state == ElementState::Pressed;
                if let PhysicalKey::Code(code) = event.physical_key {
                    if pressed {
                        self.keys_down.insert(code);
                    } else {
                        self.keys_down.remove(&code);
                    }
                    if pressed && !event.repeat && code == KeyCode::F8 {
                        self.toggle_hotswitch_manual();
                    }
                    if pressed && !event.repeat && code == KeyCode::Escape {
                        self.ui.toggle();
                        if self.ui.state.open {
                            self.set_look_capture(false);
                            self.held_grab = None;
                            self.grab_down = false;
                            self.ui.clear_pointer();
                        }
                        log::info!("system_ui open={}", self.ui.state.open);
                    }
                    let dash_text = self.ui.state.open
                        && matches!(self.ui.state.tab, DashTab::Home | DashTab::Settings);
                    if pressed && dash_text {
                        match code {
                            KeyCode::Backspace => {
                                self.ui.on_home_backspace();
                            }
                            KeyCode::Tab if self.ui.state.tab == DashTab::Home => {
                                self.ui.on_home_tab_key();
                            }
                            KeyCode::Enter | KeyCode::NumpadEnter
                                if !event.repeat && self.ui.state.tab == DashTab::Home =>
                            {
                                let action = self.ui.on_home_enter();
                                handle_ui_action(
                                    action,
                                    &mut self.ui,
                                    &self.shared,
                                    &mut self.zenoh,
                                    &mut self.auth_job,
                                    RoomBindDefer {
                                        pending: &mut self.pending_room_bind,
                                        painted: &mut self.room_bind_progress_painted,
                                    },
                                    event_loop,
                                );
                            }
                            _ => {}
                        }
                    }
                }
                if pressed {
                    if let Some(text) = event.text.as_ref() {
                        let dash_text = self.ui.state.open
                            && matches!(self.ui.state.tab, DashTab::Home | DashTab::Settings);
                        if dash_text {
                            // Skip control-ish single chars already handled as keys.
                            if text != "\u{8}" && text != "\t" && text != "\r" && text != "\n" {
                                self.ui.on_text_input(text);
                            }
                        }
                    }
                }
                // Mid-frame Grab: `ensure_display_frame` reuses last Redraw freeze
                // (what is still on screen) until the next `begin_display_frame`.
                self.rebuild_intent();
            }
            WindowEvent::RedrawRequested => {
                self.ui.pull_dash_logic(&self.shared);
                let hier = self.ui.pull_hierarchy(&self.shared);
                let xray = self.ui.pull_epoch_xray(&self.shared, false);
                if hier || xray {
                    self.ui.rebuild();
                }
                {
                    let tick = self
                        .shared
                        .world
                        .lock()
                        .ok()
                        .and_then(|w| w.last_frame_tick);
                    let now = std::time::Instant::now();
                    let mut dirty = false;
                    let stale = self.shared.key_stale();
                    if self.ui.state.worlds_key_stale != stale {
                        self.ui.state.worlds_key_stale = stale;
                        if stale {
                            self.ui.state.worlds_status =
                                "Room updated — reconnect required".into();
                            self.ui.state.tab = DashTab::Home;
                            self.ui.state.shell = OnboardingShell::Dash;
                            self.ui.state.open = true;
                        }
                        dirty = true;
                    }
                    if poll_session_enter_wait_details(&mut self.ui, &self.shared) {
                        dirty = true;
                    }
                    if let Some(t) = tick {
                        if self.ui.state.worlds_seen_frame_tick != Some(t) {
                            self.ui.state.worlds_seen_frame_tick = Some(t);
                            self.ui.state.worlds_last_frame_at = Some(now);
                            if self.ui.state.worlds_waiting_host
                                || self.ui.state.session_enter_active
                            {
                                self.ui.state.worlds_waiting_host = false;
                                self.ui.state.worlds_host_disconnect = false;
                                self.ui.state.worlds_status = "Live — host frames arriving".into();
                                self.ui.state.home_status = "Live — host frames arriving".into();
                                if self.ui.state.session_enter_active {
                                    let waited = self
                                        .ui
                                        .state
                                        .session_enter_wait_since
                                        .map(|t0| now.duration_since(t0).as_secs())
                                        .unwrap_or(0);
                                    self.ui.state.set_session_enter_progress(1.0);
                                    self.ui.state.push_session_enter_log(format!(
                                        "Live — first Frame tick={t} after {waited}s"
                                    ));
                                    self.ui.state.end_session_enter();
                                    self.ui.state.open = false;
                                }
                                dirty = true;
                            }
                        }
                    }
                    // Live stall / recovery: host skips empty Frame deltas without advancing
                    // net_tick (x0024). Presence/Manifest keepalive must count as alive.
                    const FRAME_STALL: std::time::Duration = std::time::Duration::from_secs(10);
                    if self.ui.state.worlds_waiting_host
                        && self.ui.state.worlds_host_disconnect
                        && self
                            .shared
                            .last_host_alive_at()
                            .is_some_and(|at| now.duration_since(at) < FRAME_STALL)
                    {
                        self.ui.state.worlds_waiting_host = false;
                        self.ui.state.worlds_host_disconnect = false;
                        self.ui.state.worlds_status = "Live — host signals arriving".into();
                        self.ui.state.home_status = "Live — host signals arriving".into();
                        dirty = true;
                    }
                    if self.ui.state.worlds_in_room
                        && !self.ui.state.worlds_waiting_host
                        && self.ui.state.worlds_seen_frame_tick.is_some()
                        && self
                            .shared
                            .last_host_alive_at()
                            .is_some_and(|at| now.duration_since(at) > FRAME_STALL)
                    {
                        // One-shot recovery: Presence may be invisible if RX stalled; still
                        // re-bind Intent + liveliness so a transient liveliness Delete can heal.
                        self.shared.request_intent_rebind();
                        self.ui.state.worlds_waiting_host = true;
                        self.ui.state.worlds_host_disconnect = true;
                        self.ui.state.worlds_status =
                            "Host connection lost — wait for return or Leave".into();
                        self.ui.state.home_status =
                            "Host connection lost — wait for return or Leave".into();
                        // Inventory is Live-only (x007f): stall → Waiting drops Inventory/Home/
                        // Worlds to Home so the session shell is visible. Settings may stay.
                        if matches!(
                            self.ui.state.tab,
                            DashTab::Inventory | DashTab::Home | DashTab::Worlds
                        ) {
                            self.ui.state.tab = DashTab::Home;
                        }
                        self.ui.state.shell = OnboardingShell::Dash;
                        self.ui.state.open = true;
                        dirty = true;
                    }
                    kick_waiting_host_session_poll(
                        &mut self.ui,
                        &self.shared,
                        &mut self.waiting_session_job,
                        now,
                    );
                    let (session_dirty, auto_nominated) = apply_waiting_host_session_poll(
                        &mut self.ui,
                        &self.shared,
                        &mut self.waiting_session_job,
                    );
                    if session_dirty {
                        dirty = true;
                    }
                    if auto_nominated {
                        let room_id = self.shared.get_room_id().to_string();
                        start_worlds_host(
                            &mut self.ui,
                            &self.shared,
                            &mut self.zenoh,
                            &mut self.auth_job,
                            room_id,
                            HostStartKind::Nominated,
                        );
                        dirty = true;
                    }
                    if dirty {
                        self.ui.mark_dash_logic_dirty();
                        self.ui.rebuild();
                    }
                }
                self.on_input_ownership_edge();
                self.refresh_window_title();
                self.sync_hotswitch_ui();
                self.poll_presentation_requests();
                self.poll_auth_job();
                self.drain_inventory_peer_actions(event_loop);
                self.poll_mtls_reopen();
                // Freeze display α/PrevCurr before Grab + GPU sync (x0091 同一 α).
                self.shared.begin_display_frame();
                if self.owns_input() {
                    self.maintain_look_capture();
                    self.rebuild_intent();
                } else if self.look_captured {
                    self.set_look_capture(false);
                }
                // Mirror while VR-active is intentional (non-guarantee GPU cost).
                let highlight = if self.owns_input() {
                    self.aim_highlight()
                } else {
                    None
                };
                let rendered = if let Some(gpu) = self.gpu.as_mut() {
                    match gpu.render(&mut self.ui, highlight) {
                        Ok(()) => true,
                        Err(err) => {
                            log::error!("render failed: {err:#}");
                            false
                        }
                    }
                } else {
                    false
                };
                // Only after a successful paint — never skip the Binding frame on gpu miss/fail.
                if rendered && self.pending_room_bind.is_some() {
                    self.room_bind_progress_painted = true;
                }
                self.ui.push_dash_logic(&self.shared);
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.poll_presentation_requests();
        if self.shared.presentation.exit_requested() {
            event_loop.exit();
            return;
        }
        self.on_input_ownership_edge();
        self.poll_auth_job();
        self.drain_inventory_peer_actions(event_loop);
        // After Join/Host ticket: bind only once "Binding room bus…" has painted.
        self.poll_pending_room_bind();
        self.poll_mtls_reopen();
        self.apply_pending_world_mesh();
        // x0048: re-apply sticky asset failure after Login/Start/Join settle (skips "…").
        if self
            .shared
            .overlay_asset_home_status(&mut self.ui.state.home_status)
        {
            self.ui.state.open = true;
            self.ui.state.tab = DashTab::Home;
            self.ui.mark_dash_logic_dirty();
            // Publish before next Redraw so XR cannot interleave an older push.
            self.ui.push_dash_logic(&self.shared);
        }
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

impl App {
    /// Manifest remount / Leave empty shell (x007d).
    fn apply_pending_world_mesh(&mut self) {
        let Some(mesh) = self.shared.pending_mesh_if_newer(&mut self.mesh_epoch) else {
            return;
        };
        self.mesh = mesh;
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.scene.replace_world_mesh(&self.mesh);
        }
    }

    /// Run deferred bind only after "Binding room bus…" has painted once.
    fn poll_pending_room_bind(&mut self) {
        let Some(pending) = self.pending_room_bind.take() else {
            return;
        };
        if !self.room_bind_progress_painted {
            self.pending_room_bind = Some(pending);
            return;
        }
        self.room_bind_progress_painted = false;
        complete_pending_room_bind(&mut self.ui, &self.shared, &mut self.zenoh, pending);
    }
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

fn complete_pending_room_bind(
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
                ui.state.worlds_nominated_host = false;
                ui.state.worlds_can_takeover = true;
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
    /// Already nominated via handover (`active_host_sub == self`).
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
            HostStartKind::Nominated => Ok(()),
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
fn start_worlds_catalog_refresh(
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

/// Map XR peer request → Desktop `UiAction`, applying XR selection into local state.
fn apply_inventory_peer_request(
    ui: &mut SystemUi,
    shared: &SharedState,
    req: &InventoryPeerRequest,
) -> Option<UiAction> {
    match req {
        InventoryPeerRequest::Open => Some(UiAction::InventoryOpen),
        InventoryPeerRequest::Refresh => Some(UiAction::InventoryRefresh),
        InventoryPeerRequest::TakeOut { node_id, kind } => {
            // Override carries ids without injecting a ghost Drive row into the list/mirror.
            ui.state.inventory_take_out_override = Some((node_id.clone(), kind.clone()));
            if let Some(i) = ui
                .state
                .inventory_nodes
                .iter()
                .position(|(id, _, _)| id == node_id)
            {
                ui.state.inventory_selected = Some(i);
            }
            Some(UiAction::InventoryTakeOut)
        }
        InventoryPeerRequest::PutBack { entity_ids } => {
            ui.state.scene_meshes = shared.scene_meshes();
            ui.state.scene_selected = rematch_scene_selection(&ui.state.scene_meshes, entity_ids);
            ui.state.inventory_selected = None;
            if ui.state.scene_selected.is_empty() {
                ui.state.inventory_status =
                    "XR put-back: selected meshes not in scene (Refresh / wait Manifest)".into();
                publish_inventory_mirror_from_ui(ui, shared);
                ui.rebuild();
                return None;
            }
            Some(UiAction::InventoryPutBack)
        }
        InventoryPeerRequest::Despawn { entity_id } => {
            ui.state.scene_meshes = shared.scene_meshes();
            ui.state.scene_selected =
                rematch_scene_selection(&ui.state.scene_meshes, &[*entity_id]);
            ui.state.inventory_selected = None;
            if ui.state.scene_selected.is_empty() {
                ui.state.inventory_status =
                    "XR despawn: entity not in scene (Refresh / wait Manifest)".into();
                publish_inventory_mirror_from_ui(ui, shared);
                ui.rebuild();
                return None;
            }
            Some(UiAction::InventoryDespawn)
        }
    }
}

fn publish_inventory_mirror_from_ui(ui: &SystemUi, shared: &SharedState) {
    shared.publish_inventory_mirror(
        ui.state.inventory_nodes.clone(),
        ui.state.inventory_status.clone(),
    );
}

fn inventory_status_rebuild(ui: &mut SystemUi, shared: &SharedState, status: impl Into<String>) {
    ui.state.inventory_status = status.into();
    publish_inventory_mirror_from_ui(ui, shared);
    ui.rebuild();
}

fn shared_has_host_inventory_token(shared: &SharedState) -> bool {
    shared
        .auth
        .lock()
        .ok()
        .is_some_and(|g| g.host_room_token.is_some())
}

/// Soft UI gate for Take-out/Despawn (x0098). Final auth is always on the compute host.
///
/// Allows when Intent is live and either:
/// - `host_room_token` looks like `role=host` (compute-host Dual path), or
/// - catalog `owner_sub` for the current room matches `local_sub` (owner Dual path), or
/// - catalog has **no** owner row (`catalog_owner_sub == None`) — **intentionally does not
///   early-reject** (Advanced fixed `ROOM_ID`, catalog not loaded, or row missing). Host Dual
///   remains the authority; non-owner may still reach Drive/Zenoh until host rejects.
///
/// Rejects early only when catalog **positively** says another owner and there is no host
/// Token, so known non-owner guests do not burn Drive take_out + Zenoh puts first.
///
/// `intent_bind_pending`: `None` = no Intent session / lock failed; `Some(true)` = bind
/// still in progress; `Some(false)` = session present and not pending.
fn inventory_control_gate_status(
    access: &Option<String>,
    intent_bind_pending: Option<bool>,
    host_room_token: &Option<String>,
    local_sub: Option<&str>,
    catalog_owner_sub: Option<&str>,
) -> Option<&'static str> {
    match (access.as_ref(), intent_bind_pending) {
        (None, _) => return Some("sign in required"),
        (Some(_), None) => return Some("Intent session required (join/bind first)"),
        (Some(_), Some(true)) => return Some("Intent bind in progress — wait a moment"),
        _ => {}
    }
    let host_role = host_room_token.as_deref().is_some_and(|t| {
        crate::inventory::room_token_role_unverified(t).as_deref() == Some("host")
    });
    if host_role {
        return None;
    }
    match (local_sub, catalog_owner_sub) {
        (Some(local), Some(owner)) if local == owner => None,
        // Catalog miss / Advanced ROOM_ID: do not early-reject (host Dual decides).
        (_, None) => None,
        _ => Some("owner or host role required for take-out/despawn"),
    }
}

fn catalog_owner_sub_for_room(ui: &SystemUi, room_id: &str) -> Option<String> {
    ui.state
        .worlds_rooms
        .iter()
        .find(|(id, ..)| id == room_id)
        .map(|(_, _, _, owner)| owner.clone())
        .filter(|s| !s.is_empty())
}

/// Host `SpawnRate(8/10s)` — client spaces puts so host does not silently reject.
const HOST_SPAWN_MAX_PER_WINDOW: u32 = 8;
const HOST_SPAWN_WINDOW_MS: u64 = 10_000;

/// HTTP take_out + paced Zenoh puts off the UI thread (x007f).
#[allow(clippy::too_many_arguments)]
fn run_paced_inventory_take_out(
    shared: &SharedState,
    epoch: u64,
    tx: &std::sync::mpsc::Sender<AuthJobOutcome>,
    session: &zenoh::Session,
    assets: &str,
    access: &str,
    room_id: &str,
    wire_cap: &str,
    room_key: &[u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN],
    room_key_epoch: u64,
    node_id: &str,
) -> String {
    let blobs = match crate::inventory::take_out_blobs(assets, access, node_id) {
        Ok((_node, blobs)) => blobs,
        Err(err) => {
            log::warn!("inventory take_out: {err:#}");
            return format!("take_out failed: {err:#}");
        }
    };
    let total = blobs.len();
    if total == 0 {
        return "take_out returned 0 blobs".into();
    }
    let gap_ms = HOST_SPAWN_WINDOW_MS / u64::from(HOST_SPAWN_MAX_PER_WINDOW);
    let gap = std::time::Duration::from_millis(gap_ms);
    let mut spawned = 0usize;
    for (i, blob) in blobs.iter().enumerate() {
        if !shared.auth_work_is_current(epoch) {
            return format!("spawn put aborted {spawned}/{total} (left/logout)");
        }
        if i > 0 {
            std::thread::sleep(gap);
            if !shared.auth_work_is_current(epoch) {
                return format!("spawn put aborted {spawned}/{total} (left/logout)");
            }
        }
        let (x, y, z, qx, qy, qz, qw, sx, sy, sz) = if let Some(trs) = blob.trs.as_ref() {
            (
                trs.translation[0],
                trs.translation[1],
                trs.translation[2],
                trs.rotation[0],
                trs.rotation[1],
                trs.rotation[2],
                trs.rotation[3],
                trs.scale[0],
                trs.scale[1],
                trs.scale[2],
            )
        } else {
            // Default pose with slight X offset so children do not stack.
            (i as f64 * 0.35, 1.5, 2.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0)
        };
        let control = {
            let Ok(mut gate) = shared.intent_session.lock() else {
                return format!("spawn put {spawned}/{total} then failed: intent_session lock");
            };
            let Some(intent) = gate.as_mut() else {
                return format!(
                    "spawn put {spawned}/{total} then failed: no Intent session (bind first)"
                );
            };
            intent.sign_control_spawn(room_id, &blob.blake3, [x, y, z, qx, qy, qz, qw, sx, sy, sz])
        };
        match crate::inventory::publish_inventory_spawn_pose(
            session,
            room_id,
            wire_cap,
            room_key,
            room_key_epoch,
            &control,
            &blob.blake3,
            x,
            y,
            z,
            qx,
            qy,
            qz,
            qw,
            sx,
            sy,
            sz,
        ) {
            Ok(()) => {
                spawned += 1;
                if shared.auth_work_is_current(epoch) {
                    let _ = tx.send(AuthJobOutcome::InventoryTakeOutProgress {
                        status: format!(
                            "spawn put {spawned}/{total} (paced ≤{HOST_SPAWN_MAX_PER_WINDOW}/{HOST_SPAWN_WINDOW_MS}ms)"
                        ),
                    });
                }
            }
            Err(err) => {
                // Leave/close often races the put; prefer abort copy when epoch already bumped.
                if !shared.auth_work_is_current(epoch) {
                    return format!("spawn put aborted {spawned}/{total} (left/logout)");
                }
                log::warn!("inventory spawn: {err:#}");
                return format!("spawn put {spawned}/{total} then failed: {err:#}");
            }
        }
    }
    format!(
        "spawn put {spawned}/{total} (paced ≤{HOST_SPAWN_MAX_PER_WINDOW}/{HOST_SPAWN_WINDOW_MS}ms; host may still reject)"
    )
}

fn handle_ui_action(
    action: UiAction,
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    zenoh: &mut Option<zenoh::Session>,
    auth_job: &mut Option<(u64, Receiver<AuthJobOutcome>)>,
    mut room_bind: RoomBindDefer<'_>,
    event_loop: &ActiveEventLoop,
) {
    match action {
        UiAction::HotswitchAutoChanged => {
            shared
                .presentation
                .set_hotswitch_auto(ui.state.hotswitch_auto);
            // Clearing suppress lets the user re-enable auto after a bad sensor episode.
            if ui.state.hotswitch_auto {
                shared
                    .presentation
                    .auto_suppressed
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                ui.state.hotswitch_suppressed = false;
                ui.state.refresh_hotswitch_label();
            }
            ui.rebuild();
            log::info!("hotswitch auto stored → {}", ui.state.hotswitch_auto);
        }
        UiAction::ServerOperatorChanged => {
            let mut prefs = operator_prefs::load();
            prefs.server_operator = ui.state.server_operator;
            operator_prefs::save(&prefs);
            log::info!("server operator prefs → {}", ui.state.server_operator);
        }
        UiAction::ExitClientOnly | UiAction::ExitClientAndHost => {
            // Exit Client only abolished: stop supervisor-owned host only.
            log::info!("system_ui: exit (handover stub + stop supervised host if any)");
            exit_stop_supervised_host(shared);
            shared.presentation.request_exit();
            event_loop.exit();
        }
        UiAction::ApplyOperatorEndpoints => {
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
            let mut prefs = operator_prefs::load();
            prefs.operator_chosen = true;
            prefs.operator = ui.state.operator;
            prefs.custom_host = ui.state.custom_host.clone();
            prefs.apply_endpoints(&ep);
            // Keep any existing refresh until login replaces it.
            operator_prefs::save(&prefs);
            log::info!(
                "operator endpoints applied: auth={} world={} zenoh={}",
                ep.auth_url,
                ep.world_url,
                ep.zenoh_connect
            );
        }
        UiAction::AuthLogin => {
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
        UiAction::AuthRegister => {
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
        UiAction::AuthLogout => {
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
            let mut prefs = operator_prefs::load();
            prefs.clear_session();
            operator_prefs::save(&prefs);
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
        UiAction::StartSession => {
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
                            join_room_with_access(&shared_job, &access, epoch)
                                .map_err(|e| format!("{e:#}"))
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
        UiAction::JoinRoom => {
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
                let result = join_room_with_access(&shared_job, &access, epoch)
                    .map_err(|e| format!("{e:#}"));
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
        UiAction::WorldsRefresh => {
            start_worlds_catalog_refresh(ui, shared, auth_job);
        }
        UiAction::WorldsJoinGuest => {
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
                let result = join_room_with_access(&shared_job, &access, epoch)
                    .map_err(|e| format!("{e:#}"));
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
        UiAction::WorldsHostRoom => {
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
        UiAction::WorldsHostRescue => {
            // Ignore catalog selection — always the actually joined room.
            if !ui.state.server_operator {
                ui.state.worlds_status =
                    "Enable Server Operator in Settings to host this room".into();
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
        UiAction::WorldsTakeover => {
            if !ui.state.server_operator {
                ui.state.worlds_status =
                    "Enable Server Operator in Settings to take over host".into();
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
        UiAction::WorldsLeave => {
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
        UiAction::WorldsRejoin => {
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
        UiAction::InventoryRefresh => {
            if ui.state.inventory_blocked() {
                ui.state.tab = DashTab::Home;
                ui.state.home_status = "Inventory is available when Live (x007f)".into();
                ui.mark_dash_chrome_dirty();
                ui.rebuild();
                return;
            }
            let assets = shared.get_assets_url();
            let token = shared.auth.lock().ok().and_then(|g| g.access_token.clone());
            let prev_scene: Vec<u32> = ui
                .state
                .scene_selected
                .iter()
                .filter_map(|&i| ui.state.scene_meshes.get(i).map(|(eid, _)| *eid))
                .collect();
            ui.state.scene_meshes = shared.scene_meshes();
            // Keep Desktop/XR scene multi-select across Drive Refresh (XR Open stacks Refresh).
            ui.state.scene_selected = rematch_scene_selection(&ui.state.scene_meshes, &prev_scene);
            match token {
                Some(access) => match crate::inventory::list_drive_nodes(&assets, &access) {
                    Ok(nodes) => {
                        ui.state.inventory_nodes = nodes
                            .into_iter()
                            .map(|n| (n.node_id, n.kind, n.name))
                            .collect();
                        ui.state.inventory_selected = None;
                        ui.state.inventory_status = format!(
                            "{} drive / {} scene",
                            ui.state.inventory_nodes.len(),
                            ui.state.scene_meshes.len()
                        );
                    }
                    Err(err) => {
                        ui.state.inventory_status = format!("refresh failed: {err:#}");
                        log::warn!("inventory refresh: {err:#}");
                    }
                },
                None => {
                    ui.state.inventory_status = "sign in required".into();
                }
            }
            publish_inventory_mirror_from_ui(ui, shared);
            ui.rebuild();
        }
        UiAction::InventoryTakeOut => {
            // Consume XR override immediately so busy/blocked early returns cannot
            // leak it into a later Desktop TakeOut (re-fire from XR to retry).
            let override_target = ui.state.inventory_take_out_override.take();
            if ui.state.inventory_blocked() {
                ui.state.tab = DashTab::Home;
                ui.state.home_status = "Inventory is available when Live (x007f)".into();
                ui.mark_dash_chrome_dirty();
                ui.rebuild();
                return;
            }
            if auth_job.is_some() {
                inventory_status_rebuild(ui, shared, "busy — wait for auth/inventory job");
                return;
            }
            let selected = override_target.or_else(|| {
                ui.state.inventory_selected.and_then(|i| {
                    ui.state
                        .inventory_nodes
                        .get(i)
                        .map(|(id, kind, _)| (id.clone(), kind.clone()))
                })
            });
            let Some((node_id, kind)) = selected else {
                inventory_status_rebuild(ui, shared, "select a blob or prefab node first");
                return;
            };
            if kind != "blob" && kind != "prefab" {
                inventory_status_rebuild(ui, shared, "x007f: blob/prefab nodes only");
                return;
            }
            let assets = shared.get_assets_url();
            let room_id_for_gate = shared.get_room_id();
            let catalog_owner = catalog_owner_sub_for_room(ui, room_id_for_gate.as_ref());
            let (access, host_room_token, local_sub) = {
                let Ok(auth) = shared.auth.lock() else {
                    inventory_status_rebuild(ui, shared, "auth lock failed");
                    return;
                };
                (
                    auth.access_token.clone(),
                    auth.host_room_token.clone(),
                    auth.local_sub.clone(),
                )
            };
            let intent_bind_pending = shared
                .intent_session
                .lock()
                .ok()
                .and_then(|slot| slot.as_ref().map(|s| s.bind_pending()));
            if let Some(msg) = inventory_control_gate_status(
                &access,
                intent_bind_pending,
                &host_room_token,
                local_sub.as_deref(),
                catalog_owner.as_deref(),
            ) {
                inventory_status_rebuild(ui, shared, msg);
                return;
            }
            let Some(access) = access else {
                inventory_status_rebuild(ui, shared, "sign in required");
                return;
            };
            // x0098: session-signed; host accepts owner_sub or host-role bind.
            let Some(session) = zenoh.as_ref().cloned() else {
                inventory_status_rebuild(ui, shared, "zenoh not connected");
                return;
            };
            let Some(wire_cap) = shared.wire_cap() else {
                inventory_status_rebuild(ui, shared, "no wire_cap");
                return;
            };
            let Some((room_key, room_key_epoch)) = shared.live_room_aead() else {
                inventory_status_rebuild(ui, shared, "room_key missing");
                return;
            };
            let room_id = shared.get_room_id().to_string();
            let wire_cap = wire_cap.to_string();
            let shared_job = Arc::clone(shared);
            ui.state.inventory_status = "take_out…".into();
            publish_inventory_mirror_from_ui(ui, shared);
            ui.rebuild();
            let (tx, rx) = mpsc::channel();
            let epoch = arm_auth_job(shared, auth_job, rx);
            spawn_auth_http(move || {
                let status = run_paced_inventory_take_out(
                    &shared_job,
                    epoch,
                    &tx,
                    &session,
                    &assets,
                    &access,
                    &room_id,
                    &wire_cap,
                    room_key.as_ref(),
                    room_key_epoch,
                    &node_id,
                );
                if shared_job.auth_work_is_current(epoch) {
                    let _ = tx.send(AuthJobOutcome::InventoryTakeOutDone { status });
                }
            });
        }
        UiAction::InventoryOpen => {
            if ui.state.inventory_blocked() {
                ui.state.tab = DashTab::Home;
                ui.state.home_status = "Inventory is available when Live (x007f)".into();
                ui.mark_dash_chrome_dirty();
                ui.mark_dash_form_dirty();
                ui.rebuild();
                return;
            }
            // Scene list is Manifest-derived; refresh without Drive HTTP (x006e).
            let prev_scene: Vec<u32> = ui
                .state
                .scene_selected
                .iter()
                .filter_map(|&i| ui.state.scene_meshes.get(i).map(|(eid, _)| *eid))
                .collect();
            ui.state.scene_meshes = shared.scene_meshes();
            ui.state.scene_selected = rematch_scene_selection(&ui.state.scene_meshes, &prev_scene);
            publish_inventory_mirror_from_ui(ui, shared);
            ui.rebuild();
        }
        UiAction::InventoryPutBack => {
            if ui.state.inventory_blocked() {
                ui.state.tab = DashTab::Home;
                ui.state.home_status = "Inventory is available when Live (x007f)".into();
                ui.mark_dash_chrome_dirty();
                ui.rebuild();
                return;
            }
            // Always re-sync from SharedState so Put back sees post-spawn Manifest (x006e).
            let prev_entities: Vec<u32> = ui
                .state
                .scene_selected
                .iter()
                .filter_map(|&i| ui.state.scene_meshes.get(i).map(|(eid, _)| *eid))
                .collect();
            ui.state.scene_meshes = shared.scene_meshes();
            ui.state.scene_selected = prev_entities
                .iter()
                .filter_map(|eid| ui.state.scene_meshes.iter().position(|(id, _)| id == eid))
                .collect();
            if ui.state.scene_selected.is_empty() {
                inventory_status_rebuild(
                    ui,
                    shared,
                    "select one or more scene meshes first (Refresh if empty)",
                );
                return;
            }
            let children: Vec<crate::inventory::PrefabPutBackChild> = {
                let world = shared.world.lock().ok();
                ui.state
                    .scene_selected
                    .iter()
                    .filter_map(|&i| {
                        let (entity_id, blake3) = ui.state.scene_meshes.get(i)?;
                        let trs = world.as_ref().and_then(|w| {
                            w.entities.get(entity_id).map(|p| {
                                (
                                    [p.x as f64, p.y as f64, p.z as f64],
                                    [p.qx as f64, p.qy as f64, p.qz as f64, p.qw as f64],
                                    [p.sx as f64, p.sy as f64, p.sz as f64],
                                )
                            })
                        });
                        Some((format!("mesh_{entity_id}"), blake3.clone(), trs))
                    })
                    .collect()
            };
            if children.is_empty() {
                inventory_status_rebuild(
                    ui,
                    shared,
                    "select a scene mesh first (Refresh if empty)",
                );
                return;
            }
            let assets = shared.get_assets_url();
            let access = {
                let Ok(auth) = shared.auth.lock() else {
                    inventory_status_rebuild(ui, shared, "auth lock failed");
                    return;
                };
                auth.access_token.clone()
            };
            let Some(access) = access else {
                inventory_status_rebuild(ui, shared, "sign in required");
                return;
            };
            let first_eid = children[0].0.strip_prefix("mesh_").unwrap_or("0");
            let name = format!(
                "putback-n{}-e{first_eid}-{}",
                children.len(),
                &children[0].1[..8.min(children[0].1.len())]
            );
            match crate::inventory::put_back_prefab_children(&assets, &access, &name, &children) {
                Ok(node) => {
                    ui.state.inventory_status = format!(
                        "put-back ok {} ({} children stay in room)",
                        node.name,
                        children.len()
                    );
                    if let Ok(nodes) = crate::inventory::list_drive_nodes(&assets, &access) {
                        ui.state.inventory_nodes = nodes
                            .into_iter()
                            .map(|n| (n.node_id, n.kind, n.name))
                            .collect();
                    }
                    ui.state.scene_meshes = shared.scene_meshes();
                }
                Err(err) => {
                    ui.state.inventory_status = format!("put_back failed: {err:#}");
                    log::warn!("inventory put_back: {err:#}");
                }
            }
            publish_inventory_mirror_from_ui(ui, shared);
            ui.rebuild();
        }
        UiAction::InventoryDespawn => {
            if ui.state.inventory_blocked() {
                ui.state.tab = DashTab::Home;
                ui.state.home_status = "Inventory is available when Live (x007f)".into();
                ui.mark_dash_chrome_dirty();
                ui.rebuild();
                return;
            }
            let prev_entity = ui
                .state
                .scene_selected
                .first()
                .and_then(|&i| ui.state.scene_meshes.get(i).map(|(eid, _)| *eid));
            ui.state.scene_meshes = shared.scene_meshes();
            if let Some(eid) = prev_entity {
                ui.state.scene_selected = ui
                    .state
                    .scene_meshes
                    .iter()
                    .position(|(id, _)| *id == eid)
                    .into_iter()
                    .collect();
            } else {
                ui.state
                    .scene_selected
                    .retain(|&i| i < ui.state.scene_meshes.len());
            }
            let selected = ui.state.scene_selected.first().and_then(|&i| {
                ui.state
                    .scene_meshes
                    .get(i)
                    .map(|(eid, b3)| (*eid, b3.clone()))
            });
            let Some((entity_id, blake3)) = selected else {
                inventory_status_rebuild(
                    ui,
                    shared,
                    "select a scene mesh first (Refresh if empty)",
                );
                return;
            };
            let room_id_for_gate = shared.get_room_id();
            let catalog_owner = catalog_owner_sub_for_room(ui, room_id_for_gate.as_ref());
            let (access, host_room_token, local_sub) = {
                let Ok(auth) = shared.auth.lock() else {
                    inventory_status_rebuild(ui, shared, "auth lock failed");
                    return;
                };
                (
                    auth.access_token.clone(),
                    auth.host_room_token.clone(),
                    auth.local_sub.clone(),
                )
            };
            let intent_bind_pending = shared
                .intent_session
                .lock()
                .ok()
                .and_then(|slot| slot.as_ref().map(|s| s.bind_pending()));
            if let Some(msg) = inventory_control_gate_status(
                &access,
                intent_bind_pending,
                &host_room_token,
                local_sub.as_deref(),
                catalog_owner.as_deref(),
            ) {
                inventory_status_rebuild(ui, shared, msg);
                return;
            }
            let Some(session) = zenoh.as_ref() else {
                inventory_status_rebuild(ui, shared, "zenoh not connected");
                return;
            };
            let Some(wire_cap) = shared.wire_cap() else {
                inventory_status_rebuild(ui, shared, "no wire_cap");
                return;
            };
            let Some((room_key, room_key_epoch)) = shared.live_room_aead() else {
                inventory_status_rebuild(ui, shared, "room_key missing");
                return;
            };
            let room_id = shared.get_room_id();
            let control = {
                let Ok(mut gate) = shared.intent_session.lock() else {
                    inventory_status_rebuild(ui, shared, "intent_session lock failed");
                    return;
                };
                let Some(intent) = gate.as_mut() else {
                    inventory_status_rebuild(
                        ui,
                        shared,
                        "Intent session required (join/bind first)",
                    );
                    return;
                };
                intent.sign_control_despawn(room_id.as_ref(), entity_id)
            };
            match crate::inventory::publish_inventory_despawn(
                session,
                room_id.as_ref(),
                wire_cap.as_ref(),
                room_key.as_ref(),
                room_key_epoch,
                &control,
                entity_id,
            ) {
                Ok(()) => {
                    ui.state.inventory_status = format!(
                        "despawn requested #{entity_id} ({})",
                        &blake3[..8.min(blake3.len())]
                    );
                    ui.state.scene_selected.clear();
                }
                Err(err) => {
                    ui.state.inventory_status = format!("despawn put failed: {err:#}");
                    log::warn!("inventory despawn: {err:#}");
                }
            }
            publish_inventory_mirror_from_ui(ui, shared);
            ui.rebuild();
        }
        UiAction::HierarchyCollapseToggle(row) => {
            ui.apply_hierarchy_collapse_toggle(shared, row);
        }
        UiAction::HierarchyRelayout => {
            ui.apply_hierarchy_relayout(shared);
        }
        UiAction::None => {}
    }
}

pub fn run(
    mesh: CpuMesh,
    shared: Arc<SharedState>,
    zenoh: Option<zenoh::Session>,
    role: DesktopRole,
) -> Result<()> {
    // Dev target/: refresh sibling host-server.exe off the UI/Auth path when stale.
    host_supervisor::spawn_warm_dev_host_bin();
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut ui = SystemUi::new();
    ui.state.home_room_id = shared.get_room_id().to_string();
    let prefs = operator_prefs::load();
    ui.state.operator = prefs.operator;
    ui.state.custom_host = prefs.custom_host.clone();
    ui.state.server_operator = prefs.server_operator;
    ui.state.hotswitch_available = role == DesktopRole::Companion;
    ui.state.hotswitch_auto = shared
        .presentation
        .hotswitch_auto
        .load(std::sync::atomic::Ordering::SeqCst);
    ui.state.hotswitch_suppressed = shared
        .presentation
        .auto_suppressed
        .load(std::sync::atomic::Ordering::SeqCst);
    ui.state.refresh_hotswitch_label();
    let (joined, authenticated, username) = shared
        .auth
        .lock()
        .ok()
        .map(|g| (g.is_joined(), g.is_authenticated(), g.username.clone()))
        .unwrap_or((false, false, None));
    // Boot never auto-joins; `joined` is defensive only (should be false at surface start).
    if joined {
        ui.state.home_logged_in_as = username;
        ui.state.shell = OnboardingShell::Dash;
        ui.state.tab = DashTab::Home;
        ui.state.worlds_in_room = true;
        ui.state.home_status = "In room — use Session for Leave / Host Rescue".into();
        ui.state.worlds_status = "In room — Leave or Host Rescue from Session".into();
    } else {
        ui.state.open = true;
        ui.state.tab = DashTab::Home;
        if authenticated {
            ui.state.home_logged_in_as = username;
            ui.state.shell = OnboardingShell::MainHub;
            ui.state.main_hub_tab = crate::system_ui::MainHubTab::Participation;
            ui.state.home_status = "Signed in — join a public session".into();
        } else if !prefs.operator_chosen {
            ui.state.shell = OnboardingShell::Step1Operator;
            ui.state.home_status = "Select a server operator".into();
        } else {
            ui.state.shell = OnboardingShell::Step2Auth;
            ui.state.home_status = "Sign in to continue".into();
        }
    }
    // x0048: sticky asset failure → Home status (not Coin fixture).
    if shared.overlay_asset_home_status(&mut ui.state.home_status) {
        ui.state.open = true;
        ui.state.tab = DashTab::Home;
        if ui.state.shell == OnboardingShell::MainHub {
            ui.state.shell = OnboardingShell::Dash;
        }
    }
    // Seed SharedState so XR can pull the same logical shell before first redraw.
    ui.mark_dash_logic_dirty();
    ui.push_dash_logic(&shared);
    let last_owns_input = role.owns_input(&shared.presentation);
    let boot_catalog = ui.state.shell == OnboardingShell::MainHub
        && ui.state.home_logged_in_as.is_some()
        && !joined;
    let mut app = App {
        mesh,
        shared,
        window: None,
        gpu: None,
        ui,
        zenoh,
        role,
        cursor: (0.0, 0.0),
        primary_down: false,
        keys_down: HashSet::new(),
        look_captured: false,
        device_look_active: false,
        skip_cursor_warp: false,
        last_cursor: None,
        blank_cursor: None,
        held_grab: None,
        grab_down: false,
        lmb_grab_armed: false,
        last_grab_miss_log: None,
        auth_job: None,
        waiting_session_job: None,
        last_owns_input,
        mesh_epoch: 0,
        pending_room_bind: None,
        room_bind_progress_painted: false,
    };
    // silent / remember-me MainHub entry — same catalog load as LoginOk / RegisterOk.
    if boot_catalog {
        start_worlds_catalog_refresh(&mut app.ui, &app.shared, &mut app.auth_job);
    }
    event_loop.run_app(&mut app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn access() -> Option<String> {
        Some("access".into())
    }

    /// Minimal JWT-shaped token whose unverified payload has `role`.
    fn role_token(role: &str) -> String {
        use base64::Engine;
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(format!(r#"{{"role":"{role}"}}"#));
        format!("hdr.{payload}.sig")
    }

    #[test]
    fn mouse_look_yaw_pitch_and_clamp() {
        let look = CameraLook {
            yaw: 0.0,
            pitch: 0.0,
        };
        // Mouse right → +yaw; mouse up (negative dy) → +pitch.
        let next = apply_mouse_look(look, 100.0, -50.0, 0.01, PITCH_LIMIT);
        assert!((next.yaw - 1.0).abs() < 1e-5);
        assert!((next.pitch - 0.5).abs() < 1e-5);

        let high = apply_mouse_look(
            CameraLook {
                yaw: 0.0,
                pitch: 1.5,
            },
            0.0,
            -1000.0,
            0.01,
            PITCH_LIMIT,
        );
        assert!((high.pitch - PITCH_LIMIT).abs() < 1e-5);
    }

    #[test]
    fn inventory_gate_allows_host_role_token() {
        let host = Some(role_token("host"));
        assert!(inventory_control_gate_status(
            &access(),
            Some(false),
            &host,
            Some("guest-sub"),
            Some("other-owner"),
        )
        .is_none());
    }

    #[test]
    fn inventory_gate_allows_catalog_owner_match() {
        assert!(inventory_control_gate_status(
            &access(),
            Some(false),
            &None,
            Some("owner-a"),
            Some("owner-a"),
        )
        .is_none());
    }

    #[test]
    fn inventory_gate_rejects_other_catalog_owner_without_host_token() {
        let msg = inventory_control_gate_status(
            &access(),
            Some(false),
            &None,
            Some("guest-sub"),
            Some("owner-a"),
        );
        assert_eq!(
            msg,
            Some("owner or host role required for take-out/despawn")
        );
    }

    #[test]
    fn inventory_gate_unknown_catalog_passes_to_host() {
        // Advanced ROOM_ID / catalog miss: early gate must not false-negative.
        assert!(
            inventory_control_gate_status(&access(), Some(false), &None, Some("anyone"), None,)
                .is_none()
        );
    }
}
