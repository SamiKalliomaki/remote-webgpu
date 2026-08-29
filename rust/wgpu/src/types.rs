//! The `wgpu` public data types.
//!
//! Everything that is plain data comes straight from the real `wgpu-types`
//! crate, so code compiled against real wgpu (bevy_render in particular)
//! shares type identity with this shim.  Only the types that reference our
//! backend handles (descriptors borrowing a `Buffer`, a `TextureView`, ...)
//! are defined here, mirroring wgpu 29 field for field.

use std::borrow::Cow;
use std::num::NonZeroU32;

use wgpu_types as wgt;

// ---------------------------------------------------------------------
// re-exports from the real wgpu-types crate
// ---------------------------------------------------------------------

pub use wgt::{
    AccelerationStructureFlags, AccelerationStructureGeometryFlags,
    AccelerationStructureUpdateMode, AdapterInfo, AddressMode, AstcBlock, AstcChannel, Backend,
    BackendOptions, Backends, BindGroupLayoutEntry, BindingType, BlasGeometrySizeDescriptors,
    BlasTriangleGeometrySizeDescriptor, BlendComponent, BlendFactor, BlendOperation, BlendState,
    BufferAddress, BufferBindingType, BufferSize, BufferUsages, Color, ColorTargetState,
    ColorWrites, CommandBufferDescriptor, CompareFunction, CompositeAlphaMode, DepthBiasState,
    DepthStencilState, DeviceLostReason, DeviceType, DownlevelCapabilities, DownlevelFlags,
    DownlevelLimits, Dx12BackendOptions, Dx12Compiler, Dx12SwapchainKind,
    Dx12UseFrameLatencyWaitableObject, DxcShaderModel, DynamicOffset, ExperimentalFeatures,
    Extent3d, Face, Features, FeaturesWGPU, FeaturesWebGPU, FilterMode, ForceShaderModelToken,
    FrontFace, GlBackendOptions, GlDebugFns, GlFenceBehavior, Gles3MinorVersion,
    ImageSubresourceRange, WriteOnly, WriteOnlyIter,
    IndexFormat, InstanceDescriptor, InstanceFlags, Limits, LoadOp, MemoryBudgetThresholds,
    MemoryHints, MipmapFilterMode, MultisampleState, NoopBackendOptions, Operations, Origin2d,
    Origin3d, PipelineStatisticsTypes, PollError, PollStatus, PolygonMode, PowerPreference,
    PresentMode, PresentationTimestamp, PrimitiveState, PrimitiveTopology, QueryType,
    RenderBundleDepthStencil, RequestAdapterError, SamplerBindingType, SamplerBorderColor,
    ShaderLocation, ShaderModel, ShaderRuntimeChecks, ShaderStages, StencilFaceState,
    StencilOperation, StencilState, StorageTextureAccess, StoreOp, SurfaceCapabilities,
    SurfaceStatus, TexelCopyBufferLayout, TextureAspect, TextureDimension, TextureFormat,
    TextureFormatFeatureFlags, TextureFormatFeatures, TextureSampleType, TextureUsages,
    TextureViewDimension, Trace, VertexAttribute, VertexFormat, VertexStepMode, WasmNotSend,
    WasmNotSendSync, WasmNotSync, COPY_BUFFER_ALIGNMENT, COPY_BYTES_PER_ROW_ALIGNMENT,
    MAP_ALIGNMENT, QUERY_RESOLVE_BUFFER_ALIGNMENT, QUERY_SET_MAX_QUERIES, QUERY_SIZE,
};

pub type Label<'a> = Option<&'a str>;

pub type DeviceDescriptor<'a> = wgt::DeviceDescriptor<Label<'a>>;
pub type SamplerDescriptor<'a> = wgt::SamplerDescriptor<Label<'a>>;
pub type QuerySetDescriptor<'a> = wgt::QuerySetDescriptor<Label<'a>>;
pub type CommandEncoderDescriptor<'a> = wgt::CommandEncoderDescriptor<Label<'a>>;
pub type RenderBundleDescriptor<'a> = wgt::RenderBundleDescriptor<Label<'a>>;
pub type BufferDescriptor<'a> = wgt::BufferDescriptor<Label<'a>>;
pub type TextureDescriptor<'a> = wgt::TextureDescriptor<Label<'a>, &'a [TextureFormat]>;
pub type TextureViewDescriptor<'a> = wgt::TextureViewDescriptor<Label<'a>>;
pub type SurfaceConfiguration = wgt::SurfaceConfiguration<Vec<TextureFormat>>;
pub type RequestAdapterOptions<'a, 'b> = wgt::RequestAdapterOptions<&'a crate::Surface<'b>>;
pub type PollType = wgt::PollType<SubmissionIndex>;
pub type TexelCopyBufferInfo<'a> = wgt::TexelCopyBufferInfo<&'a crate::Buffer>;
pub type TexelCopyTextureInfo<'a> = wgt::TexelCopyTextureInfo<&'a crate::Texture>;
pub type CreateBlasDescriptor<'a> = wgt::CreateBlasDescriptor<Label<'a>>;
pub type CreateTlasDescriptor<'a> = wgt::CreateTlasDescriptor<Label<'a>>;

// ---------------------------------------------------------------------
// errors
// ---------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct RequestDeviceError {
    pub(crate) message: String,
}
impl std::fmt::Display for RequestDeviceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "requesting device failed: {}", self.message)
    }
}
impl std::error::Error for RequestDeviceError {}

#[derive(Clone, Debug)]
pub struct CreateSurfaceError {
    pub(crate) message: String,
}
impl std::fmt::Display for CreateSurfaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "creating a surface failed: {}", self.message)
    }
}
impl std::error::Error for CreateSurfaceError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BufferAsyncError;
impl std::fmt::Display for BufferAsyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "error occurred when trying to async map a buffer")
    }
}
impl std::error::Error for BufferAsyncError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MapRangeError {
    NotMapped,
    OutOfRange,
}
impl std::fmt::Display for MapRangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "buffer range is not mapped")
    }
}
impl std::error::Error for MapRangeError {}

/// Lower level source of an [`Error`].
pub type ErrorSource = Box<dyn std::error::Error + Send + Sync + 'static>;

#[derive(Debug)]
pub enum Error {
    OutOfMemory { source: ErrorSource },
    Validation { source: ErrorSource, description: String },
    Internal { source: ErrorSource, description: String },
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::OutOfMemory { source }
            | Self::Validation { source, .. }
            | Self::Internal { source, .. } => Some(source.as_ref()),
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutOfMemory { .. } => write!(f, "out of memory"),
            Self::Validation { description, .. } | Self::Internal { description, .. } => {
                write!(f, "{description}")
            }
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ErrorFilter {
    OutOfMemory,
    Validation,
    Internal,
}

// ---------------------------------------------------------------------
// misc queue / buffer helper types
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SubmissionIndex(#[allow(dead_code)] pub(crate) u64);

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum MapMode {
    Read,
    Write,
}

// ---------------------------------------------------------------------
// bind groups
// ---------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct BindGroupLayoutDescriptor<'a> {
    pub label: Label<'a>,
    pub entries: &'a [BindGroupLayoutEntry],
}

#[derive(Clone, Debug)]
pub struct BufferBinding<'a> {
    pub buffer: &'a crate::Buffer,
    pub offset: BufferAddress,
    pub size: Option<BufferSize>,
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum BindingResource<'a> {
    Buffer(BufferBinding<'a>),
    BufferArray(&'a [BufferBinding<'a>]),
    Sampler(&'a crate::Sampler),
    SamplerArray(&'a [&'a crate::Sampler]),
    TextureView(&'a crate::TextureView),
    TextureViewArray(&'a [&'a crate::TextureView]),
    AccelerationStructure(&'a crate::Tlas),
}

#[derive(Clone, Debug)]
pub struct BindGroupEntry<'a> {
    pub binding: u32,
    pub resource: BindingResource<'a>,
}

#[derive(Clone, Debug)]
pub struct BindGroupDescriptor<'a> {
    pub label: Label<'a>,
    pub layout: &'a crate::BindGroupLayout,
    pub entries: &'a [BindGroupEntry<'a>],
}

#[derive(Clone, Debug, Default)]
pub struct PipelineLayoutDescriptor<'a> {
    pub label: Label<'a>,
    pub bind_group_layouts: &'a [Option<&'a crate::BindGroupLayout>],
    pub immediate_size: u32,
}

// ---------------------------------------------------------------------
// shaders and pipelines
// ---------------------------------------------------------------------

#[derive(Clone, Debug)]
#[non_exhaustive]
#[allow(clippy::large_enum_variant)]
pub enum ShaderSource<'a> {
    /// SPIR-V is not supported by the remote backend; creating a module
    /// from it panics.
    SpirV(Cow<'a, [u32]>),
    Wgsl(Cow<'a, str>),
    /// A Naga IR module; the shim writes it back out as WGSL before
    /// sending it to the browser.
    Naga(Cow<'static, naga::Module>),
}

#[derive(Clone, Debug)]
pub struct ShaderModuleDescriptor<'a> {
    pub label: Label<'a>,
    pub source: ShaderSource<'a>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PipelineCompilationOptions<'a> {
    pub constants: &'a [(&'a str, f64)],
    pub zero_initialize_workgroup_memory: bool,
}

impl PipelineCompilationOptions<'_> {
    pub const fn default() -> Self {
        Self { constants: &[], zero_initialize_workgroup_memory: true }
    }
}

impl Default for PipelineCompilationOptions<'_> {
    fn default() -> Self {
        Self::default()
    }
}

#[derive(Clone, Debug)]
pub struct PipelineCacheDescriptor<'a> {
    pub label: Label<'a>,
    pub data: Option<&'a [u8]>,
    pub fallback: bool,
}

#[derive(Clone, Debug)]
pub struct VertexBufferLayout<'a> {
    pub array_stride: BufferAddress,
    pub step_mode: VertexStepMode,
    pub attributes: &'a [VertexAttribute],
}

#[derive(Clone)]
pub struct VertexState<'a> {
    pub module: &'a crate::ShaderModule,
    pub entry_point: Option<&'a str>,
    pub compilation_options: PipelineCompilationOptions<'a>,
    pub buffers: &'a [VertexBufferLayout<'a>],
}

#[derive(Clone)]
pub struct FragmentState<'a> {
    pub module: &'a crate::ShaderModule,
    pub entry_point: Option<&'a str>,
    pub compilation_options: PipelineCompilationOptions<'a>,
    pub targets: &'a [Option<ColorTargetState>],
}

#[derive(Clone)]
pub struct RenderPipelineDescriptor<'a> {
    pub label: Label<'a>,
    pub layout: Option<&'a crate::PipelineLayout>,
    pub vertex: VertexState<'a>,
    pub primitive: PrimitiveState,
    pub depth_stencil: Option<DepthStencilState>,
    pub multisample: MultisampleState,
    pub fragment: Option<FragmentState<'a>>,
    pub multiview_mask: Option<NonZeroU32>,
    pub cache: Option<&'a crate::PipelineCache>,
}

#[derive(Clone)]
pub struct ComputePipelineDescriptor<'a> {
    pub label: Label<'a>,
    pub layout: Option<&'a crate::PipelineLayout>,
    pub module: &'a crate::ShaderModule,
    pub entry_point: Option<&'a str>,
    pub compilation_options: PipelineCompilationOptions<'a>,
    pub cache: Option<&'a crate::PipelineCache>,
}

// ---------------------------------------------------------------------
// passes
// ---------------------------------------------------------------------

#[derive(Clone)]
pub struct RenderPassColorAttachment<'a> {
    pub view: &'a crate::TextureView,
    pub depth_slice: Option<u32>,
    pub resolve_target: Option<&'a crate::TextureView>,
    pub ops: Operations<Color>,
}

#[derive(Clone)]
pub struct RenderPassDepthStencilAttachment<'a> {
    pub view: &'a crate::TextureView,
    pub depth_ops: Option<Operations<f32>>,
    pub stencil_ops: Option<Operations<u32>>,
}

#[derive(Clone)]
pub struct RenderPassTimestampWrites<'a> {
    pub query_set: &'a crate::QuerySet,
    pub beginning_of_pass_write_index: Option<u32>,
    pub end_of_pass_write_index: Option<u32>,
}

#[derive(Clone, Default)]
pub struct RenderPassDescriptor<'a> {
    pub label: Label<'a>,
    pub color_attachments: &'a [Option<RenderPassColorAttachment<'a>>],
    pub depth_stencil_attachment: Option<RenderPassDepthStencilAttachment<'a>>,
    pub timestamp_writes: Option<RenderPassTimestampWrites<'a>>,
    pub occlusion_query_set: Option<&'a crate::QuerySet>,
    pub multiview_mask: Option<NonZeroU32>,
}

#[derive(Clone)]
pub struct ComputePassTimestampWrites<'a> {
    pub query_set: &'a crate::QuerySet,
    pub beginning_of_pass_write_index: Option<u32>,
    pub end_of_pass_write_index: Option<u32>,
}

#[derive(Clone, Default)]
pub struct ComputePassDescriptor<'a> {
    pub label: Label<'a>,
    pub timestamp_writes: Option<ComputePassTimestampWrites<'a>>,
}

#[derive(Debug, Clone, Default)]
pub struct RenderBundleEncoderDescriptor<'a> {
    pub label: Label<'a>,
    pub color_formats: &'a [Option<TextureFormat>],
    pub depth_stencil: Option<RenderBundleDepthStencil>,
    pub sample_count: u32,
    pub multiview: Option<NonZeroU32>,
}

// ---------------------------------------------------------------------
// surfaces
// ---------------------------------------------------------------------

/// The unsafe raw-handle surface target.  The remote backend has no real
/// window handles; a window is identified by smuggling its client id
/// through [`raw_window_handle::WebWindowHandle`], which
/// [`crate::Instance::create_surface_unsafe`] decodes again.
#[non_exhaustive]
pub enum SurfaceTargetUnsafe {
    RawHandle {
        raw_display_handle: Option<raw_window_handle::RawDisplayHandle>,
        raw_window_handle: raw_window_handle::RawWindowHandle,
    },
}

// ---------------------------------------------------------------------
// ray tracing stubs
// ---------------------------------------------------------------------

// Acceleration structures do not exist in WebGPU.  These types exist only
// so downstream re-exports (bevy_render's in particular) compile; there is
// no way to construct the handles.

#[derive(Debug, Clone)]
pub struct Blas {
    _private: (),
}

#[derive(Debug, Clone)]
pub struct Tlas {
    _private: (),
}

#[derive(Debug, Clone)]
pub struct TlasInstance {
    _private: (),
}

#[derive(Clone)]
pub struct BlasTriangleGeometry<'a> {
    pub size: &'a BlasTriangleGeometrySizeDescriptor,
    pub vertex_buffer: &'a crate::Buffer,
    pub first_vertex: u32,
    pub vertex_stride: BufferAddress,
    pub index_buffer: Option<&'a crate::Buffer>,
    pub first_index: Option<u32>,
    pub transform_buffer: Option<&'a crate::Buffer>,
    pub transform_buffer_offset: Option<BufferAddress>,
}

#[derive(Clone)]
pub enum BlasGeometries<'a> {
    TriangleGeometries(Vec<BlasTriangleGeometry<'a>>),
}

#[derive(Clone)]
pub struct BlasBuildEntry<'a> {
    pub blas: &'a Blas,
    pub geometry: BlasGeometries<'a>,
}
