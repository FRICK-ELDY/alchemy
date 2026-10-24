//! Desktop presentation: rasterize retained Canvas to a texture, blit fullscreen with alpha.
//!
//! Future VR: reuse the same canvas texture on a world-space quad (not implemented).

use std::collections::HashMap;

use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use super::font;
use super::icons::{self, RgbaImage};
use super::layout::{Color, DrawCmd, LayoutTree};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct UiVertex {
    pos: [f32; 2],
    uv: [f32; 2],
}

pub struct UiGpu {
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    canvas_texture: wgpu::Texture,
    canvas_view: wgpu::TextureView,
    canvas_w: u32,
    canvas_h: u32,
    icons: HashMap<&'static str, RgbaImage>,
    pixels: Vec<u8>,
    dirty: bool,
}

impl UiGpu {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let icons = icons::load_bundled().context("load system_ui icons")?;
        let (canvas_texture, canvas_view) = create_canvas(device, width, height);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ui-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui-bg"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&canvas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ui-overlay"),
            source: wgpu::ShaderSource::Wgsl(include_str!("ui_overlay.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui-pl"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<UiVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // Fullscreen quad in clip space.
        let verts = [
            UiVertex {
                pos: [-1.0, -1.0],
                uv: [0.0, 1.0],
            },
            UiVertex {
                pos: [1.0, -1.0],
                uv: [1.0, 1.0],
            },
            UiVertex {
                pos: [1.0, 1.0],
                uv: [1.0, 0.0],
            },
            UiVertex {
                pos: [-1.0, -1.0],
                uv: [0.0, 1.0],
            },
            UiVertex {
                pos: [1.0, 1.0],
                uv: [1.0, 0.0],
            },
            UiVertex {
                pos: [-1.0, 1.0],
                uv: [0.0, 0.0],
            },
        ];
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ui-verts"),
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let w = width.max(1);
        let h = height.max(1);
        let pixels = vec![0u8; (w * h * 4) as usize];

        // Warm upload so bind group texture is valid.
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &canvas_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
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

        Ok(Self {
            pipeline,
            sampler,
            bind_group,
            vertex_buffer,
            canvas_texture,
            canvas_view,
            canvas_w: w,
            canvas_h: h,
            icons,
            pixels,
            dirty: true,
        })
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let w = width.max(1);
        let h = height.max(1);
        if w == self.canvas_w && h == self.canvas_h {
            return;
        }
        let (tex, view) = create_canvas(device, w, h);
        self.canvas_texture = tex;
        self.canvas_view = view;
        self.canvas_w = w;
        self.canvas_h = h;
        self.pixels.resize((w * h * 4) as usize, 0);
        self.dirty = true;
        self.rebind(device);
    }

    pub fn rebind(&mut self, device: &wgpu::Device) {
        let bgl = self.pipeline.get_bind_group_layout(0);
        self.bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui-bg"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.canvas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn rasterize_if_dirty(&mut self, queue: &wgpu::Queue, tree: &LayoutTree, open: bool) {
        if !self.dirty {
            return;
        }
        self.pixels.fill(0);
        if open {
            for cmd in &tree.draws {
                match cmd {
                    DrawCmd::Rect { rect, color } => fill_rect(
                        CanvasBlit {
                            buf: &mut self.pixels,
                            width: self.canvas_w,
                            height: self.canvas_h,
                        },
                        *rect,
                        *color,
                    ),
                    DrawCmd::Icon { rect, name } => {
                        if let Some(img) = self.icons.get(name) {
                            blit_icon(
                                CanvasBlit {
                                    buf: &mut self.pixels,
                                    width: self.canvas_w,
                                    height: self.canvas_h,
                                },
                                *rect,
                                img,
                            );
                        }
                    }
                    DrawCmd::Label {
                        rect,
                        text,
                        color,
                        scale,
                    } => {
                        let rgba = [
                            (color.r * 255.0) as u8,
                            (color.g * 255.0) as u8,
                            (color.b * 255.0) as u8,
                            (color.a * 255.0) as u8,
                        ];
                        font::blit_text(font::TextBlit {
                            buf: &mut self.pixels,
                            stride: self.canvas_w as usize,
                            height: self.canvas_h as usize,
                            origin_x: rect.x as i32,
                            origin_y: rect.y as i32,
                            text,
                            scale: scale.round() as i32,
                            rgba,
                        });
                    }
                }
            }
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.canvas_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &self.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.canvas_w * 4),
                rows_per_image: Some(self.canvas_h),
            },
            wgpu::Extent3d {
                width: self.canvas_w,
                height: self.canvas_h,
                depth_or_array_layers: 1,
            },
        );
        self.dirty = false;
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..6, 0..1);
    }
}

fn create_canvas(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ui-canvas"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

struct CanvasBlit<'a> {
    buf: &'a mut [u8],
    width: u32,
    height: u32,
}

fn fill_rect(canvas: CanvasBlit<'_>, rect: super::layout::Rect, color: Color) {
    let x0 = rect.x.floor().max(0.0) as i32;
    let y0 = rect.y.floor().max(0.0) as i32;
    let x1 = (rect.x + rect.w).ceil().min(canvas.width as f32) as i32;
    let y1 = (rect.y + rect.h).ceil().min(canvas.height as f32) as i32;
    let src = [
        (color.r * 255.0) as u8,
        (color.g * 255.0) as u8,
        (color.b * 255.0) as u8,
        (color.a * 255.0) as u8,
    ];
    let stride = canvas.width as usize;
    for py in y0..y1 {
        for px in x0..x1 {
            let i = (py as usize * stride + px as usize) * 4;
            blend_over(&mut canvas.buf[i..i + 4], src);
        }
    }
}

fn blit_icon(canvas: CanvasBlit<'_>, rect: super::layout::Rect, img: &RgbaImage) {
    let dst_w = rect.w.max(1.0) as i32;
    let dst_h = rect.h.max(1.0) as i32;
    let x0 = rect.x.floor() as i32;
    let y0 = rect.y.floor() as i32;
    let stride = canvas.width as usize;
    for dy in 0..dst_h {
        for dx in 0..dst_w {
            let px = x0 + dx;
            let py = y0 + dy;
            if px < 0 || py < 0 || px as u32 >= canvas.width || py as u32 >= canvas.height {
                continue;
            }
            let u = (dx as f32 + 0.5) / dst_w as f32;
            let v = (dy as f32 + 0.5) / dst_h as f32;
            let sx = ((u * img.width as f32) as u32).min(img.width - 1) as usize;
            let sy = ((v * img.height as f32) as u32).min(img.height - 1) as usize;
            let si = (sy * img.width as usize + sx) * 4;
            let src = [
                img.pixels[si],
                img.pixels[si + 1],
                img.pixels[si + 2],
                img.pixels[si + 3],
            ];
            if src[3] == 0 {
                continue;
            }
            let i = (py as usize * stride + px as usize) * 4;
            blend_over(&mut canvas.buf[i..i + 4], src);
        }
    }
}

fn blend_over(dst: &mut [u8], src: [u8; 4]) {
    let sa = src[3] as f32 / 255.0;
    let da = dst[3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        dst[0] = 0;
        dst[1] = 0;
        dst[2] = 0;
        dst[3] = 0;
        return;
    }
    for c in 0..3 {
        let s = src[c] as f32 / 255.0;
        let d = dst[c] as f32 / 255.0;
        let o = (s * sa + d * da * (1.0 - sa)) / out_a;
        dst[c] = (o * 255.0) as u8;
    }
    dst[3] = (out_a * 255.0) as u8;
}
