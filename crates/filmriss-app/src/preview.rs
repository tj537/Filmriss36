//! GPU live preview: draw camera frame + 3D LUT directly in egui's render pass.
//!
//! Per frame:
//! 1. `prepare`: upload the new camera frame (if any) into a 2D texture, upload the LUT
//!    into a 3D texture (RGBA16F) when the simulation changes, write the uniforms.
//! 2. `paint`: a single triangle – the fragment shader does the rest.
//!
//! The GPU work per pixel is constant (1 camera sample + 1 LUT sample + a little
//! arithmetic), no matter how complex the film simulation is.

use std::sync::Arc;

use egui_wgpu::wgpu;
use filmriss_camera::Frame;
use filmriss_core::Lut3D;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    pub tex_size: [f32; 2],
    pub intensity: f32,
    pub grain: f32,
    pub grain_size: f32,
    pub vignette: f32,
    pub split: f32,
    pub mirror: f32,
    pub seed: u32,
    pub lut_size: f32,
    pub linear_output: f32,
    pub grain_roughness: f32,
    pub pre_gain: [f32; 3],
    pub grain_chroma: f32,
    pub _pad: [f32; 4],
}

impl Uniforms {
    pub fn from_look(p: &filmriss_core::LookParams, split: f32, mirror: bool) -> Self {
        Self {
            intensity: p.intensity,
            grain: p.grain,
            grain_size: p.grain_size,
            grain_roughness: p.grain_roughness,
            grain_chroma: p.grain_chroma,
            vignette: p.vignette,
            pre_gain: p.pre_gain,
            seed: p.seed,
            split,
            mirror: if mirror { 1.0 } else { 0.0 },
            ..Default::default()
        }
    }
}

// Must match the WGSL struct layout (80 bytes, vec3 at offset 48).
const _: () = assert!(std::mem::size_of::<Uniforms>() == 80);

/// A camera frame plus an app-wide unique ID (frame numbers restart per stream).
#[derive(Clone)]
pub struct PreviewFrame {
    pub id: u64,
    pub frame: Arc<Frame>,
}

pub struct PreviewResources {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    cam_sampler: wgpu::Sampler,
    lut_sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    linear_output: bool,

    cam: Option<(wgpu::Texture, [u32; 2])>,
    uploaded_frame: u64,
    lut: Option<(wgpu::Texture, Arc<Lut3D>)>,
    bind_group: Option<wgpu::BindGroup>,
}

impl PreviewResources {
    /// Creates the pipeline & co. and registers them with egui-wgpu.
    pub fn install(rs: &egui_wgpu::RenderState) {
        let device = &rs.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("filmriss-preview"),
            source: wgpu::ShaderSource::Wgsl(include_str!("preview.wgsl").into()),
        });

        let tex_entry = |binding, dim| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: dim,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("filmriss-preview"),
            entries: &[
                tex_entry(0, wgpu::TextureViewDimension::D2),
                sampler_entry(1),
                tex_entry(2, wgpu::TextureViewDimension::D3),
                sampler_entry(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("filmriss-preview"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("filmriss-preview"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: rs.target_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let sampler = |label| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            })
        };

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("filmriss-preview-uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let resources = PreviewResources {
            pipeline,
            layout,
            cam_sampler: sampler("filmriss-cam"),
            lut_sampler: sampler("filmriss-lut"),
            uniforms,
            linear_output: rs.target_format.is_srgb(),
            cam: None,
            uploaded_frame: 0,
            lut: None,
            bind_group: None,
        };
        rs.renderer.write().callback_resources.insert(resources);
    }

    fn upload_frame(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, f: &PreviewFrame) {
        if self.uploaded_frame == f.id {
            return;
        }
        let frame = &f.frame;
        let size = [frame.width, frame.height];
        if self.cam.as_ref().is_none_or(|(_, s)| *s != size) {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("filmriss-camera"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                // Deliberately *not* Srgb: shader and LUT operate on sRGB-encoded values.
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.cam = Some((tex, size));
            self.bind_group = None;
        }
        let (tex, _) = self.cam.as_ref().unwrap();
        queue.write_texture(
            tex.as_image_copy(),
            &frame.data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * size[0]),
                rows_per_image: Some(size[1]),
            },
            wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
        );
        self.uploaded_frame = f.id;
    }

    fn upload_lut(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, lut: &Arc<Lut3D>) {
        if self.lut.as_ref().is_some_and(|(_, l)| Arc::ptr_eq(l, lut)) {
            return;
        }
        let n = lut.size() as u32;
        let extent = wgpu::Extent3d {
            width: n,
            height: n,
            depth_or_array_layers: n,
        };
        let tex = match self.lut.take() {
            Some((tex, old)) if old.size() == lut.size() => tex,
            _ => {
                self.bind_group = None;
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("filmriss-lut"),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D3,
                    format: wgpu::TextureFormat::Rgba16Float,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                })
            }
        };
        let texels: Vec<[half::f16; 4]> = lut
            .data()
            .iter()
            .map(|&[r, g, b]| [r, g, b, 1.0].map(half::f16::from_f32))
            .collect();
        queue.write_texture(
            tex.as_image_copy(),
            bytemuck::cast_slice(&texels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(n * 8),
                rows_per_image: Some(n),
            },
            extent,
        );
        self.lut = Some((tex, lut.clone()));
    }

    fn ensure_bind_group(&mut self, device: &wgpu::Device) {
        if self.bind_group.is_some() {
            return;
        }
        let (Some((cam, _)), Some((lut, _))) = (&self.cam, &self.lut) else {
            return;
        };
        let cam_view = cam.create_view(&Default::default());
        let lut_view = lut.create_view(&Default::default());
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("filmriss-preview"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&cam_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.cam_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&lut_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.lut_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.uniforms.as_entire_binding(),
                },
            ],
        }));
    }
}

/// Paint callback created for every egui frame.
pub struct PreviewCallback {
    pub frame: PreviewFrame,
    pub lut: Arc<Lut3D>,
    pub uniforms: Uniforms,
}

impl egui_wgpu::CallbackTrait for PreviewCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(r) = resources.get_mut::<PreviewResources>() else {
            return Vec::new();
        };
        r.upload_frame(device, queue, &self.frame);
        r.upload_lut(device, queue, &self.lut);
        r.ensure_bind_group(device);

        let mut u = self.uniforms;
        u.tex_size = [self.frame.frame.width as f32, self.frame.frame.height as f32];
        u.lut_size = self.lut.size() as f32;
        u.linear_output = if r.linear_output { 1.0 } else { 0.0 };
        queue.write_buffer(&r.uniforms, 0, bytemuck::bytes_of(&u));
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(r) = resources.get::<PreviewResources>() else {
            return;
        };
        if let Some(bind_group) = &r.bind_group {
            render_pass.set_pipeline(&r.pipeline);
            render_pass.set_bind_group(0, bind_group, &[]);
            render_pass.draw(0..3, 0..1);
        }
    }
}
