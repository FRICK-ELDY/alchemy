//! Shared scene GPU (Coin + floor + grabbable boxes + remote user_roots).
//!
//! MoveIntent carries Grab hold fields for x0011 (host kinematic follow).
//! `CameraLook` is Desktop-local (x0012); not published on Zenoh.
//!
//! Guarantees: draw with caller-supplied view/proj; boxes 10..18; remote roots 3..6 (x0015).
//! Non-guarantees: XR swapchain ownership, system_ui layout, avatar GLB.
//! Leave / liveliness token ownership lives here; put/undeclare is `main::leave_room`.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use winit::window::Window;

use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

use alchemy_protocol::user_root_entity_last;

use crate::auth::AuthGate;
use crate::mesh::CpuMesh;

pub const MESH_ENTITY_ID: u32 = 1;
/// Host-authoritative player roots (protocol SSoT; x0015 Presence).
pub use alchemy_protocol::USER_ROOT_ENTITY_FIRST;
/// Sentinel: Presence has not assigned a local user_root yet.
pub const LOCAL_USER_ROOT_UNSET: u32 = 0;
/// First entity id for the 3×3 grabbable grid (matches host x0010).
pub const BOX_ENTITY_FIRST: u32 = 10;
pub const BOX_COUNT: usize = 9;
/// Half-extent of each cuboid (meters). Matches host `BOX_HALF_EXTENT`.
pub const BOX_HALF_EXTENT: f32 = 0.25;
/// Placeholder size for remote player (meters half-extent).
pub const REMOTE_ROOT_HALF_EXTENT: f32 = 0.25;

pub fn is_user_root_entity(id: u32) -> bool {
    (USER_ROOT_ENTITY_FIRST..=user_root_entity_last()).contains(&id)
}
/// Half-extent of Desktop floor (meters). Larger so loco is obvious.
const FLOOR_EXTENT: f32 = 20.0;
/// Default eye height if user_root not yet received.
const DEFAULT_EYE_Y: f32 = 1.6;

#[derive(Clone, Copy, Debug)]
pub struct EntityPose {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Default for EntityPose {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        }
    }
}

impl EntityPose {
    pub fn position(self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }

    pub fn color(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    pub fn is_grabbable_box(id: u32) -> bool {
        (BOX_ENTITY_FIRST..BOX_ENTITY_FIRST + BOX_COUNT as u32).contains(&id)
    }

    pub fn is_user_root(id: u32) -> bool {
        is_user_root_entity(id)
    }
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

/// Shared locomotion + Grab hold intent published on Zenoh (mode-agnostic).
#[derive(Clone, Copy, Debug, Default)]
pub struct MoveIntent {
    pub move_x: f32,
    pub move_y: f32,
    pub move_z: f32,
    pub buttons: u32,
    /// When true (dash open), publisher forces move zeros (Grab also cleared).
    pub suppress: bool,
    /// Grabbable entity while GRAB held (`0` = none). XR grip or Desktop RMB (x0013).
    pub grab_entity_id: u32,
    /// World-space aim origin (meters): XR controller aim / Desktop eye (user_root).
    pub aim_x: f32,
    pub aim_y: f32,
    pub aim_z: f32,
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
    /// Local camera / aim follows this host entity (`LOCAL_USER_ROOT_UNSET` until Presence).
    pub local_user_root: AtomicU32,
    /// `ROOM_ID` used for Zenoh keys (x0017); shared with host_control shutdown publish.
    pub room_id: Arc<str>,
    pub auth_url: Arc<str>,
    pub world_url: Arc<str>,
    pub assets_url: Arc<str>,
    /// Auth + Room Token gate (x0018). Intent publishes only when `room_token` is set.
    pub auth: Mutex<AuthGate>,
    /// Zenoh liveliness token while joined (x0019). Drop/undeclare on leave.
    pub liveliness: Mutex<Option<zenoh::liveliness::LivelinessToken>>,
    pub entities: Mutex<std::collections::HashMap<u32, EntityPose>>,
    pub look: Mutex<CameraLook>,
    pub intent: Mutex<MoveIntent>,
}

impl SharedState {
    pub fn local_user_root(&self) -> u32 {
        self.local_user_root.load(Ordering::Relaxed)
    }

    pub fn set_local_user_root(&self, entity_id: u32) {
        self.local_user_root.store(entity_id, Ordering::Relaxed)
    }

    pub fn leave_room_local(&self) {
        self.set_local_user_root(LOCAL_USER_ROOT_UNSET);
        if let Ok(mut intent) = self.intent.lock() {
            *intent = MoveIntent {
                suppress: intent.suppress,
                ..Default::default()
            };
        }
        if let Ok(mut auth) = self.auth.lock() {
            auth.room_token = None;
            auth.local_sub = None;
        }
        if let Ok(mut token) = self.liveliness.lock() {
            *token = None;
        }
    }
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
    box_vertex_buffer: wgpu::Buffer,
    box_index_buffer: wgpu::Buffer,
    box_index_count: u32,
    /// One uniform+bind group per box slot — wgpu cannot safely rewrite one UBO mid-pass.
    box_uniform_buffers: Vec<wgpu::Buffer>,
    box_bind_groups: Vec<wgpu::BindGroup>,
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

        let (box_verts, box_indices) = unit_cube_mesh();
        let box_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("box-vertices"),
            contents: bytemuck::cast_slice(&box_verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let box_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("box-indices"),
            contents: bytemuck::cast_slice(&box_indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let mut box_uniform_buffers = Vec::with_capacity(BOX_COUNT);
        let mut box_bind_groups = Vec::with_capacity(BOX_COUNT);
        for i in 0..BOX_COUNT {
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("box-uniforms-{i}")),
                size: std::mem::size_of::<Uniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("box-bg-{i}")),
                layout: &bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buf.as_entire_binding(),
                }],
            });
            box_uniform_buffers.push(buf);
            box_bind_groups.push(bg);
        }

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
            box_vertex_buffer,
            box_index_buffer,
            box_index_count: box_indices.len() as u32,
            box_uniform_buffers,
            box_bind_groups,
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
        let id = self.shared.local_user_root();
        if id == LOCAL_USER_ROOT_UNSET {
            return EntityPose {
                x: 0.0,
                y: DEFAULT_EYE_Y,
                z: 6.0,
                ..Default::default()
            };
        }
        self.shared
            .entities
            .lock()
            .ok()
            .and_then(|m| m.get(&id).copied())
            .unwrap_or(EntityPose {
                x: 0.0,
                y: DEFAULT_EYE_Y,
                z: 6.0,
                ..Default::default()
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

    #[allow(clippy::too_many_arguments)] // render pass bundling; highlight is Desktop-only pad
    pub fn draw_world(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        color_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        view: Mat4,
        proj: Mat4,
        load_op: wgpu::LoadOp<wgpu::Color>,
        // Desktop aim / held Grab highlight (entity id), if any.
        highlight_entity: Option<u32>,
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

            // Grabbable boxes (x0010) + remote user_roots (x0014): unit cube mesh.
            // Per-slot UBOs — rewriting one buffer inside the pass would show only the last cube.
            let local_root = self.shared.local_user_root();
            type PoseList = Vec<(u32, EntityPose)>;
            let (boxes, remotes): (PoseList, PoseList) = self
                .shared
                .entities
                .lock()
                .ok()
                .map(|m| {
                    let boxes: PoseList = m
                        .iter()
                        .filter(|(id, _)| EntityPose::is_grabbable_box(**id))
                        .map(|(id, p)| (*id, *p))
                        .collect();
                    let remotes: PoseList = m
                        .iter()
                        .filter(|(id, _)| {
                            EntityPose::is_user_root(**id)
                                && (local_root == LOCAL_USER_ROOT_UNSET || **id != local_root)
                        })
                        .map(|(id, p)| (*id, *p))
                        .collect();
                    (boxes, remotes)
                })
                .unwrap_or_default();
            pass.set_vertex_buffer(0, self.box_vertex_buffer.slice(..));
            pass.set_index_buffer(self.box_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            for (id, pose) in boxes {
                let slot = (id.saturating_sub(BOX_ENTITY_FIRST) as usize).min(BOX_COUNT - 1);
                let model = Mat4::from_scale_rotation_translation(
                    Vec3::splat(BOX_HALF_EXTENT * 2.0),
                    glam::Quat::IDENTITY,
                    pose.position(),
                );
                let mut color = pose.color();
                if highlight_entity == Some(id) {
                    // Brighten so look-aim / Grab target is obvious without a crosshair.
                    color = [
                        (color[0] * 0.35 + 0.65).min(1.0),
                        (color[1] * 0.35 + 0.65).min(1.0),
                        (color[2] * 0.35 + 0.65).min(1.0),
                        1.0,
                    ];
                }
                let box_uniforms = Uniforms {
                    mvp: (proj * view * model).to_cols_array_2d(),
                    color,
                };
                self.queue.write_buffer(
                    &self.box_uniform_buffers[slot],
                    0,
                    bytemuck::bytes_of(&box_uniforms),
                );
                pass.set_bind_group(0, &self.box_bind_groups[slot], &[]);
                pass.draw_indexed(0..self.box_index_count, 0, 0..1);
            }
            // Remote players: reuse box mesh + last UBO slots (BOX_COUNT-2 / BOX_COUNT-1).
            for (i, (_id, pose)) in remotes.iter().enumerate() {
                let slot = (BOX_COUNT - 1 - i).min(BOX_COUNT - 1);
                let model = Mat4::from_scale_rotation_translation(
                    Vec3::splat(REMOTE_ROOT_HALF_EXTENT * 2.0),
                    glam::Quat::IDENTITY,
                    pose.position(),
                );
                let box_uniforms = Uniforms {
                    mvp: (proj * view * model).to_cols_array_2d(),
                    color: pose.color(),
                };
                self.queue.write_buffer(
                    &self.box_uniform_buffers[slot],
                    0,
                    bytemuck::bytes_of(&box_uniforms),
                );
                pass.set_bind_group(0, &self.box_bind_groups[slot], &[]);
                pass.draw_indexed(0..self.box_index_count, 0, 0..1);
            }
        }
    }
}

/// Axis-aligned unit cube centered at origin (half-extent 0.5 → scale by 2*half).
fn unit_cube_mesh() -> (Vec<MeshVertex>, Vec<u32>) {
    let positions = [
        [-0.5, -0.5, -0.5],
        [0.5, -0.5, -0.5],
        [0.5, 0.5, -0.5],
        [-0.5, 0.5, -0.5],
        [-0.5, -0.5, 0.5],
        [0.5, -0.5, 0.5],
        [0.5, 0.5, 0.5],
        [-0.5, 0.5, 0.5],
    ];
    let indices = [
        0, 1, 2, 0, 2, 3, // -Z
        4, 6, 5, 4, 7, 6, // +Z
        0, 4, 5, 0, 5, 1, // -Y
        3, 2, 6, 3, 6, 7, // +Y
        0, 3, 7, 0, 7, 4, // -X
        1, 5, 6, 1, 6, 2, // +X
    ];
    (
        positions
            .iter()
            .map(|p| MeshVertex { position: *p })
            .collect(),
        indices.to_vec(),
    )
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
