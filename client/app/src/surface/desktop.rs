//! Desktop winit + wgpu surface (ESC dash).

use std::sync::Arc;

use anyhow::{Context, Result};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::host_control;
use crate::mesh::CpuMesh;
use crate::scene::{create_depth_view, create_wgpu_for_window, SceneGpu, SharedState};
use crate::system_ui::{SystemUi, UiAction};

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
        let pose = self.scene.pose();
        let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
        let (view, proj) = SceneGpu::desktop_view_proj(pose, aspect);

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
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Alchemy Client — x0008 (Desktop)")
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
        log::info!("system_ui ready — press Esc for dash");
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(size.width, size.height);
                    self.ui.resize(&gpu.scene.device, size.width, size.height);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x, position.y);
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                let action = self.ui.on_click(self.cursor.0 as f32, self.cursor.1 as f32);
                handle_ui_action(action, self.zenoh.as_ref(), event_loop);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed && !event.repeat {
                    if let PhysicalKey::Code(KeyCode::Escape) = event.physical_key {
                        self.ui.toggle();
                        log::info!("system_ui open={}", self.ui.state.open);
                    }
                }
            }
            WindowEvent::RedrawRequested => {
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
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}
