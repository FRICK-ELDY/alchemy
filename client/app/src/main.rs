//! Alchemy client — Slice C-C.
//!
//! Guarantees:
//! - Subscribes to SceneManifest (blake3 refs) and RenderFrame deltas
//! - Downloads blob by Manifest blake3 (fixture fallback), BLAKE3 verify, wgpu mesh
//! - Applies upserts/removes to a local entity table; mesh follows entity 1 pose
//! - Fetches Room Token and publishes InputEvent
//!
//! Non-guarantees:
//! - OSC, Dual Editor, asset LOD, auth login UI, launcher, materials/skins
//! - Signed Manifest / world-server ledger

mod mesh;

use std::collections::HashMap;
use std::f32::consts::TAU;
use std::io::Read;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use alchemy_protocol::{
    InputEvent, Message, RenderFrame, SceneManifest, FRAME_KEY_LOCAL, INPUT_KEY_LOCAL,
    MANIFEST_KEY_LOCAL, PROTOCOL_PIN,
};
use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use mesh::{blake3_hex, load_mesh_from_glb, verify_blake3, CpuMesh, COIN_BLAKE3_HEX};
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
const DEFAULT_AUTH_URL: &str = "http://127.0.0.1:4002";
const DEFAULT_ASSETS_URL: &str = "http://127.0.0.1:4004";
const DEFAULT_ROOM_ID: &str = "local";
const DEFAULT_AUTH_USER: &str = "alice";
const DEFAULT_AUTH_PASSWORD: &str = "Password1";
const INPUT_HZ: u64 = 10;
const MANIFEST_WAIT: Duration = Duration::from_secs(5);
const MESH_ENTITY_ID: u32 = 1;

#[derive(Clone, Copy, Debug, Default)]
struct EntityPose {
    x: f32,
    y: f32,
    z: f32,
    r: f32,
    g: f32,
    b: f32,
}

struct SharedState {
    entities: Mutex<HashMap<u32, EntityPose>>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    token: String,
}

#[derive(Debug, Deserialize)]
struct AuthSession {
    access_token: String,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MeshVertex {
    position: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    mvp: [[f32; 4]; 4],
    color: [f32; 4],
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

fn fetch_auth_token(auth_url: &str) -> Result<String> {
    if let Ok(token) = std::env::var("ALCHEMY_AUTH_TOKEN") {
        if !token.is_empty() {
            return Ok(token);
        }
    }

    let user = std::env::var("AUTH_USER").unwrap_or_else(|_| DEFAULT_AUTH_USER.to_string());
    let password =
        std::env::var("AUTH_PASSWORD").unwrap_or_else(|_| DEFAULT_AUTH_PASSWORD.to_string());
    let base = auth_url.trim_end_matches('/');
    let login_url = format!("{base}/api/v1/auth/login");
    let resp: AuthSession = ureq::post(&login_url)
        .set("content-type", "application/json")
        .send_json(serde_json::json!({
            "identifier": user,
            "password": password
        }))
        .context("auth login failed (is auth-server up?)")?
        .into_json()
        .context("parse auth session")?;
    Ok(resp.access_token)
}

fn fetch_guest_token(world_url: &str, room_id: &str, auth_token: &str) -> Result<String> {
    if let Ok(token) = std::env::var("ALCHEMY_ROOM_TOKEN") {
        if !token.is_empty() {
            return Ok(token);
        }
    }

    let base = world_url.trim_end_matches('/');
    let authz = format!("Bearer {auth_token}");
    let create_url = format!("{base}/api/v1/rooms");
    let _ = ureq::post(&create_url)
        .set("content-type", "application/json")
        .set("authorization", &authz)
        .send_json(serde_json::json!({ "room_id": room_id }));

    let token_url = format!("{base}/api/v1/rooms/{room_id}/tokens");
    let resp: TokenResponse = ureq::post(&token_url)
        .set("content-type", "application/json")
        .set("authorization", &authz)
        .send_json(serde_json::json!({ "role": "guest" }))
        .context("issue guest Room Token")?
        .into_json()
        .context("parse token response")?;
    Ok(resp.token)
}

fn fetch_blob_bytes(assets_url: &str, blake3: &str, auth_token: &str) -> Result<Vec<u8>> {
    let base = assets_url.trim_end_matches('/');
    let url = format!("{base}/api/v1/blobs/{blake3}");
    let authz = format!("Bearer {auth_token}");
    let resp = ureq::get(&url)
        .set("authorization", &authz)
        .call()
        .context("assets blob download failed")?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .read_to_end(&mut bytes)
        .context("read blob body")?;
    Ok(bytes)
}

fn load_blob_bytes(assets_url: &str, blake3: &str, auth_token: &str) -> Result<Vec<u8>> {
    match fetch_blob_bytes(assets_url, blake3, auth_token) {
        Ok(bytes) => {
            log::info!(
                "downloaded blob from assets-server ({} bytes, blake3={})",
                bytes.len(),
                blake3_hex(&bytes)
            );
            Ok(bytes)
        }
        Err(err) => {
            log::warn!("assets download failed ({err:#}); falling back to fixture");
            load_fixture_bytes()
        }
    }
}

fn load_fixture_bytes() -> Result<Vec<u8>> {
    let path = fixture_path()?;
    std::fs::read(&path).with_context(|| format!("read fixture {}", path.display()))
}

fn fixture_path() -> Result<PathBuf> {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("../fixtures/Coin_PolygonalMind.glb");
    Ok(path)
}

fn client_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// First Manifest blake3 for `MESH_ENTITY_ID`, plus seed poses from entries.
fn spawn_manifest_subscriber(
    shared: Arc<SharedState>,
    blake3_tx: mpsc::SyncSender<String>,
) -> Result<()> {
    let session = open_zenoh().context("open zenoh for manifest")?;
    let subscriber = session
        .declare_subscriber(MANIFEST_KEY_LOCAL)
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_subscriber manifest failed: {e}"))?;

    thread::Builder::new()
        .name("zenoh-manifest".into())
        .spawn(move || {
            log::info!("subscribed to {MANIFEST_KEY_LOCAL}");
            let mut sent_blake3 = false;
            while let Ok(sample) = subscriber.recv() {
                let bytes: ZBytes = sample.payload().clone();
                let payload = bytes.to_bytes();
                match SceneManifest::decode(payload.as_ref()) {
                    Ok(manifest) => {
                        log::info!(
                            "SceneManifest revision={} entries={}",
                            manifest.revision,
                            manifest.entries.len()
                        );
                        if let Ok(mut map) = shared.entities.lock() {
                            for entry in &manifest.entries {
                                map.entry(entry.entity_id).or_insert(EntityPose {
                                    x: entry.x as f32,
                                    y: entry.y as f32,
                                    z: entry.z as f32,
                                    r: 0.92,
                                    g: 0.78,
                                    b: 0.28,
                                });
                            }
                        }
                        if !sent_blake3 {
                            if let Some(entry) = manifest
                                .entries
                                .iter()
                                .find(|e| e.entity_id == MESH_ENTITY_ID)
                            {
                                let _ = blake3_tx.send(entry.blake3.clone());
                                sent_blake3 = true;
                            }
                        }
                    }
                    Err(err) => log::warn!("SceneManifest decode failed: {err}"),
                }
            }
        })
        .context("spawn manifest thread")?;
    Ok(())
}

fn spawn_frame_subscriber(shared: Arc<SharedState>) -> Result<()> {
    let session = open_zenoh().context("open zenoh for frame")?;
    let subscriber = session
        .declare_subscriber(FRAME_KEY_LOCAL)
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_subscriber frame failed: {e}"))?;

    thread::Builder::new()
        .name("zenoh-frame".into())
        .spawn(move || {
            log::info!("subscribed to {FRAME_KEY_LOCAL} (delta upserts/removes)");
            while let Ok(sample) = subscriber.recv() {
                let bytes: ZBytes = sample.payload().clone();
                let payload = bytes.to_bytes();
                match RenderFrame::decode(payload.as_ref()) {
                    Ok(frame) => {
                        if let Ok(mut map) = shared.entities.lock() {
                            for e in frame.upserts {
                                map.insert(
                                    e.id,
                                    EntityPose {
                                        x: e.x as f32,
                                        y: e.y as f32,
                                        z: e.z as f32,
                                        r: e.r,
                                        g: e.g,
                                        b: e.b,
                                    },
                                );
                            }
                            for id in frame.removes {
                                map.remove(&id);
                            }
                        }
                    }
                    Err(err) => log::warn!("RenderFrame decode failed: {err}"),
                }
            }
        })
        .context("spawn frame thread")?;
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
    index_buffer: wgpu::Buffer,
    index_count: u32,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    depth_view: wgpu::TextureView,
    mesh_radius: f32,
    shared: Arc<SharedState>,
    started: Instant,
}

impl Gpu {
    async fn new(window: Arc<Window>, mesh: &CpuMesh, shared: Arc<SharedState>) -> Result<Self> {
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

        let depth_view = create_depth_view(&device, config.width, config.height);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mesh"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uniform-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mesh-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<MeshVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
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
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let vertices: Vec<MeshVertex> = mesh
            .positions
            .iter()
            .map(|p| MeshVertex { position: *p })
            .collect();

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh-vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh-indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("uniform-bg"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            vertex_buffer,
            index_buffer,
            index_count: mesh.indices.len() as u32,
            uniform_buffer,
            bind_group,
            depth_view,
            mesh_radius: mesh.radius,
            shared,
            started: Instant::now(),
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.depth_view = create_depth_view(&self.device, width, height);
    }

    fn render(&mut self) -> Result<()> {
        let pose = self
            .shared
            .entities
            .lock()
            .ok()
            .and_then(|m| m.get(&MESH_ENTITY_ID).copied())
            .unwrap_or(EntityPose {
                r: 0.92,
                g: 0.78,
                b: 0.28,
                ..Default::default()
            });

        let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
        let t = self.started.elapsed().as_secs_f32();
        let eye_dist = self.mesh_radius * 4.0 + 2.0;
        let view = Mat4::look_at_rh(
            Vec3::new(eye_dist * 0.7, eye_dist * 0.45, eye_dist),
            Vec3::new(pose.x, pose.y, pose.z),
            Vec3::Y,
        );
        let proj = Mat4::perspective_rh(50.0_f32.to_radians(), aspect, 0.01, eye_dist * 20.0);
        let model = Mat4::from_translation(Vec3::new(pose.x, pose.y, pose.z))
            * Mat4::from_rotation_y(t * 0.8)
            * Mat4::from_rotation_x(0.35);
        let mvp = proj * view * model;

        let uniforms = Uniforms {
            mvp: mvp.to_cols_array_2d(),
            color: [pose.r, pose.g, pose.b, 1.0],
        };
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

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
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.index_count, 0, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }
}

fn create_depth_view(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

struct App {
    mesh: CpuMesh,
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
            .with_title("Alchemy Client — Slice C-C (Manifest + Delta)")
            .with_inner_size(LogicalSize::new(960.0, 540.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let gpu = pollster::block_on(Gpu::new(
            Arc::clone(&window),
            &self.mesh,
            Arc::clone(&self.shared),
        ))
        .expect("init gpu");
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
                if let Some(gpu) = self.gpu.as_mut() {
                    if let Err(err) = gpu.render() {
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
    let auth_url = std::env::var("AUTH_URL").unwrap_or_else(|_| DEFAULT_AUTH_URL.to_string());
    let assets_url = std::env::var("ASSETS_URL").unwrap_or_else(|_| DEFAULT_ASSETS_URL.to_string());
    let room_id = std::env::var("ROOM_ID").unwrap_or_else(|_| DEFAULT_ROOM_ID.to_string());
    let fallback_blake3 =
        std::env::var("ALCHEMY_BLOB_BLAKE3").unwrap_or_else(|_| COIN_BLAKE3_HEX.to_string());

    log::info!(
        "client Slice C-C starting; protocol_pin={PROTOCOL_PIN} manifest={MANIFEST_KEY_LOCAL} frame={FRAME_KEY_LOCAL} input={INPUT_KEY_LOCAL} world={world_url} auth={auth_url} assets={assets_url}"
    );

    let auth_token = fetch_auth_token(&auth_url).context("fetch auth JWT")?;
    let room_token =
        fetch_guest_token(&world_url, &room_id, &auth_token).context("fetch guest Room Token")?;
    log::info!("auth JWT + guest Room Token acquired");

    let shared = Arc::new(SharedState {
        entities: Mutex::new(HashMap::new()),
    });
    let (blake3_tx, blake3_rx) = mpsc::sync_channel::<String>(1);
    spawn_manifest_subscriber(Arc::clone(&shared), blake3_tx)?;
    spawn_frame_subscriber(Arc::clone(&shared))?;
    spawn_input_publisher(room_token)?;

    let blob_blake3 = match blake3_rx.recv_timeout(MANIFEST_WAIT) {
        Ok(hex) => {
            log::info!("Manifest blake3 for entity {MESH_ENTITY_ID}: {hex}");
            hex
        }
        Err(_) => {
            log::warn!(
                "no SceneManifest within {:?}; using fallback blake3={fallback_blake3}",
                MANIFEST_WAIT
            );
            fallback_blake3
        }
    };

    let bytes = load_blob_bytes(&assets_url, &blob_blake3, &auth_token)?;
    verify_blake3(&bytes, &blob_blake3).context("reject blob: BLAKE3 mismatch")?;
    let mesh = load_mesh_from_glb(&bytes).context("decode GLB")?;
    log::info!(
        "mesh ready: {} verts, {} indices, radius={:.3}",
        mesh.positions.len(),
        mesh.indices.len(),
        mesh.radius
    );

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        mesh,
        shared,
        window: None,
        gpu: None,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}
