//! Desktop winit + wgpu surface (ESC dash + Noclip + mouse look + Grab).
//!
//! Guarantees: canvas-pixel pointer → system_ui `on_pointer` (hover/press) + click;
//! dash-closed LMB → cursor grab (`Confined` then `Locked`); mouse Δ → client-local
//! `CameraLook`; WASD projected from current yaw; look-capture + aim-down then
//! LMB/RMB/E hold → Grab via look-forward ray (x0013). Floor boxes need pitch down.
//! Look is never on Zenoh.
//! Auth login/register/logout HTTP (+ room join HTTP) runs on a worker thread; the UI
//! thread only polls the outcome (x002e / P3-9).
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
use crate::scene::{
    create_depth_view, create_wgpu_for_window, CameraLook, EntityPose, MoveIntent, SceneGpu,
    SharedState,
};
use crate::system_ui::{DashTab, SystemUi, UiAction};
use crate::world_hit::{desktop_look_ray, pick_grabbable_desktop};
use crate::{declare_room_alive, join_room_with_access, leave_room};

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
}

/// Outcome of Auth HTTP / join work off the UI thread (x002e / P3-9).
enum AuthJobOutcome {
    Login {
        username: String,
        join: Result<(), String>,
    },
    Register {
        username: String,
        join: Result<(), String>,
    },
    LoginFailed(String),
    RegisterFailed(String),
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
    fn apply_look_delta(&mut self, dx: f64, dy: f64) {
        if !self.look_captured || self.ui.state.open {
            return;
        }
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        if let Ok(mut look) = self.shared.look.lock() {
            *look = apply_mouse_look(*look, dx, dy, LOOK_SENSITIVITY, PITCH_LIMIT);
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
                self.ui.rebuild();
                log::warn!("auth worker disconnected before result");
            }
            Some(Ok(outcome)) => {
                self.auth_job = None;
                self.apply_auth_outcome(outcome);
            }
        }
    }

    fn apply_auth_outcome(&mut self, outcome: AuthJobOutcome) {
        match outcome {
            AuthJobOutcome::Login { username, join } => {
                apply_auth_join_result(
                    &mut self.ui,
                    &self.shared,
                    self.zenoh.as_ref(),
                    username,
                    join,
                    "Joined room",
                    "auth login + join ok",
                );
            }
            AuthJobOutcome::Register { username, join } => {
                apply_auth_join_result(
                    &mut self.ui,
                    &self.shared,
                    self.zenoh.as_ref(),
                    username,
                    join,
                    "Account created — joined room",
                    "auth register + join ok",
                );
            }
            AuthJobOutcome::LoginFailed(err) => {
                self.ui.state.home_status = format!("Login failed: {err}");
                self.ui.rebuild();
                log::warn!("login failed: {err}");
            }
            AuthJobOutcome::RegisterFailed(err) => {
                self.ui.state.home_status = format!("Register failed: {err}");
                self.ui.rebuild();
                log::warn!("register failed: {err}");
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
            .with_title("Alchemy Client — x0013 (Desktop Grab)")
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
        log::info!(
            "system_ui ready — Esc dash; click to look; aim box (look down, brightens) + LMB/RMB/E Grab; WASD+Space+C"
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
            WindowEvent::CloseRequested => event_loop.exit(),
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
                            self.zenoh.as_ref(),
                            &mut self.auth_job,
                            event_loop,
                        );
                    }
                } else if pressed && !self.look_captured {
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
                self.grab_down = state == ElementState::Pressed;
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
                                    self.zenoh.as_ref(),
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
                self.poll_auth_job();
                self.maintain_look_capture();
                self.rebuild_intent();
                let highlight = self.aim_highlight();
                if let Some(gpu) = self.gpu.as_mut() {
                    if let Err(err) = gpu.render(&mut self.ui, highlight) {
                        log::error!("render failed: {err:#}");
                    }
                }
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        self.poll_auth_job();
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

fn apply_auth_join_result(
    ui: &mut SystemUi,
    shared: &SharedState,
    zenoh: Option<&zenoh::Session>,
    username: String,
    join: Result<(), String>,
    ok_status: &str,
    ok_log: &str,
) {
    match join {
        Ok(()) => {
            if let Some(session) = zenoh {
                if let Err(err) = declare_room_alive(shared, session, false) {
                    log::warn!("liveliness after auth failed: {err:#}");
                }
            }
            ui.state.home_logged_in_as = Some(username);
            ui.state.home_password.clear();
            ui.state.home_status = ok_status.into();
            ui.state.open = false;
            log::info!("{ok_log}");
        }
        Err(err) => {
            // Keep AuthGate session; surface "signed in, not joined" so UI matches gate.
            ui.state.home_logged_in_as = Some(username);
            ui.state.home_password.clear();
            ui.state.home_status = format!("Signed in — join failed: {err}");
            log::warn!("join after auth failed (session kept): {err}");
        }
    }
    ui.rebuild();
}

fn handle_ui_action(
    action: UiAction,
    ui: &mut SystemUi,
    shared: &Arc<SharedState>,
    zenoh: Option<&zenoh::Session>,
    auth_job: &mut Option<Receiver<AuthJobOutcome>>,
    event_loop: &ActiveEventLoop,
) {
    match action {
        UiAction::ExitClientOnly => {
            log::info!("system_ui: exit client only");
            event_loop.exit();
        }
        UiAction::ExitClientAndHost => {
            log::info!("system_ui: exit client + host stub");
            if let Some(session) = zenoh {
                host_control::request_host_shutdown(session, shared.room_id.as_ref());
            }
            event_loop.exit();
        }
        UiAction::AuthLogin => {
            if auth_job.is_some() {
                ui.state.home_status = "Auth busy…".into();
                ui.rebuild();
                return;
            }
            let identifier = ui.state.home_identifier.clone();
            let password = ui.state.home_password.clone();
            let remember = ui.state.home_remember_me;
            let auth_url = shared.auth_url.clone();
            let shared_job = Arc::clone(shared);
            ui.state.home_status = "Signing in…".into();
            ui.rebuild();
            let (tx, rx) = mpsc::channel();
            *auth_job = Some(rx);
            spawn_auth_http(move || {
                match auth::login(&auth_url, &identifier, &password, remember) {
                    Ok(session) => {
                        let username = session.user.username.clone();
                        let access = session.access_token.clone();
                        if let Ok(mut gate) = shared_job.auth.lock() {
                            gate.apply_auth_session(session);
                        }
                        let join = join_room_with_access(&shared_job, &access)
                            .map_err(|e| format!("{e:#}"));
                        let _ = tx.send(AuthJobOutcome::Login { username, join });
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
                ui.rebuild();
                return;
            }
            let username = ui.state.home_username.clone();
            let email = ui.state.home_email.clone();
            let password = ui.state.home_password.clone();
            let birthday = ui.state.home_birthday.clone();
            let remember = ui.state.home_remember_me;
            let auth_url = shared.auth_url.clone();
            let shared_job = Arc::clone(shared);
            ui.state.home_status = "Creating account…".into();
            ui.rebuild();
            let (tx, rx) = mpsc::channel();
            *auth_job = Some(rx);
            spawn_auth_http(move || {
                match auth::register(&auth_url, &username, &email, &password, &birthday, remember) {
                    Ok(session) => {
                        let username = session.user.username.clone();
                        let access = session.access_token.clone();
                        if let Ok(mut gate) = shared_job.auth.lock() {
                            gate.apply_auth_session(session);
                        }
                        let join = join_room_with_access(&shared_job, &access)
                            .map_err(|e| format!("{e:#}"));
                        let _ = tx.send(AuthJobOutcome::Register { username, join });
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
            // Zenoh Leave on UI thread; drop local session immediately so refresh/UI stop.
            // HTTP logout is best-effort on a worker (does not gate UI).
            if let Some(session) = zenoh {
                leave_room(shared, session);
            } else {
                shared.leave_room_local();
            }
            if let Ok(mut gate) = shared.auth.lock() {
                gate.clear_session();
            }
            ui.state.home_logged_in_as = None;
            ui.state.home_status = "Signed out — Leave sent (host reclaim immediate)".into();
            ui.state.tab = DashTab::Home;
            ui.rebuild();
            log::info!("auth logout — Leave + local session cleared");
            if let Some(access) = access {
                let auth_url = shared.auth_url.clone();
                spawn_auth_http(move || {
                    let _ = auth::logout(&auth_url, &access, refresh.as_deref());
                });
            }
        }
        UiAction::None => {}
    }
}

pub fn run(mesh: CpuMesh, shared: Arc<SharedState>, zenoh: Option<zenoh::Session>) -> Result<()> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut ui = SystemUi::new();
    let joined = shared
        .auth
        .lock()
        .ok()
        .map(|g| g.is_joined())
        .unwrap_or(false);
    if joined {
        if let Ok(gate) = shared.auth.lock() {
            ui.state.home_logged_in_as = gate.username.clone();
            ui.state.home_status = "Signed in (silent / CI)".into();
        }
    } else {
        // Prompt for Home login before joining.
        ui.state.open = true;
        ui.state.tab = DashTab::Home;
    }
    let mut app = App {
        mesh,
        shared,
        window: None,
        gpu: None,
        ui,
        zenoh,
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
