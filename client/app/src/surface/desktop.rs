//! Desktop winit + wgpu surface (ESC dash + Noclip intent + x0012 mouse look).
//!
//! Guarantees: canvas-pixel pointer → system_ui `on_pointer` (hover/press) + click;
//! dash-closed LMB → cursor grab (`Confined` then `Locked`); mouse Δ → client-local
//! `CameraLook`; WASD projected from current yaw. Look is never published on Zenoh.
//!
//! Linux/WSL: prefer Confined + `CursorMoved` (recenter). `Locked` often succeeds but
//! yields no `DeviceEvent::MouseMotion`, so look appears dead.

use std::collections::HashSet;
use std::sync::Arc;

use alchemy_protocol::input_buttons;
use anyhow::{Context, Result};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{DeviceEvent, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Cursor, CursorGrabMode, CustomCursor, Window, WindowId};

use crate::host_control;
use crate::mesh::CpuMesh;
use crate::scene::{
    create_depth_view, create_wgpu_for_window, CameraLook, MoveIntent, SceneGpu, SharedState,
};
use crate::system_ui::{SystemUi, UiAction};

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

    fn render(&mut self, ui: &mut SystemUi) -> Result<()> {
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
            self.device_look_active = false;
            self.skip_cursor_warp = false;
            self.warp_cursor_center(true);
            log::info!("mouse look capture on ({mode:?})");
        } else if self.look_captured {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            self.apply_default_cursor(&window);
            self.look_captured = false;
            self.device_look_active = false;
            self.skip_cursor_warp = false;
            self.last_cursor = None;
            log::info!("mouse look capture off");
        }
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

    fn rebuild_intent(&self) {
        let dash_open = self.ui.state.open;
        let yaw = self
            .shared
            .look
            .lock()
            .ok()
            .map(|g| g.yaw)
            .unwrap_or(0.0);
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
            .with_title("Alchemy Client — x0012 (Desktop mouse look)")
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
        log::info!("system_ui ready — Esc dash; click to look; WASD+Space+C Noclip");
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
                self.primary_down = state == ElementState::Pressed;
                if self.ui.state.open {
                    self.ui.on_pointer(
                        self.cursor.0 as f32,
                        self.cursor.1 as f32,
                        self.primary_down,
                    );
                    if self.primary_down {
                        let action = self.ui.on_click(self.cursor.0 as f32, self.cursor.1 as f32);
                        handle_ui_action(action, self.zenoh.as_ref(), event_loop);
                    }
                } else if self.primary_down && !self.look_captured {
                    self.set_look_capture(true);
                }
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
                            self.ui.clear_pointer();
                        }
                        log::info!("system_ui open={}", self.ui.state.open);
                    }
                }
                self.rebuild_intent();
            }
            WindowEvent::RedrawRequested => {
                self.maintain_look_capture();
                self.rebuild_intent();
                if let Some(gpu) = self.gpu.as_mut() {
                    if let Err(err) = gpu.render(&mut self.ui) {
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
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

fn handle_ui_action(
    action: UiAction,
    zenoh: Option<&zenoh::Session>,
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
                host_control::request_host_shutdown(session);
            }
            event_loop.exit();
        }
        UiAction::None => {}
    }
}

pub fn run(mesh: CpuMesh, shared: Arc<SharedState>, zenoh: Option<zenoh::Session>) -> Result<()> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        mesh,
        shared,
        window: None,
        gpu: None,
        ui: SystemUi::new(),
        zenoh,
        cursor: (0.0, 0.0),
        primary_down: false,
        keys_down: HashSet::new(),
        look_captured: false,
        device_look_active: false,
        skip_cursor_warp: false,
        last_cursor: None,
        blank_cursor: None,
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
