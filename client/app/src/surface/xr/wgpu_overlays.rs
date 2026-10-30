//! XR-only wgpu overlays: dash UI quad, laser, controller render models (x0051).
//!
//! Guarantees: CPU canvas → `write_texture`; draw order world→controllers→dash→laser
//! is enforced by the session caller; no `openxr` types here.
//! Non-guarantees: Desktop `UiGpu` screen-space path; PBR; multiview.

use anyhow::Result;
use bytemuck::{Pod, Zeroable};
use glam::Mat4;

use crate::mesh::{CpuSubmesh, CpuTexturedMesh};

pub const LASER_COLOR: [f32; 4] = [0.15, 0.95, 1.0, 1.0];

const TEX_WGSL: &str = r#"
struct Uniforms {
    mvp: mat4x4<f32>,
    color: vec4<f32>,
}
@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
}
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.clip = uniforms.mvp * vec4<f32>(in.pos, 1.0);
    out.uv = in.uv;
    return out;
}
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(tex, samp, in.uv);
    return t * uniforms.color;
}
"#;

const FLAT_WGSL: &str = r#"
struct Uniforms {
    mvp: mat4x4<f32>,
    color: vec4<f32>,
}
@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct VsIn {
    @location(0) pos: vec3<f32>,
}
struct VsOut {
    @builtin(position) clip: vec4<f32>,
}
@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.clip = uniforms.mvp * vec4<f32>(in.pos, 1.0);
    return out;
}
@fragment
fn fs_main(_in: VsOut) -> @location(0) vec4<f32> {
    return uniforms.color;
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct OverlayUniforms {
    mvp: [[f32; 4]; 4],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TexVertex {
    pos: [f32; 3],
    uv: [f32; 2],
}

struct GpuMesh {
    vbo: wgpu::Buffer,
    ibo: wgpu::Buffer,
    index_count: u32,
}

struct TexturedSubGpu {
    mesh: GpuMesh,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    factor: [f32; 4],
    node_index: usize,
    _texture: wgpu::Texture,
    _view: wgpu::TextureView,
}

pub struct GpuController {
    submeshes: Vec<TexturedSubGpu>,
}

/// Pipelines + reusable UI / laser geometry for XR overlays.
pub struct XrOverlays {
    device: wgpu::Device,
    queue: wgpu::Queue,
    tex_pipeline: wgpu::RenderPipeline,
    flat_pipeline: wgpu::RenderPipeline,
    tex_bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    white_tex: wgpu::Texture,
    white_view: wgpu::TextureView,
    ui_mesh: GpuMesh,
    ui_uniform: wgpu::Buffer,
    ui_texture: wgpu::Texture,
    _ui_view: wgpu::TextureView,
    ui_bg: wgpu::BindGroup,
    ui_w: u32,
    ui_h: u32,
    laser_mesh: GpuMesh,
    laser_uniform: wgpu::Buffer,
    laser_bg: wgpu::BindGroup,
}

impl XrOverlays {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        ui_w: u32,
        ui_h: u32,
    ) -> Result<Self> {
        let device = device.clone();
        let queue = queue.clone();

        let tex_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("xr-overlay-tex-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let flat_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("xr-overlay-flat-bgl"),
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

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("xr-overlay-samp"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let tex_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xr-overlay-tex"),
            source: wgpu::ShaderSource::Wgsl(TEX_WGSL.into()),
        });
        let flat_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xr-overlay-flat"),
            source: wgpu::ShaderSource::Wgsl(FLAT_WGSL.into()),
        });

        let tex_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xr-overlay-tex-pl"),
            bind_group_layouts: &[&tex_bgl],
            push_constant_ranges: &[],
        });
        let flat_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xr-overlay-flat-pl"),
            bind_group_layouts: &[&flat_bgl],
            push_constant_ranges: &[],
        });

        // Match SceneGpu (`Less`) — shared depth Load; LessEqual caused ash/SceneGpu mismatch.
        let depth_stencil = Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        });

        let tex_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("xr-overlay-tex-pipe"),
            layout: Some(&tex_pl),
            vertex: wgpu::VertexState {
                module: &tex_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<TexVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &tex_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: depth_stencil.clone(),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let flat_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("xr-overlay-flat-pipe"),
            layout: Some(&flat_pl),
            vertex: wgpu::VertexState {
                module: &flat_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &flat_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let (white_tex, white_view) =
            create_rgba_texture(&device, &queue, &[255, 255, 255, 255], 1, 1);

        let ui_mesh = upload_textured_mesh(&device, &ui_plane_submesh())?;
        let blank = vec![0u8; (ui_w * ui_h * 4) as usize];
        let (ui_texture, ui_view) = create_rgba_texture(&device, &queue, &blank, ui_w, ui_h);
        let ui_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("xr-ui-ubo"),
            size: std::mem::size_of::<OverlayUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ui_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xr-ui-bg"),
            layout: &tex_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ui_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&ui_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let laser_mesh = upload_pos_mesh(&device, &laser_beam_submesh())?;
        let laser_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("xr-laser-ubo"),
            size: std::mem::size_of::<OverlayUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let laser_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xr-laser-bg"),
            layout: &flat_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: laser_uniform.as_entire_binding(),
            }],
        });

        Ok(Self {
            device,
            queue,
            tex_pipeline,
            flat_pipeline,
            tex_bgl,
            sampler,
            white_tex,
            white_view,
            ui_mesh,
            ui_uniform,
            ui_texture,
            _ui_view: ui_view,
            ui_bg,
            ui_w,
            ui_h,
            laser_mesh,
            laser_uniform,
            laser_bg,
        })
    }

    pub fn upload_ui_canvas(&self, pixels: &[u8], w: u32, h: u32) {
        if w != self.ui_w || h != self.ui_h || pixels.len() < (w * h * 4) as usize {
            log::warn!(
                "XR UI canvas size mismatch (got {w}x{h}, expect {}x{})",
                self.ui_w,
                self.ui_h
            );
            return;
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.ui_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
    }

    pub fn upload_controller(&self, mesh: &CpuTexturedMesh) -> Result<GpuController> {
        let mut submeshes = Vec::with_capacity(mesh.submeshes.len());
        for sub in &mesh.submeshes {
            submeshes.push(self.upload_textured_sub(sub)?);
        }
        Ok(GpuController { submeshes })
    }

    fn upload_textured_sub(&self, sub: &CpuSubmesh) -> Result<TexturedSubGpu> {
        let mesh = upload_textured_mesh(&self.device, sub)?;
        let (tex, view) = match &sub.base_color {
            Some(img) => create_rgba_texture(
                &self.device,
                &self.queue,
                &img.pixels,
                img.width,
                img.height,
            ),
            None => (self.white_tex.clone(), self.white_view.clone()),
        };
        let uniform = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("xr-ctrl-ubo"),
            size: std::mem::size_of::<OverlayUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xr-ctrl-bg"),
            layout: &self.tex_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        Ok(TexturedSubGpu {
            mesh,
            uniform,
            bind_group,
            factor: sub.base_color_factor,
            node_index: sub.node_index,
            _texture: tex,
            _view: view,
        })
    }

    /// One pass for all controllers; caller supplies slices (no owned Vec) via `emit`.
    pub fn draw_controllers(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        color_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        view_proj: Mat4,
        mut emit: impl FnMut(&mut dyn FnMut(&GpuController, Mat4, &[Mat4], &[bool])),
    ) {
        let mut pass = begin_load_pass(encoder, color_view, depth_view, "xr-overlay-ctrl");
        pass.set_pipeline(&self.tex_pipeline);
        emit(&mut |gpu, root, worlds, visible| {
            for sub in &gpu.submeshes {
                if !visible.get(sub.node_index).copied().unwrap_or(false) {
                    continue;
                }
                let node_world = worlds
                    .get(sub.node_index)
                    .copied()
                    .unwrap_or(Mat4::IDENTITY);
                let mvp = view_proj * root * node_world;
                write_uniforms(&self.queue, &sub.uniform, mvp, sub.factor);
                pass.set_bind_group(0, &sub.bind_group, &[]);
                pass.set_vertex_buffer(0, sub.mesh.vbo.slice(..));
                pass.set_index_buffer(sub.mesh.ibo.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..sub.mesh.index_count, 0, 0..1);
            }
        });
    }

    pub fn draw_dash(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        color_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        view_proj: Mat4,
        model: Mat4,
    ) {
        let mut pass = begin_load_pass(encoder, color_view, depth_view, "xr-overlay-dash");
        pass.set_pipeline(&self.tex_pipeline);
        write_uniforms(&self.queue, &self.ui_uniform, view_proj * model, [1.0; 4]);
        pass.set_bind_group(0, &self.ui_bg, &[]);
        pass.set_vertex_buffer(0, self.ui_mesh.vbo.slice(..));
        pass.set_index_buffer(self.ui_mesh.ibo.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.ui_mesh.index_count, 0, 0..1);
    }

    pub fn draw_laser(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        color_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        view_proj: Mat4,
        model: Mat4,
    ) {
        let mut pass = begin_load_pass(encoder, color_view, depth_view, "xr-overlay-laser");
        pass.set_pipeline(&self.flat_pipeline);
        write_uniforms(
            &self.queue,
            &self.laser_uniform,
            view_proj * model,
            LASER_COLOR,
        );
        pass.set_bind_group(0, &self.laser_bg, &[]);
        pass.set_vertex_buffer(0, self.laser_mesh.vbo.slice(..));
        pass.set_index_buffer(self.laser_mesh.ibo.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.laser_mesh.index_count, 0, 0..1);
    }
}

fn begin_load_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    color_view: &'a wgpu::TextureView,
    depth_view: &'a wgpu::TextureView,
    label: &str,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: color_view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth_view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
    })
}

fn write_uniforms(queue: &wgpu::Queue, buf: &wgpu::Buffer, mvp: Mat4, color: [f32; 4]) {
    let u = OverlayUniforms {
        mvp: mvp.to_cols_array_2d(),
        color,
    };
    queue.write_buffer(buf, 0, bytemuck::bytes_of(&u));
}

fn create_rgba_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pixels: &[u8],
    w: u32,
    h: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("xr-overlay-rgba"),
        size: wgpu::Extent3d {
            width: w.max(1),
            height: h.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w.max(1) * 4),
            rows_per_image: Some(h.max(1)),
        },
        wgpu::Extent3d {
            width: w.max(1),
            height: h.max(1),
            depth_or_array_layers: 1,
        },
    );
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

fn upload_textured_mesh(device: &wgpu::Device, sub: &CpuSubmesh) -> Result<GpuMesh> {
    use wgpu::util::DeviceExt;
    let mut verts = Vec::with_capacity(sub.positions.len());
    for (i, p) in sub.positions.iter().enumerate() {
        let uv = sub.uvs.get(i).copied().unwrap_or([0.0, 0.0]);
        verts.push(TexVertex { pos: *p, uv });
    }
    let vbo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("xr-overlay-tex-vbo"),
        contents: bytemuck::cast_slice(&verts),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let ibo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("xr-overlay-tex-ibo"),
        contents: bytemuck::cast_slice(&sub.indices),
        usage: wgpu::BufferUsages::INDEX,
    });
    Ok(GpuMesh {
        vbo,
        ibo,
        index_count: sub.indices.len() as u32,
    })
}

fn upload_pos_mesh(device: &wgpu::Device, sub: &CpuSubmesh) -> Result<GpuMesh> {
    use wgpu::util::DeviceExt;
    let vbo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("xr-overlay-pos-vbo"),
        contents: bytemuck::cast_slice(&sub.positions),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let ibo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("xr-overlay-pos-ibo"),
        contents: bytemuck::cast_slice(&sub.indices),
        usage: wgpu::BufferUsages::INDEX,
    });
    Ok(GpuMesh {
        vbo,
        ibo,
        index_count: sub.indices.len() as u32,
    })
}

/// Unit quad in local XY (scaled by `dash_quad_model`). UV: top-left origin.
fn ui_plane_submesh() -> CpuSubmesh {
    CpuSubmesh {
        positions: vec![
            [-0.5, -0.5, 0.0],
            [0.5, -0.5, 0.0],
            [0.5, 0.5, 0.0],
            [-0.5, 0.5, 0.0],
        ],
        uvs: vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
        indices: vec![0, 1, 2, 0, 2, 3],
        base_color: None,
        base_color_factor: [1.0, 1.0, 1.0, 1.0],
        node_index: 0,
    }
}

/// Unit box along local -Z from z=0 to z=-1.
fn laser_beam_submesh() -> CpuSubmesh {
    let positions = vec![
        [-0.5, -0.5, 0.0],
        [0.5, -0.5, 0.0],
        [0.5, 0.5, 0.0],
        [-0.5, 0.5, 0.0],
        [-0.5, -0.5, -1.0],
        [0.5, -0.5, -1.0],
        [0.5, 0.5, -1.0],
        [-0.5, 0.5, -1.0],
    ];
    let indices = vec![
        0, 1, 2, 0, 2, 3, 4, 6, 5, 4, 7, 6, 1, 5, 6, 1, 6, 2, 0, 3, 7, 0, 7, 4, 3, 2, 6, 3, 6, 7,
        0, 4, 5, 0, 5, 1,
    ];
    CpuSubmesh {
        positions,
        uvs: vec![[0.0, 0.0]; 8],
        indices,
        base_color: None,
        base_color_factor: LASER_COLOR,
        node_index: 0,
    }
}
