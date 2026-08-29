//! `wgpu::util` equivalents.

use std::future::Future;

use crate::*;

pub use wgpu_types::{DispatchIndirectArgs, DrawIndexedIndirectArgs, DrawIndirectArgs, TextureDataOrder};

#[derive(Debug, Clone)]
pub struct BufferInitDescriptor<'a> {
    pub label: Label<'a>,
    pub contents: &'a [u8],
    pub usage: BufferUsages,
}

pub trait DeviceExt {
    fn create_buffer_init(&self, desc: &BufferInitDescriptor<'_>) -> Buffer;

    fn create_texture_with_data(
        &self,
        queue: &Queue,
        desc: &TextureDescriptor<'_>,
        order: TextureDataOrder,
        data: &[u8],
    ) -> Texture;
}

impl DeviceExt for Device {
    fn create_buffer_init(&self, desc: &BufferInitDescriptor<'_>) -> Buffer {
        if desc.contents.is_empty() {
            return self.create_buffer(&BufferDescriptor {
                label: desc.label,
                size: 0,
                usage: desc.usage,
                mapped_at_creation: false,
            });
        }
        let unpadded_size = desc.contents.len() as BufferAddress;
        let align_mask = COPY_BUFFER_ALIGNMENT - 1;
        let padded_size = ((unpadded_size + align_mask) & !align_mask).max(COPY_BUFFER_ALIGNMENT);
        let buffer = self.create_buffer(&BufferDescriptor {
            label: desc.label,
            size: padded_size,
            usage: desc.usage,
            mapped_at_creation: true,
        });
        {
            let mut view = buffer.get_mapped_range_mut(0..unpadded_size);
            view.copy_from_slice(desc.contents);
        }
        buffer.unmap();
        buffer
    }

    fn create_texture_with_data(
        &self,
        queue: &Queue,
        desc: &TextureDescriptor<'_>,
        order: TextureDataOrder,
        data: &[u8],
    ) -> Texture {
        // Implicitly add the COPY_DST usage
        let mut desc = desc.to_owned();
        desc.usage |= TextureUsages::COPY_DST;
        let texture = self.create_texture(&desc);

        let (block_width, block_height) = desc.format.block_dimensions();
        let block_size = desc
            .format
            .block_copy_size(None)
            .expect("copying to depth textures is unsupported");
        let layer_iterations = desc.array_layer_count();

        let (outer_iteration, inner_iteration) = match order {
            TextureDataOrder::LayerMajor => (layer_iterations, desc.mip_level_count),
            TextureDataOrder::MipMajor => (desc.mip_level_count, layer_iterations),
        };

        let mut binary_offset = 0;
        for outer in 0..outer_iteration {
            for inner in 0..inner_iteration {
                let (layer, mip) = match order {
                    TextureDataOrder::LayerMajor => (outer, inner),
                    TextureDataOrder::MipMajor => (inner, outer),
                };

                let mut mip_size = desc.mip_level_size(mip).unwrap();
                if desc.dimension != TextureDimension::D3 {
                    mip_size.depth_or_array_layers = 1;
                }
                let mip_physical = mip_size.physical_size(desc.format);
                let width_blocks = mip_physical.width / block_width;
                let height_blocks = mip_physical.height / block_height;

                let bytes_per_row = width_blocks * block_size;
                let data_size =
                    bytes_per_row * height_blocks * mip_size.depth_or_array_layers;

                let end_offset = binary_offset + data_size as usize;

                queue.write_texture(
                    TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: mip,
                        origin: Origin3d { x: 0, y: 0, z: layer },
                        aspect: TextureAspect::All,
                    },
                    &data[binary_offset..end_offset],
                    TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(bytes_per_row),
                        rows_per_image: Some(height_blocks),
                    },
                    mip_physical,
                );

                binary_offset = end_offset;
            }
        }

        texture
    }
}

/// Initialize the adapter obeying the `WGPU_ADAPTER_NAME` environment
/// variable; the remote backend has exactly one adapter, so this simply
/// requests it.
pub fn initialize_adapter_from_env(
    instance: &Instance,
    compatible_surface: Option<&Surface<'_>>,
) -> impl Future<Output = Result<Adapter, RequestAdapterError>> {
    instance.request_adapter(&RequestAdapterOptions {
        power_preference: PowerPreference::default(),
        force_fallback_adapter: false,
        compatible_surface,
    })
}

pub async fn initialize_adapter_from_env_or_default(
    instance: &Instance,
    compatible_surface: Option<&Surface<'_>>,
) -> Result<Adapter, RequestAdapterError> {
    initialize_adapter_from_env(instance, compatible_surface).await
}

pub fn align_to<T>(value: T, alignment: T) -> T
where
    T: std::ops::Add<Output = T>
        + std::ops::Sub<Output = T>
        + std::ops::Rem<Output = T>
        + Default
        + PartialEq
        + Copy,
{
    let remainder = value % alignment;
    if remainder == T::default() {
        value
    } else {
        value + (alignment - remainder)
    }
}

/// Efficiently performs many buffer writes by sharing and reusing temporary
/// buffers.
pub struct StagingBelt {
    device: Device,
    pending: Vec<Buffer>,
}

impl StagingBelt {
    pub fn new(device: Device, _chunk_size: BufferAddress) -> Self {
        StagingBelt { device, pending: Vec::new() }
    }

    pub fn write_buffer(
        &mut self,
        encoder: &mut CommandEncoder,
        target: &Buffer,
        offset: BufferAddress,
        size: BufferSize,
    ) -> BufferViewMut<'_> {
        let staging = self.device.create_buffer(&BufferDescriptor {
            label: Some("staging belt buffer"),
            size: size.get(),
            usage: BufferUsages::COPY_SRC,
            mapped_at_creation: true,
        });
        encoder.copy_buffer_to_buffer(&staging, 0, target, offset, size.get());
        let view = staging.get_mapped_range_mut(..).detach();
        self.pending.push(staging);
        view
    }

    /// Prepare currently mapped buffers for use in a submission: unmaps them
    /// so the recorded copies see the written data.
    pub fn finish(&mut self) {
        for buffer in &self.pending {
            buffer.unmap();
        }
    }

    /// Recall the buffers; with the simple one-buffer-per-write strategy this
    /// just drops them.
    pub fn recall(&mut self) {
        self.pending.clear();
    }
}

impl std::fmt::Debug for StagingBelt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StagingBelt").finish()
    }
}

/// Renders a source texture into a target texture view with a trivial
/// sampling pipeline.
#[derive(Debug)]
pub struct TextureBlitter {
    pipeline: RenderPipeline,
    bind_group_layout: BindGroupLayout,
    sampler: Sampler,
}

const BLIT_SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertexOutput {
    var out: VertexOutput;
    out.tex_coords = vec2<f32>(
        f32((vi << 1u) & 2u),
        f32(vi & 2u),
    );
    out.position = vec4<f32>(out.tex_coords * 2.0 - 1.0, 0.0, 1.0);
    out.tex_coords.y = 1.0 - out.tex_coords.y;
    return out;
}

@group(0) @binding(0)
var texture: texture_2d<f32>;
@group(0) @binding(1)
var texture_sampler: sampler;

@fragment
fn fs_main(vs: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(texture, texture_sampler, vs.tex_coords);
}
"#;

impl TextureBlitter {
    pub fn new(device: &Device, format: TextureFormat) -> Self {
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("wgpu::util::TextureBlitter::sampler"),
            ..Default::default()
        });
        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("wgpu::util::TextureBlitter::bind_group_layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    visibility: ShaderStages::FRAGMENT,
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    visibility: ShaderStages::FRAGMENT,
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("wgpu::util::TextureBlitter::pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("wgpu::util::TextureBlitter::shader"),
            source: ShaderSource::Wgsl(BLIT_SHADER.into()),
        });
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("wgpu::util::TextureBlitter::pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        TextureBlitter { pipeline, bind_group_layout, sampler }
    }

    pub fn copy(
        &self,
        device: &Device,
        encoder: &mut CommandEncoder,
        source: &TextureView,
        target: &TextureView,
    ) {
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("wgpu::util::TextureBlitter::bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                BindGroupEntry { binding: 0, resource: BindingResource::TextureView(source) },
                BindGroupEntry { binding: 1, resource: BindingResource::Sampler(&self.sampler) },
            ],
        });
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("wgpu::util::TextureBlitter::pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: Operations { load: LoadOp::Load, store: StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
