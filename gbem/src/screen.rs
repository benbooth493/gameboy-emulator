//! GPU-accelerated Game Boy screen rendering via a custom wgpu pipeline
//! embedded in egui as a paint callback.

use eframe::egui;
use eframe::egui_wgpu::{self, wgpu};
use gb_core::{SCREEN_H, SCREEN_W};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    params: [f32; 4],
}

/// Long-lived GPU resources, stored in egui's callback resources.
pub struct ScreenRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    tex_cur: wgpu::Texture,
    tex_prev: wgpu::Texture,
    uniforms: wgpu::Buffer,
}

impl ScreenRenderer {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gb_screen"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/gb_screen.wgsl").into()),
        });

        let make_tex = |label| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: SCREEN_W as u32,
                    height: SCREEN_H as u32,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let tex_cur = make_tex("gb_frame_cur");
        let tex_prev = make_tex("gb_frame_prev");

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gb_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gb_uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gb_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
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
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gb_bg"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &tex_cur.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &tex_prev.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: uniforms.as_entire_binding(),
                },
            ],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gb_pl"),
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gb_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(target_format.into())],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        ScreenRenderer {
            pipeline,
            bind_group,
            tex_cur,
            tex_prev,
            uniforms,
        }
    }

    fn upload(&self, queue: &wgpu::Queue, tex: &wgpu::Texture, shades: &[u8]) {
        // Expand 0..3 to 0..255 so R8Unorm sampling lands on 0, 1/3, 2/3, 1.
        let bytes: Vec<u8> = shades.iter().map(|&s| s * 85).collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SCREEN_W as u32),
                rows_per_image: Some(SCREEN_H as u32),
            },
            wgpu::Extent3d {
                width: SCREEN_W as u32,
                height: SCREEN_H as u32,
                depth_or_array_layers: 1,
            },
        );
    }
}

/// Per-frame paint callback carrying the two most recent framebuffers.
pub struct ScreenCallback {
    pub current: Vec<u8>,
    pub previous: Vec<u8>,
    pub ghosting: f32,
    pub grid: f32,
}

impl egui_wgpu::CallbackTrait for ScreenCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let r: &ScreenRenderer = resources.get().expect("ScreenRenderer not registered");
        r.upload(queue, &r.tex_cur, &self.current);
        r.upload(queue, &r.tex_prev, &self.previous);
        queue.write_buffer(
            &r.uniforms,
            0,
            bytemuck::bytes_of(&Uniforms {
                params: [SCREEN_W as f32, SCREEN_H as f32, self.ghosting, self.grid],
            }),
        );
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let r: &ScreenRenderer = resources.get().expect("ScreenRenderer not registered");
        render_pass.set_pipeline(&r.pipeline);
        render_pass.set_bind_group(0, &r.bind_group, &[]);
        render_pass.draw(0..6, 0..1);
    }
}
