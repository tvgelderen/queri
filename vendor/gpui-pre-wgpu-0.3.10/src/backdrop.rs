//! In-window Gaussian blur. The scene's sampling barriers ensure that a dialog
//! samples the workspace, never its own foreground or a later overlay.
use bytemuck::{Pod, Zeroable};
use gpui::{BackdropBlur, DevicePixels, Size};
use wgpu::util::DeviceExt;

const DOWNSAMPLE: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    bounds: [f32; 4],
    sigma: f32,
    opacity: f32,
    padding: [f32; 2],
}

struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Target {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat, width: u32, height: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("backdrop_texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self { texture, view }
    }
}

struct Pipelines {
    downsample: wgpu::RenderPipeline,
    horizontal: wgpu::RenderPipeline,
    vertical: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    copy: wgpu::RenderPipeline,
}

struct Pass<'a> {
    pipeline: &'a wgpu::RenderPipeline,
    source: &'a wgpu::TextureView,
    blurred: &'a wgpu::TextureView,
    target: &'a wgpu::TextureView,
    load: wgpu::LoadOp<wgpu::Color>,
}

pub(crate) struct BackdropRenderer {
    layout: wgpu::BindGroupLayout,
    pipelines: Pipelines,
    sampler: wgpu::Sampler,
    copy_params: wgpu::Buffer,
    target: Target,
    snapshot: Target,
    small: Target,
    scratch: Target,
}

impl BackdropRenderer {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        size: Size<DevicePixels>,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("backdrop_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("backdrop_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("backdrop.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("backdrop_pipeline_layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_fullscreen"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipelines = Pipelines {
            downsample: pipeline("fs_downsample"),
            horizontal: pipeline("fs_horizontal"),
            vertical: pipeline("fs_vertical"),
            composite: pipeline("fs_composite"),
            copy: pipeline("fs_copy"),
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("backdrop_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let copy_params = Self::params_buffer(device, Params::zeroed());
        let width = size.width.0 as u32;
        let height = size.height.0 as u32;
        let small_width = width.div_ceil(DOWNSAMPLE);
        let small_height = height.div_ceil(DOWNSAMPLE);
        Self {
            layout,
            pipelines,
            sampler,
            copy_params,
            target: Target::new(device, format, width, height),
            snapshot: Target::new(device, format, width, height),
            small: Target::new(device, format, small_width, small_height),
            scratch: Target::new(device, format, small_width, small_height),
        }
    }

    pub fn matches_size(&self, size: Size<DevicePixels>) -> bool {
        self.target.texture.width() == size.width.0 as u32
            && self.target.texture.height() == size.height.0 as u32
    }

    pub fn target_view(&self) -> &wgpu::TextureView {
        &self.target.view
    }

    fn params_buffer(device: &wgpu::Device, params: Params) -> wgpu::Buffer {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("backdrop_params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        })
    }

    pub fn paint(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        blur: &BackdropBlur,
    ) {
        // Sampling an attachment in its own render pass is invalid. Keep an
        // untouched snapshot for both the blur and opacity-correct compositing.
        encoder.copy_texture_to_texture(
            self.target.texture.as_image_copy(),
            self.snapshot.texture.as_image_copy(),
            self.target.texture.size(),
        );
        let clipped = blur.bounds.intersect(&blur.content_mask.bounds);
        let params = Self::params_buffer(
            device,
            Params {
                bounds: [
                    clipped.left().0,
                    clipped.top().0,
                    clipped.right().0,
                    clipped.bottom().0,
                ],
                sigma: blur.blur_radius.0 / DOWNSAMPLE as f32,
                opacity: blur.opacity,
                padding: [0.; 2],
            },
        );
        for (pipeline, source, target) in [
            (
                &self.pipelines.downsample,
                &self.snapshot.view,
                &self.small.view,
            ),
            (
                &self.pipelines.horizontal,
                &self.small.view,
                &self.scratch.view,
            ),
            (
                &self.pipelines.vertical,
                &self.scratch.view,
                &self.small.view,
            ),
        ] {
            self.pass(
                device,
                encoder,
                &params,
                Pass {
                    pipeline,
                    source,
                    target,
                    blurred: &self.snapshot.view,
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                },
            );
        }
        self.pass(
            device,
            encoder,
            &params,
            Pass {
                pipeline: &self.pipelines.composite,
                source: &self.snapshot.view,
                blurred: &self.small.view,
                target: &self.target.view,
                load: wgpu::LoadOp::Load,
            },
        );
    }

    pub fn blit(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::TextureView,
    ) {
        self.pass(
            device,
            encoder,
            &self.copy_params,
            Pass {
                pipeline: &self.pipelines.copy,
                source: &self.target.view,
                blurred: &self.snapshot.view,
                target: output,
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            },
        );
    }

    fn pass(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        params: &wgpu::Buffer,
        pass: Pass,
    ) {
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("backdrop_bindings"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(pass.source),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(pass.blurred),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("backdrop_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: pass.target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: pass.load,
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            ..Default::default()
        });
        render_pass.set_pipeline(pass.pipeline);
        render_pass.set_bind_group(0, &bindings, &[]);
        render_pass.draw(0..3, 0..1);
    }
}
