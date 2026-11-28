//! Uber-SDF wgpu pipeline for Spatial Lens panels (x00ba / x00ed).
//!
//! Guarantees: naga-valid WGSL; upload headers/prims; cached bind group; α without depth write;
//! Tier-1 Static MSDF atlas bind (texture + sampler).
//! Non-guarantees: XR CompositionLayerQuad; multi-eye UBO split; CJK dynamic atlas.

#![forbid(unsafe_code)]

use anyhow::Result;
use bytemuck::{Pod, Zeroable};
use glam::Mat4;

use super::msdf::{bake_static_atlas, ATLAS_SIZE};
use super::prim::{GpuPanelHeader, GpuUiPrim};
use super::shaders::UBER_SDF_WGSL;

pub const VS_ENTRY: &str = "vs_panel_quad";
pub const FS_ENTRY: &str = "fs_uber_sdf";

#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FrameUniforms {
    view_proj: [[f32; 4]; 4],
}

/// Parse + validate Uber-SDF WGSL (wgpu re-exported naga; no new dep).
#[cfg_attr(not(test), allow(dead_code))]
pub fn validate_uber_sdf_wgsl() -> Result<(), String> {
    let module = wgpu::naga::front::wgsl::parse_str(UBER_SDF_WGSL).map_err(|e| e.to_string())?;
    for name in [VS_ENTRY, FS_ENTRY] {
        if !module.entry_points.iter().any(|ep| ep.name == name) {
            return Err(format!("missing entry point {name}"));
        }
    }
    let mut validator = wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::all(),
    );
    validator.validate(&module).map_err(|e| e.to_string())?;
    Ok(())
}

pub struct SpatialLensGpu {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    frame_buf: wgpu::Buffer,
    panel_buf: wgpu::Buffer,
    prim_buf: wgpu::Buffer,
    msdf_view: wgpu::TextureView,
    msdf_samp: wgpu::Sampler,
    bind_group: wgpu::BindGroup,
    panel_cap: u64,
    prim_cap: u64,
    panel_count: u32,
}

impl SpatialLensGpu {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
    ) -> Result<Self> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spatial-lens-uber-sdf"),
            source: wgpu::ShaderSource::Wgsl(UBER_SDF_WGSL.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("spatial-lens-bgl"),
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
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("spatial-lens-pl"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("spatial-lens-rp"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some(VS_ENTRY),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(FS_ENTRY),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                // α blend: do not punch holes in world geometry behind soft edges.
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let frame_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("spatial-lens-frame"),
            size: std::mem::size_of::<FrameUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (msdf_view, msdf_samp) = create_msdf_atlas(device, queue);

        let panel_cap = 4u64;
        let prim_cap = 256u64;
        let panel_buf = empty_storage(
            device,
            "spatial-lens-panels",
            panel_cap * std::mem::size_of::<GpuPanelHeader>() as u64,
        );
        let prim_buf = empty_storage(
            device,
            "spatial-lens-prims",
            prim_cap * std::mem::size_of::<GpuUiPrim>() as u64,
        );
        let bind_group = make_bind_group(
            device,
            &bind_group_layout,
            &frame_buf,
            &panel_buf,
            &prim_buf,
            &msdf_view,
            &msdf_samp,
        );

        Ok(Self {
            pipeline,
            bind_group_layout,
            frame_buf,
            panel_buf,
            prim_buf,
            msdf_view,
            msdf_samp,
            bind_group,
            panel_cap,
            prim_cap,
            panel_count: 0,
        })
    }

    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        headers: &[GpuPanelHeader],
        prims: &[GpuUiPrim],
    ) {
        self.ensure_caps(device, headers.len() as u64, prims.len() as u64);
        if !headers.is_empty() {
            queue.write_buffer(&self.panel_buf, 0, bytemuck::cast_slice(headers));
        }
        if !prims.is_empty() {
            queue.write_buffer(&self.prim_buf, 0, bytemuck::cast_slice(prims));
        }
        self.panel_count = headers.len() as u32;
    }

    fn ensure_caps(&mut self, device: &wgpu::Device, panels: u64, prims: u64) {
        let panels = panels.max(1);
        let prims = prims.max(1);
        let mut rebuilt = false;
        if panels > self.panel_cap {
            self.panel_cap = panels.next_power_of_two();
            self.panel_buf = empty_storage(
                device,
                "spatial-lens-panels",
                self.panel_cap * std::mem::size_of::<GpuPanelHeader>() as u64,
            );
            rebuilt = true;
        }
        if prims > self.prim_cap {
            self.prim_cap = prims.next_power_of_two();
            self.prim_buf = empty_storage(
                device,
                "spatial-lens-prims",
                self.prim_cap * std::mem::size_of::<GpuUiPrim>() as u64,
            );
            rebuilt = true;
        }
        if rebuilt {
            self.bind_group = make_bind_group(
                device,
                &self.bind_group_layout,
                &self.frame_buf,
                &self.panel_buf,
                &self.prim_buf,
                &self.msdf_view,
                &self.msdf_samp,
            );
        }
    }

    pub fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        color_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        view: Mat4,
        proj: Mat4,
    ) {
        if self.panel_count == 0 {
            return;
        }
        let uniforms = FrameUniforms {
            view_proj: (proj * view).to_cols_array_2d(),
        };
        queue.write_buffer(&self.frame_buf, 0, bytemuck::bytes_of(&uniforms));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("spatial-lens-pass"),
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
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..6, 0..self.panel_count);
    }
}

fn create_msdf_atlas(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::TextureView, wgpu::Sampler) {
    let pixels = bake_static_atlas();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("spatial-lens-msdf-atlas"),
        size: wgpu::Extent3d {
            width: ATLAS_SIZE,
            height: ATLAS_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(ATLAS_SIZE * 4),
            rows_per_image: Some(ATLAS_SIZE),
        },
        wgpu::Extent3d {
            width: ATLAS_SIZE,
            height: ATLAS_SIZE,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let samp = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("spatial-lens-msdf-samp"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });
    (view, samp)
}

fn make_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    frame: &wgpu::Buffer,
    panels: &wgpu::Buffer,
    prims: &wgpu::Buffer,
    msdf_view: &wgpu::TextureView,
    msdf_samp: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("spatial-lens-bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: frame.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: panels.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: prims.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(msdf_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(msdf_samp),
            },
        ],
    })
}

fn empty_storage(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size.max(64),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uber_sdf_wgsl_naga_validates() {
        validate_uber_sdf_wgsl().expect("uber_sdf.wgsl must parse under naga");
    }

    #[test]
    fn uber_sdf_mentions_contract() {
        assert!(UBER_SDF_WGSL.contains("GpuUiPrim"));
        assert!(UBER_SDF_WGSL.contains("GpuPanelHeader"));
        assert!(UBER_SDF_WGSL.contains(VS_ENTRY));
        assert!(UBER_SDF_WGSL.contains(FS_ENTRY));
        assert!(UBER_SDF_WGSL.contains("sample_direct_number"));
        assert!(UBER_SDF_WGSL.contains("u64_div10"));
        assert!(UBER_SDF_WGSL.contains("payload.y"));
        // G0 glass: glow payload + LOD2 ShellFill + inner gradient.
        assert!(UBER_SDF_WGSL.contains("payload.z"));
        assert!(UBER_SDF_WGSL.contains("grad"));
        let lod2 = crate::spatial_lens::glass::lod2_wgsl_vec4_literal();
        assert!(
            UBER_SDF_WGSL.contains(&lod2),
            "uber_sdf LOD2 must match glass::LOD2_SHELL literal {lod2}"
        );
        let glow_stmt = crate::spatial_lens::glass::shell_glow_extent_wgsl_stmt();
        assert!(
            UBER_SDF_WGSL.contains(&glow_stmt),
            "uber_sdf soft-edge stmt must match glass::shell_glow_extent_wgsl_stmt `{glow_stmt}`"
        );
        // T1 Static MSDF.
        assert!(UBER_SDF_WGSL.contains("sample_msdf"));
        assert!(UBER_SDF_WGSL.contains("msdf_atlas"));
        assert!(UBER_SDF_WGSL.contains("@binding(3)"));
        assert!(UBER_SDF_WGSL.contains("@binding(4)"));
        assert!(UBER_SDF_WGSL.contains("kind == 1u"));
        assert!(UBER_SDF_WGSL.contains("direct_number_code_uv"));
        // Derivative-uniform sampling inside `sample_msdf` only (RoundedBox may use fwidth later).
        assert!(UBER_SDF_WGSL.contains("textureSampleLevel"));
        assert!(
            !UBER_SDF_WGSL.contains("textureSample(msdf_atlas"),
            "msdf must use textureSampleLevel, not textureSample"
        );
        let sample_msdf_fn = {
            let start = UBER_SDF_WGSL
                .find("fn sample_msdf(")
                .expect("sample_msdf fn");
            let rest = &UBER_SDF_WGSL[start..];
            let end = rest.find("\nfn ").unwrap_or(rest.len());
            &rest[..end]
        };
        assert!(
            !sample_msdf_fn.contains("fwidth("),
            "sample_msdf must not call fwidth (non-uniform prim paths)"
        );
        // Atlas contract literals must match msdf.rs (G0-style sync).
        use crate::spatial_lens::msdf::{
            atlas_aa_half_wgsl_const, atlas_ascii_base_wgsl_const, atlas_cell_wgsl_const,
            atlas_grid_wgsl_const, atlas_pad_wgsl_const, atlas_size_wgsl_const,
        };
        for lit in [
            atlas_size_wgsl_const(),
            atlas_cell_wgsl_const(),
            atlas_pad_wgsl_const(),
            atlas_grid_wgsl_const(),
            atlas_ascii_base_wgsl_const(),
            atlas_aa_half_wgsl_const(),
        ] {
            assert!(
                UBER_SDF_WGSL.contains(&lit),
                "uber_sdf missing atlas contract literal `{lit}`"
            );
        }
    }
}
