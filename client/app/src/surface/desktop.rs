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
use winit::event::{DeviceEvent, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Cursor, CursorGrabMode, CustomCursor, Window, WindowId};

use crate::auth;
use crate::host_control;
use crate::mesh::CpuMesh;
use crate::operator_prefs::{self, endpoints_for, OperatorKind};
use crate::scene::{
    create_depth_view, create_wgpu_for_window, CameraLook, EntityPose, MoveIntent, SceneGpu,
    SharedState,
};
use crate::surface::hotswitch::DesktopRole;
use crate::system_ui::{DashTab, OnboardingShell, SystemUi, UiAction};
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
    /// In-flight Auth HTTP (+ join) worker result (x002e).
    auth_job: Option<Receiver<AuthJobOutcome>>,
    /// Last observed Desktop input ownership (READY / vr_active edges).
    last_owns_input: bool,
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
    /// visibility ensure + guest join (`visibility` says whether the chosen level was applied).
    StartSession {
        visibility: Result<world_api::VisibilityEnsure, String>,
        join: Result<(), String>,
    },
    JoinRoom {
        result: Result<(), String>,
    },
}

fn spawn_auth_http<F>(f: F)
where
    F: FnOnce() + Send + 'static,
{
    if let Err(err) = thread::Builder::new().name("auth-http".into()).spawn(f) {
        log::error!("spawn auth-http failed: {err}");
    }
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
            leave_room(&self.shared, &mut self.zenoh);
            if let Ok(mut gate) = self.shared.auth.lock() {
                gate.clear_session();
            }
            let mut prefs = operator_prefs::load();
            prefs.clear_session();
            operator_prefs::save(&prefs);
            self.ui.state.home_logged_in_as = None;
            self.ui.state.home_create_open = false;
            self.ui.state.shell = OnboardingShell::Step2Auth;
            self.ui.state.home_status = if had_zenoh {
                "Signed out — Leave sent (host reclaim immediate)".into()
            } else {
                "Signed out — local only (no Zenoh; Leave put skipped)".into()
            };
            self.ui.state.open = true;
            self.ui.state.tab = DashTab::Home;
            self.ui.mark_dash_logic_dirty();
            self.ui.rebuild();
            log::info!("hotswitch: Desktop performed leave_room for XR AuthLogout");
        }
        if self.shared.presentation.take_host_shutdown_requested() {
            if let Some(session) = self.zenoh.as_ref() {
                let cap = self.shared.wire_cap();
                host_control::request_host_shutdown(
                    session,
                    self.shared.room_id.as_ref(),
                    cap.as_deref(),
                );
                log::info!("hotswitch: Desktop published host shutdown for XR Exit");
            } else {
                log::warn!(
                    "hotswitch: host shutdown requested but Zenoh is None — client exits without host notify"
                );
            }
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

    /// Non-blocking poll of Auth worker (x002e). Safe to call every frame.
    fn poll_auth_job(&mut self) {
        let outcome = match self.auth_job.as_ref() {
            Some(rx) => match rx.try_recv() {
                Ok(outcome) => Some(Ok(outcome)),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err(())),
            },
            None => None,
        };
        match outcome {
            None => {}
            Some(Err(())) => {
                self.auth_job = None;
                self.ui.state.home_status = "Auth worker disconnected".into();
                self.ui.mark_dash_form_dirty();
                self.ui.rebuild();
                log::warn!("auth worker disconnected before result");
            }
            Some(Ok(outcome)) => {
                self.auth_job = None;
                self.apply_auth_outcome(outcome);
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
            AuthJobOutcome::LoginOk { username } => {
                self.persist_session_prefs(&username);
                self.ui.state.home_logged_in_as = Some(username);
                self.ui.state.home_password.clear();
                self.ui.state.home_create_open = false;
                self.ui.state.shell = OnboardingShell::MainHub;
                self.ui.state.home_status = "Signed in — choose PLAY TOGETHER or SOLO".into();
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
                self.ui.state.home_status = "Account created — choose PLAY TOGETHER or SOLO".into();
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
                    &mut self.zenoh,
                    visibility,
                    join,
                );
            }
            AuthJobOutcome::JoinRoom { result } => {
                apply_join_result(
                    &mut self.ui,
                    &self.shared,
                    &mut self.zenoh,
                    result,
                    "Joined room",
                    "join room ok",
                );
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
        let id = self.shared.local_user_root();
        let eye = if id == crate::scene::LOCAL_USER_ROOT_UNSET {
            EntityPose {
                x: 0.0,
                y: 1.6,
                z: 6.0,
                ..Default::default()
            }
        } else {
            self.shared
                .world
                .lock()
                .ok()
                .and_then(|w| w.entities.get(&id).copied())
                .unwrap_or(EntityPose {
                    x: 0.0,
                    y: 1.6,
                    z: 6.0,
                    ..Default::default()
                })
        };
        let look = self.shared.look.lock().ok().map(|g| *g).unwrap_or_default();
        (eye, look)
    }

    fn grabbable_boxes(
        &self,
    ) -> (
        Vec<(u32, EntityPose)>,
        std::sync::Arc<crate::scene::BoxDefs>,
    ) {
        let defs = self.shared.box_defs_snapshot();
        let boxes = self
            .shared
            .world
            .lock()
            .ok()
            .map(|w| {
                w.entities
                    .iter()
                    .filter(|(id, _)| defs.grabbable.contains(*id))
                    .map(|(id, p)| (*id, *p))
                    .collect()
            })
            .unwrap_or_default();
        (boxes, defs)
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
                    if pressed && self.ui.state.open && self.ui.state.tab == DashTab::Home {
                        match code {
                            KeyCode::Backspace => {
                                self.ui.on_home_backspace();
                            }
                            KeyCode::Tab => {
                                self.ui.on_home_tab_key();
                            }
                            KeyCode::Enter | KeyCode::NumpadEnter if !event.repeat => {
                                let action = self.ui.on_home_enter();
                                handle_ui_action(
                                    action,
                                    &mut self.ui,
                                    &self.shared,
                                    &mut self.zenoh,
                                    &mut self.auth_job,
                                    event_loop,
                                );
                            }
                            _ => {}
                        }
                    }
                }
                if pressed {
                    if let Some(text) = event.text.as_ref() {
                        if self.ui.state.open && self.ui.state.tab == DashTab::Home {
                            // Skip control-ish single chars already handled as keys.
                            if text != "\u{8}" && text != "\t" && text != "\r" && text != "\n" {
                                self.ui.on_text_input(text);
                            }
                        }
                    }
                }
                self.rebuild_intent();
            }
            WindowEvent::RedrawRequested => {
                self.ui.pull_dash_logic(&self.shared);
                self.on_input_ownership_edge();
                self.refresh_window_title();
                self.sync_hotswitch_ui();
                self.poll_presentation_requests();
                self.poll_auth_job();
                self.poll_mtls_reopen();
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
                if let Some(gpu) = self.gpu.as_mut() {
                    if let Err(err) = gpu.render(&mut self.ui, highlight) {
                        log::error!("render failed: {err:#}");
                    }
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
        self.poll_mtls_reopen();
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

fn apply_start_session_result(
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    zenoh: &mut Option<zenoh::Session>,
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
            match bind_room_after_join(shared, zenoh) {
                Ok(()) => {
                    ui.state.home_create_open = false;
                    ui.state.home_status = if name.is_empty() {
                        format!("Session started — access level applied ({wanted})")
                    } else {
                        format!("Session started: {name} — access level applied ({wanted})")
                    };
                    ui.state.open = false;
                    log::info!("start session ok; visibility applied={wanted}");
                }
                Err(err) => {
                    ui.state.home_status = format!("Joined but room bus failed: {err:#}");
                    log::warn!("room bus after start failed: {err:#}");
                }
            }
        }
        (Ok(world_api::VisibilityEnsure::LeftExisting), Ok(())) => {
            match bind_room_after_join(shared, zenoh) {
                Ok(()) => {
                    ui.state.home_create_open = false;
                    ui.state.home_status = format!(
                        "Joined — visibility not updated (not owner); existing ACL kept (wanted {wanted})"
                    );
                    ui.state.open = false;
                    log::info!("start session joined; visibility left existing (wanted {wanted})");
                }
                Err(err) => {
                    ui.state.home_status = format!("Joined but room bus failed: {err:#}");
                    log::warn!("room bus after start failed: {err:#}");
                }
            }
        }
        (Err(vis_err), Ok(())) => match bind_room_after_join(shared, zenoh) {
            Ok(()) => {
                ui.state.home_create_open = false;
                ui.state.home_status =
                    format!("Joined — visibility step failed ({vis_err}); existing ACL");
                ui.state.open = false;
                log::warn!("joined after visibility error: {vis_err}");
            }
            Err(err) => {
                ui.state.home_status =
                    format!("Joined but room bus failed: {err:#} (visibility: {vis_err})");
                log::warn!("room bus after start failed: {err:#}");
            }
        },
        (vis, Err(err)) => {
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
            log::warn!("start session join failed: {err}");
        }
    }
    ui.mark_dash_logic_dirty();
    ui.rebuild();
}

fn apply_join_result(
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    zenoh: &mut Option<zenoh::Session>,
    join: Result<(), String>,
    ok_status: &str,
    ok_log: &str,
) {
    match join {
        Ok(()) => match bind_room_after_join(shared, zenoh) {
            Ok(()) => {
                ui.state.home_create_open = false;
                ui.state.home_status = ok_status.into();
                ui.state.open = false;
                log::info!("{ok_log}");
            }
            Err(err) => {
                ui.state.home_status = format!("Joined but room bus failed: {err:#}");
                log::warn!("room bus after join failed: {err:#}");
            }
        },
        Err(err) => {
            ui.state.home_status = format!("Join failed: {err}");
            log::warn!("join failed (session kept): {err}");
        }
    }
    ui.mark_dash_logic_dirty();
    ui.rebuild();
}

fn handle_ui_action(
    action: UiAction,
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    zenoh: &mut Option<zenoh::Session>,
    auth_job: &mut Option<Receiver<AuthJobOutcome>>,
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
        UiAction::ExitClientOnly => {
            log::info!("system_ui: exit client only");
            shared.presentation.request_exit();
            event_loop.exit();
        }
        UiAction::ExitClientAndHost => {
            log::info!("system_ui: exit client + host stub");
            if let Some(session) = zenoh.as_ref() {
                let cap = shared.wire_cap();
                host_control::request_host_shutdown(
                    session,
                    shared.room_id.as_ref(),
                    cap.as_deref(),
                );
            }
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
            *auth_job = Some(rx);
            spawn_auth_http(move || {
                match auth::login(&auth_url, &identifier, &password, remember) {
                    Ok(session) => {
                        let username = session.user.username.clone();
                        if let Ok(mut gate) = shared_job.auth.lock() {
                            gate.apply_auth_session(session);
                        }
                        let _ = tx.send(AuthJobOutcome::LoginOk { username });
                    }
                    Err(err) => {
                        let _ = tx.send(AuthJobOutcome::LoginFailed(format!("{err:#}")));
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
            *auth_job = Some(rx);
            spawn_auth_http(move || {
                match auth::register(
                    &auth_url, &username, &email, &password, &birthday, tos_agreed, remember,
                ) {
                    Ok(session) => {
                        let username = session.user.username.clone();
                        if let Ok(mut gate) = shared_job.auth.lock() {
                            gate.apply_auth_session(session);
                        }
                        let _ = tx.send(AuthJobOutcome::RegisterOk { username });
                    }
                    Err(err) => {
                        let _ = tx.send(AuthJobOutcome::RegisterFailed(format!("{err:#}")));
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
            // Zenoh Leave + drop session (kills room bus threads) on UI thread.
            // HTTP logout is best-effort on a worker (does not gate UI).
            leave_room(shared, zenoh);
            if let Ok(mut gate) = shared.auth.lock() {
                gate.clear_session();
            }
            let mut prefs = operator_prefs::load();
            prefs.clear_session();
            operator_prefs::save(&prefs);
            ui.state.home_logged_in_as = None;
            ui.state.home_create_open = false;
            ui.state.shell = OnboardingShell::Step2Auth;
            ui.state.home_status = "Signed out — sign in again".into();
            ui.state.tab = DashTab::Home;
            ui.mark_dash_logic_dirty();
            ui.rebuild();
            log::info!("auth logout — Leave + local session cleared");
            if let Some(access) = access {
                let auth_url = shared.get_auth_url();
                spawn_auth_http(move || {
                    let _ = auth::logout(&auth_url, &access, refresh.as_deref());
                });
            }
        }
        UiAction::StartSession => {
            if auth_job.is_some() {
                ui.state.home_status = "Auth busy…".into();
                ui.mark_dash_form_dirty();
                ui.rebuild();
                return;
            }
            let level = ui.state.home_access_level;
            let access = shared.auth.lock().ok().and_then(|g| g.access_token.clone());
            let Some(access) = access else {
                ui.state.home_status = "Sign in before starting a session".into();
                ui.mark_dash_form_dirty();
                ui.rebuild();
                return;
            };
            let shared_job = Arc::clone(shared);
            ui.state.home_status = format!("Starting session ({})…", level.as_str());
            ui.mark_dash_form_dirty();
            ui.rebuild();
            let (tx, rx) = mpsc::channel();
            *auth_job = Some(rx);
            spawn_auth_http(move || {
                let world_url = shared_job.get_world_url();
                let visibility = world_api::ensure_room_visibility(
                    &world_url,
                    &shared_job.room_id,
                    &access,
                    level,
                )
                .map_err(|e| format!("{e:#}"));
                let join = match &visibility {
                    Ok(_) => {
                        join_room_with_access(&shared_job, &access).map_err(|e| format!("{e:#}"))
                    }
                    Err(_) => Err("skipped join (visibility step failed)".into()),
                };
                let _ = tx.send(AuthJobOutcome::StartSession { visibility, join });
            });
        }
        UiAction::JoinRoom => {
            if auth_job.is_some() {
                ui.state.home_status = "Auth busy…".into();
                ui.mark_dash_form_dirty();
                ui.rebuild();
                return;
            }
            let access = shared.auth.lock().ok().and_then(|g| g.access_token.clone());
            let Some(access) = access else {
                ui.state.home_status = "Sign in before joining".into();
                ui.mark_dash_form_dirty();
                ui.rebuild();
                return;
            };
            let shared_job = Arc::clone(shared);
            ui.state.home_status = "Joining room…".into();
            ui.mark_dash_form_dirty();
            ui.rebuild();
            let (tx, rx) = mpsc::channel();
            *auth_job = Some(rx);
            spawn_auth_http(move || {
                let result =
                    join_room_with_access(&shared_job, &access).map_err(|e| format!("{e:#}"));
                let _ = tx.send(AuthJobOutcome::JoinRoom { result });
            });
        }
        UiAction::InventoryRefresh => {
            let assets = shared.get_assets_url();
            let token = shared.auth.lock().ok().and_then(|g| g.access_token.clone());
            ui.state.scene_meshes = shared.scene_meshes();
            ui.state.scene_selected = None;
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
            ui.rebuild();
        }
        UiAction::InventoryTakeOut => {
            let selected = ui.state.inventory_selected.and_then(|i| {
                ui.state
                    .inventory_nodes
                    .get(i)
                    .map(|(id, kind, _)| (id.clone(), kind.clone()))
            });
            let Some((node_id, kind)) = selected else {
                ui.state.inventory_status = "select a blob node first".into();
                ui.rebuild();
                return;
            };
            if kind != "blob" {
                ui.state.inventory_status = "x006d: blob nodes only".into();
                ui.rebuild();
                return;
            }
            let assets = shared.get_assets_url();
            let (access, room_token) = {
                let Ok(auth) = shared.auth.lock() else {
                    ui.state.inventory_status = "auth lock failed".into();
                    ui.rebuild();
                    return;
                };
                (auth.access_token.clone(), auth.room_token.clone())
            };
            let (Some(access), Some(room_token)) = (access, room_token) else {
                ui.state.inventory_status = "join a room as host first".into();
                ui.rebuild();
                return;
            };
            if crate::inventory::room_token_role_unverified(&room_token).as_deref() != Some("host")
            {
                ui.state.inventory_status = "host role required for take-out".into();
                ui.rebuild();
                return;
            }
            let Some(session) = zenoh.as_ref() else {
                ui.state.inventory_status = "zenoh not connected".into();
                ui.rebuild();
                return;
            };
            let Some(wire_cap) = shared.wire_cap() else {
                ui.state.inventory_status = "no wire_cap".into();
                ui.rebuild();
                return;
            };
            let Some(room_key) = shared.room_key() else {
                ui.state.inventory_status = "room_key missing".into();
                ui.rebuild();
                return;
            };
            match crate::inventory::take_out_blob_blake3(&assets, &access, &node_id) {
                Ok((_node, blake3)) => {
                    match crate::inventory::publish_inventory_spawn(
                        session,
                        shared.room_id.as_ref(),
                        wire_cap.as_ref(),
                        room_key.as_ref(),
                        &room_token,
                        &blake3,
                        0.0,
                        1.5,
                        2.0,
                    ) {
                        Ok(()) => {
                            ui.state.inventory_status =
                                format!("spawn requested {}", &blake3[..8.min(blake3.len())]);
                        }
                        Err(err) => {
                            ui.state.inventory_status = format!("spawn put failed: {err:#}");
                            log::warn!("inventory spawn: {err:#}");
                        }
                    }
                }
                Err(err) => {
                    ui.state.inventory_status = format!("take_out failed: {err:#}");
                    log::warn!("inventory take_out: {err:#}");
                }
            }
            ui.rebuild();
        }
        UiAction::InventoryOpen => {
            // Scene list is Manifest-derived; refresh without Drive HTTP (x006e).
            ui.state.scene_meshes = shared.scene_meshes();
            if ui
                .state
                .scene_selected
                .is_some_and(|i| i >= ui.state.scene_meshes.len())
            {
                ui.state.scene_selected = None;
            }
            ui.rebuild();
        }
        UiAction::InventoryPutBack => {
            // Always re-sync from SharedState so Put back sees post-spawn Manifest (x006e).
            let prev_entity = ui
                .state
                .scene_selected
                .and_then(|i| ui.state.scene_meshes.get(i).map(|(eid, _)| *eid));
            ui.state.scene_meshes = shared.scene_meshes();
            if let Some(eid) = prev_entity {
                ui.state.scene_selected =
                    ui.state.scene_meshes.iter().position(|(id, _)| *id == eid);
            } else if ui
                .state
                .scene_selected
                .is_some_and(|i| i >= ui.state.scene_meshes.len())
            {
                ui.state.scene_selected = None;
            }
            let selected = ui.state.scene_selected.and_then(|i| {
                ui.state
                    .scene_meshes
                    .get(i)
                    .map(|(eid, b3)| (*eid, b3.clone()))
            });
            let Some((entity_id, blake3)) = selected else {
                ui.state.inventory_status = "select a scene mesh first (Refresh if empty)".into();
                ui.rebuild();
                return;
            };
            let assets = shared.get_assets_url();
            let access = {
                let Ok(auth) = shared.auth.lock() else {
                    ui.state.inventory_status = "auth lock failed".into();
                    ui.rebuild();
                    return;
                };
                auth.access_token.clone()
            };
            let Some(access) = access else {
                ui.state.inventory_status = "sign in required".into();
                ui.rebuild();
                return;
            };
            let trs = shared.world.lock().ok().and_then(|world| {
                world.entities.get(&entity_id).map(|p| {
                    (
                        [p.x as f64, p.y as f64, p.z as f64],
                        [p.qx as f64, p.qy as f64, p.qz as f64, p.qw as f64],
                        [p.sx as f64, p.sy as f64, p.sz as f64],
                    )
                })
            });
            // Include entity_id so same blake3 / different entities do not upsert-collide.
            let name = format!("putback-e{entity_id}-{}", &blake3[..8.min(blake3.len())]);
            match crate::inventory::put_back_prefab(&assets, &access, &name, &blake3, trs) {
                Ok(node) => {
                    ui.state.inventory_status = format!(
                        "put-back ok {} (#{entity_id} stays; Take out is blob-only)",
                        node.name
                    );
                    // Refresh Drive list so the new prefab is visible.
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
            ui.rebuild();
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
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut ui = SystemUi::new();
    ui.state.home_room_id = shared.room_id.to_string();
    let prefs = operator_prefs::load();
    ui.state.operator = prefs.operator;
    ui.state.custom_host = prefs.custom_host.clone();
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
    if joined {
        ui.state.home_logged_in_as = username;
        ui.state.shell = OnboardingShell::Dash;
        ui.state.home_status = "Signed in (silent / CI)".into();
    } else {
        ui.state.open = true;
        ui.state.tab = DashTab::Home;
        if authenticated {
            ui.state.home_logged_in_as = username;
            ui.state.shell = OnboardingShell::MainHub;
            ui.state.home_status = "Signed in — choose PLAY TOGETHER or SOLO".into();
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
        last_owns_input,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
