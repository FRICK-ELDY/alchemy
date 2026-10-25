//! Shared scene GPU (Coin + floor). Surface-agnostic: render into any color+depth views.
//!
//! Guarantees: draw with caller-supplied view/proj.
//! Non-guarantees: XR swapchain ownership, system_ui layout.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use winit::window::Window;

use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

use crate::mesh::CpuMesh;

pub const MESH_ENTITY_ID: u32 = 1;
/// Host-authoritative player root (x0009 Noclip camera position).
pub const USER_ROOT_ENTITY_ID: u32 = 3;
/// Half-extent of Desktop floor (meters). Larger so loco is obvious.
const FLOOR_EXTENT: f32 = 20.0;
/// Default eye height if user_root not yet received.
const DEFAULT_EYE_Y: f32 = 1.6;

#[derive(Clone, Copy, Debug, Default)]
pub struct EntityPose {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// Desktop look (client-local). Position comes from host user_root.
#[derive(Clone, Copy, Debug)]
pub struct CameraLook {
    pub yaw: f32,
    pub pitch: f32,
}

impl Default for CameraLook {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
        }
    }
}

/// Shared locomotion intent published on Zenoh (mode-agnostic).
#[derive(Clone, Copy, Debug, Default)]
pub struct MoveIntent {
    pub move_x: f32,
    pub move_y: f32,
    pub move_z: f32,
    pub buttons: u32,
    /// When true (dash open), publisher forces zeros.
    pub suppress: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MeshVertex {
    position: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Uniforms {
    pub mvp: [[f32; 4]; 4],
    pub color: [f32; 4],
}

pub struct SharedState {
    pub entities: Mutex<std::collections::HashMap<u32, EntityPose>>,
    pub look: Mutex<CameraLook>,
    pub intent: Mutex<MoveIntent>,
}

pub struct SceneGpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub pipeline: wgpu::RenderPipeline,
    pub surface_format: wgpu::TextureFormat,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    floor_vertex_buffer: wgpu::Buffer,
    floor_index_buffer: wgpu::Buffer,
    floor_index_count: u32,
    floor_uniform_buffer: wgpu::Buffer,
    floor_bind_group: wgpu::BindGroup,
    coin_uniform_buffer: wgpu::Buffer,
    coin_bind_group: wgpu::BindGroup,
    mesh_radius: f32,
    shared: Arc<SharedState>,
    last_pose_log: Instant,
}

impl SceneGpu {
    pub fn from_device(
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        mesh: &CpuMesh,
        shared: Arc<SharedState>,
    ) -> Result<Self> {
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
                    format: surface_format,
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

        let e = FLOOR_EXTENT;
        let floor_verts = [
            MeshVertex {
                position: [-e, 0.0, -e],
            },
            MeshVertex {
                position: [e, 0.0, -e],
            },
            MeshVertex {
                position: [e, 0.0, e],
            },
            MeshVertex {
                position: [-e, 0.0, e],
            },
        ];
        let floor_indices: [u32; 6] = [0, 1, 2, 0, 2, 3];
        let floor_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("floor-vertices"),
            contents: bytemuck::cast_slice(&floor_verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let floor_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("floor-indices"),
            contents: bytemuck::cast_slice(&floor_indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let floor_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("floor-uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let coin_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("coin-uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let floor_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("floor-bg"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: floor_uniform_buffer.as_entire_binding(),
            }],
        });
        let coin_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("coin-bg"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: coin_uniform_buffer.as_entire_binding(),
            }],
        });

        Ok(Self {
            device,
            queue,
            pipeline,
            surface_format,
            vertex_buffer,
            index_buffer,
            index_count: mesh.indices.len() as u32,
            floor_vertex_buffer,
            floor_index_buffer,
            floor_index_count: floor_indices.len() as u32,
            floor_uniform_buffer,
            floor_bind_group,
            coin_uniform_buffer,
            coin_bind_group,
            mesh_radius: mesh.radius,
            shared,
            last_pose_log: Instant::now(),
        })
    }

    pub fn coin_pose(&self) -> EntityPose {
        self.shared
            .entities
            .lock()
            .ok()
            .and_then(|m| m.get(&MESH_ENTITY_ID).copied())
            .unwrap_or_default()
    }

    pub fn user_root_pose(&self) -> EntityPose {
        self.shared
            .entities
            .lock()
            .ok()
            .and_then(|m| m.get(&USER_ROOT_ENTITY_ID).copied())
            .unwrap_or(EntityPose {
                x: 0.0,
                y: DEFAULT_EYE_Y,
                z: 6.0,
            })
    }

    pub fn look(&self) -> CameraLook {
        self.shared.look.lock().ok().map(|g| *g).unwrap_or_default()
    }

    pub fn maybe_log_pose(&mut self) {
        let coin = self.coin_pose();
        let user = self.user_root_pose();
        if self.last_pose_log.elapsed() >= Duration::from_secs(1) {
            log::info!(
                "poses: coin=({:.3},{:.3},{:.3}) user_root=({:.3},{:.3},{:.3})",
                coin.x,
                coin.y,
                coin.z,
                user.x,
                user.y,
                user.z
            );
            self.last_pose_log = Instant::now();
        }
    }

    /// Desktop FPS camera at user_root with client-local yaw/pitch.
    pub fn desktop_view_proj(eye: EntityPose, look: CameraLook, aspect: f32) -> (Mat4, Mat4) {
        let yaw = look.yaw;
        let pitch = look.pitch.clamp(-1.54, 1.54);
        let forward = Vec3::new(
            yaw.sin() * pitch.cos(),
            pitch.sin(),
            -yaw.cos() * pitch.cos(),
        )
        .normalize();
        let eye_pos = Vec3::new(eye.x, eye.y, eye.z);
        let view = Mat4::look_at_rh(eye_pos, eye_pos + forward, Vec3::Y);
        let proj = Mat4::perspective_rh(70.0_f32.to_radians(), aspect, 0.05, 200.0);
        (view, proj)
    }

    pub fn draw_world(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        color_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        view: Mat4,
        proj: Mat4,
        load_op: wgpu::LoadOp<wgpu::Color>,
    ) {
        let pose = self.coin_pose();
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: load_op,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
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

            let floor_uniforms = Uniforms {
                mvp: (proj * view).to_cols_array_2d(),
                color: [0.22, 0.24, 0.28, 1.0],
            };
            self.queue.write_buffer(
                &self.floor_uniform_buffer,
                0,
                bytemuck::bytes_of(&floor_uniforms),
            );
            pass.set_bind_group(0, &self.floor_bind_group, &[]);
            pass.set_vertex_buffer(0, self.floor_vertex_buffer.slice(..));
            pass.set_index_buffer(self.floor_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.floor_index_count, 0, 0..1);

            const PHYS_BALL_RADIUS: f32 = 0.25;
            let visual_scale =
                (PHYS_BALL_RADIUS * 0.95 / self.mesh_radius.max(1e-3)).clamp(0.02, 4.0);
            let model = Mat4::from_translation(Vec3::new(pose.x, pose.y, pose.z))
                * Mat4::from_rotation_x(0.25)
                * Mat4::from_scale(Vec3::splat(visual_scale));
            let height_t = (pose.y / 5.0).clamp(0.0, 1.0);
            let coin_color = [
                0.95 * height_t + 0.15,
                0.75 * (1.0 - height_t) + 0.25,
                0.2,
                1.0,
            ];
            let coin_uniforms = Uniforms {
                mvp: (proj * view * model).to_cols_array_2d(),
                color: coin_color,
            };
            self.queue.write_buffer(
                &self.coin_uniform_buffer,
                0,
                bytemuck::bytes_of(&coin_uniforms),
            );
            pass.set_bind_group(0, &self.coin_bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.index_count, 0, 0..1);
        }
    }
}

pub fn create_depth_view(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
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

pub async fn create_wgpu_for_window(
    window: Arc<Window>,
) -> Result<(
    wgpu::Instance,
    wgpu::Surface<'static>,
    wgpu::Adapter,
    wgpu::Device,
    wgpu::Queue,
    wgpu::SurfaceConfiguration,
    wgpu::TextureFormat,
)> {
    let size = window.inner_size();
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL,
        ..Default::default()
    });
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
    Ok((instance, surface, adapter, device, queue, config, format))
}
