//! Alchemy client — Slice B.
//!
//! Guarantees:
//! - Subscribes to frame key and decodes RenderFrame
//! - Fetches Room Token from world-server and publishes InputEvent
//! - Shows a moving colored triangle driven by entity (x, y)
//!
//! Non-guarantees:
//! - OSC, Dual Editor, asset LOD, auth login UI, launcher

use std::f32::consts::TAU;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use alchemy_protocol::{
    InputEvent, Message, RenderFrame, FRAME_KEY_LOCAL, INPUT_KEY_LOCAL, PROTOCOL_PIN,
};
use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use serde::Deserialize;
use wgpu::util::DeviceExt;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};
use zenoh::bytes::ZBytes;
use zenoh::Config;
use zenoh::Wait;

const CONNECT: &str = "tcp/127.0.0.1:7447";
const DEFAULT_WORLD_URL: &str = "http://127.0.0.1:4003";
const DEFAULT_ROOM_ID: &str = "local";
const INPUT_HZ: u64 = 10;

#[derive(Clone, Copy, Debug, Default)]
struct EntityPose {
    x: f32,
    y: f32,
    r: f32,
    g: f32,
    b: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 2],
    color: [f32; 3],
}

struct SharedState {
    pose: Mutex<EntityPose>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    token: String,
}

fn open_zenoh() -> Result<zenoh::Session> {
    let mut config = Config::default();
    config
        .insert_json5("mode", r#""client""#)
        .map_err(|e| anyhow::anyhow!("zenoh mode: {e}"))?;
    config
        .insert_json5("connect/endpoints", &format!(r#"["{CONNECT}"]"#))
        .map_err(|e| anyhow::anyhow!("zenoh connect: {e}"))?;
    zenoh::open(config)
        .wait()
        .map_err(|e| anyhow::anyhow!("zenoh open failed: {e}"))
}

fn fetch_guest_token(world_url: &str, room_id: &str) -> Result<String> {
    if let Ok(token) = std::env::var("ALCHEMY_ROOM_TOKEN") {
        if !token.is_empty() {
            return Ok(token);
        }
    }

    let base = world_url.trim_end_matches('/');
    let create_url = format!("{base}/api/v1/rooms");
    let _ = ureq::post(&create_url)
        .set("content-type", "application/json")
        .send_json(serde_json::json!({ "room_id": room_id }));

    let token_url = format!("{base}/api/v1/rooms/{room_id}/tokens");
    let resp: TokenResponse = ureq::post(&token_url)
        .set("content-type", "application/json")
        .send_json(serde_json::json!({ "role": "guest", "sub": "client" }))
        .context("issue guest Room Token")?
        .into_json()
        .context("parse token response")?;
    Ok(resp.token)
}

fn client_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn spawn_zenoh_subscriber(shared: Arc<SharedState>) -> Result<()> {
    let session = open_zenoh().context("open zenoh")?;
    let subscriber = session
        .declare_subscriber(FRAME_KEY_LOCAL)
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_subscriber failed: {e}"))?;

    thread::Builder::new()
        .name("zenoh-frame".into())
        .spawn(move || {
            log::info!("subscribed to {FRAME_KEY_LOCAL}");
            while let Ok(sample) = subscriber.recv() {
                let bytes: ZBytes = sample.payload().clone();
                let payload = bytes.to_bytes();
                match RenderFrame::decode(payload.as_ref()) {
                    Ok(frame) => {
                        if let Some(e) = frame.entities.first() {
                            if let Ok(mut pose) = shared.pose.lock() {
                                pose.x = e.x as f32;
                                pose.y = e.y as f32;
                                pose.r = e.r;
                                pose.g = e.g;
                                pose.b = e.b;
                            }
                        }
                    }
                    Err(err) => log::warn!("RenderFrame decode failed: {err}"),
                }
            }
        })
        .context("spawn zenoh thread")?;
    Ok(())
}

fn spawn_input_publisher(room_token: String) -> Result<()> {
    let session = open_zenoh().context("open zenoh for input")?;
    let publisher = session
        .declare_publisher(INPUT_KEY_LOCAL)
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_publisher input failed: {e}"))?;

    thread::Builder::new()
        .name("zenoh-input-pub".into())
        .spawn(move || {
            log::info!("publishing InputEvent to {INPUT_KEY_LOCAL}");
            let period = Duration::from_nanos(1_000_000_000 / INPUT_HZ);
            let start = Instant::now();
            let mut next = Instant::now();
            loop {
                let now = Instant::now();
                if now < next {
                    thread::sleep(next - now);
                }
                next += period;

                let t = start.elapsed().as_secs_f32();
                let event = InputEvent {
                    room_token: room_token.clone(),
                    client_time_ms: client_time_ms(),
                    move_x: (t * TAU / 6.0).cos() * 1.2,
                    move_y: (t * TAU / 6.0).sin() * 0.6,
                    move_z: 0.0,
                };
                if let Err(err) = publisher.put(event.encode_to_vec()).wait() {
                    log::warn!("input publish failed: {err}");
                }
            }
        })
        .context("spawn input publisher")?;
    Ok(())
}

struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
}

impl Gpu {
    async fn new(window: Arc<Window>) -> Result<Self> {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window.clone())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .context("no wgpu adapter")?;
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("alchemy-client"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: Default::default(),
                },
                None,
            )
            .await
            .context("request_device")?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("triangle"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pipeline-layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("triangle-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x3],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vertices"),
            contents: bytemuck::cast_slice(&triangle_vertices(EntityPose::default())),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            vertex_buffer,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    fn render(&mut self, pose: EntityPose) -> Result<()> {
        let verts = triangle_vertices(pose);
        self.queue
            .write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&verts));

        let frame = self
            .surface
            .get_current_texture()
            .context("get_current_texture")?;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.05,
                            g: 0.06,
                            b: 0.09,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.draw(0..3, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }
}

fn triangle_vertices(pose: EntityPose) -> [Vertex; 3] {
    let cx = (pose.x / 2.0).clamp(-0.85, 0.85);
    let cy = (pose.y / 2.0).clamp(-0.85, 0.85);
    let s = 0.18;
    let color = [pose.r.max(0.2), pose.g.max(0.2), pose.b.max(0.2)];
    [
        Vertex {
            position: [cx, cy + s],
            color,
        },
        Vertex {
            position: [cx - s, cy - s],
            color,
        },
        Vertex {
            position: [cx + s, cy - s],
            color,
        },
    ]
}

struct App {
    shared: Arc<SharedState>,
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Alchemy Client — Slice B")
            .with_inner_size(LogicalSize::new(960.0, 540.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let gpu = pollster::block_on(Gpu::new(Arc::clone(&window))).expect("init gpu");
        self.window = Some(window);
        self.gpu = Some(gpu);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => {
                let pose = self.shared.pose.lock().map(|p| *p).unwrap_or_default();
                if let Some(gpu) = self.gpu.as_mut() {
                    if let Err(err) = gpu.render(pose) {
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

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let world_url = std::env::var("WORLD_URL").unwrap_or_else(|_| DEFAULT_WORLD_URL.to_string());
    let room_id = std::env::var("ROOM_ID").unwrap_or_else(|_| DEFAULT_ROOM_ID.to_string());

    log::info!(
        "client Slice B starting; protocol_pin={PROTOCOL_PIN} frame={FRAME_KEY_LOCAL} input={INPUT_KEY_LOCAL} world={world_url}"
    );

    let room_token = fetch_guest_token(&world_url, &room_id).context("fetch guest Room Token")?;
    log::info!("guest Room Token acquired");

    let shared = Arc::new(SharedState {
        pose: Mutex::new(EntityPose {
            r: 0.2,
            g: 0.7,
            b: 1.0,
            ..Default::default()
        }),
    });
    spawn_zenoh_subscriber(Arc::clone(&shared))?;
    spawn_input_publisher(room_token)?;

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        shared,
        window: None,
        gpu: None,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}
