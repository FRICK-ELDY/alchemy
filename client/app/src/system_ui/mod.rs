//! system_ui — retained Canvas dash (Desktop overlay; VR quad later).
//!
//! Guarantees: ESC toggle, hit-tested chrome, client exit action plumbing.
//! Non-guarantees: OpenXR menu button, settings contents, mic/sync real control.

mod dash;
mod font;
mod gpu;
mod icons;
mod layout;

pub use dash::{DashState, DashTab};
pub use gpu::UiGpu;
pub use layout::{LayoutTree, WidgetId};

use dash::rebuild;

/// Owns dash state + retained layout tree.
pub struct SystemUi {
    pub state: DashState,
    pub tree: LayoutTree,
    pub gpu: Option<UiGpu>,
    /// Keeps wgpu device alive when canvas is used CPU-only (OpenXR path).
    _gpu_keepalive: Option<(wgpu::Device, wgpu::Queue)>,
    last_fps_instant: std::time::Instant,
    frames_in_window: u32,
    fps: f32,
}

impl SystemUi {
    pub fn new() -> Self {
        Self {
            state: DashState::default(),
            tree: LayoutTree::default(),
            gpu: None,
            _gpu_keepalive: None,
            last_fps_instant: std::time::Instant::now(),
            frames_in_window: 0,
            fps: 0.0,
        }
    }

    pub fn init_gpu(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        self.state.canvas_w = width as f32;
        self.state.canvas_h = height as f32;
        self.gpu = Some(UiGpu::new(device, queue, format, width, height)?);
        self.rebuild();
        Ok(())
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.state.canvas_w = width.max(1) as f32;
        self.state.canvas_h = height.max(1) as f32;
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.resize(device, width, height);
        }
        self.rebuild();
    }

    pub fn toggle(&mut self) {
        self.state.open = !self.state.open;
        self.rebuild();
    }

    pub fn set_time_now(&mut self) {
        let now = chrono_lite_now();
        if self.state.time_label != now {
            self.state.time_label = now;
            if self.state.open {
                self.rebuild();
            }
        }
    }

    pub fn tick_fps(&mut self) {
        self.frames_in_window += 1;
        let elapsed = self.last_fps_instant.elapsed();
        if elapsed >= std::time::Duration::from_secs(1) {
            self.fps = self.frames_in_window as f32 / elapsed.as_secs_f32();
            self.frames_in_window = 0;
            self.last_fps_instant = std::time::Instant::now();
            let label = format!("{:.0} FPS", self.fps);
            if self.state.fps_label != label {
                self.state.fps_label = label;
                if self.state.open {
                    self.rebuild();
                }
            }
        }
    }

    pub fn rebuild(&mut self) {
        rebuild(&mut self.tree, &self.state);
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.mark_dirty();
        }
    }

    pub fn on_click(&mut self, px: f32, py: f32) -> UiAction {
        if !self.state.open {
            return UiAction::None;
        }
        match self.tree.hit_test(px, py) {
            Some(WidgetId::TabExit) | Some(WidgetId::CloseSession) => {
                self.state.tab = DashTab::Exit;
                self.rebuild();
                UiAction::None
            }
            Some(WidgetId::TabSettings) => {
                self.state.tab = DashTab::Settings;
                self.rebuild();
                UiAction::None
            }
            Some(WidgetId::ExitClientOnly) => UiAction::ExitClientOnly,
            Some(WidgetId::ExitClientAndHost) => UiAction::ExitClientAndHost,
            Some(WidgetId::Mic)
            | Some(WidgetId::MicOff)
            | Some(WidgetId::Volume)
            | Some(WidgetId::Hearing)
            | Some(WidgetId::SyncBadge)
            | Some(WidgetId::SettingsPlaceholder) => {
                log::info!("system_ui stub click");
                UiAction::None
            }
            None => UiAction::None,
        }
    }

    pub fn prepare_frame(&mut self, queue: &wgpu::Queue) {
        self.set_time_now();
        self.tick_fps();
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.rasterize_if_dirty(queue, &self.tree, self.state.open);
        }
    }

    /// Update labels and rasterize CPU canvas (no GPU upload). Used by OpenXR.
    pub fn prepare_cpu_canvas(&mut self) {
        self.set_time_now();
        self.tick_fps();
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.rasterize_cpu(&self.tree, self.state.open);
        }
    }

    pub fn canvas_rgba(&self) -> Option<(&[u8], u32, u32)> {
        self.gpu.as_ref().map(|g| g.canvas_rgba())
    }

    /// Init canvas resources without a window surface (OpenXR uses CPU pixels only).
    pub fn init_cpu_canvas(&mut self, width: u32, height: u32) -> anyhow::Result<()> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: true,
        }))
        .ok_or_else(|| anyhow::anyhow!("no wgpu adapter for system_ui CPU canvas"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("system_ui-cpu"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
                memory_hints: Default::default(),
            },
            None,
        ))?;
        self.init_gpu(
            &device,
            &queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            width,
            height,
        )?;
        self._gpu_keepalive = Some((device, queue));
        Ok(())
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if !self.state.open {
            return;
        }
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.draw(pass);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiAction {
    None,
    ExitClientOnly,
    ExitClientAndHost,
}

fn chrono_lite_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Local offset is not applied (UTC) — good enough for Slice E stub clock.
    let s = secs % 86400;
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let sec = s % 60;
    format!("{h:02}:{m:02}:{sec:02}")
}
